use pdfxml::{
    load_annotations_from_pdf, Annotation, BorderStyle, CalloutLine, LineEndingStyle,
    PdfAnnotationExporter, XfdfDocument, XfdfParseMode, XfdfParseOptions, XFDF_NAMESPACE,
};
use std::fs;
use tempfile::NamedTempFile;

fn xml(annots: &str) -> String {
    format!(r#"<xfdf xmlns="{XFDF_NAMESPACE}"><annots>{annots}</annots></xfdf>"#)
}

fn strict(annots: &str) -> XfdfDocument {
    XfdfDocument::parse_with_options(&xml(annots), &XfdfParseOptions {
        mode: XfdfParseMode::Strict, ..Default::default()
    }).unwrap().document
}

fn exported(doc: &XfdfDocument) -> (NamedTempFile, lopdf::Document) {
    let file = NamedTempFile::new().unwrap();
    PdfAnnotationExporter::new().export_to_new_pdf(doc, file.path()).unwrap();
    let pdf = lopdf::Document::load(file.path()).unwrap();
    (file, pdf)
}

fn ids(pdf: &lopdf::Document) -> Vec<lopdf::ObjectId> {
    pdf.get_object(pdf.get_pages()[&1]).unwrap().as_dict().unwrap()
        .get(b"Annots").unwrap().as_array().unwrap().iter()
        .map(|a| a.as_reference().unwrap()).collect()
}

fn appearance<'a>(pdf: &'a lopdf::Document, dict: &lopdf::Dictionary) -> &'a lopdf::Stream {
    let id = dict.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().unwrap();
    pdf.get_object(id).unwrap().as_stream().unwrap()
}

fn plain(stream: &lopdf::Stream) -> Vec<u8> {
    stream.decompressed_content().unwrap_or_else(|_| stream.content.clone())
}

fn operations(stream: &lopdf::Stream) -> Vec<lopdf::content::Operation> {
    lopdf::content::Content::decode(&plain(stream)).unwrap().operations
}

fn text(doc: &XfdfDocument) -> &pdfxml::FreeTextAnnotation {
    match &doc.annotations[0] { Annotation::FreeText(a) => a, _ => panic!("expected FreeText") }
}

