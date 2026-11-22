//! Phase 4.5 Network Modes enforcement for `wavical-shell`.

use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;
use wavical_netpolicy::allowlist::Allowlist;
use wavical_netpolicy::csp::{ULTRA_SAFE_CSP, generate_csp};
use wavical_netpolicy::decisions::{
    BridgeDecision, NavigationDecision, PolicyContext, PolicyDecision, decide_bridge_allowed,
    decide_navigation, decide_open_external, decide_remote_ui, decide_top_level_navigation,
    validate_dev_url,
};
use wavical_netpolicy::diagnostics::{DevDiagnosticEvent, normalize_csp_directive};
#[cfg(feature = "dev-shell")]
use wavical_netpolicy::diagnostics::{
    MAX_JS_ERROR_CHARS, map_directive_to_resource_type_and_allowlist_key, sanitize_location,
    sanitize_single_line, truncate_chars,
};
use wavical_netpolicy::html::CspHash;
use wavical_netpolicy::limits::{
    MAX_RECORDED_EVENTS_PER_SESSION, OPEN_EXTERNAL_RATE_LIMIT_COUNT,
    OPEN_EXTERNAL_RATE_LIMIT_WINDOW_SECS,
};
use wavical_netpolicy::origin::{
    AllowlistPattern, OriginHost, ParsedOrigin, default_port_for_scheme, extract_origin,
};
use wavical_netpolicy::{ExternalLinksMode, NetworkMode};
use wavical_protocol::{HEADER_NOSNIFF, resolve_asset_path};
use wry::NewWindowResponse;
use wry::http::{Request, Response, StatusCode, Uri};

use crate::{HEADER_CSP, build_safe_404_response};

/// Response header used in `ultra-safe` mode to disable speculative DNS prefetching.
pub const HEADER_DNS_PREFETCH_CONTROL: &str = "X-DNS-Prefetch-Control";

/// WebView2 additional browser arguments for `ultra-safe` mode (WP1 Item 9).
/// Preserves `wry`'s default disabled features (`msWebOOUI,msPdfOOUI,msSmartScreenProtection`).
pub const ULTRA_SAFE_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,OptimizationHints,NetworkTimeServiceQuerying --dns-prefetch-disable";

/// Errors returned when validating/opening an external URL in the system browser.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OpenExternalError {
    #[error("external link denied by policy: {0}")]
    PolicyDenied(&'static str),
    #[error("external link rate limit exceeded")]
    RateLimited,
    #[error("failed to launch system browser")]
    LaunchFailed,
}

/// A single recorded blocked event entry stored in `.wavical/recorded-blocks.json` (Plan 1.7).
/// Never stores full URLs, query strings, or content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedBlockEntry {
    #[serde(rename = "type")]
    pub block_type: String,
    pub origin: String,
    pub count: usize,
}

/// On-disk format for `.wavical/recorded-blocks.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RecordedBlocksFile {
    pub version: u32,
    pub total_events: usize,
    pub entries: Vec<RecordedBlockEntry>,
}

/// Bounded in-session recorder for `--record` on `dev-safe` and `dev-ultra-safe`.
#[derive(Debug)]
pub struct BlockRecorder {
    out_path: Option<PathBuf>,
    state: Mutex<RecordedBlocksFile>,
}

impl BlockRecorder {
    /// Creates a new recorder optionally backed by `.wavical/recorded-blocks.json`.
    #[must_use]
    pub fn new(out_path: Option<PathBuf>) -> Self {
        Self {
            out_path,
            state: Mutex::new(RecordedBlocksFile {
                version: 1,
                total_events: 0,
                entries: Vec::new(),
            }),
        }
    }

    /// Records a blocked event, storing only `(block_type, normalized_origin, count)`.
    /// Enforces `MAX_RECORDED_EVENTS_PER_SESSION` (10,000).
    pub fn record_raw(&self, block_type: &str, raw_uri_or_kind: &str) {
        let origin = sanitize_recorded_origin(raw_uri_or_kind);
        let mut guard = match self.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.total_events >= MAX_RECORDED_EVENTS_PER_SESSION {
            return;
        }
        guard.total_events += 1;
        if let Some(existing) = guard
            .entries
            .iter_mut()
            .find(|e| e.block_type == block_type && e.origin == origin)
        {
            existing.count += 1;
        } else {
            guard.entries.push(RecordedBlockEntry {
                block_type: block_type.to_string(),
                origin,
                count: 1,
            });
        }
        if let Some(ref path) = self.out_path {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(&*guard) {
                let _ = fs::write(path, json);
            }
        }
    }

    /// Returns a snapshot of recorded entries.
    #[must_use]
    pub fn snapshot(&self) -> RecordedBlocksFile {
        match self.state.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// Maps a CSP `effectiveDirective` to a Wavical allowlist category.
#[must_use]
pub fn map_csp_directive_to_type(directive: &str) -> &'static str {
    let d = directive.trim().to_ascii_lowercase();
    if d.starts_with("script-src") {
        "scripts"
    } else if d.starts_with("style-src") {
        "styles"
    } else if d.starts_with("font-src") {
        "fonts"
    } else if d.starts_with("img-src") {
        "images"
    } else if d.starts_with("media-src") {
        "media"
    } else if d.starts_with("connect-src") {
        "connect"
    } else if d.starts_with("frame-src") {
        "frames"
    } else {
        "other"
    }
}

fn sanitize_recorded_origin(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("inline") || trimmed.eq_ignore_ascii_case("'inline'") {
        return "inline".to_string();
    }
    if trimmed.eq_ignore_ascii_case("eval") || trimmed.eq_ignore_ascii_case("'eval'") {
        return "eval".to_string();
    }
    if trimmed.to_ascii_lowercase().starts_with("data:") {
        return "data:".to_string();
    }
    if let Ok(origin) = extract_origin(trimmed) {
        return origin.as_canonical();
    }
    if let Ok(parsed) = Url::parse(trimmed) {
        return format!("{}:", parsed.scheme().to_ascii_lowercase());
    }
    "unknown".to_string()
}

/// Subset of `wavical-manifest.json` read by `wavical-shell` at startup for built apps.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShellBuildManifest {
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub allowlist_sha256: Option<String>,
    #[serde(default)]
    pub remote_url: Option<String>,
    #[serde(default)]
    pub external_links: Option<String>,
    #[serde(default)]
    pub inline_script_hashes: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub assets: BTreeMap<String, String>,
}

/// Resolved network policy for a `wavical-shell` window.
#[derive(Debug, Clone)]
pub struct ShellNetworkPolicy {
    pub mode: NetworkMode,
    pub is_dev_command: bool,
    pub app_origin: ParsedOrigin,
    pub allowlist: Option<Allowlist>,
    pub remote_url: Option<String>,
    pub remote_url_origin: Option<ParsedOrigin>,
    pub dev_url_origin: Option<ParsedOrigin>,
    pub external_links: ExternalLinksMode,
    pub app_capabilities: Vec<String>,
    /// Per-page inline script hashes (keyed by relative path like `"index.html"`).
    pub inline_script_hashes: BTreeMap<String, Vec<CspHash>>,
    /// Expected SHA-256 hex digests for built-app serve-time integrity verification.
    pub manifest_assets: Option<BTreeMap<String, String>>,
    /// Optional `--record` block recorder (dev-safe / dev-ultra-safe only).
    pub recorder: Option<Arc<BlockRecorder>>,
}

impl Default for ShellNetworkPolicy {
    fn default() -> Self {
        Self {
            mode: NetworkMode::UltraSafe,
            is_dev_command: false,
            app_origin: ParsedOrigin {
                scheme: "http".to_string(),
                host: OriginHost::Domain("wavical.localhost".to_string()),
                port: 80,
            },
            allowlist: None,
            remote_url: None,
            remote_url_origin: None,
            dev_url_origin: None,
            external_links: ExternalLinksMode::Browser,
            app_capabilities: Vec::new(),
            inline_script_hashes: BTreeMap::new(),
            manifest_assets: None,
            recorder: None,
        }
    }
}

