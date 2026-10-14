//! 这些是集成测试。
//!
//! 和“只测一个小函数”的单元测试不同，
//! 这里更关心整条真实流程能不能走通：
//! - XFDF 能不能正确解析
//! - PDF 能不能正确导出
//! - 导出后再读回来，关键字段会不会丢

use pdfxml::{load_annotations_from_pdf, Annotation, PdfAnnotationExporter, XfdfDocument};
use std::collections::HashMap;
use std::fs;
use tempfile::NamedTempFile;

// 这些测试不是只测某个小函数，
// 而是尽量模拟“真实使用时会怎么走”。
// 也就是说，它们更像是在检查：
// “从输入到输出这一整条路，最后有没有按预期工作”。

#[test]
fn test_full_workflow_minimal_xfdf() {
    // 最小流程测试：
    // 给一段最简单的 XFDF，确认能成功解析出 1 条注释。
    let xfdf_content = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text subject="Test" page="0" rect="100,700,250,730" color="#FFFF00">
      Hello World
    </text>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf_content).expect("Failed to parse XFDF");

    assert_eq!(doc.annotations.len(), 1);
    assert_eq!(doc.total_pages(), 1);
}

#[test]
fn test_export_to_pdf() {
    // 导出测试：
    // 确认一份带多条注释、跨多页的 XFDF，
    // 真的能写出一个非空 PDF 文件。
    let xfdf_content = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text subject="Comment 1" page="0" rect="100,700,250,730" color="#FF0000">
      First comment
    </text>
    <highlight page="0" rect="50,600,200,620" color="#FFFF00"/>
    <text subject="Comment on page 2" page="1" rect="100,500,300,550" color="#00FF00">
      Second page comment
    </text>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf_content).expect("Failed to parse XFDF");
    assert_eq!(doc.annotations.len(), 3);

    let output_file = NamedTempFile::new().expect("Failed to create temp file");
    let output_path = output_file.path().to_path_buf();

    let mut exporter = PdfAnnotationExporter::new();
    let result = exporter.export_to_new_pdf(&doc, &output_path);

    assert!(result.is_ok());
    assert!(output_path.exists());

    let metadata = fs::metadata(&output_path).unwrap();
    assert!(metadata.len() > 0);
}

#[test]
fn test_round_trip_pdf_to_xfdf() {
    // round-trip 测试：
    // 先 XFDF -> PDF，再 PDF -> XFDF，
    // 看常见字段有没有在来回过程中丢掉。
    let xfdf_content = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text name="note-1" subject="Comment 1" page="0" rect="100,700,250,730" title="Alice" color="#FF0000">
      First comment
    </text>
    <square page="0" rect="50,600,200,620" color="#FFFF00" width="2"/>
    <circle page="1" rect="100,500,300,550" color="#00FF00" interiorcolor="#FFCCCC" width="3"/>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf_content).expect("Failed to parse XFDF");
    let output_file = NamedTempFile::new().expect("Failed to create temp pdf");
    let output_path = output_file.path().to_path_buf();

    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_new_pdf(&doc, &output_path).expect("Failed to export pdf");

    let loaded = load_annotations_from_pdf(&output_path).expect("Failed to load annotations from pdf");
    assert_eq!(loaded.annotations.len(), 3);
    assert_eq!(loaded.total_pages(), 2);

    let xml = loaded.to_xfdf_string().expect("Failed to serialize xfdf");
    assert!(xml.contains("<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">"));
    assert!(xml.contains("<contents>First comment</contents>"));
    assert!(xml.contains("<text name=\"note-1\" page=\"0\" rect=\"100,700,250,730\" title=\"Alice\" subject=\"Comment 1\" color=\"#FF0000\">"));
    assert!(xml.contains("<square page=\"0\" rect=\"50,600,200,620\" color=\"#FFFF00\" style=\"solid\" width=\"2\"/>"));
    assert!(xml.contains("<circle page=\"1\" rect=\"100,500,300,550\" color=\"#00FF00\" style=\"solid\" width=\"3\" interiorcolor=\"#FFCCCC\"/>"));
}

#[test]
fn test_round_trip_freetext_textcolor_pdf_to_xfdf() {
    // 检查 FreeText 的文字颜色和边框颜色能不能一起保住。
    let xfdf_content = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <freetext page="0" rect="100,700,250,760" color="#0000FF" TextColor="#FF0000" subject="FreeText">Hello FreeText</freetext>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf_content).expect("Failed to parse freetext XFDF");
    let output_file = NamedTempFile::new().expect("Failed to create temp pdf");
    let output_path = output_file.path().to_path_buf();

    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_new_pdf(&doc, &output_path).expect("Failed to export freetext pdf");

    let loaded = load_annotations_from_pdf(&output_path).expect("Failed to load freetext annotations from pdf");
    assert_eq!(loaded.annotations.len(), 1);

    let xml = loaded.to_xfdf_string().expect("Failed to serialize freetext xfdf");
    assert!(xml.contains("<freetext"));
    assert!(xml.contains("TextColor=\"#FF0000\""));
    assert!(xml.contains("color=\"#0000FF\""));
}

