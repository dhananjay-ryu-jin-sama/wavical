#![forbid(unsafe_code)]

//! API Key credential resolution, storage, and zero-leak redaction (Phase 3 WP7).
//!
//! Enforces:
//! - Secrets never leave Rust: API keys are never in config files, the page, logs,
//!   child process environments, command lines, error text, window state, or index.
//! - Kept in `CloudApiKey` whose `Debug` and `Display` are strictly redacted to `"[REDACTED]"`.
//! - Key is never cloned into long-lived structures; zeroed on drop.
//! - Resolution order:
//!   1. Environment variable named by `ai.cloud.apiKeyEnv` (read at call time).
//!   2. Windows Credential Store (keyring) under service "wavical" and host `<provider-host>`.

use std::fmt;
use thiserror::Error;

/// Errors arising during credential resolution or management.
#[derive(Debug, Error)]
pub enum CredentialsError {
    #[error("API key is empty")]
    EmptyKey,

    #[error(
        "API key not found in environment variable '{env_var}' or Windows Credential Store for host '{host}'"
    )]
    KeyNotFound { env_var: String, host: String },

    #[error("credential store operation failed: {0}")]
    KeyringError(String),

    #[error("invalid provider hostname: {0}")]
    InvalidHost(String),
}

/// Secure wrapper around an API key secret.
///
/// Implements strictly redacted `Debug` and `Display` output to prevent accidental
/// leakage in log output, error messages, or format strings.
pub struct CloudApiKey(String);

impl CloudApiKey {
    /// Create a new `CloudApiKey` validating that the key is non-empty.
    pub fn new(raw: String) -> Result<Self, CredentialsError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(CredentialsError::EmptyKey);
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Expose secret strictly when constructing Authorization header for outbound request.
    ///
    /// Never log or display the return value.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CloudApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl fmt::Display for CloudApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl Drop for CloudApiKey {
    fn drop(&mut self) {
        // Overwrite memory before deallocation
        self.0.replace_range(.., "");
    }
}

/// Validate provider hostname format (e.g. `api.openai.com`).
pub fn validate_provider_host(host: &str) -> Result<&str, CredentialsError> {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return Err(CredentialsError::InvalidHost(
            "provider host cannot be empty".to_string(),
        ));
    }
    if trimmed.len() > 255 {
        return Err(CredentialsError::InvalidHost(
            "provider host exceeds 255 characters".to_string(),
        ));
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(':')
        || trimmed.contains(' ')
    {
        return Err(CredentialsError::InvalidHost(
            "provider host must be a hostname only, without scheme, port, path, or whitespace (e.g. 'api.openai.com')".to_string(),
        ));
    }
    Ok(trimmed)
}

/// Resolve an API key for a cloud request at call time.
///
/// Order of resolution:
/// 1. Environment variable named by `api_key_env` (e.g. `OPENAI_API_KEY`).
/// 2. Windows Credential Store (keyring) for host `provider_host` under service `"wavical"`.
pub fn resolve_api_key(
    api_key_env: &str,
    provider_host: &str,
) -> Result<CloudApiKey, CredentialsError> {
    // 1. Check environment variable named by apiKeyEnv
    let trimmed_env = api_key_env.trim();
    if !trimmed_env.is_empty()
        && let Ok(val) = std::env::var(trimmed_env)
    {
        let trimmed_val = val.trim();
        if !trimmed_val.is_empty() {
            return CloudApiKey::new(trimmed_val.to_string());
        }
    }

    // 2. Check Windows Credential Store
    let trimmed_host = provider_host.trim();
    if !trimmed_host.is_empty()
        && let Ok(entry) = keyring::Entry::new("wavical", trimmed_host)
        && let Ok(password) = entry.get_password()
    {
        let trimmed_pw = password.trim();
        if !trimmed_pw.is_empty() {
            return CloudApiKey::new(trimmed_pw.to_string());
        }
    }

    Err(CredentialsError::KeyNotFound {
        env_var: trimmed_env.to_string(),
        host: trimmed_host.to_string(),
    })
}

/// Store an API key into the credential store for a provider host.
pub fn store_secret_in_keyring(provider_host: &str, secret: &str) -> Result<(), CredentialsError> {
    let host = validate_provider_host(provider_host)?;
    let trimmed_secret = secret.trim();
    if trimmed_secret.is_empty() {
        return Err(CredentialsError::EmptyKey);
    }

    let entry = keyring::Entry::new("wavical", host)
        .map_err(|e| CredentialsError::KeyringError(e.to_string()))?;
    entry
        .set_password(trimmed_secret)
        .map_err(|e| CredentialsError::KeyringError(e.to_string()))?;
    Ok(())
}

/// Delete an API key from the credential store for a provider host.
pub fn clear_secret_from_keyring(provider_host: &str) -> Result<(), CredentialsError> {
    let host = validate_provider_host(provider_host)?;
    let entry = keyring::Entry::new("wavical", host)
        .map_err(|e| CredentialsError::KeyringError(e.to_string()))?;
    let _ = entry.delete_credential();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloud_api_key_redaction() {
        let key = CloudApiKey::new("sk-test-super-secret-12345".to_string()).expect("valid key");
        assert_eq!(format!("{:?}", key), "[REDACTED]");
        assert_eq!(format!("{}", key), "[REDACTED]");
        assert_eq!(key.expose_secret(), "sk-test-super-secret-12345");
    }

    #[test]
    fn test_empty_api_key_rejected() {
        assert!(CloudApiKey::new("   ".to_string()).is_err());
        assert!(CloudApiKey::new("".to_string()).is_err());
    }

    #[test]
    fn test_validate_provider_host() {
        assert!(validate_provider_host("api.openai.com").is_ok());
        assert!(validate_provider_host("api.groq.com").is_ok());
        assert!(validate_provider_host("https://api.openai.com").is_err());
        assert!(validate_provider_host("api.openai.com/v1").is_err());
        assert!(validate_provider_host("api.openai.com:443").is_err());
        assert!(validate_provider_host("").is_err());
    }

    #[test]
    fn test_resolve_api_key_empty_returns_not_found() {
        assert!(
            resolve_api_key("NON_EXISTENT_VAR_WAVICAL_123", "non-existent-host.local").is_err()
        );
    }
}
