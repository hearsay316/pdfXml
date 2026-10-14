use super::*;

fn invalid(message: impl Into<String>) -> PdfXmlError {
    PdfXmlError::PdfProcessing(message.into())
}

impl PdfAnnotationExporter {
    pub(super) fn border_width(annotation: &Annotation) -> Option<f32> {
        match annotation {
            Annotation::FreeText(a) => Some(a.width),
            Annotation::Square(a) => Some(a.width),
            Annotation::Circle(a) => Some(a.width),
            Annotation::Line(a) => Some(a.width),
            Annotation::Polygon(a) => Some(a.width),
            Annotation::Ink(a) => Some(a.width),
            Annotation::Link(a) => Some(a.border_width),
            _ => None,
        }
    }

    pub(super) fn validate_advanced_fields(annotation: &Annotation) -> Result<()> {
        let base = annotation.base();
        if base.intent.as_ref().is_some_and(|i| i.is_empty() || i.chars().any(|c| c.is_whitespace() || c.is_control())) {
            return Err(invalid("Intent 必须是非空名称，不含空白/控制字符"));
        }
        let width = Self::border_width(annotation);
        if width.is_some_and(|w| !w.is_finite() || w < 0.0) {
            return Err(invalid("边框宽度必须是非负有限数"));
        }
        if width.is_none() && (base.border_style.is_some() || base.dashes.is_some()) {
            return Err(invalid("当前注释类型不支持 BS 边框"));
        }
        if let Some(dashes) = &base.dashes {
            if dashes.iter().any(|n| !n.is_finite() || *n < 0.0)
                || (!dashes.is_empty() && !dashes.iter().any(|n| *n > 0.0)) {
                return Err(invalid("虚线长度必须非负有限，非空数组至少包含一个正数"));
            }
        }
        if let Annotation::FreeText(text) = annotation {
            if !(0..=2).contains(&text.align) {
                return Err(invalid("FreeText align 必须是 0、1 或 2"));
            }
            if let Some(fringe) = &text.fringe {
                if [fringe.left, fringe.bottom, fringe.right, fringe.top].iter()
                    .any(|v| !v.is_finite() || *v < 0.0 || !(*v as f32).is_finite()) {
                    return Err(invalid("FreeText fringe 必须是四个非负有限数"));
                }
            }
            if text.callout.as_ref().is_some_and(|c| CalloutLine::from_coordinates(&c.coordinates()).is_none()) {
                return Err(invalid("FreeText callout 坐标必须是有限数"));
            }
        }
        Ok(())
    }

    fn effective_border_style(base: &AnnotationBase) -> BorderStyle {
        base.border_style.unwrap_or_else(|| {
            if base.dashes.is_some() { BorderStyle::Dashed } else { BorderStyle::Solid }
        })
    }

    pub(super) fn border_dash_commands(base: &AnnotationBase) -> String {
        let dashes = if Self::effective_border_style(base) == BorderStyle::Dashed {
            base.dashes.as_deref().unwrap_or(&[3.0])
        } else {
            &[]
        };
        format!("[{}] 0 d\n", dashes.iter().map(f32::to_string).collect::<Vec<_>>().join(" "))
    }

    pub(super) fn apply_border_fields(dict: &mut lopdf::Dictionary, base: &AnnotationBase, width: f32) {
        let style = Self::effective_border_style(base);
        let mut border = lopdf::Dictionary::new();
        border.set("W", lopdf::Object::Real(width));
        border.set("S", lopdf::Object::Name(style.pdf_name().as_bytes().to_vec()));
        if let Some(dashes) = &base.dashes {
            border.set("D", lopdf::Object::Array(dashes.iter().map(|n| lopdf::Object::Real(*n)).collect()));
        }
        dict.set("BS", border);
        if matches!(style, BorderStyle::Bevelled | BorderStyle::Inset | BorderStyle::Underline) {
            dict.remove(b"AP");
        }
    }

