//! 这个文件可以理解成“库的总入口”。
//!
//! 别的 Rust 项目接入 `pdfxml` 时，最先接触到的通常就是这里。
//! 因为这里决定了两件事：
//! - 哪些模块对外公开
//! - 外部最常用的类型和函数有哪些
//!
//! 如果你只是想会用这个库，不想一下子钻进所有实现细节，
//! 那直接从这个文件看就够了。
//!
//! 你可以把这里当成“对外使用说明 + API 门面”：
//! - 想读 XFDF：看 [`load_xfdf`]
//! - 想从 PDF 读注释：看 [`load_annotations_from_pdf`]
//! - 想把注释导出成 PDF：看 [`export_annotations`]
//! - 想自己细调导出行为：看 [`PdfAnnotationExporter`]
//!
//! 建议阅读顺序：
//! 1. 先看下面的 `pub mod ...`，知道项目分成哪几块
//! 2. 再看 `pub use ...`，知道外部能直接拿到哪些 API
//! 3. 最后看几个顶层函数，理解最常见调用流程

/// 注释数据结构模块。
///
/// 这里定义了文本注释、高亮、线条、图章等 Rust 类型。
/// 如果你想在代码里直接查看或修改注释内容，通常会用到这里导出的类型。
pub mod annotation;

/// 统一错误类型模块。
///
/// 读取文件、解析 XML、处理 PDF 失败时，
/// 最后都会尽量统一收口到这里定义的错误类型。
pub mod error;

/// PDF 读写模块。
///
/// 这里放的是把注释真正写进 PDF、或者从 PDF 里读回注释的核心逻辑。
pub mod pdf;

/// XFDF/XML 解析模块。
///
/// 它负责把 XFDF 字符串解析成 Rust 结构，
/// 也负责把 Rust 结构重新写回 XFDF 字符串。
pub mod xfdf;

pub use annotation::*;
pub use error::{PdfXmlError, Result};
pub use pdf::PdfAnnotationExporter;
pub use xfdf::{XfdfDocument, XfdfField};

use std::fs;
use std::path::Path;

/// 从磁盘读取一个 XFDF 文件，并解析成 [`XfdfDocument`]。
///
/// 可以把它理解成“两步合成一步”：
/// 1. 先把文件内容读出来
/// 2. 再把读到的 XML 解析成程序能直接使用的数据结构
///
/// # 示例
///
/// ```no_run
/// use pdfxml::load_xfdf;
///
/// let doc = load_xfdf("examples/sample.xfdf")?;
/// assert!(!doc.annotations.is_empty());
/// # Ok::<(), pdfxml::PdfXmlError>(())
/// ```
pub fn load_xfdf(path: impl AsRef<Path>) -> Result<XfdfDocument> {
    let content = fs::read_to_string(path)
        .map_err(|e| PdfXmlError::PdfProcessing(format!("读取 XFDF 文件失败: {}", e)))?;
    XfdfDocument::parse(&content)
}

/// 从 PDF 中读取注释，并转换成项目统一使用的 [`XfdfDocument`]。
///
/// 这样做的好处是：
/// - 后面可以继续复用同一套数据结构
/// - 测试和导出 XFDF 的逻辑也都能继续复用
///
/// # 示例
///
/// ```no_run
/// use pdfxml::load_annotations_from_pdf;
///
/// let doc = load_annotations_from_pdf("annotated.pdf")?;
/// println!("{}", doc.annotations.len());
/// # Ok::<(), pdfxml::PdfXmlError>(())
/// ```
pub fn load_annotations_from_pdf(path: impl AsRef<Path>) -> Result<XfdfDocument> {
    let mut exporter = PdfAnnotationExporter::new();
    exporter.load_annotations_from_pdf(path.as_ref())
}

/// 把 PDF 里的注释直接导出成一个标准 XFDF 文件。
///
/// # 示例
///
/// ```no_run
/// use pdfxml::export_pdf_annotations_to_xfdf;
///
/// export_pdf_annotations_to_xfdf("annotated.pdf", "exported.xfdf")?;
/// # Ok::<(), pdfxml::PdfXmlError>(())
/// ```
pub fn export_pdf_annotations_to_xfdf(
    input_pdf: impl AsRef<Path>,
    output_xfdf: impl AsRef<Path>,
) -> Result<()> {
    let xfdf_doc = load_annotations_from_pdf(input_pdf)?;
    let xml = xfdf_doc.to_xfdf_string()?;
    fs::write(output_xfdf, xml)
        .map_err(|e| PdfXmlError::PdfProcessing(format!("写入 XFDF 文件失败: {}", e)))?;
    Ok(())
}

/// 把已经解析好的注释导出成 PDF。
///
/// 规则很简单：
/// - 如果给了 `target_pdf`，就把注释合并进已有 PDF
/// - 如果没有给 `target_pdf`，就新建一个 PDF 来放这些注释
///
/// 这个函数适合直接给 SDK 调用方使用。
/// 如果你需要更细的控制，也可以直接使用 [`PdfAnnotationExporter`]。
///
/// # 示例
///
/// ```no_run
/// use pdfxml::{export_annotations, XfdfDocument};
///
/// let doc = XfdfDocument::parse(r#"
/// <?xml version="1.0" encoding="UTF-8" ?>
/// <xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
///   <annots>
///     <text page="0" rect="100,700,250,730">Hello</text>
///   </annots>
/// </xfdf>
/// "#)?;
/// export_annotations(&doc, Option::<&str>::None, "output.pdf")?;
/// # Ok::<(), pdfxml::PdfXmlError>(())
/// ```
pub fn export_annotations(
    xfdf_doc: &XfdfDocument,
    target_pdf: Option<impl AsRef<Path>>,
    output_path: impl AsRef<Path>,
) -> Result<()> {
    let mut exporter = PdfAnnotationExporter::new();
    match target_pdf {
        Some(target_pdf) => {
            exporter.export_to_existing_pdf(xfdf_doc, target_pdf.as_ref(), output_path.as_ref())
        }
        None => exporter.export_to_new_pdf(xfdf_doc, output_path.as_ref()),
    }
}
