use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};

use url::{Host, Url};

use typesense::error::ApiError;
use typesense::models::ApiStatsResponse;
use typesense::{Client, Error as SdkError, NodeConfig};

use super::error::{GevoError, TypesenseMsg};

/// Server-facing operations for the connection test and health screens.
// Native async fn keeps the mockall mock simple; callers needing Send futures use the concrete type.
#[allow(async_fn_in_trait)]
#[cfg_attr(test, mockall::automock)]
pub trait TypesenseClient {
    /// `GET /health`: whether the node reports itself healthy.
    async fn health(&self) -> Result<bool, GevoError>;

    /// `GET /stats.json`.
    async fn stats(&self) -> Result<ApiStatsResponse, GevoError>;

    /// Verifies the configured key is an admin key by listing `GET /keys`, which only admin keys may do.
    async fn check_auth(&self) -> Result<(), GevoError>;
}

/// `TypesenseClient` backed by the official `typesense` SDK.
pub struct SdkTypesenseClient {
    client: Client,
}

impl SdkTypesenseClient {
    pub fn new(url: &str, api_key: &str) -> Result<Self, GevoError> {
        let client = Client::builder()
            .nodes(vec![NodeConfig::new(url)])
            .api_key(api_key)
            .build()
            .map_err(|e| GevoError::Api(TypesenseMsg(e.to_string())))?;
        Ok(Self { client })
    }

    /// Connects to `127.0.0.1:local_port` while keeping the URL's hostname for SNI and certificate checks.
    pub fn new_via_tunnel(url: &str, api_key: &str, local_port: u16) -> Result<Self, GevoError> {
        let (url, resolve_host) = tunnel_endpoint(url, local_port)?;
        let mut node = NodeConfig::new(url);
        if let Some(host) = resolve_host {
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, local_port));
            node = node.http_builder(move |b| b.resolve(&host, addr));
        }
        let client = Client::builder()
            .nodes(vec![node])
            .api_key(api_key)
            .build()
            .map_err(|e| GevoError::Api(TypesenseMsg(e.to_string())))?;
        Ok(Self { client })
    }
}

/// Rewrites the URL's port to the tunnel's; a hostname is kept and returned so DNS can be pinned to
/// loopback (reqwest always uses the URL port), while an IP host is replaced by loopback.
fn tunnel_endpoint(url: &str, local_port: u16) -> Result<(String, Option<String>), GevoError> {
    let mut url =
        Url::parse(url).map_err(|e| GevoError::Tunnel(format!("invalid profile URL: {e}")))?;
    let domain = match url.host() {
        Some(Host::Domain(d)) => Some(d.to_owned()),
        Some(_) => None,
        None => return Err(GevoError::Tunnel("profile URL needs a host".to_string())),
    };
    if domain.is_none() {
        url.set_host(Some("127.0.0.1"))
            .map_err(|e| GevoError::Tunnel(e.to_string()))?;
    }
    url.set_port(Some(local_port))
        .map_err(|()| GevoError::Tunnel("profile URL cannot carry a port".to_string()))?;
    Ok((url.as_str().trim_end_matches('/').to_owned(), domain))
}

impl fmt::Debug for SdkTypesenseClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SdkTypesenseClient").finish_non_exhaustive()
    }
}

impl TypesenseClient for SdkTypesenseClient {
    async fn health(&self) -> Result<bool, GevoError> {
        let status = self.client.operations().health().await.map_err(map_error)?;
        Ok(status.ok)
    }

    async fn stats(&self) -> Result<ApiStatsResponse, GevoError> {
        self.client
            .operations()
            .retrieve_api_stats()
            .await
            .map_err(map_error)
    }

    async fn check_auth(&self) -> Result<(), GevoError> {
        self.client
            .keys()
            .retrieve()
            .await
            .map(drop)
            .map_err(map_error)
    }
}

fn map_error<E>(err: SdkError<E>) -> GevoError
where
    E: fmt::Debug + 'static,
{
    match err {
        SdkError::AllNodesFailed { source } | SdkError::Api(source) => map_api_error(source),
        SdkError::Deserialization(e) => GevoError::Api(TypesenseMsg(e.to_string())),
    }
}

fn map_api_error<E>(err: ApiError<E>) -> GevoError {
    match err {
        ApiError::ResponseError(resp) => {
            let message = server_message(&resp.content)
                .unwrap_or_else(|| fallback_message(resp.status, &resp.content));
            if matches!(resp.status.as_u16(), 401 | 403) {
                GevoError::Auth(message)
            } else {
                GevoError::Api(TypesenseMsg(message))
            }
        }
        ApiError::Serde(e) => GevoError::Api(TypesenseMsg(e.to_string())),
        other => GevoError::Network(other.to_string()),
    }
}

fn server_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(value.get("message")?.as_str()?.to_owned())
}

