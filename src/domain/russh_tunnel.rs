use std::net::TcpListener as StdTcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{
    HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate,
    check_known_hosts_path, known_hosts::learn_known_hosts_path, load_secret_key,
};
use tokio::io::copy_bidirectional;
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};

use crate::domain::error::GevoError;
use crate::domain::ssh_tunnel::{
    AuthPlan, HostKeyPrompt, SshTunnel, TunnelHandle, TunnelRequest, plan_auth,
};

fn tunnel_err(context: &str, err: impl std::fmt::Display) -> GevoError {
    GevoError::Tunnel(format!("{context}: {err}"))
}

/// Accepts a key already in `known_hosts`; records an unseen one only if the user confirms;
/// rejects a key that differs from the recorded one without prompting.
fn verify_host_key(
    host: &str,
    port: u16,
    key: &PublicKey,
    known_hosts: &Path,
    prompt: &dyn HostKeyPrompt,
) -> Result<(), GevoError> {
    let known = match check_known_hosts_path(host, port, key, known_hosts) {
        Ok(known) => known,
        Err(russh::keys::Error::KeyChanged { line }) => {
            return Err(GevoError::Tunnel(format!(
                "SSH host key for {host}:{port} changed (known_hosts line {line}); refusing to connect"
            )));
        }
        Err(err) => return Err(tunnel_err("cannot read known_hosts", err)),
    };
    if known {
        return Ok(());
    }
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    if !prompt.confirm(host, port, key.algorithm().as_str(), &fingerprint) {
        return Err(GevoError::Tunnel(format!(
            "SSH host key for {host}:{port} was rejected by the user"
        )));
    }
    learn_known_hosts_path(host, port, key, known_hosts)
        .map_err(|err| tunnel_err("cannot record SSH host key", err))
}

fn known_hosts_path() -> Result<PathBuf, GevoError> {
    dirs::home_dir()
        .map(|home| home.join(".ssh").join("known_hosts"))
        .ok_or_else(|| GevoError::Tunnel("cannot locate the home directory".to_string()))
}

fn bind_local(port: u16) -> Result<TcpListener, GevoError> {
    let std_listener = StdTcpListener::bind(("127.0.0.1", port))
        .map_err(|err| tunnel_err(&format!("cannot bind local port {port}"), err))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|err| tunnel_err("cannot configure local listener", err))?;
    TcpListener::from_std(std_listener).map_err(|err| tunnel_err("cannot use local listener", err))
}

fn load_key(path: &Path, passphrase: Option<&str>) -> Result<PrivateKey, GevoError> {
    load_secret_key(path, passphrase)
        .map_err(|err| tunnel_err(&format!("cannot load SSH key {}", path.display()), err))
}

struct Client {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    prompt: Arc<dyn HostKeyPrompt>,
}

/// Carries the host-key verdict out of the handler, which russh would otherwise flatten.
#[derive(Debug)]
enum HandlerError {
    Russh(russh::Error),
    HostKey(GevoError),
}

impl From<russh::Error> for HandlerError {
    fn from(err: russh::Error) -> Self {
        Self::Russh(err)
    }
}

impl client::Handler for Client {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_key else {
            return Ok(false);
        };
        verify_host_key(
            &self.host,
            self.port,
            key,
            &self.known_hosts,
            self.prompt.as_ref(),
        )
        .map(|()| true)
        .map_err(|err| {
            tracing::error!(category = err.category(), %err, "SSH host key rejected");
            HandlerError::HostKey(err)
        })
    }
}

async fn authenticate(
    session: &mut Handle<Client>,
    username: &str,
    plan: AuthPlan<'_>,
) -> Result<(), GevoError> {
    let auth_err = |err: russh::Error| tunnel_err("SSH authentication failed", err);
    let result = match plan {
        AuthPlan::Password(password) => session
            .authenticate_password(username, password)
            .await
            .map_err(auth_err)?,
        AuthPlan::Key { path, passphrase } => {
            let key = load_key(path, passphrase)?;
            let hash_alg = session
                .best_supported_rsa_hash()
                .await
                .map_err(auth_err)?
                .flatten();
            session
                .authenticate_publickey(
                    username,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                )
                .await
                .map_err(auth_err)?
        }
    };
    if result.success() {
        Ok(())
    } else {
        Err(GevoError::Tunnel(
            "SSH authentication rejected by the server".to_string(),
        ))
    }
}

