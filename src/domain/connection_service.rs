use std::sync::Arc;

use url::Url;
use zeroize::Zeroizing;

use crate::domain::error::{GevoError, TypesenseMsg};
use crate::domain::profile::{ConnectionProfile, SshConfig};
use crate::domain::ssh_tunnel::{SshTunnel, TunnelHandle, TunnelRequest};
use crate::domain::typesense_client::{SdkTypesenseClient, TypesenseClient};

/// Outcome of one step of a connection test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
    Ok,
    /// The step does not apply to this profile (no SSH tunnel).
    Skipped,
    /// An earlier step failed.
    NotRun,
    Failed(GevoError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionTestReport {
    pub tunnel: StepStatus,
    pub health: StepStatus,
    pub auth: StepStatus,
}

impl ConnectionTestReport {
    pub fn succeeded(&self) -> bool {
        [&self.tunnel, &self.health, &self.auth]
            .iter()
            .all(|s| matches!(s, StepStatus::Ok | StepStatus::Skipped))
    }
}

/// Builds a Typesense client for a base URL and API key.
#[cfg_attr(
    test,
    mockall::automock(type Client = crate::domain::typesense_client::MockTypesenseClient;)
)]
pub trait ClientFactory {
    type Client: TypesenseClient;

    fn build(&self, url: &str, api_key: &str) -> Result<Self::Client, GevoError>;

    /// Like `build`, but reaches the profile's host through a local forward on `local_port`.
    fn build_via_tunnel(
        &self,
        url: &str,
        api_key: &str,
        local_port: u16,
    ) -> Result<Self::Client, GevoError>;
}

pub struct SdkClientFactory;

impl ClientFactory for SdkClientFactory {
    type Client = SdkTypesenseClient;

    fn build(&self, url: &str, api_key: &str) -> Result<Self::Client, GevoError> {
        SdkTypesenseClient::new(url, api_key)
    }

    fn build_via_tunnel(
        &self,
        url: &str,
        api_key: &str,
        local_port: u16,
    ) -> Result<Self::Client, GevoError> {
        SdkTypesenseClient::new_via_tunnel(url, api_key, local_port)
    }
}

pub struct ConnectionService<T, F> {
    tunnel: Arc<T>,
    clients: F,
}

