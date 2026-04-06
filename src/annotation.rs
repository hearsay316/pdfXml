//! 注释数据结构定义
//!
//! 定义各种 PDF 注释类型的结构体，用于表示 XFDF 中的注释

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 注释颜色（RGB）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color {
    /// 从十六进制颜色字符串解析（如 "#FF0000"）
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }

        let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;

        Some(Color { r, g, b })
    }

    /// 转换为 PDF 颜色数组格式
    pub fn to_pdf_array(&self) -> Vec<lopdf::Object> {
        vec![
            lopdf::Object::Real(self.r),
            lopdf::Object::Real(self.g),
            lopdf::Object::Real(self.b),
        ]
    }
}

impl Default for Color {
    fn default() -> Self {
        Color { r: 1.0, g: 1.0, b: 0.0 } // 默认黄色
    }
}

/// 矩形坐标 (x1, y1, x2, y2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

impl Rect {
    /// 从 XFDF rect 字符串解析（如 "100,200,300,400"）
    pub fn from_string(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 4 {
            return None;
        }

        Some(Rect {
            left: parts[0].trim().parse().ok()?,
            bottom: parts[1].trim().parse().ok()?,
            right: parts[2].trim().parse().ok()?,
            top: parts[3].trim().parse().ok()?,
        })
    }

    /// 转换为 PDF 矩形数组格式
    pub fn to_pdf_array(&self) -> Vec<lopdf::Object> {
        vec![
            lopdf::Object::Real(self.left as f32),
            lopdf::Object::Real(self.bottom as f32),
            lopdf::Object::Real(self.right as f32),
            lopdf::Object::Real(self.top as f32),
        ]
    }
}

/// 注释基础属性
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationBase {
    /// 唯一标识符
    #[serde(rename = "name", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// 页码（从0开始）
    #[serde(default)]
    pub page: usize,

    /// 边界矩形
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,

    /// 标题/作者
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// 主题/标题
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,

    /// 内容
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,

    /// 创建日期（XFDF 格式：D:YYYYMMDDHHmmSS）
    #[serde(rename = "creationdate", skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<String>,

    /// 修改日期
    #[serde(rename = "date", skip_serializing_if = "Option::is_none")]
    pub modification_date: Option<String>,

    /// 颜色（十六进制字符串）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    /// 不透明度（0.0 - 1.0）
    #[serde(default = "default_opacity")]
    pub opacity: f32,

    /// 标志位
    #[serde(default)]
    pub flags: u32,

    /// 自定义属性
    #[serde(flatten)]
    pub extra: HashMap<String, String>,
}

fn default_opacity() -> f32 { 1.0 }

/// 文本注释（便签）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 打开状态
    #[serde(default)]
    pub open: bool,

    /// 图标类型（Comment, Help, Insert, Key, NewParagraph, Note, Paragraph）
    #[serde(rename = "icon", default = "default_icon")]
    pub icon_type: String,
}

fn default_icon() -> String { "Note".to_string() }

/// 高亮注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 坐标点数组 (QuadPoints)，格式: "x1,y1,x2,y2,x3,y3,x4,y4,..."
    #[serde(rename = "coords", skip_serializing_if = "Option::is_none")]
    pub coords: Option<String>,
}

/// 下划线注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnderlineAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 坐标点数组 (QuadPoints)
    #[serde(rename = "coords", skip_serializing_if = "Option::is_none")]
    pub coords: Option<String>,
}

/// 删除线注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrikeOutAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 坐标点数组 (QuadPoints)
    #[serde(rename = "coords", skip_serializing_if = "Option::is_none")]
    pub coords: Option<String>,
}

/// 波浪线注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquigglyAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 坐标点数组 (QuadPoints)
    #[serde(rename = "coords", skip_serializing_if = "Option::is_none")]
    pub coords: Option<String>,
}

