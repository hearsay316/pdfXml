use crate::service::{self, AnnotationOverview, ConversionMode};
use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, App, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    label::Label,
    scroll::ScrollableElement,
    ActiveTheme, Sizable, StyledExt,
};
use std::path::Path;

pub struct PdfXmlGuiApp {
    mode: ConversionMode,
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
            status_message: "准备就绪".to_string(),
            error_message: None,
        }
    }

    fn read_input(entity: &gpui::Entity<InputState>, cx: &App) -> String {
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

    fn select_mode(
        &mut self,
        new_mode: ConversionMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode == new_mode {
            return;
        }
        self.mode = new_mode;
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
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("XFDF/XML", &["xfdf", "xml"])
            .pick_file()
        {
            let s = path.display().to_string();
            let out = service::suggest_output_path(&path, self.mode);
            Self::set_input_value(&self.xfdf_path, s.clone(), window, cx);
            Self::set_input_value(&self.output_path, out.display().to_string(), window, cx);
            self.set_success(format!("已选择 XFDF：{}", s), cx);
        }
    }

    fn browse_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick_file()
        {
            let s = path.display().to_string();
            Self::set_input_value(&self.pdf_path, s.clone(), window, cx);
            if self.mode == ConversionMode::PdfToXfdf {
                let out = service::suggest_output_path(&path, self.mode);
                Self::set_input_value(&self.output_path, out.display().to_string(), window, cx);
            }
            self.set_success(format!("已选择 PDF：{}", s), cx);
        }
    }

    fn browse_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = Self::read_input(&self.output_path, cx);
        let mut dialog = rfd::FileDialog::new();

        if !current.trim().is_empty() {
            let p = std::path::PathBuf::from(&current);
            if let Some(parent) = p.parent().filter(|p| !p.as_os_str().is_empty()) {
                dialog = dialog.set_directory(parent);
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                dialog = dialog.set_file_name(name);
            }
        }

        dialog = match self.mode {
            ConversionMode::XfdfToPdf => dialog.add_filter("PDF", &["pdf"]),
            ConversionMode::PdfToXfdf => dialog.add_filter("XFDF", &["xfdf"]),
        };

        if let Some(path) = dialog.save_file() {
            let s = path.display().to_string();
            Self::set_input_value(&self.output_path, s.clone(), window, cx);
            self.set_success(format!("已选择输出：{}", s), cx);
        }
    }

    fn load_annotations(&mut self, cx: &mut Context<Self>) {
        let input = match self.mode {
            ConversionMode::XfdfToPdf => Self::read_input(&self.xfdf_path, cx),
            ConversionMode::PdfToXfdf => Self::read_input(&self.pdf_path, cx),
        };
        if input.trim().is_empty() {
            self.set_error("请先选择输入文件", cx);
            return;
        }
        match service::load_annotation_overview(self.mode, Path::new(&input)) {
            Ok(overview) => {
                let total = overview.total_annotations;
                self.overview = overview;
                self.set_success(format!("已加载 {} 条批注", total), cx);
            }
            Err(e) => self.set_error(format!("加载失败：{}", e), cx),
        }
    }

    fn convert(&mut self, cx: &mut Context<Self>) {
        let output = Self::read_input(&self.output_path, cx);
        if output.trim().is_empty() {
            self.set_error("请先选择输出文件", cx);
            return;
        }

        let result = match self.mode {
            ConversionMode::XfdfToPdf => {
                let xfdf = Self::read_input(&self.xfdf_path, cx);
                let pdf = Self::read_input(&self.pdf_path, cx);
                if xfdf.trim().is_empty() {
                    self.set_error("请先选择 XFDF 文件", cx);
                    return;
                }
                if pdf.trim().is_empty() {
                    self.set_error("请先选择目标 PDF 文件", cx);
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
                    self.set_error("请先选择 PDF 文件", cx);
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
            Err(e) => self.set_error(format!("转换失败：{}", e), cx),
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
            (
                theme.danger.opacity(0.1),
                theme.danger.opacity(0.3),
                theme.danger,
            )
        } else {
            (
                theme.success.opacity(0.1),
                theme.success.opacity(0.3),
                theme.success,
            )
        };

        // 卡片样式
        let card = {
            let bg = theme.background;
            let border = theme.border;
            move || {
                div()
                    .v_flex()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .bg(bg)
                    .border_1()
                    .border_color(border)
            }
        };

        // 文件输入行
        let file_row = |label: &str, entity: &gpui::Entity<InputState>| {
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .w(px(100.))
                        .flex_none()
                        .child(Label::new(label.to_string()).text_sm()),
                )
                .child(Input::new(entity).flex_1())
        };

        let mut content = div()
            .v_flex()
            .w_full()
            .max_w(px(900.))
            .mx_auto()
            .bg(theme.background)
            .text_color(theme.foreground)
            .p_6()
            .gap_6()
            // 标题区
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        Label::new("PDF 批注转换工具")
                            .text_2xl()
                            .font_bold()
                            .text_color(theme.foreground),
                    )
                    .child(
                        Label::new("在 XFDF 与 PDF 之间转换批注数据")
                            .text_sm()
                            .text_color(theme.muted_foreground),
                    ),
            )
            // 模式切换
            .child(
                div().h_flex().gap_3().child(
                    Button::new("mode-xfdf")
                        .label("XFDF + PDF → PDF")
                        .when(self.mode == ConversionMode::XfdfToPdf, |b| b.primary())
                        .when(self.mode != ConversionMode::XfdfToPdf, |b| b.ghost())
                        .on_click(cx.listener(|v, _, w, cx| {
                            v.select_mode(ConversionMode::XfdfToPdf, w, cx)
                        })),
                ).child(
                    Button::new("mode-pdf")
                        .label("PDF → XFDF")
                        .when(self.mode == ConversionMode::PdfToXfdf, |b| b.primary())
                        .when(self.mode != ConversionMode::PdfToXfdf, |b| b.ghost())
                        .on_click(cx.listener(|v, _, w, cx| {
                            v.select_mode(ConversionMode::PdfToXfdf, w, cx)
                        })),
                ),
            );

        // 文件选择卡片
        if self.mode == ConversionMode::XfdfToPdf {
            content = content.child(
                card()
                    .child(
                        Label::new("输入文件")
                            .text_base()
                            .font_semibold()
                            .text_color(theme.foreground),
                    )
                    .child(
                        file_row("XFDF 文件", &self.xfdf_path).child(
                            Button::new("browse-xfdf")
                                .label("浏览")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|v, _, w, cx| v.browse_xfdf(w, cx))),
                        ),
                    )
                    .child(
                        file_row("目标 PDF", &self.pdf_path).child(
                            Button::new("browse-pdf")
                                .label("浏览")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|v, _, w, cx| v.browse_pdf(w, cx))),
                        ),
                    )
                    .child(
                        file_row("输出 PDF", &self.output_path).child(
                            Button::new("browse-output")
                                .label("浏览")
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
                            .text_base()
                            .font_semibold()
                            .text_color(theme.foreground),
                    )
                    .child(
                        file_row("PDF 文件", &self.pdf_path).child(
                            Button::new("browse-pdf")
                                .label("浏览")
                                .small()
                                .ghost()
                                .on_click(cx.listener(|v, _, w, cx| v.browse_pdf(w, cx))),
                        ),
                    )
                    .child(
                        file_row("输出 XFDF", &self.output_path).child(
                            Button::new("browse-output")
                                .label("浏览")
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
                .gap_3()
                .child(
                    Button::new("load")
                        .label("预览批注")
                        .ghost()
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
                .p_3()
                .rounded_lg()
                .bg(status_bg)
                .border_1()
                .border_color(status_border)
                .child(Label::new(status_text).text_sm().text_color(status_fg)),
        );

        // 批注预览
        if !self.overview.rows.is_empty() {
            let mut stats = div()
                .h_flex()
                .gap_4()
                .flex_wrap()
                .child(
                    Label::new(format!("总计：{} 条", self.overview.total_annotations))
                        .text_sm()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    Label::new(format!("页数：{}", self.overview.total_pages))
                        .text_sm()
                        .text_color(theme.muted_foreground),
                );

            for (type_name, count) in &self.overview.type_counts {
                stats = stats.child(
                    Label::new(format!("{}：{}", type_name, count))
                        .text_sm()
                        .text_color(theme.muted_foreground),
                );
            }

            let mut list = div().v_flex().gap_2();
            for row in self.overview.rows.iter().take(12) {
                list = list.child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(theme.background)
                        .border_1()
                        .border_color(theme.border)
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .h_flex()
                                .gap_3()
                                .child(
                                    Label::new(format!("#{}", row.index))
                                        .text_xs()
                                        .text_color(theme.muted_foreground),
                                )
                                .child(
                                    Label::new(row.annotation_type.clone())
                                        .text_xs()
                                        .font_semibold()
                                        .text_color(theme.primary),
                                )
                                .child(
                                    Label::new(format!("第 {} 页", row.page))
                                        .text_xs()
                                        .text_color(theme.muted_foreground),
                                ),
                        )
                        .when(!row.contents.is_empty(), |this| {
                            this.child(
                                Label::new(row.contents.clone())
                                    .text_sm()
                                    .text_color(theme.foreground),
                            )
                        }),
                );
            }

            content = content.child(
                card()
                    .child(
                        Label::new(format!(
                            "批注预览（显示前 12 条，共 {} 条）",
                            self.overview.total_annotations
                        ))
                        .text_base()
                        .font_semibold()
                        .text_color(theme.foreground),
                    )
                    .child(stats)
                    .child(
                        div()
                            .max_h(px(400.))
                            .overflow_y_scrollbar()
                            .v_flex()
                            .gap_2()
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
