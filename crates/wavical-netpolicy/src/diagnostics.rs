//! Pure diagnostic formatting, deduplication, rate-limiting, and CSP summary helpers
//! shared between `wavical-shell` and `wavical-cli`.
//!
//! Zero `tao`, `wry`, network, or OS dependencies.

#![forbid(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::NetworkMode;

/// Maximum character length for forwarded uncaught JS error and `console.error` summaries.
pub const MAX_JS_ERROR_CHARS: usize = 500;

/// Default maximum diagnostic lines forwarded to the terminal per second.
pub const DEV_DIAGNOSTIC_RATE_LIMIT_PER_SEC: usize = 50;

/// Maximum distinct deduplication keys retained in memory per dev session.
pub const MAX_TRACKED_DEDUP_KEYS: usize = 2_000;

/// Interval (in milliseconds) before flushing deduplicated repeat counts.
pub const DEDUP_FLUSH_INTERVAL_MS: u64 = 350;

/// Structured diagnostic event emitted by `wavical-shell-dev` to `wavical-cli`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DevDiagnosticEvent {
    #[serde(rename_all = "camelCase")]
    CspViolation {
        directive: String,
        origin: String,
        resource_type: String,
    },
    #[serde(rename_all = "camelCase")]
    BlockedNavigation { origin: String, rule: String },
    #[serde(rename_all = "camelCase")]
    BlockedNewWindow { origin: String, rule: String },
    #[serde(rename_all = "camelCase")]
    JsError { message: String, location: String },
    #[serde(rename_all = "camelCase")]
    ConsoleError { message: String, location: String },
    #[serde(rename_all = "camelCase")]
    ResourceLoadFailed {
        resource_type: String,
        origin: String,
    },
}

/// Normalize a CSP `effectiveDirective` (e.g. `script-src-elem` -> `script-src`).
#[must_use]
pub fn normalize_csp_directive(directive: &str) -> String {
    let lower = directive.trim().to_ascii_lowercase();
    match lower.as_str() {
        "script-src-elem" | "script-src-attr" => "script-src".to_string(),
        "style-src-elem" | "style-src-attr" => "style-src".to_string(),
        _ if !lower.is_empty() => lower,
        _ => "default-src".to_string(),
    }
}

/// Map a normalized CSP directive to `(human_resource_type, allowlist_json_key)`.
#[must_use]
pub fn map_directive_to_resource_type_and_allowlist_key(
    directive: &str,
) -> (&'static str, Option<&'static str>) {
    let d = normalize_csp_directive(directive);
    if d.starts_with("script-src") {
        ("script", Some("scripts"))
    } else if d.starts_with("style-src") {
        ("style", Some("styles"))
    } else if d.starts_with("font-src") {
        ("font", Some("fonts"))
    } else if d.starts_with("img-src") {
        ("image", Some("images"))
    } else if d.starts_with("media-src") {
        ("media", Some("media"))
    } else if d.starts_with("connect-src") {
        ("connect", Some("connect"))
    } else if d.starts_with("frame-src") {
        ("frame", Some("frames"))
    } else if d.starts_with("form-action") {
        ("form", None)
    } else {
        ("resource", None)
    }
}

/// Truncate `input` to at most `max_chars` Unicode scalar values, appending `"..."` when truncated.
#[must_use]
pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    for (count, (idx, _)) in input.char_indices().enumerate() {
        if count == max_chars {
            let cut_chars = max_chars.saturating_sub(3);
            let cut_idx = input
                .char_indices()
                .nth(cut_chars)
                .map(|(i, _)| i)
                .unwrap_or(idx);
            let mut out = input[..cut_idx].to_string();
            if max_chars >= 3 {
                out.push_str("...");
            }
            return out;
        }
    }
    input.to_string()
}

/// Collapse ASCII control characters (`\r`, `\n`, `\t`, etc.) into spaces to prevent terminal/log injection.
#[must_use]
pub fn sanitize_single_line(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;
    for ch in input.chars() {
        if ch.is_control() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = ch == ' ';
        }
    }
    out.trim().to_string()
}

