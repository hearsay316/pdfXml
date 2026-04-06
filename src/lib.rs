/// 注释的数据结构。
///
/// 这里定义了“文本注释、高亮、线条、圆形”等 Rust 类型。
/// 如果你想在代码里自己查看或修改注释内容，通常会用到这里导出的类型。
pub mod annotation;
/// 统一的错误类型。
///
/// 库里读文件、解析 XML、生成 PDF 失败时，都会尽量收口到这里。
pub mod error;
/// PDF 导出模块。
///
/// 这里面放的是把注释真正写进 PDF 的核心逻辑。
pub mod pdf;
/// XFDF/XML 解析模块。
///
/// 它负责把 XFDF 字符串解析成 Rust 结构体。
pub mod xfdf;

pub use annotation::*;
pub use error::{PdfXmlError, Result};
pub use pdf::PdfAnnotationExporter;
pub use xfdf::{XfdfDocument, XfdfField};

use std::fs;
use std::path::Path;

/// 从磁盘读取一个 XFDF 文件，并解析成 `XfdfDocument`。
///
/// 可以把它理解成两步合成一步：
/// 1. 先把文件内容读出来
/// 2. 再把 XML 解析成程序能处理的数据
///
/// 适合“我已经有一个 `.xfdf` 文件，想直接导入”的场景。
pub fn load_xfdf(path: impl AsRef<Path>) -> Result<XfdfDocument> {
    let content = fs::read_to_string(path)
        .map_err(|e| PdfXmlError::PdfProcessing(format!("读取 XFDF 文件失败: {}", e)))?;
    XfdfDocument::parse(&content)
}

/// 把解析后的注释导出成 PDF。
///
/// - `xfdf_doc`：已经解析好的注释数据
/// - `target_pdf`：如果传入现有 PDF，就把注释合并进去
/// - `output_path`：输出文件路径
///
/// 规则很简单：
/// - 有 `target_pdf` → 合并到已有 PDF
/// - 没有 `target_pdf` → 新建一个只包含注释的 PDF
///
/// 这是给 SDK 使用者准备的顶层便捷函数。
/// 如果你想手动控制导出器，也可以直接使用 `PdfAnnotationExporter`。
pub fn export_annotations(
    xfdf_doc: &XfdfDocument,
    target_pdf: Option<impl AsRef<Path>>,
    output_path: impl AsRef<Path>,
) -> Result<()> {
    let mut exporter = PdfAnnotationExporter::new();
    match target_pdf {
        Some(target_pdf) => exporter.export_to_existing_pdf(xfdf_doc, target_pdf.as_ref(), output_path.as_ref()),
        None => exporter.export_to_new_pdf(xfdf_doc, output_path.as_ref()),
    }
}
