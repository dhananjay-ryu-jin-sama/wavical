#![forbid(unsafe_code)]

//! Communication channel and session security between Wavical and the engine.
//!
//! Enforces:
//! - Local loopback only (`127.0.0.1`).
//! - Ephemeral dynamically assigned port.
//! - Cryptographically secure per-session bearer token (256-bit CSPRNG).
//! - Strict redaction: tokens are NEVER logged or displayed in debug strings.

use std::fmt;
use std::net::TcpListener;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChannelError {
    #[error("failed to generate secure CSPRNG token: {0}")]
    EntropyError(String),

    #[error("failed to bind ephemeral port on 127.0.0.1: {0}")]
    PortBindingError(String),
}

/// Opaque wrapper around the engine session bearer token.
///
/// Implements custom `Debug` and `Display` to guarantee the raw secret token is
/// NEVER accidentally exposed in error messages, panic traces, or log outputs.
#[derive(Clone, PartialEq, Eq)]
pub struct EngineSessionToken(String);

impl EngineSessionToken {
    /// Generate a new cryptographically secure 256-bit unguessable session token.
    pub fn generate() -> Result<Self, ChannelError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| ChannelError::EntropyError(e.to_string()))?;
        let token = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        Ok(Self(token))
    }

    /// Explicit accessor for constructing the HTTP `Authorization: Bearer <token>` header.
    ///
    /// Must only be used in internal HTTP client requests to the local engine.
    pub fn expose_secret_bearer_token(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EngineSessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED_ENGINE_SESSION_TOKEN]")
    }
}

impl fmt::Display for EngineSessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED_ENGINE_SESSION_TOKEN]")
    }
}

/// Configuration of the authenticated loopback communication channel.
#[derive(Clone, Debug)]
pub struct EngineChannel {
    pub host: String,
    pub port: u16,
    pub token: EngineSessionToken,
}

impl EngineChannel {
    /// Create a new channel bound to 127.0.0.1 on an available ephemeral port
    /// with a fresh CSPRNG session token.
    pub fn new_ephemeral() -> Result<Self, ChannelError> {
        let port = find_available_loopback_port()?;
        let token = EngineSessionToken::generate()?;
        Ok(Self {
            host: "127.0.0.1".to_string(),
            port,
            token,
        })
    }

    /// Return the base HTTP URL of the local engine.
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// Return the formatted `Authorization` header value.
    pub fn auth_header_value(&self) -> String {
        format!("Bearer {}", self.token.expose_secret_bearer_token())
    }
}

/// Find a free ephemeral TCP port on `127.0.0.1`.
pub fn find_available_loopback_port() -> Result<u16, ChannelError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| ChannelError::PortBindingError(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| ChannelError::PortBindingError(e.to_string()))?
        .port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generation_and_redaction() {
        let token = EngineSessionToken::generate().expect("token generation");
        assert_eq!(token.expose_secret_bearer_token().len(), 64);
        assert!(
            token
                .expose_secret_bearer_token()
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );

        // Confirm Debug and Display REDACT the token completely
        assert_eq!(format!("{token:?}"), "[REDACTED_ENGINE_SESSION_TOKEN]");
        assert_eq!(format!("{token}"), "[REDACTED_ENGINE_SESSION_TOKEN]");
        assert!(!format!("{token:?}").contains(token.expose_secret_bearer_token()));
    }

    #[test]
    fn test_ephemeral_channel_properties() {
        let channel = EngineChannel::new_ephemeral().expect("channel creation");
        assert_eq!(channel.host, "127.0.0.1");
        assert!(channel.port > 0);
        assert_eq!(
            channel.base_url(),
            format!("http://127.0.0.1:{}", channel.port)
        );
        assert!(channel.auth_header_value().starts_with("Bearer "));
        assert_eq!(channel.auth_header_value().len(), 7 + 64);
    }
}
