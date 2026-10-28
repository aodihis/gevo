use std::fs;
use std::path::{Path, PathBuf};

use crate::settings::Settings;

/// Default config file location, under the OS config dir per `AGENTS.md` §9's logging
/// directory convention (same "OS data/config dir, not temp" rule applies here).
pub fn default_path() -> PathBuf {
    crate::app_path(dirs::config_dir(), "config.toml")
}

/// Loads settings from `path`. Falls back to `Settings::default()` if the file is missing,
/// unreadable, or not valid TOML — a corrupt config should never block startup.
pub fn load(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str(&contents).ok())
        .unwrap_or_default()
}

/// Persists `settings` to `path` as TOML, creating parent directories as needed.
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let toml = toml::to_string_pretty(settings).map_err(std::io::Error::other)?;
    fs::write(path, toml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::ThemePreference;

    use crate::theme::Accent;

    #[test]
    fn default_path_ends_with_gevo_config_toml() {
        let path = default_path();
        let mut components: Vec<_> = path.components().collect();
        let file = components.pop().unwrap();
        let dir = components.pop().unwrap();
        assert_eq!(file.as_os_str(), "config.toml");
        assert_eq!(dir.as_os_str(), "gevo");
    }

    #[test]
    fn load_on_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(load(&path), Settings::default());
    }

    #[test]
    fn load_on_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.toml");
        fs::write(&path, "not valid toml {{{").unwrap();
        assert_eq!(load(&path), Settings::default());
    }

    #[test]
    fn save_then_load_round_trips_non_default_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let settings = Settings {
            theme_preference: ThemePreference::Light,
            accent: Accent::Violet,
            logging_enabled: true,
        };

        save(&path, &settings).unwrap();
        let loaded = load(&path);

        assert_eq!(loaded, settings);
    }
}
