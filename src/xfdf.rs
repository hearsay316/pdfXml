//! 这个文件专门负责“XFDF/XML 和 Rust 对象之间来回转换”。
//!
//! 你可以把它理解成项目里的“翻译层”：
//! - 一边负责把 XFDF/XML 读成 Rust 结构
//! - 另一边负责把 Rust 结构再写回 XFDF/XML
//!
//! 它不负责真正操作 PDF，那部分在 `pdf.rs`。
//! 它更像是在做：
//! “把外部文件格式，变成程序内部好处理的数据；
//!  再把内部数据重新变回标准交换格式。”
//!
//! 如果你第一次看这个文件，建议顺序是：
//! 1. 先看 [`XfdfDocument`]，知道最终会得到什么对象
//! 2. 再看 [`XfdfDocument::parse`]，理解 XFDF 是怎么读进来的
//! 3. 最后看 [`XfdfDocument::to_xfdf_string`]，理解对象是怎么再写回 XML 的

use crate::annotation::*;
use crate::error::{PdfXmlError, Result};
use log::warn;
use once_cell::sync::Lazy;
use regex::Regex;

mod parser;

pub const XFDF_NAMESPACE: &str = "http://ns.adobe.com/xfdf/";
use std::collections::HashMap;

// 这个文件负责把 XFDF/XML 和 Rust 数据结构互相转换。
// 如果把项目想成一条流水线：
// 1. `xfdf.rs` 负责“读懂 XML”
// 2. `annotation.rs` 负责“定义数据长什么样”
// 3. `pdf.rs` 负责“把这些数据写进 PDF”
//
// 阅读建议：
// 1. 先看 `XfdfDocument`
// 2. 再看 `parse`
// 3. 最后看 `to_xfdf_string`
// 这样最容易看懂“进来是什么，出去又是什么”。

// 正则表达式缓存 - 用于富文本解析
static HTML_BREAKS_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?is)<\s*br\s*/?\s*>").unwrap()
});

static HTML_BLOCKS_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?is)</\s*(p|div|li|tr|h[1-6])\s*>").unwrap()
});

static HTML_TAGS_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?is)<[^>]+>").unwrap()
});

