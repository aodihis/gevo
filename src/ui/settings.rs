use eframe::egui;

use crate::state::Settings;
use crate::ui::theme::Accent;

/// Renders the theme (system/dark/light) + accent controls, mutating `settings` in place.
/// Returns whether anything changed, so the caller knows to persist it. Render glue only —
/// exempt from unit tests per `AGENTS.md` §6; `Settings::default` (in `crate::state`) is
/// what's tested.
pub fn render(settings: &mut Settings, ui: &mut egui::Ui) -> bool {
    use strum::IntoEnumIterator;

    let before = *settings;

    ui.heading("Settings");
    ui.separator();

    ui.label("Theme");
    settings.theme_preference.buttons(ui);

    ui.add_space(8.0);

    ui.label("Accent");
    ui.horizontal(|ui| {
        for accent in Accent::iter() {
            ui.selectable_value(&mut settings.accent, accent, accent.name());
        }
    });

    ui.add_space(8.0);

    ui.checkbox(&mut settings.logging_enabled, "Enable file logging");
    ui.label("Takes effect on restart.");

    *settings != before
}