impl ShellNetworkPolicy {
    /// Returns true if `uri` matches the configured `remote_url_origin`.
    #[must_use]
    pub fn is_remote_ui_origin(&self, uri: &Uri) -> bool {
        let Some(ref remote) = self.remote_url_origin else {
            return false;
        };
        extract_origin(&uri.to_string()).is_ok_and(|o| &o == remote)
    }

    /// Builds a `PolicyContext` for calling `wavical-netpolicy` decision functions.
    #[must_use]
    pub fn as_context(&self) -> PolicyContext<'_> {
        // Plan 1.2 / Task 5: localExceptions are refused in ultra-safe.
        let effective_allowlist = if self.mode == NetworkMode::UltraSafe {
            None
        } else {
            self.allowlist.as_ref()
        };
        let effective_dev_url = if self.is_dev_command {
            self.dev_url_origin.as_ref()
        } else {
            None
        };
        PolicyContext {
            mode: self.mode,
            is_dev_command: self.is_dev_command,
            app_origin: &self.app_origin,
            allowlist: effective_allowlist,
            remote_url_origin: self.remote_url_origin.as_ref(),
            dev_url_origin: effective_dev_url,
            app_capabilities: &self.app_capabilities,
        }
    }

    /// Generates the CSP string for a specific relative asset path (e.g. `"index.html"`).
    #[must_use]
    pub fn csp_for_path(&self, rel_path: &str) -> String {
        if self.mode == NetworkMode::UltraSafe {
            return ULTRA_SAFE_CSP.to_string();
        }
        let normalized = rel_path.trim_start_matches('/');
        let hashes = self
            .inline_script_hashes
            .get(normalized)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let ctx = self.as_context();
        generate_csp(self.mode, ctx.allowlist, hashes, ctx.dev_url_origin)
            .unwrap_or_else(|_| ULTRA_SAFE_CSP.to_string())
    }
}

/// Sliding-window rate limiter for `openExternal` calls (5 per 10 seconds per window).
#[derive(Debug, Default)]
pub struct OpenExternalRateLimiter {
    timestamps: Mutex<VecDeque<Instant>>,
}

impl OpenExternalRateLimiter {
    /// Records an attempt at `now`; returns `true` if within limit, `false` if rate-limited.
    pub fn check_and_record(&self, now: Instant) -> bool {
        let window = Duration::from_secs(OPEN_EXTERNAL_RATE_LIMIT_WINDOW_SECS);
        let mut guard = match self.timestamps.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        while let Some(&front) = guard.front() {
            if now.duration_since(front) >= window {
                guard.pop_front();
            } else {
                break;
            }
        }
        if guard.len() >= OPEN_EXTERNAL_RATE_LIMIT_COUNT {
            return false;
        }
        guard.push_back(now);
        true
    }
}

/// Parses `window.externalLinks` strictly (`"browser"` or `"in-app"`).
#[must_use]
pub fn parse_external_links_mode(raw: &str) -> Option<ExternalLinksMode> {
    match raw.trim() {
        "browser" => Some(ExternalLinksMode::Browser),
        "in-app" => Some(ExternalLinksMode::InApp),
        _ => None,
    }
}

/// Parses a mode string strictly (`"open"`, `"safe"`, `"ultra-safe"`).
#[must_use]
pub fn parse_network_mode(raw: &str) -> Option<NetworkMode> {
    match raw.trim() {
        "open" => Some(NetworkMode::Open),
        "safe" => Some(NetworkMode::Safe),
        "ultra-safe" => Some(NetworkMode::UltraSafe),
        _ => None,
    }
}

/// Formats `NetworkMode` for structured logs.
#[must_use]
pub fn mode_label(mode: NetworkMode) -> &'static str {
    match mode {
        NetworkMode::Open => "open",
        NetworkMode::Safe => "safe",
        NetworkMode::UltraSafe => "ultra-safe",
    }
}

/// Extracts only the `scheme://host[:port]` origin (never query/path) for safe logging (Plan 1.8).
#[must_use]
pub fn safe_log_origin(raw_url: &str) -> String {
    if let Ok(origin) = extract_origin(raw_url) {
        return origin.as_canonical();
    }
    if let Ok(parsed) = Url::parse(raw_url.trim()) {
        return format!("{}:", parsed.scheme());
    }
    "<invalid>".to_string()
}

/// Shared allowlist SHA-256 comparison used by both startup integrity refusal and policy resolution.
#[must_use]
pub fn verify_allowlist_sha256(allowlist_bytes: &[u8], expected_sha256: Option<&str>) -> bool {
    let actual_hash = hex::encode(Sha256::digest(allowlist_bytes));
    expected_sha256 == Some(actual_hash.as_str())
}

fn find_allowlist_path(manifest_path: &Path, project_root: Option<&Path>) -> Option<PathBuf> {
    project_root
        .map(|p| p.join("wavical.allowlist.json"))
        .filter(|p| p.is_file())
        .or_else(|| {
            manifest_path
                .parent()
                .map(|p| p.join("wavical.allowlist.json"))
                .filter(|p| p.is_file())
        })
}

