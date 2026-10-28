use eframe::egui::{self, ThemePreference};

use crate::theme::Accent;

/// User-controlled app settings, per `Settings.dc.html`: theme preference, accent, and the
/// file-logging toggle (`AGENTS.md` §9). Persisted to disk as TOML by `crate::config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub theme_preference: ThemePreference,
    pub accent: Accent,
    pub logging_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_preference: ThemePreference::System,
            accent: Accent::Amber,
            logging_enabled: false,
        }
    }
}

/// Renders the theme (system/dark/light) + accent controls, mutating `settings` in place.
/// Returns whether anything changed, so the caller knows to persist it. Render glue only —
/// exempt from unit tests per `AGENTS.md` §6; `Settings::default` above is what's tested.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_follow_system_theme_with_amber_accent_and_logging_off() {
        let settings = Settings::default();
        assert_eq!(settings.theme_preference, ThemePreference::System);
        assert_eq!(settings.accent, Accent::Amber);
        assert!(!settings.logging_enabled);
    }
}