#[test]
fn intent_standard_and_legacy_names_round_trip_without_typewriter_override() {
    for (tag, intent, additional) in [
        ("freetext", "FreeText", ""),
        ("freetext", "FreeTextTypeWriter", ""),
        ("line", "LineDimension", r#"start="1,1" end="20,20""#),
        ("polygon", "PolygonCloud", r#"vertices="1,1 20,1 10,20""#),
        ("polyline", "PolyLineDimension", r#"vertices="1,1 20,20""#),
        ("ink", "InkHighlight", ""),
        ("stamp", "StampImage", ""),
        ("highlight", "Highlight", ""),
    ] {
        for attribute in ["intent", "IT"] {
            let doc = strict(&format!(r#"<{tag} page="0" rect="0,0,120,50" {attribute}="{intent}" {additional}><contents>Text</contents></{tag}>"#));
            assert_eq!(doc.annotations[0].base().intent.as_deref(), Some(intent));
            assert!(!doc.annotations[0].base().extra.contains_key(attribute));
            let (file, pdf) = exported(&doc);
            let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
            assert_eq!(dict.get(b"IT").unwrap().as_name().unwrap(), intent.as_bytes());
            let loaded = load_annotations_from_pdf(file.path()).unwrap();
            assert_eq!(loaded.annotations[0].base().intent.as_deref(), Some(intent));
            let output = loaded.to_xfdf_string().unwrap();
            assert!(output.contains(&format!(r#"intent="{intent}""#)));
            assert!(!output.contains(" IT="));
        }
    }
}

#[test]
fn callout_coordinates_are_typed_bounded_and_keep_the_optional_knee() {
    for values in ["1,2,3,4", "1,2,3,4,5,6"] {
        let callout = CalloutLine::from_string(values).unwrap();
        assert_eq!(callout.knee.is_some(), values.split(',').count() == 6);
        assert_eq!(CalloutLine::from_coordinates(&callout.coordinates()), Some(callout));
    }
    for values in ["", "1,2", "1,2,3,4,5", "1,2,3,4,5,6,7,8", "NaN,2,3,4", "1,2,inf,4", "1,2,1e300,4", "1,2,,4"] {
        assert!(CalloutLine::from_string(values).is_none(), "{values}");
    }
}

#[test]
fn callout_has_real_dictionary_geometry_text_resources_and_opacity() {
    let doc = strict(r##"<freetext page="0" rect="0,0,300,200" intent="FreeTextCallout" callout="20,20,90,65,140,100" fringe="140,70,10,20" head="ClosedArrow" width="2" style="dash" dashes="5,2" opacity="0.4" TextColor="#123456"><contents>Callout text</contents><defaultappearance>/Courier 14 Tf</defaultappearance></freetext>"##);
    let (file, pdf) = exported(&doc);
    let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
    assert_eq!(dict.get(b"CL").unwrap().as_array().unwrap().len(), 6);
    assert_eq!(dict.get(b"LE").unwrap().as_name().unwrap(), b"ClosedArrow");
    assert_eq!(dict.get(b"IT").unwrap().as_name().unwrap(), b"FreeTextCallout");
    let stream = appearance(&pdf, dict);
    let ops = operations(stream);
    assert!(ops.iter().any(|o| o.operator == "d" && o.operands[0].as_array().unwrap().len() == 2));
    assert!(ops.iter().any(|o| o.operator == "B"));
    assert!(ops.iter().any(|o| o.operator == "Do"));
    assert!(ops.iter().any(|o| o.operator == "cm" && o.operands[4].as_float().unwrap() == 142.0 && o.operands[5].as_float().unwrap() == 72.0));
    let resources = stream.dict.get(b"Resources").unwrap().as_dict().unwrap();
    let state = resources.get(b"ExtGState").unwrap().as_dict().unwrap().get(b"Opacity").unwrap().as_dict().unwrap();
    assert_eq!(state.get(b"CA").unwrap().as_float().unwrap(), 0.4);
    assert_eq!(state.get(b"ca").unwrap().as_float().unwrap(), 0.4);
    let inner = resources.get(b"XObject").unwrap().as_dict().unwrap().get(b"Text").unwrap().as_reference().unwrap();
    let inner = pdf.get_object(inner).unwrap().as_stream().unwrap();
    assert!(String::from_utf8_lossy(&plain(inner)).contains("/Courier 14 Tf"));
    let read = load_annotations_from_pdf(file.path()).unwrap();
    assert_eq!(text(&read).callout, text(&doc).callout);
    assert_eq!(text(&read).head_style, LineEndingStyle::ClosedArrow);
    assert_eq!(text(&read).base.dashes, Some(vec![5.0, 2.0]));
    let fringe = text(&read).fringe.as_ref().unwrap();
    assert_eq!([fringe.left, fringe.bottom, fringe.right, fringe.top], [140.0, 70.0, 10.0, 20.0]);
    assert!(XfdfDocument::parse_with_options(&read.to_xfdf_string().unwrap(), &XfdfParseOptions {
        mode: XfdfParseMode::Strict, ..Default::default()
    }).is_ok());
}

#[test]
fn outlying_callout_expands_bounds_without_moving_or_scaling_the_text_box() {
    let doc = strict(r#"<freetext page="0" rect="100,100,200,160" callout="20,40,100,120" head="OpenArrow" width="2"><contents>Outside</contents></freetext>"#);
    let before = text(&doc).base.rect.clone().unwrap();
    let (file, pdf) = exported(&doc);
    let read = load_annotations_from_pdf(file.path()).unwrap();
    let rect = text(&read).base.rect.as_ref().unwrap();
    let fringe = text(&read).fringe.as_ref().unwrap();
    assert!(rect.left < 20.0 && rect.bottom < 40.0);
    assert_eq!(rect.left + fringe.left, before.left);
    assert_eq!(rect.bottom + fringe.bottom, before.bottom);
    assert_eq!(rect.right - fringe.right, before.right);
    assert_eq!(rect.top - fringe.top, before.top);
    let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
    let bbox = appearance(&pdf, dict).dict.get(b"BBox").unwrap().as_array().unwrap();
    assert_eq!(bbox[2].as_float().unwrap() as f64, rect.right - rect.left);
    assert_eq!(text(&doc).base.rect.as_ref().unwrap().left, before.left);
    let (second_file, _) = exported(&read);
    let second = load_annotations_from_pdf(second_file.path()).unwrap();
    assert_eq!(text(&second).base.rect.as_ref().unwrap().left, rect.left);
}

#[test]
fn callout_infers_intent_and_default_width_but_preserves_explicit_zero() {
    for (width, expected) in [("", 1.0), (r#"width="0""#, 0.0)] {
        let doc = strict(&format!(r#"<freetext page="0" rect="0,0,100,50" callout="10,10,90,30" {width}/>"#));
        assert_eq!(text(&doc).base.intent.as_deref(), Some("FreeTextCallout"));
        assert_eq!(text(&doc).width, expected);
        let serialized = doc.to_xfdf_string().unwrap();
        let read = XfdfDocument::parse(&serialized).unwrap();
        assert_eq!(text(&read).width, expected);
        let (_, pdf) = exported(&doc);
        let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
        assert!(dict.get(b"DA").is_ok());
        let ops = operations(appearance(&pdf, dict));
        assert_eq!(ops.iter().any(|o| o.operator == "S"), expected > 0.0);
    }
}

#[test]
fn contradictory_intent_has_a_diagnostic_and_is_not_silently_overwritten() {
    for annots in [
        r#"<freetext page="0" rect="0,0,100,50" intent="FreeTextTypeWriter" callout="10,10,90,30"/>"#,
        r#"<freetext page="0" rect="0,0,100,50" intent="FreeText" IT="FreeTextTypeWriter"/>"#,
    ] {
        let report = XfdfDocument::parse_with_report(&xml(annots)).unwrap();
        assert!(!report.diagnostics.is_empty());
        assert!(XfdfDocument::parse_with_options(&xml(annots), &XfdfParseOptions {
            mode: XfdfParseMode::Strict, ..Default::default()
        }).is_err());
        let (_, pdf) = exported(&report.document);
        let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
        assert_eq!(dict.get(b"IT").unwrap().as_name().unwrap(), text(&report.document).base.intent.as_ref().unwrap().as_bytes());
    }
    let doc = strict(r#"<freetext page="0" rect="0,0,100,50"><contents>Plain</contents></freetext>"#);
    let (_, pdf) = exported(&doc);
    assert!(pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap().get(b"IT").is_err());
}

#[test]
fn shape_dash_width_and_opacity_are_preserved_in_both_dictionary_and_appearance() {
    let doc = strict(r#"<square page="0" rect="0,0,100,50" width="0.25" style="dash" dashes="4,2" opacity="0.5"/>
        <circle page="0" rect="0,0,100,50" width="0.25" style="dash" dashes="4,2" opacity="0.5"/>
        <line page="0" rect="0,0,100,50" start="5,5" end="95,45" width="0.25" style="dash" dashes="4,2" opacity="0.5"/>
        <polygon page="0" rect="0,0,100,50" vertices="5,5 95,5 50,45" width="0.25" style="dash" dashes="4,2" opacity="0.5"/>"#);
    let (file, pdf) = exported(&doc);
    for id in ids(&pdf) {
        let dict = pdf.get_object(id).unwrap().as_dict().unwrap();
        let bs = dict.get(b"BS").unwrap().as_dict().unwrap();
        assert_eq!(bs.get(b"W").unwrap().as_float().unwrap(), 0.25);
        assert_eq!(bs.get(b"S").unwrap().as_name().unwrap(), b"D");
        let stream = appearance(&pdf, dict);
        let ops = operations(stream);
        assert!(ops.iter().any(|o| o.operator == "w" && o.operands[0].as_float().unwrap() == 0.25));
        if dict.get(b"Subtype").unwrap().as_name().unwrap() == b"Square" {
            let rectangle = ops.iter().find(|o| o.operator == "re").unwrap();
            let values: Vec<f32> = rectangle.operands.iter().map(|n| n.as_float().unwrap()).collect();
            assert_eq!(values, [0.125, 0.125, 99.75, 49.75]);
        }
        let dash = ops.iter().find(|o| o.operator == "d").unwrap();
        assert_eq!(dash.operands[0].as_array().unwrap().len(), 2);
        let resources = stream.dict.get(b"Resources").unwrap().as_dict().unwrap();
        let state = resources.get(b"ExtGState").unwrap().as_dict().unwrap().get(b"BorderOpacity").unwrap().as_dict().unwrap();
        assert_eq!(state.get(b"CA").unwrap().as_float().unwrap(), 0.5);
    }
    let read = load_annotations_from_pdf(file.path()).unwrap();
    for annotation in &read.annotations {
        assert_eq!(annotation.base().border_style, Some(BorderStyle::Dashed));
        assert_eq!(annotation.base().dashes, Some(vec![4.0, 2.0]));
    }
}

#[test]
fn zero_width_shapes_do_not_become_one_point_or_hairline_strokes() {
    let doc = strict(r#"<square page="0" rect="0,0,100,50" width="0"/>
        <circle page="0" rect="0,0,100,50" width="0"/>
        <line page="0" rect="0,0,100,50" start="5,5" end="95,45" head="OpenArrow" width="0"/>
        <polygon page="0" rect="0,0,100,50" vertices="5,5 95,5 50,45" width="0"/>"#);
    let (_, pdf) = exported(&doc);
    for id in ids(&pdf) {
        let dict = pdf.get_object(id).unwrap().as_dict().unwrap();
        assert_eq!(dict.get(b"BS").unwrap().as_dict().unwrap().get(b"W").unwrap().as_float().unwrap(), 0.0);
        assert!(!operations(appearance(&pdf, dict)).iter().any(|o| matches!(o.operator.as_str(), "S" | "s" | "B" | "b")));
    }
}

#[test]
fn all_border_styles_round_trip_without_claiming_unimplemented_appearances() {
    for (style, name) in [("solid", "S"), ("dash", "D"), ("bevelled", "B"), ("inset", "I"), ("underline", "U")] {
        let doc = strict(&format!(r#"<square page="0" rect="0,0,100,50" width="2" style="{style}"/>"#));
        let (file, pdf) = exported(&doc);
        let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
        assert_eq!(dict.get(b"BS").unwrap().as_dict().unwrap().get(b"S").unwrap().as_name().unwrap(), name.as_bytes());
        assert_eq!(dict.get(b"AP").is_ok(), matches!(name, "S" | "D"));
        let read = load_annotations_from_pdf(file.path()).unwrap();
        assert_eq!(read.annotations[0].base().border_style, doc.annotations[0].base().border_style);
        if !matches!(name, "S" | "D") { assert!(!doc.pdf_conversion_warnings().is_empty()); }
    }
}

#[test]
fn malformed_advanced_numbers_are_diagnosed_and_unknown_styles_survive() {
    for attribute in [r#"callout="1,2,3""#, r#"callout="1,2,NaN,4""#, r#"fringe="-1,0,0,0""#,
        r#"width="inf""#, r#"dashes="0,0""#, r#"dashes="2,-1""#, r#"dashes="NaN""#, r#"head="Invalid""#, r#"intent="bad intent""#] {
        let value = xml(&format!(r#"<freetext page="0" rect="0,0,100,50" {attribute}/>"#));
        let report = XfdfDocument::parse_with_report(&value).unwrap();
        assert!(!report.diagnostics.is_empty(), "{attribute}");
        assert!(XfdfDocument::parse_with_options(&value, &XfdfParseOptions {
            mode: XfdfParseMode::Strict, ..Default::default()
        }).is_err(), "{attribute}");
    }
    for style in ["cloudy", "vendor-border", "font: bold 14px Arial;"] {
        let doc = strict(&format!(r#"<freetext page="0" rect="0,0,100,50" style="{style}"/>"#));
        assert_eq!(text(&doc).base.extra["style"], style);
        let output = doc.to_xfdf_string().unwrap();
        assert_eq!(text(&XfdfDocument::parse(&output).unwrap()).base.extra["style"], style);
        assert!(!doc.pdf_conversion_warnings().is_empty());
    }
}

#[test]
fn dash_defaults_empty_patterns_and_explicit_solid_are_consistent() {
    for (attrs, expected_style, expected_dash_length) in [
        (r#"dashes="4,2""#, "D", 2),
        (r#"style="dash""#, "D", 1),
        (r#"style="dash" dashes="""#, "D", 0),
        (r#"style="solid" dashes="4,2""#, "S", 0),
        (r#"border-style="dash" dashes="4,2""#, "D", 2),
    ] {
        let doc = strict(&format!(r#"<square page="0" rect="0,0,100,50" {attrs}/>"#));
        let (_, pdf) = exported(&doc);
        let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
        assert_eq!(dict.get(b"BS").unwrap().as_dict().unwrap().get(b"S").unwrap().as_name().unwrap(), expected_style.as_bytes());
        let ops = operations(appearance(&pdf, dict));
        assert_eq!(ops.iter().find(|o| o.operator == "d").unwrap().operands[0].as_array().unwrap().len(), expected_dash_length);
    }
}

#[test]
fn invalid_sdk_values_fail_before_overwriting_output_and_allow_reuse() {
    let original = strict(r#"<freetext page="0" rect="0,0,100,50" callout="10,10,90,30"/>"#);
    let mut exporter = PdfAnnotationExporter::new();
    for case in 0..8 {
        let mut doc = original.clone();
        if let Annotation::FreeText(a) = &mut doc.annotations[0] {
            match case {
                0 => a.width = f32::NAN,
                1 => a.width = -1.0,
                2 => a.callout.as_mut().unwrap().start[0] = f64::INFINITY,
                3 => a.base.dashes = Some(vec![0.0, 0.0]),
                4 => a.base.dashes = Some(vec![1.0, f32::NAN]),
                5 => a.base.intent = Some("bad intent".into()),
                6 => a.align = 3,
                7 => a.fringe = Some(pdfxml::Rect { left: 100.0, bottom: 0.0, right: 1.0, top: 0.0 }),
                _ => unreachable!(),
            }
        }
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), b"sentinel").unwrap();
        assert!(exporter.export_to_new_pdf(&doc, file.path()).is_err(), "{case}");
        assert_eq!(fs::read(file.path()).unwrap(), b"sentinel");
        exporter.export_to_new_pdf(&original, file.path()).unwrap();
    }
}

#[test]
fn every_callout_line_ending_has_a_parseable_appearance_and_round_trip() {
    for name in ["None", "Square", "Circle", "Diamond", "OpenArrow", "ClosedArrow", "Butt", "ROpenArrow", "RClosedArrow", "Slash"] {
        let doc = strict(&format!(r#"<freetext page="0" rect="0,0,100,50" callout="20,20,80,30" head="{name}"/>"#));
        let (file, pdf) = exported(&doc);
        let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
        assert_eq!(dict.get(b"LE").unwrap().as_name().unwrap(), name.as_bytes());
        assert!(!operations(appearance(&pdf, dict)).is_empty());
        let read = load_annotations_from_pdf(file.path()).unwrap();
        assert_eq!(text(&read).head_style.name(), name);
    }
}

#[test]
fn pdf_indirect_dash_numbers_and_legacy_border_patterns_are_read() {
    let doc = strict(r#"<link page="0" rect="0,0,100,50" width="2" uri="https://example.test"/>"#);
    for legacy in [false, true] {
        let (file, mut pdf) = exported(&doc);
        let id = ids(&pdf)[0];
        let four = pdf.add_object(4);
        let two = pdf.add_object(2);
        let dashes = vec![lopdf::Object::Reference(four), lopdf::Object::Reference(two)];
        let dict = pdf.get_object_mut(id).unwrap().as_dict_mut().unwrap();
        if legacy {
            dict.remove(b"BS");
            dict.set("Border", lopdf::Object::Array(vec![0.into(), 0.into(), 2.into(), lopdf::Object::Array(dashes)]));
        } else {
            let bs = dict.get_mut(b"BS").unwrap().as_dict_mut().unwrap();
            bs.set("S", lopdf::Object::Name(b"D".to_vec()));
            bs.set("D", lopdf::Object::Array(dashes));
        }
        pdf.save(file.path()).unwrap();
        let read = load_annotations_from_pdf(file.path()).unwrap();
        assert_eq!(read.annotations[0].base().border_style, Some(BorderStyle::Dashed));
        assert_eq!(read.annotations[0].base().dashes, Some(vec![4.0, 2.0]));
    }
}

#[test]
fn nonnumeric_pdf_callout_is_not_silently_shortened_to_a_valid_line() {
    let doc = strict(r#"<freetext page="0" rect="0,0,100,50" callout="20,20,80,30"/>
        <caret page="0" rect="120,0,140,20"/>"#);
    let (file, mut pdf) = exported(&doc);
    let id = ids(&pdf)[0];
    pdf.get_object_mut(id).unwrap().as_dict_mut().unwrap().set("CL", lopdf::Object::Array(vec![
        20.into(), 20.into(), 80.into(), 30.into(), lopdf::Object::string_literal("not-a-number")
    ]));
    pdf.save(file.path()).unwrap();
    let read = load_annotations_from_pdf(file.path()).unwrap();
    assert_eq!(read.annotations.len(), 1);
    assert!(matches!(read.annotations[0], Annotation::Caret(_)));
}

#[test]
fn empty_callout_contents_keep_geometry_without_a_text_xobject() {
    for contents in ["", "<contents/>"] {
        let doc = strict(&format!(r#"<freetext page="0" rect="0,0,100,50" callout="10,10,90,30" fringe="2,3,4,5" head="OpenArrow" width="1">{contents}</freetext>"#));
        let (file, pdf) = exported(&doc);
        let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
        assert!(dict.get(b"DA").is_ok());
        assert_eq!(dict.get(b"CL").unwrap().as_array().unwrap().len(), 4);
        let stream = appearance(&pdf, dict);
        assert!(!operations(stream).iter().any(|o| o.operator == "Do"));
        assert!(stream.dict.get(b"Resources").unwrap().as_dict().unwrap().get(b"XObject").is_err());
        let read = load_annotations_from_pdf(file.path()).unwrap();
        assert_eq!(text(&read).base.contents, text(&doc).base.contents);
        assert_eq!(text(&read).callout, text(&doc).callout);
    }
}

#[test]
fn cjk_callout_uses_glyph_outlines_or_reports_a_missing_font() {
    let doc = strict(r#"<freetext page="0" rect="0,0,300,100" callout="10,10,80,50" fringe="80,20,10,10" width="1"><contents>中文引出线</contents></freetext>"#);
    let file = NamedTempFile::new().unwrap();
    if let Err(error) = PdfAnnotationExporter::new().export_to_new_pdf(&doc, file.path()) {
        assert!(error.to_string().contains("未找到可用的中文字体文件"), "{error}");
        return;
    }
    let pdf = lopdf::Document::load(file.path()).unwrap();
    let dict = pdf.get_object(ids(&pdf)[0]).unwrap().as_dict().unwrap();
    let stream = appearance(&pdf, dict);
    let text_id = stream.dict.get(b"Resources").unwrap().as_dict().unwrap()
        .get(b"XObject").unwrap().as_dict().unwrap().get(b"Text").unwrap().as_reference().unwrap();
    let text_stream = pdf.get_object(text_id).unwrap().as_stream().unwrap();
    let ops = operations(text_stream);
    assert!(ops.iter().filter(|o| o.operator == "f").count() >= "中文引出线".chars().count());
    assert!(ops.iter().any(|o| o.operator == "c" || o.operator == "l"));
    let read = load_annotations_from_pdf(file.path()).unwrap();
    assert_eq!(text(&read).base.contents.as_deref(), Some("中文引出线"));
    assert_eq!(text(&read).callout, text(&doc).callout);
}

#[test]
fn advanced_annotations_merge_without_losing_existing_annotations_or_page_content() {
    let base = strict(r#"<caret name="existing" page="0" rect="10,10,30,30"/>"#);
    let (input, original) = exported(&base);
    let doc = strict(r#"<freetext name="added-callout" page="0" rect="100,100,200,160" callout="20,40,100,120" head="OpenArrow" width="2" style="dash" dashes="4,2"><contents>Merged callout</contents></freetext>
        <polygon name="added-polygon" page="1" rect="0,0,100,50" vertices="5,5 95,5 50,45" width="0.5" style="dash" dashes="5,3" intent="PolygonDimension"/>"#);
    let output = NamedTempFile::new().unwrap();
    PdfAnnotationExporter::new().export_to_existing_pdf(&doc, input.path(), output.path()).unwrap();
    let pdf = lopdf::Document::load(output.path()).unwrap();
    assert_eq!(pdf.get_pages().len(), 2);
    assert_eq!(original.get_page_content(original.get_pages()[&1]).unwrap(), pdf.get_page_content(pdf.get_pages()[&1]).unwrap());
    let page_zero_ids = ids(&pdf);
    assert_eq!(page_zero_ids.len(), 2);
    let callout = pdf.get_object(page_zero_ids[1]).unwrap().as_dict().unwrap();
    assert_eq!(callout.get(b"IT").unwrap().as_name().unwrap(), b"FreeTextCallout");
    assert!(operations(appearance(&pdf, callout)).iter().any(|o| o.operator == "Do"));
    let read = load_annotations_from_pdf(output.path()).unwrap();
    assert_eq!(read.annotations.len(), 3);
    assert_eq!(read.annotations[0].base().name.as_deref(), Some("existing"));
    assert_eq!(read.annotations[1].base().name.as_deref(), Some("added-callout"));
    assert_eq!(read.annotations[2].base().page, 1);
    assert_eq!(read.annotations[2].base().intent.as_deref(), Some("PolygonDimension"));
    assert_eq!(read.annotations[2].base().dashes, Some(vec![5.0, 3.0]));
    assert!(XfdfDocument::parse_with_options(&read.to_xfdf_string().unwrap(), &XfdfParseOptions {
        mode: XfdfParseMode::Strict, ..Default::default()
    }).is_ok());
}
