//! PDF 生成和注释导出模块
//!
//! 这个文件负责做项目里最“落地”的那一步：
//! 把已经解析好的注释数据，真正写进 PDF。
//!
//! 如果把整个项目想成一条流水线，那它大概是这样：
//! 1. `xfdf.rs` 负责把 XFDF/XML 解析成 Rust 结构体
//! 2. `pdf.rs` 负责把这些结构体变成 PDF 里的对象和外观
//!
//! 所以这里的核心关注点不是“怎么读 XML”，而是：
//! - 注释在 PDF 里要写成什么字典
//! - 颜色、边框、文字、坐标怎么落到 PDF 对象里
//! - 怎样补显式 AP（Appearance Stream），让阅读器更稳定地显示图形
//!
//! 使用 lopdf 库创建 PDF 文件并将 XFDF 注释写入

use base64::Engine;
use crate::annotation::*;
use crate::error::{PdfXmlError, Result};
use crate::xfdf::XfdfDocument;
use image::GenericImageView;
use log::{debug, info, warn};
use lopdf;
use std::fs;
use std::path::{Path, PathBuf};
use ttf_parser::{Face, OutlineBuilder};

fn contains_chinese(s: &str) -> bool {
    // 这是一个很实用的小判断：
    // 如果内容里有中文，就不能完全按简单西文字体那套方式处理。
    // 后面的 FreeText 渲染、编码和字体选择都会用到它。
    s.chars().any(|c| {
        matches!(c,
            '\u{4E00}'..='\u{9FFF}' |
            '\u{3400}'..='\u{4DBF}' |
            '\u{3000}'..='\u{303F}' |
            '\u{FF00}'..='\u{FFEF}'
        )
    })
}

fn to_utf16be(s: &str) -> Vec<u8> {
    // PDF 里如果直接写中文，常常需要转成 UTF-16BE。
    // 可以简单理解成：把字符串变成 PDF 更容易正确识别的字节格式。
    let mut result = Vec::with_capacity(s.len() * 2 + 2);
    result.push(0xFE);
    result.push(0xFF);
    for ch in s.encode_utf16() {
        result.push((ch >> 8) as u8);
        result.push((ch & 0xFF) as u8);
    }
    result
}

fn escape_pdf_literal_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)")
}

fn replace_font_in_da(da: &str, new_font: &str) -> String {
    let trimmed = da.trim();
    let re = regex::Regex::new(r"/[A-Za-z][A-Za-z0-9_-]*\s+\d+(\.\d+)?\s*Tf").unwrap();
    if let Some(m) = re.find(trimmed) {
        let original = m.as_str();
        let parts: Vec<&str> = original.split_whitespace().collect();
        if parts.len() >= 3 {
            let size = parts[parts.len() - 2];
            let prefix = trimmed[..m.start()].trim_end();
            if prefix.is_empty() {
                format!("{} {} Tf", new_font, size)
            } else {
                format!("{} {} {} Tf", prefix, new_font, size)
            }
        } else {
            trimmed.to_string()
        }
    } else if trimmed.is_empty() {
        format!("{} 12 Tf", new_font)
    } else {
        format!("{} {} 12 Tf", trimmed, new_font)
    }
}

fn apply_text_color_to_da(da: &str, text_color: Option<&str>) -> String {
    let Some(text_color) = text_color else {
        return da.trim().to_string();
    };
    let Some(color) = Color::from_hex(text_color) else {
        return da.trim().to_string();
    };

    let trimmed = da.trim();
    let re = regex::Regex::new(r"(?:\d*\.?\d+\s+){2}\d*\.?\d+\s+r[gG]").unwrap();
    let replacement_rg = format!("{} {} {} rg", color.r, color.g, color.b);
    let replacement_rg_stroke = format!("{} {} {} RG", color.r, color.g, color.b);

    let updated = re
        .replace_all(trimmed, |caps: &regex::Captures| {
            let matched = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
            if matched.ends_with("RG") {
                replacement_rg_stroke.clone()
            } else {
                replacement_rg.clone()
            }
        })
        .to_string();

    if updated == trimmed {
        format!("{} {} {} RG {}", color.r, color.g, color.b, replacement_rg)
    } else {
        updated
    }
}

fn parse_font_size_from_da(da: &str) -> f32 {
    let re = regex::Regex::new(r"/[A-Za-z][A-Za-z0-9_-]*\s+(\d+(?:\.\d+)?)\s*Tf").unwrap();
    re.captures(da)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<f32>().ok())
        .unwrap_or(12.0)
}

fn extract_fill_color_from_da(da: &str) -> (f32, f32, f32) {
    let re = regex::Regex::new(r"(\d*\.?\d+)\s+(\d*\.?\d+)\s+(\d*\.?\d+)\s+rg").unwrap();
    re.captures(da)
        .and_then(|caps| {
            Some((
                caps.get(1)?.as_str().parse().ok()?,
                caps.get(2)?.as_str().parse().ok()?,
                caps.get(3)?.as_str().parse().ok()?,
            ))
        })
        .unwrap_or((0.0, 0.0, 0.0))
}

fn parse_alignment_from_style(style: Option<&str>) -> i32 {
    let Some(style) = style else { return 0; };
    let lower = style.to_ascii_lowercase();
    if lower.contains("text-align:center") {
        1
    } else if lower.contains("text-align:right") {
        2
    } else {
        0
    }
}

#[derive(Debug, Clone)]
struct FreeTextRenderSpec {
    // 真正要画出来的文字内容。
    contents: String,
    // PDF 里的默认外观字符串（DA）。
    // 里面通常带字体、字号、颜色这类信息。
    da: String,
    // 对齐方式：0=左对齐，1=居中，2=右对齐。
    align: i32,
    // 是否包含中文/中日韩文字。
    // 这个标记会决定后面走“普通文字 AP”还是“字形轮廓 AP”。
    is_cjk: bool,
}

// 这个结构体是给 ttf-parser 用的“画笔”。
// 字体库告诉我们：一个字形要 move_to / line_to / curve_to 到哪里，
// 我们就在这里把这些动作翻译成 PDF path 命令。
struct GlyphPathBuilder {
    path: String,
    current_x: f32,
    current_y: f32,
    start_x: f32,
    start_y: f32,
    has_current_point: bool,
}

impl GlyphPathBuilder {
    fn new() -> Self {
        Self {
            path: String::new(),
            current_x: 0.0,
            current_y: 0.0,
            start_x: 0.0,
            start_y: 0.0,
            has_current_point: false,
        }
    }
}

impl OutlineBuilder for GlyphPathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.push_str(&format!("{} {} m\n", x, y));
        self.current_x = x;
        self.current_y = y;
        self.start_x = x;
        self.start_y = y;
        self.has_current_point = true;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.push_str(&format!("{} {} l\n", x, y));
        self.current_x = x;
        self.current_y = y;
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        if !self.has_current_point {
            self.move_to(x, y);
            return;
        }

        let x0 = self.current_x;
        let y0 = self.current_y;
        let c1x = x0 + (2.0 / 3.0) * (x1 - x0);
        let c1y = y0 + (2.0 / 3.0) * (y1 - y0);
        let c2x = x + (2.0 / 3.0) * (x1 - x);
        let c2y = y + (2.0 / 3.0) * (y1 - y);
        self.path.push_str(&format!("{} {} {} {} {} {} c\n", c1x, c1y, c2x, c2y, x, y));
        self.current_x = x;
        self.current_y = y;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.push_str(&format!("{} {} {} {} {} {} c\n", x1, y1, x2, y2, x, y));
        self.current_x = x;
        self.current_y = y;
    }

    fn close(&mut self) {
        self.path.push_str("h\n");
        self.current_x = self.start_x;
        self.current_y = self.start_y;
    }
}

