use chess_engine::{app_icon, ui_app::ChessApp};
use eframe::egui::{IconData, ViewportBuilder};

fn main() -> eframe::Result {
    let app_title = if cfg!(debug_assertions) {
        "Chess Engine (debug build)"
    } else {
        "Chess Engine"
    };
    let app_icon = app_icon::render_icon(app_icon::DEFAULT_ICON_SIZE);
    let native_options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([900.0, 720.0])
            .with_icon(IconData {
                rgba: app_icon.rgba,
                width: app_icon.width,
                height: app_icon.height,
            }),
        ..Default::default()
    };

    eframe::run_native(
        app_title,
        native_options,
        Box::new(|cc| Ok(Box::new(ChessApp::new(cc)))),
    )
}
