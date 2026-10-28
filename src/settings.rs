use eframe::egui::{self, ThemePreference};

use crate::theme::Accent;

/// User-controlled theme preference, per `Settings.dc.html`. Lives in memory only for now;
/// persistence to config lands in a later PR (plan item 1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub theme_preference: ThemePreference,
    pub accent: Accent,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_preference: ThemePreference::System,
            accent: Accent::Amber,
        }
    }
}

/// Renders the theme (system/dark/light) + accent controls, mutating `settings` in place.
/// Render glue only — exempt from unit tests per `AGENTS.md` §6; `Settings::default` above is
/// what's tested.
pub fn render(settings: &mut Settings, ui: &mut egui::Ui) {
    use strum::IntoEnumIterator;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_follow_system_theme_with_amber_accent() {
        let settings = Settings::default();
        assert_eq!(settings.theme_preference, ThemePreference::System);
        assert_eq!(settings.accent, Accent::Amber);
    }
}