/// Strip query strings (`?...`) and fragments (`#...`) from a `file:line[:col]` location string.
#[must_use]
pub fn sanitize_location(raw: &str) -> String {
    let clean = sanitize_single_line(raw);
    if clean.is_empty() {
        return String::new();
    }

    // Preserve trailing `:line` or `:line:col` if present after a URL/path.
    let (base, line_suffix) = split_trailing_line_col(&clean);
    let stripped_base = base
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let result = if stripped_base.is_empty() {
        line_suffix.trim_start_matches(':').to_string()
    } else {
        format!("{stripped_base}{line_suffix}")
    };
    truncate_chars(&result, 200)
}

fn split_trailing_line_col(s: &str) -> (&str, &str) {
    // Match optional `:digits` or `:digits:digits` at the very end.
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    let mut colons = 0usize;
    while i > 0 {
        let start = i;
        while i > 0 && bytes[i - 1].is_ascii_digit() {
            i -= 1;
        }
        if i == start || i == 0 || bytes[i - 1] != b':' {
            return (s, "");
        }
        colons += 1;
        i -= 1; // step before ':'
        if colons == 2 || (colons == 1 && !looks_like_colon_digits(&bytes[..i])) {
            return (&s[..i], &s[i..]);
        }
    }
    (s, "")
}

fn looks_like_colon_digits(prefix: &[u8]) -> bool {
    if let Some(pos) = prefix.iter().rposition(|&b| b == b':') {
        let tail = &prefix[pos + 1..];
        return !tail.is_empty() && tail.iter().all(|b| b.is_ascii_digit());
    }
    false
}

/// Format an error message + `file:line`, strictly truncated to `MAX_JS_ERROR_CHARS` (500 chars).
#[must_use]
pub fn format_error_summary(message: &str, location: &str) -> String {
    let clean_msg = sanitize_single_line(message);
    let clean_loc = sanitize_location(location);
    if clean_loc.is_empty() {
        return truncate_chars(&clean_msg, MAX_JS_ERROR_CHARS);
    }

    let suffix = format!(" ({clean_loc})");
    let suffix_chars = suffix.chars().count();
    if suffix_chars + 16 < MAX_JS_ERROR_CHARS {
        let msg_budget = MAX_JS_ERROR_CHARS - suffix_chars;
        let truncated_msg = truncate_chars(&clean_msg, msg_budget);
        format!("{truncated_msg}{suffix}")
    } else {
        truncate_chars(&format!("{clean_msg}{suffix}"), MAX_JS_ERROR_CHARS)
    }
}

