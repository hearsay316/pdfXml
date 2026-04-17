//! 这个文件负责把 XFDF/XML 和 Rust 数据结构互相转换。
//!
//! 如果把项目想成一条流水线：
//! 1. `xfdf.rs` 负责“读懂 XML”
//! 2. `annotation.rs` 负责“定义数据长什么样”
//! 3. `pdf.rs` 负责“把这些数据写进 PDF”
//!
//! 阅读建议：
//! 1. 先看 `XfdfDocument`
//! 2. 再看 `parse`
//! 3. 最后看 `to_xfdf_string`
//! 这样最容易看懂“进来是什么，出去又是什么”。

use crate::annotation::*;
use crate::error::{PdfXmlError, Result};
use log::{debug, warn};
use quick_xml::events::Event;
use quick_xml::Reader;
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

fn extract_plain_text_from_richtext(input: &str) -> String {
    // 有些 XFDF 会把内容放在富文本里，里面带 HTML 标签。
    // 这里尽量保留换行/段落语义，再做基础实体解码。
    let normalized_breaks = regex::Regex::new(r"(?is)<\s*br\s*/?\s*>")
        .unwrap()
        .replace_all(input, "\n")
        .to_string();
    let normalized_blocks = regex::Regex::new(r"(?is)</\s*(p|div|li|tr|h[1-6])\s*>")
        .unwrap()
        .replace_all(&normalized_breaks, "\n")
        .to_string();
    let no_tags = regex::Regex::new(r"(?is)<[^>]+>")
        .unwrap()
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
#[derive(Debug, Clone)]
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
}

/// XFDF 表单字段
#[derive(Debug, Clone)]
/// XFDF 里的表单字段。
pub struct XfdfField {
    #[allow(dead_code)]
    pub name: String,
    pub value: Option<String>,
    pub children: Vec<XfdfField>,
}

impl XfdfDocument {
    // 这是这个文件里最重要的入口。
    // 它一边读 XML，一边把读到的内容整理成程序能直接使用的结构。
    //
    // 因为 XML 不是一次性整块读完的，
    // 所以这里会看到很多“临时变量”。
    // 它们的作用很像便签纸：
    // 先把读到一半的信息记下来，等后面拼完整了再放进最终结果。
    /// 解析 XFDF/XML 字符串，得到统一的文档对象。
    pub fn parse(xml_str: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml_str);
        reader.config_mut().trim_text(true);
        
        let mut doc = XfdfDocument {
            xmlns: None,
            fields: Vec::new(),
            annotations: Vec::new(),
            metadata: HashMap::new(),
        };
        
        // 下面这些变量就是“解析过程中的临时工作台”。
        // 因为 XML 是一段一段读出来的，所以要先把中间状态存起来，
        // 等读完整个注释后，再一次性组装成最终对象。
        let mut buf = Vec::new();
        let mut in_annots = false;
        let mut in_fields = false;
        let mut current_field_stack: Vec<XfdfField> = Vec::new();
        let mut current_annotation_attrs: HashMap<String, String> = HashMap::new();
        let mut current_annotation_content: String = String::new();
        let mut current_annotation_type: Option<String> = None;

        // 追踪注释内部子元素
        let mut in_inklist = false;
        let mut current_gesture_data: String = String::new();
        let mut inklist_gestures: Vec<String> = Vec::new();
        let mut current_child_tag: Option<String> = None;  // 当前正在处理的子标签名
        let mut child_tag_content: String = String::new();  // 子标签的文本内容