#[test]
fn test_round_trip_freetext_textcolor_precedes_color() {
    // 检查优先级：
    // 如果同时有 TextColor 和 color，
    // 应该优先把文字颜色当成文字颜色，而不是被外框颜色盖掉。
    let xfdf_content = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <freetext page="0" rect="120,620,280,690" color="#00FF00" TextColor="#112233">Priority check</freetext>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf_content).expect("Failed to parse freetext priority XFDF");
    let output_file = NamedTempFile::new().expect("Failed to create temp pdf");
    let output_path = output_file.path().to_path_buf();

    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_new_pdf(&doc, &output_path).expect("Failed to export freetext priority pdf");

    let loaded = load_annotations_from_pdf(&output_path).expect("Failed to load freetext priority annotations from pdf");
    let xml = loaded.to_xfdf_string().expect("Failed to serialize freetext priority xfdf");

    assert!(xml.contains("TextColor=\"#112233\""));
    assert!(xml.contains("color=\"#00FF00\""));
}

#[test]
fn test_round_trip_popup_name_and_opacity() {
    // 检查两件之前容易丢的东西：
    // 1. 注释名字 name
    // 2. 透明度 opacity
    // 还顺手检查 Popup 和父注释的关系能不能读回来。
    let xfdf_content = r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text name="note-1" page="0" rect="100,700,250,730" color="#FF0000" opacity="0.4">
      Parent note
    </text>
    <popup name="popup-1" page="0" rect="160,640,260,700" parent="note-1" open="yes"/>
  </annots>
</xfdf>"##;

    let doc = XfdfDocument::parse(xfdf_content).expect("Failed to parse popup XFDF");
    let output_file = NamedTempFile::new().expect("Failed to create temp pdf");
    let output_path = output_file.path().to_path_buf();

    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_new_pdf(&doc, &output_path).expect("Failed to export popup pdf");

    let loaded = load_annotations_from_pdf(&output_path).expect("Failed to load popup annotations from pdf");
    assert_eq!(loaded.annotations.len(), 2);

    let text = loaded
        .annotations
        .iter()
        .find_map(|annotation| match annotation {
            Annotation::Text(text) => Some(text),
            _ => None,
        })
        .expect("Missing text annotation");
    assert_eq!(text.base.name.as_deref(), Some("note-1"));
    assert!((text.base.opacity - 0.4).abs() < 0.0001);

    let popup = loaded
        .annotations
        .iter()
        .find_map(|annotation| match annotation {
            Annotation::Popup(popup) => Some(popup),
            _ => None,
        })
        .expect("Missing popup annotation");
    assert_eq!(popup.base.name.as_deref(), Some("popup-1"));
    assert_eq!(popup.parent_name.as_deref(), Some("note-1"));
    assert!(popup.open);
}

#[test]
fn test_export_to_existing_pdf_appends_missing_pages() {
    // 检查“补页”逻辑：
    // 原 PDF 只有 1 页，但注释出现在第 2 页时，
    // 导出逻辑应该把第 2 页补出来，而不是把注释丢掉。
    let base_pdf_doc = XfdfDocument {
        xmlns: Some("http://ns.adobe.com/xfdf/".to_string()),
        fields: Vec::new(),
        annotations: Vec::new(),
        metadata: HashMap::new(),
        ..XfdfDocument::default()
    };
    let merge_doc = XfdfDocument::parse(
        r##"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text page="1" rect="100,680,250,720" color="#3366FF">Second page note</text>
  </annots>
</xfdf>"##,
    )
    .expect("Failed to parse merge XFDF");

    let input_file = NamedTempFile::new().expect("Failed to create source pdf");
    let output_file = NamedTempFile::new().expect("Failed to create merged pdf");

    let mut exporter = PdfAnnotationExporter::new();
    exporter
        .export_to_new_pdf(&base_pdf_doc, input_file.path())
        .expect("Failed to create base pdf");
    exporter
        .export_to_existing_pdf(&merge_doc, input_file.path(), output_file.path())
        .expect("Failed to merge annotations into existing pdf");

    let loaded = load_annotations_from_pdf(output_file.path()).expect("Failed to load merged annotations");
    assert_eq!(loaded.annotations.len(), 1);
    assert_eq!(loaded.total_pages(), 2);
    assert_eq!(loaded.annotations[0].page(), 1);
}