/// Produce a one-line mode-matched hint for a [`DevDiagnosticEvent`].
#[must_use]
pub fn format_diagnostic_hint(mode: NetworkMode, event: &DevDiagnosticEvent) -> String {
    match event {
        DevDiagnosticEvent::CspViolation {
            directive, origin, ..
        } => {
            let norm_dir = normalize_csp_directive(directive);
            let (_, allowlist_key) = map_directive_to_resource_type_and_allowlist_key(&norm_dir);

            if origin == "inline" && norm_dir.starts_with("script-src") {
                return match mode {
                    NetworkMode::UltraSafe => {
                        "dev-ultra-safe extracts <script> tags, or move inline handlers to external JS; allowed in \"wavical dev\"".to_string()
                    }
                    NetworkMode::Safe => {
                        "move inline event handlers to external JS with addEventListener, or re-save to rehash static <script> tags".to_string()
                    }
                    NetworkMode::Open => "check inline script CSP configuration".to_string(),
                };
            }

            if origin == "inline" && norm_dir.starts_with("style-src") {
                return match mode {
                    NetworkMode::UltraSafe => {
                        "allowed in \"wavical dev\"; in dev-safe set \"inlineStyles\": true in wavical.allowlist.json".to_string()
                    }
                    NetworkMode::Safe => {
                        "set \"inlineStyles\": true in wavical.allowlist.json, or run \"wavical dev\"".to_string()
                    }
                    NetworkMode::Open => "check inline style CSP configuration".to_string(),
                };
            }

            if origin == "eval" {
                return match mode {
                    NetworkMode::UltraSafe | NetworkMode::Safe => {
                        "allowed only in \"wavical dev\"; remove eval/new Function for safe or ultra-safe mode".to_string()
                    }
                    NetworkMode::Open => "check eval CSP configuration".to_string(),
                };
            }

            if origin == "data:" && norm_dir.starts_with("img-src") {
                return match mode {
                    NetworkMode::UltraSafe => {
                        "allowed in \"wavical dev\"; in dev-safe set \"dataImages\": true in wavical.allowlist.json".to_string()
                    }
                    NetworkMode::Safe => {
                        "set \"dataImages\": true in wavical.allowlist.json, or run \"wavical dev\"".to_string()
                    }
                    NetworkMode::Open => "check data: image CSP configuration".to_string(),
                };
            }

            if origin.starts_with("http://") || origin.starts_with("ws://") {
                return match mode {
                    NetworkMode::UltraSafe => {
                        "plain http/ws is blocked in ultra-safe; in \"wavical dev\" or dev-safe add it to localExceptions in wavical.allowlist.json".to_string()
                    }
                    NetworkMode::Safe | NetworkMode::Open => {
                        "add it to localExceptions (with a reason) in wavical.allowlist.json".to_string()
                    }
                };
            }

            match mode {
                NetworkMode::UltraSafe => match allowlist_key {
                    Some(key) => format!(
                        "allowed in \"wavical dev\"; in dev-safe add it to {key} in wavical.allowlist.json"
                    ),
                    None => "allowed in \"wavical dev\"".to_string(),
                },
                NetworkMode::Safe => match allowlist_key {
                    Some(key) => format!(
                        "add it to {key} in wavical.allowlist.json, or run \"wavical dev\""
                    ),
                    None => "blocked by safe-mode CSP; run \"wavical dev\" if needed".to_string(),
                },
                NetworkMode::Open => {
                    "blocked by open-mode CSP; check scheme or localExceptions in wavical.allowlist.json".to_string()
                }
            }
        }
        DevDiagnosticEvent::BlockedNavigation { rule, .. } => {
            if rule == "baseline_blocked_scheme" {
                "scheme is blocked by baseline security policy in all modes".to_string()
            } else if rule.contains("local_exception") || rule.contains("plain_http") {
                match mode {
                    NetworkMode::UltraSafe => {
                        "plain http is blocked in ultra-safe; in \"wavical dev\" or dev-safe add it to localExceptions in wavical.allowlist.json".to_string()
                    }
                    NetworkMode::Safe | NetworkMode::Open => {
                        "add it to localExceptions (with a reason) in wavical.allowlist.json".to_string()
                    }
                }
            } else {
                match mode {
                    NetworkMode::UltraSafe => {
                        "allowed for https:// in \"wavical dev\"; in dev-safe add it to navigate in wavical.allowlist.json".to_string()
                    }
                    NetworkMode::Safe => {
                        "add it to navigate in wavical.allowlist.json, or run \"wavical dev\"".to_string()
                    }
                    NetworkMode::Open => format!("blocked by open-mode navigation policy ({rule})"),
                }
            }
        }
        DevDiagnosticEvent::BlockedNewWindow { rule, .. } => {
            if rule == "baseline_blocked_scheme" {
                "scheme is blocked by baseline security policy in all modes".to_string()
            } else if rule == "rate_limit_exceeded" {
                "external link rate limit (5 per 10s) exceeded; reduce window.open calls"
                    .to_string()
            } else if rule.contains("local_exception") || rule.contains("plain_http") {
                match mode {
                    NetworkMode::UltraSafe => {
                        "plain http links cannot open externally; use https:// in \"wavical dev\" or dev-safe".to_string()
                    }
                    NetworkMode::Safe | NetworkMode::Open => {
                        "external links require https:// (and openExternal entry in dev-safe)".to_string()
                    }
                }
            } else {
                match mode {
                    NetworkMode::UltraSafe => {
                        "allowed for https:// in \"wavical dev\" (opens in system browser); in dev-safe add it to openExternal in wavical.allowlist.json".to_string()
                    }
                    NetworkMode::Safe => {
                        "add it to openExternal in wavical.allowlist.json, or run \"wavical dev\"".to_string()
                    }
                    NetworkMode::Open => {
                        format!("blocked by open-mode new-window policy ({rule})")
                    }
                }
            }
        }
        DevDiagnosticEvent::JsError { .. } => match mode {
            NetworkMode::UltraSafe => {
                "check DevTools (F12) for stack trace (a CSP-blocked script may have caused this in dev-ultra-safe)".to_string()
            }
            NetworkMode::Safe => {
                "check DevTools (F12) for stack trace (a blocked script in dev-safe may have caused this)".to_string()
            }
            NetworkMode::Open => "check DevTools (F12) for full stack trace".to_string(),
        },
        DevDiagnosticEvent::ConsoleError { .. } => {
            "check DevTools (F12) Console for details".to_string()
        }
        DevDiagnosticEvent::ResourceLoadFailed {
            resource_type,
            origin,
        } => {
            if origin.contains("wavical.localhost")
                || origin.contains("://localhost")
                || origin.contains("://127.0.0.1")
            {
                "check that the file exists in src/ or your dev server is running".to_string()
            } else {
                match mode {
                    NetworkMode::UltraSafe => format!(
                        "failed to load {resource_type}; check URL or run \"wavical dev\" / \"wavical vendor\""
                    ),
                    NetworkMode::Safe => format!(
                        "failed to load {resource_type}; check network, URL, or wavical.allowlist.json"
                    ),
                    NetworkMode::Open => format!(
                        "failed to load {resource_type}; check network, URL, or TLS certificate (DevTools F12 -> Network)"
                    ),
                }
            }
        }
    }
}

