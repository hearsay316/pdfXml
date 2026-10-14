mod app;
mod service;

use app::PdfXmlGuiApp;
use gpui::{AppContext, WindowOptions};
use gpui_component::Root;

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    
    app.run(move |cx| {
        gpui_component::init(cx);

        let bounds = gpui::Bounds::centered(
            None,
            gpui::size(gpui::px(1000.0), gpui::px(800.0)),
            cx,
        );

        cx.spawn(async move |cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("PDF 批注转换工具".into()),
                        appears_transparent: false,
                        traffic_light_position: None,
                    }),
                    focus: true,
                    show: true,
                    kind: gpui::WindowKind::Normal,
                    is_movable: true,
                    display_id: None,
                    app_id: Some("com.pdfxml.gui".to_string()),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| PdfXmlGuiApp::new(window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Failed to open window");
        })
        .detach();
    });
}
