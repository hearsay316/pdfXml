use crate::service::{self, AnnotationOverview, ConversionMode};
use gpui::prelude::FluentBuilder;
use gpui::{div, px, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Window};
use gpui_component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    label::Label,
    scroll::ScrollableElement,
    Sizable,
    StyledExt,
};
use rfd::FileDialog;
use std::path::Path;

pub struct PdfXmlGuiApp {
    mode: ConversionMode,
    // XFDF→PDF 模式: xfdf_path=XFDF输入, pdf_path=目标PDF输入, output_path=输出PDF
    // PDF→XFDF 模式: pdf_path=PDF输入, output_path=输出XFDF
    xfdf_path: gpui::Entity<InputState>,
    pdf_path: gpui::Entity<InputState>,
    output_path: gpui::Entity<InputState>,
    overview: AnnotationOverview,
    status_message: String,
    error_message: Option<String>,
}

impl PdfXmlGuiApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            mode: ConversionMode::XfdfToPdf,
            xfdf_path: cx.new(|cx| InputState::new(window, cx)),
            pdf_path: cx.new(|cx| InputState::new(window, cx)),
            output_path: cx.new(|cx| InputState::new(window, cx)),
            overview: AnnotationOverview::default(),
            status_message: "准备就绪。".to_string(),
            error_message: None,
        }
    }

    fn read_input(entity: &gpui::Entity<InputState>, cx: &gpui::App) -> String {
        entity.read(cx).value().to_string()
    }

    fn set_input_value(
        entity: &gpui::Entity<InputState>,
        value: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = value.into();
        entity.update(cx, |input, cx| input.set_value(value, window, cx));
    }

    fn set_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.error_message = Some(message.into());
        cx.notify();
    }

    fn set_success(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.error_message = None;
        self.status_message = message.into();
        cx.notify();
    }

    fn select_mode(&mut self, new_mode: ConversionMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == new_mode {
            return;
        }
        self.mode = new_mode;
        // 切换模式时清空所有路径
        Self::set_input_value(&self.xfdf_path, "", window, cx);
        Self::set_input_value(&self.pdf_path, "", window, cx);
        Self::set_input_value(&self.output_path, "", window, cx);
        self.overview = AnnotationOverview::default();
        self.error_message = None;
        self.status_message = match self.mode {
            ConversionMode::XfdfToPdf => "模式：XFDF + PDF → 合并输出 PDF".to_string(),
            ConversionMode::PdfToXfdf => "模式：PDF → 导出 XFDF".to_string(),
        };
        cx.notify();
    }

    fn browse_xfdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(
                FileDialog::new()
                    .add_filter("XFDF/XML", &["xfdf", "xml"])
                    .pick_file(),
            );
        });
        match rx.recv() {
            Ok(Some(path)) => {
                let s = path.display().to_string();
                // 自动推导输出路径
                let out = service::suggest_output_path(&path, self.mode);
                Self::set_input_value(&self.xfdf_path, s.clone(), window, cx);
                Self::set_input_value(&self.output_path, out.display().to_string(), window, cx);
                self.set_success(format!("已选择 XFDF：{}", s), cx);
            }
            Ok(None) => {}
            Err(_) => self.set_error("文件对话框出错。", cx),
        }
    }

    fn browse_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.mode;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(FileDialog::new().add_filter("PDF", &["pdf"]).pick_file());
        });
        match rx.recv() {
            Ok(Some(path)) => {
                let s = path.display().to_string();
                Self::set_input_value(&self.pdf_path, s.clone(), window, cx);
                // PDF→XFDF 模式下自动推导输出路径
                if mode == ConversionMode::PdfToXfdf {
                    let out = service::suggest_output_path(&path, mode);
                    Self::set_input_value(&self.output_path, out.display().to_string(), window, cx);
                }
                self.set_success(format!("已选择 PDF：{}", s), cx);
            }
            Ok(None) => {}
            Err(_) => self.set_error("文件对话框出错。", cx),
        }
    }

    fn browse_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.mode;
        let current = Self::read_input(&self.output_path, cx);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut dialog = FileDialog::new();
            if !current.trim().is_empty() {
                let p = std::path::PathBuf::from(&current);
                if let Some(parent) = p.parent().filter(|p| !p.as_os_str().is_empty()) {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    dialog = dialog.set_file_name(name);
                }
            }
            dialog = match mode {
                ConversionMode::XfdfToPdf => dialog.add_filter("PDF", &["pdf"]),
                ConversionMode::PdfToXfdf => dialog.add_filter("XFDF", &["xfdf"]),
            };
            let _ = tx.send(dialog.save_file());
        });
        match rx.recv() {
            Ok(Some(path)) => {
                let s = path.display().to_string();
                Self::set_input_value(&self.output_path, s.clone(), window, cx);
                self.set_success(format!("已选择输出：{}", s), cx);
            }
            Ok(None) => {}
            Err(_) => self.set_error("文件对话框出错。", cx),
        }
    }

    fn load_annotations(&mut self, cx: &mut Context<Self>) {
        let input = match self.mode {
            ConversionMode::XfdfToPdf => Self::read_input(&self.xfdf_path, cx),
            ConversionMode::PdfToXfdf => Self::read_input(&self.pdf_path, cx),
        };
        if input.trim().is_empty() {
            self.set_error("请先选择输入文件。", cx);
            return;
        }
        match service::load_annotation_overview(self.mode, Path::new(&input)) {
            Ok(overview) => {
                let total = overview.total_annotations;
                self.overview = overview;
                self.set_success(format!("已加载批注，共 {} 条。", total), cx);
            }
            Err(e) => self.set_error(e.to_string(), cx),
        }
    }

    fn convert(&mut self, cx: &mut Context<Self>) {
        let output = Self::read_input(&self.output_path, cx);
        if output.trim().is_empty() {
            self.set_error("请先选择输出文件。", cx);
            return;
        }

        let result = match self.mode {
            ConversionMode::XfdfToPdf => {
                let xfdf = Self::read_input(&self.xfdf_path, cx);
                let pdf = Self::read_input(&self.pdf_path, cx);
                if xfdf.trim().is_empty() {
                    self.set_error("请先选择 XFDF 文件。", cx);
                    return;
                }
                if pdf.trim().is_empty() {
                    self.set_error("请先选择目标 PDF 文件。", cx);
                    return;
                }
                service::convert(
                    self.mode,
                    Path::new(&xfdf),
                    Path::new(&output),
                    Some(Path::new(&pdf)),
                )
            }
            ConversionMode::PdfToXfdf => {
                let pdf = Self::read_input(&self.pdf_path, cx);
                if pdf.trim().is_empty() {
                    self.set_error("请先选择 PDF 文件。", cx);
                    return;
                }
                service::convert(self.mode, Path::new(&pdf), Path::new(&output), None)
            }
        };

        match result {
            Ok(r) => {
                self.overview = r.overview;
                self.set_success(r.message, cx);
            }
            Err(e) => self.set_error(e.to_string(), cx),
        }
    }
}

