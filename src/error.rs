//! 错误类型定义
//!
//! 这个文件的作用是把项目里可能出现的失败情况，
//! 统一收口成一个错误枚举 `PdfXmlError`。
//!
//! 这样无论错误来自：
//! - XML 解析
//! - PDF 处理
//! - 颜色/坐标/日期解析
//! 最后都能用一致的方式往外返回。

use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum PdfXmlError {
    /// XFDF/XML 文本本身解析失败。
    #[error("XML 解析错误: {0}")]
    XmlParse(#[from] quick_xml::Error),

    /// PDF 读写、对象构建、保存等过程中的错误。
    #[error("PDF 处理错误: {0}")]
    PdfProcessing(String),

    /// XFDF 结构不符合当前程序预期。
    #[error("无效的 XFDF 格式: {0}")]
    InvalidXfdfFormat(String),

    /// 遇到了当前还没实现的注释类型。
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

// 把 lopdf 的底层错误统一转换成我们自己的错误类型。
// 这样项目对外只暴露 PdfXmlError，不需要让上层同时处理很多种错误枚举。
impl From<lopdf::Error> for PdfXmlError {
    fn from(err: lopdf::Error) -> Self {
        PdfXmlError::PdfProcessing(format!("{}", err))
    }
}

/// 项目统一使用的 Result 别名。
///
/// 写成 `Result<T>` 就等于：
/// `std::result::Result<T, PdfXmlError>`
pub type Result<T> = std::result::Result<T, PdfXmlError>;
