use crate::error::{PdfXmlError, Result};
use log::warn;
use lopdf::Document;
use std::fs;
use std::path::Path;

pub(super) fn load_document(input_path: &Path) -> Result<Document> {
    let mut bytes = fs::read(input_path)
        .map_err(|error| PdfXmlError::PdfProcessing(format!("加载PDF失败: {error}")))?;
    let original_error = match Document::load_mem(&bytes) {
        Ok(document) => return Ok(document),
        Err(error) => error,
    };

    if matches!(&original_error, lopdf::Error::Parse(_)) {
        if let Some(separator) = inline_xref_separator(&bytes) {
            bytes[separator] = b'\n';
            return match Document::load_mem(&bytes) {
                Ok(document) => {
                    warn!("PDF 兼容读取：xref 与首个分段共行，在内存中兼容其分隔符；原文件和对象偏移未改变");
                    Ok(document)
                }
                Err(error) => Err(PdfXmlError::PdfProcessing(format!(
                    "加载PDF失败: {original_error}；xref 分隔符兼容重试失败: {error}"
                ))),
            };
        }
    }

    Err(PdfXmlError::PdfProcessing(format!(
        "加载PDF失败: {original_error}"
    )))
}

fn inline_xref_separator(bytes: &[u8]) -> Option<usize> {
    let header_offset = bytes.windows(5).position(|window| window == b"%PDF-")?;
    let document = &bytes[header_offset..];
    let eof_offset = document.windows(5).rposition(|window| window == b"%%EOF")?;
    let footer_offset = document[..eof_offset]
        .windows(9)
        .rposition(|window| window == b"startxref")?;
    if footer_offset == 0 || !is_pdf_whitespace(document[footer_offset - 1]) {
        return None;
    }

    if !is_pdf_whitespace(*document.get(footer_offset + 9)?) {
        return None;
    }
    let mut fields = document[footer_offset + 9..eof_offset]
        .split(|byte| is_pdf_whitespace(*byte))
        .filter(|field| !field.is_empty());
    let offset_field = fields.next()?;
    if fields.next().is_some() || !offset_field.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let xref_offset = std::str::from_utf8(offset_field)
        .ok()?
        .parse::<usize>()
        .ok()?;
    if xref_offset >= footer_offset {
        return None;
    }

    let section = &document[xref_offset..footer_offset];
    if !section.starts_with(b"xref")
        || !matches!(section.get(4), Some(b' ' | b'\t'))
        || !section.get(5).is_some_and(u8::is_ascii_digit)
    {
        return None;
    }
    Some(header_offset + xref_offset + 4)
}

fn is_pdf_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 12 | b'\r' | b' ')
}