    pub(super) fn apply_shape_appearance_properties(
        document: &mut lopdf::Document,
        dict: &lopdf::Dictionary,
        base: &AnnotationBase,
    ) -> Result<()> {
        let Ok(appearance) = dict.get(b"AP") else { return Ok(()); };
        let id = appearance.as_dict()?.get(b"N")?.as_reference()?;
        let stream = document.get_object_mut(id)?.as_stream_mut()?;
        let mut content = format!("q\n/BorderOpacity gs\n{}", Self::border_dash_commands(base)).into_bytes();
        content.extend_from_slice(&stream.content);
        content.extend_from_slice(b"\nQ\n");
        stream.set_content(content);
        let mut state = lopdf::Dictionary::new();
        state.set("Type", lopdf::Object::Name(b"ExtGState".to_vec()));
        state.set("CA", lopdf::Object::Real(base.opacity));
        state.set("ca", lopdf::Object::Real(base.opacity));
        let mut states = lopdf::Dictionary::new();
        states.set("BorderOpacity", state);
        let resources = stream.dict.get_mut(b"Resources")?.as_dict_mut()?;
        resources.set("ExtGState", states);
        Ok(())
    }

    pub(super) fn read_border_fields(
        document: &lopdf::Document,
        dict: &lopdf::Dictionary,
    ) -> Result<(Option<BorderStyle>, Option<Vec<f32>>)> {
        let (style, values) = if let Ok(bs) = dict.get(b"BS") {
            let bs = Self::resolve_object(document, bs)?.as_dict()?;
            let name = Self::dict_name_or_string(document, bs, b"S")?.unwrap_or_else(|| "S".into());
            let style = BorderStyle::from_name(&name).ok_or_else(|| invalid(format!("不支持的 PDF BS/S: {name}")))?;
            (Some(style), Self::dict_number_list(document, bs, b"D")?)
        } else if let Ok(border) = dict.get(b"Border") {
            let border = Self::resolve_object(document, border)?.as_array()?;
            let values = match border.get(3) {
                Some(value) => Some(Self::resolve_object(document, value)?.as_array()?.iter()
                    .map(|v| Self::object_number(Self::resolve_object(document, v)?)
                        .ok_or_else(|| invalid("Border 虚线参数不是数字")))
                    .collect::<Result<Vec<_>>>()?),
                None => None,
            };
            (Some(if values.is_some() { BorderStyle::Dashed } else { BorderStyle::Solid }), values)
        } else {
            (None, None)
        };
        let dashes = values.map(|values| {
            if values.iter().any(|n| !n.is_finite() || *n < 0.0 || !(*n as f32).is_finite())
                || (!values.is_empty() && !values.iter().any(|n| *n > 0.0)) {
                return Err(invalid("无效 PDF 虚线数组"));
            }
            Ok(values.iter().map(|n| *n as f32).collect())
        }).transpose()?;
        Ok((style, dashes))
    }