/// Format a single diagnostic line (with optional `[xN]` deduplication prefix) ending with its mode-matched hint.
#[must_use]
pub fn format_diagnostic_line(
    mode: NetworkMode,
    event: &DevDiagnosticEvent,
    count: usize,
) -> String {
    let hint = format_diagnostic_hint(mode, event);
    let body = match event {
        DevDiagnosticEvent::CspViolation {
            directive,
            origin,
            resource_type,
        } => {
            let norm_dir = normalize_csp_directive(directive);
            let clean_origin = sanitize_single_line(origin);
            let clean_type = sanitize_single_line(resource_type);
            format!("blocked {clean_type} {clean_origin} ({norm_dir}) — {hint}")
        }
        DevDiagnosticEvent::BlockedNavigation { origin, rule } => {
            let clean_origin = sanitize_single_line(origin);
            let clean_rule = sanitize_single_line(rule);
            format!("blocked navigation {clean_origin} (rule: {clean_rule}) — {hint}")
        }
        DevDiagnosticEvent::BlockedNewWindow { origin, rule } => {
            let clean_origin = sanitize_single_line(origin);
            let clean_rule = sanitize_single_line(rule);
            format!("blocked new-window {clean_origin} (rule: {clean_rule}) — {hint}")
        }
        DevDiagnosticEvent::JsError { message, location } => {
            let summary = format_error_summary(message, location);
            format!("js error: {summary} — {hint}")
        }
        DevDiagnosticEvent::ConsoleError { message, location } => {
            let summary = format_error_summary(message, location);
            format!("console.error: {summary} — {hint}")
        }
        DevDiagnosticEvent::ResourceLoadFailed {
            resource_type,
            origin,
        } => {
            let clean_type = sanitize_single_line(resource_type);
            let clean_origin = sanitize_single_line(origin);
            format!("failed resource load {clean_type} {clean_origin} — {hint}")
        }
    };

    if count > 1 {
        format!("[x{count}] {body}")
    } else {
        body
    }
}

