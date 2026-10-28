use std::path::PathBuf;

use crate::domain::profile::ConnectionProfile;

/// Persists the list of saved `ConnectionProfile`s. Non-secret fields only -- see
/// `ConnectionProfile` and `CredentialStore`.
pub trait ProfileStore {
    fn load_all(&self) -> Vec<ConnectionProfile>;
    fn save_all(&self, profiles: &[ConnectionProfile]) -> std::io::Result<()>;
}

#[derive(Default, serde::Deserialize)]
struct ProfileFile {
    profiles: Vec<ConnectionProfile>,
}

/// Borrowed counterpart of `ProfileFile`, so `save_all` can serialize without cloning the list.
#[derive(serde::Serialize)]
struct ProfileFileRef<'a> {
    profiles: &'a [ConnectionProfile],
}

/// `ProfileStore` backed by a single TOML file in the OS config dir, per plan item 2.3.
pub struct FileProfileStore {
    path: PathBuf,
}

impl FileProfileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> PathBuf {
        crate::utils::paths::app_path(dirs::config_dir(), "profiles.toml")
    }
}

impl ProfileStore for FileProfileStore {
    /// Falls back to an empty list if the file is missing, unreadable, or not valid TOML --
    /// a corrupt profiles file should never block startup (same fallback as `utils::config`).
    fn load_all(&self) -> Vec<ConnectionProfile> {
        crate::utils::toml_file::load_or_default::<ProfileFile>(&self.path).profiles
    }

    fn save_all(&self, profiles: &[ConnectionProfile]) -> std::io::Result<()> {
        crate::utils::toml_file::save(&self.path, &ProfileFileRef { profiles })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::domain::profile::{SshAuthMethod, SshConfig};

    #[test]
    fn default_path_ends_with_gevo_profiles_toml() {
        let path = FileProfileStore::default_path();
        let mut components: Vec<_> = path.components().collect();
        let file = components.pop().unwrap();
        let dir = components.pop().unwrap();
        assert_eq!(file.as_os_str(), "profiles.toml");
        assert_eq!(dir.as_os_str(), "gevo");
    }

    #[test]
    fn load_all_on_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileProfileStore::new(dir.path().join("does-not-exist.toml"));
        assert_eq!(store.load_all(), Vec::new());
    }

    #[test]
    fn load_all_on_corrupt_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.toml");
        fs::write(&path, "not valid toml {{{").unwrap();
        let store = FileProfileStore::new(path);
        assert_eq!(store.load_all(), Vec::new());
    }

    #[test]
    fn save_then_load_round_trips_profiles_including_ssh_config() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileProfileStore::new(dir.path().join("nested").join("profiles.toml"));

        let mut with_ssh = ConnectionProfile::new("Prod", "https://typesense.example.com:443");
        with_ssh.ssh = Some(SshConfig {
            host: "bastion.example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth: SshAuthMethod::PrivateKey {
                key_path: "/home/deploy/.ssh/id_ed25519".into(),
            },
            local_bind_port: 8108,
        });
        let without_ssh = ConnectionProfile::new("Local", "http://localhost:8108");
        let profiles = vec![with_ssh, without_ssh];

        store.save_all(&profiles).unwrap();
        let loaded = store.load_all();

        assert_eq!(loaded, profiles);
    }

    #[test]
    fn save_all_with_empty_list_round_trips_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileProfileStore::new(dir.path().join("profiles.toml"));

        store.save_all(&[]).unwrap();

        assert_eq!(store.load_all(), Vec::new());
    }
}