    pub(super) fn build_freetext_appearance(
        document: &mut lopdf::Document,
        dict: &mut lopdf::Dictionary,
        annotation: &FreeTextAnnotation,
        spec: Option<&FreeTextRenderSpec>,
    ) -> Result<Option<lopdf::Object>> {
        let rect = annotation.base.rect.clone().unwrap_or(Rect {
            left: 100.0, bottom: 700.0, right: 200.0, top: 720.0,
        });
        let bounds = Rect {
            left: rect.left.min(rect.right), bottom: rect.bottom.min(rect.top),
            right: rect.left.max(rect.right), top: rect.bottom.max(rect.top),
        };
        let (left, bottom, right, top) = annotation.fringe.as_ref()
            .map(|r| (r.left, r.bottom, r.right, r.top)).unwrap_or_default();
        let text_rect = Rect {
            left: bounds.left + left, bottom: bounds.bottom + bottom,
            right: bounds.right - right, top: bounds.top - top,
        };
        if text_rect.right <= text_rect.left || text_rect.top <= text_rect.bottom {
            return Err(invalid("FreeText fringe 未留下有效文字区域"));
        }
        let mut outer = bounds;
        if let Some(callout) = &annotation.callout {
            let margin = if annotation.head_style == LineEndingStyle::None {
                annotation.width as f64 / 2.0
            } else {
                (annotation.width as f64 * 4.0).max(6.0) + annotation.width as f64
            };
            for point in callout.coordinates().as_chunks::<2>().0 {
                outer.left = outer.left.min(point[0] - margin);
                outer.bottom = outer.bottom.min(point[1] - margin);
                outer.right = outer.right.max(point[0] + margin);
                outer.top = outer.top.max(point[1] + margin);
            }
        }
        let width = (outer.right - outer.left) as f32;
        let height = (outer.top - outer.bottom) as f32;
        if !width.is_finite() || !height.is_finite()
            || [outer.left, outer.bottom, outer.right, outer.top].iter().any(|n| !(*n as f32).is_finite()) {
            return Err(invalid("FreeText 外观范围超出 PDF 数值范围"));
        }
        dict.set("Rect", lopdf::Object::Array(outer.to_pdf_array()));
        dict.set("RD", lopdf::Object::Array(Rect {
            left: text_rect.left - outer.left, bottom: text_rect.bottom - outer.bottom,
            right: outer.right - text_rect.right, top: outer.top - text_rect.top,
        }.to_pdf_array()));
        let x = (text_rect.left - outer.left) as f32;
        let y = (text_rect.bottom - outer.bottom) as f32;
        let text_width = (text_rect.right - text_rect.left) as f32;
        let text_height = (text_rect.top - text_rect.bottom) as f32;
        let stroke = spec.map(|s| extract_fill_color_from_da(&s.da))
            .or_else(|| annotation.base.color.as_deref().and_then(Color::from_hex).map(|c| (c.r, c.g, c.b)))
            .unwrap_or((0.0, 0.0, 0.0));
        let mut content = format!("q\n/Opacity gs\n{} {} {} RG\n{} {} {} rg\n{} w\n{}",
            stroke.0, stroke.1, stroke.2, stroke.0, stroke.1, stroke.2,
            annotation.width, Self::border_dash_commands(&annotation.base));
        if annotation.width > 0.0 {
            if let Some(callout) = &annotation.callout {
                let coords = callout.coordinates();
                for (index, point) in coords.as_chunks::<2>().0.iter().enumerate() {
                    content.push_str(&format!("{} {} {}\n", point[0] - outer.left,
                        point[1] - outer.bottom, if index == 0 { "m" } else { "l" }));
                }
                content.push_str("S\n[] 0 d\n");
                if let Some(path) = Self::build_arrowhead_path(
                    (coords[0] - outer.left) as f32, (coords[1] - outer.bottom) as f32,
                    (coords[2] - outer.left) as f32, (coords[3] - outer.bottom) as f32,
                    annotation.width * 4.0, annotation.head_style.name()) {
                    content.push_str(&path);
                }
                content.push_str(&Self::border_dash_commands(&annotation.base));
            }
            let inset = annotation.width / 2.0;
            content.push_str(&format!("{} {} {} {} re\nS\n", x + inset, y + inset,
                (text_width - annotation.width).max(0.0), (text_height - annotation.width).max(0.0)));
        }
        let mut resources = lopdf::Dictionary::new();
        let mut opacity = lopdf::Dictionary::new();
        opacity.set("Type", lopdf::Object::Name(b"ExtGState".to_vec()));
        opacity.set("CA", lopdf::Object::Real(annotation.base.opacity));
        opacity.set("ca", lopdf::Object::Real(annotation.base.opacity));
        let mut states = lopdf::Dictionary::new();
        states.set("Opacity", opacity);
        resources.set("ExtGState", states);
        let padding = annotation.width;
        let inner_width = text_width - padding * 2.0;
        let inner_height = text_height - padding * 2.0;
        if let Some(spec) = spec.filter(|_| inner_width > 0.0 && inner_height > 0.0) {
            let local_rect = Rect { left: 0.0, bottom: 0.0, right: inner_width as f64, top: inner_height as f64 };
            let text = if spec.is_cjk { Self::build_cjk_ap_stream(spec, Some(&local_rect))? }
                else { Self::build_text_ap_stream(spec, Some(&local_rect)) };
            let mut xobjects = lopdf::Dictionary::new();
            xobjects.set("Text", lopdf::Object::Reference(document.add_object(text)));
            resources.set("XObject", xobjects);
            content.push_str(&format!("q\n{} {} {} {} re\nW n\n1 0 0 1 {} {} cm\n/Text Do\nQ\n",
                x + padding, y + padding, inner_width, inner_height, x + padding, y + padding));
        }
        content.push_str("Q\n");
        let mut stream = lopdf::Dictionary::new();
        stream.set("Type", lopdf::Object::Name(b"XObject".to_vec()));
        stream.set("Subtype", lopdf::Object::Name(b"Form".to_vec()));
        stream.set("FormType", 1);
        stream.set("BBox", lopdf::Object::Array(vec![0.into(), 0.into(), width.into(), height.into()]));
        stream.set("Resources", resources);
        Ok(Some(lopdf::Object::Stream(lopdf::Stream::new(stream, content.into_bytes()))))
    }
}
