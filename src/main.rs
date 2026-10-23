use eframe::egui;
use gevo::APP_NAME;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|_cc| Ok(Box::<GevoApp>::default())),
    )
}

#[derive(Default)]
struct GevoApp;

impl eframe::App for GevoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading(APP_NAME);
    }
}