fn extract_plain_text_from_richtext(input: &str) -> String {
    // 有些 XFDF 会把内容放在富文本里，里面带 HTML 标签。
    // 这里尽量保留换行/段落语义，再做基础实体解码。
    let normalized_breaks = HTML_BREAKS_PATTERN
        .replace_all(input, "\n")
        .to_string();
    let normalized_blocks = HTML_BLOCKS_PATTERN
        .replace_all(&normalized_breaks, "\n")
        .to_string();
    let no_tags = HTML_TAGS_PATTERN
        .replace_all(&normalized_blocks, "")
        .to_string();
    let decoded = no_tags
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");

    decoded
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// XFDF 文档
#[derive(Debug, Clone, Default)]
/// 一份完整的 XFDF 文档。
///
/// 它里面通常会包含：
/// - 文档级元数据
/// - 表单字段
/// - 注释列表
pub struct XfdfDocument {
    /// XML 命名空间
    pub xmlns: Option<String>,
    
    /// 表单字段
    pub fields: Vec<XfdfField>,
    
    /// 注释列表
    pub annotations: Vec<Annotation>,
    
    /// 元数据
    pub metadata: HashMap<String, String>,

    /// 标准 <f href="..."/> 文档引用；解析时不会打开该路径。
    pub file_reference: Option<String>,
    pub ids: Option<XfdfIds>,
    /// 无法转换为当前 Annotation 模型的原始注释，仅用于 XFDF 往返。
    pub unsupported_annotations: Vec<String>,
    /// 未实现的文档级扩展，仅用于 XFDF 往返。
    pub extensions: Vec<String>,
}

/// XFDF 表单字段
#[derive(Debug, Clone, Default)]
/// XFDF 里的表单字段。
pub struct XfdfField {
    #[allow(dead_code)]
    pub name: String,
    /// 第一个值，保留原有访问方式；None 与 Some("") 含义不同。
    pub value: Option<String>,
    /// 同一字段的第二个及后续 <value>，用于多选表单。
    pub additional_values: Vec<String>,
    pub children: Vec<XfdfField>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XfdfIds {
    pub original: Option<String>,
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum XfdfParseMode {
    #[default]
    Compatible,
    Strict,
}

/// 资源边界与兼容策略。严格模式拒绝所有 Warning，但允许已保留的扩展。
#[derive(Debug, Clone)]
pub struct XfdfParseOptions {
    pub mode: XfdfParseMode,
    pub max_input_bytes: usize,
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_annotations: usize,
    pub max_page_index: usize,
}

impl Default for XfdfParseOptions {
    fn default() -> Self {
        Self {
            mode: XfdfParseMode::Compatible,
            max_input_bytes: 64 * 1024 * 1024,
            max_depth: 128,
            max_nodes: 250_000,
            max_annotations: 100_000,
            max_page_index: 10_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XfdfDiagnosticSeverity { Info, Warning }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XfdfDiagnosticCode {
    MissingNamespace,
    LegacyStructure,
    InvalidAttribute,
    MissingAttribute,
    UnsupportedAnnotation,
    PreservedExtension,
    UnsupportedStructure,
}

#[derive(Debug, Clone)]
pub struct XfdfDiagnostic {
    pub severity: XfdfDiagnosticSeverity,
    pub code: XfdfDiagnosticCode,
    pub byte_offset: u64,
    pub element: String,
    pub annotation_name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct XfdfParseReport {
    pub document: XfdfDocument,
    pub diagnostics: Vec<XfdfDiagnostic>,
}

impl XfdfField {
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.value.iter().chain(self.additional_values.iter()).map(String::as_str)
    }
}

impl XfdfDocument {
    /// 解析 XFDF/XML 字符串，得到统一的文档对象。
    ///
    /// # 示例
    ///
    /// ```
    /// use pdfxml::XfdfDocument;
    ///
    /// let xml = r#"<?xml version="1.0" encoding="UTF-8" ?>
    /// <xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
    ///   <annots>
    ///     <text page="0" rect="100,700,250,730">Hello</text>
    ///   </annots>
    /// </xfdf>"#;
    ///
    /// let doc = XfdfDocument::parse(xml)?;
    /// assert_eq!(doc.annotations.len(), 1);
    /// # Ok::<(), pdfxml::PdfXmlError>(())
    /// ```
    pub fn parse(xml_str: &str) -> Result<Self> {
        let report = Self::parse_with_report(xml_str)?;
        for diagnostic in &report.diagnostics {
            if diagnostic.severity == XfdfDiagnosticSeverity::Warning {
                warn!("XFDF {:?} at byte {}: {}", diagnostic.code, diagnostic.byte_offset, diagnostic.message);
            }
        }
        Ok(report.document)
    }

    /// 兼容模式解析，并返回可供 CLI/GUI/SDK 展示的结构化诊断。
    ///
    /// ```
    /// use pdfxml::{XfdfDocument, XfdfDiagnosticCode};
    ///
    /// let report = XfdfDocument::parse_with_report(
    ///     r#"<xfdf xmlns="http://ns.adobe.com/xfdf/"><annots><fileattachment page="0" rect="0,0,20,20"/></annots></xfdf>"#,
    /// )?;
    /// assert!(report.document.annotations.is_empty());
    /// assert_eq!(report.document.unsupported_annotations.len(), 1);
    /// assert_eq!(report.diagnostics[0].code, XfdfDiagnosticCode::UnsupportedAnnotation);
    /// # Ok::<(), pdfxml::PdfXmlError>(())
    /// ```
    pub fn parse_with_report(xml_str: &str) -> Result<XfdfParseReport> {
        Self::parse_with_options(xml_str, &XfdfParseOptions::default())
    }

    pub fn parse_with_options(xml_str: &str, options: &XfdfParseOptions) -> Result<XfdfParseReport> {
        parser::parse(xml_str, options)
    }

    /// 根据标签名判断：它是不是我们支持的注释类型。
    ///
    /// 比如 `text`、`highlight`、`square`、`polyline` 都算。
    /// 如果不是这里列出的标签，解析器就不会把它当成注释对象来构建。
    fn is_annotation_tag(tag: &str) -> bool {
        matches!(tag,
            "text" | "highlight" | "underline" | "strikeout" | "squiggly" |
            "freetext" | "square" | "circle" | "line" |
            "polygon" | "polyline" | "ink" | "stamp" |
            "caret" | "fileattachment" | "sound" | "link" |
            "popup" | "widget"
        )
    }
    
    /// 把“注释类型 + 属性字典 + 文本内容”组装成真正的 `Annotation`。
    ///
    /// 可以把它理解成解析阶段的“最后一道装配工序”。
    /// 前面的 `parse` 负责把 XML 里的字符串先收集起来，
    /// 这里再根据标签类型决定应该创建哪一种注释结构体。
    fn build_annotation(
        annotation_type: &str,
        attrs: &HashMap<String, String>,
        content: &str,
    ) -> Result<Annotation> {
        let base = Self::build_base(annotation_type, attrs, content)?;
        
        Ok(match annotation_type {
            "text" => Annotation::Text(TextAnnotation {
                base,
                open: attrs.get("open").map(|v| v == "yes").unwrap_or(false),
                icon_type: attrs.get("icon").cloned().unwrap_or_else(|| "Note".to_string()),
            }),
            "highlight" => Annotation::Highlight(HighlightAnnotation {
                base,
                coords: attrs.get("coords").cloned(),
            }),
            "underline" => Annotation::Underline(UnderlineAnnotation {
                base,
                coords: attrs.get("coords").cloned(),
            }),
            "strikeout" => Annotation::StrikeOut(StrikeOutAnnotation {
                base,
                coords: attrs.get("coords").cloned(),
            }),
            "squiggly" => Annotation::Squiggly(SquigglyAnnotation {
                base,
                coords: attrs.get("coords").cloned(),
            }),
            "freetext" => Annotation::FreeText(FreeTextAnnotation {
                base,
                default_style: attrs.get("defaultstyle").cloned(),
                default_appearance: attrs.get("defaultappearance").cloned(),
                text_color: attrs.get("TextColor").cloned(),
                align: attrs.get("align")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                callout: attrs.get("callout").and_then(|v| CalloutLine::from_string(v)),
                fringe: attrs.get("fringe").and_then(|v| Rect::from_string(v)),
                head_style: attrs.get("head").and_then(|v| LineEndingStyle::from_name(v)).unwrap_or_default(),
                width: attrs.get("width").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            }),
            "square" => Annotation::Square(SquareAnnotation {
                base,
                width: attrs.get("width")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.0),
            }),
            "circle" => Annotation::Circle(CircleAnnotation {
                base,
                width: attrs.get("width")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.0),
                interior_color: attrs.get("interiorcolor").cloned(),
            }),
            "line" => Annotation::Line(LineAnnotation {
                base,
                start: attrs.get("start").cloned(),
                end: attrs.get("end").cloned(),
                head_style: attrs.get("head").cloned().unwrap_or_default(),
                tail_style: attrs.get("tail").cloned().unwrap_or_default(),
                width: attrs.get("width")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.0),
            }),
            "polygon" | "polyline" => Annotation::Polygon(PolygonAnnotation {
                base,
                vertices: attrs.get("vertices").cloned(),
                is_closed: annotation_type == "polygon",
                width: attrs.get("width").and_then(|v| v.parse().ok()).unwrap_or(1.0),
            }),
            "ink" => {
                // 处理 inklist 数据
                let ink_list_data = attrs.get("_inklist")
                    .map(|s| s.split('\x1b').map(String::from).collect())
                    .unwrap_or_default();
                Annotation::Ink(InkAnnotation {
                    base,
                    ink_list: ink_list_data,
                    width: attrs.get("width")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1.0),
            })},
            "stamp" => Annotation::Stamp(StampAnnotation {
                base,
                icon: attrs.get("icon").cloned().unwrap_or_default(),
                image_data: attrs.get("imagedata").cloned(),
            }),
            "link" => Annotation::Link(LinkAnnotation {
                base,
                target: None, // 由结构化子元素解析器填写。
                highlight_mode: parser::highlight_mode(attrs.get("highlight").map(String::as_str))?,
                coords: attrs.get("coords").cloned(),
                border_width: attrs.get("width").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            }),
            "caret" => Annotation::Caret(CaretAnnotation {
                base,
                symbol: match attrs.get("symbol").map(String::as_str).unwrap_or("None") {
                    "None" => CaretSymbol::None,
                    "P" => CaretSymbol::P,
                    other => return Err(PdfXmlError::InvalidXfdfFormat(format!("未知 Caret symbol: {other}"))),
                },
                fringe: attrs.get("fringe").and_then(|v| Rect::from_string(v)),
            }),
            "popup" => Annotation::Popup(PopupAnnotation {
                base,
                open: attrs.get("open").map(|v| v == "yes").unwrap_or(false),
                parent_name: attrs.get("parent").cloned(),
            }),
            other => {
                return Err(PdfXmlError::UnsupportedAnnotationType(other.to_string()));
            }
        })
    }
    
    /// 构建注释对象（包含子元素数据）
    fn build_annotation_with_children(
        annotation_type: &str,
        attrs: &HashMap<String, String>,
        content: &str,
    ) -> Result<Annotation> {
        // 直接复用 build_annotation，因为子元素数据已经通过 _inklist 等特殊 key 传入 attrs
        Self::build_annotation(annotation_type, attrs, content)
    }
    // 这里负责把“零散的字符串属性”整理成统一的基础字段。
    // 例如 page、rect、title、color 这些，
    // 不管最终是 Text 还是 Highlight，都会先经过这里。
    fn build_base(annotation_type: &str, attrs: &HashMap<String, String>, content: &str) -> Result<AnnotationBase> {
        // 这里负责把“零散的字符串属性”整理成统一的基础字段。
        // 例如 page、rect、title、color 这些，
        // 不管最终是 Text 还是 Highlight，都会先经过这里。
        let contents = if content.trim().is_empty() && !attrs.contains_key("_contents-present") {
            attrs.get("contents-richtext")
                .map(|rich| extract_plain_text_from_richtext(rich))
                .filter(|text| !text.trim().is_empty())
        } else {
            Some(content.to_string())
        };

        Ok(AnnotationBase {
            name: attrs.get("name").cloned(),
            page: attrs.get("page")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0),
            rect: attrs.get("rect").and_then(|r| Rect::from_string(r)),
            title: attrs.get("title").cloned(),
            subject: attrs.get("subject").cloned(),
            contents,
            creation_date: attrs.get("creationdate").cloned(),
            modification_date: attrs.get("date").cloned(),
            color: attrs.get("color").cloned(),
            opacity: attrs.get("opacity")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(1.0),
            flags: attrs.get("flags")
                .and_then(|v| parser::parse_flags(v))
                .unwrap_or_default(),
            intent: attrs.get("intent").or_else(|| attrs.get("IT")).cloned(),
            border_style: Self::supports_border(annotation_type).then(|| {
                attrs.get("style").or_else(|| attrs.get("border-style"))
                    .and_then(|v| BorderStyle::from_name(v))
            }).flatten(),
            dashes: Self::supports_border(annotation_type).then(|| {
                attrs.get("dashes").and_then(|v| parser::parse_dashes(v))
            }).flatten(),
            extra: attrs.iter()
                .filter(|(k, v)| !Self::is_known_attr(annotation_type, k)
                    || (matches!(k.as_str(), "style" | "border-style") && BorderStyle::from_name(v).is_none()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        })
    }
    
    /// 只消费当前类型实际建模的属性，避免同名扩展被其他类型吞掉。
    fn is_known_attr(annotation_type: &str, key: &str) -> bool {
        if matches!(key,
            "name" | "page" | "rect" | "title" | "subject" |
            "creationdate" | "date" | "color" | "opacity" | "flags" | "intent" | "IT" | "_contents-present"
        ) {
            return true;
        }
        if Self::supports_border(annotation_type) && matches!(key, "style" | "border-style" | "dashes") {
            return true;
        }
        match annotation_type {
            "text" => matches!(key, "open" | "icon"),
            "highlight" | "underline" | "strikeout" | "squiggly" => key == "coords",
            "freetext" => matches!(key, "defaultstyle" | "defaultappearance" | "TextColor" | "align" | "callout" | "fringe" | "head" | "width"),
            "square" => key == "width",
            "circle" => matches!(key, "width" | "interiorcolor"),
            "line" => matches!(key, "start" | "end" | "head" | "tail" | "width"),
            "polygon" | "polyline" => matches!(key, "vertices" | "width"),
            "ink" => matches!(key, "_inklist" | "width"),
            "stamp" => matches!(key, "icon" | "imagedata"),
            "link" => matches!(key, "uri" | "dest" | "highlight" | "coords" | "width"),
            "caret" => matches!(key, "symbol" | "fringe"),
            "popup" => matches!(key, "open" | "parent"),
            _ => false,
        }
    }

    fn supports_border(annotation_type: &str) -> bool {
        matches!(annotation_type, "freetext" | "square" | "circle" | "line" | "polygon" | "polyline" | "ink" | "link")
    }

    /// 获取指定页面上的所有注释。
    ///
    /// # 示例
    ///
    /// ```
    /// use pdfxml::XfdfDocument;
    ///
    /// let doc = XfdfDocument::parse(r#"<?xml version="1.0" encoding="UTF-8" ?>
    /// <xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
    ///   <annots>
    ///     <text page="0" rect="100,700,250,730">Hello</text>
    ///     <text page="1" rect="100,650,250,680">World</text>
    ///   </annots>
    /// </xfdf>"#)?;
    ///
    /// assert_eq!(doc.get_annotations_for_page(0).len(), 1);
    /// # Ok::<(), pdfxml::PdfXmlError>(())
    /// ```
    pub fn get_annotations_for_page(&self, page: usize) -> Vec<&Annotation> {
        self.annotations.iter().filter(|a| a.page() == page).collect()
    }

    /// 返回文档总页数。
    ///
    /// 如果没有任何注释，默认返回 `1`。
    ///
    /// # 示例
    ///
    /// ```
    /// use pdfxml::XfdfDocument;
    ///
    /// let doc = XfdfDocument::parse(r#"<?xml version="1.0" encoding="UTF-8" ?>
    /// <xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
    ///   <annots>
    ///     <text page="2" rect="100,700,250,730">Hello</text>
    ///   </annots>
    /// </xfdf>"#)?;
    ///
    /// assert_eq!(doc.total_pages(), 3);
    /// # Ok::<(), pdfxml::PdfXmlError>(())
    /// ```
    pub fn total_pages(&self) -> usize {
        self.annotations
            .iter()
            .map(|a| a.page())
            .max()
            .map(|p| p.saturating_add(1))
            .unwrap_or(1)
    }
    // 这个函数和 `parse` 正好相反。
    // `parse` 是把 XML 变成 Rust 对象；
    // 这里则是把 Rust 对象重新拼回标准 XFDF 字符串。
    /// 把当前文档重新序列化成 XFDF 字符串。
    ///
    /// # 示例
    ///
    /// ```
    /// use pdfxml::XfdfDocument;
    ///
    /// let doc = XfdfDocument::parse(r#"<?xml version="1.0" encoding="UTF-8" ?>
    /// <xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
    ///   <annots>
    ///     <text page="0" rect="100,700,250,730">Hello</text>
    ///   </annots>
    /// </xfdf>"#)?;
    ///
    /// let xml = doc.to_xfdf_string()?;
    /// assert!(xml.contains("<xfdf"));
    /// # Ok::<(), pdfxml::PdfXmlError>(())
    /// ```
    pub fn to_xfdf_string(&self) -> Result<String> {
        if self.xmlns.as_deref().is_some_and(|ns| ns != XFDF_NAMESPACE) {
            return Err(PdfXmlError::InvalidXfdfFormat("不支持的 XFDF 命名空间".into()));
        }
        let mut root_attrs = vec![("xmlns".into(), XFDF_NAMESPACE.into())];
        let mut metadata = self.metadata.iter().filter(|(k,_)| k.as_str() != "xmlns").collect::<Vec<_>>();
        metadata.sort_by(|a,b| a.0.cmp(b.0));
        if !self.metadata.contains_key("xml:space") { root_attrs.push(("xml:space".into(), "preserve".into())); }
        root_attrs.extend(metadata.into_iter().map(|(k,v)| (k.clone(), v.clone())));
        let mut xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<xfdf{}>\n", attrs_to_string(&root_attrs));
        if let Some(href) = &self.file_reference {
            xml.push_str(&format!("  <f href=\"{}\"/>\n", escape_xml_attr(href)));
        }
        if let Some(ids) = &self.ids {
            let mut attrs = Vec::new();
            if let Some(original) = &ids.original { attrs.push(("original".into(), original.clone())); }
            if let Some(modified) = &ids.modified { attrs.push(("modified".into(), modified.clone())); }
            xml.push_str(&format!("  <ids{}/>\n", attrs_to_string(&attrs)));
        }
        if !self.fields.is_empty() {
            xml.push_str("  <fields>\n");
            for field in &self.fields { write_field(&mut xml, field, 4); }
            xml.push_str("  </fields>\n");
        }
        if !self.annotations.is_empty() || !self.unsupported_annotations.is_empty() {
            xml.push_str("  <annots>\n");
            for annotation in &self.annotations { xml.push_str(&annotation_to_xfdf_element(annotation)?); }
            for raw in &self.unsupported_annotations { xml.push_str("    "); xml.push_str(raw); xml.push('\n'); }
            xml.push_str("  </annots>\n");
        }
        for raw in &self.extensions { xml.push_str("  "); xml.push_str(raw); xml.push('\n'); }
        xml.push_str("</xfdf>\n");
        parser::read_xml(&xml, &XfdfParseOptions::default())?;
        Ok(xml)
    }

    /// 当前注释导出器不会应用表单或任意扩展。调用方可在转换前展示此损失提示。
    pub fn pdf_conversion_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self.fields.is_empty() { warnings.push("表单字段仅支持 XFDF 往返；当前 PDF 注释导出不会填写 AcroForm".into()); }
        if !self.unsupported_annotations.is_empty() { warnings.push(format!("{} 条未实现的注释只保留在 XFDF 中，不写入 PDF", self.unsupported_annotations.len())); }
        if !self.extensions.is_empty() { warnings.push("文档级 XML 扩展不写入 PDF".into()); }
        if self.file_reference.is_some() || self.ids.is_some() { warnings.push("f/ids 仅用于 XFDF 文档引用，不会自动打开文档或改写 PDF 标识".into()); }
        if self.annotations.iter().any(|a| a.base().extra.keys().any(|k| k != "contents-richtext")) {
            warnings.push("部分注释包含未映射的扩展属性/子元素：XFDF 保留不等于 PDF 保真".into());
        }
        if self.annotations.iter().any(|a| matches!(a, Annotation::Link(LinkAnnotation { target: Some(LinkTarget::Named { .. }), .. }))) {
            warnings.push("命名目的地只保留名称引用：新 PDF 不创建名称树，合并时需目标 PDF 已有同名目的地".into());
        }
        if self.annotations.iter().any(|a| a.base().extra.contains_key("contents-richtext")) {
            warnings.push("富文本 XML 会保留为 PDF RC，但当前外观流仍按纯文本绘制".into());
        }
        if self.annotations.iter().any(|a| matches!(a.base().border_style, Some(BorderStyle::Bevelled | BorderStyle::Inset | BorderStyle::Underline))) {
            warnings.push("bevelled/inset/underline 边框写入 PDF BS；当前不生成这些样式的显式外观，由阅读器决定显示".into());
        }
        if self.annotations.iter().any(|a| a.base().intent.as_deref().is_some_and(|i| !matches!(i, "FreeText" | "FreeTextCallout" | "FreeTextTypeWriter"))) {
            warnings.push("Intent 名称会写入 PDF IT；不据此执行测量、云线或高亮型墨迹等专用绘制".into());
        }
        warnings
    }

}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_attr(value: &str) -> String {
    escape_xml_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn format_rect(rect: &Rect) -> String {
    format!("{},{},{},{}", rect.left, rect.bottom, rect.right, rect.top)
}

fn format_opacity(opacity: f32) -> String { opacity.to_string() }

fn write_field(xml: &mut String, field: &XfdfField, indent: usize) {
    let pad = " ".repeat(indent);
    xml.push_str(&format!("{pad}<field name=\"{}\">\n", escape_xml_attr(&field.name)));
    for value in field.values() {
        xml.push_str(&format!("{pad}  <value>{}</value>\n", escape_xml_text(value)));
    }
    for child in &field.children { write_field(xml, child, indent + 2); }
    xml.push_str(&format!("{pad}</field>\n"));
}

fn push_base_attrs(attrs: &mut Vec<(String, String)>, base: &AnnotationBase, annotation_type: &str) {
    if let Some(name) = &base.name {
        attrs.push(("name".to_string(), name.clone()));
    }
    attrs.push(("page".to_string(), base.page.to_string()));
    if let Some(rect) = &base.rect {
        attrs.push(("rect".to_string(), format_rect(rect)));
    }
    if let Some(title) = &base.title {
        attrs.push(("title".to_string(), title.clone()));
    }
    if let Some(subject) = &base.subject {
        attrs.push(("subject".to_string(), subject.clone()));
    }
    if let Some(creation_date) = &base.creation_date {
        attrs.push(("creationdate".to_string(), creation_date.clone()));
    }
    if let Some(modification_date) = &base.modification_date {
        attrs.push(("date".to_string(), modification_date.clone()));
    }
    if let Some(color) = &base.color {
        attrs.push(("color".to_string(), color.clone()));
    }
    if (base.opacity - 1.0).abs() > f32::EPSILON {
        attrs.push(("opacity".to_string(), format_opacity(base.opacity)));
    }
    if base.flags != 0 {
        attrs.push(("flags".to_string(), parser::flags_string(base.flags)));
    }
    if let Some(intent) = &base.intent { attrs.push(("intent".into(), intent.clone())); }
    if let Some(style) = base.border_style { attrs.push(("style".into(), style.xfdf_name().into())); }
    if let Some(dashes) = &base.dashes { attrs.push(("dashes".into(), dashes.iter().map(f32::to_string).collect::<Vec<_>>().join(","))); }
    let mut extra = base.extra.iter().filter(|(key,_)| !key.starts_with("_xml:") && key.as_str() != "contents-richtext"
        && (!XfdfDocument::is_known_attr(annotation_type, key) || (matches!(key.as_str(), "style" | "border-style") && base.border_style.is_none()))).collect::<Vec<_>>();
    extra.sort_by(|a,b| a.0.cmp(b.0));
    for (key, value) in extra { attrs.push((key.clone(), value.clone())); }
}

fn attrs_to_string(attrs: &[(String, String)]) -> String {
    attrs
        .iter()
        .map(|(key, value)| format!(" {}=\"{}\"", key, escape_xml_attr(value)))
        .collect::<String>()
}

fn annotation_to_xfdf_element(annotation: &Annotation) -> Result<String> {
    let annotation_type = annotation.annotation_type().to_ascii_lowercase();
    let (tag, attrs, contents) = match annotation {
        Annotation::Text(text) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &text.base, &annotation_type);
            if text.open {
                attrs.push(("open".to_string(), "yes".to_string()));
            }
            if text.icon_type != "Note" {
                attrs.push(("icon".to_string(), text.icon_type.clone()));
            }
            ("text", attrs, text.base.contents.clone())
        }
        Annotation::Highlight(highlight) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &highlight.base, &annotation_type);
            if let Some(coords) = &highlight.coords {
                attrs.push(("coords".to_string(), coords.clone()));
            }
            ("highlight", attrs, highlight.base.contents.clone())
        }
        Annotation::Underline(underline) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &underline.base, &annotation_type);
            if let Some(coords) = &underline.coords {
                attrs.push(("coords".to_string(), coords.clone()));
            }
            ("underline", attrs, underline.base.contents.clone())
        }
        Annotation::StrikeOut(strikeout) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &strikeout.base, &annotation_type);
            if let Some(coords) = &strikeout.coords {
                attrs.push(("coords".to_string(), coords.clone()));
            }
            ("strikeout", attrs, strikeout.base.contents.clone())
        }
        Annotation::Squiggly(squiggly) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &squiggly.base, &annotation_type);
            if let Some(coords) = &squiggly.coords {
                attrs.push(("coords".to_string(), coords.clone()));
            }
            ("squiggly", attrs, squiggly.base.contents.clone())
        }
        Annotation::FreeText(freetext) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &freetext.base, &annotation_type);
            if let Some(text_color) = &freetext.text_color {
                attrs.push(("TextColor".to_string(), text_color.clone()));
            }
            if freetext.align != 0 {
                attrs.push(("align".to_string(), freetext.align.to_string()));
            }
            if let Some(callout) = &freetext.callout { attrs.push(("callout".into(), callout.coordinates().iter().map(f64::to_string).collect::<Vec<_>>().join(","))); }
            if let Some(fringe) = &freetext.fringe { attrs.push(("fringe".into(), format_rect(fringe))); }
            if freetext.head_style != LineEndingStyle::None { attrs.push(("head".into(), freetext.head_style.name().into())); }
            if freetext.width != 0.0 || freetext.callout.is_some() { attrs.push(("width".into(), freetext.width.to_string())); }
            ("freetext", attrs, freetext.base.contents.clone())
        }
        Annotation::Square(square) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &square.base, &annotation_type);
            if (square.width - 1.0).abs() > f32::EPSILON {
                attrs.push(("width".to_string(), format_opacity(square.width)));
            }
            ("square", attrs, square.base.contents.clone())
        }
        Annotation::Circle(circle) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &circle.base, &annotation_type);
            if (circle.width - 1.0).abs() > f32::EPSILON {
                attrs.push(("width".to_string(), format_opacity(circle.width)));
            }
            if let Some(interior_color) = &circle.interior_color {
                attrs.push(("interiorcolor".to_string(), interior_color.clone()));
            }
            ("circle", attrs, circle.base.contents.clone())
        }
        Annotation::Line(line) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &line.base, &annotation_type);
            if let Some(start) = &line.start {
                attrs.push(("start".to_string(), start.clone()));
            }
            if let Some(end) = &line.end {
                attrs.push(("end".to_string(), end.clone()));
            }
            if !line.head_style.is_empty() {
                attrs.push(("head".to_string(), line.head_style.clone()));
            }
            if !line.tail_style.is_empty() {
                attrs.push(("tail".to_string(), line.tail_style.clone()));
            }
            if (line.width - 1.0).abs() > f32::EPSILON {
                attrs.push(("width".to_string(), format_opacity(line.width)));
            }
            ("line", attrs, line.base.contents.clone())
        }
        Annotation::Polygon(polygon) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &polygon.base, &annotation_type);
            if polygon.width != 1.0 { attrs.push(("width".into(), polygon.width.to_string())); }
            if let Some(vertices) = &polygon.vertices {
                attrs.push(("vertices".to_string(), vertices.clone()));
            }
            let tag = if polygon.is_closed { "polygon" } else { "polyline" };
            (tag, attrs, polygon.base.contents.clone())
        }
        Annotation::Ink(ink) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &ink.base, &annotation_type);
            if (ink.width - 1.0).abs() > f32::EPSILON { attrs.push(("width".into(), format_opacity(ink.width))); }
            ("ink", attrs, ink.base.contents.clone())
        }
        Annotation::Stamp(stamp) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &stamp.base, &annotation_type);
            if !stamp.icon.is_empty() {
                attrs.push(("icon".to_string(), stamp.icon.clone()));
            }
            ("stamp", attrs, stamp.base.contents.clone())
        }
        Annotation::Link(link) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &link.base, &annotation_type);
            let mode = match link.highlight_mode { LinkHighlightMode::None => "None", LinkHighlightMode::Invert => "Invert", LinkHighlightMode::Outline => "Outline", LinkHighlightMode::Push => "Push" };
            if link.highlight_mode != LinkHighlightMode::Invert { attrs.push(("highlight".into(), mode.into())); }
            if let Some(coords) = &link.coords { attrs.push(("coords".into(), coords.clone())); }
            if link.border_width != 0.0 { attrs.push(("width".into(), format_opacity(link.border_width))); }
            ("link", attrs, link.base.contents.clone())
        }
        Annotation::Caret(caret) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &caret.base, &annotation_type);
            if caret.symbol == CaretSymbol::P { attrs.push(("symbol".into(), "P".into())); }
            if let Some(fringe) = &caret.fringe { attrs.push(("fringe".into(), format_rect(fringe))); }
            ("caret", attrs, caret.base.contents.clone())
        }
        Annotation::Popup(popup) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &popup.base, &annotation_type);
            if popup.open {
                attrs.push(("open".to_string(), "yes".to_string()));
            }
            if let Some(parent_name) = &popup.parent_name {
                attrs.push(("parent".to_string(), parent_name.clone()));
            }
            ("popup", attrs, popup.base.contents.clone())
        }
    };

    let mut children = String::new();
    if let Some(contents) = contents {
        children.push_str(&format!("      <contents>{}</contents>\n", escape_xml_text(&contents)));
    }
    if let Some(rich) = annotation.base().extra.get("contents-richtext") {
        let wrapped = format!("<fragment>{rich}</fragment>");
        let payload = if parser::read_xml(&wrapped, &XfdfParseOptions::default()).is_ok() {
            rich.clone()
        } else {
            format!("<![CDATA[{}]]>", rich.replace("]]>", "]]]]><![CDATA[>"))
        };
        children.push_str(&format!("      <contents-richtext>{payload}</contents-richtext>\n"));
    }
    match annotation {
        Annotation::FreeText(text) => {
            for (tag,value) in [("defaultstyle", &text.default_style), ("defaultappearance", &text.default_appearance)] {
                if let Some(value) = value { children.push_str(&format!("      <{tag}>{}</{tag}>\n", escape_xml_text(value))); }
            }
        }
        Annotation::Stamp(stamp) => if let Some(image) = &stamp.image_data {
            children.push_str(&format!("      <imagedata>{}</imagedata>\n", escape_xml_text(image)));
        },
        Annotation::Ink(ink) if !ink.ink_list.is_empty() => {
            children.push_str("      <inklist>\n");
            for gesture in &ink.ink_list { children.push_str(&format!("        <gesture>{}</gesture>\n", escape_xml_text(gesture))); }
            children.push_str("      </inklist>\n");
        }
        Annotation::Link(link) => if let Some(target) = &link.target { children.push_str(&link_target_xml(target)?); },
        _ => {}
    }
    let mut extra = annotation.base().extra.iter().filter(|(k,_)| k.starts_with("_xml:")).collect::<Vec<_>>();
    extra.sort_by(|a,b| a.0.cmp(b.0));
    for (_,raw) in extra { children.push_str("      "); children.push_str(raw); children.push('\n'); }
    let attrs = attrs_to_string(&attrs);
    if children.is_empty() { Ok(format!("    <{tag}{attrs}/>\n")) }
    else { Ok(format!("    <{tag}{attrs}>\n{children}    </{tag}>\n")) }
}

