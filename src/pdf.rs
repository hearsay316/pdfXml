//! 这个文件负责真正和 PDF 打交道。
//!
//! 前面的模块主要是在整理数据：
//! - `xfdf.rs` 负责把 XFDF 读成 Rust 结构
//! - `annotation.rs` 负责定义这些结构长什么样
//!
//! 而这里负责的是最后一步：
//! - 把注释写进 PDF
//! - 从 PDF 里把注释读回来
//! - 为某些注释补上外观流，让不同阅读器显示得更稳定
//!
//! 这个文件比较长，建议按下面顺序阅读：
//! 1. 先看最前面的字符串、颜色、字体小工具
//! 2. 再看 FreeText 和各种图形的 AP 外观流怎么生成
//! 3. 再看“从 PDF 读回注释”的那一段
//! 4. 最后看“导出到新 PDF / 合并到现有 PDF”的主流程

use base64::Engine;
use crate::annotation::*;
use crate::error::{PdfXmlError, Result};
use crate::xfdf::XfdfDocument;
use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use log::{debug, info, warn};
use lopdf;
use std::collections::{hash_map::Entry, HashMap};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use ttf_parser::{Face, OutlineBuilder};

// 这个文件是项目里最“动手干活”的一层。
// 前面的模块只是把数据整理好，
// 真正把注释写进 PDF、或者从 PDF 里读回注释，都是这里负责。
//
// 这个文件比较长，建议按下面顺序阅读：
// 1. 先看最前面的字符串、颜色、字体小工具
// 2. 再看 FreeText 和各种图形的 AP 外观流怎么生成
// 3. 再看“从 PDF 读回注释”的那一段
// 4. 最后看“导出到新 PDF / 合并到现有 PDF”的主流程

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
    let fill_re = regex::Regex::new(r"(?:\d*\.?\d+\s+){2}\d*\.?\d+\s+rg").unwrap();
    let stroke_re = regex::Regex::new(r"(?:\d*\.?\d+\s+){2}\d*\.?\d+\s+RG").unwrap();
    let replacement_rg = format!("{} {} {} rg", color.r, color.g, color.b);
    let replacement_rg_stroke = format!("{} {} {} RG", color.r, color.g, color.b);

    let with_fill = if fill_re.is_match(trimmed) {
        fill_re.replace_all(trimmed, replacement_rg.as_str()).to_string()
    } else if trimmed.is_empty() {
        replacement_rg.clone()
    } else {
        format!("{} {}", trimmed, replacement_rg)
    };

    if stroke_re.is_match(&with_fill) {
        stroke_re.replace_all(&with_fill, replacement_rg_stroke.as_str()).to_string()
    } else {
        format!("{} {}", replacement_rg_stroke, with_fill)
    }
}

fn parse_da_color_captures(caps: &regex::Captures) -> Option<(f32, f32, f32)> {
    Some((
        caps.get(1)?.as_str().parse().ok()?,
        caps.get(2)?.as_str().parse().ok()?,
        caps.get(3)?.as_str().parse().ok()?,
    ))
}

fn extract_fill_color_from_da(da: &str) -> (f32, f32, f32) {
    let fill_re = regex::Regex::new(r"(\d*\.?\d+)\s+(\d*\.?\d+)\s+(\d*\.?\d+)\s+rg").unwrap();
    if let Some(caps) = fill_re.captures(da) {
        if let Some(color) = parse_da_color_captures(&caps) {
            return color;
        }
    }

    let stroke_re = regex::Regex::new(r"(\d*\.?\d+)\s+(\d*\.?\d+)\s+(\d*\.?\d+)\s+RG").unwrap();
    stroke_re
        .captures(da)
        .and_then(|caps| parse_da_color_captures(&caps))
        .unwrap_or((0.0, 0.0, 0.0))
}

fn parse_text_color_from_da_hex(da: &str) -> Option<String> {
    let (r, g, b) = extract_fill_color_from_da(da);
    if r == 0.0 && g == 0.0 && b == 0.0 {
        let fill_re = regex::Regex::new(r"(\d*\.?\d+)\s+(\d*\.?\d+)\s+(\d*\.?\d+)\s+r[gG]").unwrap();
        fill_re.captures(da)?;
    }

    Some(format!(
        "#{:02X}{:02X}{:02X}",
        (r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (b.clamp(0.0, 1.0) * 255.0).round() as u8,
    ))
}

fn parse_font_size_from_da(da: &str) -> f32 {
    let tf_re = regex::Regex::new(r"/[A-Za-z][A-Za-z0-9_-]*\s+(\d+(?:\.\d+)?)\s*Tf").unwrap();
    tf_re
        .captures(da)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<f32>().ok())
        .filter(|size| *size > 0.0)
        .unwrap_or(12.0)
}