impl Render for PdfXmlGuiApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let is_error = self.error_message.is_some();
        let status_text = self
            .error_message
            .clone()
            .unwrap_or_else(|| self.status_message.clone());

        let (status_bg, status_border, status_fg) = if is_error {
            (theme.danger.opacity(0.08), theme.danger.opacity(0.25), theme.danger)
        } else {
            (theme.success.opacity(0.08), theme.success.opacity(0.25), theme.success)
        };

        let card = {
            let bg = theme.background;
            let border = theme.border;
            let shadow = theme.shadow;
            move || {
                div()
                    .v_flex()
                    .gap_3()
                    .p_4()
                    .rounded_md()
                    .bg(bg)
                    .border_1()
                    .border_color(border)
                    .when(shadow, |this| this.shadow_sm())
            }
        };

        // 文件行辅助：label + input + 浏览按钮
        let file_row = |label: &str, entity: &gpui::Entity<InputState>| {
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(div().w_24().flex_none().child(Label::new(label.to_string()).text_sm()))
                .child(Input::new(entity).flex_1())
        };

        let mut content = div()
            .v_flex()
            .w_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .p_4()
            .gap_4()
            // 标题
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(Label::new("PDF/XML 批注转换工具").text_xl().font_bold())
                    .child(
                        Label::new("在 XFDF 与 PDF 之间转换批注数据")
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    ),
            )
            // 模式切换
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("mode-xfdf")
                            .label("XFDF + PDF → PDF（合并）")
                            .when(self.mode == ConversionMode::XfdfToPdf, |b| b.primary())
                            .when(self.mode != ConversionMode::XfdfToPdf, |b| b.ghost())
                            .on_click(cx.listener(|v, _, w, cx| v.select_mode(ConversionMode::XfdfToPdf, w, cx))),
                    )
                    .child(
                        Button::new("mode-pdf")
                            .label("PDF → XFDF（导出）")
                            .when(self.mode == ConversionMode::PdfToXfdf, |b| b.primary())
                            .when(self.mode != ConversionMode::PdfToXfdf, |b| b.ghost())
                            .on_click(cx.listener(|v, _, w, cx| v.select_mode(ConversionMode::PdfToXfdf, w, cx))),
                    ),
            );

        // 文件选择卡片
        if self.mode == ConversionMode::XfdfToPdf {
            content = content.child(
                card()
                    .child(
                        Label::new("输入文件")
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.muted_foreground),
                    )
                    // XFDF 输入
                    .child(
                        file_row("XFDF 文件", &self.xfdf_path)
                            .child(
                                Button::new("browse-xfdf")
                                    .label("浏览...")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|v, _, w, cx| v.browse_xfdf(w, cx))),
                            ),
                    )
                    // 目标 PDF 输入
                    .child(
                        file_row("目标 PDF", &self.pdf_path)
                            .child(
                                Button::new("browse-pdf")
                                    .label("浏览...")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|v, _, w, cx| v.browse_pdf(w, cx))),
                            ),
                    )
                    .child(
                        Label::new("输出文件")
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        file_row("输出 PDF", &self.output_path)
                            .child(
                                Button::new("browse-output")
                                    .label("浏览...")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|v, _, w, cx| v.browse_output(w, cx))),
                            ),
                    ),
            );
        } else {
            content = content.child(
                card()
                    .child(
                        Label::new("输入文件")
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        file_row("PDF 文件", &self.pdf_path)
                            .child(
                                Button::new("browse-pdf")
                                    .label("浏览...")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|v, _, w, cx| v.browse_pdf(w, cx))),
                            ),
                    )
                    .child(
                        Label::new("输出文件")
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        file_row("输出 XFDF", &self.output_path)
                            .child(
                                Button::new("browse-output")
                                    .label("浏览...")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|v, _, w, cx| v.browse_output(w, cx))),
                            ),
                    ),
            );
        }

        // 操作按钮
        content = content.child(
            div()
                .h_flex()
                .gap_2()
                .justify_center()
                .child(
                    Button::new("load")
                        .label("预览批注")
                        .secondary()
                        .on_click(cx.listener(|v, _, _, cx| v.load_annotations(cx))),
                )
                .child(
                    Button::new("convert")
                        .label("开始转换")
                        .primary()
                        .on_click(cx.listener(|v, _, _, cx| v.convert(cx))),
                ),
        );

        // 状态栏
        content = content.child(
            div()
                .v_flex()
                .gap_1()
                .p_3()
                .rounded_md()
                .bg(status_bg)
                .border_1()
                .border_color(status_border)
                .child(
                    Label::new(status_text)
                        .text_sm()
                        .text_color(status_fg),
                ),
        );

        // 批注摘要
        if self.overview.total_annotations > 0 {
            let radius = theme.radius;
            let mut type_tags = div().h_flex().gap_1().flex_wrap();
            for (ty, count) in &self.overview.type_counts {
                type_tags = type_tags.child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded(radius * 0.5)
                        .bg(theme.secondary)
                        .text_color(theme.secondary_foreground)
                        .child(Label::new(format!("{}: {}", ty, count)).text_xs()),
                );
            }

            content = content.child(
                card()
                    .child(
                        Label::new("批注摘要")
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_4()
                            .child(
                                div()
                                    .v_flex()
                                    .gap_1()
                                    .flex_1()
                                    .child(Label::new("批注总数").text_xs().text_color(theme.muted_foreground))
                                    .child(
                                        Label::new(self.overview.total_annotations.to_string())
                                            .text_xl()
                                            .font_bold(),
                                    ),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .gap_1()
                                    .flex_1()
                                    .child(Label::new("涉及页数").text_xs().text_color(theme.muted_foreground))
                                    .child(
                                        Label::new(self.overview.total_pages.to_string())
                                            .text_xl()
                                            .font_bold(),
                                    ),
                            ),
                    )
                    .child(type_tags),
            );

            // 批注预览列表
            let radius = theme.radius;
            let mut list = div().v_flex().gap_1();
            for row in self.overview.rows.iter().take(12) {
                let rect_text = if row.rect.is_empty() {
                    "Rect: -".to_string()
                } else {
                    row.rect.clone()
                };
                list = list.child(
                    div()
                        .p_2()
                        .rounded(radius * 0.5)
                        .bg(theme.accent.opacity(0.05))
                        .child(
                            Label::new(format!(
                                "#{} [{}] 第{}页 | {} | {}",
                                row.index,
                                row.annotation_type,
                                row.page,
                                if row.title.is_empty() { "-" } else { &row.title },
                                if row.contents.is_empty() { "-" } else { &row.contents },
                            ))
                            .text_sm(),
                        )
                        .child(Label::new(rect_text).text_xs().text_color(theme.muted_foreground)),
                );
            }

            content = content.child(
                card()
                    .child(
                        Label::new(format!("批注预览（共 {} 条，显示前 12）", self.overview.total_annotations))
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .max_h(px(256.))
                            .overflow_y_scrollbar()
                            .child(list),
                    ),
            );
        }

        div()
            .id("main-scroll")
            .size_full()
            .bg(theme.background)
            .overflow_y_scroll()
            .child(content)
    }
}