/// Reads `appId` from `wavical-manifest.json` if present and non-empty.
#[must_use]
pub fn resolve_manifest_app_id(asset_root: &Path, project_root: Option<&Path>) -> Option<String> {
    let m_path = find_manifest_path(asset_root, project_root)?;
    let raw_bytes = fs::read(&m_path).ok()?;
    let manifest = serde_json::from_slice::<ShellBuildManifest>(&raw_bytes).ok()?;
    manifest
        .app_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Verifies a built safe-mode app's `wavical.allowlist.json` against `manifest.allowlistSha256`
/// before any window or WebView2 instance is created.
///
/// Only runs when `!is_dev_command` and `wavical-manifest.json` explicitly sets `"mode": "safe"`.
/// Uses the exact same [`verify_allowlist_sha256`] check as [`resolve_shell_network_policy_with_remote`].
pub fn verify_safe_mode_startup_integrity(
    asset_root: &Path,
    project_root: Option<&Path>,
    is_dev_command: bool,
) -> Result<(), String> {
    if is_dev_command {
        return Ok(());
    }
    let Some(m_path) = find_manifest_path(asset_root, project_root) else {
        return Ok(());
    };
    let Ok(raw_bytes) = fs::read(&m_path) else {
        return Ok(());
    };
    let Ok(manifest) = serde_json::from_slice::<ShellBuildManifest>(&raw_bytes) else {
        return Ok(());
    };
    if manifest.mode.as_deref().and_then(parse_network_mode) != Some(NetworkMode::Safe) {
        return Ok(());
    }

    let Some(al_path) = find_allowlist_path(&m_path, project_root) else {
        return Err("safe-mode allowlist missing (wavical.allowlist.json not found)".to_string());
    };
    let al_bytes =
        fs::read(&al_path).map_err(|_| "safe-mode allowlist could not be read".to_string())?;
    if !verify_allowlist_sha256(&al_bytes, manifest.allowlist_sha256.as_deref()) {
        return Err(
            "safe-mode allowlist hash mismatch (wavical.allowlist.json was modified)".to_string(),
        );
    }
    if Allowlist::parse_bytes(&al_bytes).is_err() {
        return Err("safe-mode allowlist is invalid".to_string());
    }
    Ok(())
}

/// Resolves the shell's `ShellNetworkPolicy` from either the build manifest (built apps)
/// or dev CLI flags (dev commands only).
///
/// Built apps ignore `dev_mode_override` and `dev_url_raw`, and fall back to `ultra-safe`
/// if `wavical-manifest.json` or its `mode` field is missing or invalid.
pub fn resolve_shell_network_policy(
    asset_root: &Path,
    project_root: Option<&Path>,
    is_dev_command: bool,
    dev_mode_override: Option<&str>,
    dev_url_raw: Option<&str>,
    app_capabilities: Vec<String>,
) -> ShellNetworkPolicy {
    resolve_shell_network_policy_with_remote(
        asset_root,
        project_root,
        is_dev_command,
        dev_mode_override,
        dev_url_raw,
        None,
        app_capabilities,
    )
}

/// Full policy resolver supporting `window.remoteUrl` (from manifest in built apps, or CLI in dev).
pub fn resolve_shell_network_policy_with_remote(
    asset_root: &Path,
    project_root: Option<&Path>,
    is_dev_command: bool,
    dev_mode_override: Option<&str>,
    dev_url_raw: Option<&str>,
    dev_remote_url_raw: Option<&str>,
    app_capabilities: Vec<String>,
) -> ShellNetworkPolicy {
    resolve_shell_network_policy_with_options(
        asset_root,
        project_root,
        is_dev_command,
        dev_mode_override,
        dev_url_raw,
        dev_remote_url_raw,
        None,
        app_capabilities,
    )
}

/// Full policy resolver supporting `window.remoteUrl` and `window.externalLinks`
/// (from manifest in built apps, or CLI/dev-channel in dev commands).
#[allow(clippy::too_many_arguments)]
pub fn resolve_shell_network_policy_with_options(
    asset_root: &Path,
    project_root: Option<&Path>,
    is_dev_command: bool,
    dev_mode_override: Option<&str>,
    dev_url_raw: Option<&str>,
    dev_remote_url_raw: Option<&str>,
    dev_external_links_raw: Option<&str>,
    app_capabilities: Vec<String>,
) -> ShellNetworkPolicy {
    let mut policy = ShellNetworkPolicy {
        is_dev_command,
        app_capabilities,
        ..ShellNetworkPolicy::default()
    };
    let candidate_remote_url: Option<String>;

    if !is_dev_command {
        // Built app: ignore dev flags/env overrides; load strictly from manifest.
        let manifest_path = find_manifest_path(asset_root, project_root);
        let Some(m_path) = manifest_path else {
            tracing::warn!(
                security_event = "manifest_mode_fallback",
                mode = "ultra-safe",
                reason = "wavical-manifest.json not found; falling back to ultra-safe"
            );
            return policy;
        };

        let Ok(raw_bytes) = fs::read(&m_path) else {
            tracing::warn!(
                security_event = "manifest_mode_fallback",
                mode = "ultra-safe",
                reason = "failed to read wavical-manifest.json; falling back to ultra-safe"
            );
            return policy;
        };

        let Ok(manifest) = serde_json::from_slice::<ShellBuildManifest>(&raw_bytes) else {
            tracing::warn!(
                security_event = "manifest_mode_fallback",
                mode = "ultra-safe",
                reason = "invalid wavical-manifest.json; falling back to ultra-safe"
            );
            return policy;
        };

        let Some(mode_str) = manifest.mode.as_deref() else {
            tracing::warn!(
                security_event = "manifest_mode_fallback",
                mode = "ultra-safe",
                reason = "manifest has no mode: app runs in ultra-safe",
                "manifest has no mode: app runs in ultra-safe"
            );
            policy.manifest_assets = Some(manifest.assets);
            return policy;
        };

        let Some(mode) = parse_network_mode(mode_str) else {
            tracing::warn!(
                security_event = "manifest_mode_fallback",
                mode = "ultra-safe",
                reason = "manifest has invalid mode; falling back to ultra-safe"
            );
            policy.manifest_assets = Some(manifest.assets);
            return policy;
        };

        policy.mode = mode;
        policy.manifest_assets = Some(manifest.assets);
        policy.external_links = manifest
            .external_links
            .as_deref()
            .and_then(parse_external_links_mode)
            .unwrap_or(ExternalLinksMode::Browser);
        candidate_remote_url = manifest.remote_url;

        for (page, tokens) in manifest.inline_script_hashes {
            let parsed_hashes = tokens
                .into_iter()
                .filter_map(|t| {
                    t.strip_prefix("'sha256-")
                        .and_then(|rest| rest.strip_suffix('\''))
                        .map(|b64| CspHash {
                            base64_digest: b64.to_string(),
                        })
                })
                .collect();
            policy.inline_script_hashes.insert(page, parsed_hashes);
        }

        if matches!(mode, NetworkMode::Open | NetworkMode::Safe) {
            let allowlist_path = find_allowlist_path(&m_path, project_root);

            if let Some(al_path) = allowlist_path {
                if let Ok(al_bytes) = fs::read(&al_path) {
                    if mode == NetworkMode::Safe
                        && !verify_allowlist_sha256(&al_bytes, manifest.allowlist_sha256.as_deref())
                    {
                        tracing::error!(
                            security_event = "allowlist_hash_mismatch",
                            mode = "ultra-safe",
                            reason =
                                "wavical.allowlist.json hash mismatch; falling back to ultra-safe"
                        );
                        policy.mode = NetworkMode::UltraSafe;
                        return policy;
                    }
                    match Allowlist::parse_bytes(&al_bytes) {
                        Ok(al) => policy.allowlist = Some(al),
                        Err(_) if mode == NetworkMode::Safe => {
                            tracing::error!(
                                security_event = "allowlist_invalid",
                                mode = "ultra-safe",
                                reason =
                                    "invalid wavical.allowlist.json; falling back to ultra-safe"
                            );
                            policy.mode = NetworkMode::UltraSafe;
                            return policy;
                        }
                        Err(_) => {}
                    }
                }
            } else if mode == NetworkMode::Safe {
                tracing::error!(
                    security_event = "allowlist_missing",
                    mode = "ultra-safe",
                    reason =
                        "wavical.allowlist.json missing in safe build; falling back to ultra-safe"
                );
                policy.mode = NetworkMode::UltraSafe;
                return policy;
            }
        }
    } else {
        // Dev command
        policy.mode = dev_mode_override
            .and_then(parse_network_mode)
            .unwrap_or(NetworkMode::UltraSafe);
        policy.external_links = dev_external_links_raw
            .and_then(parse_external_links_mode)
            .unwrap_or(ExternalLinksMode::Browser);
        candidate_remote_url = dev_remote_url_raw.map(str::to_string);

        if let Some(raw_dev_url) = dev_url_raw {
            match validate_dev_url(raw_dev_url) {
                Ok(dev_origin) => policy.dev_url_origin = Some(dev_origin),
                Err(_) => {
                    tracing::warn!(
                        security_event = "invalid_dev_url",
                        mode = mode_label(policy.mode),
                        reason = "dev.url failed validation and was ignored"
                    );
                }
            }
        }

        if matches!(policy.mode, NetworkMode::Open | NetworkMode::Safe)
            && let Some(root) = project_root
        {
            let al_path = root.join("wavical.allowlist.json");
            if al_path.is_file()
                && let Ok(bytes) = fs::read(&al_path)
                && let Ok(al) = Allowlist::parse_bytes(&bytes)
            {
                policy.allowlist = Some(al);
            }
        }
    }

    if let Some(raw_remote) = candidate_remote_url {
        let ctx = policy.as_context();
        if decide_remote_ui(&ctx, &raw_remote) == PolicyDecision::Allow
            && let Ok(remote_origin) = extract_origin(&raw_remote)
        {
            policy.remote_url = Some(raw_remote);
            policy.remote_url_origin = Some(remote_origin);
        } else {
            tracing::warn!(
                security_event = "remote_ui_rejected",
                mode = mode_label(policy.mode),
                origin = %safe_log_origin(&raw_remote),
                "window.remoteUrl is not allowed in this mode/allowlist and was ignored"
            );
        }
    }

    tracing::info!(
        security_event = "shell_startup",
        mode = mode_label(policy.mode),
        is_dev = policy.is_dev_command,
        has_remote_ui = policy.remote_url.is_some(),
        "Wavical shell network policy initialized"
    );

    policy
}

fn find_manifest_path(
    asset_root: &Path,
    project_root: Option<&Path>,
) -> Option<std::path::PathBuf> {
    let direct = asset_root.join("wavical-manifest.json");
    if direct.is_file() {
        return Some(direct);
    }
    if let Some(parent) = asset_root.parent() {
        let p = parent.join("wavical-manifest.json");
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(root) = project_root {
        let p = root.join("wavical-manifest.json");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Protocol request handler enforcing per-mode CSP, `X-Content-Type-Options: nosniff`,
/// `X-DNS-Prefetch-Control: off` (in `ultra-safe`), and built-app serve-time SHA-256 verification.
pub fn handle_protocol_request_with_policy(
    request: Request<Vec<u8>>,
    asset_root: &Path,
    policy: &ShellNetworkPolicy,
    window_id: &str,
) -> Response<Cow<'static, [u8]>> {
    let req_path = request.uri().path();
    let rel_key = req_path.trim_start_matches('/');
    let rel_key = if rel_key.is_empty() {
        "index.html"
    } else {
        rel_key
    };
    let csp = policy.csp_for_path(rel_key);

    let add_common_headers =
        |builder: wry::http::response::Builder| -> wry::http::response::Builder {
            let b = builder
                .header("Content-Type", "text/plain; charset=utf-8")
                .header(HEADER_NOSNIFF.0, HEADER_NOSNIFF.1)
                .header(HEADER_CSP, csp.as_str());
            if policy.mode == NetworkMode::UltraSafe {
                b.header(HEADER_DNS_PREFETCH_CONTROL, "off")
            } else {
                b
            }
        };

    match resolve_asset_path(req_path, asset_root) {
        Ok(resolved) => match fs::read(&resolved.path) {
            Ok(bytes) => {
                // Task 6: Serve-time SHA-256 verification in built apps
                if !policy.is_dev_command
                    && let Some(ref manifest_assets) = policy.manifest_assets
                {
                    let actual_hash = hex::encode(Sha256::digest(&bytes));
                    let expected = manifest_assets
                        .get(rel_key)
                        .map(|s| s.strip_prefix("sha256:").unwrap_or(s.as_str()));
                    if expected != Some(actual_hash.as_str()) {
                        tracing::warn!(
                            security_event = "asset_hash_mismatch",
                            mode = mode_label(policy.mode),
                            window_id = %window_id,
                            rule = "serve_time_sha256",
                            "Asset SHA-256 mismatch against build manifest; serving 404"
                        );
                        return build_safe_404_with_headers(
                            &csp,
                            policy.mode == NetworkMode::UltraSafe,
                        );
                    }
                }

                let mut builder = Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", resolved.mime_type)
                    .header(HEADER_NOSNIFF.0, HEADER_NOSNIFF.1)
                    .header(HEADER_CSP, csp.as_str());
                if policy.mode == NetworkMode::UltraSafe {
                    builder = builder.header(HEADER_DNS_PREFETCH_CONTROL, "off");
                }
                builder.body(Cow::Owned(bytes)).unwrap_or_else(|_| {
                    build_safe_404_with_headers(&csp, policy.mode == NetworkMode::UltraSafe)
                })
            }
            Err(_) => build_safe_404_with_headers(&csp, policy.mode == NetworkMode::UltraSafe),
        },
        Err(rejection) => {
            tracing::warn!(
                security_event = "path_rejected",
                mode = mode_label(policy.mode),
                window_id = %window_id,
                rule = "path_sanitization",
                reason = %rejection,
                "Security check failed: protocol path rejected"
            );
            let _ = add_common_headers;
            build_safe_404_with_headers(&csp, policy.mode == NetworkMode::UltraSafe)
        }
    }
}

fn build_safe_404_with_headers(csp: &str, ultra_safe: bool) -> Response<Cow<'static, [u8]>> {
    if !ultra_safe {
        return build_safe_404_response(csp);
    }
    let not_found_bytes: &'static [u8] = b"404 Not Found";
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header(HEADER_NOSNIFF.0, HEADER_NOSNIFF.1)
        .header(HEADER_CSP, csp)
        .header(HEADER_DNS_PREFETCH_CONTROL, "off")
        .body(Cow::Borrowed(not_found_bytes))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(not_found_bytes)))
}