fn parse_font_resource_name_from_da(da: &str) -> Option<String> {
    // 这一步是在 DA 字符串里找“字体的名字”。
    // 例如 `/Courier 14 Tf` 里，我们要拿到的就是 `Courier`。
    // 后面生成 AP 外观流时，必须用同一个名字，
    // 不然有些 PDF 阅读器会显示成另一种默认字体。
    let tf_re = regex::Regex::new(r"/([^\s/]+)\s+\d+(?:\.\d+)?\s*Tf").unwrap();
    tf_re
        .captures(da)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

fn normalize_base_font_name(font_resource: &str) -> String {
    // 有些名字是缩写。
    // 比如 `Helv` 实际表示 `Helvetica`。
    // PDF 里“资源名”和“真正的字体名”可以不是同一个东西，
    // 所以这里把常见缩写翻译成正式名字。
    match font_resource {
        "Helv" => "Helvetica".to_string(),
        "HeBo" => "Helvetica-Bold".to_string(),
        "HeOb" => "Helvetica-Oblique".to_string(),
        "HeBO" => "Helvetica-BoldOblique".to_string(),
        "Cour" => "Courier".to_string(),
        "CoBo" => "Courier-Bold".to_string(),
        "CoOb" => "Courier-Oblique".to_string(),
        "CoBO" => "Courier-BoldOblique".to_string(),
        "TiRo" => "Times-Roman".to_string(),
        "TiBo" => "Times-Bold".to_string(),
        "TiIt" => "Times-Italic".to_string(),
        "TiBI" => "Times-BoldItalic".to_string(),
        "Symb" => "Symbol".to_string(),
        "ZaDb" => "ZapfDingbats".to_string(),
        other => other.to_string(),
    }
}

fn parse_alignment_from_style(style: Option<&str>) -> i32 {
    let Some(style) = style else { return 0; };

    for segment in style.split(';') {
        let mut parts = segment.splitn(2, ':');
        let property = parts.next().map(|s| s.trim().to_ascii_lowercase());
        let value = parts.next().map(|s| s.trim().to_ascii_lowercase());

        if property.as_deref() == Some("text-align") {
            return match value.as_deref() {
                Some("center") => 1,
                Some("right") => 2,
                _ => 0,
            };
        }
    }

    0
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

impl Default for PdfAnnotationExporter {
    fn default() -> Self {
        Self::new()
    }
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
        // 先尊重外部显式指定的字体路径，再回退到各平台常见位置。
        let mut paths = Vec::new();

        if let Ok(path) = std::env::var("PDFXML_CJK_FONT") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                paths.push(PathBuf::from(trimmed));
            }
        }

        paths.extend([
            PathBuf::from("C:/Windows/Fonts/simsun.ttc"),
            PathBuf::from("C:/Windows/Fonts/msyh.ttc"),
            PathBuf::from("C:/Windows/Fonts/simhei.ttf"),
            PathBuf::from("C:/Windows/Fonts/simkai.ttf"),
            PathBuf::from("C:/Windows/Fonts/simsunb.ttf"),
            PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
            PathBuf::from("/System/Library/Fonts/Hiragino Sans GB.ttc"),
            PathBuf::from("/System/Library/Fonts/STHeiti Light.ttc"),
            PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
            PathBuf::from("/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttf"),
            PathBuf::from("/usr/share/fonts/truetype/arphic/ukai.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/arphic/uming.ttc"),
        ]);

        paths
    }

    fn load_chinese_font_bytes() -> Result<(PathBuf, Vec<u8>)> {
        // 找到第一份可用中文字体后就直接读取。
        // 后面的中文 FreeText 外观流会拿它的字形轮廓来画字。
        for path in Self::candidate_chinese_font_paths() {
            if path.exists() {
                info!("使用中文字体轮廓: {:?}", path);
                let bytes = fs::read(&path)
                    .map_err(|e| PdfXmlError::PdfProcessing(format!("读取字体失败 {:?}: {}", path, e)))?;
                return Ok((path, bytes));
            }
        }
        Err(PdfXmlError::PdfProcessing(
            "未找到可用的中文字体文件；可通过环境变量 PDFXML_CJK_FONT 指定字体路径".to_string(),
        ))
    }

    fn parse_chinese_face<'a>(font_path: &Path, bytes: &'a [u8]) -> Result<Face<'a>> {
        // TTC/OTC 这类集合字体不能假设目标字库永远在 face 0。
        // 这里会依次尝试多个索引，提高跨平台兼容性。
        let max_faces = if font_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "ttc" | "otc"))
            .unwrap_or(false)
        {
            8
        } else {
            1
        };

        let mut last_error = None;
        for index in 0..max_faces {
            match Face::parse(bytes, index) {
                Ok(face) => return Ok(face),
                Err(err) => last_error = Some((index, err)),
            }
        }

        match last_error {
            Some((index, err)) => Err(PdfXmlError::PdfProcessing(format!(
                "解析中文字体失败 {:?} (face index {}): {}",
                font_path, index, err
            ))),
            None => Err(PdfXmlError::PdfProcessing(format!(
                "解析中文字体失败 {:?}",
                font_path
            ))),
        }
    }

    fn build_freetext_render_spec(annotation: &FreeTextAnnotation) -> Option<FreeTextRenderSpec> {
        // 这个函数先把 FreeText 需要的关键信息整理好，
        // 可以把它理解成“先准备好绘图说明书”。
        // 后面真正生成 PDF 外观流时，就只管照着这份说明书去画。
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
            // 这里的 da 已经是最后要使用的版本，
            // 里面的字体、字号、颜色都已经按规则处理过了。
            da: apply_text_color_to_da(&base_da, preferred_text_color),
            align,
            is_cjk,
        })
    }
    // 这里专门负责“公共字段”。
    // 也就是说，只要大多数注释都会有的字段，
    // 比如位置、名字、内容、颜色、日期，就统一在这里写进 PDF。
    fn set_common_annotation_fields(dict: &mut lopdf::Dictionary, base: &AnnotationBase) {
        // 这里专门负责“公共字段”。
        // 也就是说，只要大多数注释都会有的字段，
        // 比如位置、名字、内容、颜色、日期，就统一在这里写进 PDF。
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

        // `name` 是这条注释自己的名字。
        // 在 PDF 里，它要写到 `NM` 字段里。
        // 之后如果有 Popup 要找到它，或者我们想把 PDF 再导回 XFDF，
        // 都要靠这个名字，所以这里不能省略。
        if let Some(name) = &base.name {
            if contains_chinese(name) {
                dict.set("NM", lopdf::Object::String(to_utf16be(name), lopdf::StringFormat::Hexadecimal));
            } else {
                dict.set("NM", lopdf::Object::String(name.clone().into_bytes(), lopdf::StringFormat::Literal));
            }
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
        // XFDF 的 opacity 对应 PDF 注释字典里的 CA。
        // `opacity` 就是不透明度。
        // 1.0 表示完全不透明，0.4 表示比较透明。
        // PDF 里保存它的字段叫 `CA`。
        // 如果这里不写，之后从 PDF 读回来时就只会看到默认值 1.0。
        if (base.opacity - 1.0).abs() > f32::EPSILON {
            dict.set("CA", lopdf::Object::Real(base.opacity.clamp(0.0, 1.0)));
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
    // AP 可以理解成“这条注释真正画出来长什么样”。
    // 对普通西文文字来说，这里做的事情就是：
    // 1. 算出一个框
    // 2. 选字体和颜色
    // 3. 把文字画进这个框里
    fn build_text_ap_stream(spec: &FreeTextRenderSpec, rect: Option<&Rect>) -> lopdf::Object {
        // AP 可以理解成“这条注释真正画出来长什么样”。
        // 对普通西文文字来说，这里做的事情就是：
        // 1. 算出一个框
        // 2. 选字体和颜色
        // 3. 把文字画进这个框里
        // 这是“普通西文文本”的 AP 生成方式。
        // 思路比较直接：算出一个框，在里面用 Helvetica 把文字写进去。
        let font_size = parse_font_size_from_da(&spec.da);
        // 让 AP 里的字体资源名和 DA 保持一致，避免阅读器优先显示 AP 时字体跑偏。
        // 这里故意让 AP 里使用的字体名字和 DA 里写的一样。
        // 很多 PDF 阅读器显示 FreeText 时，会优先看 AP。
        // 如果 AP 里的字体和 DA 不一样，最后显示出来的字就会跑偏。
        let font_resource = parse_font_resource_name_from_da(&spec.da)
            .unwrap_or_else(|| "Helvetica".to_string());
        let base_font = normalize_base_font_name(&font_resource);
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
            "q\n{} {} {} rg\nBT\n/{} {} Tf\n1 0 0 1 {} {} Tm\n({}) Tj\nET\nQ\n",
            r, g, b, font_resource, font_size, x, (height - font_size).max(2.0), escaped
        );

        let mut helv = lopdf::Dictionary::new();
        helv.set("Type", lopdf::Object::Name(b"Font".to_vec()));
        helv.set("Subtype", lopdf::Object::Name(b"Type1".to_vec()));
        helv.set("BaseFont", lopdf::Object::Name(base_font.into_bytes()));

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
                    fonts.set(font_resource.as_bytes(), lopdf::Object::Dictionary(helv));
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
        let (font_path, font_bytes) = Self::load_chinese_font_bytes()?;
        let face = Self::parse_chinese_face(&font_path, &font_bytes)?;
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
        if raw_values.len() < 4 || !raw_values.len().is_multiple_of(2) {
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
            "q\n{} {} {} RG\n1 w\n{}S\nQ\n",
            stroke.r,
            stroke.g,
            stroke.b,
            path
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

    pub fn load_annotations_from_pdf(&mut self, input_path: &Path) -> Result<XfdfDocument> {
        let document = lopdf::Document::load(input_path)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("加载PDF失败: {}", e)))?;

        let mut xfdf_doc = XfdfDocument {
            xmlns: Some("http://ns.adobe.com/xfdf/".to_string()),
            fields: Vec::new(),
            annotations: Vec::new(),
            metadata: HashMap::new(),
        };

        for (page_number, page_id) in document.get_pages() {
            let page_annotations = Self::read_annotations_from_page(&document, page_id, (page_number - 1) as usize)?;
            xfdf_doc.annotations.extend(page_annotations);
        }

        Ok(xfdf_doc)
    }

    fn read_annotations_from_page(
        document: &lopdf::Document,
        page_id: lopdf::ObjectId,
        page_index: usize,
    ) -> Result<Vec<Annotation>> {
        let page_obj = document.get_object(page_id)?;
        let page_dict = page_obj.as_dict()
            .map_err(|e| PdfXmlError::PdfProcessing(format!("无效页面对象: {}", e)))?;

        let annots = match page_dict.get(b"Annots") {
            Ok(obj) => Self::resolve_object(document, obj)?.as_array()
                .map_err(|e| PdfXmlError::PdfProcessing(format!("Annots 不是数组: {}", e)))?,
            Err(_) => return Ok(Vec::new()),
        };

        let mut annotations = Vec::new();
        for annot_obj in annots {
            let resolved = Self::resolve_object(document, annot_obj)?;
            let annot_dict = resolved.as_dict()
                .map_err(|e| PdfXmlError::PdfProcessing(format!("注释对象不是字典: {}", e)))?;

            match Self::annotation_from_pdf_dict(document, annot_dict, page_index) {
                Ok(Some(annotation)) => annotations.push(annotation),
                Ok(None) => {}
                Err(err) => warn!("跳过第 {} 页的一条注释: {}", page_index + 1, err),
            }
        }

        Ok(annotations)
    }

    fn annotation_from_pdf_dict(
        document: &lopdf::Document,
        dict: &lopdf::Dictionary,
        page: usize,
    ) -> Result<Option<Annotation>> {
        let subtype = match dict.get(b"Subtype") {
            Ok(obj) => Self::object_name(Self::resolve_object(document, obj)?)
                .ok_or_else(|| PdfXmlError::PdfProcessing("注释 Subtype 不是名称".to_string()))?,
            Err(_) => return Ok(None),
        };

        let base = AnnotationBase {
            name: Self::dict_string(document, dict, b"NM")?,
            page,
            rect: Self::dict_rect(document, dict, b"Rect")?,
            title: Self::dict_string(document, dict, b"T")?,
            subject: Self::dict_string(document, dict, b"Subj")?,
            contents: Self::dict_string(document, dict, b"Contents")?,
            creation_date: Self::dict_string(document, dict, b"CreationDate")?,
            modification_date: Self::dict_string(document, dict, b"M")?,
            color: Self::dict_color(document, dict, b"C")?,
            opacity: Self::dict_number(document, dict, b"CA")?.unwrap_or(1.0) as f32,
            flags: Self::dict_integer(document, dict, b"F")?.unwrap_or(0) as u32,
            extra: HashMap::new(),
        };

        let annotation = match subtype.as_str() {
            "Text" => Some(Annotation::Text(TextAnnotation {
                base,
                open: Self::dict_bool(document, dict, b"Open")?.unwrap_or(false),
                icon_type: Self::dict_name_or_string(document, dict, b"Name")?
                    .unwrap_or_else(|| "Note".to_string()),
            })),
            "Highlight" => Some(Annotation::Highlight(HighlightAnnotation {
                base,
                coords: Self::dict_number_list(document, dict, b"QuadPoints")?.map(Self::numbers_to_csv),
            })),
            "Underline" => Some(Annotation::Underline(UnderlineAnnotation {
                base,
                coords: Self::dict_number_list(document, dict, b"QuadPoints")?.map(Self::numbers_to_csv),
            })),
            "StrikeOut" => Some(Annotation::StrikeOut(StrikeOutAnnotation {
                base,
                coords: Self::dict_number_list(document, dict, b"QuadPoints")?.map(Self::numbers_to_csv),
            })),
            "Squiggly" => Some(Annotation::Squiggly(SquigglyAnnotation {
                base,
                coords: Self::dict_number_list(document, dict, b"QuadPoints")?.map(Self::numbers_to_csv),
            })),
            "FreeText" => {
                let default_style = Self::dict_string(document, dict, b"DS")?;
                let default_appearance = Self::dict_string(document, dict, b"DA")?;
                let text_color = default_appearance
                    .as_deref()
                    .and_then(parse_text_color_from_da_hex)
                    .or_else(|| base.color.clone());
                Some(Annotation::FreeText(FreeTextAnnotation {
                    base,
                    default_style,
                    default_appearance,
                    text_color,
                    align: Self::dict_integer(document, dict, b"Q")?.unwrap_or(0) as i32,
                }))
            },
            "Square" => Some(Annotation::Square(SquareAnnotation {
                base,
                width: Self::annotation_width(document, dict)?.unwrap_or(1.0),
            })),
            "Circle" => Some(Annotation::Circle(CircleAnnotation {
                base,
                width: Self::annotation_width(document, dict)?.unwrap_or(1.0),
                interior_color: Self::dict_color(document, dict, b"IC")?,
            })),
            "Line" => {
                let line = Self::dict_number_list(document, dict, b"L")?;
                let (start, end) = if let Some(values) = line {
                    if values.len() >= 4 {
                        (
                            Some(format!("{},{}", values[0], values[1])),
                            Some(format!("{},{}", values[2], values[3])),
                        )
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };
                let (head_style, tail_style) = Self::line_ending_styles(document, dict)?;
                Some(Annotation::Line(LineAnnotation {
                    base,
                    start,
                    end,
                    head_style,
                    tail_style,
                    width: Self::annotation_width(document, dict)?.unwrap_or(1.0),
                }))
            }
            "Polygon" => Some(Annotation::Polygon(PolygonAnnotation {
                base,
                vertices: Self::dict_number_list(document, dict, b"Vertices")?.map(Self::numbers_to_vertices),
                is_closed: true,
            })),
            "PolyLine" => Some(Annotation::Polygon(PolygonAnnotation {
                base,
                vertices: Self::dict_number_list(document, dict, b"Vertices")?.map(Self::numbers_to_vertices),
                is_closed: false,
            })),
            "Ink" => Some(Annotation::Ink(InkAnnotation {
                base,
                ink_list: Self::dict_ink_list(document, dict, b"InkList")?.unwrap_or_default(),
                width: Self::annotation_width(document, dict)?.unwrap_or(1.0),
            })),
            "Stamp" => Some(Annotation::Stamp(StampAnnotation {
                base,
                icon: Self::dict_name_or_string(document, dict, b"Name")?.unwrap_or_default(),
                image_data: Self::stamp_image_data_from_annotation(document, dict)?,
            })),
            "Popup" => Some(Annotation::Popup(PopupAnnotation {
                base,
                open: Self::dict_bool(document, dict, b"Open")?.unwrap_or(false),
                parent_name: Self::popup_parent_name(document, dict)?,
            })),
            other => {
                warn!("跳过不支持的注释类型: {}", other);
                None
            }
        };

        Ok(annotation)
    }

    fn stamp_image_data_from_annotation(document: &lopdf::Document, dict: &lopdf::Dictionary) -> Result<Option<String>> {
        let ap = match dict.get(b"AP") {
            Ok(obj) => Self::resolve_object(document, obj)?,
            Err(_) => return Ok(None),
        };
        let ap_dict = match ap.as_dict() {
            Ok(dict) => dict,
            Err(_) => {
                warn!("Stamp 注释的 AP 不是字典，跳过 imagedata 导出");
                return Ok(None);
            }
        };
        let normal_ap = match ap_dict.get(b"N") {
            Ok(obj) => obj,
            Err(_) => return Ok(None),
        };
        Self::stamp_image_data_from_ap_stream(document, normal_ap)
    }

    fn stamp_image_data_from_ap_stream(document: &lopdf::Document, ap_obj: &lopdf::Object) -> Result<Option<String>> {
        let ap_stream = match Self::resolve_object(document, ap_obj)? {
            lopdf::Object::Stream(stream) => stream,
            _ => {
                warn!("Stamp 注释的 AP/N 不是流对象，跳过 imagedata 导出");
                return Ok(None);
            }
        };
        let subtype = ap_stream.dict.get(b"Subtype")
            .ok()
            .and_then(Self::object_name);
        if subtype.as_deref() != Some("Form") {
            warn!("Stamp 注释的 AP/N 不是 Form XObject，跳过 imagedata 导出");
            return Ok(None);
        }

        let resources = match ap_stream.dict.get(b"Resources") {
            Ok(obj) => Self::resolve_object(document, obj)?,
            Err(_) => return Ok(None),
        };
        let resources_dict = match resources.as_dict() {
            Ok(dict) => dict,
            Err(_) => {
                warn!("Stamp 注释的 AP Resources 不是字典，跳过 imagedata 导出");
                return Ok(None);
            }
        };
        let xobjects = match resources_dict.get(b"XObject") {
            Ok(obj) => Self::resolve_object(document, obj)?,
            Err(_) => return Ok(None),
        };
        let xobjects_dict = match xobjects.as_dict() {
            Ok(dict) => dict,
            Err(_) => {
                warn!("Stamp 注释的 AP XObject 不是字典，跳过 imagedata 导出");
                return Ok(None);
            }
        };

        if let Ok(image_obj) = xobjects_dict.get(b"Im0") {
            return Self::image_data_url_from_xobject(document, image_obj);
        }

        let mut image_candidates = Vec::new();
        for (_, obj) in xobjects_dict.iter() {
            let resolved = Self::resolve_object(document, obj)?;
            if let lopdf::Object::Stream(stream) = resolved {
                let subtype = stream.dict.get(b"Subtype")
                    .ok()
                    .and_then(Self::object_name);
                if subtype.as_deref() == Some("Image") {
                    image_candidates.push(obj);
                }
            }
        }

        if image_candidates.len() == 1 {
            return Self::image_data_url_from_xobject(document, image_candidates[0]);
        }

        if !image_candidates.is_empty() {
            warn!("Stamp 注释的 AP 含多个图片 XObject，暂不支持自动选择");
        }
        Ok(None)
    }

    fn image_data_url_from_xobject(document: &lopdf::Document, image_obj: &lopdf::Object) -> Result<Option<String>> {
        let image_stream = match Self::resolve_object(document, image_obj)? {
            lopdf::Object::Stream(stream) => stream,
            _ => return Ok(None),
        };
        let subtype = image_stream.dict.get(b"Subtype")
            .ok()
            .and_then(Self::object_name);
        if subtype.as_deref() != Some("Image") {
            return Ok(None);
        }

        let width = match Self::dict_integer(document, &image_stream.dict, b"Width")? {
            Some(v) if v > 0 => v as u32,
            _ => {
                warn!("Stamp 图片缺少有效 Width，跳过 imagedata 导出");
                return Ok(None);
            }
        };
        let height = match Self::dict_integer(document, &image_stream.dict, b"Height")? {
            Some(v) if v > 0 => v as u32,
            _ => {
                warn!("Stamp 图片缺少有效 Height，跳过 imagedata 导出");
                return Ok(None);
            }
        };
        let bits_per_component = Self::dict_integer(document, &image_stream.dict, b"BitsPerComponent")?.unwrap_or(8);
        if bits_per_component != 8 {
            warn!("Stamp 图片 BitsPerComponent={} 暂不支持", bits_per_component);
            return Ok(None);
        }
        let color_space = match image_stream.dict.get(b"ColorSpace") {
            Ok(obj) => Self::resolve_object(document, obj).ok().and_then(Self::object_name),
            Err(_) => None,
        };
        if color_space.as_deref() != Some("DeviceRGB") {
            warn!("Stamp 图片 ColorSpace={:?} 暂不支持", color_space);
            return Ok(None);
        }

        let rgb_bytes = if image_stream.dict.get(b"Filter").is_ok() {
            match image_stream.decompressed_content() {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("解压 Stamp 图片流失败: {}", err);
                    return Ok(None);
                }
            }
        } else {
            image_stream.content.clone()
        };
        let expected_rgb_len = (width as usize) * (height as usize) * 3;
        if rgb_bytes.len() != expected_rgb_len {
            warn!("Stamp 图片 RGB 数据长度不匹配: got {}, expected {}", rgb_bytes.len(), expected_rgb_len);
            return Ok(None);
        }

        let alpha_bytes = match image_stream.dict.get(b"SMask") {
            Ok(mask_obj) => match Self::decode_smask_alpha(document, mask_obj, width, height)? {
                Some(bytes) => bytes,
                None => return Ok(None),
            },
            Err(_) => vec![255; (width as usize) * (height as usize)],
        };

        let mut rgba_bytes = Vec::with_capacity((width as usize) * (height as usize) * 4);
        for (rgb, alpha) in rgb_bytes.chunks_exact(3).zip(alpha_bytes.iter().copied()) {
            rgba_bytes.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
        }

        let Some(rgba_image) = RgbaImage::from_raw(width, height, rgba_bytes) else {
            warn!("重建 Stamp RGBA 图片失败");
            return Ok(None);
        };

        let dynamic = DynamicImage::ImageRgba8(rgba_image);
        let mut output = Cursor::new(Vec::new());
        if let Err(err) = dynamic.write_to(&mut output, ImageFormat::Png) {
            warn!("编码 Stamp PNG 失败: {}", err);
            return Ok(None);
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(output.into_inner());
        Ok(Some(format!("data:image/png;base64,{}", encoded)))
    }


    fn decode_smask_alpha(
        document: &lopdf::Document,
        smask_obj: &lopdf::Object,
        expected_width: u32,
        expected_height: u32,
    ) -> Result<Option<Vec<u8>>> {
        let smask_stream = match Self::resolve_object(document, smask_obj)? {
            lopdf::Object::Stream(stream) => stream,
            _ => {
                warn!("Stamp 图片 SMask 不是流对象");
                return Ok(None);
            }
        };
        let subtype = smask_stream.dict.get(b"Subtype")
            .ok()
            .and_then(Self::object_name);
        if subtype.as_deref() != Some("Image") {
            warn!("Stamp 图片 SMask 不是 Image XObject");
            return Ok(None);
        }

        let width = Self::dict_integer(document, &smask_stream.dict, b"Width")?.unwrap_or_default() as u32;
        let height = Self::dict_integer(document, &smask_stream.dict, b"Height")?.unwrap_or_default() as u32;
        if width != expected_width || height != expected_height {
            warn!("Stamp 图片 SMask 尺寸不匹配: {}x{} vs {}x{}", width, height, expected_width, expected_height);
            return Ok(None);
        }
        let bits_per_component = Self::dict_integer(document, &smask_stream.dict, b"BitsPerComponent")?.unwrap_or(8);
        if bits_per_component != 8 {
            warn!("Stamp 图片 SMask BitsPerComponent={} 暂不支持", bits_per_component);
            return Ok(None);
        }
        let color_space = match smask_stream.dict.get(b"ColorSpace") {
            Ok(obj) => Self::resolve_object(document, obj).ok().and_then(Self::object_name),
            Err(_) => None,
        };
        if color_space.as_deref() != Some("DeviceGray") {
            warn!("Stamp 图片 SMask ColorSpace={:?} 暂不支持", color_space);
            return Ok(None);
        }

        let alpha = if smask_stream.dict.get(b"Filter").is_ok() {
            match smask_stream.decompressed_content() {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("解压 Stamp SMask 流失败: {}", err);
                    return Ok(None);
                }
            }
        } else {
            smask_stream.content.clone()
        };
        let expected_len = (expected_width as usize) * (expected_height as usize);
        if alpha.len() != expected_len {
            warn!("Stamp 图片 SMask 数据长度不匹配: got {}, expected {}", alpha.len(), expected_len);
            return Ok(None);
        }
        Ok(Some(alpha))
    }

    fn resolve_object<'a>(document: &'a lopdf::Document, object: &'a lopdf::Object) -> Result<&'a lopdf::Object> {
        match object {
            lopdf::Object::Reference(id) => document.get_object(*id)
                .map_err(|e| PdfXmlError::PdfProcessing(format!("读取引用对象失败: {}", e))),
            other => Ok(other),
        }
    }

    fn object_name(object: &lopdf::Object) -> Option<String> {
        match object {
            lopdf::Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
            _ => None,
        }
    }

    fn object_string(object: &lopdf::Object) -> Option<String> {
        match object {
            lopdf::Object::String(bytes, _) => {
                if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF && bytes.len() % 2 == 0 {
                    let utf16: Vec<u16> = bytes[2..]
                        .chunks_exact(2)
                        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
                        .collect();
                    String::from_utf16(&utf16).ok()
                } else {
                    Some(String::from_utf8_lossy(bytes).to_string())
                }
            }
            lopdf::Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
            _ => None,
        }
    }

    fn object_number(object: &lopdf::Object) -> Option<f64> {
        match object {
            lopdf::Object::Integer(v) => Some(*v as f64),
            lopdf::Object::Real(v) => Some(*v as f64),
            _ => None,
        }
    }

    fn dict_string(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<String>> {
        match dict.get(key) {
            Ok(obj) => Ok(Self::object_string(Self::resolve_object(document, obj)?)),
            Err(_) => Ok(None),
        }
    }

    fn dict_name_or_string(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<String>> {
        match dict.get(key) {
            Ok(obj) => {
                let resolved = Self::resolve_object(document, obj)?;
                Ok(Self::object_name(resolved).or_else(|| Self::object_string(resolved)))
            }
            Err(_) => Ok(None),
        }
    }

    fn dict_integer(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<i64>> {
        match dict.get(key) {
            Ok(obj) => Ok(Self::object_number(Self::resolve_object(document, obj)?).map(|v| v as i64)),
            Err(_) => Ok(None),
        }
    }

    fn dict_number(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<f64>> {
        match dict.get(key) {
            Ok(obj) => Ok(Self::object_number(Self::resolve_object(document, obj)?)),
            Err(_) => Ok(None),
        }
    }

    fn dict_bool(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<bool>> {
        match dict.get(key) {
            Ok(obj) => match Self::resolve_object(document, obj)? {
                lopdf::Object::Boolean(v) => Ok(Some(*v)),
                _ => Ok(None),
            },
            Err(_) => Ok(None),
        }
    }

    fn dict_number_list(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<Vec<f64>>> {
        match dict.get(key) {
            Ok(obj) => {
                let arr = Self::resolve_object(document, obj)?.as_array()
                    .map_err(|e| PdfXmlError::PdfProcessing(format!("字段 {:?} 不是数组: {}", String::from_utf8_lossy(key), e)))?;
                let values = arr.iter().filter_map(Self::object_number).collect::<Vec<_>>();
                Ok(Some(values))
            }
            Err(_) => Ok(None),
        }
    }

    fn dict_rect(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<Rect>> {
        let Some(values) = Self::dict_number_list(document, dict, key)? else {
            return Ok(None);
        };
        if values.len() < 4 {
            return Ok(None);
        }
        Ok(Some(Rect {
            left: values[0],
            bottom: values[1],
            right: values[2],
            top: values[3],
        }))
    }

    fn dict_color(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<String>> {
        let Some(values) = Self::dict_number_list(document, dict, key)? else {
            return Ok(None);
        };
        if values.len() < 3 {
            return Ok(None);
        }
        Ok(Some(format!(
            "#{:02X}{:02X}{:02X}",
            (values[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (values[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (values[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        )))
    }

    fn dict_ink_list(document: &lopdf::Document, dict: &lopdf::Dictionary, key: &[u8]) -> Result<Option<Vec<String>>> {
        match dict.get(key) {
            Ok(obj) => {
                let outer = Self::resolve_object(document, obj)?.as_array()
                    .map_err(|e| PdfXmlError::PdfProcessing(format!("InkList 不是数组: {}", e)))?;
                let mut gestures = Vec::new();
                for gesture in outer {
                    let points = Self::resolve_object(document, gesture)?.as_array()
                        .map_err(|e| PdfXmlError::PdfProcessing(format!("InkList 子项不是数组: {}", e)))?;
                    let values = points.iter().filter_map(Self::object_number).collect::<Vec<_>>();
                    gestures.push(Self::numbers_to_pairs(&values, ";"));
                }
                Ok(Some(gestures))
            }
            Err(_) => Ok(None),
        }
    }

    fn annotation_width(document: &lopdf::Document, dict: &lopdf::Dictionary) -> Result<Option<f32>> {
        if let Ok(bs) = dict.get(b"BS") {
            let bs_dict = Self::resolve_object(document, bs)?.as_dict()
                .map_err(|e| PdfXmlError::PdfProcessing(format!("BS 不是字典: {}", e)))?;
            if let Some(width) = Self::dict_number(document, bs_dict, b"W")? {
                return Ok(Some(width as f32));
            }
        }

        if let Ok(border) = dict.get(b"Border") {
            let values = Self::resolve_object(document, border)?.as_array()
                .map_err(|e| PdfXmlError::PdfProcessing(format!("Border 不是数组: {}", e)))?;
            if values.len() >= 3 {
                if let Some(width) = Self::object_number(&values[2]) {
                    return Ok(Some(width as f32));
                }
            }
        }

        Ok(None)
    }

    fn line_ending_styles(document: &lopdf::Document, dict: &lopdf::Dictionary) -> Result<(String, String)> {
        match dict.get(b"LE") {
            Ok(obj) => {
                let values = Self::resolve_object(document, obj)?.as_array()
                    .map_err(|e| PdfXmlError::PdfProcessing(format!("LE 不是数组: {}", e)))?;
                let head = values.first().and_then(Self::object_name).unwrap_or_default();
                let tail = values.get(1).and_then(Self::object_name).unwrap_or_default();
                Ok((head, tail))
            }
            Err(_) => Ok((String::new(), String::new())),
        }
    }

    fn popup_parent_name(document: &lopdf::Document, dict: &lopdf::Dictionary) -> Result<Option<String>> {
        let parent = match dict.get(b"Parent") {
            Ok(obj) => Self::resolve_object(document, obj)?,
            Err(_) => return Ok(None),
        };
        let parent_dict = parent.as_dict()
            .map_err(|e| PdfXmlError::PdfProcessing(format!("Popup Parent 不是字典: {}", e)))?;
        Self::dict_string(document, parent_dict, b"NM")
    }

    fn numbers_to_csv(values: Vec<f64>) -> String {
        values.into_iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
    }

    fn numbers_to_vertices(values: Vec<f64>) -> String {
        Self::numbers_to_pairs(&values, " ")
    }

    fn numbers_to_pairs(values: &[f64], pair_separator: &str) -> String {
        values.chunks(2)
            .filter(|chunk| chunk.len() == 2)
            .map(|chunk| format!("{},{}", chunk[0], chunk[1]))
            .collect::<Vec<_>>()
            .join(pair_separator)
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

    // PDF 不是简单地把页面放进一个列表里。
    // 它会用一棵“页面树”来管理所有页面。
    // 如果要往现有 PDF 里补新页，就得先找到这棵树的根。
    fn pages_root_id(document: &lopdf::Document) -> Result<lopdf::ObjectId> {
        let root_id = match document.trailer.get(b"Root") {
            Ok(lopdf::Object::Reference(id)) => *id,
            Ok(_) => {
                return Err(PdfXmlError::PdfProcessing(
                    "PDF Catalog Root 不是间接对象".to_string(),
                ));
            }
            Err(e) => {
                return Err(PdfXmlError::PdfProcessing(format!(
                    "无法读取 PDF Catalog Root: {}",
                    e
                )));
            }
        };

        let catalog = document.get_object(root_id)?.as_dict()
            .map_err(|e| PdfXmlError::PdfProcessing(format!("PDF Catalog 不是字典: {}", e)))?;

        match catalog.get(b"Pages") {
            Ok(lopdf::Object::Reference(id)) => Ok(*id),
            Ok(_) => Err(PdfXmlError::PdfProcessing(
                "PDF Pages 根节点不是间接对象".to_string(),
            )),
            Err(e) => Err(PdfXmlError::PdfProcessing(format!(
                "无法读取 PDF Pages 根节点: {}",
                e
            ))),
        }
    }

    // 现有 PDF 补页时，需要把新页挂回原文档的 Pages 树。
    // 补新页时，除了创建页面对象，还要把它登记进 Pages 树。
    // 这里负责做三件事：
    // 1. 让新页的 Parent 指向 Pages 根
    // 2. 把新页加进 Kids 列表
    // 3. 把总页数 Count 加 1
    fn attach_page_to_pages_root(
        document: &mut lopdf::Document,
        pages_root_id: lopdf::ObjectId,
        page_id: lopdf::ObjectId,
    ) -> Result<()> {
        if let Some(lopdf::Object::Dictionary(dict)) = document.objects.get_mut(&page_id) {
            dict.set("Parent", lopdf::Object::Reference(pages_root_id));
        } else {
            return Err(PdfXmlError::PdfProcessing(
                "新增页面对象不是字典".to_string(),
            ));
        }

        if let Some(lopdf::Object::Dictionary(dict)) = document.objects.get_mut(&pages_root_id) {
            let mut kids = match dict.get(b"Kids") {
                Ok(obj) => obj
                    .as_array()
                    .map_err(|e| PdfXmlError::PdfProcessing(format!("Pages/Kids 不是数组: {}", e)))?
                    .clone(),
                Err(_) => Vec::new(),
            };
            kids.push(lopdf::Object::Reference(page_id));

            let count = dict
                .get(b"Count")
                .ok()
                .and_then(Self::object_number)
                .unwrap_or(0.0) as i64;

            dict.set("Kids", lopdf::Object::Array(kids));
            dict.set("Count", lopdf::Object::Integer(count + 1));
            Ok(())
        } else {
            Err(PdfXmlError::PdfProcessing(
                "PDF Pages 根节点不是字典".to_string(),
            ))
        }
    }
    // 这个函数的目标是：
    // “保留原来的 PDF 内容，只把新的注释加进去”。
    //
    // 所以它不会像 `export_to_new_pdf` 那样从零开始建整份文档，
    // 而是先打开原 PDF，再决定每一页该追加什么。
    pub fn export_to_existing_pdf(&mut self, xfdf_doc: &XfdfDocument, input_path: &Path, output_path: &Path) -> Result<()> {
        // 这个函数的目标是：
        // “保留原来的 PDF 内容，只把新的注释加进去”。
        //
        // 所以它不会像 `export_to_new_pdf` 那样从零开始建整份文档，
        // 而是先打开原 PDF，再决定每一页该追加什么。
        // 这种模式不是新建空白 PDF，
        // 而是在已有 PDF 基础上，把 XFDF 里的注释挂到对应页面上。
        info!("打开现有 PDF 并合并注释");

        let mut document = lopdf::Document::load(input_path)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("加载PDF失败: {}", e)))?;

        let pages = document.get_pages();
        let existing_page_count = pages.len();
        let pages_root_id = Self::pages_root_id(&document)?;
        info!("原始 PDF 共 {} 页", existing_page_count);

        let mut font_dict = lopdf::Dictionary::new();
        font_dict.set("Type", lopdf::Object::Name(b"Font".to_vec()));
        font_dict.set("Subtype", lopdf::Object::Name(b"Type1".to_vec()));
        font_dict.set("BaseFont", lopdf::Object::Name(b"Helvetica".to_vec()));
        let font_id_ref = document.add_object(font_dict);

        // 先处理原 PDF 里已经存在的那些页。
        // 这一段只是在原来的页面上追加注释，不会新建页面。
        for page_num in 0..existing_page_count {
            let page_annotations = xfdf_doc.get_annotations_for_page(page_num);
            if page_annotations.is_empty() {
                continue;
            }

            let page_id = pages[&((page_num + 1) as u32)];
            self.add_annotations_to_page(&mut document, page_id, &page_annotations, font_id_ref)?;
            debug!("向第 {} 页添加了 {} 条注释", page_num + 1, page_annotations.len());
        }

        // XFDF 页码超出原 PDF 时，补建新页而不是静默丢弃注释。
        // 再处理 XFDF 里“超出原 PDF 页数”的那部分注释。
        // 如果原 PDF 没有对应页面，就要真的补出新页，
        // 否则这些注释就会凭空消失。
        for page_num in existing_page_count..xfdf_doc.total_pages() {
            let page_annotations = xfdf_doc.get_annotations_for_page(page_num);
            let page_id = self.create_page(&mut document, "F1", font_id_ref, &page_annotations, page_num)?;
            Self::attach_page_to_pages_root(&mut document, pages_root_id, page_id)?;
            debug!("为注释新增了第 {} 页", page_num + 1);
        }

        document.save(output_path)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("保存PDF失败: {}", e)))?;

        Ok(())
    }
    // 这个函数负责“真正新建一页”。
    // 它会同时准备：
    // 1. 页面本身
    // 2. 页面里要挂的注释
    // 3. 让这页至少有一段能被阅读器识别的内容流
    fn create_page(&self, document: &mut lopdf::Document, _font_name: &str, font_id: lopdf::ObjectId,
                     annotations: &[&Annotation], page_number: usize) -> Result<lopdf::ObjectId> {
        // 这个函数负责“真正新建一页”。
        // 它会同时准备：
        // 1. 页面本身
        // 2. 页面里要挂的注释
        // 3. 让这页至少有一段能被阅读器识别的内容流
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

        let page_id = document.add_object(page_dict);
        // 新建页面时顺手回填 P，保持注释和页面的双向关联完整。
        // 前面创建注释时，还不知道最终页面对象的编号。
        // 现在 page_id 已经有了，就回过头把每条注释的 `P` 字段补上，
        // 告诉 PDF：“这条注释属于这一页”。
        for annot_id in &annot_ids {
            if let Some(lopdf::Object::Dictionary(dict)) = document.objects.get_mut(annot_id) {
                dict.set("P", lopdf::Object::Reference(page_id));
            }
        }

        Ok(page_id)
    }
    // 这一层负责把“内存里的注释结构体”变成“PDF 里的对象”。
    // 先全部创建出来，再补 Popup 这种需要互相引用的关系。
    fn create_annotation_objects(&self, document: &mut lopdf::Document, annotations: &[&Annotation]) -> Result<Vec<lopdf::ObjectId>> {
        // 这一层负责把“内存里的注释结构体”变成“PDF 里的对象”。
        // 先全部创建出来，再补 Popup 这种需要互相引用的关系。
        let mut ids = Vec::new();
        // 先记住“注释名字 -> 注释对象编号”。
        // 这样后面处理 Popup 时，才能按名字找到它真正对应的父注释对象。
        let mut named_annotations = HashMap::new();
        let mut popup_links = Vec::new();

        for annotation in annotations {
            let annot_dict = self.build_annotation(document, annotation)?;
            let annot_id = document.add_object(annot_dict);

            if let Some(name) = annotation.base().name.as_ref() {
                match named_annotations.entry(name.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(annot_id);
                    }
                    Entry::Occupied(_) => {
                        warn!("同一页上出现重复注释名称 {:?}，Popup 关联可能不稳定", name);
                    }
                }
            }

            if let Annotation::Popup(popup) = annotation {
                if let Some(parent_name) = popup.parent_name.as_ref() {
                    popup_links.push((annot_id, parent_name.clone()));
                }
            }

            ids.push(annot_id);
        }

        // Popup 需要靠名字回连到父注释，读回 XFDF 时才能恢复 parent。
        // Popup 不是只保存一个父名字就够了，
        // 在 PDF 里它真正需要的是“指向父注释对象”的引用。
        // 所以这里第二轮再把这些对象引用补上。
        for (popup_id, parent_name) in popup_links {
            if let Some(parent_id) = named_annotations.get(&parent_name).copied() {
                if let Some(lopdf::Object::Dictionary(dict)) = document.objects.get_mut(&popup_id) {
                    dict.set("Parent", lopdf::Object::Reference(parent_id));
                }
                if let Some(lopdf::Object::Dictionary(dict)) = document.objects.get_mut(&parent_id) {
                    dict.set("Popup", lopdf::Object::Reference(popup_id));
                }
            } else {
                warn!("未找到 Popup 父注释 {:?}，跳过 Parent 关联", parent_name);
            }
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
        if parts.len() < 8 || !parts.len().is_multiple_of(8) {
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
        if values.is_empty() || !values.len().is_multiple_of(2) { return None; }
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
            if let Some(lopdf::Object::Dictionary(dict)) = document.objects.get_mut(&(annot_id.0, annot_id.1)) {
                dict.set("P", lopdf::Object::Reference(page_id));
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

        if let Some(lopdf::Object::Dictionary(dict)) = document.objects.get_mut(&(page_id.0, page_id.1)) {
            dict.set("Annots", lopdf::Object::Array(new_annots));
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
            if let Some(lopdf::Object::Dictionary(dict)) = document.objects.get_mut(&(id.0, id.1)) {
                dict.set("Parent", lopdf::Object::Reference(pg_id));
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
    fn test_parse_alignment_from_style_with_spaces() {
        assert_eq!(parse_alignment_from_style(Some("font:12px; text-align: center; color:#000")), 1);
        assert_eq!(parse_alignment_from_style(Some("text-align : right")), 2);
        assert_eq!(parse_alignment_from_style(Some("font-weight:bold")), 0);
    }

    #[test]
    fn test_freetext_ap_should_use_requested_font() {
        let annotation = FreeTextAnnotation {
            base: AnnotationBase {
                name: None,
                page: 0,
                rect: Some(Rect {
                    left: 100.0,
                    bottom: 100.0,
                    right: 240.0,
                    top: 160.0,
                }),
                title: None,
                subject: None,
                contents: Some("Hello font".to_string()),
                creation_date: None,
                modification_date: None,
                color: None,
                opacity: 1.0,
                flags: 0,
                extra: HashMap::new(),
            },
            default_style: None,
            default_appearance: Some("/Courier 14 Tf".to_string()),
            text_color: None,
            align: 0,
        };

        let mut document = lopdf::Document::with_version("1.5");
        let exporter = PdfAnnotationExporter::new();
        let dict = exporter.build_freetext_annotation(&mut document, &annotation).unwrap();

        let ap = dict.get(b"AP").unwrap().as_dict().unwrap();
        let ap_id = match ap.get(b"N").unwrap() {
            lopdf::Object::Reference(id) => *id,
            other => panic!("Expected appearance stream reference, got {:?}", other),
        };
        let stream = document.get_object(ap_id).unwrap().as_stream().unwrap();
        let content = String::from_utf8_lossy(&stream.content);
        assert!(content.contains("/Courier 14 Tf"));

        let resources = stream.dict.get(b"Resources").unwrap().as_dict().unwrap();
        let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
        let font = fonts.get(b"Courier").unwrap().as_dict().unwrap();
        assert_eq!(font.get(b"BaseFont").unwrap().as_name().unwrap(), b"Courier");
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