use eframe::egui;
use gevo::APP_NAME;
use gevo::domain::profile::ConnectionProfile;
use gevo::domain::profile_store::{FileProfileStore, ProfileStore};
use gevo::domain::ssh_tunnel::HostKeyPrompt;
use gevo::state::{Route, Router, Settings};
use gevo::ui::host_key_dialog::{
    HostKeyInbox, PROMPT_TIMEOUT, host_key_channel, show as show_host_key_dialog,
};
use gevo::ui::profile_edit::{self, Outcome};
use gevo::ui::profiles;
use gevo::ui::route;
use gevo::ui::session::{self, SessionSlot};
use gevo::ui::settings;
use gevo::ui::sidebar_switcher::{self, SwitcherAction, SwitcherState};
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
            let (prompt, host_keys) = host_key_channel(PROMPT_TIMEOUT);
            Ok(Box::new(GevoApp {
                settings,
                config_path,
                profiles,
                host_key_prompt: std::sync::Arc::new(prompt),
                host_keys,
                router: Router::default(),
                switcher: Default::default(),
                applied_preference: None,
                applied_visuals: None,
                profile_edit: None,
                session: SessionSlot::default(),
            }))
        }),
    )
}

struct GevoApp {
    router: Router,
    settings: Settings,
    profiles: Vec<ConnectionProfile>,
    // RefCell: the sidebar and central closures run in turn but are both alive at once.
    switcher: std::cell::RefCell<SwitcherState>,
    config_path: std::path::PathBuf,
    applied_preference: Option<egui::ThemePreference>,
    applied_visuals: Option<(egui::Theme, theme::Accent)>,
    profile_edit: Option<profile_edit::ProfileEditScreen>,
    session: SessionSlot,
    host_key_prompt: std::sync::Arc<dyn HostKeyPrompt>,
    host_keys: HostKeyInbox,
}

impl eframe::App for GevoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let current = self.router.current();
        let mut clicked = None;
        let mut profiles_action = None;
        let mut settings_changed = false;
        let mut leave_edit = false;
        let mut profile_saved = false;
        let mut selected = None;

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
                match sidebar_switcher::render(
                    ui,
                    &mut self.switcher.borrow_mut(),
                    &self.profiles,
                    &self.host_key_prompt,
                ) {
                    Some(SwitcherAction::ManageConnections) => clicked = Some(Route::Connections),
                    Some(SwitcherAction::Selected(id)) => selected = Some(id),
                    None => {}
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
                    profiles_action = profiles::render(
                        ui,
                        &self.profiles,
                        &self.switcher.borrow().results,
                        false,
                    );
                } else if current == Route::ConnectionEdit {
                    let screen = self.profile_edit.get_or_insert_with(|| {
                        profile_edit::ProfileEditScreen::new(None, self.host_key_prompt.clone())
                    });
                    let outcome = screen.render(ui);
                    if !matches!(outcome, Outcome::None) {
                        self.profile_edit = None;
                        leave_edit = true;
                    }
                    profile_saved = matches!(outcome, Outcome::Saved);
                } else {
                    route::dispatch(current, ui);
                }
            },
        );

        show_host_key_dialog(&ctx, &mut self.host_keys);

        // Edit's target profile is picked up by the edit screen once it exists.
        if profiles_action.is_some() {
            self.router.go_to(Route::ConnectionEdit);
        }

        if profile_saved {
            self.profiles = FileProfileStore::new(FileProfileStore::default_path()).load_all();
            // The saved profile may be the active one, so reconnect with its new settings.
            selected = self.switcher.borrow().active;
        }
        if let Some(id) = selected
            && let Some(profile) = self.profiles.iter().find(|p| p.id == id)
        {
            let pending =
                session::connect(profile.clone(), ctx.clone(), self.host_key_prompt.clone());
            self.session.begin(id, pending);
        }
        if self.session.poll() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
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
