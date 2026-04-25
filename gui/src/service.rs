use anyhow::{Context, Result};
use pdfxml::{export_annotations, load_annotations_from_pdf, load_xfdf, Annotation, XfdfDocument};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConversionMode {
    #[default]
    XfdfToPdf,
    PdfToXfdf,
}

impl ConversionMode {
    pub fn output_extension(self) -> &'static str {
        match self {
            Self::XfdfToPdf => "pdf",
            Self::PdfToXfdf => "xfdf",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AnnotationRow {
    pub index: usize,
    pub annotation_type: String,
    pub page: usize,
    pub title: String,
    pub subject: String,
    pub contents: String,
    pub rect: String,
}

#[derive(Debug, Clone, Default)]
pub struct AnnotationOverview {
    pub rows: Vec<AnnotationRow>,
    pub total_annotations: usize,
    pub total_pages: usize,
    pub type_counts: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
pub struct OperationResult {
    pub overview: AnnotationOverview,
    pub message: String,
}

pub fn suggest_output_path(input: impl AsRef<Path>, mode: ConversionMode) -> PathBuf {
    let mut path = input.as_ref().to_path_buf();
    path.set_extension(mode.output_extension());
    path
}

pub fn load_annotation_overview(
    mode: ConversionMode,
    input_path: impl AsRef<Path>,
) -> Result<AnnotationOverview> {
    let document = match mode {
        ConversionMode::XfdfToPdf => load_xfdf(&input_path)
            .with_context(|| format!("读取 XFDF 失败: {}", input_path.as_ref().display()))?,
        ConversionMode::PdfToXfdf => load_annotations_from_pdf(&input_path)
            .with_context(|| format!("读取 PDF 批注失败: {}", input_path.as_ref().display()))?,
    };

    Ok(build_overview(&document))
}

pub fn convert(
    mode: ConversionMode,
    input_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    target_pdf: Option<&Path>,
) -> Result<OperationResult> {
    match mode {
        ConversionMode::XfdfToPdf => {
            let document = load_xfdf(&input_path)
                .with_context(|| format!("读取 XFDF 失败: {}", input_path.as_ref().display()))?;
            export_annotations(&document, target_pdf, &output_path)
                .with_context(|| format!("导出 PDF 失败: {}", output_path.as_ref().display()))?;

            Ok(OperationResult {
                overview: build_overview(&document),
                message: format!(
                    "已导出 PDF：{}（{} 条批注）",
                    output_path.as_ref().display(),
                    document.annotations.len()
                ),
            })
        }
        ConversionMode::PdfToXfdf => {
            let document = load_annotations_from_pdf(&input_path)
                .with_context(|| format!("读取 PDF 批注失败: {}", input_path.as_ref().display()))?;
            let xml = document.to_xfdf_string().context("序列化 XFDF 失败")?;
            fs::write(&output_path, xml)
                .with_context(|| format!("写入 XFDF 失败: {}", output_path.as_ref().display()))?;

            Ok(OperationResult {
                overview: build_overview(&document),
                message: format!(
                    "已导出 XFDF：{}（{} 条批注）",
                    output_path.as_ref().display(),
                    document.annotations.len()
                ),
            })
        }
    }
}

fn build_overview(document: &XfdfDocument) -> AnnotationOverview {
    let mut type_counts = BTreeMap::new();
    let mut rows = Vec::with_capacity(document.annotations.len());

    for (index, annotation) in document.annotations.iter().enumerate() {
        *type_counts
            .entry(annotation.annotation_type().to_string())
            .or_insert(0usize) += 1;
        rows.push(annotation_to_row(index + 1, annotation));
    }

    AnnotationOverview {
        total_annotations: document.annotations.len(),
        total_pages: document.total_pages(),
        rows,
        type_counts: type_counts.into_iter().collect(),
    }
}

fn annotation_to_row(index: usize, annotation: &Annotation) -> AnnotationRow {
    let base = annotation.base();

    AnnotationRow {
        index,
        annotation_type: annotation.annotation_type().to_string(),
        page: annotation.page() + 1,
        title: base.title.clone().unwrap_or_default(),
        subject: base.subject.clone().unwrap_or_default(),
        contents: preview_text(base.contents.as_deref().unwrap_or_default()),
        rect: annotation
            .rect()
            .map(|rect| {
                format!(
                    "{:.2}, {:.2}, {:.2}, {:.2}",
                    rect.left, rect.bottom, rect.right, rect.top
                )
            })
            .unwrap_or_default(),
    }
}

fn preview_text(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = compact.chars().count();
    if count <= 60 {
        compact
    } else {
        let truncated = compact.chars().take(60).collect::<String>();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_output_extension_from_mode() {
        let output = suggest_output_path("demo.pdf", ConversionMode::PdfToXfdf);
        assert_eq!(
            output.extension().and_then(|ext| ext.to_str()),
            Some("xfdf")
        );
    }

    #[test]
    fn loads_sample_xfdf_into_rows() {
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/sample.xfdf");
        let overview = load_annotation_overview(ConversionMode::XfdfToPdf, &input)
            .expect("sample xfdf should load");

        assert!(overview.total_annotations >= 1);
        assert_eq!(overview.rows.len(), overview.total_annotations);
    }
}