/// Outcome of handling a user-initiated top-level external link click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalLinkClickOutcome {
    NavigateInApp,
    OpenedExternal,
    Blocked(&'static str),
    RateLimited,
    LaunchFailed,
}

/// Top-level navigation guard backed by `wavical-netpolicy` that also routes `mailto:` and `tel:`
/// navigations through the single validated opener without navigating the webview.
pub fn handle_navigation_with_policy_and_opener<F>(
    policy: &ShellNetworkPolicy,
    url_str: &str,
    window_id: &str,
    rate_limiter: &OpenExternalRateLimiter,
    opener: F,
) -> bool
where
    F: FnOnce(&str) -> Result<(), std::io::Error>,
{
    let trimmed = url_str.trim();
    if let Ok(parsed) = Url::parse(trimmed)
        && matches!(parsed.scheme(), "mailto" | "tel")
    {
        let res = open_external_validated(policy, trimmed, window_id, rate_limiter, opener);
        #[cfg(feature = "dev-shell")]
        if policy.is_dev_command {
            match res {
                Err(OpenExternalError::PolicyDenied(rule)) => {
                    crate::dev_channel::emit_dev_diagnostic(
                        &DevDiagnosticEvent::BlockedNavigation {
                            origin: safe_log_origin(trimmed),
                            rule: rule.to_string(),
                        },
                    );
                }
                Err(OpenExternalError::RateLimited) => {
                    crate::dev_channel::emit_dev_diagnostic(
                        &DevDiagnosticEvent::BlockedNavigation {
                            origin: safe_log_origin(trimmed),
                            rule: "rate_limit_exceeded".to_string(),
                        },
                    );
                }
                Ok(()) | Err(OpenExternalError::LaunchFailed) => {}
            }
        }
        let _ = res;
        return false;
    }

    handle_navigation_with_policy(policy, url_str, window_id)
}

/// Handles a user-initiated top-level link click (`<a href>`) according to `window.externalLinks`
/// (`"browser"` vs `"in-app"`), `mailto:`/`tel:` rules, and mode allowlists.
pub fn handle_external_link_click<F>(
    policy: &ShellNetworkPolicy,
    url_str: &str,
    window_id: &str,
    rate_limiter: &OpenExternalRateLimiter,
    opener: F,
) -> ExternalLinkClickOutcome
where
    F: FnOnce(&str) -> Result<(), std::io::Error>,
{
    let ctx = policy.as_context();
    match decide_top_level_navigation(&ctx, url_str, true, policy.external_links) {
        NavigationDecision::AllowInApp => ExternalLinkClickOutcome::NavigateInApp,
        NavigationDecision::OpenExternal => {
            match open_external_validated(policy, url_str, window_id, rate_limiter, opener) {
                Ok(()) => ExternalLinkClickOutcome::OpenedExternal,
                Err(OpenExternalError::PolicyDenied(rule)) => {
                    ExternalLinkClickOutcome::Blocked(rule)
                }
                Err(OpenExternalError::RateLimited) => ExternalLinkClickOutcome::RateLimited,
                Err(OpenExternalError::LaunchFailed) => ExternalLinkClickOutcome::LaunchFailed,
            }
        }
        NavigationDecision::Deny(rule) => {
            if let Some(ref rec) = policy.recorder {
                rec.record_raw("openExternal", url_str);
            }
            tracing::warn!(
                security_event = "open_external_blocked",
                mode = mode_label(policy.mode),
                block_type = "openExternal",
                origin = %safe_log_origin(url_str),
                window_id = %window_id,
                rule = rule,
                "Security check failed: external link blocked"
            );
            ExternalLinkClickOutcome::Blocked(rule)
        }
    }
}

