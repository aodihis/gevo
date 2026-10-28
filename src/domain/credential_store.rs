use uuid::Uuid;
use zeroize::Zeroizing;

const SERVICE: &str = "gevo";

/// Which secret a `CredentialStore` entry holds, for a given `ConnectionProfile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    AdminApiKey,
    SshPassword,
    SshKeyPassphrase,
}

impl SecretKind {
    fn label(self) -> &'static str {
        match self {
            SecretKind::AdminApiKey => "admin_api_key",
            SecretKind::SshPassword => "ssh_password",
            SecretKind::SshKeyPassphrase => "ssh_key_passphrase",
        }
    }
}

/// Per `0002-v1-requirements.md` §1: when the OS keychain isn't available, callers must
/// surface a clear error and refuse to persist secrets -- never fall back to plain text.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("OS keyring not available: {0}")]
    Unavailable(String),
    #[error("credential store error: {0}")]
    Other(String),
}

/// Stores/retrieves secrets (admin API key, SSH password/passphrase) in the OS-native
/// credential store, keyed by `ConnectionProfile::id`. Never persists secrets to a plain-text
/// config file (see `AGENTS.md` §9 / `0002-v1-requirements.md` §1).
pub trait CredentialStore {
    fn set(
        &self,
        profile_id: Uuid,
        kind: SecretKind,
        value: &Zeroizing<String>,
    ) -> Result<(), CredentialError>;

    fn get(
        &self,
        profile_id: Uuid,
        kind: SecretKind,
    ) -> Result<Option<Zeroizing<String>>, CredentialError>;

    fn delete(&self, profile_id: Uuid, kind: SecretKind) -> Result<(), CredentialError>;
}

fn account(profile_id: Uuid, kind: SecretKind) -> String {
    format!("{profile_id}:{}", kind.label())
}

fn map_err(err: keyring::Error) -> CredentialError {
    match err {
        keyring::Error::NoStorageAccess(_) | keyring::Error::NoDefaultStore => {
            CredentialError::Unavailable(err.to_string())
        }
        other => CredentialError::Other(other.to_string()),
    }
}

/// `CredentialStore` backed by the OS-native keychain (Windows Credential Manager, macOS
/// Keychain, Linux Secret Service), via the `keyring` crate.
#[derive(Default)]
pub struct KeyringCredentialStore;

impl KeyringCredentialStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(profile_id: Uuid, kind: SecretKind) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(SERVICE, &account(profile_id, kind)).map_err(map_err)
    }
}

impl CredentialStore for KeyringCredentialStore {
    fn set(
        &self,
        profile_id: Uuid,
        kind: SecretKind,
        value: &Zeroizing<String>,
    ) -> Result<(), CredentialError> {
        Self::entry(profile_id, kind)?
            .set_password(value)
            .map_err(map_err)
    }

    fn get(
        &self,
        profile_id: Uuid,
        kind: SecretKind,
    ) -> Result<Option<Zeroizing<String>>, CredentialError> {
        match Self::entry(profile_id, kind)?.get_password() {
            Ok(password) => Ok(Some(Zeroizing::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_err(err)),
        }
    }

    fn delete(&self, profile_id: Uuid, kind: SecretKind) -> Result<(), CredentialError> {
        match Self::entry(profile_id, kind)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_err(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_combines_profile_id_and_kind_label() {
        let id = Uuid::nil();
        assert_eq!(
            account(id, SecretKind::AdminApiKey),
            format!("{id}:admin_api_key")
        );
        assert_eq!(
            account(id, SecretKind::SshPassword),
            format!("{id}:ssh_password")
        );
        assert_eq!(
            account(id, SecretKind::SshKeyPassphrase),
            format!("{id}:ssh_key_passphrase")
        );
    }

    #[test]
    fn map_err_classifies_no_default_store_as_unavailable() {
        let err = map_err(keyring::Error::NoDefaultStore);
        assert!(matches!(err, CredentialError::Unavailable(_)));
    }

    #[test]
    fn map_err_classifies_no_entry_as_other() {
        let err = map_err(keyring::Error::NoEntry);
        assert!(matches!(err, CredentialError::Other(_)));
    }

    // Exercises the real OS keychain. Excluded from the default `cargo test` run per
    // AGENTS.md §2 -- CI runners (notably Linux) commonly have no Secret Service running.
    #[test]
    #[ignore = "touches the real OS keychain; run manually with cargo test -- --ignored"]
    fn round_trips_through_the_real_keychain() {
        let store = KeyringCredentialStore::new();
        let id = Uuid::new_v4();

        store
            .set(
                id,
                SecretKind::AdminApiKey,
                &Zeroizing::new("secret-value".to_string()),
            )
            .unwrap();
        assert_eq!(
            store
                .get(id, SecretKind::AdminApiKey)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("secret-value")
        );

        store.delete(id, SecretKind::AdminApiKey).unwrap();
        assert_eq!(store.get(id, SecretKind::AdminApiKey).unwrap(), None);
    }
}
