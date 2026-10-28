pub mod fonts;
pub mod logging;
pub mod route;
pub mod secret;
pub mod settings;
pub mod shell;
pub mod theme;

pub const APP_NAME: &str = "Gevo";

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
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
}
