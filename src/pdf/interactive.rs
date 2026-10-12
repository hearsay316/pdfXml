//! Link targets and Caret appearances. Action data is never executed by the library.
use super::*;

fn processing(message: impl Into<String>) -> PdfXmlError {
    PdfXmlError::PdfProcessing(message.into())
}

impl PdfAnnotationExporter {
    pub(super) fn build_link_fields(
        dict: &mut lopdf::Dictionary,
        link: &LinkAnnotation,
    ) -> Result<()> {
        if !link.border_width.is_finite() || link.border_width < 0.0 {
            return Err(processing("Link border_width 必须是非负有限数"));
        }
        dict.set(
            "H",
            lopdf::Object::Name(link.highlight_mode.pdf_name().as_bytes().to_vec()),
        );
        dict.set(
            "Border",
            lopdf::Object::Array(vec![
                0.into(),
                0.into(),
                lopdf::Object::Real(link.border_width),
            ]),
        );
        if let Some(coords) = &link.coords {
            let points =
                Self::parse_quadpoints(coords).ok_or_else(|| processing("无效 Link QuadPoints"))?;
            dict.set("QuadPoints", points);
        }
        match &link.target {
            Some(LinkTarget::Uri { uri, is_map }) => {
                let mut action = lopdf::Dictionary::new();
                action.set("S", lopdf::Object::Name(b"URI".to_vec()));
                action.set("URI", lopdf::Object::string_literal(uri.clone()));
                action.set("IsMap", lopdf::Object::Boolean(*is_map));
                dict.set("A", action);
            }
            Some(LinkTarget::Named { name }) => {
                dict.set("Dest", pdf_text_string(name));
            }
            Some(LinkTarget::Page { view, .. }) => {
                crate::xfdf::validate_link_view(view)?;
                // Replaced with the actual page reference after the complete page tree is available.
                let mut destination = vec![
                    lopdf::Object::Null,
                    lopdf::Object::Name(view.mode.name().as_bytes().to_vec()),
                ];
                destination.extend(view.parameters.iter().map(|parameter| match parameter {
                    Some(value) => lopdf::Object::Real(*value as f32),
                    None => lopdf::Object::Null,
                }));
                dict.set("Dest", lopdf::Object::Array(destination));
            }
            None => {}
        }
        Ok(())
    }

    pub(super) fn resolve_pending_link_destinations(
        &mut self,
        document: &mut lopdf::Document,
    ) -> Result<()> {
        let pages = document.get_pages();
        for (annotation_id, target_page) in self.pending_link_destinations.drain(..) {
            let page_number = u32::try_from(target_page)
                .ok()
                .and_then(|p| p.checked_add(1))
                .ok_or_else(|| processing("Link 目标页码超出 PDF 范围"))?;
            let page_id = *pages.get(&page_number).ok_or_else(|| {
                processing(format!(
                    "Link 目标页 {} 不存在（零基页码）；不会写入失效的页面引用",
                    target_page
                ))
            })?;
            let dict = document.get_object_mut(annotation_id)?.as_dict_mut()?;
            let destination = dict.get_mut(b"Dest")?.as_array_mut()?;
            destination[0] = lopdf::Object::Reference(page_id);
        }
        Ok(())
    }

    pub(super) fn read_link(
        document: &lopdf::Document,
        dict: &lopdf::Dictionary,
        base: AnnotationBase,
    ) -> Result<LinkAnnotation> {
        if dict.has(b"A") && dict.has(b"Dest") {
            return Err(processing("Link 同时包含 A 与 Dest，拒绝猜测目标"));
        }
        let target = if let Ok(destination) = dict.get(b"Dest") {
            Some(Self::read_link_destination(document, destination)?)
        } else if let Ok(action) = dict.get(b"A") {
            let action = Self::resolve_object(document, action)?.as_dict()?;
            match Self::dict_name_or_string(document, action, b"S")?.as_deref() {
                Some("URI") => Some(LinkTarget::Uri {
                    uri: Self::dict_string(document, action, b"URI")?
                        .ok_or_else(|| processing("URI 动作缺少 URI"))?,
                    is_map: Self::dict_bool(document, action, b"IsMap")?.unwrap_or(false),
                }),
                Some("GoTo") => Some(Self::read_link_destination(document, action.get(b"D")?)?),
                other => {
                    return Err(processing(format!(
                        "暂不支持 Link 动作 {other:?}，不执行 JavaScript/Launch/远程动作"
                    )))
                }
            }
        } else {
            None
        };
        let highlight_mode = match Self::dict_name_or_string(document, dict, b"H")?
            .as_deref()
            .unwrap_or("I")
        {
            "N" => LinkHighlightMode::None,
            "I" => LinkHighlightMode::Invert,
            "O" => LinkHighlightMode::Outline,
            "P" => LinkHighlightMode::Push,
            other => return Err(processing(format!("未知 Link /H: {other}"))),
        };
        let border_width = Self::annotation_width(document, dict)?.unwrap_or(0.0);
        if !border_width.is_finite() || border_width < 0.0 {
            return Err(processing("无效 Link 边框宽度"));
        }
        Ok(LinkAnnotation {
            base,
            target,
            highlight_mode,
            border_width,
            coords: Self::dict_number_list(document, dict, b"QuadPoints")?
                .map(Self::numbers_to_csv),
        })
    }

