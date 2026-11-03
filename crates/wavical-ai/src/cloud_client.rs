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
/// Enforces strict lexical rules before parsing:
/// - No uppercase ASCII in scheme/host
/// - No trailing dot on hostname
/// - No explicit ports (must use default 443)
pub fn validate_and_extract_cloud_host(base_url_str: &str) -> Result<String, CloudClientError> {
    let trimmed = base_url_str.trim();

    // Pre-parse lexical checks on raw string
    if !trimmed.starts_with("https://") {
        return Err(CloudClientError::InsecureScheme(trimmed.to_string()));
    }

    let after_scheme = &trimmed["https://".len()..];
    let authority = after_scheme.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err(CloudClientError::InvalidHost(trimmed.to_string()));
    }

    // Check for uppercase ASCII letters in the authority part (before parsing normalizes them)
    if authority.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(CloudClientError::InvalidHost(format!(
            "{trimmed} (uppercase hosts not allowed)"
        )));
    }

    // Extract hostname part (before port or @ for userinfo)
    // Handle userinfo (user:pass@host:port format)
    let (host_port_part, has_userinfo) = if authority.contains('@') {
        // Userinfo present, extract part after @
        let parts: Vec<&str> = authority.splitn(2, '@').collect();
        (parts.get(1).copied().unwrap_or(""), true)
    } else {
        (authority, false)
    };

    let hostname_part = host_port_part.split(':').next().unwrap_or(host_port_part);

    // Check for explicit port (allow for localhost/127.0.0.1 for testing, reject for domain names)
    if !has_userinfo && host_port_part.contains(':') {
        // Allow ports for localhost and 127.0.0.1 (test scenarios), but reject for domain names
        let is_localhost = hostname_part == "localhost" || hostname_part == "127.0.0.1";
        if !is_localhost {
            return Err(CloudClientError::InvalidHost(format!(
                "{trimmed} (explicit ports not allowed for domain names)"
            )));
        }
    }

    // Check for trailing dot on hostname
    if hostname_part.ends_with('.') {
        return Err(CloudClientError::InvalidHost(format!(
            "{trimmed} (trailing dots not allowed)"
        )));
    }

    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|e| CloudClientError::UrlParseError(format!("{trimmed}: {e}")))?;

    if parsed.scheme() != "https" {
        return Err(CloudClientError::InsecureScheme(trimmed.to_string()));
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(CloudClientError::UserInfoForbidden(trimmed.to_string()));
    }

    // REQ-02.5: IP literals are only allowed for IPv4 loopback (local test servers).
    // Any other IP (link-local metadata, private LAN, public, IPv6) is rejected to prevent SSRF.
    let host = parsed
        .host_str()
        .ok_or_else(|| CloudClientError::InvalidHost(trimmed.to_string()))?;

    // The URL parser normalises hex/decimal/octal IPv4 forms to dotted decimal, so this catches them.
    let is_ipv6_literal = host.starts_with('[');
    let non_loopback_ip = match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => !ip.is_loopback(),
        Err(_) => false,
    };
    if is_ipv6_literal || non_loopback_ip {
        return Err(CloudClientError::InvalidHost(format!(
            "{trimmed} (IP literal hosts other than IPv4 loopback not allowed)"
        )));
    }

    if host.is_empty() || host.contains(' ') {
        return Err(CloudClientError::InvalidHost(trimmed.to_string()));
    }

    // Check for potential homoglyphs: detect if normalized host differs from original authority
    // (indicates unicode normalization occurred, which may indicate homoglyphs)
    if host != hostname_part {
        return Err(CloudClientError::InvalidHost(format!(
            "{trimmed} (homoglyphs or unicode normalization not allowed)"
        )));
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