/// 自由文本注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeTextAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 默认外观字符串 (CSS-style)
    #[serde(rename = "defaultstyle", skip_serializing_if = "Option::is_none")]
    pub default_style: Option<String>,

    /// PDF DefaultAppearance 字符串 (如 "0.894 0.259 0.204 RG 0.894 0.259 0.204 rg /Helvetica 12 Tf")
    #[serde(rename = "defaultappearance", skip_serializing_if = "Option::is_none")]
    pub default_appearance: Option<String>,

    /// 文本颜色 (TextColor 属性，优先于 color)
    #[serde(rename = "TextColor", skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,

    /// 对齐方式（0=左对齐, 1=居中, 2=右对齐）
    #[serde(default)]
    pub align: i32,
}

/// 方形注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquareAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 边框宽度
    #[serde(default)]
    pub width: f32,
}

/// 圆形注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircleAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 边框宽度
    #[serde(default)]
    pub width: f32,

    /// 内部填充颜色
    #[serde(rename = "interiorcolor", skip_serializing_if = "Option::is_none")]
    pub interior_color: Option<String>,
}

/// 线条注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 起点坐标
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,  // 格式: "x,y"

    /// 终点坐标
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,    // 格式: "x,y"

    /// 线条端点样式
    #[serde(rename = "head", default)]
    pub head_style: String,

    #[serde(rename = "tail", default)]
    pub tail_style: String,

    /// 线宽
    #[serde(default = "default_line_width")]
    pub width: f32,
}

fn default_line_width() -> f32 { 1.0 }

/// 多边形注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolygonAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 顶点坐标列表（空格分隔的 x,y 对）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertices: Option<String>,
}

/// 墨水注释（手绘）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InkAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 墨迹路径列表 - 从 <inklist><gesture> 解析
    /// 每个 gesture 是一组 "x;y;x;y;..." 坐标字符串
    #[serde(rename = "inklist", default)]
    pub ink_list: Vec<String>,

    /// 线宽
    #[serde(default = "default_line_width")]
    pub width: f32,
}

/// 图章注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StampAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 图章名称（Approved, Draft, Final, etc.）
    #[serde(default)]
    pub icon: String,

    /// XFDF 中的图章图片数据（data:image/...;base64,...）
    #[serde(rename = "imagedata", skip_serializing_if = "Option::is_none")]
    pub image_data: Option<String>,
}

/// 弹出窗口注释
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopupAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 打开状态
    #[serde(default)]
    pub open: bool,

    /// 关联的父注释名称
    #[serde(rename = "parent", skip_serializing_if = "Option::is_none")]
    pub parent_name: Option<String>,
}

/// 枚举所有支持的注释类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Annotation {
    Text(TextAnnotation),
    Highlight(HighlightAnnotation),
    Underline(UnderlineAnnotation),
    StrikeOut(StrikeOutAnnotation),
    Squiggly(SquigglyAnnotation),
    FreeText(FreeTextAnnotation),
    Square(SquareAnnotation),
    Circle(CircleAnnotation),
    Line(LineAnnotation),
    Polygon(PolygonAnnotation),
    Ink(InkAnnotation),
    Stamp(StampAnnotation),
    Popup(PopupAnnotation),
}

impl Annotation {
    /// 返回注释的类型名称
    pub fn annotation_type(&self) -> &'static str {
        match self {
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
        }
    }

    /// 返回基础属性引用
    pub fn base(&self) -> &AnnotationBase {
        match self {
            Annotation::Text(a) => &a.base,
            Annotation::Highlight(a) => &a.base,
            Annotation::Underline(a) => &a.base,
            Annotation::StrikeOut(a) => &a.base,
            Annotation::Squiggly(a) => &a.base,
            Annotation::FreeText(a) => &a.base,
            Annotation::Square(a) => &a.base,
            Annotation::Circle(a) => &a.base,
            Annotation::Line(a) => &a.base,
            Annotation::Polygon(a) => &a.base,
            Annotation::Ink(a) => &a.base,
            Annotation::Stamp(a) => &a.base,
            Annotation::Popup(a) => &a.base,
        }
    }

    /// 返回页码
    pub fn page(&self) -> usize {
        self.base().page
    }

    /// 返回边界矩形
    pub fn rect(&self) -> Option<&Rect> {
        self.base().rect.as_ref()
    }
}
