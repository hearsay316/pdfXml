//! Bounded XML parsing and XFDF structure validation. No external entities or resources are loaded.
use super::*;
use quick_xml::events::Event;
use quick_xml::NsReader;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub(super) struct XmlNode {
    name: String,
    local: String,
    namespace: Option<String>,
    attrs: BTreeMap<String, String>,
    namespaces: BTreeMap<String, String>,
    children: Vec<XmlPart>,
    offset: u64,
}

#[derive(Debug)]
enum XmlPart {
    Element(XmlNode),
    Text(String),
}

impl XmlNode {
    fn elements(&self) -> impl Iterator<Item = &XmlNode> {
        self.children.iter().filter_map(|part| match part {
            XmlPart::Element(node) => Some(node),
            _ => None,
        })
    }

    fn text(&self) -> String {
        let mut output = String::new();
        for child in &self.children {
            match child {
                XmlPart::Text(text) => output.push_str(text),
                XmlPart::Element(node) => output.push_str(&node.text()),
            }
        }
        output
    }

    fn direct_text(&self) -> String {
        self.children
            .iter()
            .filter_map(|part| match part {
                XmlPart::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn xml(&self) -> String {
        self.render(true)
    }

    fn render(&self, include_scope: bool) -> String {
        let mut attrs = self.attrs.clone();
        if include_scope {
            for (key, value) in &self.namespaces {
                attrs.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
        let attrs = attrs.into_iter().collect::<Vec<_>>();
        let mut xml = format!("<{}{}>", self.name, attrs_to_string(&attrs));
        for child in &self.children {
            match child {
                XmlPart::Text(text) => xml.push_str(&escape_xml_text(text)),
                XmlPart::Element(node) => xml.push_str(&node.render(false)),
            }
        }
        xml.push_str(&format!("</{}>", self.name));
        xml
    }

    fn inner_xml(&self) -> String {
        self.children
            .iter()
            .map(|part| match part {
                XmlPart::Text(text) => escape_xml_text(text),
                XmlPart::Element(node) => node.xml(),
            })
            .collect()
    }
}

fn invalid(message: impl Into<String>) -> PdfXmlError {
    PdfXmlError::InvalidXfdfFormat(message.into())
}

fn check_xml_chars(text: &str) -> Result<()> {
    if text.chars().any(|c| !matches!(c, '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')) {
        return Err(invalid("XML 包含非法字符"));
    }
    Ok(())
}

// XML 1.0 NCName: namespace prefixes and local names cannot contain a colon.
fn valid_ncname(name: &str) -> bool {
    fn start(c: char) -> bool {
        matches!(c, 'A'..='Z' | '_' | 'a'..='z' | '\u{C0}'..='\u{D6}' |
            '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}' |
            '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' |
            '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' |
            '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
    }
    let mut chars = name.chars();
    chars.next().is_some_and(start)
        && chars.all(|c| {
            start(c)
                || matches!(c,
        '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
        })
}

fn expanded_name(
    name: &str,
    namespaces: &BTreeMap<String, String>,
    attribute: bool,
) -> Result<(Option<String>, String)> {
    if let Some((prefix, local)) = name.split_once(':') {
        if !valid_ncname(prefix) || !valid_ncname(local) || prefix == "xmlns" {
            return Err(invalid(format!("非法 XML 限定名: {name}")));
        }
        let namespace = if prefix == "xml" {
            "http://www.w3.org/XML/1998/namespace"
        } else {
            namespaces
                .get(&format!("xmlns:{prefix}"))
                .map(String::as_str)
                .filter(|ns| !ns.is_empty())
                .ok_or_else(|| invalid(format!("未声明的命名空间前缀: {prefix}")))?
        };
        Ok((Some(namespace.into()), local.into()))
    } else {
        if !valid_ncname(name) {
            return Err(invalid(format!("非法 XML 名称: {name}")));
        }
        let namespace = if attribute {
            None
        } else {
            namespaces.get("xmlns").filter(|ns| !ns.is_empty()).cloned()
        };
        Ok((namespace, name.into()))
    }
}

fn validate_namespace_binding(key: &str, value: &str) -> Result<()> {
    const XML: &str = "http://www.w3.org/XML/1998/namespace";
    const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
    if key == "xmlns:xml" {
        if value != XML {
            return Err(invalid("xml 前缀必须绑定到标准 XML 命名空间"));
        }
    } else if value == XML || value == XMLNS {
        return Err(invalid("保留命名空间不能绑定到默认或其他前缀"));
    }
    if let Some(prefix) = key.strip_prefix("xmlns:") {
        if !valid_ncname(prefix) || prefix == "xmlns" || value.is_empty() {
            return Err(invalid(format!("非法 XML 命名空间声明: {key}")));
        }
    }
    Ok(())
}

fn append_node(stack: &mut [XmlNode], root: &mut Option<XmlNode>, node: XmlNode) -> Result<()> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(XmlPart::Element(node));
    } else if root.replace(node).is_some() {
        return Err(invalid("XML 只能有一个根元素"));
    }
    Ok(())
}

/// Used by serialization too, so manually constructed extra attributes/raw XML cannot corrupt output.
pub(super) fn read_xml(xml: &str, options: &XfdfParseOptions) -> Result<XmlNode> {
    if xml.len() > options.max_input_bytes {
        return Err(invalid(format!(
            "XML 超过输入字节限制 {}",
            options.max_input_bytes
        )));
    }
    check_xml_chars(xml)?;
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_comments = true;
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root = None;
    let mut node_count = 0usize;
    let mut saw_declaration = false;
    loop {
        let offset = reader.buffer_position();
        let decoder = reader.decoder();
        let event = reader.read_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                node_count += 1;
                if node_count > options.max_nodes || stack.len() + 1 > options.max_depth {
                    return Err(invalid("XML 节点数量或嵌套深度超过限制"));
                }
                let mut attrs = BTreeMap::new();
                let mut namespaces = stack
                    .last()
                    .map(|n| n.namespaces.clone())
                    .unwrap_or_default();
                for attr in element.attributes() {
                    let attr = attr.map_err(|error| invalid(format!("非法 XML 属性: {error}")))?;
                    let key = decoder.decode(attr.key.as_ref())?.into_owned();
                    if attr.value.as_ref().contains(&b'<') {
                        return Err(invalid("XML 属性值不能包含未转义的 <"));
                    }
                    let value = attr.decode_and_unescape_value(decoder)?.into_owned();
                    check_xml_chars(&value)?;
                    if key == "xmlns" || key.starts_with("xmlns:") {
                        validate_namespace_binding(&key, &value)?;
                        namespaces.insert(key.clone(), value.clone());
                    }
                    attrs.insert(key, value);
                }
                let mut expanded_attrs = BTreeSet::new();
                for key in attrs
                    .keys()
                    .filter(|key| key.as_str() != "xmlns" && !key.starts_with("xmlns:"))
                {
                    let expanded = expanded_name(key, &namespaces, true)?;
                    if !expanded_attrs.insert(expanded) {
                        return Err(invalid(format!("命名空间解析后重复的 XML 属性: {key}")));
                    }
                }
                let name = decoder.decode(element.name().as_ref())?.into_owned();
                let (namespace, local) = expanded_name(&name, &namespaces, false)?;
                let node = XmlNode {
                    name,
                    local,
                    namespace,
                    attrs,
                    namespaces,
                    children: Vec::new(),
                    offset,
                };
                if matches!(event, Event::Empty(_)) {
                    append_node(&mut stack, &mut root, node)?;
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                let node = stack.pop().ok_or_else(|| invalid("不匹配的结束标签"))?;
                append_node(&mut stack, &mut root, node)?;
            }
            Event::Text(text) => {
                if text.as_ref().windows(3).any(|w| w == b"]]>") {
                    return Err(invalid("普通 XML 文本不能包含 ]]>"));
                }
                let text = text.unescape()?.into_owned();
                check_xml_chars(&text)?;
                if let Some(node) = stack.last_mut() {
                    node.children.push(XmlPart::Text(text));
                } else if !text.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n')) {
                    return Err(invalid("根元素外出现文本"));
                }
            }
            Event::CData(text) => {
                let text = decoder.decode(text.as_ref())?.into_owned();
                if let Some(node) = stack.last_mut() {
                    node.children.push(XmlPart::Text(text));
                } else {
                    return Err(invalid("CDATA 必须位于根元素内"));
                }
            }
            Event::DocType(_) => return Err(invalid("不允许 DTD 或外部实体")),
            Event::Decl(declaration) => {
                if saw_declaration || offset != 0 || root.is_some() || !stack.is_empty() {
                    return Err(invalid("XML 声明只能在文档开头出现一次"));
                }
                saw_declaration = true;
                if declaration.version()?.as_ref() != b"1.0" {
                    return Err(invalid("当前只支持 XML 1.0"));
                }
                if let Some(encoding) = declaration.encoding() {
                    let encoding = encoding.map_err(PdfXmlError::XmlParse)?;
                    if !encoding.eq_ignore_ascii_case(b"UTF-8")
                        && !encoding.eq_ignore_ascii_case(b"UTF8")
                    {
                        return Err(invalid("字符串解析仅支持 UTF-8；请先将源文件转码为 UTF-8"));
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(invalid("XML 文档未完整闭合"));
    }
    root.ok_or_else(|| invalid("XML 缺少根元素"))
}

struct Context<'a> {
    options: &'a XfdfParseOptions,
    diagnostics: Vec<XfdfDiagnostic>,
    allow_unbound: bool,
    annotation_count: usize,
}

impl Context<'_> {
    fn is_xfdf(&self, node: &XmlNode) -> bool {
        node.namespace.as_deref() == Some(XFDF_NAMESPACE)
            || (node.namespace.is_none() && self.allow_unbound)
    }

    fn issue(
        &mut self,
        node: &XmlNode,
        code: XfdfDiagnosticCode,
        message: impl Into<String>,
    ) -> Result<()> {
        let severity = if code == XfdfDiagnosticCode::PreservedExtension {
            XfdfDiagnosticSeverity::Info
        } else {
            XfdfDiagnosticSeverity::Warning
        };
        let diagnostic = XfdfDiagnostic {
            severity,
            code,
            byte_offset: node.offset,
            element: node.local.clone(),
            annotation_name: node.attrs.get("name").cloned(),
            message: message.into(),
        };
        if self.options.mode == XfdfParseMode::Strict && severity == XfdfDiagnosticSeverity::Warning
        {
            return Err(invalid(format!(
                "{:?} <{}> at byte {}: {}",
                code, node.local, node.offset, diagnostic.message
            )));
        }
        self.diagnostics.push(diagnostic);
        Ok(())
    }

    fn fields(&mut self, node: &XmlNode, output: &mut Vec<XfdfField>) -> Result<()> {
        for child in node.elements() {
            if child.local == "field" && self.is_xfdf(child) {
                output.push(self.field(child)?);
            } else {
                self.issue(
                    child,
                    XfdfDiagnosticCode::UnsupportedStructure,
                    "表单区域只支持 field；该元素没有转换为表单数据",
                )?;
            }
        }
        Ok(())
    }

    fn field(&mut self, node: &XmlNode) -> Result<XfdfField> {
        let name = match node.attrs.get("name") {
            Some(name) => name.clone(),
            None => {
                self.issue(
                    node,
                    XfdfDiagnosticCode::MissingAttribute,
                    "field 缺少 name，使用空名称",
                )?;
                String::new()
            }
        };
        for key in node.attrs.keys().filter(|k| {
            k.as_str() != "name" && !k.starts_with("xmlns") && k.as_str() != "xml:space"
        }) {
            self.issue(
                node,
                XfdfDiagnosticCode::UnsupportedStructure,
                format!("field 的扩展属性 {key} 没有独立字段映射"),
            )?;
        }
        if !node.direct_text().trim().is_empty() {
            self.issue(
                node,
                XfdfDiagnosticCode::UnsupportedStructure,
                "field 的直接文本不属于 value，未作为字段值导入",
            )?;
        }
        let mut field = XfdfField {
            name,
            ..XfdfField::default()
        };
        for child in node.elements() {
            match child.local.as_str() {
                "value" if self.is_xfdf(child) => {
                    if child.elements().next().is_some() {
                        self.issue(
                            child,
                            XfdfDiagnosticCode::UnsupportedStructure,
                            "value 内的标记转为纯文本",
                        )?;
                    }
                    let value = child.text();
                    if field.value.is_none() {
                        field.value = Some(value);
                    } else {
                        field.additional_values.push(value);
                    }
                }
                "field" if self.is_xfdf(child) => field.children.push(self.field(child)?),
                _ => self.issue(
                    child,
                    XfdfDiagnosticCode::UnsupportedStructure,
                    "不支持的表单子元素没有转换为表单数据",
                )?,
            }
        }
        Ok(field)
    }

    fn validate_attrs(
        &mut self,
        node: &XmlNode,
        attrs: &mut HashMap<String, String>,
    ) -> Result<()> {
        for key in [
            "page",
            "rect",
            "opacity",
            "flags",
            "color",
            "width",
            "align",
            "symbol",
            "highlight",
            "fringe",
            "coords",
            "start",
            "end",
            "callout",
            "head",
            "dashes",
            "intent",
            "IT",
        ] {
            if !XfdfDocument::is_known_attr(&node.local, key) {
                continue;
            }
            let Some(value) = attrs.get(key) else {
                continue;
            };
            let valid = match key {
                "page" => value
                    .parse::<usize>()
                    .is_ok_and(|p| p <= self.options.max_page_index),
                "rect" => Rect::from_string(value).is_some(),
                "opacity" => value
                    .parse::<f32>()
                    .is_ok_and(|n| n.is_finite() && (0.0..=1.0).contains(&n)),
                "width" => value
                    .parse::<f32>()
                    .is_ok_and(|n| n.is_finite() && n >= 0.0),
                "flags" => parse_flags(value).is_some(),
                "color" => Color::from_hex(value).is_some(),
                "align" => value.parse::<i32>().is_ok_and(|n| (0..=2).contains(&n)),
                "symbol" => matches!(value.as_str(), "None" | "P"),
                "highlight" => highlight_mode(Some(value)).is_ok(),
                "fringe" => Rect::from_string(value)
                    .is_some_and(|r| [r.left, r.bottom, r.right, r.top].iter().all(|n| *n >= 0.0)),
                "coords" => valid_numbers(value, 8, true),
                "start" | "end" => valid_numbers(value, 2, false),
                "callout" => CalloutLine::from_string(value).is_some(),
                "head" if node.local == "freetext" => LineEndingStyle::from_name(value).is_some(),
                "dashes" => parse_dashes(value).is_some(),
                "intent" | "IT" => !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control()),
                _ => true,
            };
            if !valid {
                self.issue(
                    node,
                    XfdfDiagnosticCode::InvalidAttribute,
                    format!("无效属性 {key}={value:?}，使用默认值；可能影响 PDF 转换"),
                )?;
                attrs.remove(key);
            }
        }
        for (standard, legacy) in [("intent", "IT"), ("style", "border-style")] {
            if !XfdfDocument::is_known_attr(&node.local, standard) { continue; }
            if let (Some(a), Some(b)) = (attrs.get(standard), attrs.get(legacy)) {
                if a != b {
                    self.issue(node, XfdfDiagnosticCode::InvalidAttribute,
                        format!("{standard} 与 {legacy} 冲突，优先使用 {standard}"))?;
                }
            }
        }
        for key in ["style", "border-style"] {
            if XfdfDocument::is_known_attr(&node.local, key) {
                if let Some(value) = attrs.get(key) {
                    if BorderStyle::from_name(value).is_none() {
                        self.issue(node, XfdfDiagnosticCode::PreservedExtension,
                            format!("{key} 不是已支持的边框样式，作为扩展保留；不生成相应 PDF 边框"))?;
                    }
                }
            }
        }
        for key in ["page", "rect"] {
            if !attrs.contains_key(key) && !node.attrs.contains_key(key) {
                self.issue(
                    node,
                    XfdfDiagnosticCode::MissingAttribute,
                    format!("{} 缺少 {key}，转换为 PDF 时使用默认值", node.local),
                )?;
            }
        }
        Ok(())
    }

    fn annotation(
        &mut self,
        node: &XmlNode,
        output: &mut XfdfDocument,
        parent: Option<(&str, &HashMap<String, String>)>,
    ) -> Result<()> {
        self.annotation_count += 1;
        if self.annotation_count > self.options.max_annotations {
            return Err(invalid("注释数量超过限制"));
        }
        if !self.is_xfdf(node)
            || !XfdfDocument::is_annotation_tag(&node.local)
            || matches!(node.local.as_str(), "fileattachment" | "sound" | "widget")
        {
            self.issue(
                node,
                XfdfDiagnosticCode::UnsupportedAnnotation,
                format!(
                    "{} 未实现 PDF 转换，保留原始 XML 供 XFDF 往返使用",
                    node.local
                ),
            )?;
            output.unsupported_annotations.push(node.xml());
            return Ok(());
        }
        let mut attrs = node
            .attrs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<HashMap<_, _>>();
        for key in node.attrs.keys() {
            if let Some((prefix, _)) = key.split_once(':') {
                if !matches!(prefix, "xml" | "xmlns") {
                    let declaration = format!("xmlns:{prefix}");
                    if let Some(namespace) = node.namespaces.get(&declaration) {
                        attrs
                            .entry(declaration)
                            .or_insert_with(|| namespace.clone());
                    }
                }
            }
        }
        if let Some((parent_name, parent_attrs)) = parent {
            attrs
                .entry("parent".into())
                .or_insert_with(|| parent_name.to_string());
            attrs.entry("page".into()).or_insert_with(|| {
                parent_attrs
                    .get("page")
                    .cloned()
                    .unwrap_or_else(|| "0".into())
            });
        }
        let has_popup = node
            .elements()
            .any(|n| n.local == "popup" && self.is_xfdf(n));
        if has_popup {
            attrs
                .entry("name".into())
                .or_insert_with(|| format!("pdfxml-parent-{}", node.offset));
        }
        self.validate_attrs(node, &mut attrs)?;
        let mut contents = node.direct_text().trim().to_string();
        let mut has_contents = false;
        let mut popups = Vec::new();
        for (index, child) in node.elements().enumerate() {
            if !self.is_xfdf(child) {
                attrs.insert(format!("_xml:{index}"), child.xml());
                self.issue(
                    child,
                    XfdfDiagnosticCode::PreservedExtension,
                    "非 XFDF 命名空间的子元素保留为 XML 扩展",
                )?;
                continue;
            }
            match child.local.as_str() {
                "contents" => {
                    if has_contents {
                        return Err(invalid("注释包含重复的 contents 元素"));
                    }
                    has_contents = true;
                    if child.elements().next().is_some() {
                        self.issue(
                            child,
                            XfdfDiagnosticCode::UnsupportedStructure,
                            "contents 的嵌套标记被转换为纯文本；请使用 contents-richtext 保存样式",
                        )?;
                    }
                    contents = child.text();
                    attrs.insert("_contents-present".into(), "1".into());
                }
                "contents-richtext" => {
                    attrs.insert(
                        child.local.clone(),
                        if child.elements().next().is_some() {
                            child.inner_xml()
                        } else {
                            child.text()
                        },
                    );
                }
                "defaultstyle" | "defaultappearance" if node.local == "freetext" => {
                    attrs.insert(child.local.clone(), child.text());
                }
                "imagedata" if node.local == "stamp" => {
                    attrs.insert(child.local.clone(), child.text());
                }
                "inklist" if node.local == "ink" => {
                    let mut gestures = Vec::new();
                    for gesture in child.elements() {
                        if gesture.local == "gesture" && self.is_xfdf(gesture) {
                            gestures.push(gesture.text().trim().to_string());
                        } else {
                            self.issue(
                                gesture,
                                XfdfDiagnosticCode::UnsupportedStructure,
                                "inklist 的非 gesture 子元素没有转换为墨迹路径",
                            )?;
                        }
                    }
                    attrs.insert("_inklist".into(), gestures.join("\u{1b}"));
                }
                "popup" => popups.push(child),
                "Dest" | "OnActivation" if node.local == "link" => {}
                _ => {
                    attrs.insert(format!("_xml:{index}"), child.xml());
                    self.issue(
                        child,
                        XfdfDiagnosticCode::PreservedExtension,
                        "未实现的注释子元素保留为 XML 扩展；不保证 PDF 映射",
                    )?;
                }
            }
        }
        let built = XfdfDocument::build_annotation_with_children(&node.local, &attrs, &contents)
            .and_then(|mut annotation| {
                if let Annotation::Link(link) = &mut annotation {
                    link.target = link_target(node, self.options)?;
                }
                if let Annotation::FreeText(text) = &mut annotation {
                    if text.callout.is_some() {
                        if !attrs.contains_key("width") { text.width = 1.0; }
                        match text.base.intent.as_deref() {
                            None => text.base.intent = Some("FreeTextCallout".into()),
                            Some("FreeTextCallout") => {},
                            Some(_) => self.issue(node, XfdfDiagnosticCode::InvalidAttribute,
                                "FreeText callout 与 intent 不一致；保留原意图并写入引出线")?,
                        }
                    }
                }
                Ok(annotation)
            });
        match built {
            Ok(annotation) => {
                if matches!(&annotation, Annotation::Link(link) if link.target.is_none()) {
                    self.issue(
                        node,
                        XfdfDiagnosticCode::MissingAttribute,
                        "Link 没有目标，仅保留不可跳转的链接区域",
                    )?;
                }
                output.annotations.push(annotation);
                for popup in popups {
                    self.annotation(
                        popup,
                        output,
                        Some((attrs.get("name").expect("popup parent name"), &attrs)),
                    )?;
                }
            }
            Err(error) => {
                self.issue(
                    node,
                    XfdfDiagnosticCode::UnsupportedAnnotation,
                    format!("{}；该注释保留为原始 XML，不写入 PDF", error),
                )?;
                output.unsupported_annotations.push(node.xml());
            }
        }
        Ok(())
    }
}

pub(super) fn parse(xml: &str, options: &XfdfParseOptions) -> Result<XfdfParseReport> {
    let root = read_xml(xml, options)?;
    if root.local != "xfdf"
        || root
            .namespace
            .as_deref()
            .is_some_and(|ns| ns != XFDF_NAMESPACE)
    {
        return Err(invalid("根元素必须是 XFDF 命名空间中的 xfdf"));
    }
    let mut context = Context {
        options,
        diagnostics: Vec::new(),
        allow_unbound: root.namespace.is_none(),
        annotation_count: 0,
    };
    if root.namespace.is_none() {
        context.issue(
            &root,
            XfdfDiagnosticCode::MissingNamespace,
            "缺少 XFDF 命名空间，按历史兼容格式解析",
        )?;
    }
    if !root.direct_text().trim().is_empty() {
        context.issue(
            &root,
            XfdfDiagnosticCode::UnsupportedStructure,
            "xfdf 根元素的直接文本没有文档字段映射",
        )?;
    }
    let mut doc = XfdfDocument {
        xmlns: root.namespace.clone(),
        ..XfdfDocument::default()
    };
    doc.metadata = root
        .attrs
        .iter()
        .filter(|(k, _)| k.as_str() != "xmlns")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for child in root.elements() {
        if !context.is_xfdf(child) {
            context.issue(
                child,
                XfdfDiagnosticCode::PreservedExtension,
                "文档级非 XFDF 扩展保留为原始 XML",
            )?;
            doc.extensions.push(child.xml());
            continue;
        }
        match child.local.as_str() {
            "fields" => context.fields(child, &mut doc.fields)?,
            "annots" => {
                for annotation in child.elements() {
                    context.annotation(annotation, &mut doc, None)?;
                }
            }
            "f" => {
                if let Some(href) = child.attrs.get("href") {
                    if doc.file_reference.replace(href.clone()).is_some() {
                        return Err(invalid("重复的 f 文档引用"));
                    }
                    for key in child
                        .attrs
                        .keys()
                        .filter(|k| k.as_str() != "href" && !k.starts_with("xmlns"))
                    {
                        context.issue(
                            child,
                            XfdfDiagnosticCode::UnsupportedStructure,
                            format!("f 的属性 {key} 没有独立字段映射"),
                        )?;
                    }
                }
                if child.elements().next().is_some() {
                    context.issue(
                        child,
                        XfdfDiagnosticCode::LegacyStructure,
                        "兼容历史 f/fields 包装：不再将 f 当作表单字段；输出时规范化为根级 fields",
                    )?;
                    for fields in child.elements() {
                        if fields.local == "fields" && context.is_xfdf(fields) {
                            context.fields(fields, &mut doc.fields)?;
                        } else {
                            context.issue(
                                fields,
                                XfdfDiagnosticCode::UnsupportedStructure,
                                "历史 f 包装中只支持 fields",
                            )?;
                        }
                    }
                } else if !child.attrs.contains_key("href") {
                    context.issue(
                        child,
                        XfdfDiagnosticCode::MissingAttribute,
                        "f 缺少 href，不构建文档引用或虚假表单字段",
                    )?;
                }
            }
            "ids" => {
                if doc.ids.is_some() {
                    return Err(invalid("重复的 ids 元素"));
                }
                for key in child.attrs.keys().filter(|k| {
                    !matches!(k.as_str(), "original" | "modified") && !k.starts_with("xmlns")
                }) {
                    context.issue(
                        child,
                        XfdfDiagnosticCode::UnsupportedStructure,
                        format!("ids 的属性 {key} 没有字段映射"),
                    )?;
                }
                doc.ids = Some(XfdfIds {
                    original: child.attrs.get("original").cloned(),
                    modified: child.attrs.get("modified").cloned(),
                });
            }
            _ => {
                context.issue(
                    child,
                    XfdfDiagnosticCode::PreservedExtension,
                    "文档级未知元素保留为原始 XML；不保证 PDF 映射",
                )?;
                doc.extensions.push(child.xml());
            }
        }
    }
    Ok(XfdfParseReport {
        document: doc,
        diagnostics: context.diagnostics,
    })
}

pub(super) fn parse_flags(value: &str) -> Option<u32> {
    let value = value.trim();
    if value.is_empty() {
        return Some(0);
    }
    // Legacy releases wrote unprefixed hexadecimal numbers. Keep that interpretation.
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16).ok();
    }
    if value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return u32::from_str_radix(value, 16).ok();
    }
    let mut flags = 0;
    for token in value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        flags |= match token.to_ascii_lowercase().as_str() {
            "invisible" => 1,
            "hidden" => 2,
            "print" => 4,
            "nozoom" => 8,
            "norotate" => 16,
            "noview" => 32,
            "readonly" | "read-only" => 64,
            "locked" => 128,
            "togglenoview" => 256,
            "lockedcontents" => 512,
            _ => return None,
        };
    }
    Some(flags)
}

