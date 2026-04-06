//! 集成测试

use pdfxml::{PdfAnnotationExporter, XfdfDocument};
use std::fs;
use tempfile::NamedTempFile;

#[test]
fn test_full_workflow_minimal_xfdf() {
    let xfdf_content = r#"<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text subject="Test" page="0" rect="100,700,250,730" color="#FFFF00">
      Hello World
    </text>
  </annots>
</xfdf>"#;

    let doc = XfdfDocument::parse(xfdf_content).expect("Failed to parse XFDF");
    
    assert_eq!(doc.annotations.len(), 1);
    assert_eq!(doc.total_pages(), 1);
}

#[test]
fn test_export_to_pdf() {
    let xfdf_content = r#"<?xml version="1.0" encoding="UTF-8" ?>
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
</xfdf>"#;

    let doc = XfdfDocument::parse(xfdf_content).expect("Failed to parse XFDF");
    assert_eq!(doc.annotations.len(), 3);

    // 创建临时输出文件
    let output_file = NamedTempFile::new().expect("Failed to create temp file");
    let output_path = output_file.path().to_path_buf();
    
    // 导出为 PDF
    let exporter = PdfAnnotationExporter::new();
    let result = exporter.export_to_new_pdf(&doc, &output_path);
    
    assert!(result.is_ok());
    
    // 验证文件已创建
    assert!(output_path.exists());
    
    // 检查文件大小（应该大于0）
    let metadata = fs::metadata(&output_path).unwrap();
    assert!(metadata.len() > 0);
}