        loop {
            // quick-xml 每次只吐出一个事件：
            // 可能是开始标签、结束标签、文本、空标签等。
            // 我们就是在这里一边读事件，一边决定该把数据放到哪里。
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let tag_name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                    
                    debug!("开始标签: <{}>", tag_name);
                    
                    match tag_name.as_ref() {
                        "xfdf" => {
                            for attr in e.attributes().filter_map(|a| a.ok()) {
                                let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                                let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();

                                match key.as_str() {
                                    "xmlns" => doc.xmlns = Some(value),
                                    _ => { doc.metadata.insert(key, value); }
                                }
                            }
                        }
                        "f" | "field" => {
                            in_fields = true;
                            let field_name = e.attributes()
                                .filter_map(|a| a.ok())
                                .find(|a| String::from_utf8_lossy(a.key.as_ref()) == "name")
                                .map(|a| String::from_utf8_lossy(&a.value).to_string())
                                .unwrap_or_else(|| format!("field_{}", current_field_stack.len()));
                            
                            current_field_stack.push(XfdfField {
                                name: field_name,
                                value: None,
                                children: Vec::new(),
                            });
                        }
                        "value" => {}
                        "annots" => {
                            in_annots = true;
                            debug!("进入 annots 区域");
                        }
                        // 注释类型标签
                        annot_type if Self::is_annotation_tag(annot_type) && in_annots => {
                            debug!("发现注释类型: {}", annot_type);
                            current_annotation_type = Some(annot_type.to_string());
                            current_annotation_content.clear();
                            inklist_gestures.clear();
                            
                            // 收集属性
                            current_annotation_attrs.clear();
                            for attr in e.attributes().filter_map(|a| a.ok()) {
                                let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                                let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                                debug!("  属性: {} = {}", key, value);
                                current_annotation_attrs.insert(key, value);
                            }
                        }
                        // 注释内部的子元素
                        child_tag if current_annotation_type.is_some() && in_annots => {
                            match child_tag {
                                "inklist" => {
                                    in_inklist = true;
                                    inklist_gestures.clear();
                                    debug!("进入 inklist");
                                }
                                "gesture" if in_inklist => {
                                    current_gesture_data.clear();
                                    debug!("开始 gesture");
                                }
                                "contents" | "contents-richtext" | "defaultstyle" | "defaultappearance"
                                | "trn-custom-data" | "imagedata" => {
                                    // 需要收集这些子元素的文本内容
                                    current_child_tag = Some(child_tag.to_string());
                                    child_tag_content.clear();
                                }
                                "popup" => {
                                    // popup 作为子元素忽略（它是独立注解，但嵌套在父注解 XML 中）
                                    debug!("跳过嵌套 popup");
                                }
                                _ => {
                                    debug!("未知子元素: {}", child_tag);
                                }
                            }
                        }
                        "popup" if in_annots && current_annotation_type.is_none() => {
                            current_annotation_type = Some("popup".to_string());
                            current_annotation_content.clear();
                            current_annotation_attrs.clear();
                            for attr in e.attributes().filter_map(|a| a.ok()) {
                                let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                                let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                                current_annotation_attrs.insert(key, value);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let tag_name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                    debug!("空标签: <{}/>", tag_name);

                    if tag_name == "value" && !current_field_stack.is_empty() {
                        if let Some(field) = current_field_stack.last_mut() {
                            field.value = Some(String::new());
                        }
                    }

                    // 自闭合的 popup 标签（如 <popup ... />）
                    if tag_name == "popup" && current_annotation_type.is_some() {
                        debug!("自闭合 popup 子元素，跳过");
                        continue;
                    }

                    if in_annots && Self::is_annotation_tag(&tag_name) {
                        let mut attrs = HashMap::new();
                        for attr in e.attributes().filter_map(|a| a.ok()) {
                            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                            let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                            attrs.insert(key, value);
                        }

                        match Self::build_annotation(&tag_name, &attrs, "") {
                            Ok(annotation) => {
                                doc.annotations.push(annotation);
                                debug!("成功解析自闭合 {} 注释", tag_name);
                            }
                            Err(err) => {
                                warn!("解析自闭合 {} 注解失败: {}", tag_name, err);
                            }
                        }
                    }
                }
                Ok(Event::Text(e)) => {
                    let text = e.unescape()?;
                    debug!("文本内容: [{}]", if text.len() > 100 { &text[..100] } else { &text });
                    
                    if in_fields {
                        if let Some(field) = current_field_stack.last_mut() {
                            if field.value.is_none() || field.value.as_deref() == Some("") {
                                field.value = Some(text.to_string());
                            }
                        }
                    }
                    
                    if current_annotation_type.is_some() {
                        if current_child_tag.is_some() {
                            // 子元素的内容单独收集
                            child_tag_content.push_str(&text);
                        } else if in_inklist {
                            // 在 inklist/gesture 中，文本是坐标数据
                            current_gesture_data.push_str(text.trim());
                        } else {
                            // 顶层文本作为 contents（但会被 <contents> 标签覆盖）
                            if !text.trim().is_empty() {
                                current_annotation_content.push_str(&text);
                            }
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let tag_name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                    debug!("结束标签: </{}>", tag_name);
                    
                    match tag_name.as_ref() {
                        "f" | "field" => {
                            if let Some(field) = current_field_stack.pop() {
                                if let Some(parent) = current_field_stack.last_mut() {
                                    parent.children.push(field);
                                } else {
                                    doc.fields.push(field);
                                }
                            }
                        }
                        "fields" => {
                            in_fields = false;
                        }
                        "annots" => {
                            in_annots = false;
                        }
                        "inklist" => {
                            in_inklist = false;
                            // 将 inklist 数据存入 attrs，供 build_annotation 使用
                            if !inklist_gestures.is_empty() {
                                current_annotation_attrs.insert(
                                    "_inklist".to_string(), 
                                    inklist_gestures.join("\x1b")
                                );
                            }
                            debug!("inklist 结束, 共 {} 条手势", inklist_gestures.len());
                        }
                        "gesture" if in_inklist => {
                            if !current_gesture_data.is_empty() {
                                inklist_gestures.push(current_gesture_data.clone());
                                debug!("gesture 数据: {} 个点", 
                                    current_gesture_data.matches(';').count());
                            }
                            current_gesture_data.clear();
                        }
                        // 处理子元素结束 - 将内容存入 attrs
                        "contents" | "contents-richtext" | "defaultstyle" | "defaultappearance"
                        | "trn-custom-data" | "imagedata" => {
                            if current_child_tag.as_deref() == Some(tag_name.as_str()) {
                                if tag_name == "contents" {
                                    // contents 文本覆盖顶层 content
                                    current_annotation_content = child_tag_content.clone();
                                } else if tag_name == "contents-richtext" {
                                    current_annotation_attrs.insert(tag_name.to_string(), child_tag_content.clone());
                                } else {
                                    // 其他子元素存入 attrs
                                    current_annotation_attrs.insert(tag_name.to_string(), child_tag_content.clone());
                                }
                                current_child_tag = None;
                                child_tag_content.clear();
                            }
                        }
                        annot_type if Self::is_annotation_tag(annot_type) && current_annotation_type.as_deref() == Some(annot_type) => {
                            // 完成当前注释的解析
                            if let Some(typ) = current_annotation_type.take() {
                                // 将 _inklist 从 attrs 中取出特殊处理
                                let result = Self::build_annotation_with_children(
                                    &typ,
                                    &current_annotation_attrs,
                                    &current_annotation_content,
                                );
                                match result {
                                    Ok(annotation) => {
                                        doc.annotations.push(annotation);
                                        debug!("成功解析 {} 注释", typ);
                                    }
                                    Err(e) => {
                                        warn!("解析 {} 注解失败: {}", typ, e);
                                    }
                                }
                            }
                            current_annotation_attrs.clear();
                            current_annotation_content.clear();
                            inklist_gestures.clear();
                            current_child_tag = None;
                            child_tag_content.clear();
                        }
                        _ => {}
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(PdfXmlError::XmlParse(e));
                }
                _ => {}
            }
            
            buf.clear();
        }
        
        debug!(
            "解析完成: {} 个字段, {} 条注释",
            doc.fields.len(),
            doc.annotations.len()
        );
        
        Ok(doc)
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
        let base = Self::build_base(attrs, content)?;
        
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
    fn build_base(attrs: &HashMap<String, String>, content: &str) -> Result<AnnotationBase> {
        // 这里负责把“零散的字符串属性”整理成统一的基础字段。
        // 例如 page、rect、title、color 这些，
        // 不管最终是 Text 还是 Highlight，都会先经过这里。
        let contents = if content.trim().is_empty() {
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
                .and_then(|v| u32::from_str_radix(v, 16).ok())
                .unwrap_or_default(),
            extra: attrs.iter()
                .filter(|(k, _)| !Self::is_known_attr(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        })
    }
    
    /// 判断是否为已知的基础属性
    fn is_known_attr(key: &str) -> bool {
        matches!(key,
            "name" | "page" | "rect" | "title" | "subject" |
            "creationdate" | "date" | "color" | "opacity" | "flags" |
            "open" | "icon" | "width" | "defaultstyle" | "defaultappearance" | "align" |
            "start" | "end" | "head" | "tail" | "vertices" |
            "interiorcolor" | "parent" | "coords" | "TextColor" |
            "contents-richtext" | "_inklist" | "imagedata"
        )
    }

    /// 获取指定页面上的所有注释。
    pub fn get_annotations_for_page(&self, page: usize) -> Vec<&Annotation> {
        self.annotations.iter().filter(|a| a.page() == page).collect()
    }

    /// 返回文档总页数。
    pub fn total_pages(&self) -> usize {
        self.annotations
            .iter()
            .map(|a| a.page())
            .max()
            .map(|p| p + 1)
            .unwrap_or(1)
    }
    // 这个函数和 `parse` 正好相反。
    // `parse` 是把 XML 变成 Rust 对象；
    // 这里则是把 Rust 对象重新拼回标准 XFDF 字符串。
    /// 把当前文档重新序列化成 XFDF 字符串。
    pub fn to_xfdf_string(&self) -> Result<String> {
        // 这个函数和 `parse` 正好相反。
        // `parse` 是把 XML 变成 Rust 对象；
        // 这里则是把 Rust 对象重新拼回标准 XFDF 字符串。
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n");
        xml.push_str("<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n");

        if !self.annotations.is_empty() {
            xml.push_str("  <annots>\n");
            for annotation in &self.annotations {
                xml.push_str(&annotation_to_xfdf_element(annotation)?);
            }
            xml.push_str("  </annots>\n");
        }

        xml.push_str("</xfdf>\n");
        Ok(xml)
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

fn format_opacity(opacity: f32) -> String {
    let mut s = format!("{:.3}", opacity);
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

fn push_base_attrs(attrs: &mut Vec<(String, String)>, base: &AnnotationBase) {
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
        attrs.push(("flags".to_string(), format!("{:X}", base.flags)));
    }
    for (key, value) in &base.extra {
        attrs.push((key.clone(), value.clone()));
    }
}

fn attrs_to_string(attrs: &[(String, String)]) -> String {
    attrs
        .iter()
        .map(|(key, value)| format!(" {}=\"{}\"", key, escape_xml_attr(value)))
        .collect::<String>()
}

fn annotation_to_xfdf_element(annotation: &Annotation) -> Result<String> {
    let (tag, attrs, contents) = match annotation {
        Annotation::Text(text) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &text.base);
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
            push_base_attrs(&mut attrs, &highlight.base);
            if let Some(coords) = &highlight.coords {
                attrs.push(("coords".to_string(), coords.clone()));
            }
            ("highlight", attrs, highlight.base.contents.clone())
        }
        Annotation::Underline(underline) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &underline.base);
            if let Some(coords) = &underline.coords {
                attrs.push(("coords".to_string(), coords.clone()));
            }
            ("underline", attrs, underline.base.contents.clone())
        }
        Annotation::StrikeOut(strikeout) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &strikeout.base);
            if let Some(coords) = &strikeout.coords {
                attrs.push(("coords".to_string(), coords.clone()));
            }
            ("strikeout", attrs, strikeout.base.contents.clone())
        }
        Annotation::Squiggly(squiggly) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &squiggly.base);
            if let Some(coords) = &squiggly.coords {
                attrs.push(("coords".to_string(), coords.clone()));
            }
            ("squiggly", attrs, squiggly.base.contents.clone())
        }
        Annotation::FreeText(freetext) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &freetext.base);
            if let Some(default_style) = &freetext.default_style {
                attrs.push(("defaultstyle".to_string(), default_style.clone()));
            }
            if let Some(default_appearance) = &freetext.default_appearance {
                attrs.push(("defaultappearance".to_string(), default_appearance.clone()));
            }
            if let Some(text_color) = &freetext.text_color {
                attrs.push(("TextColor".to_string(), text_color.clone()));
            }
            if freetext.align != 0 {
                attrs.push(("align".to_string(), freetext.align.to_string()));
            }
            ("freetext", attrs, freetext.base.contents.clone())
        }
        Annotation::Square(square) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &square.base);
            if (square.width - 1.0).abs() > f32::EPSILON {
                attrs.push(("width".to_string(), format_opacity(square.width)));
            }
            ("square", attrs, square.base.contents.clone())
        }
        Annotation::Circle(circle) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &circle.base);
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
            push_base_attrs(&mut attrs, &line.base);
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
            push_base_attrs(&mut attrs, &polygon.base);
            if let Some(vertices) = &polygon.vertices {
                attrs.push(("vertices".to_string(), vertices.clone()));
            }
            let tag = if polygon.is_closed { "polygon" } else { "polyline" };
            (tag, attrs, polygon.base.contents.clone())
        }
        Annotation::Ink(ink) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &ink.base);
            if (ink.width - 1.0).abs() > f32::EPSILON {
                attrs.push(("width".to_string(), format_opacity(ink.width)));
            }
            let mut xml = format!("    <ink{}>\n", attrs_to_string(&attrs));
            if let Some(contents) = &ink.base.contents {
                xml.push_str(&format!("      {}\n", escape_xml_text(contents)));
            }
            if !ink.ink_list.is_empty() {
                xml.push_str("      <inklist>\n");
                for gesture in &ink.ink_list {
                    xml.push_str(&format!("        <gesture>{}</gesture>\n", escape_xml_text(gesture)));
                }
                xml.push_str("      </inklist>\n");
            }
            xml.push_str("    </ink>\n");
            return Ok(xml);
        }
        Annotation::Stamp(stamp) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &stamp.base);
            if !stamp.icon.is_empty() {
                attrs.push(("icon".to_string(), stamp.icon.clone()));
            }
            if let Some(image_data) = &stamp.image_data {
                attrs.push(("imagedata".to_string(), image_data.clone()));
            }
            ("stamp", attrs, stamp.base.contents.clone())
        }
        Annotation::Popup(popup) => {
            let mut attrs = Vec::new();
            push_base_attrs(&mut attrs, &popup.base);
            if popup.open {
                attrs.push(("open".to_string(), "yes".to_string()));
            }
            if let Some(parent_name) = &popup.parent_name {
                attrs.push(("parent".to_string(), parent_name.clone()));
            }
            ("popup", attrs, popup.base.contents.clone())
        }
    };

    let attrs = attrs_to_string(&attrs);
    match contents {
        Some(contents) if !contents.is_empty() => Ok(format!(
            "    <{tag}{attrs}>{}</{tag}>\n",
            escape_xml_text(&contents)
        )),
        _ => Ok(format!("    <{tag}{attrs}/>\n")),
    }
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
                    extra: HashMap::new(),
                },
                open: false,
                icon_type: "Note".to_string(),
            })],
            metadata: HashMap::new(),
        };

        let xml = doc.to_xfdf_string().unwrap();
        assert!(xml.contains("<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">"));
        assert!(xml.contains("<annots>"));
        assert!(xml.contains("<text name=\"annot-1\" page=\"0\" rect=\"100,600,300,650\" title=\"Author\" subject=\"Test Comment\" date=\"D:20240101120000\" color=\"#FFFF00\">Hello &lt;XFDF&gt; &amp; PDF</text>"));
    }

    #[test]
    fn test_extract_plain_text_from_richtext_preserves_line_breaks() {
        let rich = "<body><p>Hello&nbsp;world</p><div>Second<br/>line &amp; more</div></body>";
        let text = extract_plain_text_from_richtext(rich);
        assert_eq!(text, "Hello world\nSecond\nline & more");
    }
}
