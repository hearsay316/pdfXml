//! 错误类型定义

use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum PdfXmlError {
    #[error("XML 解析错误: {0}")]
    XmlParse(#[from] quick_xml::Error),

    #[error("PDF 处理错误: {0}")]
    PdfProcessing(String),

    #[error("无效的 XFDF 格式: {0}")]
    InvalidXfdfFormat(String),

    #[error("不支持的注释类型: {0}")]
    UnsupportedAnnotationType(String),

    #[error("日期解析错误: {0}")]
    DateParse(String),

    #[error("颜色解析错误: {0}")]
    ColorParse(String),

    #[error("坐标解析错误: {0}")]
    CoordinateParse(String),
    
    #[error("无效的页面对象")]
    InvalidPageObject,
    
    #[error("更新页面失败")]
    UpdatePageFailed,
}

// 从 lopdf 错误转换 - 使用 Display trait
impl From<lopdf::Error> for PdfXmlError {
    fn from(err: lopdf::Error) -> Self {
        PdfXmlError::PdfProcessing(format!("{}", err))
    }
}

pub type Result<T> = std::result::Result<T, PdfXmlError>;
