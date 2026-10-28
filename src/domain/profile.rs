use std::path::PathBuf;

use uuid::Uuid;

/// Per-profile SSH auth method, per `0002-v1-requirements.md` §1 "SSH tunnel support".
/// The actual secret (password or key passphrase) lives in the OS keychain, keyed by the
/// owning profile's id (see `CredentialStore`, plan item 2.2) — never here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SshAuthMethod {
    Password,
    PrivateKey { key_path: PathBuf },
}

/// Non-secret SSH tunnel config for a `ConnectionProfile`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuthMethod,
    pub local_bind_port: u16,
}

/// A saved Typesense connection. Holds only non-secret fields — the admin API key and any SSH
/// password/passphrase are stored in the OS keychain (`CredentialStore`, plan item 2.2), keyed
/// by `id`, and are never part of this struct or its `Debug` output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub ssh: Option<SshConfig>,
}

impl ConnectionProfile {
    /// Creates a new profile with a fresh id and no SSH tunnel configured.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            url: url.into(),
            ssh: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_name_and_url() {
        let profile = ConnectionProfile::new("Prod", "https://typesense.example.com:443");
        assert_eq!(profile.name, "Prod");
        assert_eq!(profile.url, "https://typesense.example.com:443");
    }

    #[test]
    fn new_has_no_ssh_config_by_default() {
        let profile = ConnectionProfile::new("Prod", "https://typesense.example.com:443");
        assert_eq!(profile.ssh, None);
    }

    #[test]
    fn new_generates_a_unique_id_per_call() {
        let a = ConnectionProfile::new("A", "https://a.example.com");
        let b = ConnectionProfile::new("B", "https://b.example.com");
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn ssh_config_round_trips_through_toml() {
        let config = SshConfig {
            host: "bastion.example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth: SshAuthMethod::PrivateKey {
                key_path: PathBuf::from("/home/deploy/.ssh/id_ed25519"),
            },
            local_bind_port: 8108,
        };

        let serialized = toml::to_string(&config).unwrap();
        let deserialized: SshConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized, config);
    }

    #[test]
    fn password_auth_method_round_trips_through_toml() {
        let config = SshConfig {
            host: "bastion.example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth: SshAuthMethod::Password,
            local_bind_port: 8108,
        };

        let serialized = toml::to_string(&config).unwrap();
        let deserialized: SshConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized, config);
    }
}
