/// A Typesense API error message, carried verbatim -- per
/// `0002-v1-requirements.md` ("Typesense's own error message surfaced, not swallowed") and
/// `DESIGN_HANDOFF.md` ("Always show Typesense's `message` verbatim"). A distinct type (not a
/// bare `String`) so `GevoError::Api`'s payload is unambiguously Typesense's own text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TypesenseMsg(pub String);

/// Every server-dependent action's failure, categorized per `DESIGN_HANDOFF.md`'s
/// NETWORK / TUNNEL / AUTH / API error chips (plan item 2.4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GevoError {
    /// Unreachable server, DNS failure, connection refused/timed out, etc.
    #[error("network error: {0}")]
    Network(String),
    /// SSH connect/auth/forward failure.
    #[error("tunnel error: {0}")]
    Tunnel(String),
    /// 401/403 from Typesense (bad or missing admin key).
    #[error("authentication error: {0}")]
    Auth(String),
    /// Any other Typesense API error; wraps its message verbatim.
    #[error("Typesense error: {0}")]
    Api(TypesenseMsg),
}

impl GevoError {
    /// The error-chip category label from `DESIGN_HANDOFF.md`, for the UI and for
    /// structured error logs (`AGENTS.md` §9).
    pub fn category(&self) -> &'static str {
        match self {
            GevoError::Network(_) => "NETWORK",
            GevoError::Tunnel(_) => "TUNNEL",
            GevoError::Auth(_) => "AUTH",
            GevoError::Api(_) => "API",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_matches_design_handoff_labels() {
        assert_eq!(
            GevoError::Network("unreachable".into()).category(),
            "NETWORK"
        );
        assert_eq!(GevoError::Tunnel("auth failed".into()).category(), "TUNNEL");
        assert_eq!(GevoError::Auth("bad key".into()).category(), "AUTH");
        assert_eq!(
            GevoError::Api(TypesenseMsg("field not found".into())).category(),
            "API"
        );
    }

    #[test]
    fn display_includes_the_message_for_each_category() {
        assert!(
            GevoError::Network("host unreachable".into())
                .to_string()
                .contains("host unreachable")
        );
        assert!(
            GevoError::Tunnel("ssh handshake failed".into())
                .to_string()
                .contains("ssh handshake failed")
        );
        assert!(
            GevoError::Auth("invalid admin key".into())
                .to_string()
                .contains("invalid admin key")
        );
    }

    #[test]
    fn api_error_carries_the_typesense_message_verbatim() {
        let msg = "Not found.";
        let err = GevoError::Api(TypesenseMsg(msg.to_string()));
        assert_eq!(err.to_string(), format!("Typesense error: {msg}"));
    }
}
