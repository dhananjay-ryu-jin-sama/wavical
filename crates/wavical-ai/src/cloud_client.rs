#![forbid(unsafe_code)]

//! HTTPS client configuration and strict network isolation for Cloud AI (Phase 3 WP7).
//!
//! # Security Guarantees & Constraints:
//! - **TLS Certificate Verification**: Certificate verification is strictly ON.
//!   There is zero code or configuration option that allows disabling certificate
//!   validation (`danger_accept_invalid_certs` is nowhere in the codebase).
//! - **Strict Host Isolation**: Only the exact hostname configured in `ai.cloud.baseUrl`
//!   is ever contacted. Any request attempting to contact a different host is rejected.
//! - **Redirects**: Redirects are strictly NOT followed (`Policy::none()`). If an endpoint
//!   returns an HTTP 3xx redirect status, the request fails immediately.
//! - **Timeouts**: Connection timeout is 10 seconds (`CLOUD_CONNECT_TIMEOUT_SECS`);
//!   total request timeout is 120 seconds (`CLOUD_TOTAL_TIMEOUT_SECS`).
//! - **Body Size Limits**:
//!   - Request body is limited to at most 1 MB (`MAX_CLOUD_REQUEST_BODY_BYTES`).
//!   - Non-streamed response body is limited to at most 4 MB (`MAX_CLOUD_RESPONSE_BODY_BYTES`).
//!   - Streamed output is capped at 100,000 characters (`MAX_CHAT_STREAMED_OUTPUT_CHARS`).
//!
//! # System Proxy Settings:
//! Standard Windows system proxy auto-detection is enabled via reqwest's `system-proxy`
//! feature. Outbound HTTPS connections to the configured `baseUrl` host automatically honor
//! system proxy settings (such as Windows WinINet proxy configuration and standard HTTPS proxy
//! environment variables `HTTPS_PROXY` / `ALL_PROXY`).

use reqwest::blocking::Client;
use std::time::Duration;
use thiserror::Error;

use wavical_ipc::ai::{CLOUD_CONNECT_TIMEOUT_SECS, CLOUD_TOTAL_TIMEOUT_SECS};

/// Errors arising during cloud client construction or URL validation.
#[derive(Debug, Error)]
pub enum CloudClientError {
    #[error("failed to build HTTPS client: {0}")]
    BuildError(String),

    #[error("insecure or invalid cloud baseUrl '{0}': cloud endpoints strictly require https://")]
    InsecureScheme(String),

    #[error("invalid cloud baseUrl '{0}': missing or invalid hostname")]
    InvalidHost(String),

    #[error("userinfo credentials are not allowed in cloud baseUrl '{0}'")]
    UserInfoForbidden(String),

    #[error("target URL host '{0}' does not match configured baseUrl host '{1}'")]
    HostMismatch(String, String),

    #[error("failed to parse URL: {0}")]
    UrlParseError(String),
}

/// Create a pre-configured blocking HTTPS client for cloud inference.
///
/// Certificate verification is strictly enforced.
/// Optionally accepts a PEM-encoded custom Root Certificate for automated testing with a mock TLS server.
pub fn create_cloud_http_client(
    custom_root_cert_pem: Option<&[u8]>,
) -> Result<Client, CloudClientError> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(CLOUD_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(CLOUD_TOTAL_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none());

    if let Some(cert_pem) = custom_root_cert_pem {
        let cert = reqwest::Certificate::from_pem(cert_pem)
            .map_err(|e| CloudClientError::BuildError(format!("failed to parse root cert: {e}")))?;
        builder = builder.add_root_certificate(cert);
    }

    builder
        .build()
        .map_err(|e| CloudClientError::BuildError(e.to_string()))
}

/// Validates that a cloud base URL is strictly HTTPS, has a valid host, and contains no userinfo.
///
/// Returns the validated provider hostname (e.g. `api.openai.com`).
pub fn validate_and_extract_cloud_host(base_url_str: &str) -> Result<String, CloudClientError> {
    let trimmed = base_url_str.trim();
    if !trimmed.starts_with("https://") {
        return Err(CloudClientError::InsecureScheme(trimmed.to_string()));
    }

    let after_scheme = &trimmed["https://".len()..];
    let authority = after_scheme.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err(CloudClientError::InvalidHost(trimmed.to_string()));
    }

    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|e| CloudClientError::UrlParseError(format!("{trimmed}: {e}")))?;

    if parsed.scheme() != "https" {
        return Err(CloudClientError::InsecureScheme(trimmed.to_string()));
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(CloudClientError::UserInfoForbidden(trimmed.to_string()));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| CloudClientError::InvalidHost(trimmed.to_string()))?;

    if host.is_empty() || host.contains(' ') {
        return Err(CloudClientError::InvalidHost(trimmed.to_string()));
    }

    Ok(host.to_string())
}

/// Build and validate a target endpoint URL, ensuring its host matches the configured baseUrl host.
pub fn build_target_cloud_url(
    base_url_str: &str,
    endpoint_path: &str,
) -> Result<String, CloudClientError> {
    let configured_host = validate_and_extract_cloud_host(base_url_str)?;
    let trimmed_base = base_url_str.trim().trim_end_matches('/');
    let trimmed_path = endpoint_path.trim().trim_start_matches('/');
    let full_url_str = format!("{trimmed_base}/{trimmed_path}");

    let parsed_target = reqwest::Url::parse(&full_url_str)
        .map_err(|e| CloudClientError::UrlParseError(format!("{full_url_str}: {e}")))?;

    let target_host = parsed_target
        .host_str()
        .ok_or_else(|| CloudClientError::InvalidHost(full_url_str.clone()))?;

    if target_host != configured_host {
        return Err(CloudClientError::HostMismatch(
            target_host.to_string(),
            configured_host,
        ));
    }

    Ok(full_url_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_and_extract_cloud_host() {
        assert_eq!(
            validate_and_extract_cloud_host("https://api.openai.com/v1").unwrap(),
            "api.openai.com"
        );
        assert_eq!(
            validate_and_extract_cloud_host("https://api.groq.com/openai/v1/").unwrap(),
            "api.groq.com"
        );

        // Insecure scheme rejected
        assert!(matches!(
            validate_and_extract_cloud_host("http://api.openai.com/v1"),
            Err(CloudClientError::InsecureScheme(_))
        ));

        // Userinfo rejected
        assert!(matches!(
            validate_and_extract_cloud_host("https://user:pass@api.openai.com/v1"),
            Err(CloudClientError::UserInfoForbidden(_))
        ));

        // Missing host rejected
        let res = validate_and_extract_cloud_host("https:///v1");
        assert!(
            res.is_err(),
            "expected error for missing host, got: {res:?}"
        );
    }

    #[test]
    fn test_build_target_cloud_url_host_matching() {
        let base = "https://api.openai.com/v1";
        let target = build_target_cloud_url(base, "chat/completions").unwrap();
        assert_eq!(target, "https://api.openai.com/v1/chat/completions");
    }
}
