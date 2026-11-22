//! Pure policy decision functions for navigation, popups, external links, remote UI, and IPC bridge.

use thiserror::Error;
use url::Url;

use crate::allowlist::Allowlist;
use crate::limits::MAX_OPEN_EXTERNAL_URL_LEN;
use crate::origin::{OriginHost, ParsedOrigin, SchemePolicy, extract_origin, parse_exact_origin};
use crate::{ExternalLinksMode, NetworkMode};

/// Decision outcome for policy checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny(&'static str),
}

/// Decision outcome for top-level navigation requests distinguishing in-app navigation
/// from external system-browser / OS-handler opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationDecision {
    AllowInApp,
    OpenExternal,
    Deny(&'static str),
}

/// Bridge authorization outcome for an incoming IPC message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeDecision {
    Allow { granted_capabilities: Vec<String> },
    Deny(&'static str),
}

/// Errors when validating `dev.url`.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DevUrlError {
    #[error("dev.url is invalid: {0}")]
    InvalidOrigin(#[from] crate::origin::OriginError),
    #[error("dev.url must use localhost or 127.0.0.1 with an explicit port")]
    MustBeLoopbackWithPort,
}

/// Runtime policy context passed into decision functions.
#[derive(Debug, Clone)]
pub struct PolicyContext<'a> {
    pub mode: NetworkMode,
    pub is_dev_command: bool,
    pub app_origin: &'a ParsedOrigin,
    pub allowlist: Option<&'a Allowlist>,
    pub remote_url_origin: Option<&'a ParsedOrigin>,
    pub dev_url_origin: Option<&'a ParsedOrigin>,
    pub app_capabilities: &'a [String],
}

