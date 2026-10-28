use eframe::egui;
use gevo::APP_NAME;
use gevo::logging::{LogConfig, init_logging};
use gevo::route::{self, Route, Router};
use gevo::settings::{self, Settings};
use gevo::theme::{self, build_visuals, sidebar_bg};

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
            Ok(Box::<GevoApp>::default())
        }),
    )
}

#[derive(Default)]
struct GevoApp {
    router: Router,
    settings: Settings,
    applied_preference: Option<egui::ThemePreference>,
    applied_visuals: Option<(egui::Theme, theme::Accent)>,
}

impl eframe::App for GevoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let current = self.router.current();
        let mut clicked = None;

        let ctx = ui.ctx().clone();
        let settings = self.settings;

        // Only call set_theme when the preference itself changed; cheap either way, but this
        // keeps it off the hot path when nothing changed.
        if self.applied_preference != Some(settings.theme_preference) {
            ctx.set_theme(settings.theme_preference);
            self.applied_preference = Some(settings.theme_preference);
        }
        // ctx.theme() is a cheap memory read and is re-checked every frame (unlike the
        // preference) so a live OS dark/light flip is still picked up while on "System".
        let resolved = ctx.theme();
        if self.applied_visuals != Some((resolved, settings.accent)) {
            ctx.set_visuals(build_visuals(resolved, settings.accent));
            self.applied_visuals = Some((resolved, settings.accent));
        }

        gevo::shell::render(
            ui,
            sidebar_bg(resolved),
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
                if current == Route::Settings {
                    settings::render(&mut self.settings, ui);
                } else {
                    route::dispatch(current, ui);
                }
            },
        );

        if let Some(route) = clicked {
            self.router.go_to(route);
        }
    }
}