pub(super) fn flags_string(flags: u32) -> String {
    if flags & !1023 != 0 {
        return format!("0x{flags:X}");
    }
    [
        "invisible",
        "hidden",
        "print",
        "nozoom",
        "norotate",
        "noview",
        "readonly",
        "locked",
        "togglenoview",
        "lockedcontents",
    ]
    .iter()
    .enumerate()
    .filter(|(bit, _)| flags & (1 << bit) != 0)
    .map(|(_, name)| *name)
    .collect::<Vec<_>>()
    .join(",")
}

pub(super) fn parse_dashes(value: &str) -> Option<Vec<f32>> {
    let values = if value.trim().is_empty() {
        Vec::new()
    } else {
        value.split(',').map(|v| v.trim().parse::<f32>())
            .collect::<std::result::Result<Vec<_>, _>>().ok()?
    };
    (values.iter().all(|n| n.is_finite() && *n >= 0.0)
        && (values.is_empty() || values.iter().any(|n| *n > 0.0))).then_some(values)
}

fn valid_numbers(value: &str, group: usize, multiple: bool) -> bool {
    let values = value.split(',').collect::<Vec<_>>();
    !values.is_empty()
        && if multiple {
            values.len() % group == 0
        } else {
            values.len() == group
        }
        && values
            .iter()
            .all(|v| v.trim().parse::<f32>().is_ok_and(f32::is_finite))
}

