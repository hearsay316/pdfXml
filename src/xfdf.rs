//! XFDF (XML Forms Data Format) 解析模块
//!
//! XFDF 是 Adobe 定义的 XML 格式，用于表示 PDF 文档中的注释和表单数据。
//! 
//! 示例 XFDF 文件结构:
//! ```xml
//! <?xml version="1.0" encoding="UTF-8" ?>
//! <xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
//!   <annots>
//!     <text subject="Comment" page="0" rect="100,200,300,400"
//!           title="Author" date="D:20240101120000" color="#FF0000">
//!       This is a comment
//!     </text>
//!   </annots>
//! </xfdf>
//! ```

use crate::annotation::*;
use crate::error::{PdfXmlError, Result};
use log::{debug, warn};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;

fn extract_plain_text_from_richtext(input: &str) -> String {
    let no_tags = regex::Regex::new(r"<[^>]+>")
        .unwrap()
        .replace_all(input, " ")
        .to_string();
    let decoded = no_tags
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// XFDF 文档
#[derive(Debug, Clone)]
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
pub struct XfdfField {
    pub name: String,
    pub value: Option<String>,
    pub children: Vec<XfdfField>,
}

impl XfdfDocument {
    /// 解析 XFDF/XML 字符串为文档对象
    pub fn parse(xml_str: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml_str);
        reader.config_mut().trim_text(true);
        
        let mut doc = XfdfDocument {
            xmlns: None,
            fields: Vec::new(),
            annotations: Vec::new(),
            metadata: HashMap::new(),
        };
        
        let mut buf = Vec::new();
        let mut current_path: Vec<String> = Vec::new();
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
                        if let Some(ref child) = current_child_tag {
                            // 子元素的内容单独收集
                            child_tag_content.push_str(&text);
                        } else if in_inklist {
                            // 在 inklist/gesture 中，文本是坐标数据
                            current_gesture_data.push_str(&text.trim());
                        } else {
                            // 顶层文本作为 contents（但会被 <contents> 标签覆盖）
                            if text.trim().len() > 0 {
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
                                    if current_annotation_content.trim().is_empty() {
                                        current_annotation_attrs.insert(tag_name.to_string(), child_tag_content.clone());
                                    } else {
                                        current_annotation_attrs.insert(tag_name.to_string(), child_tag_content.clone());
                                    }
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
    
    /// 判断是否为已知的注释类型标签
    fn is_annotation_tag(tag: &str) -> bool {
        matches!(tag,
            "text" | "highlight" | "underline" | "strikeout" | "squiggly" |
            "freetext" | "square" | "circle" | "line" |
            "polygon" | "polyline" | "ink" | "stamp" |
            "caret" | "fileattachment" | "sound" | "link" |
            "popup" | "widget"
        )
    }
    
    /// 构建注释对象
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
    
    /// 构建基础注释属性
    fn build_base(attrs: &HashMap<String, String>, content: &str) -> Result<AnnotationBase> {
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
            "contents-richtext" | "_inklist"
        )
    }

    /// 获取指定页码的所有注释
    pub fn get_annotations_for_page(&self, page: usize) -> Vec<&Annotation> {
        self.annotations.iter().filter(|a| a.page() == page).collect()
    }

    /// 获取总页数（基于注释中最大的页码 + 1）
    pub fn total_pages(&self) -> usize {
        self.annotations.iter().map(|a| a.page()).max().map(|p| p + 1).unwrap_or(1)
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
    fn test_color_parsing() {
        let color = Color::from_hex("#FF0000").unwrap();
        assert!((color.r - 1.0).abs() < f32::EPSILON);
        assert!((color.g - 0.0).abs() < f32::EPSILON);
        assert!((color.b - 0.0).abs() < f32::EPSILON);
    }
}