fn link_target_xml(target: &LinkTarget) -> Result<String> {
    Ok(match target {
        LinkTarget::Uri { uri, is_map } => format!("      <OnActivation><Action><URI Name=\"{}\" IsMap=\"{}\"/></Action></OnActivation>\n", escape_xml_attr(uri), is_map),
        LinkTarget::Named { name } => format!("      <Dest><Named Name=\"{}\"/></Dest>\n", escape_xml_attr(name)),
        LinkTarget::Page { page, view } => {
            validate_link_view(view)?;
            let mut attrs = vec![("Page".into(), page.to_string())];
            for (name,value) in view.mode.parameter_names().iter().zip(&view.parameters) {
                if let Some(value) = value { attrs.push(((*name).into(), value.to_string())); }
            }
            format!("      <Dest><{}{}/></Dest>\n", view.mode.name(), attrs_to_string(&attrs))
        }
    })
}

pub(crate) fn validate_link_view(view: &LinkDestinationView) -> Result<()> {
    if view.parameters.len() != view.mode.parameter_names().len()
        || view.parameters.iter().flatten().any(|n| !n.is_finite() || !(*n as f32).is_finite())
        || (view.mode == LinkDestinationMode::FitR && view.parameters.iter().any(Option::is_none)) {
        return Err(PdfXmlError::InvalidXfdfFormat("无效 Link 目的地参数".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_xfdf() {
        let xml = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>",
            "<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">",
            "  <annots>",
            "    <text subject=\"Test Comment\" page=\"0\" rect=\"100,600,300,650\"",
            "          title=\"Author\" date=\"D:20240101120000\" color=\"#FFFF00\">",
            "      This is a test comment",
            "    </text>",
            "  </annots>",
            "</xfdf>"
        );

        let doc = XfdfDocument::parse(xml).unwrap();
        assert_eq!(doc.annotations.len(), 1);
        
        match &doc.annotations[0] {
            Annotation::Text(text) => {
                assert_eq!(text.base.subject.as_deref(), Some("Test Comment"));
                assert_eq!(text.base.page, 0);
                assert_eq!(text.base.contents.as_deref(), Some("This is a test comment"));
            }
            _ => panic!("Expected Text annotation"),
        }
    }

    #[test]
    fn test_parse_multiple_annotations() {
        let xml = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>",
            "<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">",
            "  <annots>",
            "    <highlight page=\"0\" rect=\"50,700,200,720\" color=\"#FFFF00\"/>",
            "    <underline page=\"1\" rect=\"50,500,150,520\"/>",
            "    <square page=\"0\" rect=\"300,400,450,550\" width=\"2\" color=\"#0000FF\"/>",
            "  </annots>",
            "</xfdf>"
        );

        let doc = XfdfDocument::parse(xml).unwrap();
        assert_eq!(doc.annotations.len(), 3);
        assert_eq!(doc.total_pages(), 2);  // 页码 0 和 1
    }

    #[test]
    fn test_rect_parsing() {
        let rect = Rect::from_string("100,200,300,400").unwrap();
        assert!((rect.left - 100.0).abs() < f64::EPSILON);
        assert!((rect.bottom - 200.0).abs() < f64::EPSILON);
        assert!((rect.right - 300.0).abs() < f64::EPSILON);
        assert!((rect.top - 400.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_to_xfdf_string() {
        let doc = XfdfDocument {
            xmlns: Some("http://ns.adobe.com/xfdf/".to_string()),
            fields: Vec::new(),
            annotations: vec![Annotation::Text(TextAnnotation {
                base: AnnotationBase {
                    name: Some("annot-1".to_string()),
                    page: 0,
                    rect: Some(Rect {
                        left: 100.0,
                        bottom: 600.0,
                        right: 300.0,
                        top: 650.0,
                    }),
                    title: Some("Author".to_string()),
                    subject: Some("Test Comment".to_string()),
                    contents: Some("Hello <XFDF> & PDF".to_string()),
                    creation_date: None,
                    modification_date: Some("D:20240101120000".to_string()),
                    color: Some("#FFFF00".to_string()),
                    opacity: 1.0,
                    flags: 0,
                    intent: None,
                    border_style: None,
                    dashes: None,
                    extra: HashMap::new(),
                },
                open: false,
                icon_type: "Note".to_string(),
            })],
            metadata: HashMap::new(),
            ..XfdfDocument::default()
        };

        let xml = doc.to_xfdf_string().unwrap();
        assert!(xml.contains("<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">"));
        assert!(xml.contains("<annots>"));
        assert!(xml.contains("<contents>Hello &lt;XFDF&gt; &amp; PDF</contents>"));
        assert!(xml.contains("<text name=\"annot-1\" page=\"0\" rect=\"100,600,300,650\" title=\"Author\" subject=\"Test Comment\" date=\"D:20240101120000\" color=\"#FFFF00\">"));
    }

    #[test]
    fn test_extract_plain_text_from_richtext_preserves_line_breaks() {
        let rich = "<body><p>Hello&nbsp;world</p><div>Second<br/>line &amp; more</div></body>";
        let text = extract_plain_text_from_richtext(rich);
        assert_eq!(text, "Hello world\nSecond\nline & more");
    }
}