pub struct PdfAnnotationExporter {
    // 默认页面大小。创建“新 PDF”时会用到它。
    // 目前默认值接近 A4（595 x 842 point）。
    page_size: (f64, f64),
}

impl PdfAnnotationExporter {
    /// 创建一个导出器。
    ///
    /// 你可以把它理解成“准备好一个负责写 PDF 的工具对象”。
    /// 后面无论是新建 PDF，还是合并到已有 PDF，都会通过它来完成。
    pub fn new() -> Self {
        Self {
            page_size: (595.0, 842.0),
        }
    }

    #[allow(dead_code)]
    /// 创建一个自定义页面大小的导出器。
    ///
    /// 当你不是用默认页面尺寸时，可以用这个函数覆盖宽高。
    pub fn with_page_size(width: f64, height: f64) -> Self {
        Self {
            page_size: (width, height),
        }
    }

    fn candidate_chinese_font_paths() -> Vec<PathBuf> {
        // 按顺序尝试几个常见中文字体。
        // 这里只是“候选列表”，真正能不能用还要看文件是否存在。
        vec![
            PathBuf::from("C:/Windows/Fonts/simsun.ttc"),
            PathBuf::from("C:/Windows/Fonts/msyh.ttc"),
            PathBuf::from("C:/Windows/Fonts/simhei.ttf"),
            PathBuf::from("C:/Windows/Fonts/simkai.ttf"),
            PathBuf::from("C:/Windows/Fonts/simsunb.ttf"),
        ]
    }

    fn load_chinese_font_bytes() -> Result<Vec<u8>> {
        // 找到第一份可用中文字体后就直接读取。
        // 后面的中文 FreeText 外观流会拿它的字形轮廓来画字。
        for path in Self::candidate_chinese_font_paths() {
            if path.exists() {
                info!("使用中文字体轮廓: {:?}", path);
                return fs::read(&path)
                    .map_err(|e| PdfXmlError::PdfProcessing(format!("读取字体失败 {:?}: {}", path, e)));
            }
        }
        Err(PdfXmlError::PdfProcessing("未找到可用的中文字体文件".to_string()))
    }