    fn read_link_destination(
        document: &lopdf::Document,
        object: &lopdf::Object,
    ) -> Result<LinkTarget> {
        let destination = Self::resolve_object(document, object)?;
        if matches!(
            destination,
            lopdf::Object::String(..) | lopdf::Object::Name(..)
        ) {
            return Ok(LinkTarget::Named {
                name: Self::object_string(destination)
                    .ok_or_else(|| processing("命名目的地解码失败"))?,
            });
        }
        let values = destination.as_array()?;
        if values.len() < 2 {
            return Err(processing("Link 目的地数组不完整"));
        }
        let page_id = document
            .dereference(&values[0])?
            .0
            .ok_or_else(|| processing("内部 Link 目的地必须引用页面对象"))?;
        let page = document
            .get_pages()
            .into_iter()
            .find_map(|(number, id)| (id == page_id).then_some((number - 1) as usize))
            .ok_or_else(|| processing("Link 目的地不在页面树中"))?;
        let mode = Self::object_name(Self::resolve_object(document, &values[1])?)
            .and_then(|name| LinkDestinationMode::from_name(&name))
            .ok_or_else(|| processing("不支持的 Link 目的地模式"))?;
        if values.len() != 2 + mode.parameter_names().len() {
            return Err(processing("Link 目的地参数数量不正确"));
        }
        let mut parameters = Vec::new();
        for value in &values[2..] {
            let value = Self::resolve_object(document, value)?;
            let parameter = match value {
                lopdf::Object::Null => None,
                other => Some(
                    Self::object_number(other)
                        .ok_or_else(|| processing("Link 目的地参数不是数字/null"))?,
                ),
            };
            parameters.push(parameter);
        }
        let view = LinkDestinationView { mode, parameters };
        crate::xfdf::validate_link_view(&view)?;
        Ok(LinkTarget::Page { page, view })
    }

    pub(super) fn build_caret_appearance(caret: &CaretAnnotation) -> Option<lopdf::Object> {
        let rect = caret.base.rect.as_ref()?;
        let fringe = caret.fringe.clone().unwrap_or(Rect {
            left: 0.0,
            bottom: 0.0,
            right: 0.0,
            top: 0.0,
        });
        let width = (rect.right - rect.left) as f32;
        let height = (rect.top - rect.bottom) as f32;
        let mark_width = width - fringe.left as f32 - fringe.right as f32;
        let mark_height = height - fringe.bottom as f32 - fringe.top as f32;
        if !width.is_finite() || !height.is_finite() || mark_width <= 0.0 || mark_height <= 0.0 {
            return None;
        }
        let color = caret
            .base
            .color
            .as_deref()
            .and_then(Color::from_hex)
            .unwrap_or_default();
        let x = fringe.left as f32;
        let y = fringe.bottom as f32;
        let mut resources = lopdf::Dictionary::new();
        let drawing = if caret.symbol == CaretSymbol::P {
            let mut font = lopdf::Dictionary::new();
            font.set("Type", lopdf::Object::Name(b"Font".to_vec()));
            font.set("Subtype", lopdf::Object::Name(b"Type1".to_vec()));
            font.set("BaseFont", lopdf::Object::Name(b"Helvetica".to_vec()));
            font.set("Encoding", lopdf::Object::Name(b"WinAnsiEncoding".to_vec()));
            let mut fonts = lopdf::Dictionary::new();
            fonts.set("F1", font);
            resources.set("Font", fonts);
            let size = mark_height.min(mark_width / 0.6);
            format!("BT /F1 {size} Tf {x} {y} Td (\\266) Tj ET\n")
        } else {
            format!(
                "1 w {x} {y} m {} {} l {} {y} l S\n",
                x + mark_width / 2.0,
                y + mark_height,
                x + mark_width
            )
        };
        let mut gs = lopdf::Dictionary::new();
        gs.set("CA", lopdf::Object::Real(caret.base.opacity));
        gs.set("ca", lopdf::Object::Real(caret.base.opacity));
        let mut states = lopdf::Dictionary::new();
        states.set("GS1", gs);
        resources.set("ExtGState", states);
        let mut dict = lopdf::Dictionary::new();
        dict.set("Type", lopdf::Object::Name(b"XObject".to_vec()));
        dict.set("Subtype", lopdf::Object::Name(b"Form".to_vec()));
        dict.set(
            "BBox",
            lopdf::Object::Array(vec![0.into(), 0.into(), width.into(), height.into()]),
        );
        dict.set("Resources", resources);
        let content = format!(
            "q /GS1 gs {} {} {} RG {} {} {} rg\n{drawing}Q\n",
            color.r, color.g, color.b, color.r, color.g, color.b
        );
        Some(lopdf::Object::Stream(lopdf::Stream::new(
            dict,
            content.into_bytes(),
        )))
    }
}