/// Validates `dev.url` per Plan 1.12 (`http(s)://localhost:<port>` or `http(s)://127.0.0.1:<port>`).
pub fn validate_dev_url(raw_url: &str) -> Result<ParsedOrigin, DevUrlError> {
    let origin = parse_exact_origin(raw_url, SchemePolicy::LocalException, true)?;
    if !matches!(origin.scheme.as_str(), "http" | "https") {
        return Err(DevUrlError::MustBeLoopbackWithPort);
    }
    let is_loopback = match &origin.host {
        OriginHost::Domain(d) => d == "localhost",
        OriginHost::Ipv4(ip) => ip.is_loopback(),
        OriginHost::Ipv6(ip) => ip.is_loopback(),
    };
    if !is_loopback {
        return Err(DevUrlError::MustBeLoopbackWithPort);
    }
    // Require an explicit `:port` in the input string.
    let (_, rest) = raw_url.trim().split_once("://").unwrap_or(("", ""));
    let has_explicit_port = rest
        .rsplit_once(':')
        .is_some_and(|(_, p)| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !has_explicit_port {
        return Err(DevUrlError::MustBeLoopbackWithPort);
    }
    Ok(origin)
}

/// Returns true if the URL uses a baseline-blocked scheme (`file:`, `javascript:`, `data:`, `blob:`, `ftp:`, `ms-*`, etc.).
#[must_use]
pub fn is_baseline_blocked_scheme(raw_url: &str) -> bool {
    let trimmed = raw_url.trim();
    if trimmed.chars().any(char::is_control) {
        return true;
    }
    let Ok(parsed) = Url::parse(trimmed) else {
        return true;
    };
    !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss" | "wavical")
}

/// Decides whether a top-level navigation to `target_url` is allowed in-app (non-user-initiated / redirect check).
#[must_use]
pub fn decide_navigation(ctx: &PolicyContext<'_>, target_url: &str) -> PolicyDecision {
    match decide_top_level_navigation(ctx, target_url, false, ExternalLinksMode::Browser) {
        NavigationDecision::AllowInApp => PolicyDecision::Allow,
        NavigationDecision::OpenExternal => PolicyDecision::Deny("baseline_blocked_scheme"),
        NavigationDecision::Deny(rule) => PolicyDecision::Deny(rule),
    }
}

/// Decides how a top-level navigation to `target_url` should be handled, taking into account
/// whether it was user-initiated (`<a href>` click) and the configured `window.externalLinks` policy.
#[must_use]
pub fn decide_top_level_navigation(
    ctx: &PolicyContext<'_>,
    target_url: &str,
    is_user_initiated: bool,
    external_links: ExternalLinksMode,
) -> NavigationDecision {
    let trimmed = target_url.trim();
    if trimmed.chars().any(char::is_control) {
        return NavigationDecision::Deny("baseline_blocked_scheme");
    }
    if let Ok(parsed) = Url::parse(trimmed)
        && matches!(parsed.scheme(), "mailto" | "tel")
    {
        return match decide_open_external(ctx, trimmed) {
            PolicyDecision::Allow => NavigationDecision::OpenExternal,
            PolicyDecision::Deny(rule) => NavigationDecision::Deny(rule),
        };
    }

    if is_baseline_blocked_scheme(trimmed) {
        return NavigationDecision::Deny("baseline_blocked_scheme");
    }
    let Ok(origin) = extract_origin(trimmed) else {
        return NavigationDecision::Deny("invalid_origin");
    };

    if &origin == ctx.app_origin {
        return NavigationDecision::AllowInApp;
    }
    if ctx.is_dev_command
        && let Some(dev) = ctx.dev_url_origin
        && &origin == dev
    {
        return NavigationDecision::AllowInApp;
    }

    match ctx.mode {
        NetworkMode::UltraSafe => NavigationDecision::Deny("ultra_safe_blocks_external_navigation"),
        NetworkMode::Open => {
            if origin.scheme == "http" {
                if is_local_exception(ctx.allowlist, &origin) {
                    NavigationDecision::AllowInApp
                } else {
                    NavigationDecision::Deny("http_requires_local_exception")
                }
            } else if ctx.remote_url_origin.is_some_and(|r| &origin == r) {
                NavigationDecision::AllowInApp
            } else if is_user_initiated && external_links == ExternalLinksMode::Browser {
                NavigationDecision::OpenExternal
            } else {
                NavigationDecision::AllowInApp
            }
        }
        NetworkMode::Safe => {
            if let Some(remote) = ctx.remote_url_origin
                && &origin == remote
                && is_remote_ui_listed(ctx.allowlist, remote)
            {
                return NavigationDecision::AllowInApp;
            }
            if let Some(al) = ctx.allowlist {
                if al.navigate.iter().any(|p| p.matches(&origin)) {
                    return NavigationDecision::AllowInApp;
                }
                if al.has_local_exception(&origin) {
                    return NavigationDecision::AllowInApp;
                }
            }
            if is_user_initiated
                && external_links == ExternalLinksMode::Browser
                && let PolicyDecision::Allow = decide_open_external(ctx, trimmed)
            {
                return NavigationDecision::OpenExternal;
            }
            NavigationDecision::Deny("navigation_not_allowlisted")
        }
    }
}

/// Decides whether a `window.open` / `target="_blank"` request should be routed to `openExternal`
/// or denied. In Wavical, new webview windows are never spawned; allowed requests open in the system browser.
#[must_use]
pub fn decide_new_window(ctx: &PolicyContext<'_>, target_url: &str) -> PolicyDecision {
    decide_open_external(ctx, target_url)
}

/// Decides whether opening `target_url` in the system browser or OS scheme handler is allowed
/// (Plan 1.2, 1.3, 1.9, F5).
#[must_use]
pub fn decide_open_external(ctx: &PolicyContext<'_>, target_url: &str) -> PolicyDecision {
    let trimmed = target_url.trim();
    if trimmed.len() > MAX_OPEN_EXTERNAL_URL_LEN {
        return PolicyDecision::Deny("open_external_url_too_long");
    }
    if trimmed.chars().any(char::is_control) {
        return PolicyDecision::Deny("open_external_control_chars");
    }
    let Ok(parsed) = Url::parse(trimmed) else {
        return PolicyDecision::Deny("baseline_blocked_scheme");
    };
    let scheme = parsed.scheme();
    if matches!(scheme, "mailto" | "tel") {
        if let Err(rule) = validate_mailto_or_tel_url(trimmed, &parsed) {
            return PolicyDecision::Deny(rule);
        }
        return match ctx.mode {
            NetworkMode::UltraSafe => PolicyDecision::Deny("ultra_safe_blocks_open_external"),
            NetworkMode::Open => PolicyDecision::Allow,
            NetworkMode::Safe => {
                if let Some(al) = ctx.allowlist
                    && al.allows_external_scheme(scheme)
                {
                    PolicyDecision::Allow
                } else {
                    PolicyDecision::Deny("open_external_scheme_not_allowlisted")
                }
            }
        };
    }

    if is_baseline_blocked_scheme(trimmed) {
        return PolicyDecision::Deny("baseline_blocked_scheme");
    }
    let Ok(origin) = extract_origin(trimmed) else {
        return PolicyDecision::Deny("invalid_origin");
    };
    if origin.scheme != "https" {
        return PolicyDecision::Deny("open_external_requires_https");
    }

    match ctx.mode {
        NetworkMode::UltraSafe => PolicyDecision::Deny("ultra_safe_blocks_open_external"),
        NetworkMode::Open => PolicyDecision::Allow,
        NetworkMode::Safe => {
            if let Some(al) = ctx.allowlist
                && al.open_external.iter().any(|p| p.matches(&origin))
            {
                return PolicyDecision::Allow;
            }
            PolicyDecision::Deny("open_external_not_allowlisted")
        }
    }
}

/// Validates a `mailto:` or `tel:` URI for safe single-argument OS dispatch.
/// Rejects authority (`//`), userinfo, empty targets, control chars, and encoded CR/LF/NUL/control bytes.
pub fn validate_mailto_or_tel_url(raw_trimmed: &str, parsed: &Url) -> Result<(), &'static str> {
    let scheme = parsed.scheme();
    if scheme != "mailto" && scheme != "tel" {
        return Err("baseline_blocked_scheme");
    }
    if raw_trimmed.chars().any(char::is_control) {
        return Err("open_external_control_chars");
    }
    if parsed.has_host()
        || !parsed.cannot_be_a_base()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("open_external_invalid_uri");
    }
    if contains_encoded_control_bytes(raw_trimmed) {
        return Err("open_external_control_chars");
    }
    let path_non_empty = !parsed.path().trim().is_empty();
    let has_valid_target = if scheme == "mailto" {
        path_non_empty || parsed.query().is_some_and(|q| !q.trim().is_empty())
    } else {
        path_non_empty
    };
    if !has_valid_target {
        return Err("open_external_invalid_uri");
    }
    Ok(())
}

