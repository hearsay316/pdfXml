//! 注释数据结构定义
//!
//! 这个文件主要回答一个问题：
//! “程序内部到底用什么结构，来表示一条 PDF 注释？”
//!
//! 可以把它理解成项目里的“数据模型层”：
//! - `xfdf.rs` 负责把 XML 解析出来
//! - `annotation.rs` 负责定义解析后的数据长什么样
//! - `pdf.rs` 再把这些数据写进 PDF
//!
//! 所以这里大多数内容都只是“描述数据”，
//! 而不是“处理业务逻辑”。
//! 读这个文件时，重点看两层：
//! 1. `AnnotationBase`：几乎所有注释都会共享的公共字段
//! 2. 各种具体注释结构体：表示每一种注释独有的数据

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 注释颜色（RGB）。
///
/// PDF 最终使用的是 0.0 ~ 1.0 的浮点数颜色值，
/// 所以这里会把常见的十六进制颜色转换成内部可用的 RGB。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color {
    /// 从十六进制颜色字符串解析（如 "#FF0000"）。
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

    #[allow(dead_code)]
    /// 转换为 PDF 颜色数组格式。
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

/// 矩形坐标 (x1, y1, x2, y2)。
///
/// 很多注释都需要一个外接矩形，告诉 PDF：
/// “这个注释大概占据页面上的哪一块区域”。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

impl Rect {
    /// 从 XFDF rect 字符串解析（如 "100,200,300,400"）。
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

    #[allow(dead_code)]
    /// 转换为 PDF 矩形数组格式。
    pub fn to_pdf_array(&self) -> Vec<lopdf::Object> {
        vec![
            lopdf::Object::Real(self.left as f32),
            lopdf::Object::Real(self.bottom as f32),
            lopdf::Object::Real(self.right as f32),
            lopdf::Object::Real(self.top as f32),
        ]
    }
}

/// 注释基础属性。
///
/// 可以把它理解成“所有注释都共享的公共部分”。
/// 比如：页码、矩形、标题、内容、颜色、日期等。
///
/// 具体到某种注释时，再在这个基础上补自己的专属字段。
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

/// 文本注释（便签）。
///
/// 这是最常见的“小便签”类型，通常会有一个图标，
/// 点开后能看到内容。
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

/// 高亮注释。
///
/// 核心数据不是一个矩形，而是一组 QuadPoints，
/// 用来表示被高亮文字所在的四边形区域。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 坐标点数组 (QuadPoints)，格式: "x1,y1,x2,y2,x3,y3,x4,y4,..."
    #[serde(rename = "coords", skip_serializing_if = "Option::is_none")]
    pub coords: Option<String>,
}

/// 下划线注释。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnderlineAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 坐标点数组 (QuadPoints)
    #[serde(rename = "coords", skip_serializing_if = "Option::is_none")]
    pub coords: Option<String>,
}

/// 删除线注释。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrikeOutAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 坐标点数组 (QuadPoints)
    #[serde(rename = "coords", skip_serializing_if = "Option::is_none")]
    pub coords: Option<String>,
}

/// 波浪线注释。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquigglyAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 坐标点数组 (QuadPoints)
    #[serde(rename = "coords", skip_serializing_if = "Option::is_none")]
    pub coords: Option<String>,
}

/// 自由文本注释。
///
/// 和 TextAnnotation 不同，FreeText 通常是“直接显示在页面上的文字框”。
/// 所以它除了内容，还会关心文字外观、对齐方式、字体信息等。
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

/// 方形注释。
///
/// 本质上是一个矩形框标记，通常最关心边框宽度和颜色。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquareAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 边框宽度
    #[serde(default)]
    pub width: f32,
}

/// 圆形注释。
///
/// 它的外接区域仍然由 `rect` 决定，但画出来的外观是圆/椭圆。
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

/// 线条注释。
///
/// 它最重要的数据是：
/// - 起点
/// - 终点
/// - 两端样式（比如箭头、圆点、方块）
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

/// 多边形/折线注释。
///
/// 这两种类型公用一个结构体：
/// - `polygon`：闭合图形
/// - `polyline`：不闭合折线
///
/// 真正区分它们的是 `is_closed`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolygonAnnotation {
    #[serde(flatten)]
    pub base: AnnotationBase,

    /// 顶点坐标列表（空格分隔的 x,y 对）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertices: Option<String>,

    /// true 表示 polygon（闭合），false 表示 polyline（不闭合）
    #[serde(default = "default_polygon_closed")]
    pub is_closed: bool,
}

fn default_polygon_closed() -> bool { true }

/// 墨水注释（手绘）。
///
/// 这类注释通常不是规则图形，而是用户手绘出来的一串轨迹。
/// 所以这里保存的是多段路径数据。
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

/// 图章注释。
///
/// 可以是预设图章，也可能直接带一段图片数据。
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

/// 弹出窗口注释。
///
/// 它一般不是独立内容本体，而是挂在别的注释旁边的“弹出说明框”。
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

/// 枚举所有支持的注释类型。
///
/// 这是对外最常用的统一入口：
/// 不管实际是文本、高亮、线条还是图章，最后都能先装进 `Annotation` 里。
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
    /// 返回注释的类型名称。
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

    /// 返回基础属性引用。
    ///
    /// 当外层逻辑只关心“公共字段”时，不需要先判断具体是哪种注释，
    /// 直接通过这个方法拿 `AnnotationBase` 就行。
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

    /// 返回页码。
    pub fn page(&self) -> usize {
        self.base().page
    }

    #[allow(dead_code)]
    /// 返回边界矩形。
    pub fn rect(&self) -> Option<&Rect> {
        self.base().rect.as_ref()
    }
}
