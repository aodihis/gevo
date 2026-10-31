use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::domain::connection_service::{ConnectionService, SdkClientFactory, Session};
use crate::domain::credential_store::KeyringCredentialStore;
use crate::domain::error::GevoError;
use crate::domain::profile::ConnectionProfile;
use crate::domain::russh_tunnel::RusshTunnel;
use crate::domain::ssh_tunnel::HostKeyPrompt;
use crate::domain::typesense_client::SdkTypesenseClient;
use crate::ui::profile_form::ProfileForm;
use crate::ui::task;

pub type LiveSession = Session<SdkTypesenseClient>;

type Connecting = Receiver<Result<LiveSession, GevoError>>;

/// The connection the server screens talk to. Lives in memory only.
#[derive(Default)]
pub enum SessionState {
    #[default]
    None,
    Connecting,
    Ready(LiveSession),
    Failed(GevoError),
}

#[derive(Default)]
pub struct SessionSlot {
    profile: Option<Uuid>,
    state: SessionState,
    pending: Option<Connecting>,
}

impl SessionSlot {
    pub fn profile(&self) -> Option<Uuid> {
        self.profile
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// The live client, once connected.
    pub fn client(&self) -> Option<Arc<SdkTypesenseClient>> {
        match &self.state {
            SessionState::Ready(session) => Some(session.client()),
            _ => None,
        }
    }

    /// Drops the current session (closing its tunnel) and waits for `pending`. A result from an
    /// earlier connect is discarded because its receiver is replaced.
    pub fn begin(&mut self, profile: Uuid, pending: Connecting) {
        self.profile = Some(profile);
        self.state = SessionState::Connecting;
        self.pending = Some(pending);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Collects a finished connect; true while one is still running.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => Err(GevoError::Tunnel(
                "connection worker stopped unexpectedly".to_string(),
            )),
        };
        self.pending = None;
        self.state = match result {
            Ok(session) => SessionState::Ready(session),
            Err(err) => SessionState::Failed(err),
        };
        false
    }
}

/// A profile without a stored admin key cannot be used, so fail before touching the network.
fn require_admin_key(key: &Zeroizing<String>) -> Result<(), GevoError> {
    if key.is_empty() {
        Err(GevoError::Auth(
            "no admin API key is stored for this profile".to_string(),
        ))
    } else {
        Ok(())
    }
}

/// Reads the profile's secrets from the keychain and opens its session on a worker.
pub fn connect(
    profile: ConnectionProfile,
    ctx: egui::Context,
    prompt: Arc<dyn HostKeyPrompt>,
) -> Connecting {
    task::run(ctx, async move {
        let form = ProfileForm::load(&profile, &KeyringCredentialStore::new());
        require_admin_key(&form.admin_key)?;
        let service = ConnectionService::new(RusshTunnel::new(prompt), SdkClientFactory);
        service
            .connect(&profile, &form.admin_key, form.ssh_secret_for_test())
            .await
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;
    use crate::domain::connection_service::ClientFactory;

    fn live() -> LiveSession {
        let client = SdkClientFactory
            .build("http://localhost:8108", "key")
            .unwrap();
        Session::new(client, None)
    }

    #[test]
    fn starts_empty() {
        let slot = SessionSlot::default();
        assert!(matches!(slot.state(), SessionState::None));
        assert!(slot.profile().is_none());
        assert!(slot.client().is_none());
    }

    #[test]
    fn begin_marks_connecting_for_the_profile() {
        let mut slot = SessionSlot::default();
        let id = Uuid::new_v4();
        let (_tx, rx) = channel();
        slot.begin(id, rx);
        assert!(matches!(slot.state(), SessionState::Connecting));
        assert_eq!(slot.profile(), Some(id));
        assert!(slot.poll());
    }

    #[test]
    fn poll_stores_a_ready_session() {
        let mut slot = SessionSlot::default();
        let (tx, rx) = channel();
        slot.begin(Uuid::new_v4(), rx);
        tx.send(Ok(live())).unwrap();
        assert!(!slot.poll());
        assert!(matches!(slot.state(), SessionState::Ready(_)));
        assert!(slot.client().is_some());
    }

    #[test]
    fn poll_stores_a_failure() {
        let mut slot = SessionSlot::default();
        let (tx, rx) = channel();
        slot.begin(Uuid::new_v4(), rx);
        tx.send(Err(GevoError::Tunnel("denied".into()))).unwrap();
        slot.poll();
        assert!(matches!(
            slot.state(),
            SessionState::Failed(GevoError::Tunnel(_))
        ));
        assert!(slot.client().is_none());
    }

    #[test]
    fn a_worker_that_died_becomes_a_failure() {
        let mut slot = SessionSlot::default();
        let (tx, rx) = channel();
        slot.begin(Uuid::new_v4(), rx);
        drop(tx);
        assert!(!slot.poll());
        assert!(matches!(slot.state(), SessionState::Failed(_)));
    }

    #[test]
    fn a_newer_begin_discards_the_older_result() {
        let mut slot = SessionSlot::default();
        let (old_tx, old_rx) = channel();
        slot.begin(Uuid::new_v4(), old_rx);
        let (_new_tx, new_rx) = channel();
        let second = Uuid::new_v4();
        slot.begin(second, new_rx);
        assert!(old_tx.send(Ok(live())).is_err());
        assert!(slot.poll());
        assert_eq!(slot.profile(), Some(second));
        assert!(matches!(slot.state(), SessionState::Connecting));
    }

    #[test]
    fn clear_resets_everything() {
        let mut slot = SessionSlot::default();
        let (_tx, rx) = channel();
        slot.begin(Uuid::new_v4(), rx);
        slot.clear();
        assert!(matches!(slot.state(), SessionState::None));
        assert!(!slot.poll());
        assert!(slot.profile().is_none());
    }

    #[test]
    fn an_empty_admin_key_is_an_auth_error() {
        let err = require_admin_key(&Zeroizing::default()).unwrap_err();
        assert_eq!(err.category(), "AUTH");
        assert!(require_admin_key(&Zeroizing::new("k".to_string())).is_ok());
    }
}