async fn forward_connections(
    listener: TcpListener,
    session: Arc<Handle<Client>>,
    remote_host: Arc<str>,
    remote_port: u16,
) {
    while let Ok((mut socket, peer)) = listener.accept().await {
        let session = Arc::clone(&session);
        let remote_host = Arc::clone(&remote_host);
        tokio::spawn(async move {
            let channel = session
                .channel_open_direct_tcpip(
                    &*remote_host,
                    u32::from(remote_port),
                    peer.ip().to_string(),
                    u32::from(peer.port()),
                )
                .await;
            match channel {
                Ok(channel) => {
                    let mut stream = channel.into_stream();
                    let _ = copy_bidirectional(&mut socket, &mut stream).await;
                }
                Err(err) => {
                    tracing::warn!(category = "TUNNEL", %err, "port forward channel refused");
                }
            }
        });
    }
}

async fn open_forward(
    request: &TunnelRequest<'_>,
    prompt: Arc<dyn HostKeyPrompt>,
    plan: AuthPlan<'_>,
    listener: TcpListener,
) -> Result<(), GevoError> {
    let ssh = request.ssh;
    let handler = Client {
        host: ssh.host.clone(),
        port: ssh.port,
        known_hosts: known_hosts_path()?,
        prompt,
    };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let mut session = client::connect(config, (ssh.host.as_str(), ssh.port), handler)
        .await
        .map_err(|err| match err {
            HandlerError::HostKey(err) => err,
            HandlerError::Russh(other) => tunnel_err("cannot connect to SSH host", other),
        })?;
    authenticate(&mut session, &ssh.username, plan).await?;

    tokio::spawn(forward_connections(
        listener,
        Arc::new(session),
        Arc::from(request.remote_host),
        request.remote_port,
    ));
    Ok(())
}

/// Owns the runtime driving the forward; dropping it aborts the tasks and closes the session.
struct RuntimeGuard(Option<Runtime>);

impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

/// `SshTunnel` backed by `russh` direct-tcpip channels.
pub struct RusshTunnel {
    prompt: Arc<dyn HostKeyPrompt>,
}

impl RusshTunnel {
    pub fn new(prompt: Arc<dyn HostKeyPrompt>) -> Self {
        Self { prompt }
    }
}

