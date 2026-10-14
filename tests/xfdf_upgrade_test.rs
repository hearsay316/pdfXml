//! Regression coverage for XFDF fidelity, diagnostics and interactive annotations.
use pdfxml::{
    load_annotations_from_pdf, load_xfdf_with_options, Annotation, CaretSymbol, Color,
    LinkDestinationMode, LinkTarget, PdfAnnotationExporter, Rect, XfdfDiagnosticCode, XfdfDocument,
    XfdfParseMode, XfdfParseOptions, XFDF_NAMESPACE,
};
use std::fs;
use tempfile::NamedTempFile;

fn document(body: &str) -> String {
    format!(r#"<xfdf xmlns="{XFDF_NAMESPACE}" xml:space="preserve">{body}</xfdf>"#)
}

fn strict(xml: &str) -> pdfxml::Result<pdfxml::XfdfParseReport> {
    XfdfDocument::parse_with_options(
        xml,
        &XfdfParseOptions {
            mode: XfdfParseMode::Strict,
            ..Default::default()
        },
    )
}

#[test]
fn fields_references_ids_metadata_and_entities_round_trip() {
    let xml = r#"<x:xfdf xmlns:x="http://ns.adobe.com/xfdf/" xml:space="preserve" custom="a &amp; &quot;b&quot;">
      <x:f href="a&amp;b.pdf"/><x:ids original="a&amp;b" modified="modified"/>
      <x:fields><x:field name="A &amp; B"><x:value> leading <![CDATA[<中>]]>&amp; trailing </x:value><x:value/><x:value>third</x:value>
        <x:field name="child"><x:value>nested</x:value></x:field><x:field name="empty"/>
      </x:field></x:fields></x:xfdf>"#;
    let report = strict(xml).unwrap();
    assert!(report.diagnostics.is_empty());
    let doc = report.document;
    assert_eq!(doc.file_reference.as_deref(), Some("a&b.pdf"));
    assert_eq!(doc.ids.as_ref().unwrap().original.as_deref(), Some("a&b"));
    assert_eq!(doc.metadata["custom"], "a & \"b\"");
    let field = &doc.fields[0];
    assert_eq!(field.name, "A & B");
    assert_eq!(
        field.values().collect::<Vec<_>>(),
        [" leading <中>& trailing ", "", "third"]
    );
    assert_eq!(field.children[0].value.as_deref(), Some("nested"));
    assert!(field.children[1].value.is_none());
    let output = doc.to_xfdf_string().unwrap();
    let second = strict(&output).unwrap().document;
    assert_eq!(
        second.fields[0].values().collect::<Vec<_>>(),
        field.values().collect::<Vec<_>>()
    );
    assert_eq!(second.ids, doc.ids);
    assert_eq!(second.metadata["custom"], doc.metadata["custom"]);
    assert!(!second.pdf_conversion_warnings().is_empty());
}

#[test]
fn annotation_contents_cdata_entities_and_whitespace_are_not_lost() {
    let doc = strict(&document(r#"<annots><text name="a&amp;b" page="0" rect="0,0,10,20" title="a &quot;quote&quot;"><contents> first <![CDATA[<x>]]>&amp;
last </contents></text><text page="0" rect="0,0,1,1"><contents/></text></annots>"#)).unwrap().document;
    assert_eq!(doc.annotations[0].base().name.as_deref(), Some("a&b"));
    assert_eq!(
        doc.annotations[0].base().title.as_deref(),
        Some("a \"quote\"")
    );
    assert_eq!(
        doc.annotations[0].base().contents.as_deref(),
        Some(" first <x>&\nlast ")
    );
    assert_eq!(doc.annotations[1].base().contents.as_deref(), Some(""));
    let reread = strict(&doc.to_xfdf_string().unwrap()).unwrap().document;
    assert_eq!(
        reread.annotations[0].base().contents,
        doc.annotations[0].base().contents
    );
    assert_eq!(
        reread.annotations[1].base().contents,
        doc.annotations[1].base().contents
    );
}

#[test]
fn rich_text_styles_and_unknown_children_survive_xfdf_round_trip() {
    let doc = strict(&document(r#"<annots><text page="0" rect="0,0,10,20">
      <contents-richtext><body xmlns="http://www.w3.org/1999/xhtml"><p><span style="font-weight:bold">Hello &amp; world</span></p><p>Second</p></body></contents-richtext>
      <vendor:data xmlns:vendor="urn:vendor" label="a&amp;b"><vendor:item>extra</vendor:item></vendor:data>
      </text></annots>"#)).unwrap().document;
    assert_eq!(
        doc.annotations[0].base().contents.as_deref(),
        Some("Hello & world\nSecond")
    );
    let rich = &doc.annotations[0].base().extra["contents-richtext"];
    assert!(rich.contains("font-weight:bold"));
    let output = doc.to_xfdf_string().unwrap();
    assert!(output.contains("vendor:item"));
    let reread = strict(&output).unwrap().document;
    assert_eq!(
        &reread.annotations[0].base().extra["contents-richtext"],
        rich
    );
    assert_eq!(
        reread.annotations[0].base().contents,
        doc.annotations[0].base().contents
    );
}

#[test]
fn rich_text_in_cdata_can_round_trip_non_xml_html_entities() {
    let doc = XfdfDocument::parse(&document(r#"<annots><text page="0" rect="0,0,20,20"><contents-richtext><![CDATA[<body><p>A&nbsp;B</p></body>]]></contents-richtext></text></annots>"#)).unwrap();
    assert_eq!(doc.annotations[0].base().contents.as_deref(), Some("A B"));
    let output = doc.to_xfdf_string().unwrap();
    let reread = XfdfDocument::parse(&output).unwrap();
    assert_eq!(
        reread.annotations[0].base().extra["contents-richtext"],
        doc.annotations[0].base().extra["contents-richtext"]
    );
}

#[test]
fn nested_popups_do_not_replace_the_parent_annotation() {
    for popup in [
        r#"<popup rect="20,20,40,40" open="yes"/>"#,
        r#"<popup rect="20,20,40,40" open="yes"><contents>popup</contents></popup>"#,
    ] {
        let xml = document(&format!(
            r#"<annots><text page="2" rect="0,0,10,20"><contents>parent</contents>{popup}</text><text name="next" page="0" rect="1,1,2,2"/></annots>"#
        ));
        let doc = strict(&xml).unwrap().document;
        assert_eq!(doc.annotations.len(), 3);
        assert!(matches!(doc.annotations[0], Annotation::Text(_)));
        assert_eq!(
            doc.annotations[0].base().contents.as_deref(),
            Some("parent")
        );
        match &doc.annotations[1] {
            Annotation::Popup(popup) => {
                assert_eq!(popup.parent_name, doc.annotations[0].base().name);
                assert_eq!(popup.base.page, 2);
                assert!(popup.open);
            }
            _ => panic!("expected popup"),
        }
        let reread = strict(&doc.to_xfdf_string().unwrap()).unwrap().document;
        assert_eq!(reread.annotations.len(), 3);
    }
}

#[test]
fn flags_support_names_and_keep_legacy_hex_compatibility() {
    for (value, mask) in [
        ("print,readonly,locked", 196),
        ("hidden nozoom norotate", 26),
        ("10", 16),
        ("0xA", 10),
        ("0x8000", 32768),
    ] {
        let doc = strict(&document(&format!(
            r#"<annots><text page="0" rect="0,0,1,1" flags="{value}"/></annots>"#
        )))
        .unwrap()
        .document;
        assert_eq!(doc.annotations[0].base().flags, mask);
        let output = doc.to_xfdf_string().unwrap();
        let reread = strict(&output).unwrap().document;
        assert_eq!(reread.annotations[0].base().flags, mask);
    }
}

#[test]
fn invalid_attributes_are_diagnosed_and_strict_mode_rejects_them() {
    for (tag, key, value) in [
        ("text", "page", "-1"),
        ("text", "rect", "0,NaN,1,2"),
        ("text", "opacity", "2"),
        ("text", "color", "三abc"),
        ("text", "flags", "unknown"),
        ("square", "width", "NaN"),
        ("freetext", "align", "3"),
        ("caret", "symbol", "unknown"),
        ("caret", "fringe", "-1,0,0,0"),
    ] {
        let xml = document(
            &format!(r#"<annots><{tag} page="0" rect="0,0,1,1" {key}="{value}"/></annots>"#)
                .replace(
                    if key == "page" {
                        "page=\"0\" "
                    } else if key == "rect" {
                        "rect=\"0,0,1,1\" "
                    } else {
                        "__not_present__"
                    },
                    "",
                ),
        );
        let report = XfdfDocument::parse_with_report(&xml).unwrap();
        assert_eq!(report.document.annotations.len(), 1);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == XfdfDiagnosticCode::InvalidAttribute),
            "{key}"
        );
        assert!(strict(&xml).is_err());
    }
    assert!(Color::from_hex("三abc").is_none());
    assert!(Rect::from_string("0,0,inf,10").is_none());
}

#[test]
fn malformed_attributes_and_document_structure_never_succeed_silently() {
    for xml in [
        r#"<xfdf xmlns="http://ns.adobe.com/xfdf/"><annots><text page="0" page="1"/></annots></xfdf>"#,
        r#"<xfdf xmlns="http://ns.adobe.com/xfdf/"><annots><text title=unquoted/></annots></xfdf>"#,
        r#"<xfdf xmlns="http://ns.adobe.com/xfdf/"><annots>"#,
        r#"<xfdf xmlns="http://ns.adobe.com/xfdf/"/><xfdf/>"#,
        "<other/>",
        "",
        "text<xfdf/>",
        r#"<xfdf xmlns="urn:wrong"/>"#,
        r#"<x:xfdf/>"#,
        r#"<xfdf xmlns="http://ns.adobe.com/xfdf/" a:attr="x"/>"#,
    ] {
        assert!(
            XfdfDocument::parse(xml).is_err(),
            "unexpected success: {xml}"
        );
    }
    assert!(strict(r#"<x:xfdf xmlns:x="http://ns.adobe.com/xfdf/"/>"#).is_ok());
    let report = XfdfDocument::parse_with_report("<xfdf/>").unwrap();
    assert_eq!(
        report.diagnostics[0].code,
        XfdfDiagnosticCode::MissingNamespace
    );
    assert!(strict("<xfdf/>").is_err());
}

#[test]
fn spoofed_annotation_namespace_is_not_treated_as_a_supported_annotation() {
    let report = XfdfDocument::parse_with_report(&document(
        r#"<annots><v:text xmlns:v="urn:vendor" page="0" rect="0,0,10,20"/></annots>"#,
    ))
    .unwrap();
    assert!(report.document.annotations.is_empty());
    assert_eq!(report.document.unsupported_annotations.len(), 1);
    assert!(strict(&document(
        r#"<annots><v:text xmlns:v="urn:vendor"/></annots>"#
    ))
    .is_err());
}

#[test]
fn unsupported_annotations_are_reported_and_preserved_without_pdf_support_claims() {
    let xml = document(
        r#"<annots><fileattachment page="0" rect="0,0,1,1"><data>unsafe or unsupported</data></fileattachment><redact page="0" rect="0,0,1,1"/></annots>"#,
    );
    let report = XfdfDocument::parse_with_report(&xml).unwrap();
    assert!(report.document.annotations.is_empty());
    assert_eq!(report.document.unsupported_annotations.len(), 2);
    assert_eq!(report.diagnostics.len(), 2);
    assert!(strict(&xml).is_err());
    let output = report.document.to_xfdf_string().unwrap();
    let reread = XfdfDocument::parse_with_report(&output).unwrap();
    assert_eq!(reread.document.unsupported_annotations.len(), 2);
    assert!(!reread.document.pdf_conversion_warnings().is_empty());
}

#[test]
fn legacy_f_fields_wrapper_is_normalized_without_creating_a_fake_field() {
    let xml = document(
        r#"<f id="legacy"><fields><field name="Title"><value>Hello</value></field></fields></f>"#,
    );
    let report = XfdfDocument::parse_with_report(&xml).unwrap();
    assert_eq!(report.document.fields.len(), 1);
    assert_eq!(report.document.fields[0].name, "Title");
    assert!(report.document.file_reference.is_none());
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.code == XfdfDiagnosticCode::LegacyStructure));
    assert!(strict(&xml).is_err());
    assert!(strict(&report.document.to_xfdf_string().unwrap()).is_ok());
}

#[test]
fn xml_resource_limits_and_dtd_are_enforced() {
    let xml = document(r#"<annots><text page="0" rect="0,0,1,1"/></annots>"#);
    for options in [
        XfdfParseOptions {
            max_input_bytes: 8,
            ..Default::default()
        },
        XfdfParseOptions {
            max_depth: 2,
            ..Default::default()
        },
        XfdfParseOptions {
            max_nodes: 2,
            ..Default::default()
        },
        XfdfParseOptions {
            max_annotations: 0,
            ..Default::default()
        },
    ] {
        assert!(XfdfDocument::parse_with_options(&xml, &options).is_err());
    }
    for xml in [
        r#"<!DOCTYPE xfdf [<!ENTITY a SYSTEM "file:///C:/private">]><xfdf/>"#,
        r#"<?xml version="1.0" encoding="GBK"?><xfdf/>"#,
    ] {
        assert!(XfdfDocument::parse(xml).is_err());
    }
    let input = NamedTempFile::new().unwrap();
    fs::write(input.path(), document("<fields/>")).unwrap();
    assert!(load_xfdf_with_options(
        input.path(),
        &XfdfParseOptions {
            max_input_bytes: 5,
            ..Default::default()
        }
    )
    .is_err());
    fs::write(input.path(), [0xff, 0xfe, 0x00]).unwrap();
    assert!(load_xfdf_with_options(input.path(), &XfdfParseOptions::default()).is_err());
    fs::write(input.path(), "中文").unwrap();
    let error = load_xfdf_with_options(
        input.path(),
        &XfdfParseOptions {
            max_input_bytes: 1,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("字节限制"));
}

#[test]
fn deterministic_output_preserves_float_precision_and_ink_gestures() {
    let doc = strict(&document(r#"<annots><ink page="0" rect="0,0,100,100" opacity="0.12345678" zz="2" aa="1"><contents> Ink </contents><inklist><gesture>1;2;<![CDATA[3;4]]></gesture><gesture>5;6;7;8</gesture></inklist></ink></annots>"#)).unwrap().document;
    let first = doc.to_xfdf_string().unwrap();
    assert_eq!(first, doc.to_xfdf_string().unwrap());
    let reread = strict(&first).unwrap().document;
    assert_eq!(
        reread.annotations[0].base().opacity,
        doc.annotations[0].base().opacity
    );
    match &reread.annotations[0] {
        Annotation::Ink(ink) => assert_eq!(ink.ink_list, ["1;2;3;4", "5;6;7;8"]),
        _ => panic!("expected ink"),
    }
}

#[test]
fn standard_link_uri_named_and_page_modes_round_trip() {
    let targets = [
        r#"<OnActivation><Action><URI Name="https://example.test/?a=1&amp;b=2" IsMap="true"/></Action></OnActivation>"#,
        r#"<Dest><Named Name="chapter &amp; 1"/></Dest>"#,
        r#"<Dest><XYZ Page="1" Left="10" Top="null" Zoom="1.25"/></Dest>"#,
        r#"<Dest><Fit Page="1"/></Dest>"#,
        r#"<Dest><FitB Page="1"/></Dest>"#,
        r#"<Dest><FitH Page="1" Top="10"/></Dest>"#,
        r#"<Dest><FitBH Page="1" Top="10"/></Dest>"#,
        r#"<Dest><FitV Page="1" Left="10"/></Dest>"#,
        r#"<Dest><FitBV Page="1" Left="10"/></Dest>"#,
        r#"<OnActivation><Action><GoTo><Dest><FitR Page="1" Left="1" Bottom="2" Right="3" Top="4"/></Dest></GoTo></Action></OnActivation>"#,
    ];
    for target in targets {
        let xml = document(&format!(
            r#"<annots><link page="0" rect="0,0,10,20" width="2" highlight="Outline">{target}</link></annots>"#
        ));
        let doc = strict(&xml).unwrap().document;
        let reread = strict(&doc.to_xfdf_string().unwrap()).unwrap().document;
        match (&doc.annotations[0], &reread.annotations[0]) {
            (Annotation::Link(first), Annotation::Link(second)) => {
                assert_eq!(first.target, second.target)
            }
            _ => panic!("expected link"),
        }
    }
}

#[test]
fn link_convenience_attributes_are_normalized_and_unsafe_actions_are_not_implemented() {
    let doc = strict(&document(r#"<annots><link page="0" rect="0,0,10,20" uri="https://example.test/?a=1&amp;b=2"/></annots>"#)).unwrap().document;
    let output = doc.to_xfdf_string().unwrap();
    assert!(output.contains("<OnActivation>"));
    for target in [
        r#"<OnActivation><Action><JavaScript Name="alert(1)"/></Action></OnActivation>"#,
        r#"<OnActivation><Action><Launch/></Action></OnActivation>"#,
        r#"<Dest><FitR Page="1"/></Dest>"#,
        r#"<Dest><XYZ Page="1" Zoom="NaN"/></Dest>"#,
    ] {
        let xml = document(&format!(
            r#"<annots><link page="0" rect="0,0,10,20">{target}</link></annots>"#
        ));
        let report = XfdfDocument::parse_with_report(&xml).unwrap();
        assert!(report.document.annotations.is_empty());
        assert_eq!(report.document.unsupported_annotations.len(), 1);
        assert!(strict(&xml).is_err());
    }
}

#[test]
fn link_and_caret_pdf_round_trip_has_real_page_references_and_appearance() {
    let xml = document(
        r##"<annots>
      <link name="uri" page="0" rect="10,10,100,30" highlight="Push" width="2"><OnActivation><Action><URI Name="https://example.test/?a=1&amp;b=2" IsMap="true"/></Action></OnActivation></link>
      <link name="named" page="0" rect="10,40,100,60" dest="章节"/>
      <link name="page" page="0" rect="10,70,100,90"><Dest><XYZ Page="1" Left="12" Top="null" Zoom="1.5"/></Dest></link>
      <caret name="caret" page="1" rect="100,100,120,130" symbol="P" fringe="1,2,3,4" color="#334455" flags="print" opacity="0.5"><contents>插入段落</contents></caret>
    </annots>"##,
    );
    let original = strict(&xml).unwrap().document;
    let file = NamedTempFile::new().unwrap();
    PdfAnnotationExporter::new()
        .export_to_new_pdf(&original, file.path())
        .unwrap();
    let read = load_annotations_from_pdf(file.path()).unwrap();
    assert_eq!(read.annotations.len(), 4);
    for (first, second) in original.annotations.iter().zip(&read.annotations) {
        match (first, second) {
            (Annotation::Link(first), Annotation::Link(second)) => {
                assert_eq!(first.target, second.target);
                assert_eq!(first.highlight_mode, second.highlight_mode);
                assert_eq!(first.border_width, second.border_width);
            }
            (Annotation::Caret(first), Annotation::Caret(second)) => {
                assert_eq!(second.symbol, CaretSymbol::P);
                assert_eq!(second.fringe.as_ref().unwrap().top, 4.0);
                assert_eq!(second.base.contents, first.base.contents);
                assert_eq!(second.base.flags, 4);
                assert_eq!(second.base.opacity, 0.5);
            }
            _ => panic!("wrong annotation type"),
        }
    }
    let pdf = lopdf::Document::load(file.path()).unwrap();
    let pages = pdf.get_pages();
    let page = pdf.get_object(pages[&1]).unwrap().as_dict().unwrap();
    let annotations = page.get(b"Annots").unwrap().as_array().unwrap();
    let link = pdf
        .get_object(annotations[2].as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(
        link.get(b"Dest").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap(),
        pages[&2]
    );
    let page = pdf.get_object(pages[&2]).unwrap().as_dict().unwrap();
    let annotations = page.get(b"Annots").unwrap().as_array().unwrap();
    let caret = pdf
        .get_object(annotations[0].as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    let appearance = caret
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .as_reference()
        .unwrap();
    assert!(pdf.get_object(appearance).unwrap().as_stream().is_ok());
    assert!(strict(&read.to_xfdf_string().unwrap()).is_ok());
}

#[test]
fn merged_pdf_links_can_target_newly_appended_pages() {
    let base = XfdfDocument::default();
    let input = NamedTempFile::new().unwrap();
    let output = NamedTempFile::new().unwrap();
    let mut exporter = PdfAnnotationExporter::new();
    exporter.export_to_new_pdf(&base, input.path()).unwrap();
    let doc = strict(&document(r#"<annots><link page="0" rect="0,0,10,20"><Dest><Fit Page="1"/></Dest></link><caret page="1" rect="10,20,30,40"/></annots>"#)).unwrap().document;
    exporter
        .export_to_existing_pdf(&doc, input.path(), output.path())
        .unwrap();
    let read = load_annotations_from_pdf(output.path()).unwrap();
    assert_eq!(read.annotations.len(), 2);
    match &read.annotations[0] {
        Annotation::Link(link) => assert!(matches!(
            link.target,
            Some(LinkTarget::Page { page: 1, .. })
        )),
        _ => panic!("expected link"),
    }
}

#[test]
fn invalid_link_page_does_not_overwrite_output_and_exporter_is_reusable() {
    let doc = strict(&document(
        r#"<annots><link page="0" rect="0,0,10,20"><Dest><Fit Page="8"/></Dest></link></annots>"#,
    ))
    .unwrap()
    .document;
    let output = NamedTempFile::new().unwrap();
    fs::write(output.path(), b"unchanged").unwrap();
    let mut exporter = PdfAnnotationExporter::new();
    assert!(exporter.export_to_new_pdf(&doc, output.path()).is_err());
    assert_eq!(fs::read(output.path()).unwrap(), b"unchanged");
    exporter
        .export_to_new_pdf(&XfdfDocument::default(), output.path())
        .unwrap();
}

#[test]
fn rich_text_is_preserved_in_pdf_rc_without_claiming_styled_rendering() {
    let original = strict(&document(r#"<annots><freetext page="0" rect="0,0,120,40"><contents-richtext><body xmlns="http://www.w3.org/1999/xhtml"><p><b>Bold</b> &amp; plain</p></body></contents-richtext></freetext></annots>"#)).unwrap().document;
    let file = NamedTempFile::new().unwrap();
    PdfAnnotationExporter::new()
        .export_to_new_pdf(&original, file.path())
        .unwrap();
    let read = load_annotations_from_pdf(file.path()).unwrap();
    assert_eq!(
        read.annotations[0].base().extra["contents-richtext"],
        original.annotations[0].base().extra["contents-richtext"]
    );
    assert_eq!(
        read.annotations[0].base().contents,
        original.annotations[0].base().contents
    );
}

#[test]
fn repository_samples_remain_parseable_and_round_trip() {
    for xml in [
        include_str!("../examples/minimal.xfdf"),
        include_str!("../examples/sample.xfdf"),
        include_str!("../examples/index.xml"),
    ] {
        let original = XfdfDocument::parse_with_report(xml).unwrap().document;
        assert!(!original.annotations.is_empty());
        let output = original.to_xfdf_string().unwrap();
        let reread = XfdfDocument::parse(&output).unwrap();
        assert_eq!(reread.annotations.len(), original.annotations.len());
        assert_eq!(reread.fields.len(), original.fields.len());
    }
}

#[test]
fn manually_corrupted_raw_extension_cannot_be_serialized_as_valid_xml() {
    let doc = XfdfDocument {
        extensions: vec!["</xfdf><external/>".into()],
        ..Default::default()
    };
    assert!(doc.to_xfdf_string().is_err());
}

#[test]
fn destination_modes_keep_nulls_and_page_numbers_in_the_typed_model() {
    let doc = strict(&document(r#"<annots><link page="0" rect="0,0,1,1"><Dest><XYZ Page="3" Left="0" Zoom="2"/></Dest></link></annots>"#)).unwrap().document;
    match &doc.annotations[0] {
        Annotation::Link(link) => match &link.target {
            Some(LinkTarget::Page { page, view }) => {
                assert_eq!(*page, 3);
                assert_eq!(view.mode, LinkDestinationMode::XYZ);
                assert_eq!(view.parameters, [Some(0.0), None, Some(2.0)]);
            }
            _ => panic!("expected page target"),
        },
        _ => panic!("expected link"),
    }
}

#[test]
fn type_specific_attributes_do_not_consume_other_annotations_extensions() {
    let xml = document(
        r#"<annots>
        <freetext page="0" rect="0,0,100,50" fringe="1,2,3,4" width="2" symbol="custom" IT="FreeTextCallout" callout="1,2,3,4"/>
        <text page="0" rect="0,0,20,20" highlight="custom" align="left"/>
        <polyline page="0" rect="0,0,20,20" width="2" head="OpenArrow" vertices="0,0;20,20"/>
        <caret page="0" rect="0,0,20,20" defaultstyle="vendor-style"/>
    </annots>"#,
    );
    let original = strict(&xml).unwrap().document;
    let reread = strict(&original.to_xfdf_string().unwrap())
        .unwrap()
        .document;
    for (index, attributes) in [
        &["symbol"][..],
        &["highlight", "align"][..],
        &["head"][..],
        &["defaultstyle"][..],
    ]
    .iter()
    .enumerate()
    {
        for key in *attributes {
            assert_eq!(
                reread.annotations[index].base().extra.get(*key),
                original.annotations[index].base().extra.get(*key),
                "{key}"
            );
            assert!(
                original.annotations[index].base().extra.contains_key(*key),
                "{key}"
            );
        }
    }
}

#[test]
fn unsupported_style_children_are_preserved_instead_of_consumed_by_the_wrong_type() {
    let xml = document(
        r#"<annots><text page="0" rect="0,0,10,20"><defaultstyle vendor="x">color:red</defaultstyle><imagedata>not-a-stamp</imagedata></text></annots>"#,
    );
    let original = strict(&xml).unwrap().document;
    let serialized = original.to_xfdf_string().unwrap();
    assert!(serialized.contains("<defaultstyle"));
    assert!(serialized.contains("vendor=\"x\""));
    assert!(serialized.contains("not-a-stamp"));
    let reread = strict(&serialized).unwrap().document;
    assert_eq!(
        original.annotations[0].base().extra,
        reread.annotations[0].base().extra
    );
}

#[test]
fn utf8_bom_and_a_single_xml_declaration_are_supported() {
    for prefix in ["", "\u{feff}"] {
        let xml = format!(
            "{prefix}<?xml version=\"1.0\" encoding=\"UTF-8\"?>{}",
            document("<fields/>")
        );
        assert!(strict(&xml).is_ok());
    }
    for xml in [
        format!(" <?xml version=\"1.0\"?>{}", document("")),
        format!(
            "<?xml version=\"1.0\"?><?xml version=\"1.0\"?>{}",
            document("")
        ),
        document("<?xml version=\"1.0\"?>"),
    ] {
        assert!(XfdfDocument::parse(&xml).is_err());
    }
}

#[test]
fn malformed_xml_names_expanded_duplicate_attributes_and_comments_are_rejected() {
    for body in [
        r#"<annots><text page="0" rect="0,0,10,20" 1invalid="x"/></annots>"#,
        r#"<annots><1invalid/></annots>"#,
        r#"<annots><vendor::item xmlns:vendor="urn:vendor"/></annots>"#,
        r#"<annots><text xmlns:a="urn:vendor" xmlns:b="urn:vendor" a:id="1" b:id="2"/></annots>"#,
        "<!--bad--comment-->",
    ] {
        assert!(XfdfDocument::parse(&document(body)).is_err(), "{body}");
    }
    let mut doc = XfdfDocument::default();
    doc.metadata.insert("1invalid".into(), "x".into());
    assert!(doc.to_xfdf_string().is_err());
    doc.metadata.clear();
    doc.metadata.insert("供应商".into(), "有效名称".into());
    assert!(strict(&doc.to_xfdf_string().unwrap()).is_ok());
}

#[test]
fn all_internal_destination_modes_round_trip_through_real_pdf_objects() {
    let destinations = [
        r#"<XYZ Page="1" Left="12.5" Top="null" Zoom="1.25"/>"#,
        r#"<Fit Page="1"/>"#,
        r#"<FitH Page="1"/>"#,
        r#"<FitV Page="1" Left="20.5"/>"#,
        r#"<FitR Page="1" Left="1" Bottom="2" Right="3" Top="4"/>"#,
        r#"<FitB Page="1"/>"#,
        r#"<FitBH Page="1" Top="null"/>"#,
        r#"<FitBV Page="1" Left="50"/>"#,
    ];
    let mut annotations = String::new();
    for (index, destination) in destinations.iter().enumerate() {
        annotations.push_str(&format!(r#"<link name="mode-{index}" page="0" rect="0,0,10,20"><Dest>{destination}</Dest></link>"#));
    }
    annotations.push_str(r#"<caret page="1" rect="0,0,20,20"/>"#);
    let original = strict(&document(&format!("<annots>{annotations}</annots>")))
        .unwrap()
        .document;
    let output = NamedTempFile::new().unwrap();
    PdfAnnotationExporter::new()
        .export_to_new_pdf(&original, output.path())
        .unwrap();
    let loaded = load_annotations_from_pdf(output.path()).unwrap();
    assert_eq!(loaded.annotations.len(), original.annotations.len());
    let pdf = lopdf::Document::load(output.path()).unwrap();
    let target_page_id = pdf.get_pages()[&2];
    for (index, (before, after)) in original
        .annotations
        .iter()
        .zip(&loaded.annotations)
        .take(8)
        .enumerate()
    {
        match (before, after) {
            (Annotation::Link(before), Annotation::Link(after)) => {
                assert_eq!(before.target, after.target)
            }
            _ => panic!("expected link"),
        }
        let dict = pdf
            .objects
            .values()
            .filter_map(|o| o.as_dict().ok())
            .find(|d| {
                d.get(b"NM").ok().and_then(|o| o.as_str().ok())
                    == Some(format!("mode-{index}").as_bytes())
            })
            .unwrap();
        assert_eq!(
            dict.get(b"Dest").unwrap().as_array().unwrap()[0]
                .as_reference()
                .unwrap(),
            target_page_id
        );
    }
}

#[test]
fn invalid_programmatic_geometry_is_rejected_before_overwriting_output() {
    for (opacity, right) in [
        (f32::NAN, 20.0),
        (2.0, 20.0),
        (1.0, f64::INFINITY),
        (1.0, f64::MAX),
    ] {
        let mut doc = strict(&document(
            r#"<annots><caret page="0" rect="0,0,20,20"/></annots>"#,
        ))
        .unwrap()
        .document;
        if let Annotation::Caret(caret) = &mut doc.annotations[0] {
            caret.base.opacity = opacity;
            caret.base.rect.as_mut().unwrap().right = right;
        }
        let output = NamedTempFile::new().unwrap();
        fs::write(output.path(), b"unchanged").unwrap();
        assert!(PdfAnnotationExporter::new()
            .export_to_new_pdf(&doc, output.path())
            .is_err());
        assert_eq!(fs::read(output.path()).unwrap(), b"unchanged");
    }
    assert!(Rect::from_string("0,0,1e300,20").is_none());
    assert!(Color::from_hex("##123456").is_none());
}

#[test]
fn named_destinations_have_an_explicit_resolution_limit_warning() {
    let doc = strict(&document(r#"<annots><link page="0" rect="0,0,10,20"><Dest><Named Name="chapter-one"/></Dest></link></annots>"#)).unwrap().document;
    assert!(doc
        .pdf_conversion_warnings()
        .iter()
        .any(|w| w.contains("命名目的地")));
}

#[test]
fn annotation_attribute_namespaces_inherited_from_annots_survive_round_trip() {
    let original = strict(&document(
        r#"<annots xmlns:vendor="urn:vendor"><text page="0" rect="0,0,20,20" vendor:id="value"/></annots>"#,
    )).unwrap().document;
    let serialized = original.to_xfdf_string().unwrap();
    let reread = strict(&serialized).unwrap().document;
    assert_eq!(reread.annotations[0].base().extra["vendor:id"], "value");
    assert_eq!(
        reread.annotations[0].base().extra["xmlns:vendor"],
        "urn:vendor"
    );
}

#[test]
fn pdf_goto_actions_and_link_quadpoints_are_read_without_executing_actions() {
    let original = strict(&document(r#"<annots>
        <link page="0" rect="0,0,20,20" coords="0,20,20,20,0,0,20,0"><Dest><Fit Page="1"/></Dest></link>
        <caret page="1" rect="0,0,20,20"/>
    </annots>"#)).unwrap().document;
    let output = NamedTempFile::new().unwrap();
    PdfAnnotationExporter::new()
        .export_to_new_pdf(&original, output.path())
        .unwrap();
    let mut pdf = lopdf::Document::load(output.path()).unwrap();
    let annotation_id = pdf
        .objects
        .iter()
        .find_map(|(object_id, object)| {
            let dict = object.as_dict().ok()?;
            (dict.get(b"Subtype").ok()?.as_name().ok()? == b"Link").then_some(*object_id)
        })
        .unwrap();
    let dict = pdf
        .get_object_mut(annotation_id)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    let destination = dict.remove(b"Dest").unwrap();
    let mut action = lopdf::Dictionary::new();
    action.set("S", lopdf::Object::Name(b"GoTo".to_vec()));
    action.set("D", destination);
    dict.set("A", action);
    pdf.save(output.path()).unwrap();
    let reread = load_annotations_from_pdf(output.path()).unwrap();
    match (&original.annotations[0], &reread.annotations[0]) {
        (Annotation::Link(before), Annotation::Link(after)) => {
            assert_eq!(before.target, after.target);
            assert_eq!(before.coords, after.coords);
        }
        _ => panic!("expected link"),
    }
    let dict = pdf
        .get_object_mut(annotation_id)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    let mut unsupported = lopdf::Dictionary::new();
    unsupported.set("S", lopdf::Object::Name(b"JavaScript".to_vec()));
    unsupported.set("JS", lopdf::Object::string_literal("throw 'must not run'"));
    dict.set("A", unsupported);
    pdf.save(output.path()).unwrap();
    let reread = load_annotations_from_pdf(output.path()).unwrap();
    assert_eq!(reread.annotations.len(), 1);
    assert!(matches!(reread.annotations[0], Annotation::Caret(_)));
}
