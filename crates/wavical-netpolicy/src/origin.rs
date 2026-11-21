//! Parsed-URL origin normalization, validation, and wildcard matching.

use std::collections::HashSet;
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

use thiserror::Error;
use url::{Host, Url};

use crate::limits::{MAX_HOST_LEN, MAX_ORIGIN_LEN};

/// Errors produced during origin or pattern parsing and validation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OriginError {
    #[error("origin is empty")]
    Empty,
    #[error("origin exceeds maximum length of {MAX_ORIGIN_LEN} characters")]
    OriginTooLong,
    #[error("host exceeds maximum length of {MAX_HOST_LEN} characters")]
    HostTooLong,
    #[error("invalid URL syntax")]
    InvalidUrl,
    #[error("origin must not contain userinfo")]
    UserinfoNotAllowed,
    #[error("origin must not contain a path, query, or fragment")]
    PathQueryOrFragmentNotAllowed,
    #[error("scheme '{0}' is not allowed in this context")]
    DisallowedScheme(String),
    #[error("origin has no host")]
    MissingHost,
    #[error("IP literal hosts are only allowed in localExceptions")]
    IpLiteralNotAllowed,
    #[error("invalid wildcard pattern")]
    InvalidWildcard,
    #[error("wildcard on public suffix '{0}' is not allowed")]
    PublicSuffixWildcard(String),
}

/// Normalized host representation from a parsed URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OriginHost {
    Domain(String),
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
}

impl OriginHost {
    /// Returns true if this host is an IP literal.
    #[must_use]
    pub fn is_ip(&self) -> bool {
        matches!(self, Self::Ipv4(_) | Self::Ipv6(_))
    }

    /// Formats the host for canonical origin / CSP output.
    #[must_use]
    pub fn to_authority_host(&self) -> String {
        match self {
            Self::Domain(d) => d.clone(),
            Self::Ipv4(ip) => ip.to_string(),
            Self::Ipv6(ip) => format!("[{ip}]"),
        }
    }
}

/// A normalized URL origin `(scheme, lowercased punycode host, port)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParsedOrigin {
    pub scheme: String,
    pub host: OriginHost,
    pub port: u16,
}

impl ParsedOrigin {
    /// Returns the canonical origin string (omitting default ports 80/443).
    #[must_use]
    pub fn as_canonical(&self) -> String {
        let default_port = default_port_for_scheme(&self.scheme);
        if Some(self.port) == default_port {
            format!("{}://{}", self.scheme, self.host.to_authority_host())
        } else {
            format!(
                "{}://{}:{}",
                self.scheme,
                self.host.to_authority_host(),
                self.port
            )
        }
    }

    /// Returns the corresponding WebSocket origin (`http` -> `ws`, `https` -> `wss`).
    #[must_use]
    pub fn to_websocket_origin(&self) -> Option<Self> {
        let ws_scheme = match self.scheme.as_str() {
            "http" => "ws",
            "https" => "wss",
            "ws" | "wss" => self.scheme.as_str(),
            _ => return None,
        };
        Some(Self {
            scheme: ws_scheme.to_string(),
            host: self.host.clone(),
            port: self.port,
        })
    }
}

impl fmt::Display for ParsedOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_canonical())
    }
}

/// A validated allowlist entry: either an exact origin or a leading `*.` subdomain wildcard.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AllowlistPattern {
    Exact(ParsedOrigin),
    WildcardSubdomain {
        scheme: String,
        base_domain: String,
        port: u16,
    },
}

impl AllowlistPattern {
    /// Returns the canonical string representation for serialization/CSP output.
    #[must_use]
    pub fn as_canonical(&self) -> String {
        match self {
            Self::Exact(origin) => origin.as_canonical(),
            Self::WildcardSubdomain {
                scheme,
                base_domain,
                port,
            } => {
                let default_port = default_port_for_scheme(scheme);
                if Some(*port) == default_port {
                    format!("{scheme}://*.{base_domain}")
                } else {
                    format!("{scheme}://*.{base_domain}:{port}")
                }
            }
        }
    }