fn classic_xref_pdf() -> Vec<u8> {
    let annotations = XfdfDocument::parse(
        r#"<xfdf xmlns="http://ns.adobe.com/xfdf/"><annots>
        <text name="original" page="0" rect="10,10,30,30"><contents>original note</contents></text>
        </annots></xfdf>"#,
    ).unwrap();
    let source = NamedTempFile::new().unwrap();
    PdfAnnotationExporter::new().export_to_new_pdf(&annotations, source.path()).unwrap();
    let mut document = lopdf::Document::load(source.path()).unwrap();
    document.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn inline_xref_pdf(separator: u8, suffix: &[u8]) -> Vec<u8> {
    let mut bytes = classic_xref_pdf();
    let xref_start = lopdf::Document::load_mem(&bytes).unwrap().xref_start;
    assert_eq!(&bytes[xref_start..xref_start + 5], b"xref\n");
    bytes[xref_start + 4] = separator;
    bytes.extend_from_slice(suffix);
    bytes
}

#[test]
fn test_pdf_input_inline_xref_and_webfastload() {
    let bytes = inline_xref_pdf(b' ', b"\nWebFastLoad\xEF\xBB\xBF<FileProperty><FileName>sample</FileName></FileProperty>");
    assert!(lopdf::Document::load_mem(&bytes).is_err());
    let input = NamedTempFile::new().unwrap();
    fs::write(input.path(), &bytes).unwrap();

    let loaded = load_annotations_from_pdf(input.path()).unwrap();
    assert_eq!(loaded.annotations.len(), 1);
    assert_eq!(loaded.annotations[0].base().name.as_deref(), Some("original"));
    assert_eq!(fs::read(input.path()).unwrap(), bytes);
}

#[test]
fn test_pdf_input_inline_xref_merge_preserves_content_and_source() {
    for separator in *b" \t" {
        let bytes = inline_xref_pdf(separator, b"\nWebFastLoad<FileProperty/>");
        let input = NamedTempFile::new().unwrap();
        let output = NamedTempFile::new().unwrap();
        fs::write(input.path(), &bytes).unwrap();
        let added = XfdfDocument::parse(
            r#"<xfdf xmlns="http://ns.adobe.com/xfdf/"><annots>
            <text name="added" page="0" rect="40,40,60,60"><contents>added note</contents></text>
            </annots></xfdf>"#,
        ).unwrap();
        PdfAnnotationExporter::new().export_to_existing_pdf(&added, input.path(), output.path()).unwrap();

        let merged = lopdf::Document::load(output.path()).unwrap();
        assert_eq!(merged.get_pages().len(), 1);
        assert!(merged.extract_text(&[1]).unwrap().contains("Annotations - Page 1"));
        let loaded = load_annotations_from_pdf(output.path()).unwrap();
        let names: Vec<_> = loaded.annotations.iter().map(|annotation| annotation.base().name.as_deref()).collect();
        assert_eq!(names, vec![Some("original"), Some("added")]);
        assert_eq!(fs::read(input.path()).unwrap(), bytes);
    }
}

#[test]
fn test_pdf_input_standard_xref_accepts_webfastload_without_fallback() {
    let mut bytes = classic_xref_pdf();
    bytes.extend_from_slice(b"\nWebFastLoad\xEF\xBB\xBF<FileProperty/>");
    assert!(lopdf::Document::load_mem(&bytes).is_ok());
    let input = NamedTempFile::new().unwrap();
    fs::write(input.path(), &bytes).unwrap();
    assert_eq!(load_annotations_from_pdf(input.path()).unwrap().annotations.len(), 1);
    assert_eq!(fs::read(input.path()).unwrap(), bytes);
}

#[test]
fn test_pdf_input_invalid_xref_does_not_overwrite_output() {
    let mut bytes = inline_xref_pdf(b' ', b"");
    let startxref = bytes.windows(9).rposition(|window| window == b"startxref").unwrap();
    bytes[startxref + 10] = b'x';
    let input = NamedTempFile::new().unwrap();
    let output = NamedTempFile::new().unwrap();
    fs::write(input.path(), &bytes).unwrap();
    fs::write(output.path(), b"keep existing output").unwrap();

    assert!(load_annotations_from_pdf(input.path()).is_err());
    assert!(PdfAnnotationExporter::new().export_to_existing_pdf(&XfdfDocument::default(), input.path(), output.path()).is_err());
    assert_eq!(fs::read(input.path()).unwrap(), bytes);
    assert_eq!(fs::read(output.path()).unwrap(), b"keep existing output");
}

#[test]
fn test_pdf_input_broken_latest_revision_is_not_rolled_back() {
    let mut bytes = inline_xref_pdf(b' ', b"");
    bytes.extend_from_slice(b"\nstartxref\n99999999\n%%EOF\n");
    let input = NamedTempFile::new().unwrap();
    fs::write(input.path(), &bytes).unwrap();
    assert!(load_annotations_from_pdf(input.path()).is_err());
    assert_eq!(fs::read(input.path()).unwrap(), bytes);
}

#[test]
fn test_pdf_input_inline_xref_preserves_stream_bytes() {
    let mut document = lopdf::Document::load_mem(&classic_xref_pdf()).unwrap();
    let page_id = document.get_pages()[&1];
    let stream_id = document.get_page_contents(page_id)[0];
    let mut content = document.get_object(stream_id).unwrap().as_stream().unwrap().content.clone();
    content.extend_from_slice(b"\n% xref 0 175\n% startxref\n% %%EOF\n");
    document.set_object(stream_id, lopdf::Stream::new(lopdf::Dictionary::new(), content.clone()));
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let xref_start = lopdf::Document::load_mem(&bytes).unwrap().xref_start;
    bytes[xref_start + 4] = b' ';

    let input = NamedTempFile::new().unwrap();
    let output = NamedTempFile::new().unwrap();
    fs::write(input.path(), &bytes).unwrap();
    PdfAnnotationExporter::new().export_to_existing_pdf(
        &XfdfDocument::default(), input.path(), output.path(),
    ).unwrap();

    let merged = lopdf::Document::load(output.path()).unwrap();
    assert_eq!(merged.get_object(stream_id).unwrap().as_stream().unwrap().content, content);
    assert_eq!(fs::read(input.path()).unwrap(), bytes);
}

#[test]
fn test_pdf_input_inline_xref_with_header_prefix() {
    let mut bytes = b"external prefix\n".to_vec();
    bytes.extend_from_slice(&inline_xref_pdf(b' ', b"\nWebFastLoad<FileProperty/>"));
    let input = NamedTempFile::new().unwrap();
    fs::write(input.path(), &bytes).unwrap();
    let loaded = load_annotations_from_pdf(input.path()).unwrap();
    assert_eq!(loaded.annotations.len(), 1);
    assert_eq!(loaded.annotations[0].base().name.as_deref(), Some("original"));
    assert_eq!(fs::read(input.path()).unwrap(), bytes);
}

#[test]
fn test_pdf_input_inline_xref_uses_latest_incremental_revision() {
    let previous_bytes = classic_xref_pdf();
    let previous_document = lopdf::Document::load_mem(&previous_bytes).unwrap();
    let annotation_id = previous_document.objects.iter().find_map(|(object_id, object)| {
        let dictionary = object.as_dict().ok()?;
        (dictionary.get(b"NM").ok()?.as_str().ok()? == b"original").then_some(*object_id)
    }).unwrap();
    let mut incremental = lopdf::IncrementalDocument::create_from(previous_bytes, previous_document);
    incremental.opt_clone_object_to_new_document(annotation_id).unwrap();
    incremental.new_document.get_object_mut(annotation_id).unwrap().as_dict_mut().unwrap()
        .set("Contents", lopdf::Object::string_literal("latest revision"));
    let mut bytes = Vec::new();
    incremental.save_to(&mut bytes).unwrap();
    let latest_xref = lopdf::Document::load_mem(&bytes).unwrap().xref_start;
    assert_eq!(&bytes[latest_xref..latest_xref + 5], b"xref\n");
    bytes[latest_xref + 4] = b' ';
    bytes.extend_from_slice(b"\nWebFastLoad<FileProperty/>");

    let input = NamedTempFile::new().unwrap();
    fs::write(input.path(), &bytes).unwrap();
    let loaded = load_annotations_from_pdf(input.path()).unwrap();
    assert_eq!(loaded.annotations.len(), 1);
    assert_eq!(loaded.annotations[0].base().contents.as_deref(), Some("latest revision"));
    assert_eq!(fs::read(input.path()).unwrap(), bytes);
}