pub(super) fn highlight_mode(value: Option<&str>) -> Result<LinkHighlightMode> {
    Ok(match value.unwrap_or("Invert") {
        "None" | "N" => LinkHighlightMode::None,
        "Invert" | "I" => LinkHighlightMode::Invert,
        "Outline" | "O" => LinkHighlightMode::Outline,
        "Push" | "P" => LinkHighlightMode::Push,
        other => return Err(invalid(format!("无效 Link highlight: {other}"))),
    })
}

fn only_child<'a>(node: &'a XmlNode, namespace: Option<&str>) -> Result<&'a XmlNode> {
    let children = node.elements().collect::<Vec<_>>();
    if children.len() != 1 || children[0].namespace.as_deref() != namespace {
        return Err(invalid(format!(
            "{} 必须包含一个同命名空间的目标元素",
            node.local
        )));
    }
    Ok(children[0])
}

fn destination(node: &XmlNode, options: &XfdfParseOptions) -> Result<LinkTarget> {
    let child = only_child(node, node.namespace.as_deref())?;
    if child.local == "Named" {
        let name = child
            .attrs
            .get("Name")
            .ok_or_else(|| invalid("Named 缺少 Name"))?
            .clone();
        return Ok(LinkTarget::Named { name });
    }
    let mode = LinkDestinationMode::from_name(&child.local)
        .ok_or_else(|| invalid("不支持的 Link 目的地显示方式"))?;
    let page = child
        .attrs
        .get("Page")
        .ok_or_else(|| invalid("Link 目的地缺少 Page"))?
        .parse::<usize>()
        .map_err(|_| invalid("无效 Link Page"))?;
    if page > options.max_page_index {
        return Err(invalid("Link 目标页码超过限制"));
    }
    let mut parameters = Vec::new();
    for name in mode.parameter_names() {
        let parameter = match child.attrs.get(*name) {
            Some(value) if value != "null" => {
                let n = value
                    .parse::<f64>()
                    .map_err(|_| invalid(format!("无效目的地参数 {name}")))?;
                if !n.is_finite() || !(n as f32).is_finite() {
                    return Err(invalid("目的地参数必须为可表示的有限数"));
                }
                Some(n)
            }
            _ => None,
        };
        if mode == LinkDestinationMode::FitR && parameter.is_none() {
            return Err(invalid("FitR 必须具有四个矩形参数"));
        }
        parameters.push(parameter);
    }
    Ok(LinkTarget::Page {
        page,
        view: LinkDestinationView { mode, parameters },
    })
}