    fn load_chinese_face() -> Result<Face<'static>> {
        // 把字体字节解析成 ttf-parser 能理解的 Face 对象。
        // 后面可以通过它拿到每个字的轮廓。
        let bytes = Self::load_chinese_font_bytes()?;
        let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        Face::parse(leaked, 0)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("解析中文字体失败: {}", e)))
    }

    fn build_freetext_render_spec(annotation: &FreeTextAnnotation) -> Option<FreeTextRenderSpec> {
        // 先把 FreeText 真正渲染时需要的关键信息整理出来，
        // 这样后面的“生成西文 AP / 生成中文 AP”就不用重复判断一遍。
        let base = &annotation.base;
        let contents = base.contents.as_ref()?.trim().to_string();
        if contents.is_empty() {
            return None;
        }

        let preferred_text_color = annotation.text_color.as_deref().or(base.color.as_deref());
        let align = if annotation.align != 0 {
            annotation.align
        } else {
            parse_alignment_from_style(annotation.default_style.as_deref())
        };
        let is_cjk = contains_chinese(&contents);

        // DA 可以理解成“默认文字外观说明”。
        // 这里会尽量把字体、字号、颜色整理成后面好直接使用的形式。
        let base_da = if let Some(da) = &annotation.default_appearance {
            if is_cjk {
                replace_font_in_da(da, "/F0")
            } else {
                da.trim().to_string()
            }
        } else if is_cjk {
            "/F0 12 Tf".to_string()
        } else {
            "/Helvetica 12 Tf".to_string()
        };

        Some(FreeTextRenderSpec {
            contents,
            da: apply_text_color_to_da(&base_da, preferred_text_color),
            align,
            is_cjk,
        })
    }

    fn set_common_annotation_fields(dict: &mut lopdf::Dictionary, base: &AnnotationBase) {
        if let Some(r) = base.rect.as_ref() {
            dict.set("Rect", lopdf::Object::Array(vec![
                lopdf::Object::Real(r.left as f32),
                lopdf::Object::Real(r.bottom as f32),
                lopdf::Object::Real(r.right as f32),
                lopdf::Object::Real(r.top as f32),
            ]));
        } else {
            dict.set("Rect", lopdf::Object::Array(vec![
                lopdf::Object::Real(100.0),
                lopdf::Object::Real(700.0),
                lopdf::Object::Real(200.0),
                lopdf::Object::Real(720.0),
            ]));
        }

        if let Some(c) = &base.contents {
            if contains_chinese(c) {
                dict.set("Contents", lopdf::Object::String(to_utf16be(c), lopdf::StringFormat::Hexadecimal));
            } else {
                dict.set("Contents", lopdf::Object::String(c.clone().into_bytes(), lopdf::StringFormat::Literal));
            }
        }
        if let Some(t) = &base.title {
            if contains_chinese(t) {
                dict.set("T", lopdf::Object::String(to_utf16be(t), lopdf::StringFormat::Hexadecimal));
            } else {
                dict.set("T", lopdf::Object::String(t.clone().into_bytes(), lopdf::StringFormat::Literal));
            }
        }
        if let Some(s) = &base.subject {
            if contains_chinese(s) {
                dict.set("Subj", lopdf::Object::String(to_utf16be(s), lopdf::StringFormat::Hexadecimal));
            } else {
                dict.set("Subj", lopdf::Object::String(s.clone().into_bytes(), lopdf::StringFormat::Literal));
            }
        }

        if let Some(cs) = &base.color {
            if let Some(c) = Color::from_hex(cs) {
                dict.set("C", lopdf::Object::Array(vec![
                    lopdf::Object::Real(c.r),
                    lopdf::Object::Real(c.g),
                    lopdf::Object::Real(c.b),
                ]));
            }
        } else {
            dict.set("C", lopdf::Object::Array(vec![
                lopdf::Object::Real(1.0),
                lopdf::Object::Real(1.0),
                lopdf::Object::Real(0.0),
            ]));
        }

        if let Some(dt) = &base.modification_date {
            dict.set("M", lopdf::Object::String(dt.clone().into_bytes(), lopdf::StringFormat::Literal));
        }
        if let Some(dt) = &base.creation_date {
            dict.set("CreationDate", lopdf::Object::String(dt.clone().into_bytes(), lopdf::StringFormat::Literal));
        }
        if base.flags != 0 {
            dict.set("F", lopdf::Object::Integer(base.flags as i64));
        }
    }

    fn build_base_annotation_dict(annotation: &Annotation) -> lopdf::Dictionary {
        let subtype = match annotation {
            Annotation::Text(_) => "Text",
            Annotation::Highlight(_) => "Highlight",
            Annotation::Underline(_) => "Underline",
            Annotation::StrikeOut(_) => "StrikeOut",
            Annotation::Squiggly(_) => "Squiggly",
            Annotation::FreeText(_) => "FreeText",
            Annotation::Square(_) => "Square",
            Annotation::Circle(_) => "Circle",
            Annotation::Line(_) => "Line",
            Annotation::Polygon(p) => {
                if p.is_closed { "Polygon" } else { "PolyLine" }
            }
            Annotation::Ink(_) => "Ink",
            Annotation::Stamp(_) => "Stamp",
            Annotation::Popup(_) => "Popup",
        };

        let mut dict = lopdf::Dictionary::new();
        dict.set("Type", lopdf::Object::Name(b"Annot".to_vec()));
        dict.set("Subtype", lopdf::Object::Name(subtype.as_bytes().to_vec()));
        Self::set_common_annotation_fields(&mut dict, annotation.base());
        dict
    }

    fn build_text_ap_stream(spec: &FreeTextRenderSpec, rect: Option<&Rect>) -> lopdf::Object {
        // 这是“普通西文文本”的 AP 生成方式。
        // 思路比较直接：算出一个框，在里面用 Helvetica 把文字写进去。
        let font_size = parse_font_size_from_da(&spec.da);
        let (width, height) = if let Some(rect) = rect {
            (
                ((rect.right - rect.left).abs() as f32).max(1.0),
                ((rect.top - rect.bottom).abs() as f32).max(font_size + 4.0),
            )
        } else {
            (100.0, (font_size + 4.0).max(20.0))
        };

        let (r, g, b) = extract_fill_color_from_da(&spec.da);
        let escaped = escape_pdf_literal_string(&spec.contents);
        // 这里的 text_width 只是一个近似值，目的不是排版系统级精确，
        // 而是让左中右对齐大致可用。
        let text_width = spec.contents.chars().count() as f32 * font_size * 0.55;
        let x = if spec.align == 1 {
            ((width - text_width) / 2.0).max(2.0)
        } else if spec.align == 2 {
            (width - text_width - 2.0).max(2.0)
        } else {
            2.0
        };

        let content = format!(
            "q\n{} {} {} rg\nBT\n/Helvetica {} Tf\n1 0 0 1 {} {} Tm\n({}) Tj\nET\nQ\n",
            r, g, b, font_size, x, (height - font_size).max(2.0), escaped
        );

        let mut helv = lopdf::Dictionary::new();
        helv.set("Type", lopdf::Object::Name(b"Font".to_vec()));
        helv.set("Subtype", lopdf::Object::Name(b"Type1".to_vec()));
        helv.set("BaseFont", lopdf::Object::Name(b"Helvetica".to_vec()));

        let stream_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"XObject".to_vec())),
            ("Subtype", lopdf::Object::Name(b"Form".to_vec())),
            ("BBox", lopdf::Object::Array(vec![
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(width),
                lopdf::Object::Real(height),
            ])),
            ("Resources", lopdf::Object::Dictionary({
                let mut resources = lopdf::Dictionary::new();
                resources.set("Font", lopdf::Object::Dictionary({
                    let mut fonts = lopdf::Dictionary::new();
                    fonts.set("Helvetica", lopdf::Object::Dictionary(helv));
                    fonts
                }));
                resources
            })),
        ]);

        lopdf::Object::Stream(lopdf::Stream::new(stream_dict, content.into_bytes()))
    }

    fn build_cjk_ap_stream(spec: &FreeTextRenderSpec, rect: Option<&Rect>) -> Result<lopdf::Object> {
        // 中文这条路不能简单依赖 Helvetica 直接写字，
        // 所以这里改成“拿到字体字形轮廓，然后把每个字真正画出来”。
        let face = Self::load_chinese_face()?;
        let font_size = parse_font_size_from_da(&spec.da);
        let (width, height) = if let Some(rect) = rect {
            (
                ((rect.right - rect.left).abs() as f32).max(1.0),
                ((rect.top - rect.bottom).abs() as f32).max(font_size + 4.0),
            )
        } else {
            (100.0, (font_size + 4.0).max(20.0))
        };

        let (r, g, b) = extract_fill_color_from_da(&spec.da);
        let units_per_em = face.units_per_em() as f32;
        let scale = font_size / units_per_em;
        let baseline = 2.0 + font_size * 0.15;

        let advances: Vec<f32> = spec.contents.chars().map(|ch| {
            face.glyph_index(ch)
                .and_then(|gid| face.glyph_hor_advance(gid))
                .map(|w| w as f32 * scale)
                .unwrap_or(font_size)
        }).collect();
        let total_advance: f32 = advances.iter().sum();
        let mut x_cursor = if spec.align == 1 {
            ((width - total_advance) / 2.0).max(2.0)
        } else if spec.align == 2 {
            (width - total_advance - 2.0).max(2.0)
        } else {
            2.0
        };

        let mut content = format!("q\n{} {} {} rg\n", r, g, b);
        for (index, ch) in spec.contents.chars().enumerate() {
            if let Some(glyph_id) = face.glyph_index(ch) {
                let mut builder = GlyphPathBuilder::new();
                if face.outline_glyph(glyph_id, &mut builder).is_some() {
                    // 对每个字符：
                    // 1. 取字形轮廓
                    // 2. 缩放到目标字号
                    // 3. 平移到当前 x_cursor 位置
                    // 4. 用 path 填充出来
                    content.push_str("q\n");
                    content.push_str(&format!("{} 0 0 {} {} {} cm\n", scale, scale, x_cursor, baseline));
                    content.push_str(&builder.path);
                    content.push_str("f\nQ\n");
                }
            }
            x_cursor += advances[index].max(font_size * 0.5);
        }
        content.push_str("Q\n");

        let stream_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"XObject".to_vec())),
            ("Subtype", lopdf::Object::Name(b"Form".to_vec())),
            ("BBox", lopdf::Object::Array(vec![
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(width),
                lopdf::Object::Real(height),
            ])),
        ]);

        Ok(lopdf::Object::Stream(lopdf::Stream::new(stream_dict, content.into_bytes())))
    }


    fn decode_stamp_image_data(image_data: &str) -> Result<Vec<u8>> {
        // stamp 里的图片通常是 data URL/base64，这里先把它还原成原始字节。
        let encoded = image_data
            .split(",")
            .nth(1)
            .ok_or_else(|| PdfXmlError::PdfProcessing("无效的 stamp imagedata 格式".to_string()))?;
        let normalized = encoded
            .replace("\\n", "")
            .replace("\\r", "")
            .replace(char::is_whitespace, "");
        base64::engine::general_purpose::STANDARD
            .decode(normalized)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("解码 stamp 图片失败: {}", e)))
    }

    fn build_stamp_ap_stream(
        &self,
        document: &mut lopdf::Document,
        annotation: &StampAnnotation,
    ) -> Result<Option<lopdf::Object>> {
        let Some(image_data) = annotation.image_data.as_deref() else {
            return Ok(None);
        };
        let Some(rect) = annotation.base.rect.as_ref() else {
            return Ok(None);
        };

        let image_bytes = Self::decode_stamp_image_data(image_data)?;
        let image = image::load_from_memory(&image_bytes)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("解析 stamp 图片失败: {}", e)))?;
        let rgba = image.to_rgba8();
        let (img_width, img_height) = image.dimensions();

        let mut rgb_bytes = Vec::with_capacity((img_width * img_height * 3) as usize);
        let mut alpha_bytes = Vec::with_capacity((img_width * img_height) as usize);
        for pixel in rgba.pixels() {
            rgb_bytes.extend_from_slice(&[pixel[0], pixel[1], pixel[2]]);
            alpha_bytes.push(pixel[3]);
        }

        let smask_id = document.add_object(lopdf::Stream::new(
            lopdf::Dictionary::from_iter(vec![
                ("Type", lopdf::Object::Name(b"XObject".to_vec())),
                ("Subtype", lopdf::Object::Name(b"Image".to_vec())),
                ("Width", lopdf::Object::Integer(img_width as i64)),
                ("Height", lopdf::Object::Integer(img_height as i64)),
                ("ColorSpace", lopdf::Object::Name(b"DeviceGray".to_vec())),
                ("BitsPerComponent", lopdf::Object::Integer(8)),
            ]),
            alpha_bytes,
        ));

        let image_stream = lopdf::Stream::new(
            lopdf::Dictionary::from_iter(vec![
                ("Type", lopdf::Object::Name(b"XObject".to_vec())),
                ("Subtype", lopdf::Object::Name(b"Image".to_vec())),
                ("Width", lopdf::Object::Integer(img_width as i64)),
                ("Height", lopdf::Object::Integer(img_height as i64)),
                ("ColorSpace", lopdf::Object::Name(b"DeviceRGB".to_vec())),
                ("BitsPerComponent", lopdf::Object::Integer(8)),
                ("SMask", lopdf::Object::Reference(smask_id)),
            ]),
            rgb_bytes,
        );
        let image_id = document.add_object(image_stream);

        let width = ((rect.right - rect.left).abs() as f32).max(1.0);
        let height = ((rect.top - rect.bottom).abs() as f32).max(1.0);
        let content = format!("q\n{} 0 0 {} 0 0 cm\n/Im0 Do\nQ\n", width, height);

        let stream_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"XObject".to_vec())),
            ("Subtype", lopdf::Object::Name(b"Form".to_vec())),
            ("BBox", lopdf::Object::Array(vec![
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(width),
                lopdf::Object::Real(height),
            ])),
            ("Resources", lopdf::Object::Dictionary({
                let mut resources = lopdf::Dictionary::new();
                resources.set("XObject", lopdf::Object::Dictionary({
                    let mut xobjects = lopdf::Dictionary::new();
                    xobjects.set("Im0", lopdf::Object::Reference(image_id));
                    xobjects
                }));
                resources
            })),
        ]);

        Ok(Some(lopdf::Object::Stream(lopdf::Stream::new(stream_dict, content.into_bytes()))))
    }
    fn build_square_ap_stream(annotation: &SquareAnnotation) -> Option<lopdf::Object> {
        let rect = annotation.base.rect.as_ref()?;
        let width = ((rect.right - rect.left).abs() as f32).max(1.0);
        let height = ((rect.top - rect.bottom).abs() as f32).max(1.0);
        let line_width = annotation.width.max(1.0);
        let inset = (line_width / 2.0).max(0.5);
        let draw_width = (width - line_width).max(0.0);
        let draw_height = (height - line_width).max(0.0);
        let color = annotation.base.color.as_deref().and_then(Color::from_hex).unwrap_or(Color {
            r: 0.894,
            g: 0.259,
            b: 0.204,
        });

        let content = format!(
            "q\n{} {} {} RG\n{} w\n[] 0 d\n{} {} {} {} re\nS\nQ\n",
            color.r,
            color.g,
            color.b,
            line_width,
            inset,
            inset,
            draw_width,
            draw_height,
        );

        let stream_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"XObject".to_vec())),
            ("Subtype", lopdf::Object::Name(b"Form".to_vec())),
            ("FormType", lopdf::Object::Integer(1)),
            ("BBox", lopdf::Object::Array(vec![
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(width),
                lopdf::Object::Real(height),
            ])),
            ("Matrix", lopdf::Object::Array(vec![
                lopdf::Object::Integer(1),
                lopdf::Object::Integer(0),
                lopdf::Object::Integer(0),
                lopdf::Object::Integer(1),
                lopdf::Object::Integer(0),
                lopdf::Object::Integer(0),
            ])),
            ("Resources", lopdf::Object::Dictionary(lopdf::Dictionary::new())),
        ]);

        Some(lopdf::Object::Stream(lopdf::Stream::new(stream_dict, content.into_bytes())))
    }

    fn build_circle_ap_stream(annotation: &CircleAnnotation) -> Option<lopdf::Object> {
        let rect = annotation.base.rect.as_ref()?;
        let width = ((rect.right - rect.left).abs() as f32).max(1.0);
        let height = ((rect.top - rect.bottom).abs() as f32).max(1.0);
        let line_width = annotation.width.max(1.0);
        let inset = (line_width / 2.0).max(0.5);
        let stroke = annotation.base.color.as_deref().and_then(Color::from_hex).unwrap_or(Color {
            r: 0.894,
            g: 0.259,
            b: 0.204,
        });
        let fill = annotation.interior_color.as_deref().and_then(Color::from_hex);
        let has_fill = fill.is_some();

        let rx = ((width - line_width).max(0.0)) / 2.0;
        let ry = ((height - line_width).max(0.0)) / 2.0;
        let cx = inset + rx;
        let cy = inset + ry;
        let k = 0.552_284_8_f32;
        let ox = rx * k;
        let oy = ry * k;

        let mut content = format!(
            "q\n{} {} {} RG\n{} w\n",
            stroke.r, stroke.g, stroke.b, line_width
        );
        if let Some(fill) = &fill {
            content.push_str(&format!("{} {} {} rg\n", fill.r, fill.g, fill.b));
        }
        content.push_str(&format!(
            "{} {} m\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{}\nQ\n",
            cx + rx, cy,
            cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry,
            cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy,
            cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry,
            cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy,
            if has_fill { "b" } else { "S" }
        ));

        let stream_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"XObject".to_vec())),
            ("Subtype", lopdf::Object::Name(b"Form".to_vec())),
            ("FormType", lopdf::Object::Integer(1)),
            ("BBox", lopdf::Object::Array(vec![
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(width),
                lopdf::Object::Real(height),
            ])),
            ("Resources", lopdf::Object::Dictionary(lopdf::Dictionary::new())),
        ]);

        Some(lopdf::Object::Stream(lopdf::Stream::new(stream_dict, content.into_bytes())))
    }

    fn build_arrowhead_path(tip_x: f32, tip_y: f32, base_x: f32, base_y: f32, size: f32, style: &str) -> Option<String> {
        // 这里专门负责生成线段端点的小图形，比如箭头、方块、圆点。
        // 返回的是一小段 PDF path 命令，最后会拼进 line 的 AP 里。
        let style = style.trim();
        if style.is_empty() || style.eq_ignore_ascii_case("None") {
            return None;
        }

        let dx = tip_x - base_x;
        let dy = tip_y - base_y;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f32::EPSILON {
            return None;
        }

        let ux = dx / length;
        let uy = dy / length;
        let px = -uy;
        let py = ux;
        let arrow_len = size.max(6.0);
        let arrow_half_width = (arrow_len * 0.45).max(3.0);
        let bx = tip_x - ux * arrow_len;
        let by = tip_y - uy * arrow_len;
        let lx = bx + px * arrow_half_width;
        let ly = by + py * arrow_half_width;
        let rx = bx - px * arrow_half_width;
        let ry = by - py * arrow_half_width;

        match style {
            "OpenArrow" => Some(format!(
                "{} {} m\n{} {} l\n{} {} m\n{} {} l\nS\n",
                lx, ly, tip_x, tip_y, rx, ry, tip_x, tip_y
            )),
            "ClosedArrow" => Some(format!(
                "{} {} m\n{} {} l\n{} {} l\nh\nB\n",
                lx, ly, tip_x, tip_y, rx, ry
            )),
            "Square" => {
                let half = arrow_half_width.max(3.0);
                Some(format!(
                    "{} {} {} {} re\nB\n",
                    tip_x - half,
                    tip_y - half,
                    half * 2.0,
                    half * 2.0
                ))
            }
            "Circle" => {
                let r = arrow_half_width.max(3.0);
                let k = 0.552_284_8_f32;
                let ox = r * k;
                let oy = r * k;
                Some(format!(
                    "{} {} m\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\n{} {} {} {} {} {} c\nB\n",
                    tip_x + r, tip_y,
                    tip_x + r, tip_y + oy, tip_x + ox, tip_y + r, tip_x, tip_y + r,
                    tip_x - ox, tip_y + r, tip_x - r, tip_y + oy, tip_x - r, tip_y,
                    tip_x - r, tip_y - oy, tip_x - ox, tip_y - r, tip_x, tip_y - r,
                    tip_x + ox, tip_y - r, tip_x + r, tip_y - oy, tip_x + r, tip_y,
                ))
            }
            _ => Some(format!(
                "{} {} m\n{} {} l\n{} {} m\n{} {} l\nS\n",
                lx, ly, tip_x, tip_y, rx, ry, tip_x, tip_y
            )),
        }
    }

    fn build_line_ap_stream(annotation: &LineAnnotation) -> Option<lopdf::Object> {
        let rect = annotation.base.rect.as_ref()?;
        let width = ((rect.right - rect.left).abs() as f32).max(1.0);
        let height = ((rect.top - rect.bottom).abs() as f32).max(1.0);
        let line_width = annotation.width.max(1.0);
        let stroke = annotation.base.color.as_deref().and_then(Color::from_hex).unwrap_or(Color {
            r: 0.894,
            g: 0.259,
            b: 0.204,
        });
        let start = Self::parse_point(annotation.start.as_deref()?)?;
        let end = Self::parse_point(annotation.end.as_deref()?)?;
        let sx = (start.0 - rect.left) as f32;
        let sy = (start.1 - rect.bottom) as f32;
        let ex = (end.0 - rect.left) as f32;
        let ey = (end.1 - rect.bottom) as f32;

        let mut content = format!(
            "q\n{} {} {} RG\n{} {} {} rg\n{} w\n{} {} m\n{} {} l\nS\n",
            stroke.r, stroke.g, stroke.b, stroke.r, stroke.g, stroke.b, line_width, sx, sy, ex, ey
        );

        if let Some(path) = Self::build_arrowhead_path(sx, sy, ex, ey, line_width * 4.0, &annotation.head_style) {
            content.push_str(&path);
        }
        if let Some(path) = Self::build_arrowhead_path(ex, ey, sx, sy, line_width * 4.0, &annotation.tail_style) {
            content.push_str(&path);
        }
        content.push_str("Q\n");

        let stream_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"XObject".to_vec())),
            ("Subtype", lopdf::Object::Name(b"Form".to_vec())),
            ("FormType", lopdf::Object::Integer(1)),
            ("BBox", lopdf::Object::Array(vec![
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(width),
                lopdf::Object::Real(height),
            ])),
            ("Resources", lopdf::Object::Dictionary(lopdf::Dictionary::new())),
        ]);

        Some(lopdf::Object::Stream(lopdf::Stream::new(stream_dict, content.into_bytes())))
    }

    fn build_polygon_ap_stream(annotation: &PolygonAnnotation) -> Option<lopdf::Object> {
        let rect = annotation.base.rect.as_ref()?;
        let width = ((rect.right - rect.left).abs() as f32).max(1.0);
        let height = ((rect.top - rect.bottom).abs() as f32).max(1.0);
        let stroke = annotation.base.color.as_deref().and_then(Color::from_hex).unwrap_or(Color {
            r: 0.894,
            g: 0.259,
            b: 0.204,
        });
        let coords = annotation.vertices.as_deref()?;
        let raw_values: Vec<f64> = coords.split_whitespace()
            .flat_map(|pair| pair.split(','))
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();
        if raw_values.len() < 4 || raw_values.len() % 2 != 0 {
            return None;
        }

        let mut points = Vec::new();
        for chunk in raw_values.chunks(2) {
            points.push(((chunk[0] - rect.left) as f32, (chunk[1] - rect.bottom) as f32));
        }
        let (first_x, first_y) = points[0];
        let mut path = format!("{} {} m\n", first_x, first_y);
        for (x, y) in points.iter().skip(1) {
            path.push_str(&format!("{} {} l\n", x, y));
        }
        if annotation.is_closed {
            path.push_str("h\n");
        }

        let content = format!(
            "q\n{} {} {} RG\n1 w\n{}{}\nQ\n",
            stroke.r,
            stroke.g,
            stroke.b,
            path,
            if annotation.is_closed { "S" } else { "S" }
        );

        let stream_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"XObject".to_vec())),
            ("Subtype", lopdf::Object::Name(b"Form".to_vec())),
            ("FormType", lopdf::Object::Integer(1)),
            ("BBox", lopdf::Object::Array(vec![
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(width),
                lopdf::Object::Real(height),
            ])),
            ("Resources", lopdf::Object::Dictionary(lopdf::Dictionary::new())),
        ]);

        Some(lopdf::Object::Stream(lopdf::Stream::new(stream_dict, content.into_bytes())))
    }


    fn build_freetext_annotation(
        &self,
        document: &mut lopdf::Document,
        annotation: &FreeTextAnnotation,
    ) -> Result<lopdf::Dictionary> {
        let mut dict = Self::build_base_annotation_dict(&Annotation::FreeText(annotation.clone()));
        if let Some(style) = &annotation.default_style {
            dict.set("DS", lopdf::Object::String(style.clone().into_bytes(), lopdf::StringFormat::Literal));
        }

        let Some(spec) = Self::build_freetext_render_spec(annotation) else {
            return Ok(dict);
        };

        dict.set("DA", lopdf::Object::String(spec.da.as_bytes().to_vec(), lopdf::StringFormat::Literal));
        dict.set("Q", lopdf::Object::Integer(spec.align as i64));
        dict.set("IT", lopdf::Object::Name(b"FreeTextTypeWriter".to_vec()));
        dict.set("RD", lopdf::Object::Array(vec![
            lopdf::Object::Real(0.0),
            lopdf::Object::Real(0.0),
            lopdf::Object::Real(0.0),
            lopdf::Object::Real(0.0),
        ]));

        let ap_stream = if spec.is_cjk {
            Self::build_cjk_ap_stream(&spec, annotation.base.rect.as_ref())?
        } else {
            Self::build_text_ap_stream(&spec, annotation.base.rect.as_ref())
        };
        let ap_stream_id = document.add_object(ap_stream);
        dict.set("AP", lopdf::Object::Dictionary({
            let mut ap_dict = lopdf::Dictionary::new();
            ap_dict.set("N", lopdf::Object::Reference(ap_stream_id));
            ap_dict
        }));

        Ok(dict)
    }

    pub fn export_to_new_pdf(&mut self, xfdf_doc: &XfdfDocument, output_path: &Path) -> Result<()> {
        // 这种模式下，不依赖外部已有 PDF，
        // 而是直接创建一个新的 PDF，把每一页的注释放进去。
        info!("创建新 PDF，共 {} 页", xfdf_doc.total_pages());

        let mut document = lopdf::Document::with_version("1.5");
        let font_name = "BFont";

        let mut font_dict = lopdf::Dictionary::new();
        font_dict.set("Type", lopdf::Object::Name(b"Font".to_vec()));
        font_dict.set("Subtype", lopdf::Object::Name(b"Type1".to_vec()));
        font_dict.set("BaseFont", lopdf::Object::Name(b"Helvetica".to_vec()));
        let font_id_ref = document.add_object(font_dict);

        let total_pages = xfdf_doc.total_pages();
        let mut page_ids = Vec::new();

        for page_num in 0..total_pages {
            info!("处理第 {} 页", page_num + 1);
            let page_annotations = xfdf_doc.get_annotations_for_page(page_num);
            let page_id = self.create_page(&mut document, font_name, font_id_ref, &page_annotations, page_num)?;
            page_ids.push(page_id);
        }

        self.setup_document_structure(&mut document, &page_ids)?;

        document.save(output_path)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("保存PDF失败: {}", e)))?;

        Ok(())
    }

    pub fn export_to_existing_pdf(&mut self, xfdf_doc: &XfdfDocument, input_path: &Path, output_path: &Path) -> Result<()> {
        // 这种模式不是新建空白 PDF，
        // 而是在已有 PDF 基础上，把 XFDF 里的注释挂到对应页面上。
        info!("打开现有 PDF 并合并注释");

        let mut document = lopdf::Document::load(input_path)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("加载PDF失败: {}", e)))?;

        let pages = document.get_pages();
        let existing_page_count = pages.len();
        info!("原始 PDF 共 {} 页", existing_page_count);

        let total_pages = std::cmp::max(xfdf_doc.total_pages(), existing_page_count);
        let mut font_dict = lopdf::Dictionary::new();
        font_dict.set("Type", lopdf::Object::Name(b"Font".to_vec()));
        font_dict.set("Subtype", lopdf::Object::Name(b"Type1".to_vec()));
        font_dict.set("BaseFont", lopdf::Object::Name(b"Helvetica".to_vec()));
        let font_id_ref = document.add_object(font_dict);

        for page_num in 0..total_pages {
            if page_num < existing_page_count && !xfdf_doc.get_annotations_for_page(page_num).is_empty() {
                let page_id = pages[&((page_num + 1) as u32)];
                let page_annotations = xfdf_doc.get_annotations_for_page(page_num);
                self.add_annotations_to_page(&mut document, page_id, &page_annotations, font_id_ref)?;
                debug!("向第 {} 页添加了 {} 条注释", page_num + 1, page_annotations.len());
            }
        }

        document.save(output_path)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("保存PDF失败: {}", e)))?;

        Ok(())
    }

    fn create_page(&self, document: &mut lopdf::Document, _font_name: &str, font_id: lopdf::ObjectId,
                     annotations: &[&Annotation], page_number: usize) -> Result<lopdf::ObjectId> {
        let content_stream = format!(
            "BT\n/F1 12 Tf\n50 {} Td\n(Annotations - Page {}) Tj\nET\n",
            self.page_size.1 as i32 - 50, page_number + 1
        );

        let content_id = document.add_object(lopdf::Stream::new(
            lopdf::Dictionary::new(),
            content_stream.as_bytes().to_vec(),
        ));

        let annot_ids = self.create_annotation_objects(document, annotations)?;
        let annots_array = lopdf::Object::Array(annot_ids.iter().map(|id| lopdf::Object::Reference(*id)).collect::<Vec<_>>());

        let mut page_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"Page".to_vec())),
            ("Parent", lopdf::Object::Null),
            ("MediaBox", lopdf::Object::Array(vec![
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(0.0),
                lopdf::Object::Real(self.page_size.0 as f32),
                lopdf::Object::Real(self.page_size.1 as f32),
            ])),
            ("Contents", lopdf::Object::Reference(content_id)),
            ("Resources", lopdf::Object::Dictionary({
                let mut r = lopdf::Dictionary::new();
                r.set("Font", lopdf::Dictionary::from_iter(vec![
                    ("F1", lopdf::Object::Reference(font_id))
                ]));
                r
            })),
        ]);

        if !annot_ids.is_empty() {
            page_dict.set("Annots", annots_array);
        }

        Ok(document.add_object(page_dict))
    }

    fn create_annotation_objects(&self, document: &mut lopdf::Document, annotations: &[&Annotation]) -> Result<Vec<lopdf::ObjectId>> {
        let mut ids = Vec::new();
        for annotation in annotations {
            let annot_dict = self.build_annotation(document, annotation)?;
            ids.push(document.add_object(annot_dict));
        }
        Ok(ids)
    }

    fn build_annotation(&self, document: &mut lopdf::Document, annotation: &Annotation) -> Result<lopdf::Dictionary> {
        // 这里是“把一种注释翻译成 PDF 字典”的总入口。
        // 前面解析器已经告诉我们：这是一条 text / square / line / polygon ...
        // 现在要决定：PDF 里该写哪些字段，是否要补 AP，要不要带边框、坐标、文字等。
        let mut dict = Self::build_base_annotation_dict(annotation);

        match annotation {
            Annotation::Text(t) => {
                if t.open {
                    dict.set("Open", lopdf::Object::Boolean(true));
                }
                dict.set("Name", lopdf::Object::Name(t.icon_type.clone().into_bytes()));
            }
            Annotation::Highlight(h) => {
                if let Some(ref coords) = h.coords {
                    if let Some(qp) = Self::parse_quadpoints(coords) {
                        dict.set("QuadPoints", qp);
                    }
                }
            }
            Annotation::Underline(u) => {
                if let Some(ref coords) = u.coords {
                    if let Some(qp) = Self::parse_quadpoints(coords) {
                        dict.set("QuadPoints", qp);
                    }
                }
            }
            Annotation::StrikeOut(s) => {
                if let Some(ref coords) = s.coords {
                    if let Some(qp) = Self::parse_quadpoints(coords) {
                        dict.set("QuadPoints", qp);
                    }
                }
            }
            Annotation::Squiggly(sq) => {
                if let Some(ref coords) = sq.coords {
                    if let Some(qp) = Self::parse_quadpoints(coords) {
                        dict.set("QuadPoints", qp);
                    }
                }
            }
            Annotation::FreeText(f) => {
                dict = self.build_freetext_annotation(document, f)?;
            }
            Annotation::Square(sq) => {
                // square 不只是写“这是个矩形注释”，
                // 还会尽量补显式 AP，让阅读器直接看到我们画好的外观。
                let line_width = sq.width.max(1.0);
                dict.set("BS", lopdf::Dictionary::from_iter(vec![
                    ("W", lopdf::Object::Real(line_width)),
                    ("S", lopdf::Object::Name(b"S".to_vec())),
                ]));
                dict.set("Border", lopdf::Object::Array(vec![
                    lopdf::Object::Integer(0),
                    lopdf::Object::Integer(0),
                    lopdf::Object::Real(line_width),
                ]));
                dict.set("RD", lopdf::Object::Array(vec![
                    lopdf::Object::Real(0.0),
                    lopdf::Object::Real(0.0),
                    lopdf::Object::Real(0.0),
                    lopdf::Object::Real(0.0),
                ]));
                if let Some(ap_stream) = Self::build_square_ap_stream(sq) {
                    let ap_stream_id = document.add_object(ap_stream);
                    dict.set("AP", lopdf::Object::Dictionary({
                        let mut ap_dict = lopdf::Dictionary::new();
                        ap_dict.set("N", lopdf::Object::Reference(ap_stream_id));
                        ap_dict
                    }));
                }
            }
            Annotation::Circle(c) => {
                // circle 的思路和 square 类似：
                // 除了基础字典字段，还会补边框、填充色和显式 AP。
                let line_width = c.width.max(1.0);
                dict.set("BS", lopdf::Dictionary::from_iter(vec![
                    ("W", lopdf::Object::Real(line_width)),
                    ("S", lopdf::Object::Name(b"S".to_vec())),
                ]));
                if let Some(ic) = &c.interior_color {
                    if let Some(clr) = Color::from_hex(ic) {
                        dict.set("IC", lopdf::Object::Array(vec![
                            lopdf::Object::Real(clr.r),
                            lopdf::Object::Real(clr.g),
                            lopdf::Object::Real(clr.b),
                        ]));
                    }
                }
                if let Some(ap_stream) = Self::build_circle_ap_stream(c) {
                    let ap_stream_id = document.add_object(ap_stream);
                    dict.set("AP", lopdf::Object::Dictionary({
                        let mut ap_dict = lopdf::Dictionary::new();
                        ap_dict.set("N", lopdf::Object::Reference(ap_stream_id));
                        ap_dict
                    }));
                }
            }
            Annotation::Line(l) => {
                // line 需要的关键数据是起点、终点，以及两端样式。
                // 如果补了显式 AP，线段和箭头在不同阅读器里会更稳定。
                let line_width = l.width.max(1.0);
                if let (Some(start_str), Some(end_str)) = (&l.start, &l.end) {
                    if let (Some(start), Some(end)) = (Self::parse_point(start_str), Self::parse_point(end_str)) {
                        dict.set("L", lopdf::Object::Array(vec![
                            lopdf::Object::Real(start.0 as f32), lopdf::Object::Real(start.1 as f32),
                            lopdf::Object::Real(end.0 as f32), lopdf::Object::Real(end.1 as f32),
                        ]));
                    }
                }
                dict.set("BS", lopdf::Dictionary::from_iter(vec![
                    ("W", lopdf::Object::Real(line_width)),
                    ("S", lopdf::Object::Name(b"S".to_vec())),
                ]));
                if !l.head_style.is_empty() || !l.tail_style.is_empty() {
                    dict.set("LE", lopdf::Object::Array(vec![
                        lopdf::Object::Name(if l.head_style.is_empty() { b"None".to_vec() } else { l.head_style.as_bytes().to_vec() }),
                        lopdf::Object::Name(if l.tail_style.is_empty() { b"None".to_vec() } else { l.tail_style.as_bytes().to_vec() }),
                    ]));
                }
                if let Some(ap_stream) = Self::build_line_ap_stream(l) {
                    let ap_stream_id = document.add_object(ap_stream);
                    dict.set("AP", lopdf::Object::Dictionary({
                        let mut ap_dict = lopdf::Dictionary::new();
                        ap_dict.set("N", lopdf::Object::Reference(ap_stream_id));
                        ap_dict
                    }));
                }
            }
            Annotation::Polygon(p) => {
                // polygon / polyline 的核心数据是顶点列表。
                // closed=true 时按封闭图形处理，否则按折线处理。
                if let Some(ref vertices_str) = p.vertices {
                    if let Some(verts) = Self::parse_vertices(vertices_str) {
                        dict.set("Vertices", verts);
                    }
                }
                if let Some(ap_stream) = Self::build_polygon_ap_stream(p) {
                    let ap_stream_id = document.add_object(ap_stream);
                    dict.set("AP", lopdf::Object::Dictionary({
                        let mut ap_dict = lopdf::Dictionary::new();
                        ap_dict.set("N", lopdf::Object::Reference(ap_stream_id));
                        ap_dict
                    }));
                }
            }
            Annotation::Ink(ink) => {
                if !ink.ink_list.is_empty() {
                    let inklist_arrays: Vec<lopdf::Object> = ink.ink_list.iter()
                        .filter_map(|gesture| Self::parse_ink_gesture(gesture))
                        .map(lopdf::Object::Array)
                        .collect();
                    if !inklist_arrays.is_empty() {
                        dict.set("InkList", lopdf::Object::Array(inklist_arrays));
                    }
                }
                dict.set("BS", lopdf::Dictionary::from_iter(vec![
                    ("W", lopdf::Object::Real(ink.width)),
                    ("S", lopdf::Object::Name(b"S".to_vec())),
                ]));
            }
            Annotation::Stamp(s) => {
                if s.icon.is_empty() {
                    dict.set("Name", lopdf::Object::Name(b"Approved".to_vec()));
                } else {
                    dict.set("Name", lopdf::Object::Name(s.icon.clone().into_bytes()));
                }
                if let Some(ap_stream) = self.build_stamp_ap_stream(document, s)? {
                    let ap_stream_id = document.add_object(ap_stream);
                    dict.set("AP", lopdf::Object::Dictionary({
                        let mut ap_dict = lopdf::Dictionary::new();
                        ap_dict.set("N", lopdf::Object::Reference(ap_stream_id));
                        ap_dict
                    }));
                }
            }
            Annotation::Popup(p) => {
                if p.open {
                    dict.set("Open", lopdf::Object::Boolean(true));
                }
            }
        }

        Ok(dict)
    }

    fn parse_quadpoints(coords: &str) -> Option<lopdf::Object> {
        let parts: Vec<&str> = coords.split(',').collect();
        if parts.len() < 8 || parts.len() % 8 != 0 {
            warn!("无效的 QuadPoints 坐标数: {} (需要是8的倍数)", parts.len());
            return None;
        }
        let values: Vec<lopdf::Object> = parts.iter()
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .map(|v| lopdf::Object::Real(v as f32))
            .collect();
        if values.len() == parts.len() {
            Some(lopdf::Object::Array(values))
        } else {
            None
        }
    }

    fn parse_point(point_str: &str) -> Option<(f64, f64)> {
        let parts: Vec<&str> = point_str.split(',').collect();
        if parts.len() != 2 { return None; }
        let x = parts[0].trim().parse::<f64>().ok()?;
        let y = parts[1].trim().parse::<f64>().ok()?;
        Some((x, y))
    }

    fn parse_vertices(vertices_str: &str) -> Option<lopdf::Object> {
        let values: Vec<lopdf::Object> = vertices_str.split_whitespace()
            .flat_map(|pair| pair.split(','))
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .map(|v| lopdf::Object::Real(v as f32))
            .collect();
        if values.is_empty() || values.len() % 2 != 0 { return None; }
        Some(lopdf::Object::Array(values))
    }

    fn parse_ink_gesture(gesture: &str) -> Option<Vec<lopdf::Object>> {
        let points: Vec<lopdf::Object> = gesture.split(';')
            .filter_map(|pair| {
                let parts: Vec<&str> = pair.split(',').collect();
                if parts.len() >= 2 {
                    let x = parts[0].trim().parse::<f64>().ok()?;
                    let y = parts[1].trim().parse::<f64>().ok()?;
                    Some(vec![lopdf::Object::Real(x as f32), lopdf::Object::Real(y as f32)])
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        if points.is_empty() { None } else { Some(points) }
    }

    fn add_annotations_to_page(&self, document: &mut lopdf::Document, page_id: lopdf::ObjectId,
                              annotations: &[&Annotation], _font_id: lopdf::ObjectId) -> Result<()> {
        let new_ids = self.create_annotation_objects(document, annotations)?;
        let new_refs: Vec<lopdf::Object> = new_ids.iter().map(|id| lopdf::Object::Reference(*id)).collect();

        for annot_id in &new_ids {
            if let Some(obj) = document.objects.get_mut(&(annot_id.0, annot_id.1)) {
                if let lopdf::Object::Dictionary(ref mut dict) = obj {
                    dict.set("P", lopdf::Object::Reference(page_id));
                }
            }
        }

        let page_dict = document.get_object(page_id)?.as_dict()
            .map_err(|e| PdfXmlError::PdfProcessing(format!("无效页面对象: {}", e)))?;

        let new_annots = if let Ok(arr) = page_dict.get(b"Annots").and_then(|o| o.as_array()) {
            let mut v = arr.clone();
            v.extend(new_refs);
            v
        } else {
            new_refs
        };

        if let Some(obj) = document.objects.get_mut(&(page_id.0, page_id.1)) {
            if let lopdf::Object::Dictionary(ref mut dict) = obj {
                dict.set("Annots", lopdf::Object::Array(new_annots));
            }
        }

        Ok(())
    }

    fn setup_document_structure(&self, document: &mut lopdf::Document, page_ids: &[lopdf::ObjectId]) -> Result<()> {
        let kids: Vec<lopdf::Object> = page_ids.iter().map(|id| lopdf::Object::Reference(*id)).collect();
        let pg_dict = lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"Pages".to_vec())),
            ("Kids", lopdf::Object::Array(kids)),
            ("Count", lopdf::Object::Integer(page_ids.len() as i64)),
        ]);
        let pg_id = document.add_object(pg_dict);

        for id in page_ids {
            if let Some(obj) = document.objects.get_mut(&(id.0, id.1)) {
                if let lopdf::Object::Dictionary(ref mut dict) = obj {
                    dict.set("Parent", lopdf::Object::Reference(pg_id));
                }
            }
        }

        let catalog_id = document.add_object(lopdf::Dictionary::from_iter(vec![
            ("Type", lopdf::Object::Name(b"Catalog".to_vec())),
            ("Pages", lopdf::Object::Reference(pg_id)),
        ]));
        document.trailer.set("Root", lopdf::Object::Reference(catalog_id));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use super::*;

    #[test]
    fn test_contains_chinese() {
        assert!(contains_chinese("Hello 世界"));
        assert!(!contains_chinese("Hello World"));
    }

    #[test]
    fn test_to_utf16be() {
        let bytes = to_utf16be("你好");
        assert_eq!(bytes[0], 0xFE);
        assert_eq!(bytes[1], 0xFF);
        assert_eq!(bytes.len(), 6);
    }

    #[test]
    fn test_escape_pdf_literal_string() {
        assert_eq!(escape_pdf_literal_string("(a)\\b"), "\\(a\\)\\\\b");
    }

    #[test]
    fn test_quadratic_curve_conversion_uses_current_point() {
        let mut builder = GlyphPathBuilder::new();
        builder.move_to(10.0, 20.0);
        builder.quad_to(16.0, 26.0, 30.0, 40.0);
        assert!(builder.path.contains("14 24 20.666666 30.666666 30 40 c"));
    }

    #[test]
    fn test_circle_annotation_should_emit_explicit_ap() {
        let circle = CircleAnnotation {
            base: AnnotationBase {
                name: None,
                page: 0,
                rect: Some(Rect {
                    left: 100.0,
                    bottom: 100.0,
                    right: 180.0,
                    top: 160.0,
                }),
                title: None,
                subject: None,
                contents: None,
                creation_date: None,
                modification_date: None,
                color: Some("#E44234".to_string()),
                opacity: 1.0,
                flags: 0,
                extra: HashMap::new(),
            },
            width: 2.0,
            interior_color: Some("#FFF2CC".to_string()),
        };

        let mut document = lopdf::Document::with_version("1.5");
        let exporter = PdfAnnotationExporter::new();
        let dict = exporter.build_annotation(&mut document, &Annotation::Circle(circle)).unwrap();

        assert!(dict.get(b"AP").is_ok(), "circle 应补显式 AP");
    }

    #[test]
    fn test_line_annotation_should_emit_explicit_ap() {
        let line = LineAnnotation {
            base: AnnotationBase {
                name: None,
                page: 0,
                rect: Some(Rect {
                    left: 100.0,
                    bottom: 100.0,
                    right: 220.0,
                    top: 180.0,
                }),
                title: None,
                subject: None,
                contents: None,
                creation_date: None,
                modification_date: None,
                color: Some("#E44234".to_string()),
                opacity: 1.0,
                flags: 0,
                extra: HashMap::new(),
            },
            start: Some("110,110".to_string()),
            end: Some("210,170".to_string()),
            head_style: "OpenArrow".to_string(),
            tail_style: "ClosedArrow".to_string(),
            width: 2.0,
        };

        let mut document = lopdf::Document::with_version("1.5");
        let exporter = PdfAnnotationExporter::new();
        let dict = exporter.build_annotation(&mut document, &Annotation::Line(line.clone())).unwrap();

        assert!(dict.get(b"AP").is_ok(), "line 应补显式 AP");
        let ap = PdfAnnotationExporter::build_line_ap_stream(&line).unwrap();
        let stream = ap.as_stream().unwrap();
        let content = String::from_utf8_lossy(&stream.content);
        assert!(content.contains("B\n") || content.matches("S\n").count() >= 2, "line 端点应写入 AP 图形");
    }

    #[test]
    fn test_polygon_annotation_should_emit_explicit_ap() {
        let polygon = PolygonAnnotation {
            base: AnnotationBase {
                name: None,
                page: 0,
                rect: Some(Rect {
                    left: 100.0,
                    bottom: 100.0,
                    right: 220.0,
                    top: 220.0,
                }),
                title: None,
                subject: None,
                contents: None,
                creation_date: None,
                modification_date: None,
                color: Some("#E44234".to_string()),
                opacity: 1.0,
                flags: 0,
                extra: HashMap::new(),
            },
            vertices: Some("110,110 210,120 180,210 120,200".to_string()),
            is_closed: true,
        };

        let mut document = lopdf::Document::with_version("1.5");
        let exporter = PdfAnnotationExporter::new();
        let dict = exporter.build_annotation(&mut document, &Annotation::Polygon(polygon)).unwrap();

        assert!(dict.get(b"AP").is_ok(), "polygon 应补显式 AP");
        assert_eq!(dict.get(b"Subtype").unwrap().as_name().unwrap(), b"Polygon");
    }

    #[test]
    fn test_polyline_xfdf_should_emit_polyline_ap() {
        let xml = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>",
            "<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">",
            "  <annots>",
            "    <polyline page=\"0\" rect=\"100,100,220,220\" color=\"#E44234\" vertices=\"110,110 210,120 180,210\"/>",
            "  </annots>",
            "</xfdf>"
        );

        let doc = XfdfDocument::parse(xml).unwrap();
        assert_eq!(doc.annotations.len(), 1);

        let mut document = lopdf::Document::with_version("1.5");
        let exporter = PdfAnnotationExporter::new();
        let dict = exporter.build_annotation(&mut document, &doc.annotations[0]).unwrap();

        assert!(dict.get(b"AP").is_ok(), "polyline 应补显式 AP");
        assert_eq!(dict.get(b"Subtype").unwrap().as_name().unwrap(), b"PolyLine");
    }
}
