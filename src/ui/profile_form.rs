use std::path::PathBuf;

use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::domain::connection_service::{
    ClientFactory, ConnectionService, ConnectionTestReport, StepStatus,
};
use crate::domain::credential_store::{CredentialError, CredentialStore, SecretKind};
use crate::domain::error::GevoError;
use crate::domain::profile::{ConnectionProfile, SshAuthMethod, SshConfig};
use crate::domain::profile_store::ProfileStore;
use crate::domain::ssh_tunnel::SshTunnel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Http,
    Https,
}

impl Protocol {
    pub fn scheme(self) -> &'static str {
        match self {
            Protocol::Http => "http",
            Protocol::Https => "https",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshAuthKind {
    Password,
    PrivateKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Host,
    Port,
    AdminKey,
    SshHost,
    SshPort,
    SshUsername,
    KeyPath,
    LocalBindPort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: Field,
    pub message: &'static str,
}

#[derive(Clone)]
/// Editable state behind the connection form. Numeric fields stay text so half-typed input
/// isn't lost; `validate` parses them.
pub struct ProfileForm {
    pub id: Uuid,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: String,
    pub admin_key: Zeroizing<String>,
    pub ssh_enabled: bool,
    pub ssh_host: String,
    pub ssh_port: String,
    pub ssh_username: String,
    pub ssh_auth: SshAuthKind,
    pub key_path: String,
    /// SSH password, or the private-key passphrase.
    pub ssh_secret: Zeroizing<String>,
    pub local_bind_port: String,
}

// Manual so the secrets never reach logs.
impl std::fmt::Debug for ProfileForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileForm")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("host", &self.host)
            .field("admin_key", &"<redacted>")
            .field("ssh_secret", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl ProfileForm {
    pub fn blank() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            protocol: Protocol::Http,
            host: String::new(),
            port: "8108".into(),
            admin_key: Zeroizing::default(),
            ssh_enabled: false,
            ssh_host: String::new(),
            ssh_port: "22".into(),
            ssh_username: String::new(),
            ssh_auth: SshAuthKind::Password,
            key_path: String::new(),
            ssh_secret: Zeroizing::default(),
            local_bind_port: "18108".into(),
        }
    }

    /// Fills the form from a saved profile and its keychain secrets. Missing or unreadable
    /// secrets just leave the fields empty so the user can re-enter them.
    pub fn load(profile: &ConnectionProfile, creds: &impl CredentialStore) -> Self {
        let mut form = Self::blank();
        form.id = profile.id;
        form.name = profile.name.clone();
        if let Ok(url) = Url::parse(&profile.url) {
            form.protocol = if url.scheme() == "https" {
                Protocol::Https
            } else {
                Protocol::Http
            };
            form.host = url
                .host_str()
                .unwrap_or_default()
                .trim_matches(['[', ']'])
                .to_owned();
            if let Some(port) = url.port_or_known_default() {
                form.port = port.to_string();
            }
        }
        form.admin_key = read_secret(creds, profile.id, SecretKind::AdminApiKey);
        if let Some(ssh) = &profile.ssh {
            form.ssh_enabled = true;
            form.ssh_host = ssh.host.clone();
            form.ssh_port = ssh.port.to_string();
            form.ssh_username = ssh.username.clone();
            form.local_bind_port = ssh.local_bind_port.to_string();
            match &ssh.auth {
                SshAuthMethod::Password => form.ssh_auth = SshAuthKind::Password,
                SshAuthMethod::PrivateKey { key_path } => {
                    form.ssh_auth = SshAuthKind::PrivateKey;
                    form.key_path = key_path.to_string_lossy().into_owned();
                }
            }
            form.ssh_secret = read_secret(creds, profile.id, ssh_secret_kind(form.ssh_auth));
        }
        form
    }

    pub fn validate(&self) -> Vec<FieldError> {
        let mut errors = Vec::new();
        let mut require = |ok: bool, field, message| {
            if !ok {
                errors.push(FieldError { field, message });
            }
        };
        require(
            !self.name.trim().is_empty(),
            Field::Name,
            "Name is required",
        );
        require(
            self.url().is_some(),
            Field::Host,
            "Enter a valid host, e.g. localhost",
        );
        require(
            parse_port(&self.port, false).is_some(),
            Field::Port,
            "Port must be 1-65535",
        );
        require(
            !self.admin_key.trim().is_empty(),
            Field::AdminKey,
            "Admin API key is required",
        );
        if self.ssh_enabled {
            require(
                !self.ssh_host.trim().is_empty() && !self.ssh_host.contains(char::is_whitespace),
                Field::SshHost,
                "SSH host is required",
            );
            require(
                parse_port(&self.ssh_port, false).is_some(),
                Field::SshPort,
                "Port must be 1-65535",
            );
            require(
                !self.ssh_username.trim().is_empty(),
                Field::SshUsername,
                "Username is required",
            );
            if self.ssh_auth == SshAuthKind::PrivateKey {
                require(
                    !self.key_path.trim().is_empty(),
                    Field::KeyPath,
                    "Private key file is required",
                );
            }
            // 0 lets the OS pick a free port.
            require(
                parse_port(&self.local_bind_port, true).is_some(),
                Field::LocalBindPort,
                "Port must be 0-65535",
            );
        }
        errors
    }

    /// Maps a valid form to a profile; returns the validation errors otherwise.
    pub fn to_profile(&self) -> Result<ConnectionProfile, Vec<FieldError>> {
        let errors = self.validate();
        if !errors.is_empty() {
            return Err(errors);
        }
        let ssh = self.ssh_enabled.then(|| SshConfig {
            host: self.ssh_host.trim().to_owned(),
            port: parse_port(&self.ssh_port, false).unwrap_or(22),
            username: self.ssh_username.trim().to_owned(),
            auth: match self.ssh_auth {
                SshAuthKind::Password => SshAuthMethod::Password,
                SshAuthKind::PrivateKey => SshAuthMethod::PrivateKey {
                    key_path: PathBuf::from(self.key_path.trim()),
                },
            },
            local_bind_port: parse_port(&self.local_bind_port, true).unwrap_or(0),
        });
        Ok(ConnectionProfile {
            id: self.id,
            name: self.name.trim().to_owned(),
            url: self.url().unwrap_or_default(),
            ssh,
        })
    }

    /// The SSH password/passphrase to hand the tunnel, if one was entered.
    pub fn ssh_secret_for_test(&self) -> Option<&Zeroizing<String>> {
        (self.ssh_enabled && !self.ssh_secret.is_empty()).then_some(&self.ssh_secret)
    }

    fn url(&self) -> Option<String> {
        let host = self.host.trim();
        let port = parse_port(&self.port, false)?;
        if host.is_empty()
            || host.contains(['/', '@', '?', '#'])
            || host.contains(char::is_whitespace)
        {
            return None;
        }
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let url = format!("{}://{host}:{port}", self.protocol.scheme());
        Url::parse(&url).ok()?.host_str()?;
        Some(url)
    }
}

fn parse_port(text: &str, allow_zero: bool) -> Option<u16> {
    let port: u16 = text.trim().parse().ok()?;
    (allow_zero || port != 0).then_some(port)
}

fn ssh_secret_kind(auth: SshAuthKind) -> SecretKind {
    match auth {
        SshAuthKind::Password => SecretKind::SshPassword,
        SshAuthKind::PrivateKey => SecretKind::SshKeyPassphrase,
    }
}

fn read_secret(creds: &impl CredentialStore, id: Uuid, kind: SecretKind) -> Zeroizing<String> {
    match creds.get(id, kind) {
        Ok(Some(secret)) => secret,
        Ok(None) => Zeroizing::default(),
        Err(err) => {
            tracing::warn!(category = "AUTH", %err, "could not read secret from keychain");
            Zeroizing::default()
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("the form has invalid fields")]
    Invalid(Vec<FieldError>),
    #[error(transparent)]
    Credentials(#[from] CredentialError),
    #[error("could not write profiles: {0}")]
    Store(#[from] std::io::Error),
}

/// Validates, writes secrets to the keychain, then upserts the profile. Secrets go first so a
/// keychain failure never leaves a profile without its key; nothing falls back to plain text.
pub fn save(
    form: &ProfileForm,
    store: &impl ProfileStore,
    creds: &impl CredentialStore,
) -> Result<ConnectionProfile, SaveError> {
    let profile = form.to_profile().map_err(SaveError::Invalid)?;
    creds.set(profile.id, SecretKind::AdminApiKey, &form.admin_key)?;

    let wanted = profile.ssh.as_ref().map(|_| ssh_secret_kind(form.ssh_auth));
    for kind in [SecretKind::SshPassword, SecretKind::SshKeyPassphrase] {
        if Some(kind) == wanted && !form.ssh_secret.is_empty() {
            creds.set(profile.id, kind, &form.ssh_secret)?;
        } else {
            creds.delete(profile.id, kind)?;
        }
    }

    let mut profiles = store.load_all();
    match profiles.iter_mut().find(|p| p.id == profile.id) {
        Some(existing) => *existing = profile.clone(),
        None => profiles.push(profile.clone()),
    }
    store.save_all(&profiles)?;
    Ok(profile)
}

/// Runs the stepwise connection test for the form's current (unsaved) values.
pub async fn run_test<T, F>(
    service: &ConnectionService<T, F>,
    form: &ProfileForm,
) -> Result<ConnectionTestReport, Vec<FieldError>>
where
    T: SshTunnel + Send + Sync + 'static,
    F: ClientFactory,
{
    let profile = form.to_profile()?;
    Ok(service
        .test(&profile, &form.admin_key, form.ssh_secret_for_test())
        .await)
}

/// Folds a save result into the screen's error state; true when the save succeeded.
pub fn record_save(
    result: Result<ConnectionProfile, SaveError>,
    errors: &mut Vec<FieldError>,
    save_error: &mut Option<String>,
) -> bool {
    *save_error = None;
    match result {
        Ok(profile) => {
            tracing::info!(profile = %profile.name, "profile saved");
            true
        }
        Err(SaveError::Invalid(found)) => {
            *errors = found;
            false
        }
        Err(err) => {
            tracing::error!(category = "AUTH", %err, "failed to save profile");
            *save_error = Some(err.to_string());
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    SshHost,
    SshAuth,
    PortForward,
    Health,
    AdminKey,
}

impl WizardStep {
    pub const ALL: [WizardStep; 5] = [
        WizardStep::SshHost,
        WizardStep::SshAuth,
        WizardStep::PortForward,
        WizardStep::Health,
        WizardStep::AdminKey,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WizardStep::SshHost => "Reach SSH host",
            WizardStep::SshAuth => "SSH authentication",
            WizardStep::PortForward => "Open port forward",
            WizardStep::Health => "GET /health",
            WizardStep::AdminKey => "Verify admin key",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowState {
    Pending,
    Running,
    Ok,
    Skipped,
    NotRun,
    Failed(GevoError),
}

/// Where the connection test is, for the "Connection test" card.
pub enum TestProgress {
    Idle,
    Running,
    Done(ConnectionTestReport),
}

/// One row state per `WizardStep::ALL`. The service reports tunnel setup as a single step, so
/// a tunnel failure is shown on the first SSH row and the rest of the wizard as not run.
pub fn wizard_rows(progress: &TestProgress) -> [RowState; 5] {
    let report = match progress {
        TestProgress::Idle => return std::array::from_fn(|_| RowState::Pending),
        TestProgress::Running => return std::array::from_fn(|_| RowState::Running),
        TestProgress::Done(report) => report,
    };
    let tunnel_rows = match &report.tunnel {
        StepStatus::Ok => [RowState::Ok, RowState::Ok, RowState::Ok],
        StepStatus::Skipped => [RowState::Skipped, RowState::Skipped, RowState::Skipped],
        StepStatus::NotRun => [RowState::NotRun, RowState::NotRun, RowState::NotRun],
        StepStatus::Failed(e) => [
            RowState::Failed(e.clone()),
            RowState::NotRun,
            RowState::NotRun,
        ],
    };
    let [host, auth, forward] = tunnel_rows;
    [host, auth, forward, row(&report.health), row(&report.auth)]
}

fn row(status: &StepStatus) -> RowState {
    match status {
        StepStatus::Ok => RowState::Ok,
        StepStatus::Skipped => RowState::Skipped,
        StepStatus::NotRun => RowState::NotRun,
        StepStatus::Failed(e) => RowState::Failed(e.clone()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;
    use crate::domain::error::TypesenseMsg;

    #[derive(Default)]
    struct FakeCreds {
        secrets: RefCell<HashMap<(Uuid, &'static str), String>>,
        fail_set: bool,
    }

    fn tag(kind: SecretKind) -> &'static str {
        match kind {
            SecretKind::AdminApiKey => "admin",
            SecretKind::SshPassword => "pw",
            SecretKind::SshKeyPassphrase => "pass",
        }
    }

    impl CredentialStore for FakeCreds {
        fn set(
            &self,
            id: Uuid,
            kind: SecretKind,
            v: &Zeroizing<String>,
        ) -> Result<(), CredentialError> {
            if self.fail_set {
                return Err(CredentialError::Unavailable("no keyring".into()));
            }
            self.secrets
                .borrow_mut()
                .insert((id, tag(kind)), v.to_string());
            Ok(())
        }
        fn get(
            &self,
            id: Uuid,
            kind: SecretKind,
        ) -> Result<Option<Zeroizing<String>>, CredentialError> {
            Ok(self
                .secrets
                .borrow()
                .get(&(id, tag(kind)))
                .cloned()
                .map(Zeroizing::new))
        }
        fn delete(&self, id: Uuid, kind: SecretKind) -> Result<(), CredentialError> {
            self.secrets.borrow_mut().remove(&(id, tag(kind)));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeStore {
        profiles: RefCell<Vec<ConnectionProfile>>,
    }

    impl ProfileStore for FakeStore {
        fn load_all(&self) -> Vec<ConnectionProfile> {
            self.profiles.borrow().clone()
        }
        fn save_all(&self, profiles: &[ConnectionProfile]) -> std::io::Result<()> {
            *self.profiles.borrow_mut() = profiles.to_vec();
            Ok(())
        }
    }

    fn valid() -> ProfileForm {
        let mut f = ProfileForm::blank();
        f.name = "Prod".into();
        f.host = "ts.example.com".into();
        f.admin_key = Zeroizing::new("xyz".into());
        f
    }

    fn valid_ssh() -> ProfileForm {
        let mut f = valid();
        f.ssh_enabled = true;
        f.ssh_host = "jump".into();
        f.ssh_username = "deploy".into();
        f.ssh_secret = Zeroizing::new("hunter2".into());
        f
    }

    #[tokio::test]
    async fn run_test_passes_form_values_to_the_service() {
        use crate::domain::connection_service::MockClientFactory;
        use crate::domain::ssh_tunnel::{MockSshTunnel, TunnelHandle};
        use crate::domain::typesense_client::MockTypesenseClient;

        let mut tunnel = MockSshTunnel::new();
        tunnel.expect_open().returning(|r| {
            assert_eq!(r.secret.map(|s| s.as_str()), Some("hunter2"));
            Ok(TunnelHandle::new(4242, Box::new(())))
        });
        let mut clients = MockClientFactory::new();
        clients
            .expect_build_via_tunnel()
            .withf(|url, key, port| {
                url == "http://ts.example.com:8108" && key == "xyz" && *port == 4242
            })
            .returning(|_, _, _| {
                let mut c = MockTypesenseClient::new();
                c.expect_health().returning(|| Ok(true));
                c.expect_check_auth().returning(|| Ok(()));
                Ok(c)
            });
        let service = ConnectionService::new(tunnel, clients);
        let report = run_test(&service, &valid_ssh()).await.unwrap();
        assert!(report.succeeded());
    }

    #[tokio::test]
    async fn run_test_rejects_invalid_form_without_touching_the_network() {
        use crate::domain::connection_service::MockClientFactory;
        use crate::domain::ssh_tunnel::MockSshTunnel;

        let service = ConnectionService::new(MockSshTunnel::new(), MockClientFactory::new());
        let errors = run_test(&service, &ProfileForm::blank()).await.unwrap_err();
        assert!(!errors.is_empty());
    }

    #[test]
    fn record_save_clears_a_stale_save_error_on_the_next_attempt() {
        let mut errors = Vec::new();
        let mut save_error = None;
        let io = SaveError::Store(std::io::Error::other("disk full"));
        assert!(!record_save(Err(io), &mut errors, &mut save_error));
        assert!(save_error.is_some());

        let invalid = SaveError::Invalid(vec![FieldError {
            field: Field::Name,
            message: "x",
        }]);
        assert!(!record_save(Err(invalid), &mut errors, &mut save_error));
        assert_eq!(save_error, None);
        assert_eq!(errors.len(), 1);

        let io = SaveError::Store(std::io::Error::other("disk full"));
        record_save(Err(io), &mut errors, &mut save_error);
        let ok = Ok(valid().to_profile().unwrap());
        assert!(record_save(ok, &mut errors, &mut save_error));
        assert_eq!(save_error, None);
    }

    fn fields(errors: &[FieldError]) -> Vec<Field> {
        errors.iter().map(|e| e.field).collect()
    }

    #[test]
    fn blank_form_has_design_defaults_and_requires_input() {
        let f = ProfileForm::blank();
        assert_eq!(
            (
                f.port.as_str(),
                f.ssh_port.as_str(),
                f.local_bind_port.as_str()
            ),
            ("8108", "22", "18108")
        );
        assert_eq!(
            fields(&f.validate()),
            vec![Field::Name, Field::Host, Field::AdminKey]
        );
    }

    #[test]
    fn valid_form_has_no_errors_and_maps_to_profile_url() {
        let p = valid().to_profile().unwrap();
        assert_eq!(p.url, "http://ts.example.com:8108");
        assert_eq!(p.name, "Prod");
        assert_eq!(p.ssh, None);
    }

    #[test]
    fn https_protocol_and_ipv6_host_build_valid_url() {
        let mut f = valid();
        f.protocol = Protocol::Https;
        f.host = "::1".into();
        assert_eq!(f.to_profile().unwrap().url, "https://[::1]:8108");
    }

    #[test]
    fn invalid_hosts_and_ports_are_rejected() {
        for host in ["", "  ", "a b", "http://x", "x/y", "u@x"] {
            let mut f = valid();
            f.host = host.into();
            assert_eq!(fields(&f.validate()), vec![Field::Host], "host {host:?}");
        }
        for port in ["0", "70000", "abc", ""] {
            let mut f = valid();
            f.port = port.into();
            assert!(
                fields(&f.validate()).contains(&Field::Port),
                "port {port:?}"
            );
        }
    }

    #[test]
    fn whitespace_only_name_and_key_are_rejected() {
        let mut f = valid();
        f.name = "  ".into();
        f.admin_key = Zeroizing::new(" ".into());
        assert_eq!(fields(&f.validate()), vec![Field::Name, Field::AdminKey]);
    }

    #[test]
    fn ssh_fields_are_ignored_while_ssh_is_disabled() {
        let mut f = valid();
        f.ssh_port = "bad".into();
        assert!(f.validate().is_empty());
    }

    #[test]
    fn ssh_fields_are_validated_when_enabled() {
        let mut f = valid();
        f.ssh_enabled = true;
        f.ssh_port = "0".into();
        f.local_bind_port = "x".into();
        f.ssh_auth = SshAuthKind::PrivateKey;
        assert_eq!(
            fields(&f.validate()),
            vec![
                Field::SshHost,
                Field::SshPort,
                Field::SshUsername,
                Field::KeyPath,
                Field::LocalBindPort
            ]
        );
    }

    #[test]
    fn local_bind_port_zero_is_allowed() {
        let mut f = valid_ssh();
        f.local_bind_port = "0".into();
        assert_eq!(f.to_profile().unwrap().ssh.unwrap().local_bind_port, 0);
    }

    #[test]
    fn password_and_key_auth_map_to_ssh_config() {
        let ssh = valid_ssh().to_profile().unwrap().ssh.unwrap();
        assert_eq!(
            (ssh.host.as_str(), ssh.port, ssh.local_bind_port),
            ("jump", 22, 18108)
        );
        assert_eq!(ssh.auth, SshAuthMethod::Password);

        let mut f = valid_ssh();
        f.ssh_auth = SshAuthKind::PrivateKey;
        f.key_path = " /home/u/.ssh/id ".into();
        let ssh = f.to_profile().unwrap().ssh.unwrap();
        assert_eq!(
            ssh.auth,
            SshAuthMethod::PrivateKey {
                key_path: "/home/u/.ssh/id".into()
            }
        );
    }

    #[test]
    fn to_profile_returns_errors_for_invalid_form() {
        assert!(ProfileForm::blank().to_profile().is_err());
    }

    #[test]
    fn ssh_secret_for_test_requires_ssh_and_a_value() {
        let mut f = valid_ssh();
        assert!(f.ssh_secret_for_test().is_some());
        f.ssh_secret = Zeroizing::default();
        assert!(f.ssh_secret_for_test().is_none());
        let mut f = valid_ssh();
        f.ssh_enabled = false;
        assert!(f.ssh_secret_for_test().is_none());
    }

    #[test]
    fn debug_output_redacts_secrets() {
        let out = format!("{:?}", valid_ssh());
        assert!(!out.contains("xyz") && !out.contains("hunter2"));
        assert!(out.contains("<redacted>"));
    }

    #[test]
    fn save_writes_secrets_to_keychain_and_profile_to_store() {
        let (store, creds) = (FakeStore::default(), FakeCreds::default());
        let f = valid_ssh();
        let p = save(&f, &store, &creds).unwrap();
        assert_eq!(store.load_all(), vec![p.clone()]);
        let admin = creds.get(p.id, SecretKind::AdminApiKey).unwrap().unwrap();
        assert_eq!(admin.as_str(), "xyz");
        let pw = creds.get(p.id, SecretKind::SshPassword).unwrap().unwrap();
        assert_eq!(pw.as_str(), "hunter2");
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("xyz") && !json.contains("hunter2"));
    }

    #[test]
    fn save_updates_existing_profile_in_place() {
        let (store, creds) = (FakeStore::default(), FakeCreds::default());
        let mut f = valid();
        save(&f, &store, &creds).unwrap();
        f.name = "Renamed".into();
        save(&f, &store, &creds).unwrap();
        let all = store.load_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Renamed");
    }

    #[test]
    fn save_removes_ssh_secrets_when_tunnel_is_disabled_or_auth_switches() {
        let (store, creds) = (FakeStore::default(), FakeCreds::default());
        let mut f = valid_ssh();
        let id = save(&f, &store, &creds).unwrap().id;
        f.ssh_auth = SshAuthKind::PrivateKey;
        f.key_path = "/k".into();
        f.ssh_secret = Zeroizing::default();
        save(&f, &store, &creds).unwrap();
        assert!(creds.get(id, SecretKind::SshPassword).unwrap().is_none());
        assert!(
            creds
                .get(id, SecretKind::SshKeyPassphrase)
                .unwrap()
                .is_none()
        );

        f.ssh_enabled = false;
        f.ssh_secret = Zeroizing::new("p".into());
        save(&f, &store, &creds).unwrap();
        assert!(
            creds
                .get(id, SecretKind::SshKeyPassphrase)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn save_rejects_invalid_form_without_side_effects() {
        let (store, creds) = (FakeStore::default(), FakeCreds::default());
        let err = save(&ProfileForm::blank(), &store, &creds).unwrap_err();
        assert!(matches!(err, SaveError::Invalid(_)));
        assert!(store.load_all().is_empty());
        assert!(creds.secrets.borrow().is_empty());
    }

    #[test]
    fn save_does_not_write_profile_when_keychain_fails() {
        let store = FakeStore::default();
        let creds = FakeCreds {
            fail_set: true,
            ..Default::default()
        };
        let err = save(&valid(), &store, &creds).unwrap_err();
        assert!(matches!(
            err,
            SaveError::Credentials(CredentialError::Unavailable(_))
        ));
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn load_round_trips_a_saved_profile_with_secrets() {
        let (store, creds) = (FakeStore::default(), FakeCreds::default());
        let mut f = valid_ssh();
        f.protocol = Protocol::Https;
        f.ssh_auth = SshAuthKind::PrivateKey;
        f.key_path = "/k".into();
        f.ssh_secret = Zeroizing::new("phrase".into());
        let p = save(&f, &store, &creds).unwrap();

        let loaded = ProfileForm::load(&p, &creds);
        assert_eq!(loaded.id, p.id);
        assert_eq!(loaded.protocol, Protocol::Https);
        assert_eq!(
            (loaded.host.as_str(), loaded.port.as_str()),
            ("ts.example.com", "8108")
        );
        assert_eq!(loaded.admin_key.as_str(), "xyz");
        assert_eq!(loaded.ssh_auth, SshAuthKind::PrivateKey);
        assert_eq!(loaded.ssh_secret.as_str(), "phrase");
        assert_eq!(loaded.to_profile().unwrap(), p);
    }

    #[test]
    fn load_without_stored_secrets_leaves_them_empty() {
        let p = ConnectionProfile::new("A", "http://[::1]:9");
        let f = ProfileForm::load(&p, &FakeCreds::default());
        assert!(f.admin_key.is_empty());
        assert_eq!(f.host, "::1");
        assert!(!f.ssh_enabled);
    }

    fn report(tunnel: StepStatus, health: StepStatus, auth: StepStatus) -> TestProgress {
        TestProgress::Done(ConnectionTestReport {
            tunnel,
            health,
            auth,
        })
    }

    #[test]
    fn idle_and_running_progress_fill_every_row() {
        assert!(
            wizard_rows(&TestProgress::Idle)
                .iter()
                .all(|r| *r == RowState::Pending)
        );
        assert!(
            wizard_rows(&TestProgress::Running)
                .iter()
                .all(|r| *r == RowState::Running)
        );
    }

    #[test]
    fn successful_ssh_report_is_all_ok() {
        let rows = wizard_rows(&report(StepStatus::Ok, StepStatus::Ok, StepStatus::Ok));
        assert!(rows.iter().all(|r| *r == RowState::Ok));
    }

    #[test]
    fn report_without_ssh_skips_the_three_ssh_rows() {
        let rows = wizard_rows(&report(StepStatus::Skipped, StepStatus::Ok, StepStatus::Ok));
        assert_eq!(
            rows[..3],
            [RowState::Skipped, RowState::Skipped, RowState::Skipped]
        );
        assert_eq!(rows[3..], [RowState::Ok, RowState::Ok]);
    }

    #[test]
    fn tunnel_failure_marks_first_row_failed_and_rest_not_run() {
        let e = GevoError::Tunnel("auth failed".into());
        let rows = wizard_rows(&report(
            StepStatus::Failed(e.clone()),
            StepStatus::NotRun,
            StepStatus::NotRun,
        ));
        assert_eq!(rows[0], RowState::Failed(e));
        assert!(rows[1..].iter().all(|r| *r == RowState::NotRun));
    }

    #[test]
    fn auth_failure_keeps_earlier_rows_ok_and_carries_message() {
        let e = GevoError::Api(TypesenseMsg(
            "Forbidden - a valid `x-typesense-api-key` header must be sent.".into(),
        ));
        let rows = wizard_rows(&report(
            StepStatus::Ok,
            StepStatus::Ok,
            StepStatus::Failed(e.clone()),
        ));
        assert_eq!(rows[3], RowState::Ok);
        assert_eq!(rows[4], RowState::Failed(e));
    }

    #[test]
    fn health_failure_leaves_admin_key_not_run() {
        let e = GevoError::Network("refused".into());
        let rows = wizard_rows(&report(
            StepStatus::Skipped,
            StepStatus::Failed(e.clone()),
            StepStatus::NotRun,
        ));
        assert_eq!(rows[3], RowState::Failed(e));
        assert_eq!(rows[4], RowState::NotRun);
    }

    #[test]
    fn every_wizard_step_has_a_label() {
        assert!(WizardStep::ALL.iter().all(|s| !s.label().is_empty()));
        assert_eq!(Protocol::Https.scheme(), "https");
    }
}