fn fallback_message(status: impl fmt::Display, body: &str) -> String {
    if body.is_empty() {
        format!("HTTP {status}")
    } else {
        body.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use typesense::error::ResponseContent;

    fn response_error(status: u16, body: &str) -> SdkError<()> {
        SdkError::Api(ApiError::ResponseError(ResponseContent {
            status: status.try_into().unwrap(),
            content: body.to_string(),
            entity: None,
        }))
    }

    #[test]
    fn unauthorized_maps_to_auth_with_server_message() {
        let err = map_error(response_error(
            401,
            r#"{"message": "Forbidden - a valid `x-typesense-api-key` header must be sent."}"#,
        ));
        assert_eq!(
            err,
            GevoError::Auth(
                "Forbidden - a valid `x-typesense-api-key` header must be sent.".into()
            )
        );
    }

    #[test]
    fn forbidden_maps_to_auth() {
        let err = map_error(response_error(403, r#"{"message": "Forbidden"}"#));
        assert_eq!(err.category(), "AUTH");
    }

    #[test]
    fn other_status_maps_to_api_with_message_verbatim() {
        let err = map_error(response_error(404, r#"{"message": "Not Found"}"#));
        assert_eq!(err, GevoError::Api(TypesenseMsg("Not Found".into())));
    }

    #[test]
    fn non_json_body_is_used_as_is() {
        let err = map_error(response_error(502, "Bad Gateway"));
        assert_eq!(err, GevoError::Api(TypesenseMsg("Bad Gateway".into())));
    }

    #[test]
    fn empty_body_falls_back_to_status() {
        let err = map_error(response_error(500, ""));
        assert_eq!(err.category(), "API");
        assert!(err.to_string().contains("500"));
    }

    #[test]
    fn io_error_maps_to_network() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let err = map_error(SdkError::<()>::Api(ApiError::Io(io)));
        assert_eq!(err.category(), "NETWORK");
        assert!(err.to_string().contains("refused"));
    }

    #[test]
    fn serde_error_maps_to_api() {
        let serde = serde_json::from_str::<u8>("x").unwrap_err();
        let err = map_error(SdkError::<()>::Deserialization(serde));
        assert_eq!(err.category(), "API");
    }

    #[test]
    fn all_nodes_failed_maps_by_last_error() {
        let source = ApiError::ResponseError(ResponseContent {
            status: 401.try_into().unwrap(),
            content: r#"{"message": "nope"}"#.to_string(),
            entity: None,
        });
        let err = map_error(SdkError::<()>::AllNodesFailed { source });
        assert_eq!(err, GevoError::Auth("nope".into()));
    }

    #[test]
    fn new_accepts_valid_config() {
        assert!(SdkTypesenseClient::new("http://localhost:8108", "key").is_ok());
    }

    #[test]
    fn tunnel_endpoint_keeps_hostname_and_swaps_port() {
        let (url, host) = tunnel_endpoint("https://ts.example.com:8443", 5000).unwrap();
        assert_eq!(url, "https://ts.example.com:5000");
        assert_eq!(host.as_deref(), Some("ts.example.com"));
    }

    #[test]
    fn tunnel_endpoint_adds_port_when_url_has_none() {
        let (url, host) = tunnel_endpoint("https://ts.example.com", 5000).unwrap();
        assert_eq!(url, "https://ts.example.com:5000");
        assert_eq!(host.as_deref(), Some("ts.example.com"));
    }

    #[test]
    fn tunnel_endpoint_points_ip_hosts_at_loopback() {
        let (url, host) = tunnel_endpoint("http://10.0.0.5:8108", 5000).unwrap();
        assert_eq!(url, "http://127.0.0.1:5000");
        assert_eq!(host, None);
    }

    #[test]
    fn tunnel_endpoint_rejects_invalid_url() {
        assert_eq!(
            tunnel_endpoint("not a url", 5000).unwrap_err().category(),
            "TUNNEL"
        );
    }

    #[test]
    fn new_via_tunnel_accepts_valid_config() {
        assert!(SdkTypesenseClient::new_via_tunnel("https://ts.example.com", "key", 5000).is_ok());
    }

    #[test]
    fn debug_does_not_leak_api_key() {
        let client = SdkTypesenseClient::new("http://localhost:8108", "super-secret-key").unwrap();
        assert!(!format!("{client:?}").contains("super-secret-key"));
    }

    #[tokio::test]
    async fn mock_client_serves_canned_results() {
        let mut mock = MockTypesenseClient::new();
        mock.expect_health().returning(|| Ok(true));
        mock.expect_check_auth()
            .returning(|| Err(GevoError::Auth("bad key".into())));
        mock.expect_stats()
            .returning(|| Ok(ApiStatsResponse::new()));

        assert!(mock.health().await.unwrap());
        assert_eq!(mock.check_auth().await.unwrap_err().category(), "AUTH");
        assert_eq!(mock.stats().await.unwrap(), ApiStatsResponse::new());
    }

    // Needs a real server: TYPESENSE_URL and TYPESENSE_API_KEY (admin).
    fn live_client() -> SdkTypesenseClient {
        let url = std::env::var("TYPESENSE_URL").expect("TYPESENSE_URL");
        let key = std::env::var("TYPESENSE_API_KEY").expect("TYPESENSE_API_KEY");
        SdkTypesenseClient::new(&url, &key).unwrap()
    }

    #[tokio::test]
    #[ignore = "requires a running Typesense"]
    async fn live_health_stats_and_auth() {
        let client = live_client();
        assert!(client.health().await.unwrap());
        client.stats().await.unwrap();
        client.check_auth().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a running Typesense"]
    async fn live_wrong_key_fails_auth() {
        let url = std::env::var("TYPESENSE_URL").expect("TYPESENSE_URL");
        let client = SdkTypesenseClient::new(&url, "wrong-key").unwrap();
        assert_eq!(client.check_auth().await.unwrap_err().category(), "AUTH");
    }
}
