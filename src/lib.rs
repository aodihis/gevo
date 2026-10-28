pub mod config;
pub mod fonts;
pub mod logging;
pub mod route;
pub mod secret;
pub mod settings;
pub mod shell;
pub mod theme;

use std::path::PathBuf;

pub const APP_NAME: &str = "Gevo";

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Joins an OS base directory (falling back to the temp dir if unavailable) with `gevo/<leaf>`.
pub(crate) fn app_path(base: Option<PathBuf>, leaf: &str) -> PathBuf {
    base.unwrap_or_else(std::env::temp_dir)
        .join("gevo")
        .join(leaf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_gevo() {
        assert_eq!(APP_NAME, "Gevo");
    }

    #[test]
    fn version_is_semver_shaped() {
        let v = version();
        let parts: Vec<&str> = v.split('.').collect();
        assert_eq!(parts.len(), 3, "expected x.y.z, got {v}");
        for part in parts {
            assert!(
                part.chars().all(|c| c.is_ascii_digit()),
                "non-numeric segment in {v}"
            );
        }
    }

    #[test]
    fn app_path_uses_base_when_present() {
        let base = std::path::PathBuf::from("/some/base");
        assert_eq!(
            app_path(Some(base), "leaf"),
            std::path::PathBuf::from("/some/base/gevo/leaf")
        );
    }

    #[test]
    fn app_path_falls_back_to_temp_dir_when_base_missing() {
        let path = app_path(None, "leaf");
        assert!(path.starts_with(std::env::temp_dir()));
        assert!(path.ends_with(std::path::Path::new("gevo").join("leaf")));
    }
}