/// Summarize a full `Content-Security-Policy` header to the key directives that matter on one line.
#[must_use]
pub fn summarize_csp(csp: &str) -> String {
    const IMPORTANT_DIRECTIVES: &[&str] = &[
        "default-src",
        "script-src",
        "style-src",
        "img-src",
        "font-src",
        "media-src",
        "connect-src",
        "frame-src",
    ];

    let mut kept = Vec::new();
    for raw_part in csp.split(';') {
        let part = raw_part.split_whitespace().collect::<Vec<_>>().join(" ");
        if part.is_empty() {
            continue;
        }
        let name = part.split(' ').next().unwrap_or("");
        if !IMPORTANT_DIRECTIVES.contains(&name) {
            continue;
        }
        if name == "script-src" {
            let tokens: Vec<&str> = part.split(' ').collect();
            let hash_count = tokens.iter().filter(|t| t.starts_with("'sha256-")).count();
            if hash_count > 2 {
                let non_hash: Vec<&str> = tokens
                    .into_iter()
                    .filter(|t| !t.starts_with("'sha256-"))
                    .collect();
                kept.push(format!(
                    "{} (+{hash_count} inline-hashes)",
                    non_hash.join(" ")
                ));
                continue;
            }
        }
        kept.push(part);
    }

    if kept.is_empty() {
        sanitize_single_line(csp)
    } else {
        kept.join("; ")
    }
}

#[derive(Debug, Clone)]
struct DedupEntry {
    event: DevDiagnosticEvent,
    total_count: usize,
    unprinted_count: usize,
    last_printed_at: Option<Instant>,
}

/// Deduplicates identical dev diagnostics with a count and rate-limits terminal forwarding.
#[derive(Debug)]
pub struct DevDiagnosticAggregator {
    mode: NetworkMode,
    rate_limit_per_sec: usize,
    emitted_timestamps: VecDeque<Instant>,
    suppressed_in_window: usize,
    last_rate_summary_at: Option<Instant>,
    entries: HashMap<String, DedupEntry>,
    insertion_order: VecDeque<String>,
}

impl DevDiagnosticAggregator {
    /// Creates a new aggregator with the default 50 lines/sec limit.
    #[must_use]
    pub fn new(mode: NetworkMode) -> Self {
        Self::with_rate_limit(mode, DEV_DIAGNOSTIC_RATE_LIMIT_PER_SEC)
    }

    /// Creates a new aggregator with an explicit per-second line limit.
    #[must_use]
    pub fn with_rate_limit(mode: NetworkMode, rate_limit_per_sec: usize) -> Self {
        Self {
            mode,
            rate_limit_per_sec: rate_limit_per_sec.max(1),
            emitted_timestamps: VecDeque::new(),
            suppressed_in_window: 0,
            last_rate_summary_at: None,
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
        }
    }

    /// Record an incoming diagnostic event at `now`.
    ///
    /// First occurrences emit immediately (unless rate-limited); subsequent identical events
    /// increment the deduplicated count and are emitted later by [`Self::flush`].
    pub fn push(&mut self, event: DevDiagnosticEvent, now: Instant) -> Option<String> {
        self.prune_window(now);
        let key = dedup_key(&event);

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.total_count += 1;
            entry.unprinted_count += 1;
            return None;
        }

        if self.entries.len() >= MAX_TRACKED_DEDUP_KEYS
            && let Some(oldest) = self.insertion_order.pop_front()
        {
            self.entries.remove(&oldest);
        }