/// Top-level navigation guard backed by `wavical-netpolicy`.
pub fn handle_navigation_with_policy(
    policy: &ShellNetworkPolicy,
    url_str: &str,
    window_id: &str,
) -> bool {
    let ctx = policy.as_context();
    match decide_navigation(&ctx, url_str) {
        PolicyDecision::Allow => true,
        PolicyDecision::Deny(rule) => {
            if let Some(ref rec) = policy.recorder {
                rec.record_raw("navigate", url_str);
            }
            let safe_origin = safe_log_origin(url_str);
            tracing::warn!(
                security_event = "navigation_blocked",
                mode = mode_label(policy.mode),
                block_type = "navigate",
                origin = %safe_origin,
                window_id = %window_id,
                rule = rule,
                "Security check failed: navigation blocked"
            );
            #[cfg(feature = "dev-shell")]
            if policy.is_dev_command {
                crate::dev_channel::emit_dev_diagnostic(&DevDiagnosticEvent::BlockedNavigation {
                    origin: safe_origin,
                    rule: rule.to_string(),
                });
            }
            false
        }
    }
}

/// One validated function for opening an external `https://`, `mailto:`, or `tel:` link in the
/// system browser or OS scheme handler (Plan 1.3, 1.9, WP4 Task 3, F5).
pub fn open_external_validated<F>(
    policy: &ShellNetworkPolicy,
    url_str: &str,
    window_id: &str,
    rate_limiter: &OpenExternalRateLimiter,
    opener: F,
) -> Result<(), OpenExternalError>
where
    F: FnOnce(&str) -> Result<(), std::io::Error>,
{
    let ctx = policy.as_context();
    if let PolicyDecision::Deny(rule) = decide_open_external(&ctx, url_str) {
        if let Some(ref rec) = policy.recorder {
            rec.record_raw("openExternal", url_str);
        }
        tracing::warn!(
            security_event = "open_external_blocked",
            mode = mode_label(policy.mode),
            block_type = "openExternal",
            origin = %safe_log_origin(url_str),
            window_id = %window_id,
            rule = rule,
            "Security check failed: external link blocked"
        );
        return Err(OpenExternalError::PolicyDenied(rule));
    }

    if !rate_limiter.check_and_record(Instant::now()) {
        tracing::warn!(
            security_event = "open_external_rate_limited",
            mode = mode_label(policy.mode),
            block_type = "openExternal",
            origin = %safe_log_origin(url_str),
            window_id = %window_id,
            rule = "rate_limit_exceeded",
            "Security check failed: external link rate limit exceeded"
        );
        return Err(OpenExternalError::RateLimited);
    }

    let parsed = Url::parse(url_str.trim()).map_err(|_| OpenExternalError::LaunchFailed)?;
    tracing::info!(
        security_event = "open_external_allowed",
        mode = mode_label(policy.mode),
        window_id = %window_id,
        scheme = %parsed.scheme(),
        "Opened external link via validated opener"
    );
    opener(parsed.as_str()).map_err(|_| OpenExternalError::LaunchFailed)
}

/// Default Windows system browser launcher using `rundll32.exe url.dll,FileProtocolHandler`
/// (no `cmd.exe`, zero `unsafe`).
pub fn launch_system_browser_windows(validated_https_url: &str) -> Result<(), std::io::Error> {
    Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(validated_https_url)
        .spawn()
        .map(|_| ())
}

/// New-window / popup handler backed by `open_external_validated`.
/// Always denies spawning a new webview window; allowed `https://` links open in the system browser.
pub fn handle_new_window_with_policy(
    policy: &ShellNetworkPolicy,
    url_str: &str,
    window_id: &str,
    rate_limiter: &Arc<OpenExternalRateLimiter>,
) -> NewWindowResponse {
    let res = open_external_validated(
        policy,
        url_str,
        window_id,
        rate_limiter,
        launch_system_browser_windows,
    );
    #[cfg(feature = "dev-shell")]
    if policy.is_dev_command {
        match res {
            Err(OpenExternalError::PolicyDenied(rule)) => {
                crate::dev_channel::emit_dev_diagnostic(&DevDiagnosticEvent::BlockedNewWindow {
                    origin: safe_log_origin(url_str),
                    rule: rule.to_string(),
                });
            }
            Err(OpenExternalError::RateLimited) => {
                crate::dev_channel::emit_dev_diagnostic(&DevDiagnosticEvent::BlockedNewWindow {
                    origin: safe_log_origin(url_str),
                    rule: "rate_limit_exceeded".to_string(),
                });
            }
            Ok(()) | Err(OpenExternalError::LaunchFailed) => {}
        }
    }
    let _ = res;
    NewWindowResponse::Deny
}

/// Internal IPC method used by the dev shell init script to forward diagnostics.
pub const DEV_REPORT_IPC_METHOD: &str = "dev.report";

/// Internal IPC method used by built apps to log CSP violations via tracing (origin and type only).
pub const CSP_TELEMETRY_IPC_METHOD: &str = "telemetry.cspBlock";

/// Internal IPC method used by the init script to route trusted user-initiated external link clicks
/// through [`handle_external_link_click`].
pub const INTERNAL_OPEN_EXTERNAL_IPC_METHOD: &str = "internal.openExternal";

/// Strict payload for internal external-link click forwarding (`internal.openExternal`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalOpenExternalParams {
    pub url: String,
    pub top_frame: bool,
}

/// Strict payload for built-app CSP violation telemetry (`telemetry.cspBlock`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CspTelemetryParams {
    pub directive: String,
    pub blocked_uri: String,
    pub top_frame: bool,
}

/// Strict payload for dev-shell diagnostic reports (`dev.report`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevReportParams {
    pub kind: String,
    #[serde(default)]
    pub directive: Option<String>,
    #[serde(default)]
    pub blocked_uri: Option<String>,
    #[serde(default)]
    pub resource_type: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    pub top_frame: bool,
}

/// Outcome of inspecting a candidate diagnostic/telemetry bridge report message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeReportOutcome {
    /// Report was validated and processed (with optional dev diagnostic event when dev forwarding is active).
    Handled(Option<DevDiagnosticEvent>),
    /// Report failed validation or came from a non-top frame and was dropped.
    Dropped,
    /// Message should be handled by standard `process_ipc_message` (e.g. `dev.report` in production shell).
    ForwardToIpc,
}

/// Fast pre-check before routing an IPC body through [`process_bridge_report_message`].
#[must_use]
pub fn is_bridge_report_candidate(body: &str) -> bool {
    body.contains("\"telemetry.cspBlock\"")
        || body.contains("\"dev.report\"")
        || body.contains("\"internal.openExternal\"")
}