    /// Returns true if `candidate` matches this pattern.
    ///
    /// Wildcards match one or more subdomain labels under `base_domain` (e.g.
    /// `*.company.com` matches `a.company.com` and `a.b.company.com`), never
    /// the bare `company.com` parent and never `evilcompany.com`.
    #[must_use]
    pub fn matches(&self, candidate: &ParsedOrigin) -> bool {
        match self {
            Self::Exact(origin) => origin == candidate,
            Self::WildcardSubdomain {
                scheme,
                base_domain,
                port,
            } => {
                if &candidate.scheme != scheme || candidate.port != *port {
                    return false;
                }
                let OriginHost::Domain(candidate_host) = &candidate.host else {
                    return false;
                };
                if candidate_host.len() <= base_domain.len() + 1 {
                    return false;
                }
                let suffix_start = candidate_host.len() - base_domain.len();
                candidate_host.as_bytes()[suffix_start - 1] == b'.'
                    && &candidate_host[suffix_start..] == base_domain.as_str()
            }
        }
    }
}

/// Allowed scheme policy for parsing allowlist entries or origins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemePolicy {
    HttpsOnly,
    HttpsOrWss,
    LocalException,
    AnyWeb,
}

impl SchemePolicy {
    fn allows(self, scheme: &str) -> bool {
        match self {
            Self::HttpsOnly => scheme == "https",
            Self::HttpsOrWss => matches!(scheme, "https" | "wss"),
            Self::LocalException | Self::AnyWeb => {
                matches!(scheme, "http" | "https" | "ws" | "wss")
            }
        }
    }
}

/// Returns the default port for standard web/websocket schemes.
#[must_use]
pub fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

/// Extracts and normalizes a `ParsedOrigin` from a full URL (e.g. navigation or IPC URI).
///
/// Rejects userinfo, missing hosts, non-web schemes, and oversized origins/hosts.
pub fn extract_origin(raw_url: &str) -> Result<ParsedOrigin, OriginError> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(OriginError::Empty);
    }
    if trimmed.len() > MAX_ORIGIN_LEN {
        return Err(OriginError::OriginTooLong);
    }

    let parsed = Url::parse(trimmed).map_err(|_| OriginError::InvalidUrl)?;
    let scheme = parsed.scheme().to_ascii_lowercase();
    if !SchemePolicy::AnyWeb.allows(&scheme) {
        return Err(OriginError::DisallowedScheme(scheme));
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(OriginError::UserinfoNotAllowed);
    }
    // Also reject raw `@` in authority even if username is empty (`https://@host`).
    if let Some(after_scheme) = trimmed.split_once("://").map(|(_, rest)| rest) {
        let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
        if authority.contains('@') {
            return Err(OriginError::UserinfoNotAllowed);
        }
    }

    let host = match parsed.host() {
        Some(Host::Domain(d)) => {
            let clean = d.trim_end_matches('.');
            if clean.is_empty() {
                return Err(OriginError::MissingHost);
            }
            if clean.len() > MAX_HOST_LEN {
                return Err(OriginError::HostTooLong);
            }
            OriginHost::Domain(clean.to_ascii_lowercase())
        }
        Some(Host::Ipv4(ip)) => OriginHost::Ipv4(ip),
        Some(Host::Ipv6(ip)) => OriginHost::Ipv6(ip),
        None => return Err(OriginError::MissingHost),
    };

    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| OriginError::DisallowedScheme(scheme.clone()))?;

    Ok(ParsedOrigin { scheme, host, port })
}

/// Parses an exact origin entry (no paths, queries, fragments, wildcards, or userinfo).
pub fn parse_exact_origin(
    raw: &str,
    scheme_policy: SchemePolicy,
    allow_ip: bool,
) -> Result<ParsedOrigin, OriginError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(OriginError::Empty);
    }
    if trimmed.len() > MAX_ORIGIN_LEN {
        return Err(OriginError::OriginTooLong);
    }
    if trimmed.contains('*') {
        return Err(OriginError::InvalidWildcard);
    }

    // Reject any path, query, or fragment after `scheme://authority`.
    let (_, rest) = trimmed.split_once("://").ok_or(OriginError::InvalidUrl)?;
    if rest.contains(['/', '?', '#']) {
        return Err(OriginError::PathQueryOrFragmentNotAllowed);
    }

    let origin = extract_origin(trimmed)?;
    if !scheme_policy.allows(&origin.scheme) {
        return Err(OriginError::DisallowedScheme(origin.scheme));
    }
    if !allow_ip && origin.host.is_ip() {
        return Err(OriginError::IpLiteralNotAllowed);
    }
    Ok(origin)
}

