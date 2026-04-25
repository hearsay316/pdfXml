mod app;
mod service;

use app::PdfXmlGuiApp;
use gpui::{AppContext, WindowOptions};
use gpui_component::Root;

fn main() {
    let _ = env_logger::try_init();

    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| PdfXmlGuiApp::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
