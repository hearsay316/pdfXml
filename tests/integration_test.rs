//! 集成测试

use pdfxml::{load_annotations_from_pdf, PdfAnnotationExporter, XfdfDocument};
use std::fs;
use tempfile::NamedTempFile;

#[test]
fn test_full_workflow_minimal_xfdf() {
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
    assert!(xml.contains("<text page=\"0\" rect=\"100,700,250,730\" title=\"Alice\" subject=\"Comment 1\" color=\"#FF0000\">First comment</text>"));
    assert!(xml.contains("<square page=\"0\" rect=\"50,600,200,620\" color=\"#FFFF00\" width=\"2\"/>"));
    assert!(xml.contains("<circle page=\"1\" rect=\"100,500,300,550\" color=\"#00FF00\" width=\"3\" interiorcolor=\"#FFCCCC\"/>"));
}

#[test]
fn test_round_trip_freetext_textcolor_pdf_to_xfdf() {
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