/// Parses an allowlist pattern (exact origin or leading `*.` subdomain wildcard).
pub fn parse_allowlist_pattern(
    raw: &str,
    scheme_policy: SchemePolicy,
) -> Result<AllowlistPattern, OriginError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(OriginError::Empty);
    }
    if trimmed.len() > MAX_ORIGIN_LEN {
        return Err(OriginError::OriginTooLong);
    }

    let (raw_scheme, rest) = trimmed.split_once("://").ok_or(OriginError::InvalidUrl)?;
    if rest.contains(['/', '?', '#']) {
        return Err(OriginError::PathQueryOrFragmentNotAllowed);
    }
    if rest.contains('@') {
        return Err(OriginError::UserinfoNotAllowed);
    }

    if !rest.contains('*') {
        let allow_ip = matches!(scheme_policy, SchemePolicy::LocalException);
        let exact = parse_exact_origin(trimmed, scheme_policy, allow_ip)?;
        return Ok(AllowlistPattern::Exact(exact));
    }

    // Wildcard must be strictly a leading `*.` on the host.
    let scheme = raw_scheme.to_ascii_lowercase();
    if !scheme_policy.allows(&scheme) {
        return Err(OriginError::DisallowedScheme(scheme));
    }
    let after_wildcard = rest
        .strip_prefix("*.")
        .ok_or(OriginError::InvalidWildcard)?;
    if after_wildcard.contains('*') || after_wildcard.is_empty() {
        return Err(OriginError::InvalidWildcard);
    }

    // Parse `scheme://<after_wildcard>` via `url::Url` to get punycode & port normalization.
    let synthetic = format!("{scheme}://{after_wildcard}");
    let base = parse_exact_origin(&synthetic, scheme_policy, false)?;
    let OriginHost::Domain(base_domain) = base.host else {
        return Err(OriginError::InvalidWildcard);
    };

    // Must have at least two labels after `*.` (e.g. `company.com`, never `com`).
    let labels: Vec<&str> = base_domain.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|l| l.is_empty()) {
        return Err(OriginError::InvalidWildcard);
    }

    if is_multi_label_public_suffix(&base_domain) {
        return Err(OriginError::PublicSuffixWildcard(base_domain));
    }

    Ok(AllowlistPattern::WildcardSubdomain {
        scheme: base.scheme,
        base_domain,
        port: base.port,
    })
}

/// Checks whether `domain` is a known multi-label public suffix (e.g. `co.uk`, `github.io`).
///
/// Built-in pure-Rust static table for offline determinism (since `psl` crate is not
/// in the offline cargo cache).
fn is_multi_label_public_suffix(domain: &str) -> bool {
    static SUFFIXES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let set = SUFFIXES.get_or_init(|| {
        [
            "co.uk",
            "org.uk",
            "ac.uk",
            "gov.uk",
            "me.uk",
            "net.uk",
            "ltd.uk",
            "plc.uk",
            "com.au",
            "net.au",
            "org.au",
            "edu.au",
            "gov.au",
            "co.jp",
            "ne.jp",
            "or.jp",
            "ac.jp",
            "go.jp",
            "com.br",
            "net.br",
            "org.br",
            "gov.br",
            "com.cn",
            "net.cn",
            "org.cn",
            "gov.cn",
            "co.in",
            "net.in",
            "org.in",
            "gov.in",
            "ac.in",
            "co.nz",
            "net.nz",
            "org.nz",
            "co.za",
            "org.za",
            "com.mx",
            "com.sg",
            "com.hk",
            "com.tw",
            "co.kr",
            "com.ar",
            "com.tr",
            "com.pl",
            "eu.org",
            "github.io",
            "gitlab.io",
            "herokuapp.com",
            "appspot.com",
            "cloudfront.net",
            "s3.amazonaws.com",
            "pages.dev",
            "workers.dev",
            "vercel.app",
            "netlify.app",
            "azurewebsites.net",
            "blogspot.com",
            "firebaseapp.com",
            "web.app",
        ]
        .into_iter()
        .collect()
    });
    set.contains(domain)
}
