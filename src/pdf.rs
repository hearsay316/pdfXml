//! PDF 生成和注释导出模块
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
    contents: String,
    da: String,
    align: i32,
    is_cjk: bool,
}

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
    page_size: (f64, f64),
}

impl PdfAnnotationExporter {
    pub fn new() -> Self {
        Self {
            page_size: (595.0, 842.0),
        }
    }

    pub fn with_page_size(width: f64, height: f64) -> Self {
        Self {
            page_size: (width, height),
        }
    }

    fn candidate_chinese_font_paths() -> Vec<PathBuf> {
        vec![
            PathBuf::from("C:/Windows/Fonts/simsun.ttc"),
            PathBuf::from("C:/Windows/Fonts/msyh.ttc"),
            PathBuf::from("C:/Windows/Fonts/simhei.ttf"),
            PathBuf::from("C:/Windows/Fonts/simkai.ttf"),
            PathBuf::from("C:/Windows/Fonts/simsunb.ttf"),
        ]
    }

    fn load_chinese_font_bytes() -> Result<Vec<u8>> {
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
        let bytes = Self::load_chinese_font_bytes()?;
        let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        Face::parse(leaked, 0)
            .map_err(|e| PdfXmlError::PdfProcessing(format!("解析中文字体失败: {}", e)))
    }

    fn build_freetext_render_spec(annotation: &FreeTextAnnotation) -> Option<FreeTextRenderSpec> {
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
            Annotation::Polygon(_) => "Polygon",
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
                dict.set("BS", lopdf::Dictionary::from_iter(vec![
                    ("W", lopdf::Object::Real(c.width)),
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
            }
            Annotation::Line(l) => {
                if let (Some(start_str), Some(end_str)) = (&l.start, &l.end) {
                    if let (Some(start), Some(end)) = (Self::parse_point(start_str), Self::parse_point(end_str)) {
                        dict.set("L", lopdf::Object::Array(vec![
                            lopdf::Object::Real(start.0 as f32), lopdf::Object::Real(start.1 as f32),
                            lopdf::Object::Real(end.0 as f32), lopdf::Object::Real(end.1 as f32),
                        ]));
                    }
                }
                dict.set("BS", lopdf::Dictionary::from_iter(vec![
                    ("W", lopdf::Object::Real(l.width)),
                    ("S", lopdf::Object::Name(b"S".to_vec())),
                ]));
                if !l.head_style.is_empty() {
                    dict.set("LE", lopdf::Object::Array(vec![
                        lopdf::Object::Name(l.head_style.as_bytes().to_vec()),
                        lopdf::Object::Name(l.tail_style.as_bytes().to_vec()),
                    ]));
                }
            }
            Annotation::Polygon(p) => {
                if let Some(ref vertices_str) = p.vertices {
                    if let Some(verts) = Self::parse_vertices(vertices_str) {
                        dict.set("Vertices", verts);
                    }
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
}
