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
    assert!(xml.contains("<text name=\"note-1\" page=\"0\" rect=\"100,700,250,730\" title=\"Alice\" subject=\"Comment 1\" color=\"#FF0000\">First comment</text>"));
    assert!(xml.contains("<square page=\"0\" rect=\"50,600,200,620\" color=\"#FFFF00\" width=\"2\"/>"));
    assert!(xml.contains("<circle page=\"1\" rect=\"100,500,300,550\" color=\"#00FF00\" width=\"3\" interiorcolor=\"#FFCCCC\"/>"));
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