        let can_emit = self.emitted_timestamps.len() < self.rate_limit_per_sec;
        if can_emit {
            self.emitted_timestamps.push_back(now);
            let line = format_diagnostic_line(self.mode, &event, 1);
            self.entries.insert(
                key.clone(),
                DedupEntry {
                    event,
                    total_count: 1,
                    unprinted_count: 0,
                    last_printed_at: Some(now),
                },
            );
            self.insertion_order.push_back(key);
            Some(line)
        } else {
            self.suppressed_in_window += 1;
            if self.last_rate_summary_at.is_none() {
                self.last_rate_summary_at = Some(now);
            }
            self.entries.insert(
                key.clone(),
                DedupEntry {
                    event,
                    total_count: 1,
                    unprinted_count: 0,
                    last_printed_at: Some(now),
                },
            );
            self.insertion_order.push_back(key);
            None
        }
    }

    /// Flush deduplicated repeat counts and any rate-limit suppression summary.
    pub fn flush(&mut self, now: Instant, force: bool) -> Vec<String> {
        self.prune_window(now);
        let mut out = Vec::new();

        if self.suppressed_in_window > 0 {
            let window_elapsed = self
                .last_rate_summary_at
                .is_some_and(|t| now.duration_since(t) >= Duration::from_secs(1));
            if force || window_elapsed {
                out.push(format!(
                    "rate limit: suppressed {} diagnostic message(s) (limit: {} lines/s)",
                    self.suppressed_in_window, self.rate_limit_per_sec
                ));
                self.suppressed_in_window = 0;
                self.last_rate_summary_at = Some(now);
            }
        }

        let flush_delay = Duration::from_millis(DEDUP_FLUSH_INTERVAL_MS);
        let keys: Vec<String> = self.insertion_order.iter().cloned().collect();
        for key in keys {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if entry.unprinted_count == 0 {
                continue;
            }
            let ready = force
                || entry
                    .last_printed_at
                    .is_none_or(|t| now.duration_since(t) >= flush_delay);
            if !ready {
                continue;
            }
            if !force && self.emitted_timestamps.len() >= self.rate_limit_per_sec {
                self.suppressed_in_window += 1;
                if self.last_rate_summary_at.is_none() {
                    self.last_rate_summary_at = Some(now);
                }
                continue;
            }
            if !force {
                self.emitted_timestamps.push_back(now);
            }
            out.push(format_diagnostic_line(
                self.mode,
                &entry.event,
                entry.total_count,
            ));
            entry.unprinted_count = 0;
            entry.last_printed_at = Some(now);
        }

        out
    }

    fn prune_window(&mut self, now: Instant) {
        let one_sec = Duration::from_secs(1);
        while let Some(&front) = self.emitted_timestamps.front() {
            if now.duration_since(front) >= one_sec {
                self.emitted_timestamps.pop_front();
            } else {
                break;
            }
        }
    }
}