/// Validate and process a diagnostic/telemetry report or internal external-link click message
/// sent over `window.ipc.postMessage`.
pub fn process_bridge_report_message_with_opener<F>(
    uri: &Uri,
    is_top_frame: bool,
    body: &str,
    config: &crate::WindowConfig,
    rate_limiter: &OpenExternalRateLimiter,
    opener: F,
) -> Result<BridgeReportOutcome, crate::IpcOriginError>
where
    F: FnOnce(&str) -> Result<(), std::io::Error>,
{
    if body.contains("\"internal.openExternal\"") {
        let is_wavical_scheme = uri
            .scheme_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("wavical"));
        let allowed = if !is_top_frame {
            false
        } else if is_wavical_scheme {
            crate::is_origin_allowed(uri, &config.allowed_origins)
        } else {
            authorize_bridge_request(&config.network_policy, uri, is_top_frame, &config.window_id)
                .is_ok()
                || crate::is_origin_allowed(uri, &config.allowed_origins)
        };
        if !allowed {
            let origin_str = crate::extract_origin(uri).unwrap_or_else(|| uri.to_string());
            tracing::warn!(
                security_event = "origin_rejected",
                window_id = %config.window_id,
                origin = %origin_str,
                reason = "External link caller origin or frame is not authorized",
                "Security check failed: bridge external link origin rejected"
            );
            return Err(crate::IpcOriginError::DisallowedOrigin(origin_str));
        }

        let Ok(request) = wavical_ipc::parse_request(body) else {
            return Ok(BridgeReportOutcome::Dropped);
        };
        if request.method == INTERNAL_OPEN_EXTERNAL_IPC_METHOD {
            if request.id != 0 {
                return Ok(BridgeReportOutcome::ForwardToIpc);
            }
            let Ok(params) = serde_json::from_value::<InternalOpenExternalParams>(request.params)
            else {
                return Ok(BridgeReportOutcome::Dropped);
            };
            if !params.top_frame {
                return Ok(BridgeReportOutcome::Dropped);
            }
            return match handle_external_link_click(
                &config.network_policy,
                &params.url,
                &config.window_id,
                rate_limiter,
                opener,
            ) {
                ExternalLinkClickOutcome::Blocked(rule) => Ok(BridgeReportOutcome::Handled(Some(
                    DevDiagnosticEvent::BlockedNewWindow {
                        origin: safe_log_origin(&params.url),
                        rule: rule.to_string(),
                    },
                ))),
                ExternalLinkClickOutcome::RateLimited => Ok(BridgeReportOutcome::Handled(Some(
                    DevDiagnosticEvent::BlockedNewWindow {
                        origin: safe_log_origin(&params.url),
                        rule: "rate_limit_exceeded".to_string(),
                    },
                ))),
                ExternalLinkClickOutcome::NavigateInApp
                | ExternalLinkClickOutcome::OpenedExternal
                | ExternalLinkClickOutcome::LaunchFailed => Ok(BridgeReportOutcome::Handled(None)),
            };
        }
    }

    process_bridge_report_message(uri, is_top_frame, body, config)
}

/// Validate and process a diagnostic/telemetry report sent over `window.ipc.postMessage`.
///
/// Enforces the exact same origin check, top-frame check, and `wavical_ipc::parse_request`
/// validation (1 MB size cap, null-byte check, depth <= 32, `deny_unknown_fields`) as any IPC call.
pub fn process_bridge_report_message(
    uri: &Uri,
    is_top_frame: bool,
    body: &str,
    config: &crate::WindowConfig,
) -> Result<BridgeReportOutcome, crate::IpcOriginError> {
    let is_wavical_scheme = uri
        .scheme_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("wavical"));
    let allowed = if !is_top_frame {
        false
    } else if is_wavical_scheme {
        crate::is_origin_allowed(uri, &config.allowed_origins)
    } else {
        authorize_bridge_request(&config.network_policy, uri, is_top_frame, &config.window_id)
            .is_ok()
            || crate::is_origin_allowed(uri, &config.allowed_origins)
    };

    if !allowed {
        let origin_str = crate::extract_origin(uri).unwrap_or_else(|| uri.to_string());
        tracing::warn!(
            security_event = "origin_rejected",
            window_id = %config.window_id,
            origin = %origin_str,
            reason = "Bridge report caller origin or frame is not authorized",
            "Security check failed: bridge report origin rejected"
        );
        return Err(crate::IpcOriginError::DisallowedOrigin(origin_str));
    }

    let request = match wavical_ipc::parse_request(body) {
        Ok(req) => req,
        Err(wavical_ipc::IpcError::MessageTooLarge { size, max }) => {
            tracing::warn!(
                security_event = "oversize_message",
                window_id = %config.window_id,
                size = size,
                max_size = max,
                reason = "Raw bridge report exceeds maximum size limit",
                "Security check failed: oversize IPC bridge report rejected"
            );
            return Ok(BridgeReportOutcome::Dropped);
        }
        Err(other_err) => {
            tracing::warn!(
                security_event = "malformed_message",
                window_id = %config.window_id,
                payload = %crate::truncate_payload(body, 64),
                reason = %other_err,
                "Security check failed: malformed IPC bridge report rejected"
            );
            return Ok(BridgeReportOutcome::Dropped);
        }
    };

    if request.method == CSP_TELEMETRY_IPC_METHOD {
        let Ok(params) = serde_json::from_value::<CspTelemetryParams>(request.params) else {
            return Ok(BridgeReportOutcome::Dropped);
        };
        if !params.top_frame {
            return Ok(BridgeReportOutcome::Dropped);
        }
        let norm_dir = normalize_csp_directive(&params.directive);
        let block_type = map_csp_directive_to_type(&norm_dir);
        let safe_origin = sanitize_recorded_origin(&params.blocked_uri);

        tracing::warn!(
            security_event = "csp_blocked",
            mode = mode_label(config.network_policy.mode),
            block_type = block_type,
            origin = %safe_origin,
            directive = %norm_dir,
            window_id = %config.window_id,
            "Security check failed: resource blocked by CSP"
        );

        if config.network_policy.is_dev_command
            && let Some(ref rec) = config.network_policy.recorder
        {
            rec.record_raw(block_type, &params.blocked_uri);
        }
        // Built apps never forward console/terminal diagnostics.
        return Ok(BridgeReportOutcome::Handled(None));
    }

    if request.method == DEV_REPORT_IPC_METHOD {
        #[cfg(not(feature = "dev-shell"))]
        {
            return Ok(BridgeReportOutcome::ForwardToIpc);
        }

        #[cfg(feature = "dev-shell")]
        {
            // Forwarding is strictly disabled in non-dev windows.
            if !config.network_policy.is_dev_command {
                return Ok(BridgeReportOutcome::ForwardToIpc);
            }

            let Ok(params) = serde_json::from_value::<DevReportParams>(request.params) else {
                return Ok(BridgeReportOutcome::Dropped);
            };
            if !params.top_frame {
                return Ok(BridgeReportOutcome::Dropped);
            }

            match params.kind.as_str() {
                "csp" => {
                    let Some(directive) = params.directive else {
                        return Ok(BridgeReportOutcome::Dropped);
                    };
                    let Some(blocked_uri) = params.blocked_uri else {
                        return Ok(BridgeReportOutcome::Dropped);
                    };
                    let norm_dir = normalize_csp_directive(&directive);
                    let (res_type, _) = map_directive_to_resource_type_and_allowlist_key(&norm_dir);
                    let rec_type = map_csp_directive_to_type(&norm_dir);
                    let safe_origin = sanitize_recorded_origin(&blocked_uri);

                    if let Some(ref rec) = config.network_policy.recorder {
                        rec.record_raw(rec_type, &blocked_uri);
                    }

                    tracing::warn!(
                        security_event = "csp_blocked",
                        mode = mode_label(config.network_policy.mode),
                        block_type = rec_type,
                        origin = %safe_origin,
                        directive = %norm_dir,
                        window_id = %config.window_id,
                        "Security check failed: resource blocked by CSP"
                    );

                    return Ok(BridgeReportOutcome::Handled(Some(
                        DevDiagnosticEvent::CspViolation {
                            directive: norm_dir,
                            origin: safe_origin,
                            resource_type: res_type.to_string(),
                        },
                    )));
                }
                "jsError" => {
                    let msg = params.message.unwrap_or_default();
                    let loc = params.location.unwrap_or_default();
                    return Ok(BridgeReportOutcome::Handled(Some(
                        DevDiagnosticEvent::JsError {
                            message: truncate_chars(
                                &sanitize_single_line(&msg),
                                MAX_JS_ERROR_CHARS,
                            ),
                            location: sanitize_location(&loc),
                        },
                    )));
                }
                "consoleError" => {
                    let msg = params.message.unwrap_or_default();
                    let loc = params.location.unwrap_or_default();
                    return Ok(BridgeReportOutcome::Handled(Some(
                        DevDiagnosticEvent::ConsoleError {
                            message: truncate_chars(
                                &sanitize_single_line(&msg),
                                MAX_JS_ERROR_CHARS,
                            ),
                            location: sanitize_location(&loc),
                        },
                    )));
                }
                "resourceError" => {
                    let Some(blocked_uri) = params.blocked_uri else {
                        return Ok(BridgeReportOutcome::Dropped);
                    };
                    let safe_origin = sanitize_recorded_origin(&blocked_uri);
                    let res_type =
                        sanitize_single_line(params.resource_type.as_deref().unwrap_or("resource"));
                    let final_type = if res_type.is_empty() {
                        "resource".to_string()
                    } else {
                        res_type
                    };
                    return Ok(BridgeReportOutcome::Handled(Some(
                        DevDiagnosticEvent::ResourceLoadFailed {
                            resource_type: final_type,
                            origin: safe_origin,
                        },
                    )));
                }
                _ => return Ok(BridgeReportOutcome::Dropped),
            }
        }
    }

    Ok(BridgeReportOutcome::ForwardToIpc)
}

