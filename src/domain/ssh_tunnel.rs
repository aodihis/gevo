use std::fmt;
use std::path::Path;

use zeroize::Zeroizing;

use crate::domain::error::GevoError;
use crate::domain::profile::{SshAuthMethod, SshConfig};

/// What to forward and how to authenticate. `secret` is the SSH password, or the optional
/// key passphrase for `SshAuthMethod::PrivateKey`.
pub struct TunnelRequest<'a> {
    pub ssh: &'a SshConfig,
    pub remote_host: &'a str,
    pub remote_port: u16,
    pub secret: Option<&'a Zeroizing<String>>,
}

impl fmt::Debug for TunnelRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TunnelRequest")
            .field("ssh", &self.ssh)
            .field("remote_host", &self.remote_host)
            .field("remote_port", &self.remote_port)
            .field("secret", &self.secret.map(|_| "<redacted>"))
            .finish()
    }
}

/// An open local port forward; dropping it closes the tunnel.
pub struct TunnelHandle {
    local_port: u16,
    _guard: Box<dyn Send>,
}

impl TunnelHandle {
    pub fn new(local_port: u16, guard: Box<dyn Send>) -> Self {
        Self {
            local_port,
            _guard: guard,
        }
    }

    pub fn local_port(&self) -> u16 {
        self.local_port
    }
}

impl fmt::Debug for TunnelHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TunnelHandle")
            .field("local_port", &self.local_port)
            .finish()
    }
}

/// Opens a local port forward through an SSH server. Blocking: call off the UI thread.
#[cfg_attr(test, mockall::automock)]
pub trait SshTunnel {
    fn open<'a>(&self, request: &TunnelRequest<'a>) -> Result<TunnelHandle, GevoError>;
}

/// Asks the user whether to trust a host key not yet in `known_hosts`. Blocking.
#[cfg_attr(test, mockall::automock)]
pub trait HostKeyPrompt: Send + Sync {
    fn confirm(&self, host: &str, port: u16, key_type: &str, fingerprint_sha256: &str) -> bool;
}

/// How to authenticate, resolved from a request before any network I/O.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AuthPlan<'a> {
    Password(&'a str),
    Key {
        path: &'a Path,
        passphrase: Option<&'a str>,
    },
}

pub(crate) fn plan_auth<'a>(request: &TunnelRequest<'a>) -> Result<AuthPlan<'a>, GevoError> {
    let secret = request.secret.map(|s| s.as_str());
    match &request.ssh.auth {
        SshAuthMethod::Password => secret.map(AuthPlan::Password).ok_or_else(|| {
            GevoError::Tunnel("SSH password is not set for this profile".to_string())
        }),
        SshAuthMethod::PrivateKey { key_path } => Ok(AuthPlan::Key {
            path: key_path,
            passphrase: secret,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn config(auth: SshAuthMethod) -> SshConfig {
        SshConfig {
            host: "bastion.example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth,
            local_bind_port: 8108,
        }
    }

    fn request<'a>(ssh: &'a SshConfig, secret: Option<&'a Zeroizing<String>>) -> TunnelRequest<'a> {
        TunnelRequest {
            ssh,
            remote_host: "127.0.0.1",
            remote_port: 8108,
            secret,
        }
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn debug_redacts_the_secret() {
        let ssh = config(SshAuthMethod::Password);
        let secret = Zeroizing::new("hunter2".to_string());
        let shown = format!("{:?}", request(&ssh, Some(&secret)));
        assert!(!shown.contains("hunter2"));
        assert!(shown.contains("<redacted>"));
        assert!(shown.contains("bastion.example.com"));
    }

    #[test]
    fn debug_shows_no_secret_marker_when_absent() {
        let ssh = config(SshAuthMethod::Password);
        assert!(format!("{:?}", request(&ssh, None)).contains("secret: None"));
    }

    #[test]
    fn handle_exposes_local_port() {
        let handle = TunnelHandle::new(9000, Box::new(()));
        assert_eq!(handle.local_port(), 9000);
        assert!(format!("{handle:?}").contains("9000"));
    }

    #[test]
    fn dropping_handle_drops_its_guard() {
        let closed = Arc::new(AtomicBool::new(false));
        let handle = TunnelHandle::new(9000, Box::new(DropFlag(closed.clone())));
        assert!(!closed.load(Ordering::SeqCst));
        drop(handle);
        assert!(closed.load(Ordering::SeqCst));
    }

    #[test]
    fn mock_tunnel_returns_a_handle() {
        let mut mock = MockSshTunnel::new();
        mock.expect_open()
            .withf(|r| r.remote_port == 8108)
            .returning(|r| Ok(TunnelHandle::new(r.ssh.local_bind_port, Box::new(()))));
        let ssh = config(SshAuthMethod::Password);
        let handle = mock.open(&request(&ssh, None)).unwrap();
        assert_eq!(handle.local_port(), 8108);
    }

    #[test]
    fn mock_tunnel_can_fail_with_a_tunnel_error() {
        let mut mock = MockSshTunnel::new();
        mock.expect_open()
            .returning(|_| Err(GevoError::Tunnel("refused".into())));
        let ssh = config(SshAuthMethod::Password);
        let err = mock.open(&request(&ssh, None)).unwrap_err();
        assert_eq!(err.category(), "TUNNEL");
    }

    #[test]
    fn plan_auth_uses_the_password() {
        let ssh = config(SshAuthMethod::Password);
        let secret = Zeroizing::new("pw".to_string());
        assert_eq!(
            plan_auth(&request(&ssh, Some(&secret))).unwrap(),
            AuthPlan::Password("pw")
        );
    }

    #[test]
    fn plan_auth_rejects_password_auth_without_a_password() {
        let ssh = config(SshAuthMethod::Password);
        let err = plan_auth(&request(&ssh, None)).unwrap_err();
        assert!(matches!(err, GevoError::Tunnel(_)));
    }

    #[test]
    fn plan_auth_passes_key_path_and_optional_passphrase() {
        let key_path = PathBuf::from("/keys/id_ed25519");
        let ssh = config(SshAuthMethod::PrivateKey {
            key_path: key_path.clone(),
        });
        assert_eq!(
            plan_auth(&request(&ssh, None)).unwrap(),
            AuthPlan::Key {
                path: &key_path,
                passphrase: None
            }
        );
        let pass = Zeroizing::new("phrase".to_string());
        assert_eq!(
            plan_auth(&request(&ssh, Some(&pass))).unwrap(),
            AuthPlan::Key {
                path: &key_path,
                passphrase: Some("phrase")
            }
        );
    }
}
