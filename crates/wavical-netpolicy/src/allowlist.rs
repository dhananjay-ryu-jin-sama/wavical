//! Allowlist parser and validator for `wavical.allowlist.json` (Plan 1.4).

use std::collections::HashSet;

use serde::Deserialize;
use thiserror::Error;

use crate::limits::{MAX_ALLOWLIST_FILE_BYTES, MAX_ENTRIES_PER_LIST, MAX_LOCAL_EXCEPTIONS};
use crate::origin::{
    AllowlistPattern, OriginError, ParsedOrigin, SchemePolicy, parse_allowlist_pattern,
    parse_exact_origin,
};

/// Errors produced when parsing or validating `wavical.allowlist.json`.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AllowlistError {
    #[error("allowlist file exceeds maximum size of {MAX_ALLOWLIST_FILE_BYTES} bytes")]
    FileTooLarge,
    #[error("invalid allowlist JSON: {0}")]
    InvalidJson(String),
    #[error("unsupported allowlist version {0}; expected 1")]
    UnsupportedVersion(u32),
    #[error("list '{list}' exceeds maximum of {MAX_ENTRIES_PER_LIST} entries")]
    ListTooLong { list: &'static str },
    #[error("localExceptions exceeds maximum of {MAX_LOCAL_EXCEPTIONS} entries")]
    TooManyLocalExceptions,
    #[error("duplicate entry '{entry}' in '{list}'")]
    DuplicateEntry { list: &'static str, entry: String },
    #[error("localExceptions[{index}] requires a non-empty reason")]
    MissingLocalExceptionReason { index: usize },
    #[error("invalid entry in '{list}': {source}")]
    InvalidEntry {
        list: &'static str,
        #[source]
        source: OriginError,
    },
    #[error("remoteUi.bridge capability '{0}' is not granted in wavical.config.js")]
    BridgeCapabilityNotGranted(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawAllowlist {
    version: u32,
    #[serde(default)]
    scripts: Vec<String>,
    #[serde(default)]
    styles: Vec<String>,
    #[serde(default)]
    fonts: Vec<String>,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    media: Vec<String>,
    #[serde(default)]
    connect: Vec<String>,
    #[serde(default)]
    frames: Vec<String>,
    #[serde(default)]
    navigate: Vec<String>,
    #[serde(default)]
    open_external: Vec<String>,
    #[serde(default)]
    inline_styles: bool,
    #[serde(default)]
    data_images: bool,
    #[serde(default)]
    remote_ui: Option<RawRemoteUi>,
    #[serde(default)]
    local_exceptions: Vec<RawLocalException>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRemoteUi {
    origin: String,
    #[serde(default)]
    bridge: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLocalException {
    origin: String,
    reason: String,
}

/// Validated `remoteUi` configuration from `wavical.allowlist.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUiAllowlist {
    pub origin: ParsedOrigin,
    pub bridge: Vec<String>,
}

/// Validated `localExceptions` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalException {
    pub origin: ParsedOrigin,
    pub reason: String,
}

/// Validated `wavical.allowlist.json` policy.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Allowlist {
    pub version: u32,
    pub scripts: Vec<AllowlistPattern>,
    pub styles: Vec<AllowlistPattern>,
    pub fonts: Vec<AllowlistPattern>,
    pub images: Vec<AllowlistPattern>,
    pub media: Vec<AllowlistPattern>,
    pub connect: Vec<AllowlistPattern>,
    pub frames: Vec<AllowlistPattern>,
    pub navigate: Vec<AllowlistPattern>,
    pub open_external: Vec<AllowlistPattern>,
    pub inline_styles: bool,
    pub data_images: bool,
    pub remote_ui: Option<RemoteUiAllowlist>,
    pub local_exceptions: Vec<LocalException>,
}

impl Allowlist {
    /// Parses and validates `wavical.allowlist.json` from raw UTF-8 bytes.
    pub fn parse_bytes(raw_bytes: &[u8]) -> Result<Self, AllowlistError> {
        if raw_bytes.len() > MAX_ALLOWLIST_FILE_BYTES {
            return Err(AllowlistError::FileTooLarge);
        }
        let raw: RawAllowlist = serde_json::from_slice(raw_bytes)
            .map_err(|e| AllowlistError::InvalidJson(e.to_string()))?;

        if raw.version != 1 {
            return Err(AllowlistError::UnsupportedVersion(raw.version));
        }

        let scripts = parse_pattern_list("scripts", &raw.scripts, SchemePolicy::HttpsOnly)?;
        let styles = parse_pattern_list("styles", &raw.styles, SchemePolicy::HttpsOnly)?;
        let fonts = parse_pattern_list("fonts", &raw.fonts, SchemePolicy::HttpsOnly)?;
        let images = parse_pattern_list("images", &raw.images, SchemePolicy::HttpsOnly)?;
        let media = parse_pattern_list("media", &raw.media, SchemePolicy::HttpsOnly)?;
        let connect = parse_pattern_list("connect", &raw.connect, SchemePolicy::HttpsOrWss)?;
        let frames = parse_pattern_list("frames", &raw.frames, SchemePolicy::HttpsOnly)?;
        let navigate = parse_pattern_list("navigate", &raw.navigate, SchemePolicy::HttpsOnly)?;
        let open_external =
            parse_pattern_list("openExternal", &raw.open_external, SchemePolicy::HttpsOnly)?;

        let remote_ui = match raw.remote_ui {
            Some(rui) => {
                let origin = parse_exact_origin(&rui.origin, SchemePolicy::HttpsOnly, false)
                    .map_err(|source| AllowlistError::InvalidEntry {
                        list: "remoteUi.origin",
                        source,
                    })?;
                let mut seen_caps = HashSet::new();
                let mut bridge = Vec::with_capacity(rui.bridge.len());
                for cap in rui.bridge {
                    if !seen_caps.insert(cap.clone()) {
                        return Err(AllowlistError::DuplicateEntry {
                            list: "remoteUi.bridge",
                            entry: cap,
                        });
                    }
                    bridge.push(cap);
                }
                Some(RemoteUiAllowlist { origin, bridge })
            }
            None => None,
        };

        if raw.local_exceptions.len() > MAX_LOCAL_EXCEPTIONS {
            return Err(AllowlistError::TooManyLocalExceptions);
        }
        let mut seen_local = HashSet::new();
        let mut local_exceptions = Vec::with_capacity(raw.local_exceptions.len());
        for (idx, exc) in raw.local_exceptions.into_iter().enumerate() {
            let reason = exc.reason.trim().to_string();
            if reason.is_empty() {
                return Err(AllowlistError::MissingLocalExceptionReason { index: idx });
            }
            let origin = parse_exact_origin(&exc.origin, SchemePolicy::LocalException, true)
                .map_err(|source| AllowlistError::InvalidEntry {
                    list: "localExceptions",
                    source,
                })?;
            let key = origin.as_canonical();
            if !seen_local.insert(key.clone()) {
                return Err(AllowlistError::DuplicateEntry {
                    list: "localExceptions",
                    entry: key,
                });
            }
            local_exceptions.push(LocalException { origin, reason });
        }

        Ok(Self {
            version: raw.version,
            scripts,
            styles,
            fonts,
            images,
            media,
            connect,
            frames,
            navigate,
            open_external,
            inline_styles: raw.inline_styles,
            data_images: raw.data_images,
            remote_ui,
            local_exceptions,
        })
    }

    /// Validates that `remoteUi.bridge` capabilities are a subset of `granted_capabilities`.
    pub fn validate_remote_ui_bridge(
        &self,
        granted_capabilities: &[String],
    ) -> Result<(), AllowlistError> {
        let Some(rui) = &self.remote_ui else {
            return Ok(());
        };
        let granted: HashSet<&str> = granted_capabilities.iter().map(String::as_str).collect();
        for cap in &rui.bridge {
            if !granted.contains(cap.as_str()) {
                return Err(AllowlistError::BridgeCapabilityNotGranted(cap.clone()));
            }
        }
        Ok(())
    }

    /// Returns true if `origin` is explicitly listed in `localExceptions`.
    #[must_use]
    pub fn has_local_exception(&self, origin: &ParsedOrigin) -> bool {
        self.local_exceptions.iter().any(|e| &e.origin == origin)
    }
}

fn parse_pattern_list(
    list_name: &'static str,
    entries: &[String],
    scheme_policy: SchemePolicy,
) -> Result<Vec<AllowlistPattern>, AllowlistError> {
    if entries.len() > MAX_ENTRIES_PER_LIST {
        return Err(AllowlistError::ListTooLong { list: list_name });
    }
    let mut seen = HashSet::with_capacity(entries.len());
    let mut out = Vec::with_capacity(entries.len());
    for raw in entries {
        let pat = parse_allowlist_pattern(raw, scheme_policy).map_err(|source| {
            AllowlistError::InvalidEntry {
                list: list_name,
                source,
            }
        })?;
        let canonical = pat.as_canonical();
        if !seen.insert(canonical.clone()) {
            return Err(AllowlistError::DuplicateEntry {
                list: list_name,
                entry: canonical,
            });
        }
        out.push(pat);
    }
    Ok(out)
}
