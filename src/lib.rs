pub mod annotation;
pub mod error;
pub mod pdf;
pub mod xfdf;

pub use annotation::*;
pub use error::{PdfXmlError, Result};
pub use pdf::PdfAnnotationExporter;
pub use xfdf::{XfdfDocument, XfdfField};

use std::fs;
use std::path::Path;

pub fn load_xfdf(path: impl AsRef<Path>) -> Result<XfdfDocument> {
    let content = fs::read_to_string(path)
        .map_err(|e| PdfXmlError::PdfProcessing(format!("读取 XFDF 文件失败: {}", e)))?;
    XfdfDocument::parse(&content)
}

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