/// Builds the initialization script with an origin and top-frame guard so `window.wavical`
/// is only exposed on authorized top-level origins (removed on navigation away, restored on return).
#[must_use]
pub fn build_init_script(policy: &ShellNetworkPolicy) -> String {
    let mut allowed = vec![
        policy.app_origin.as_canonical(),
        "wavical://localhost".to_string(),
        "https://wavical.localhost".to_string(),
    ];
    if policy.is_dev_command
        && let Some(ref dev) = policy.dev_url_origin
    {
        allowed.push(dev.as_canonical());
    }
    if policy.mode != NetworkMode::UltraSafe
        && let Some(ref remote) = policy.remote_url_origin
    {
        let allow_remote = match policy.mode {
            NetworkMode::Open => true,
            NetworkMode::Safe => policy
                .allowlist
                .as_ref()
                .and_then(|al| al.remote_ui.as_ref())
                .is_some_and(|rui| &rui.origin == remote),
            NetworkMode::UltraSafe => false,
        };
        if allow_remote {
            allowed.push(remote.as_canonical());
        }
    }

    let mut in_app_exact = allowed.clone();
    let mut in_app_wildcards = Vec::new();
    if policy.mode != NetworkMode::UltraSafe
        && let Some(ref al) = policy.allowlist
    {
        for exc in &al.local_exceptions {
            if matches!(exc.origin.scheme.as_str(), "http" | "https") {
                in_app_exact.push(exc.origin.as_canonical());
            }
        }
        if policy.mode == NetworkMode::Safe {
            for pat in &al.navigate {
                match pat {
                    AllowlistPattern::Exact(o) => in_app_exact.push(o.as_canonical()),
                    AllowlistPattern::WildcardSubdomain {
                        scheme,
                        base_domain,
                        port,
                    } => {
                        in_app_wildcards.push(serde_json::json!({
                            "scheme": scheme,
                            "baseDomain": base_domain,
                            "port": port,
                            "defaultPort": default_port_for_scheme(scheme),
                        }));
                    }
                }
            }
        }
    }

    let allowed_json = serde_json::to_string(&allowed).unwrap_or_else(|_| "[]".to_string());
    let in_app_exact_json =
        serde_json::to_string(&in_app_exact).unwrap_or_else(|_| "[]".to_string());
    let in_app_wildcards_json =
        serde_json::to_string(&in_app_wildcards).unwrap_or_else(|_| "[]".to_string());
    let external_links_mode = match policy.external_links {
        ExternalLinksMode::Browser => "browser",
        ExternalLinksMode::InApp => "in-app",
    };
    let report_hook = build_report_hook_js(policy.is_dev_command);
    let link_hook = build_external_link_click_hook_js(
        external_links_mode,
        &in_app_exact_json,
        &in_app_wildcards_json,
    );
    format!(
        "(() => {{ if (window.top !== window.self) return; const __allowed = {allowed_json}; if (!__allowed.includes(window.location.origin)) return;\n{}\n{link_hook}\n{report_hook}\n}})();",
        crate::WAVICAL_INIT_SCRIPT
    )
}

fn build_external_link_click_hook_js(
    external_links_mode: &str,
    in_app_exact_json: &str,
    in_app_wildcards_json: &str,
) -> String {
    let mode_json =
        serde_json::to_string(external_links_mode).unwrap_or_else(|_| "\"browser\"".to_string());
    format!(
        r#"
const __externalLinksMode = {mode_json};
const __inAppExact = {in_app_exact_json};
const __inAppWildcards = {in_app_wildcards_json};
const __isInAppUrl = (u) => {{
  if (u.origin === window.location.origin) return true;
  if (__inAppExact.includes(u.origin)) return true;
  const scheme = String(u.protocol || '').replace(/:$/, '').toLowerCase();
  const host = String(u.hostname || '').toLowerCase().replace(/\.$/, '');
  for (const w of __inAppWildcards) {{
    if (w.scheme !== scheme) continue;
    const effPort = u.port ? Number(u.port) : w.defaultPort;
    if (effPort !== w.port) continue;
    if (host.length > w.baseDomain.length + 1 && host.endsWith('.' + w.baseDomain)) return true;
  }}
  return false;
}};
document.addEventListener('click', (e) => {{
  try {{
    if (!e.isTrusted || e.defaultPrevented) return;
    if (typeof e.button === 'number' && e.button !== 0) return;
    const el = e.target && typeof e.target.closest === 'function' ? e.target.closest('a[href], area[href]') : null;
    if (!el) return;
    const tgt = String(el.getAttribute('target') || '').trim().toLowerCase();
    if (tgt === '_blank' || el.hasAttribute('download')) return;
    const rawHref = el.getAttribute('href');
    if (!rawHref || rawHref.trim().startsWith('#')) return;
    let resolved;
    try {{
      resolved = new URL(el.href || rawHref, window.location.href);
    }} catch (_) {{
      return;
    }}
    const scheme = String(resolved.protocol || '').toLowerCase();
    const isMailOrTel = (scheme === 'mailto:' || scheme === 'tel:');
    const isWeb = (scheme === 'https:' || scheme === 'http:');
    if (!isMailOrTel && !isWeb) return;
    if (isWeb) {{
      if (__externalLinksMode !== 'browser') return;
      if (__isInAppUrl(resolved)) return;
    }}
    e.preventDefault();
    if (window.ipc && typeof window.ipc.postMessage === 'function') {{
      window.ipc.postMessage(JSON.stringify({{
        id: 0,
        method: 'internal.openExternal',
        params: {{ url: isMailOrTel ? (el.href || rawHref) : resolved.href, topFrame: true }}
      }}));
    }}
  }} catch (_) {{}}
}});
"#
    )
}

