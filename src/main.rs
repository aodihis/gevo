use eframe::egui;
use gevo::APP_NAME;
use gevo::logging::{LogConfig, init_logging};
use gevo::route::{self, Route, Router};
use gevo::theme::{Accent, ThemeMode, build_visuals, sidebar_bg};

/// Top-level sidebar nav destinations, per `Sidebar.dc.html` ("Server" nav + bottom section).
/// Other routes (ConnectionEdit, SchemaEditor, Documents, Import, the modals) are reached from
/// within a screen, not from this top-level list.
const NAV_ROUTES: [Route; 5] = [
    Route::Connections,
    Route::Collections,
    Route::ApiKeys,
    Route::Health,
    Route::Settings,
];

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
struct GevoApp {
    router: Router,
}

impl eframe::App for GevoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let current = self.router.current();
        let mut clicked = None;

        gevo::shell::render(
            ui,
            sidebar_bg(ThemeMode::Dark),
            |ui| {
                ui.heading(APP_NAME);
                ui.separator();
                for nav_route in NAV_ROUTES {
                    if ui
                        .selectable_label(current == nav_route, nav_route.title())
                        .clicked()
                    {
                        clicked = Some(nav_route);
                    }
                }
            },
            |ui| {
                ui.label(current.title());
            },
            |ui| {
                route::dispatch(current, ui);
            },
        );

        if let Some(route) = clicked {
            self.router.go_to(route);
        }
    }
}
