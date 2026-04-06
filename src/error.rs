//! 这个文件专门放项目里统一使用的错误类型。
//!
//! 这样做的好处是：
//! - 不管错误来自 XML、PDF、颜色还是日期格式
//! - 最后都能统一变成 `PdfXmlError`
//! - 外部调用者不用同时处理很多种完全不同的错误类型

use thiserror::Error;

/// 项目统一使用的错误枚举。
///
/// 可以把它理解成“失败原因的大分类列表”。
/// 排查问题时，先看属于哪一类，再看里面带的详细信息。
#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum PdfXmlError {
    /// XFDF/XML 本身解析失败。
    #[error("XML 解析错误: {0}")]
    XmlParse(#[from] quick_xml::Error),

    /// PDF 读取、对象构建、保存等过程中的错误。
    #[error("PDF 处理错误: {0}")]
    PdfProcessing(String),

    /// XFDF 结构不符合当前程序预期。
    #[error("无效的 XFDF 格式: {0}")]
    InvalidXfdfFormat(String),

    /// 遇到了当前还没有实现的注释类型。
    #[error("不支持的注释类型: {0}")]
    UnsupportedAnnotationType(String),

    /// 日期字符串格式不正确，无法解析。
    #[error("日期解析错误: {0}")]
    DateParse(String),

    /// 颜色字符串格式不正确，无法解析。
    #[error("颜色解析错误: {0}")]
    ColorParse(String),

    /// 坐标字符串格式不正确，无法解析。
    #[error("坐标解析错误: {0}")]
    CoordinateParse(String),

    /// 页面对象不符合预期，无法继续处理。
    #[error("无效的页面对象")]
    InvalidPageObject,

    /// 往页面写回修改时失败。
    #[error("更新页面失败")]
    UpdatePageFailed,
}

// 把 `lopdf` 的底层错误统一转换成我们自己的错误类型。
// 这样项目对外只暴露 `PdfXmlError`，
// 上层就不需要同时记住一堆第三方库的错误类型。
impl From<lopdf::Error> for PdfXmlError {
    fn from(err: lopdf::Error) -> Self {
        PdfXmlError::PdfProcessing(format!("{}", err))
    }
}

/// 项目统一使用的结果类型别名。
///
/// 写成 `Result<T>`，实际等于：
/// `std::result::Result<T, PdfXmlError>`
pub type Result<T> = std::result::Result<T, PdfXmlError>;
