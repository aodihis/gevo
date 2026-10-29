use eframe::egui;
use gevo::APP_NAME;
use gevo::domain::profile::ConnectionProfile;
use gevo::domain::profile_store::{FileProfileStore, ProfileStore};
use gevo::state::{Route, Router, Settings};
use gevo::ui::profile_edit::{self, Outcome};
use gevo::ui::profiles::{self, LastTest};
use gevo::ui::route;
use gevo::ui::settings;
use gevo::ui::sidebar_switcher::{self, DotState, SwitcherAction, SwitcherState};
use gevo::ui::theme::{self, build_visuals, sidebar_bg};
use gevo::utils::config;
use gevo::utils::logging::{LogConfig, init_logging};

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
    let config_path = config::default_path();
    let settings = config::load(&config_path);

    // Keep the guard alive for the process lifetime so buffered log lines get flushed.
    // The logging toggle in Settings takes effect on restart (see src/ui/settings.rs).
    let _log_guard = init_logging(&LogConfig {
        enabled: settings.logging_enabled,
        ..LogConfig::default()
    });

    let profiles = FileProfileStore::new(FileProfileStore::default_path()).load_all();

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            gevo::ui::fonts::install_fonts(&cc.egui_ctx);
            Ok(Box::new(GevoApp {
                settings,
                config_path,
                profiles,
                ..Default::default()
            }))
        }),
    )
}

#[derive(Default)]
struct GevoApp {
    router: Router,
    settings: Settings,
    profiles: Vec<ConnectionProfile>,
    switcher: SwitcherState,
    config_path: std::path::PathBuf,
    last_tests: std::collections::HashMap<uuid::Uuid, LastTest>,
    applied_preference: Option<egui::ThemePreference>,
    applied_visuals: Option<(egui::Theme, theme::Accent)>,
    profile_edit: Option<profile_edit::ProfileEditScreen>,
}

impl eframe::App for GevoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let current = self.router.current();
        let mut clicked = None;
        let mut profiles_action = None;
        let mut settings_changed = false;
        let mut leave_edit = false;

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

        gevo::ui::shell::render(
            ui,
            sidebar_bg(resolved),
            |ui| {
                ui.heading(APP_NAME);
                ui.separator();
                // Dot stays idle until connection activation (later phase) produces a report.
                if let Some(SwitcherAction::ManageConnections) = sidebar_switcher::render(
                    ui,
                    &mut self.switcher,
                    &self.profiles,
                    DotState::from_report(None),
                ) {
                    clicked = Some(Route::Connections);
                }
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
                    settings_changed = settings::render(&mut self.settings, ui);
                } else if current == Route::Connections {
                    profiles_action = profiles::render(ui, &self.profiles, &self.last_tests, false);
                } else if current == Route::ConnectionEdit {
                    let screen = self
                        .profile_edit
                        .get_or_insert_with(|| profile_edit::ProfileEditScreen::new(None));
                    let outcome = screen.render(ui);
                    if !matches!(outcome, Outcome::None) {
                        self.profile_edit = None;
                        leave_edit = true;
                    }
                    if matches!(outcome, Outcome::Saved) {
                        self.profiles =
                            FileProfileStore::new(FileProfileStore::default_path()).load_all();
                    }
                } else {
                    route::dispatch(current, ui);
                }
            },
        );

        // Edit's target profile is picked up by the edit screen once it exists.
        if profiles_action.is_some() {
            self.router.go_to(Route::ConnectionEdit);
        }

        if leave_edit {
            clicked = Some(Route::Connections);
        }
        if let Some(route) = clicked {
            self.router.go_to(route);
        }

        if settings_changed && let Err(err) = config::save(&self.config_path, &self.settings) {
            tracing::error!(category = "CONFIG", %err, "failed to save settings");
        }
    }
}