#[cfg(feature = "dev-shell")]
fn build_report_hook_js(is_dev_command: bool) -> &'static str {
    if is_dev_command {
        r#"
const __safeOrigin = (raw) => {
  const s = String(raw || '');
  if (s === 'inline' || s === 'eval' || s === 'wasm-eval') return s;
  if (s.startsWith('data:')) return 'data:';
  if (s.startsWith('blob:')) return 'blob:';
  try {
    const u = new URL(s, window.location.href);
    return (u.origin && u.origin !== 'null') ? u.origin : (u.protocol || '');
  } catch (_) {
    return '';
  }
};
const __stripUrlQuery = (raw) => {
  const s = String(raw || '');
  if (!s) return '';
  try {
    const u = new URL(s, window.location.href);
    return u.origin + u.pathname;
  } catch (_) {
    return s.split('?')[0].split('#')[0];
  }
};
const __sendDevReport = (params) => {
  try {
    if (window.top !== window.self) return;
    if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;
    window.ipc.postMessage(JSON.stringify({
      id: 0,
      method: 'dev.report',
      params: Object.assign({}, params, { topFrame: true })
    }));
  } catch (_) {}
};
const __recentCsp = new Map();
const __directiveToType = (dir) => {
  const d = String(dir || '').toLowerCase();
  if (d.startsWith('script-src')) return 'script';
  if (d.startsWith('style-src')) return 'style';
  if (d.startsWith('font-src')) return 'font';
  if (d.startsWith('img-src')) return 'image';
  if (d.startsWith('media-src')) return 'media';
  if (d.startsWith('connect-src')) return 'connect';
  if (d.startsWith('frame-src')) return 'frame';
  return 'resource';
};
document.addEventListener('securitypolicyviolation', (e) => {
  const directive = String(e.effectiveDirective || e.violatedDirective || '');
  const origin = __safeOrigin(e.blockedURI || '');
  const rtype = __directiveToType(directive);
  __recentCsp.set(rtype + '|' + origin, Date.now());
  __sendDevReport({
    kind: 'csp',
    directive: directive,
    blockedUri: origin,
    resourceType: rtype
  });
});
window.addEventListener('error', (e) => {
  const tgt = e && e.target;
  if (tgt && tgt !== window && tgt.tagName) {
    const tag = String(tgt.tagName).toLowerCase();
    let rtype = 'resource';
    if (tag === 'script') rtype = 'script';
    else if (tag === 'link') rtype = (String(tgt.rel || '').toLowerCase().includes('stylesheet')) ? 'style' : 'link';
    else if (tag === 'img') rtype = 'image';
    else if (tag === 'audio' || tag === 'video' || tag === 'source') rtype = 'media';
    else if (tag === 'iframe') rtype = 'frame';
    const rawUrl = tgt.src || tgt.href || '';
    if (!rawUrl) return;
    const origin = __safeOrigin(rawUrl);
    const key = rtype + '|' + origin;
    const ts = __recentCsp.get(key);
    if (ts && (Date.now() - ts) < 1000) return;
    __sendDevReport({
      kind: 'resourceError',
      resourceType: rtype,
      blockedUri: origin
    });
    return;
  }
  const msg = String((e && (e.message || (e.error && e.error.message))) || 'Uncaught error').slice(0, 500);
  const file = __stripUrlQuery((e && e.filename) || '');
  const line = (e && typeof e.lineno === 'number' && e.lineno > 0) ? (':' + e.lineno) : '';
  __sendDevReport({
    kind: 'jsError',
    message: msg,
    location: (file + line).slice(0, 200)
  });
}, true);
window.addEventListener('unhandledrejection', (e) => {
  const reason = e && e.reason;
  const msg = String((reason && reason.message) ? reason.message : (reason || 'Unhandled promise rejection')).slice(0, 500);
  let loc = '';
  if (reason && typeof reason.stack === 'string') {
    const m = reason.stack.match(/(?:https?|wavical):\/\/[^\s)]+:\d+/);
    if (m) {
      const idx = m[0].lastIndexOf(':');
      loc = __stripUrlQuery(m[0].slice(0, idx)) + m[0].slice(idx);
    }
  }
  __sendDevReport({
    kind: 'jsError',
    message: ('Unhandled rejection: ' + msg).slice(0, 500),
    location: loc.slice(0, 200)
  });
});
if (typeof console !== 'undefined' && typeof console.error === 'function') {
  const __origConsoleError = console.error.bind(console);
  console.error = function(...args) {
    try {
      const msg = args.map((a) => {
        if (a instanceof Error) return a.message || String(a);
        if (typeof a === 'string') return a;
        try { return JSON.stringify(a); } catch (_) { return String(a); }
      }).join(' ').slice(0, 500);
      let loc = '';
      try {
        const stack = (new Error()).stack || '';
        const lines = stack.split('\n');
        for (let i = 1; i < lines.length; i++) {
          const l = lines[i];
          if (l.includes('__sendDevReport') || l.includes('console.error')) continue;
          const m = l.match(/((?:https?|wavical):\/\/[^\s()]+):(\d+)(?::\d+)?/);
          if (m) {
            loc = __stripUrlQuery(m[1]) + ':' + m[2];
            break;
          }
        }
      } catch (_) {}
      __sendDevReport({
        kind: 'consoleError',
        message: msg,
        location: loc.slice(0, 200)
      });
    } catch (_) {}
    return __origConsoleError(...args);
  };
}
"#
    } else {
        built_app_csp_telemetry_js()
    }
}

#[cfg(not(feature = "dev-shell"))]
fn build_report_hook_js(_is_dev_command: bool) -> &'static str {
    built_app_csp_telemetry_js()
}

fn built_app_csp_telemetry_js() -> &'static str {
    r#"
document.addEventListener('securitypolicyviolation', (e) => {
  try {
    if (window.top !== window.self) return;
    if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;
    const raw = String(e.blockedURI || '');
    let safeOrigin = raw;
    if (raw === 'inline' || raw === 'eval' || raw === 'wasm-eval') {
      safeOrigin = raw;
    } else if (raw.startsWith('data:')) {
      safeOrigin = 'data:';
    } else if (raw.startsWith('blob:')) {
      safeOrigin = 'blob:';
    } else {
      try {
        const u = new URL(raw, window.location.href);
        safeOrigin = (u.origin && u.origin !== 'null') ? u.origin : (u.protocol || '');
      } catch (_) {}
    }
    window.ipc.postMessage(JSON.stringify({
      id: 0,
      method: 'telemetry.cspBlock',
      params: {
        directive: String(e.effectiveDirective || e.violatedDirective || ''),
        blockedUri: safeOrigin,
        topFrame: true
      }
    }));
  } catch (_) {}
});
"#
}

/// Friendly offline error page with a retry button (Plan 1.6, WP5 Task 4).
#[must_use]
pub fn build_offline_error_html(retry_url: &str) -> String {
    let safe_url = html_escape(retry_url);
    format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Offline</title></head>
<body style="font-family:system-ui,sans-serif;max-width:480px;margin:10vh auto;padding:24px;">
  <h1>You appear to be offline</h1>
  <p>Wavical could not reach the application server. Check your network connection and try again.</p>
  <p><a href="{safe_url}"><button type="button">Retry</button></a></p>
</body>
</html>"#
    )
}

/// Friendly certificate error page that fails closed with no bypass (Plan 1.6, WP5 Task 4).
#[must_use]
pub fn build_cert_error_html() -> String {
    r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Certificate Error</title></head>
<body style="font-family:system-ui,sans-serif;max-width:480px;margin:10vh auto;padding:24px;">
  <h1>Connection is not trusted</h1>
  <p>The server's TLS certificate could not be verified. For your security, Wavical cannot bypass certificate errors.</p>
  <p>If this is an internal company application, please contact your IT department about installing the company root certificate in Windows.</p>
</body>
</html>"#
        .to_string()
}

fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}
/// Checks IPC bridge authorization using `wavical_netpolicy::decisions::decide_bridge_allowed`.
pub fn authorize_bridge_request(
    policy: &ShellNetworkPolicy,
    uri: &Uri,
    is_top_frame: bool,
    window_id: &str,
) -> Result<Vec<String>, &'static str> {
    let uri_str = uri.to_string();
    let ctx = policy.as_context();
    match decide_bridge_allowed(&ctx, &uri_str, is_top_frame) {
        BridgeDecision::Allow {
            granted_capabilities,
        } => Ok(granted_capabilities),
        BridgeDecision::Deny(rule) => {
            tracing::warn!(
                security_event = "origin_rejected",
                mode = mode_label(policy.mode),
                block_type = "bridge",
                origin = %safe_log_origin(&uri_str),
                window_id = %window_id,
                rule = rule,
                "Security check failed: IPC origin rejected"
            );
            Err(rule)
        }
    }
}