fn contains_encoded_control_bytes(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(mut val) = decode_hex_pair(bytes[i + 1], bytes[i + 2])
        {
            // Unwrap up to 4 layers of %25-encoded percent signs (e.g. %250d, %25250a).
            let mut j = i + 3;
            let mut depth = 0;
            while val == b'%' && depth < 4 && j + 1 < bytes.len() {
                if let Some(next_val) = decode_hex_pair(bytes[j], bytes[j + 1]) {
                    val = next_val;
                    j += 2;
                    depth += 1;
                } else {
                    break;
                }
            }
            if val <= 0x1F || val == 0x7F {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn decode_hex_pair(hi: u8, lo: u8) -> Option<u8> {
    let h = hex_nibble(hi)?;
    let l = hex_nibble(lo)?;
    Some((h << 4) | l)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decides whether loading the whole UI from `window.remoteUrl` is allowed (Plan 1.2, 1.6).
#[must_use]
pub fn decide_remote_ui(ctx: &PolicyContext<'_>, remote_url: &str) -> PolicyDecision {
    if is_baseline_blocked_scheme(remote_url) {
        return PolicyDecision::Deny("baseline_blocked_scheme");
    }
    let Ok(origin) = extract_origin(remote_url) else {
        return PolicyDecision::Deny("invalid_origin");
    };
    match ctx.mode {
        NetworkMode::UltraSafe => PolicyDecision::Deny("remote_ui_forbidden_in_ultra_safe"),
        NetworkMode::Open => {
            if origin.scheme == "https" || is_local_exception(ctx.allowlist, &origin) {
                PolicyDecision::Allow
            } else {
                PolicyDecision::Deny("remote_ui_requires_https_or_local_exception")
            }
        }
        NetworkMode::Safe => {
            if is_remote_ui_listed(ctx.allowlist, &origin) {
                PolicyDecision::Allow
            } else {
                PolicyDecision::Deny("remote_ui_origin_not_in_allowlist")
            }
        }
    }
}

/// Decides whether the IPC bridge is allowed for `caller_url` and `is_top_frame` (Plan 1.2, 1.3, 1.6, 1.12).
#[must_use]
pub fn decide_bridge_allowed(
    ctx: &PolicyContext<'_>,
    caller_url: &str,
    is_top_frame: bool,
) -> BridgeDecision {
    if !is_top_frame {
        return BridgeDecision::Deny("bridge_denied_for_subframe");
    }
    if is_baseline_blocked_scheme(caller_url) {
        return BridgeDecision::Deny("baseline_blocked_scheme");
    }
    let Ok(origin) = extract_origin(caller_url) else {
        return BridgeDecision::Deny("invalid_origin");
    };

    if &origin == ctx.app_origin {
        return BridgeDecision::Allow {
            granted_capabilities: ctx.app_capabilities.to_vec(),
        };
    }

    if ctx.is_dev_command
        && let Some(dev) = ctx.dev_url_origin
        && &origin == dev
    {
        return BridgeDecision::Allow {
            granted_capabilities: ctx.app_capabilities.to_vec(),
        };
    }

    if let Some(remote) = ctx.remote_url_origin
        && &origin == remote
    {
        match ctx.mode {
            NetworkMode::UltraSafe => {
                return BridgeDecision::Deny("remote_ui_forbidden_in_ultra_safe");
            }
            NetworkMode::Open => {
                return BridgeDecision::Allow {
                    granted_capabilities: ctx.app_capabilities.to_vec(),
                };
            }
            NetworkMode::Safe => {
                if let Some(al) = ctx.allowlist
                    && let Some(rui) = &al.remote_ui
                    && &rui.origin == remote
                {
                    return BridgeDecision::Allow {
                        granted_capabilities: rui.bridge.clone(),
                    };
                }
                return BridgeDecision::Deny("remote_ui_not_in_allowlist");
            }
        }
    }

    BridgeDecision::Deny("origin_not_authorized_for_bridge")
}

fn is_local_exception(allowlist: Option<&Allowlist>, origin: &ParsedOrigin) -> bool {
    allowlist.is_some_and(|al| al.has_local_exception(origin))
}

fn is_remote_ui_listed(allowlist: Option<&Allowlist>, origin: &ParsedOrigin) -> bool {
    allowlist
        .and_then(|al| al.remote_ui.as_ref())
        .is_some_and(|rui| &rui.origin == origin)
}
