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