impl SshTunnel for RusshTunnel {
    fn open<'a>(&self, request: &TunnelRequest<'a>) -> Result<TunnelHandle, GevoError> {
        let plan = plan_auth(request)?;
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .enable_time()
            .build()
            .map_err(|err| tunnel_err("cannot start tunnel runtime", err))?;

        // Entered so the listener registers with this runtime's reactor.
        let enter = runtime.enter();
        let listener = bind_local(request.ssh.local_bind_port)?;
        let local_port = listener
            .local_addr()
            .map_err(|err| tunnel_err("cannot read local port", err))?
            .port();
        drop(enter);

        runtime.block_on(open_forward(
            request,
            Arc::clone(&self.prompt),
            plan,
            listener,
        ))?;
        Ok(TunnelHandle::new(
            local_port,
            Box::new(RuntimeGuard(Some(runtime))),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpStream;

    use russh::keys::ssh_key::public::{Ed25519PublicKey, KeyData};
    use zeroize::Zeroizing;

    use super::*;
    use crate::domain::profile::{SshAuthMethod, SshConfig};
    use crate::domain::ssh_tunnel::MockHostKeyPrompt;

    fn ed25519(byte: u8) -> PublicKey {
        PublicKey::from(KeyData::Ed25519(Ed25519PublicKey([byte; 32])))
    }

    fn known_hosts_with(key: &PublicKey) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        learn_known_hosts_path("example.com", 22, key, &path).unwrap();
        (dir, path)
    }

    fn never_prompted() -> MockHostKeyPrompt {
        let mut prompt = MockHostKeyPrompt::new();
        prompt.expect_confirm().never();
        prompt
    }

    #[test]
    fn unknown_host_key_accepted_by_user_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let key = ed25519(1);
        let expected = key.fingerprint(HashAlg::Sha256).to_string();
        let mut prompt = MockHostKeyPrompt::new();
        prompt
            .expect_confirm()
            .withf(move |host, port, key_type, fp| {
                host == "example.com"
                    && *port == 22
                    && key_type == "ssh-ed25519"
                    && fp == expected
                    && fp.starts_with("SHA256:")
            })
            .once()
            .returning(|_, _, _, _| true);

        verify_host_key("example.com", 22, &key, &path, &prompt).unwrap();

        assert!(check_known_hosts_path("example.com", 22, &key, &path).unwrap());
    }

    #[test]
    fn unknown_host_key_rejected_by_user_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        let mut prompt = MockHostKeyPrompt::new();
        prompt.expect_confirm().once().returning(|_, _, _, _| false);

        let err = verify_host_key("example.com", 22, &ed25519(1), &path, &prompt).unwrap_err();

        assert!(matches!(&err, GevoError::Tunnel(msg) if msg.contains("rejected by the user")));
        assert!(!path.exists());
    }

    #[test]
    fn known_matching_host_key_does_not_prompt() {
        let key = ed25519(1);
        let (_dir, path) = known_hosts_with(&key);

        verify_host_key("example.com", 22, &key, &path, &never_prompted()).unwrap();
    }

    #[test]
    fn known_changed_host_key_is_rejected_without_prompting() {
        let (_dir, path) = known_hosts_with(&ed25519(1));

        let err =
            verify_host_key("example.com", 22, &ed25519(2), &path, &never_prompted()).unwrap_err();

        assert!(
            matches!(&err, GevoError::Tunnel(msg) if msg.contains("example.com") && msg.contains("changed"))
        );
    }

    #[tokio::test]
    async fn bind_local_picks_a_free_port_when_given_zero() {
        let listener = bind_local(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
    }

    #[tokio::test]
    async fn bind_local_fails_when_the_port_is_taken() {
        let taken = StdTcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();

        let err = bind_local(port).unwrap_err();

        assert!(matches!(err, GevoError::Tunnel(_)));
    }

    #[test]
    fn load_key_reports_a_missing_file() {
        let err = load_key(Path::new("/definitely/not/here"), None).unwrap_err();
        assert!(matches!(&err, GevoError::Tunnel(msg) if msg.contains("cannot load SSH key")));
    }

    #[test]
    fn open_without_a_password_fails_before_touching_the_network() {
        let ssh = SshConfig {
            host: "127.0.0.1".into(),
            port: 22,
            username: "deploy".into(),
            auth: SshAuthMethod::Password,
            local_bind_port: 0,
        };
        let request = TunnelRequest {
            ssh: &ssh,
            remote_host: "127.0.0.1",
            remote_port: 8108,
            secret: None,
        };

        let err = RusshTunnel::new(Arc::new(MockHostKeyPrompt::new()))
            .open(&request)
            .unwrap_err();

        assert!(matches!(err, GevoError::Tunnel(_)));
    }

    // Set GEVO_TEST_SSH_HOST / _USER / _PASSWORD; forwards to the server's own port 22.
    #[test]
    #[ignore = "needs a real SSH server; run manually with cargo test -- --ignored"]
    fn forwards_a_local_port_through_a_real_ssh_server() {
        let var = |name: &str| std::env::var(name).unwrap();
        let ssh = SshConfig {
            host: var("GEVO_TEST_SSH_HOST"),
            port: 22,
            username: var("GEVO_TEST_SSH_USER"),
            auth: SshAuthMethod::Password,
            local_bind_port: 0,
        };
        let password = Zeroizing::new(var("GEVO_TEST_SSH_PASSWORD"));
        let request = TunnelRequest {
            ssh: &ssh,
            remote_host: "127.0.0.1",
            remote_port: 22,
            secret: Some(&password),
        };

        let mut prompt = MockHostKeyPrompt::new();
        prompt.expect_confirm().returning(|_, _, _, _| true);
        let handle = RusshTunnel::new(Arc::new(prompt)).open(&request).unwrap();
        let port = handle.local_port();
        assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());

        drop(handle);
        std::thread::sleep(Duration::from_millis(200));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
