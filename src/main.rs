use eframe::egui;
use gevo::APP_NAME;
use gevo::logging::{LogConfig, init_logging};
use gevo::theme::{Accent, ThemeMode, build_visuals, sidebar_bg};

fn main() -> eframe::Result {
    // Keep the guard alive for the process lifetime so buffered log lines get flushed.
    let _log_guard = init_logging(&LogConfig::default());

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            gevo::fonts::install_fonts(&cc.egui_ctx);
            cc.egui_ctx
                .set_visuals(build_visuals(ThemeMode::Dark, Accent::Amber));
            Ok(Box::<GevoApp>::default())
        }),
    )
}

#[derive(Default)]
struct GevoApp;

impl eframe::App for GevoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        gevo::shell::render(
            ui,
            sidebar_bg(ThemeMode::Dark),
            |ui| {
                ui.heading(APP_NAME);
            },
            |ui| {
                ui.label("Gevo");
            },
            |_ui| {},
        );
    }
}
