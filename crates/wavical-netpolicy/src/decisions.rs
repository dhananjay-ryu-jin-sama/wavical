//! Pure policy decision functions for navigation, popups, external links, remote UI, and IPC bridge.

use thiserror::Error;
use url::Url;

use crate::NetworkMode;
use crate::allowlist::Allowlist;
use crate::limits::MAX_OPEN_EXTERNAL_URL_LEN;
use crate::origin::{OriginHost, ParsedOrigin, SchemePolicy, extract_origin, parse_exact_origin};

/// Decision outcome for policy checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
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
    let Ok(parsed) = Url::parse(trimmed) else {
        return true;
    };
    !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss" | "wavical")
}

/// Decides whether a top-level navigation to `target_url` is allowed.
#[must_use]
pub fn decide_navigation(ctx: &PolicyContext<'_>, target_url: &str) -> PolicyDecision {
    if is_baseline_blocked_scheme(target_url) {
        return PolicyDecision::Deny("baseline_blocked_scheme");
    }
    let Ok(origin) = extract_origin(target_url) else {
        return PolicyDecision::Deny("invalid_origin");
    };

    if &origin == ctx.app_origin {
        return PolicyDecision::Allow;
    }
    if ctx.is_dev_command
        && let Some(dev) = ctx.dev_url_origin
        && &origin == dev
    {
        return PolicyDecision::Allow;
    }

    match ctx.mode {
        NetworkMode::UltraSafe => PolicyDecision::Deny("ultra_safe_blocks_external_navigation"),
        NetworkMode::Open => {
            if origin.scheme == "https"
                || (origin.scheme == "http" && is_local_exception(ctx.allowlist, &origin))
            {
                PolicyDecision::Allow
            } else {
                PolicyDecision::Deny("http_requires_local_exception")
            }
        }
        NetworkMode::Safe => {
            if let Some(remote) = ctx.remote_url_origin
                && &origin == remote
                && is_remote_ui_listed(ctx.allowlist, remote)
            {
                return PolicyDecision::Allow;
            }
            if let Some(al) = ctx.allowlist {
                if al.navigate.iter().any(|p| p.matches(&origin)) {
                    return PolicyDecision::Allow;
                }
                if al.has_local_exception(&origin) {
                    return PolicyDecision::Allow;
                }
            }
            PolicyDecision::Deny("navigation_not_allowlisted")
        }
    }
}

/// Decides whether a `window.open` / `target="_blank"` request should be routed to `openExternal`
/// or denied. In Wavical, new webview windows are never spawned; allowed requests open in the system browser.
#[must_use]
pub fn decide_new_window(ctx: &PolicyContext<'_>, target_url: &str) -> PolicyDecision {
    decide_open_external(ctx, target_url)
}

/// Decides whether opening `target_url` in the system browser is allowed (Plan 1.2, 1.3, 1.9).
#[must_use]
pub fn decide_open_external(ctx: &PolicyContext<'_>, target_url: &str) -> PolicyDecision {
    let trimmed = target_url.trim();
    if trimmed.len() > MAX_OPEN_EXTERNAL_URL_LEN {
        return PolicyDecision::Deny("open_external_url_too_long");
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