impl<T, F> ConnectionService<T, F>
where
    T: SshTunnel + Send + Sync + 'static,
    F: ClientFactory,
{
    pub fn new(tunnel: T, clients: F) -> Self {
        Self {
            tunnel: Arc::new(tunnel),
            clients,
        }
    }

    /// Tunnel (if configured) -> health -> auth, stopping at the first failure.
    /// `ssh_secret` is the SSH password or key passphrase.
    pub async fn test(
        &self,
        profile: &ConnectionProfile,
        api_key: &Zeroizing<String>,
        ssh_secret: Option<&Zeroizing<String>>,
    ) -> ConnectionTestReport {
        // Held until the test ends so the forward stays open.
        let mut _tunnel_guard = None;
        let (tunnel, local_port) = match &profile.ssh {
            None => (StepStatus::Skipped, None),
            Some(ssh) => match self.open_tunnel(ssh, &profile.url, ssh_secret).await {
                Ok(handle) => {
                    let port = handle.local_port();
                    _tunnel_guard = Some(handle);
                    (StepStatus::Ok, Some(port))
                }
                Err(e) => {
                    return ConnectionTestReport {
                        tunnel: StepStatus::Failed(e),
                        health: StepStatus::NotRun,
                        auth: StepStatus::NotRun,
                    };
                }
            },
        };

        let mut report = ConnectionTestReport {
            tunnel,
            health: StepStatus::NotRun,
            auth: StepStatus::NotRun,
        };
        let built = match local_port {
            None => self.clients.build(&profile.url, api_key),
            Some(port) => self.clients.build_via_tunnel(&profile.url, api_key, port),
        };
        let client = match built {
            Ok(c) => c,
            Err(e) => {
                report.health = StepStatus::Failed(e);
                return report;
            }
        };
        report.health = match client.health().await {
            Ok(true) => StepStatus::Ok,
            Ok(false) => StepStatus::Failed(GevoError::Api(TypesenseMsg(
                "node reports it is not healthy".to_string(),
            ))),
            Err(e) => StepStatus::Failed(e),
        };
        if report.health != StepStatus::Ok {
            return report;
        }
        report.auth = match client.check_auth().await {
            Ok(()) => StepStatus::Ok,
            Err(e) => StepStatus::Failed(e),
        };
        report
    }

    /// Opens a forward to the profile URL's host and port.
    async fn open_tunnel(
        &self,
        ssh: &SshConfig,
        profile_url: &str,
        ssh_secret: Option<&Zeroizing<String>>,
    ) -> Result<TunnelHandle, GevoError> {
        let url = Url::parse(profile_url)
            .map_err(|e| GevoError::Tunnel(format!("invalid profile URL: {e}")))?;
        let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
            return Err(GevoError::Tunnel(
                "profile URL needs a host and port to tunnel to".to_string(),
            ));
        };
        let host = host.trim_matches(|c| c == '[' || c == ']');
        // spawn_blocking needs 'static, so the request inputs are owned copies.
        let (ssh, host, secret) = (ssh.clone(), host.to_owned(), ssh_secret.cloned());
        let tunnel = Arc::clone(&self.tunnel);
        let handle = tokio::task::spawn_blocking(move || {
            tunnel.open(&TunnelRequest {
                ssh: &ssh,
                remote_host: &host,
                remote_port: port,
                secret: secret.as_ref(),
            })
        })
        .await
        .map_err(|e| GevoError::Tunnel(format!("tunnel task failed: {e}")))??;
        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::domain::profile::{SshAuthMethod, SshConfig};
    use crate::domain::ssh_tunnel::{MockSshTunnel, TunnelHandle};
    use crate::domain::typesense_client::MockTypesenseClient;

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn key() -> Zeroizing<String> {
        Zeroizing::new("admin-key".to_string())
    }

    fn plain_profile() -> ConnectionProfile {
        ConnectionProfile::new("Prod", "https://ts.example.com:8443")
    }

    fn ssh_profile() -> ConnectionProfile {
        let mut p = ConnectionProfile::new("Prod", "http://10.0.0.5:8108");
        p.ssh = Some(SshConfig {
            host: "bastion".into(),
            port: 22,
            username: "deploy".into(),
            auth: SshAuthMethod::Password,
            local_bind_port: 0,
        });
        p
    }

    fn healthy_client() -> MockTypesenseClient {
        let mut c = MockTypesenseClient::new();
        c.expect_health().returning(|| Ok(true));
        c.expect_check_auth().returning(|| Ok(()));
        c
    }

    fn tunnel_factory_returning(
        client: MockTypesenseClient,
        expect_url: &'static str,
        expect_port: u16,
    ) -> MockClientFactory {
        let mut f = MockClientFactory::new();
        f.expect_build().never();
        let mut client = Some(client);
        f.expect_build_via_tunnel()
            .withf(move |url, k, port| {
                url == expect_url && k == "admin-key" && *port == expect_port
            })
            .times(1)
            .returning(move |_, _, _| Ok(client.take().unwrap()));
        f
    }

    fn factory_returning(
        client: MockTypesenseClient,
        expect_url: &'static str,
    ) -> MockClientFactory {
        let mut f = MockClientFactory::new();
        let mut client = Some(client);
        f.expect_build()
            .withf(move |url, k| url == expect_url && k == "admin-key")
            .times(1)
            .returning(move |_, _| Ok(client.take().unwrap()));
        f
    }

    #[tokio::test]
    async fn no_ssh_skips_tunnel_and_tests_profile_url() {
        let mut tunnel = MockSshTunnel::new();
        tunnel.expect_open().never();
        let svc = ConnectionService::new(
            tunnel,
            factory_returning(healthy_client(), "https://ts.example.com:8443"),
        );

        let report = svc.test(&plain_profile(), &key(), None).await;

        assert_eq!(
            report,
            ConnectionTestReport {
                tunnel: StepStatus::Skipped,
                health: StepStatus::Ok,
                auth: StepStatus::Ok,
            }
        );
        assert!(report.succeeded());
    }

    #[tokio::test]
    async fn ssh_opens_tunnel_to_url_host_and_builds_client_via_tunnel() {
        let mut tunnel = MockSshTunnel::new();
        tunnel
            .expect_open()
            .withf(|req| {
                req.remote_host == "10.0.0.5"
                    && req.remote_port == 8108
                    && req.secret.map(|s| s.as_str()) == Some("pw")
            })
            .times(1)
            .returning(|_| Ok(TunnelHandle::new(54321, Box::new(()))));
        let svc = ConnectionService::new(
            tunnel,
            tunnel_factory_returning(healthy_client(), "http://10.0.0.5:8108", 54321),
        );
        let pw = Zeroizing::new("pw".to_string());

        let report = svc.test(&ssh_profile(), &key(), Some(&pw)).await;

        assert_eq!(report.tunnel, StepStatus::Ok);
        assert!(report.succeeded());
    }

    #[tokio::test]
    async fn ssh_strips_brackets_from_ipv6_url_host() {
        let mut tunnel = MockSshTunnel::new();
        tunnel
            .expect_open()
            .withf(|req| req.remote_host == "::1" && req.remote_port == 8108)
            .times(1)
            .returning(|_| Ok(TunnelHandle::new(54321, Box::new(()))));
        let svc = ConnectionService::new(
            tunnel,
            tunnel_factory_returning(healthy_client(), "http://[::1]:8108", 54321),
        );
        let mut profile = ConnectionProfile::new("Prod", "http://[::1]:8108");
        profile.ssh = ssh_profile().ssh;

        let report = svc.test(&profile, &key(), None).await;

        assert_eq!(report.tunnel, StepStatus::Ok);
    }

    #[tokio::test]
    async fn ssh_uses_scheme_default_port_when_url_has_none() {
        let mut p = ssh_profile();
        p.url = "https://ts.example.com".into();
        let mut tunnel = MockSshTunnel::new();
        tunnel
            .expect_open()
            .withf(|req| req.remote_host == "ts.example.com" && req.remote_port == 443)
            .returning(|_| Ok(TunnelHandle::new(1000, Box::new(()))));
        let svc = ConnectionService::new(
            tunnel,
            tunnel_factory_returning(healthy_client(), "https://ts.example.com", 1000),
        );

        assert!(svc.test(&p, &key(), None).await.succeeded());
    }

    #[tokio::test]
    async fn tunnel_failure_stops_and_marks_later_steps_not_run() {
        let mut tunnel = MockSshTunnel::new();
        tunnel
            .expect_open()
            .returning(|_| Err(GevoError::Tunnel("handshake failed".into())));
        let mut factory = MockClientFactory::new();
        factory.expect_build().never();
        let svc = ConnectionService::new(tunnel, factory);

        let report = svc.test(&ssh_profile(), &key(), None).await;

        assert_eq!(
            report,
            ConnectionTestReport {
                tunnel: StepStatus::Failed(GevoError::Tunnel("handshake failed".into())),
                health: StepStatus::NotRun,
                auth: StepStatus::NotRun,
            }
        );
        assert!(!report.succeeded());
    }

    #[tokio::test]
    async fn tunnel_build_failure_is_reported_on_health_step() {
        let mut tunnel = MockSshTunnel::new();
        tunnel
            .expect_open()
            .returning(|_| Ok(TunnelHandle::new(7, Box::new(()))));
        let mut factory = MockClientFactory::new();
        factory
            .expect_build_via_tunnel()
            .returning(|_, _, _| Err(GevoError::Api(TypesenseMsg("bad tunnel url".into()))));
        let svc = ConnectionService::new(tunnel, factory);

        let report = svc.test(&ssh_profile(), &key(), None).await;

        assert_eq!(report.tunnel, StepStatus::Ok);
        assert_eq!(
            report.health,
            StepStatus::Failed(GevoError::Api(TypesenseMsg("bad tunnel url".into())))
        );
    }

    #[tokio::test]
    async fn invalid_profile_url_with_ssh_fails_tunnel_step() {
        let mut p = ssh_profile();
        p.url = "not a url".into();
        let mut tunnel = MockSshTunnel::new();
        tunnel.expect_open().never();
        let svc = ConnectionService::new(tunnel, MockClientFactory::new());

        let report = svc.test(&p, &key(), None).await;

        assert!(matches!(
            report.tunnel,
            StepStatus::Failed(GevoError::Tunnel(_))
        ));
        assert_eq!(report.health, StepStatus::NotRun);
    }

    #[tokio::test]
    async fn health_error_stops_before_auth() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_health()
            .returning(|| Err(GevoError::Network("connection refused".into())));
        client.expect_check_auth().never();
        let svc = ConnectionService::new(
            MockSshTunnel::new(),
            factory_returning(client, "https://ts.example.com:8443"),
        );

        let report = svc.test(&plain_profile(), &key(), None).await;

        assert_eq!(report.tunnel, StepStatus::Skipped);
        assert_eq!(
            report.health,
            StepStatus::Failed(GevoError::Network("connection refused".into()))
        );
        assert_eq!(report.auth, StepStatus::NotRun);
    }

    #[tokio::test]
    async fn unhealthy_node_fails_health_with_api_error() {
        let mut client = MockTypesenseClient::new();
        client.expect_health().returning(|| Ok(false));
        client.expect_check_auth().never();
        let svc = ConnectionService::new(
            MockSshTunnel::new(),
            factory_returning(client, "https://ts.example.com:8443"),
        );

        let report = svc.test(&plain_profile(), &key(), None).await;

        assert!(matches!(
            report.health,
            StepStatus::Failed(GevoError::Api(_))
        ));
        assert_eq!(report.auth, StepStatus::NotRun);
    }

    #[tokio::test]
    async fn auth_failure_keeps_typesense_message_verbatim() {
        let mut client = MockTypesenseClient::new();
        client.expect_health().returning(|| Ok(true));
        client
            .expect_check_auth()
            .returning(|| Err(GevoError::Auth("Forbidden - bad key".into())));
        let svc = ConnectionService::new(
            MockSshTunnel::new(),
            factory_returning(client, "https://ts.example.com:8443"),
        );

        let report = svc.test(&plain_profile(), &key(), None).await;

        assert_eq!(report.health, StepStatus::Ok);
        assert_eq!(
            report.auth,
            StepStatus::Failed(GevoError::Auth("Forbidden - bad key".into()))
        );
        assert!(!report.succeeded());
    }

    #[tokio::test]
    async fn client_build_failure_is_reported_on_health_step() {
        let mut factory = MockClientFactory::new();
        factory
            .expect_build()
            .returning(|_, _| Err(GevoError::Api(TypesenseMsg("bad url".into()))));
        let svc = ConnectionService::new(MockSshTunnel::new(), factory);

        let report = svc.test(&plain_profile(), &key(), None).await;

        assert_eq!(
            report.health,
            StepStatus::Failed(GevoError::Api(TypesenseMsg("bad url".into())))
        );
        assert_eq!(report.auth, StepStatus::NotRun);
    }

    #[tokio::test]
    async fn tunnel_stays_open_during_test_and_closes_after() {
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = dropped.clone();
        let mut tunnel = MockSshTunnel::new();
        tunnel
            .expect_open()
            .returning(move |_| Ok(TunnelHandle::new(1, Box::new(DropFlag(flag.clone())))));
        let mut client = MockTypesenseClient::new();
        let seen = dropped.clone();
        client.expect_health().returning(move || {
            assert!(!seen.load(Ordering::SeqCst), "tunnel dropped mid-test");
            Ok(true)
        });
        client.expect_check_auth().returning(|| Ok(()));
        let svc = ConnectionService::new(
            tunnel,
            tunnel_factory_returning(client, "http://10.0.0.5:8108", 1),
        );

        svc.test(&ssh_profile(), &key(), None).await;

        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn succeeded_is_false_when_any_step_is_not_ok_or_skipped() {
        let ok = ConnectionTestReport {
            tunnel: StepStatus::Skipped,
            health: StepStatus::Ok,
            auth: StepStatus::Ok,
        };
        assert!(ok.succeeded());
        let not_run = ConnectionTestReport {
            auth: StepStatus::NotRun,
            ..ok
        };
        assert!(!not_run.succeeded());
    }

    #[test]
    fn sdk_factory_builds_a_client_for_a_valid_url() {
        assert!(SdkClientFactory.build("http://localhost:8108", "k").is_ok());
    }
}