fn link_target(node: &XmlNode, options: &XfdfParseOptions) -> Result<Option<LinkTarget>> {
    let candidates = node
        .elements()
        .filter(|n| {
            n.namespace == node.namespace && matches!(n.local.as_str(), "Dest" | "OnActivation")
        })
        .collect::<Vec<_>>();
    let simple_count =
        usize::from(node.attrs.contains_key("uri")) + usize::from(node.attrs.contains_key("dest"));
    if candidates.len() + simple_count > 1 {
        return Err(invalid("Link 同时声明了多个目标"));
    }
    if let Some(uri) = node.attrs.get("uri") {
        return Ok(Some(LinkTarget::Uri {
            uri: uri.clone(),
            is_map: false,
        }));
    }
    if let Some(name) = node.attrs.get("dest") {
        return Ok(Some(LinkTarget::Named { name: name.clone() }));
    }
    let Some(target) = candidates.first() else {
        return Ok(None);
    };
    if target.local == "Dest" {
        return destination(target, options).map(Some);
    }
    let action = only_child(target, target.namespace.as_deref())?;
    if action.local != "Action" {
        return Err(invalid("OnActivation 缺少 Action"));
    }
    let child = only_child(action, action.namespace.as_deref())?;
    match child.local.as_str() {
        "URI" => {
            let uri = child
                .attrs
                .get("Name")
                .ok_or_else(|| invalid("URI 缺少 Name"))?
                .clone();
            let is_map = match child
                .attrs
                .get("IsMap")
                .map(String::as_str)
                .unwrap_or("false")
            {
                "true" | "yes" | "1" => true,
                "false" | "no" | "0" => false,
                _ => return Err(invalid("无效 URI IsMap")),
            };
            Ok(Some(LinkTarget::Uri { uri, is_map }))
        }
        "GoTo" => {
            let dest = only_child(child, child.namespace.as_deref())?;
            if dest.local != "Dest" {
                return Err(invalid("GoTo 缺少 Dest"));
            }
            destination(dest, options).map(Some)
        }
        other => Err(invalid(format!(
            "不支持 Link 动作 {other}；不会执行脚本或 Launch/远程动作"
        ))),
    }
}
