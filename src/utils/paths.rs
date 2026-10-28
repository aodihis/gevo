use std::path::PathBuf;

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
    fn app_path_uses_base_when_present() {
        let base = PathBuf::from("/some/base");
        assert_eq!(
            app_path(Some(base), "leaf"),
            PathBuf::from("/some/base/gevo/leaf")
        );
    }

    #[test]
    fn app_path_falls_back_to_temp_dir_when_base_missing() {
        let path = app_path(None, "leaf");
        assert!(path.starts_with(std::env::temp_dir()));
        assert!(path.ends_with(std::path::Path::new("gevo").join("leaf")));
    }
}