fn dedup_key(event: &DevDiagnosticEvent) -> String {
    match event {
        DevDiagnosticEvent::CspViolation {
            directive,
            origin,
            resource_type,
        } => format!(
            "csp|{}|{}|{}",
            normalize_csp_directive(directive),
            sanitize_single_line(origin),
            sanitize_single_line(resource_type)
        ),
        DevDiagnosticEvent::BlockedNavigation { origin, rule } => format!(
            "nav|{}|{}",
            sanitize_single_line(origin),
            sanitize_single_line(rule)
        ),
        DevDiagnosticEvent::BlockedNewWindow { origin, rule } => format!(
            "popup|{}|{}",
            sanitize_single_line(origin),
            sanitize_single_line(rule)
        ),
        DevDiagnosticEvent::JsError { message, location } => {
            format!("js|{}", format_error_summary(message, location))
        }
        DevDiagnosticEvent::ConsoleError { message, location } => {
            format!("console|{}", format_error_summary(message, location))
        }
        DevDiagnosticEvent::ResourceLoadFailed {
            resource_type,
            origin,
        } => format!(
            "res|{}|{}",
            sanitize_single_line(resource_type),
            sanitize_single_line(origin)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csp::{ULTRA_SAFE_CSP, generate_csp};

    #[test]
    fn hint_text_matches_each_mode() {
        let csp_ev = DevDiagnosticEvent::CspViolation {
            directive: "script-src-elem".to_string(),
            origin: "https://cdn.tailwindcss.com".to_string(),
            resource_type: "script".to_string(),
        };

        let ultra = format_diagnostic_line(NetworkMode::UltraSafe, &csp_ev, 1);
        assert_eq!(
            ultra,
            "blocked script https://cdn.tailwindcss.com (script-src) — allowed in \"wavical dev\"; in dev-safe add it to scripts in wavical.allowlist.json"
        );

        let safe = format_diagnostic_line(NetworkMode::Safe, &csp_ev, 1);
        assert!(safe.contains("add it to scripts in wavical.allowlist.json"));
        assert!(safe.contains("wavical dev"));

        let open = format_diagnostic_line(NetworkMode::Open, &csp_ev, 1);
        assert!(open.contains("open-mode CSP"));

        let nav_ev = DevDiagnosticEvent::BlockedNavigation {
            origin: "https://login.company.com".to_string(),
            rule: "not_in_navigate_allowlist".to_string(),
        };
        assert!(
            format_diagnostic_line(NetworkMode::UltraSafe, &nav_ev, 1)
                .contains("in dev-safe add it to navigate")
        );
        assert!(
            format_diagnostic_line(NetworkMode::Safe, &nav_ev, 1)
                .contains("add it to navigate in wavical.allowlist.json")
        );

        let baseline_nav = DevDiagnosticEvent::BlockedNavigation {
            origin: "file:".to_string(),
            rule: "baseline_blocked_scheme".to_string(),
        };
        assert!(
            format_diagnostic_line(NetworkMode::Open, &baseline_nav, 1)
                .contains("baseline security policy in all modes")
        );
    }

    #[test]
    fn js_error_and_console_error_truncate_to_500_chars_and_strip_query() {
        let huge = "E".repeat(900);
        let loc = "http://wavical.localhost/app.js?token=SECRET_TOKEN:42:10";
        let summary = format_error_summary(&huge, loc);
        assert!(summary.chars().count() <= MAX_JS_ERROR_CHARS);
        assert!(!summary.contains("SECRET_TOKEN"));
        assert!(summary.contains("http://wavical.localhost/app.js:42:10"));
        assert!(summary.contains("..."));
    }

    #[test]
    fn deduplicates_with_count_and_rate_limits_at_50_per_sec() {
        let t0 = Instant::now();
        let mut agg = DevDiagnosticAggregator::new(NetworkMode::UltraSafe);

        let ev = DevDiagnosticEvent::CspViolation {
            directive: "img-src".to_string(),
            origin: "https://www.designarena.ai".to_string(),
            resource_type: "image".to_string(),
        };
        let first = agg.push(ev.clone(), t0).expect("first prints immediately");
        assert!(first.starts_with("blocked image https://www.designarena.ai (img-src)"));
        for _ in 0..5 {
            assert!(agg.push(ev.clone(), t0).is_none());
        }

        // Flush after delay shows [x6]
        let flushed = agg.flush(t0 + Duration::from_millis(400), false);
        assert_eq!(flushed.len(), 1);
        assert!(flushed[0].starts_with("[x6] blocked image https://www.designarena.ai"));

        // Flood 70 distinct origins within the same second (already emitted 2 in window)
        let t1 = t0 + Duration::from_secs(2);
        let mut emitted = 0usize;
        for i in 0..70 {
            let distinct = DevDiagnosticEvent::CspViolation {
                directive: "script-src".to_string(),
                origin: format!("https://cdn{i}.example.com"),
                resource_type: "script".to_string(),
            };
            if agg.push(distinct, t1).is_some() {
                emitted += 1;
            }
        }
        assert_eq!(emitted, DEV_DIAGNOSTIC_RATE_LIMIT_PER_SEC);

        let summary_lines = agg.flush(t1 + Duration::from_secs(1), false);
        assert!(
            summary_lines
                .iter()
                .any(|l| l.contains("rate limit: suppressed 20 diagnostic message(s)"))
        );
    }

    #[test]
    fn summarize_csp_keeps_key_directives() {
        let summary = summarize_csp(ULTRA_SAFE_CSP);
        assert!(summary.contains("default-src 'self'"));
        assert!(summary.contains("script-src 'self'"));
        assert!(summary.contains("connect-src 'self'"));
        assert!(!summary.contains("object-src"));

        let open = generate_csp(NetworkMode::Open, None, &[], None).unwrap();
        let open_summary = summarize_csp(&open);
        assert!(open_summary.contains("script-src 'self' https: 'unsafe-inline' 'unsafe-eval'"));
    }
}
