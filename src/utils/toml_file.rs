use std::fs;
use std::path::Path;

/// Loads `T` from `path` as TOML. Falls back to `T::default()` if the file is missing,
/// unreadable, or not valid TOML -- a corrupt file should never block startup.
pub fn load_or_default<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str(&contents).ok())
        .unwrap_or_default()
}

/// Like `load_or_default`, but if the file exists and fails to parse, renames it to
/// `<path>.bak` (overwriting any previous backup) before returning the default, instead of
/// silently discarding it. Use this for data that's expensive or impossible to re-create by
/// hand (e.g. saved connection profiles) -- `load_or_default` is fine when falling back to
/// defaults is harmless (e.g. app settings).
pub fn load_or_backup<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(contents) = fs::read_to_string(path) else {
        return T::default();
    };
    match toml::from_str(&contents) {
        Ok(value) => value,
        Err(_) => {
            let backup = path.with_extension("toml.bak");
            let _ = fs::remove_file(&backup);
            let _ = fs::rename(path, backup);
            T::default()
        }
    }
}

/// Persists `value` to `path` as TOML, creating parent directories as needed.
pub fn save<T: serde::Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let toml = toml::to_string_pretty(value).map_err(std::io::Error::other)?;
    fs::write(path, toml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    #[test]
    fn load_or_default_on_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(load_or_default::<Sample>(&path), Sample::default());
    }

    #[test]
    fn load_or_default_on_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.toml");
        fs::write(&path, "not valid toml {{{").unwrap();
        assert_eq!(load_or_default::<Sample>(&path), Sample::default());
    }

    #[test]
    fn load_or_backup_on_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(load_or_backup::<Sample>(&path), Sample::default());
    }

    #[test]
    fn load_or_backup_on_corrupt_file_renames_it_and_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.toml");
        fs::write(&path, "not valid toml {{{").unwrap();

        let loaded = load_or_backup::<Sample>(&path);

        assert_eq!(loaded, Sample::default());
        assert!(!path.exists(), "corrupt file should have been moved aside");
        let backup = dir.path().join("corrupt.toml.bak");
        assert_eq!(fs::read_to_string(backup).unwrap(), "not valid toml {{{");
    }

    #[test]
    fn load_or_backup_overwrites_a_previous_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.toml");
        let backup = dir.path().join("corrupt.toml.bak");
        fs::write(&backup, "stale backup").unwrap();
        fs::write(&path, "fresh corruption").unwrap();

        load_or_backup::<Sample>(&path);

        assert_eq!(fs::read_to_string(backup).unwrap(), "fresh corruption");
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("sample.toml");
        let value = Sample {
            name: "profile".into(),
            count: 3,
        };

        save(&path, &value).unwrap();

        assert_eq!(load_or_default::<Sample>(&path), value);
    }
}
