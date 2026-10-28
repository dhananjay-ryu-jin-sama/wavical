#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use thiserror::Error;

/// Default window width in pixels.
pub const DEFAULT_WINDOW_WIDTH: u32 = 1200;

/// Default window height in pixels.
pub const DEFAULT_WINDOW_HEIGHT: u32 = 800;

/// Default development server port.
pub const DEFAULT_DEV_PORT: u16 = 7890;

/// Maximum allowable execution time for the Node config evaluation subprocess (5 seconds).
pub const CONFIG_NODE_TIMEOUT: Duration = Duration::from_secs(5);

fn default_dev_port() -> u16 {
    DEFAULT_DEV_PORT
}

/// Window configuration options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Persist window size, position, and maximized state across sessions (default: true).
    #[serde(
        default,
        rename = "persistState",
        alias = "persist_state",
        skip_serializing_if = "Option::is_none"
    )]
    pub persist_state: Option<bool>,
}

impl WindowConfig {
    /// Return the explicit width or the framework default (1200).
    pub fn width_or_default(&self) -> u32 {
        self.width.unwrap_or(DEFAULT_WINDOW_WIDTH)
    }

    /// Return the explicit height or the framework default (800).
    pub fn height_or_default(&self) -> u32 {
        self.height.unwrap_or(DEFAULT_WINDOW_HEIGHT)
    }

    /// Return the explicit title or fallback to the provided app identifier.
    pub fn title_or_default<'a>(&'a self, app_id: &'a str) -> &'a str {
        self.title.as_deref().unwrap_or(app_id)
    }

    /// Return whether window state persistence is enabled (default: true).
    pub fn persist_state_or_default(&self) -> bool {
        self.persist_state.unwrap_or(true)
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: Some(DEFAULT_WINDOW_WIDTH),
            height: Some(DEFAULT_WINDOW_HEIGHT),
            title: None,
            persist_state: None,
        }
    }
}

/// Development server configuration options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevConfig {
    #[serde(default = "default_dev_port")]
    pub port: u16,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_DEV_PORT,
        }
    }
}

/// Top-level application configuration declared in `wavical.config.js`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Unique identifier for the application (alphanumeric, hyphens, underscores).
    #[serde(rename = "appId", alias = "app_id")]
    pub app_id: String,

    /// Optional window settings (width, height, title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowConfig>,

    /// Map of declared capabilities and their granted state (e.g. `core.ping: true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<HashMap<String, bool>>,

    /// Optional application version string (e.g. "0.1.0"). Defaults to "0.1.0".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Optional development server settings (port).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev: Option<DevConfig>,

    /// Optional dialog settings (fixed filter specifications).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialog: Option<DialogConfig>,

    /// Optional single-instance lock setting (default: true).
    #[serde(
        default,
        rename = "singleInstance",
        alias = "single_instance",
        skip_serializing_if = "Option::is_none"
    )]
    pub single_instance: Option<bool>,

    /// Optional system tray configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray: Option<TrayConfig>,

    /// Optional global shortcuts registered at application startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcuts: Option<Vec<String>>,

    /// Optional file associations list (each extension must match `^[a-z0-9]{1,10}$`).
    #[serde(
        default,
        rename = "fileAssociations",
        alias = "file_associations",
        skip_serializing_if = "Option::is_none"
    )]
    pub file_associations: Option<Vec<String>>,

    /// Optional AI subsystem configuration (Phase 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiConfig>,
}

/// Configuration options for the AI subsystem (`ai` section).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiConfig {
    /// Enable AI features (default: false). Everything below is ignored if false.
    #[serde(default)]
    pub enabled: bool,

    /// Inference routing mode ('local', 'cloud', or 'local-first', default: 'local').
    #[serde(default = "default_ai_mode")]
    pub mode: AiMode,

    /// Local inference engine configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<AiLocalConfig>,

    /// Cloud inference fallback and direct provider configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<AiCloudConfig>,
}

fn default_ai_mode() -> AiMode {
    AiMode::Local
}

/// AI execution routing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AiMode {
    #[default]
    Local,
    Cloud,
    LocalFirst,
}

/// Local inference model configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiLocalConfig {
    /// Catalog identifier for the chat completion model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,

    /// Catalog identifier for the document embedding model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_model: Option<String>,

    /// Context window size in tokens (ceiling 8192, default 4096).
    #[serde(default = "default_context_size")]
    pub context_size: usize,

    /// Maximum tokens per generated response (ceiling 4096, default 1024).
    #[serde(default = "default_local_max_tokens")]
    pub max_tokens: u32,

    /// Inactivity idle timeout in minutes before engine process shutdown (default 10).
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u32,

    /// Compute threads ('auto' or count between 1 and 128, default: 'auto').
    #[serde(default = "default_threads")]
    pub threads: AiThreads,
}

fn default_context_size() -> usize {
    wavical_ipc::DEFAULT_LOCAL_CONTEXT_SIZE
}

fn default_local_max_tokens() -> u32 {
    wavical_ipc::DEFAULT_CHAT_MAX_TOKENS
}

fn default_idle_minutes() -> u32 {
    10
}

fn default_threads() -> AiThreads {
    AiThreads::Auto("auto".to_string())
}

fn default_true() -> bool {
    true
}

/// Thread allocation setting for local engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AiThreads {
    Auto(String),
    Count(usize),
}

/// Cloud provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiCloudConfig {
    /// Cloud API protocol ('openai-compatible').
    pub protocol: String,

    /// Base URL of provider endpoint (strictly HTTPS, no userinfo).
    pub base_url: String,

    /// Model name at provider (e.g. 'gpt-4o-mini').
    pub model: String,

    /// Environment variable name holding the API key (e.g. 'OPENAI_API_KEY').
    pub api_key_env: String,

    /// Require explicit native consent dialog before first cloud call (default: true).
    #[serde(default = "default_true")]
    pub require_consent: bool,
}

/// System tray configuration options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrayConfig {
    /// Relative path inside src/ to the tray icon file (e.g. "icon.png").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Dialog configuration options defining allowed file filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DialogConfig {
    /// Fixed list of allowed file filters mapped by filter ID.
    #[serde(default)]
    pub filters: HashMap<String, DialogFilterConfig>,
}

/// Filter specification with human-readable name and list of allowed file extensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DialogFilterConfig {
    pub name: String,
    pub extensions: Vec<String>,
}

impl AppConfig {
    /// Construct a new minimal `AppConfig` with the given appId.
    pub fn new(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            version: None,
            window: None,
            capabilities: None,
            dev: None,
            dialog: None,
            single_instance: None,
            tray: None,
            shortcuts: None,
            file_associations: None,
            ai: None,
        }
    }

    /// Return the configured tray icon path relative to src/, if specified.
    pub fn tray_icon(&self) -> Option<&str> {
        self.tray.as_ref().and_then(|t| t.icon.as_deref())
    }

    /// Return whether single-instance lock is enabled (default: true).
    pub fn single_instance_or_default(&self) -> bool {
        self.single_instance.unwrap_or(true)
    }

    /// Return whether window state persistence is enabled (default: true).
    pub fn persist_window_state(&self) -> bool {
        self.window
            .as_ref()
            .map(|w| w.persist_state_or_default())
            .unwrap_or(true)
    }

    /// Effective application version string (default: "0.1.0").
    pub fn version_or_default(&self) -> &str {
        self.version.as_deref().unwrap_or("0.1.0")
    }

    /// Helper to access appId in snake_case style.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// Helper alias for camelCase callers.
    #[allow(non_snake_case)]
    pub fn appId(&self) -> &str {
        &self.app_id
    }

    /// Effective window width (uses default 1200 if not specified).
    pub fn window_width(&self) -> u32 {
        self.window
            .as_ref()
            .and_then(|w| w.width)
            .unwrap_or(DEFAULT_WINDOW_WIDTH)
    }

    /// Effective window height (uses default 800 if not specified).
    pub fn window_height(&self) -> u32 {
        self.window
            .as_ref()
            .and_then(|w| w.height)
            .unwrap_or(DEFAULT_WINDOW_HEIGHT)
    }

    /// Effective window title (uses appId if not specified).
    pub fn window_title(&self) -> &str {
        self.window
            .as_ref()
            .and_then(|w| w.title.as_deref())
            .unwrap_or(&self.app_id)
    }

    /// Effective dev port (uses default 7890 if not specified).
    pub fn dev_port(&self) -> u16 {
        self.dev
            .as_ref()
            .map(|d| d.port)
            .unwrap_or(DEFAULT_DEV_PORT)
    }

    /// Check if a specific capability has been explicitly granted (`true`).
    pub fn is_capability_granted(&self, capability: &str) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|caps| caps.get(capability).copied())
            .unwrap_or(false)
    }

    /// Return all capabilities that are explicitly granted (`true`).
    pub fn granted_capabilities(&self) -> Vec<String> {
        let mut granted = Vec::new();
        if let Some(ref caps) = self.capabilities {
            for (name, &is_granted) in caps {
                if is_granted {
                    granted.push(name.clone());
                }
            }
        }
        granted.sort();
        granted
    }
}

/// Compute Levenshtein edit distance between two strings.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    let mut dp = vec![vec![0usize; n + 1]; m + 1];

    for i in 0..=m {
        if let Some(row) = dp.get_mut(i)
            && let Some(val) = row.get_mut(0)
        {
            *val = i;
        }
    }
    for (j, val) in dp
        .get_mut(0)
        .unwrap_or(&mut Vec::new())
        .iter_mut()
        .enumerate()
    {
        *val = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars.get(i - 1) == b_chars.get(j - 1) {
                0
            } else {
                1
            };
            let del = dp
                .get(i - 1)
                .and_then(|r| r.get(j))
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let ins = dp
                .get(i)
                .and_then(|r| r.get(j - 1))
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let sub = dp
                .get(i - 1)
                .and_then(|r| r.get(j - 1))
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(cost);

            let min_val = del.min(ins).min(sub);
            if let Some(row) = dp.get_mut(i)
                && let Some(cell) = row.get_mut(j)
            {
                *cell = min_val;
            }
        }
    }

    dp.get(m).and_then(|r| r.get(n)).copied().unwrap_or(0)
}

/// Find the closest matching capability name within a reasonable distance threshold.
pub fn closest_match<'a>(query: &str, candidates: &'a [String]) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;

    for candidate in candidates {
        let dist = levenshtein_distance(query, candidate);
        match best {
            None => best = Some((candidate.as_str(), dist)),
            Some((_, best_dist)) if dist < best_dist => {
                best = Some((candidate.as_str(), dist));
            }
            _ => {}
        }
    }

    if let Some((cand, dist)) = best
        && (dist <= 3 || dist <= query.len().saturating_div(2) + 1)
    {
        return Some(cand);
    }
    None
}

fn format_unknown_capability(unknown: &str, known: &[String]) -> String {
    let known_str = known.join(", ");
    if let Some(suggestion) = closest_match(unknown, known) {
        format!("unknown capability '{unknown}'; did you mean '{suggestion}'? (known: {known_str})")
    } else {
        format!("unknown capability '{unknown}' (known: {known_str})")
    }
}

/// Comprehensive errors that can occur during config loading and validation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("missing required field '{0}'")]
    MissingField(String),

    #[error("{}", format_unknown_capability(.0, .1))]
    UnknownCapability(String, Vec<String>),

    #[error("failed to parse config JSON: {0}")]
    MalformedJson(String),

    #[error("config parsing timed out (>5 seconds); check for infinite loops in wavical.config.js")]
    NodeTimeout,

    #[error("Node subprocess failed: {0}")]
    NodeError(String),

    #[error("wavical.config.js not found in project root")]
    ConfigNotFound,

    #[error("appId must be alphanumeric (a-z, 0-9, -, _), got '{0}'")]
    InvalidAppId(String),

    #[error("invalid file association extension: {0}")]
    InvalidFileAssociation(String),

    #[error("invalid shortcut specification: {0}")]
    InvalidShortcut(String),

    #[error("AI configuration error: {0}")]
    InvalidAiConfig(String),

    #[error(
        "in 'capabilities.{0}': capability '{0}' requires 'ai.enabled: true'; set 'ai: {{ enabled: true }}' in wavical.config.js to enable AI capabilities"
    )]
    AiCapabilityRequiresAiEnabled(String),
}

impl ValidationError {
    /// Return the suggested capability name if this error is an `UnknownCapability` with a close match.
    pub fn suggestion(&self) -> Option<&str> {
        match self {
            ValidationError::UnknownCapability(unknown, known) => closest_match(unknown, known),
            _ => None,
        }
    }
}

/// Validate that an appId contains only alphanumeric characters, hyphens, and underscores.
pub fn is_valid_app_id(id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn extract_missing_field(msg: &str) -> Option<String> {
    if let Some(start) = msg.find("missing field `") {
        let remainder = &msg[start + "missing field `".len()..];
        if let Some(end) = remainder.find('`') {
            return Some(remainder[..end].to_string());
        }
    }
    if let Some(start) = msg.find("missing field '") {
        let remainder = &msg[start + "missing field '".len()..];
        if let Some(end) = remainder.find('\'') {
            return Some(remainder[..end].to_string());
        }
    }
    None
}

pub const CONFIG_DELIMITER_START: &str = "__WAVICAL_CONFIG_JSON_START__";

/// Delimiter marking the end of the evaluated config JSON payload.
pub const CONFIG_DELIMITER_END: &str = "__WAVICAL_CONFIG_JSON_END__";

/// Extract JSON payload between delimiter tokens if present, or return trimmed string.
pub fn extract_config_json(raw: &str) -> &str {
    if let Some(start_idx) = raw.find(CONFIG_DELIMITER_START) {
        let after_start = &raw[start_idx + CONFIG_DELIMITER_START.len()..];
        if let Some(end_idx) = after_start.find(CONFIG_DELIMITER_END) {
            return after_start[..end_idx].trim();
        }
    }
    raw.trim()
}

/// Parse and validate a JSON string into an `AppConfig`.
///
/// Enforces:
/// - Presence and formatting of required `appId`
/// - Strict schema validation (`deny_unknown_fields`)
/// - Capability validation against the `wavical-ipc` registry
/// - File associations extension regex (`^[a-z0-9]{1,10}$`)
/// - Global shortcut grammar and uniqueness limits (max 32)
pub fn parse_config_json(json_str: &str) -> Result<AppConfig, ValidationError> {
    let extracted = extract_config_json(json_str);
    let trimmed = extracted.trim();
    if trimmed.is_empty() || trimmed == "undefined" || trimmed == "null" {
        return Err(ValidationError::MalformedJson(
            "wavical.config.js must export a default configuration object".to_string(),
        ));
    }

    let config: AppConfig = serde_json::from_str(trimmed).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("missing field `appId`") || msg.contains("missing field 'appId'") {
            ValidationError::MissingField("appId".to_string())
        } else if let Some(field) = extract_missing_field(&msg) {
            ValidationError::MissingField(field)
        } else {
            ValidationError::MalformedJson(msg)
        }
    })?;

    if config.app_id.trim().is_empty() {
        return Err(ValidationError::MissingField("appId".to_string()));
    }

    if !is_valid_app_id(&config.app_id) {
        return Err(ValidationError::InvalidAppId(config.app_id.clone()));
    }

    if let Some(ref caps) = config.capabilities {
        let known = wavical_ipc::known_capabilities();
        let mut sorted_known = known.clone();
        sorted_known.sort();

        let mut declared_keys: Vec<&String> = caps.keys().collect();
        declared_keys.sort();

        for cap_name in declared_keys {
            if !known.iter().any(|k| k == cap_name) {
                return Err(ValidationError::UnknownCapability(
                    cap_name.clone(),
                    sorted_known,
                ));
            }
        }
    }

    if let Some(ref exts) = config.file_associations {
        for ext in exts {
            wavical_ipc::validate_file_extension(ext)
                .map_err(|e| ValidationError::InvalidFileAssociation(e.to_string()))?;
        }
    }

    if let Some(ref shortcuts) = config.shortcuts {
        wavical_ipc::validate_shortcut_list(shortcuts)
            .map_err(|e| ValidationError::InvalidShortcut(e.to_string()))?;
    }

    if let Some(ref ai) = config.ai {
        validate_ai_config(ai)?;
    }

    // Phase 3 rule: any ai.* capability requires ai.enabled: true
    let ai_enabled = config.ai.as_ref().map(|a| a.enabled).unwrap_or(false);
    if let Some(ref caps) = config.capabilities {
        let mut sorted_caps: Vec<&String> = caps.keys().collect();
        sorted_caps.sort();
        for cap in sorted_caps {
            let is_granted = *caps.get(cap).unwrap_or(&false);
            if is_granted && cap.starts_with("ai.") && !ai_enabled {
                return Err(ValidationError::AiCapabilityRequiresAiEnabled(cap.clone()));
            }
        }
    }

    Ok(config)
}

/// Validate an environment variable name (`^[A-Z][A-Z0-9_]*$`).
pub fn is_valid_env_var_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_uppercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Hook for catalog membership validation. In WP2, validates catalog model ID format.
/// Validate catalog model identifier format and catalog membership.
pub fn validate_catalog_membership_hook(id: &str) -> Result<(), ValidationError> {
    wavical_ipc::validate_catalog_model_id(id)
        .map_err(|e| ValidationError::InvalidAiConfig(e.to_string()))?;

    if !wavical_ipc::is_known_catalog_model(id) {
        let known = wavical_ipc::CATALOG_MODEL_IDS.join(", ");
        return Err(ValidationError::InvalidAiConfig(format!(
            "model '{id}' is not in the approved model catalog; select an approved model ID (known: {known})"
        )));
    }

    Ok(())
}

/// Validate the `ai` section of `wavical.config.js`.
pub fn validate_ai_config(ai: &AiConfig) -> Result<(), ValidationError> {
    if let Some(ref local) = ai.local {
        if let Some(ref chat_model) = local.chat_model {
            validate_catalog_membership_hook(chat_model)?;
        }
        if let Some(ref embed_model) = local.embed_model {
            validate_catalog_membership_hook(embed_model)?;
        }

        if local.context_size == 0
            || local.context_size > wavical_ipc::MAX_LOCAL_CONTEXT_SIZE_CEILING
        {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.local.contextSize': value {} exceeds hard ceiling of {} (or is 0); set contextSize to at most {}",
                local.context_size,
                wavical_ipc::MAX_LOCAL_CONTEXT_SIZE_CEILING,
                wavical_ipc::MAX_LOCAL_CONTEXT_SIZE_CEILING
            )));
        }

        if local.max_tokens == 0 || local.max_tokens > wavical_ipc::MAX_CHAT_MAX_TOKENS_CEILING {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.local.maxTokens': value {} exceeds hard ceiling of {} (or is 0); set maxTokens to at most {}",
                local.max_tokens,
                wavical_ipc::MAX_CHAT_MAX_TOKENS_CEILING,
                wavical_ipc::MAX_CHAT_MAX_TOKENS_CEILING
            )));
        }

        if local.idle_minutes == 0 {
            return Err(ValidationError::InvalidAiConfig(
                "in 'ai.local.idleMinutes': idleMinutes must be greater than 0; set idleMinutes to at least 1".to_string(),
            ));
        }

        match local.threads {
            AiThreads::Auto(ref s) => {
                if s != "auto" {
                    return Err(ValidationError::InvalidAiConfig(format!(
                        "in 'ai.local.threads': invalid thread specification '{s}'; must be 'auto' or an integer between 1 and 128"
                    )));
                }
            }
            AiThreads::Count(c) => {
                if c == 0 || c > 128 {
                    return Err(ValidationError::InvalidAiConfig(format!(
                        "in 'ai.local.threads': invalid thread count {c}; must be between 1 and 128"
                    )));
                }
            }
        }
    }

    if let Some(ref cloud) = ai.cloud {
        if cloud.protocol != "openai-compatible" {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.cloud.protocol': unsupported protocol '{}'; Phase 3 supports 'openai-compatible'. Set protocol to 'openai-compatible'.",
                cloud.protocol
            )));
        }

        if cloud.model.trim().is_empty() {
            return Err(ValidationError::InvalidAiConfig(
                "in 'ai.cloud.model': model name cannot be empty; specify a provider model name (e.g. 'gpt-4o-mini')".to_string(),
            ));
        }

        // baseUrl validation: https-only, has host, no userinfo
        if !cloud.base_url.starts_with("https://") {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.cloud.baseUrl': insecure or invalid scheme in '{}'; cloud endpoints strictly require https:// (e.g. 'https://api.openai.com/v1'). Update baseUrl to start with https://",
                cloud.base_url
            )));
        }

        let after_scheme = &cloud.base_url["https://".len()..];
        let authority = after_scheme.split('/').next().unwrap_or("");
        if authority.is_empty() {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.cloud.baseUrl': missing host in '{}'; specify a valid HTTPS host (e.g. 'https://api.openai.com/v1')",
                cloud.base_url
            )));
        }
        if authority.contains('@') {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.cloud.baseUrl': userinfo credentials are not permitted in '{}'; remove credentials from URL and supply API keys via apiKeyEnv instead",
                cloud.base_url
            )));
        }

        let host = authority.split(':').next().unwrap_or("");
        if host.is_empty() || host.contains(' ') || host.starts_with('.') || host.ends_with('.') {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.cloud.baseUrl': invalid host in '{}'; specify a valid host without spaces (e.g. 'https://api.openai.com/v1')",
                cloud.base_url
            )));
        }

        // apiKeyEnv validation: variable name matching ^[A-Z][A-Z0-9_]*$, reject keys
        if cloud.api_key_env.starts_with("sk-") {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.cloud.apiKeyEnv': value '{}' looks like an actual API key; apiKeyEnv must be an environment variable NAME (matching ^[A-Z][A-Z0-9_]*$), never the secret key itself. Store your secret key in an environment variable and specify its name here.",
                cloud.api_key_env
            )));
        }

        if !is_valid_env_var_name(&cloud.api_key_env) {
            return Err(ValidationError::InvalidAiConfig(format!(
                "in 'ai.cloud.apiKeyEnv': '{}' is not a valid environment variable name; must match pattern ^[A-Z][A-Z0-9_]*$ (e.g. 'OPENAI_API_KEY'). Update apiKeyEnv to a valid uppercase variable name.",
                cloud.api_key_env
            )));
        }
    }

    Ok(())
}

/// Internal execution output capturing process status and buffered standard streams.
#[derive(Debug)]
pub struct NodeExecutionOutput {
    pub status: std::process::ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Execute Node evaluation of `wavical.config.js` with an explicit timeout.
pub fn execute_node_config_script(
    project_root: &Path,
    timeout: Duration,
) -> Result<NodeExecutionOutput, ValidationError> {
    let mut cmd = Command::new("node");
    cmd.arg("-e")
        .arg(
            "console.log = (...args) => console.error(...args);\
             console.info = (...args) => console.error(...args);\
             console.debug = (...args) => console.error(...args);\
             const m = await import('./wavical.config.js');\
             const c = m ? m.default : undefined;\
             process.stdout.write('__WAVICAL_CONFIG_JSON_START__\\n' + JSON.stringify(c) + '\\n__WAVICAL_CONFIG_JSON_END__\\n');",
        )
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| ValidationError::NodeError(format!("Failed to spawn Node process: {e}")))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let child_arc = Arc::new(Mutex::new(child));
    let child_arc_worker = Arc::clone(&child_arc);

    let (tx, rx) = mpsc::channel();

    let reader_handle = std::thread::spawn(move || {
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();

        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut stdout_buf);
        }
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut stderr_buf);
        }

        let status = {
            if let Ok(mut guard) = child_arc_worker.lock() {
                guard.wait()
            } else {
                return;
            }
        };

        if let Ok(status) = status {
            let _ = tx.send(NodeExecutionOutput {
                status,
                stdout: stdout_buf,
                stderr: stderr_buf,
            });
        }
    });

    match rx.recv_timeout(timeout) {
        Ok(output) => {
            let _ = reader_handle.join();
            Ok(output)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if let Ok(mut guard) = child_arc.lock() {
                let _ = guard.kill();
            }
            let _ = reader_handle.join();
            Err(ValidationError::NodeTimeout)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = reader_handle.join();
            Err(ValidationError::NodeError(
                "Node process worker disconnected unexpectedly".to_string(),
            ))
        }
    }
}

/// Resolve the timeout for Node config execution, respecting the `WAVICAL_NODE_TIMEOUT` env var (in seconds) if set.
pub fn get_node_timeout() -> Duration {
    if let Ok(val) = std::env::var("WAVICAL_NODE_TIMEOUT")
        && let Ok(secs) = val.parse::<u64>()
    {
        return Duration::from_secs(secs);
    }
    CONFIG_NODE_TIMEOUT
}

/// Read, evaluate, and validate `wavical.config.js` in the given project root.
///
/// Shells out to Node with a timeout (default 5s, configurable via WAVICAL_NODE_TIMEOUT),
/// converts the export to JSON, and validates the parsed structure and capabilities.
pub fn validate_config(project_root: &Path) -> Result<AppConfig, ValidationError> {
    let config_path = project_root.join("wavical.config.js");
    if !config_path.exists() {
        return Err(ValidationError::ConfigNotFound);
    }

    let timeout = get_node_timeout();
    let output = execute_node_config_script(project_root, timeout)?;

    if !output.status.success() {
        let stderr_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ValidationError::NodeError(if stderr_str.is_empty() {
            format!("Node process exited with code {:?}", output.status.code())
        } else {
            stderr_str
        }));
    }

    let stdout_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_config_json(&stdout_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_valid_minimal_config() {
        let json = r#"{"appId": "my-app"}"#;
        let config = parse_config_json(json).expect("should parse minimal config");
        assert_eq!(config.app_id, "my-app");
        assert_eq!(config.app_id(), "my-app");
        assert_eq!(config.appId(), "my-app");
        assert_eq!(config.window_width(), DEFAULT_WINDOW_WIDTH);
        assert_eq!(config.window_height(), DEFAULT_WINDOW_HEIGHT);
        assert_eq!(config.window_title(), "my-app");
        assert_eq!(config.dev_port(), DEFAULT_DEV_PORT);
        assert!(config.granted_capabilities().is_empty());
    }

    #[test]
    fn test_valid_full_config() {
        let json = r#"{
            "appId": "photo-editor",
            "window": {
                "width": 1400,
                "height": 900,
                "title": "Photo Editor Pro"
            },
            "capabilities": {
                "core.ping": true,
                "core.echo": false
            },
            "dev": {
                "port": 8080
            }
        }"#;

        let config = parse_config_json(json).expect("should parse full config");
        assert_eq!(config.app_id, "photo-editor");
        assert_eq!(config.window_width(), 1400);
        assert_eq!(config.window_height(), 900);
        assert_eq!(config.window_title(), "Photo Editor Pro");
        assert_eq!(config.dev_port(), 8080);
        assert!(config.is_capability_granted("core.ping"));
        assert!(!config.is_capability_granted("core.echo"));
        assert_eq!(config.granted_capabilities(), vec!["core.ping".to_string()]);
    }

    #[test]
    fn test_version_field_parsing_and_default() {
        let minimal = r#"{"appId": "test"}"#;
        let cfg1 = parse_config_json(minimal).unwrap();
        assert_eq!(cfg1.version, None);
        assert_eq!(cfg1.version_or_default(), "0.1.0");

        let with_ver = r#"{"appId": "test", "version": "2.1.0"}"#;
        let cfg2 = parse_config_json(with_ver).unwrap();
        assert_eq!(cfg2.version.as_deref(), Some("2.1.0"));
        assert_eq!(cfg2.version_or_default(), "2.1.0");
    }

    #[test]
    fn test_missing_app_id() {
        let json = r#"{"window": {"width": 800}}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::MissingField(field) => assert_eq!(field, "appId"),
            other => panic!("Expected MissingField('appId'), got: {other:?}"),
        }
    }

    #[test]
    fn test_empty_app_id() {
        let json = r#"{"appId": "   "}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::MissingField(field) => assert_eq!(field, "appId"),
            other => panic!("Expected MissingField('appId'), got: {other:?}"),
        }
    }

    #[test]
    fn test_invalid_app_id_characters() {
        let json = r#"{"appId": "My App!"}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::InvalidAppId(id) => assert_eq!(id, "My App!"),
            other => panic!("Expected InvalidAppId, got: {other:?}"),
        }
    }

    #[test]
    fn test_unknown_capability_suggestion_pong_to_ping() {
        let json = r#"{"appId": "test", "capabilities": {"core.pong": true}}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::UnknownCapability(ref cap, ref known) => {
                assert_eq!(cap, "core.pong");
                assert!(known.contains(&"core.ping".to_string()));
                assert_eq!(err.suggestion(), Some("core.ping"));
                let msg = err.to_string();
                assert!(msg.contains("unknown capability 'core.pong'"));
                assert!(msg.contains("did you mean 'core.ping'?"));
            }
            other => panic!("Expected UnknownCapability, got: {other:?}"),
        }
    }

    #[test]
    fn test_unknown_capability_suggestion_echo2_to_echo() {
        let json = r#"{"appId": "test", "capabilities": {"core.echo2": true}}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::UnknownCapability(ref cap, ref known) => {
                assert_eq!(cap, "core.echo2");
                assert!(known.contains(&"core.echo".to_string()));
                assert_eq!(err.suggestion(), Some("core.echo"));
                let msg = err.to_string();
                assert!(msg.contains("unknown capability 'core.echo2'"));
                assert!(msg.contains("did you mean 'core.echo'?"));
            }
            other => panic!("Expected UnknownCapability, got: {other:?}"),
        }
    }

    #[test]
    fn test_unknown_capability_without_close_match() {
        let json = r#"{"appId": "test", "capabilities": {"nonexistent.capability.xyz": true}}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::UnknownCapability(ref cap, _) => {
                assert_eq!(cap, "nonexistent.capability.xyz");
                assert_eq!(err.suggestion(), None);
                let msg = err.to_string();
                assert!(msg.contains("unknown capability 'nonexistent.capability.xyz'"));
                assert!(msg.contains("known: ai.chat, ai.index"));
                assert!(msg.contains("core.ping"));
                assert!(msg.contains("tray.set"));
            }
            other => panic!("Expected UnknownCapability, got: {other:?}"),
        }
    }

    #[test]
    fn test_phase2_all_native_capabilities_accepted() {
        let json = r#"{
            "appId": "test-native-caps",
            "capabilities": {
                "core.ping": true,
                "core.echo": true,
                "badge.set": true,
                "clipboard.read": true,
                "clipboard.write": true,
                "dialog.openFile": true,
                "dialog.saveFile": true,
                "fs.readGranted": true,
                "fs.writeGranted": true,
                "menu.showContextMenu": true,
                "notify.send": true,
                "shortcut.register": true,
                "tray.set": true
            }
        }"#;

        let config = parse_config_json(json).expect("all Phase 2 capabilities must be accepted");
        assert_eq!(config.app_id, "test-native-caps");

        let expected_caps = [
            "badge.set",
            "clipboard.read",
            "clipboard.write",
            "core.echo",
            "core.ping",
            "dialog.openFile",
            "dialog.saveFile",
            "fs.readGranted",
            "fs.writeGranted",
            "menu.showContextMenu",
            "notify.send",
            "shortcut.register",
            "tray.set",
        ];

        for cap in expected_caps {
            assert!(
                config.is_capability_granted(cap),
                "Capability '{cap}' should be granted"
            );
        }
    }

    #[test]
    fn test_phase2_unknown_capability_typo_suggestions() {
        let test_cases = [
            ("clipboard.red", "clipboard.read"),
            ("clipboard.writte", "clipboard.write"),
            ("notify.snd", "notify.send"),
            ("dialog.openFil", "dialog.openFile"),
            ("dialog.saveFil", "dialog.saveFile"),
            ("fs.readGrantd", "fs.readGranted"),
            ("fs.writeGrantd", "fs.writeGranted"),
            ("menu.showContextMen", "menu.showContextMenu"),
            ("tray.sett", "tray.set"),
            ("shortcut.registar", "shortcut.register"),
            ("badge.sett", "badge.set"),
        ];

        for (typo, expected_suggestion) in test_cases {
            let json = format!(r#"{{"appId": "test", "capabilities": {{"{typo}": true}}}}"#);
            let err =
                parse_config_json(&json).expect_err(&format!("typo '{typo}' should be rejected"));
            match err {
                ValidationError::UnknownCapability(ref cap, _) => {
                    assert_eq!(cap, typo);
                    assert_eq!(
                        err.suggestion(),
                        Some(expected_suggestion),
                        "Expected suggestion '{expected_suggestion}' for typo '{typo}'"
                    );
                    assert!(
                        err.to_string()
                            .contains(&format!("did you mean '{expected_suggestion}'?")),
                        "Error string should contain suggestion for '{typo}'"
                    );
                }
                other => panic!("Expected UnknownCapability for '{typo}', got: {other:?}"),
            }
        }
    }

    #[test]
    fn test_deny_unknown_fields_app_config() {
        let json = r#"{"appId": "test", "windowTitle": "invalid top level field"}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::MalformedJson(msg) => {
                assert!(
                    msg.contains("unknown field `windowTitle`")
                        || msg.contains("unknown field 'windowTitle'")
                );
            }
            other => panic!("Expected MalformedJson due to unknown field, got: {other:?}"),
        }
    }

    #[test]
    fn test_deny_unknown_fields_window_config() {
        let json = r#"{"appId": "test", "window": {"extra": 123}}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::MalformedJson(msg) => {
                assert!(
                    msg.contains("unknown field `extra`") || msg.contains("unknown field 'extra'")
                );
            }
            other => panic!("Expected MalformedJson due to unknown window field, got: {other:?}"),
        }
    }

    #[test]
    fn test_deny_unknown_fields_dev_config() {
        let json = r#"{"appId": "test", "dev": {"host": "localhost"}}"#;
        let err = parse_config_json(json).unwrap_err();
        match err {
            ValidationError::MalformedJson(msg) => {
                assert!(
                    msg.contains("unknown field `host`") || msg.contains("unknown field 'host'")
                );
            }
            other => panic!("Expected MalformedJson due to unknown dev field, got: {other:?}"),
        }
    }

    #[test]
    fn test_levenshtein_distance_cases() {
        assert_eq!(levenshtein_distance("core.ping", "core.pong"), 1);
        assert_eq!(levenshtein_distance("core.echo", "core.echo2"), 1);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("", "test"), 4);
        assert_eq!(levenshtein_distance("test", "test"), 0);
    }

    #[test]
    fn test_integration_valid_config_js() {
        let dir = tempdir().expect("tempdir creation should succeed");
        let config_file = dir.path().join("wavical.config.js");
        let mut file = std::fs::File::create(&config_file).expect("file create should succeed");
        file.write_all(
            b"export default {\n  appId: 'integration-app',\n  window: {\n    width: 1024,\n    height: 768,\n    title: 'Integration Window'\n  },\n  capabilities: {\n    'core.ping': true\n  },\n  dev: {\n    port: 9000\n  }\n};\n"
        )
        .expect("write should succeed");

        let config = validate_config(dir.path()).expect("validate_config should succeed");
        assert_eq!(config.app_id, "integration-app");
        assert_eq!(config.window_width(), 1024);
        assert_eq!(config.window_height(), 768);
        assert_eq!(config.window_title(), "Integration Window");
        assert_eq!(config.dev_port(), 9000);
        assert!(config.is_capability_granted("core.ping"));
    }

    #[test]
    fn test_integration_typo_capability_suggests_ping() {
        let dir = tempdir().expect("tempdir creation should succeed");
        let config_file = dir.path().join("wavical.config.js");
        let mut file = std::fs::File::create(&config_file).expect("file create should succeed");
        file.write_all(
            b"export default {\n  appId: 'typo-app',\n  capabilities: {\n    'core.pong': true\n  }\n};\n"
        )
        .expect("write should succeed");

        let err = validate_config(dir.path()).expect_err("should reject typo capability");
        match err {
            ValidationError::UnknownCapability(ref cap, _) => {
                assert_eq!(cap, "core.pong");
                assert_eq!(err.suggestion(), Some("core.ping"));
                assert!(err.to_string().contains("did you mean 'core.ping'?"));
            }
            other => panic!("Expected UnknownCapability, got: {other:?}"),
        }
    }

    #[test]
    fn test_integration_unknown_capability_rejects_echo2() {
        let dir = tempdir().expect("tempdir creation should succeed");
        let config_file = dir.path().join("wavical.config.js");
        let mut file = std::fs::File::create(&config_file).expect("file create should succeed");
        file.write_all(
            b"export default {\n  appId: 'echo2-app',\n  capabilities: {\n    'core.echo2': true\n  }\n};\n"
        )
        .expect("write should succeed");

        let err = validate_config(dir.path()).expect_err("should reject core.echo2");
        match err {
            ValidationError::UnknownCapability(ref cap, _) => {
                assert_eq!(cap, "core.echo2");
                assert_eq!(err.suggestion(), Some("core.echo"));
                assert!(err.to_string().contains("unknown capability 'core.echo2'"));
            }
            other => panic!("Expected UnknownCapability, got: {other:?}"),
        }
    }

    #[test]
    fn test_integration_config_not_found() {
        let dir = tempdir().expect("tempdir creation should succeed");
        let err = validate_config(dir.path()).expect_err("should fail when config missing");
        match err {
            ValidationError::ConfigNotFound => {}
            other => panic!("Expected ConfigNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_integration_syntax_error_in_config() {
        let dir = tempdir().expect("tempdir creation should succeed");
        let config_file = dir.path().join("wavical.config.js");
        let mut file = std::fs::File::create(&config_file).expect("file create should succeed");
        file.write_all(b"export default {{{ broken javascript\n")
            .expect("write should succeed");

        let err = validate_config(dir.path()).expect_err("should reject syntax error");
        match err {
            ValidationError::NodeError(msg) => {
                assert!(msg.contains("SyntaxError") || msg.contains("error"));
            }
            other => panic!("Expected NodeError, got: {other:?}"),
        }
    }

    #[test]
    fn test_integration_node_timeout() {
        let dir = tempdir().expect("tempdir creation should succeed");
        let config_file = dir.path().join("wavical.config.js");
        let mut file = std::fs::File::create(&config_file).expect("file create should succeed");
        file.write_all(b"while(true) {}\nexport default { appId: 'loop' };\n")
            .expect("write should succeed");

        let err = execute_node_config_script(dir.path(), Duration::from_millis(250))
            .expect_err("should timeout");
        match err {
            ValidationError::NodeTimeout => {}
            other => panic!("Expected NodeTimeout, got: {other:?}"),
        }
    }

    // Property-based test suite for AppConfig roundtrip
    fn arb_app_config() -> impl Strategy<Value = AppConfig> {
        let app_id_strat = "[a-z0-9_-]{1,30}";
        let window_strat = proptest::option::of(
            (
                proptest::option::of(100u32..4000u32),
                proptest::option::of(100u32..4000u32),
                proptest::option::of("[a-zA-Z0-9 _-]{1,50}"),
                proptest::option::of(proptest::bool::ANY),
            )
                .prop_map(|(w, h, t, p)| WindowConfig {
                    width: w,
                    height: h,
                    title: t,
                    persist_state: p,
                }),
        );

        let caps_strat = proptest::option::of(proptest::collection::hash_map(
            proptest::sample::select(vec!["core.ping".to_string(), "core.echo".to_string()]),
            proptest::bool::ANY,
            0..=2,
        ));

        let dev_strat =
            proptest::option::of((1024u16..65535u16).prop_map(|port| DevConfig { port }));
        let version_strat = proptest::option::of("[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}");
        let single_instance_strat = proptest::option::of(proptest::bool::ANY);

        (
            app_id_strat,
            version_strat,
            window_strat,
            caps_strat,
            dev_strat,
            single_instance_strat,
        )
            .prop_map(
                |(app_id, version, window, capabilities, dev, single_instance)| AppConfig {
                    app_id,
                    version,
                    window,
                    capabilities,
                    dev,
                    dialog: None,
                    single_instance,
                    tray: None,
                    shortcuts: None,
                    file_associations: None,
                    ai: None,
                },
            )
    }

    #[test]
    fn test_tray_config_parsing_and_deny_unknown_fields() {
        let json = r#"{
            "appId": "test-tray",
            "tray": {
                "icon": "assets/tray.ico"
            }
        }"#;
        let config = parse_config_json(json).expect("valid tray config should parse");
        assert_eq!(config.tray_icon(), Some("assets/tray.ico"));

        // Unknown field inside tray rejected
        let bad_json = r#"{
            "appId": "test-tray",
            "tray": {
                "icon": "assets/tray.ico",
                "unknownField": 123
            }
        }"#;
        let err = parse_config_json(bad_json).expect_err("unknown field in tray must be rejected");
        match err {
            ValidationError::MalformedJson(msg) => {
                assert!(msg.contains("unknown field `unknownField`"));
            }
            other => panic!("Unexpected error: {other:?}"),
        }
    }

    #[test]
    fn test_file_associations_and_shortcuts_config_validation() {
        let valid_json = r#"{
            "appId": "test-assoc",
            "fileAssociations": ["wav", "demo123"],
            "shortcuts": ["Ctrl+Shift+K", "Alt+F4"]
        }"#;
        let config = parse_config_json(valid_json).expect("valid config should parse");
        assert_eq!(
            config.file_associations.as_deref(),
            Some(&["wav".to_string(), "demo123".to_string()][..])
        );
        assert_eq!(
            config.shortcuts.as_deref(),
            Some(&["Ctrl+Shift+K".to_string(), "Alt+F4".to_string()][..])
        );

        // Invalid extension (contains dot)
        let bad_ext = r#"{
            "appId": "test-assoc",
            "fileAssociations": [".wav"]
        }"#;
        assert!(matches!(
            parse_config_json(bad_ext),
            Err(ValidationError::InvalidFileAssociation(_))
        ));

        // Invalid extension (too long)
        let bad_len = r#"{
            "appId": "test-assoc",
            "fileAssociations": ["toolongext11"]
        }"#;
        assert!(matches!(
            parse_config_json(bad_len),
            Err(ValidationError::InvalidFileAssociation(_))
        ));

        // Invalid shortcut (duplicate)
        let dup_shortcut = r#"{
            "appId": "test-assoc",
            "shortcuts": ["Ctrl+K", "Ctrl+K"]
        }"#;
        assert!(matches!(
            parse_config_json(dup_shortcut),
            Err(ValidationError::InvalidShortcut(_))
        ));
    }

    #[test]
    fn test_ai_config_defaults() {
        let json = r#"{
            "appId": "test-ai",
            "ai": {}
        }"#;
        let config = parse_config_json(json).expect("valid minimal AI config should parse");
        let ai = config.ai.expect("ai section should be present");
        assert!(!ai.enabled);
        assert_eq!(ai.mode, AiMode::Local);
        assert!(ai.local.is_none());
        assert!(ai.cloud.is_none());
    }

    #[test]
    fn test_ai_config_deny_unknown_fields() {
        // 1. Unknown key in ai top-level
        let bad_ai = r#"{
            "appId": "test-ai",
            "ai": {
                "unknownTopKey": 123
            }
        }"#;
        let err = parse_config_json(bad_ai).unwrap_err();
        assert!(err.to_string().contains("unknown field `unknownTopKey`"));

        // 2. Unknown key in ai.local
        let bad_local = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "unknownLocalKey": "value"
                }
            }
        }"#;
        let err = parse_config_json(bad_local).unwrap_err();
        assert!(err.to_string().contains("unknown field `unknownLocalKey`"));

        // 3. Unknown key in ai.cloud
        let bad_cloud = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "OPENAI_API_KEY",
                    "unknownCloudKey": true
                }
            }
        }"#;
        let err = parse_config_json(bad_cloud).unwrap_err();
        assert!(err.to_string().contains("unknown field `unknownCloudKey`"));
    }

    #[test]
    fn test_ai_mode_parsing() {
        for (mode_str, expected_mode) in [
            ("local", AiMode::Local),
            ("cloud", AiMode::Cloud),
            ("local-first", AiMode::LocalFirst),
        ] {
            let json = format!(r#"{{"appId": "test-ai", "ai": {{"mode": "{mode_str}"}}}}"#);
            let config = parse_config_json(&json).expect("valid mode should parse");
            assert_eq!(config.ai.unwrap().mode, expected_mode);
        }

        let bad_mode = r#"{"appId": "test-ai", "ai": {"mode": "unsupported-mode"}}"#;
        assert!(parse_config_json(bad_mode).is_err());
    }

    #[test]
    fn test_ai_base_url_rules() {
        // Insecure http rejected
        let http_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "http://api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "OPENAI_API_KEY"
                }
            }
        }"#;
        let err = parse_config_json(http_json).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("in 'ai.cloud.baseUrl'"));
        assert!(msg.contains("cloud endpoints strictly require https://"));

        // Missing host rejected
        let no_host_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "OPENAI_API_KEY"
                }
            }
        }"#;
        let err = parse_config_json(no_host_json).unwrap_err();
        assert!(err.to_string().contains("missing host in 'https://'"));

        // Userinfo rejected
        let userinfo_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://user:pass@api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "OPENAI_API_KEY"
                }
            }
        }"#;
        let err = parse_config_json(userinfo_json).unwrap_err();
        assert!(
            err.to_string()
                .contains("userinfo credentials are not permitted")
        );

        // Valid baseUrl accepted
        let valid_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "OPENAI_API_KEY"
                }
            }
        }"#;
        assert!(parse_config_json(valid_json).is_ok());
    }

    #[test]
    fn test_ai_api_key_env_rules() {
        // Value starting with sk- rejected as an actual key
        let sk_key_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "sk-proj-1234567890abcdef"
                }
            }
        }"#;
        let err = parse_config_json(sk_key_json).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("in 'ai.cloud.apiKeyEnv'"));
        assert!(msg.contains("looks like an actual API key"));

        // Lowercase variable name rejected
        let lower_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "openai_api_key"
                }
            }
        }"#;
        let err = parse_config_json(lower_json).unwrap_err();
        assert!(
            err.to_string()
                .contains("must match pattern ^[A-Z][A-Z0-9_]*$")
        );

        // Punctuation dash rejected
        let dash_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "OPENAI-API-KEY"
                }
            }
        }"#;
        let err = parse_config_json(dash_json).unwrap_err();
        assert!(
            err.to_string()
                .contains("must match pattern ^[A-Z][A-Z0-9_]*$")
        );

        // Leading digit rejected
        let digit_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "1OPENAI_KEY"
                }
            }
        }"#;
        let err = parse_config_json(digit_json).unwrap_err();
        assert!(
            err.to_string()
                .contains("must match pattern ^[A-Z][A-Z0-9_]*$")
        );

        // Valid uppercase variable name accepted
        let valid_json = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "cloud": {
                    "protocol": "openai-compatible",
                    "baseUrl": "https://api.openai.com/v1",
                    "model": "gpt-4o-mini",
                    "apiKeyEnv": "MY_PROVIDER_KEY_V2"
                }
            }
        }"#;
        assert!(parse_config_json(valid_json).is_ok());
    }

    #[test]
    fn test_ai_local_bounds_and_catalog_hook() {
        // contextSize ceiling 8192
        let over_ctx = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "contextSize": 8193
                }
            }
        }"#;
        let err = parse_config_json(over_ctx).unwrap_err();
        assert!(err.to_string().contains("exceeds hard ceiling of 8192"));

        let zero_ctx = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "contextSize": 0
                }
            }
        }"#;
        assert!(parse_config_json(zero_ctx).is_err());

        // maxTokens ceiling 4096
        let over_tokens = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "maxTokens": 4097
                }
            }
        }"#;
        let err = parse_config_json(over_tokens).unwrap_err();
        assert!(err.to_string().contains("exceeds hard ceiling of 4096"));

        // idleMinutes 0 rejected
        let zero_idle = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "idleMinutes": 0
                }
            }
        }"#;
        let err = parse_config_json(zero_idle).unwrap_err();
        assert!(
            err.to_string()
                .contains("idleMinutes must be greater than 0")
        );

        // Catalog model id format hook
        let bad_model_id = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "chatModel": "Qwen/2.5-1.5B"
                }
            }
        }"#;
        let err = parse_config_json(bad_model_id).unwrap_err();
        assert!(
            err.to_string()
                .contains("must match pattern [a-z0-9._-]{1,64}")
        );

        // Valid model id accepted
        let valid_local = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "chatModel": "qwen2.5-1.5b-instruct",
                    "embedModel": "bge-small-en-v1.5",
                    "contextSize": 4096,
                    "maxTokens": 1024,
                    "idleMinutes": 10,
                    "threads": "auto"
                }
            }
        }"#;
        assert!(parse_config_json(valid_local).is_ok());

        // Numeric threads accepted
        let threads_num = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "threads": 8
                }
            }
        }"#;
        assert!(parse_config_json(threads_num).is_ok());

        // Thread count > 128 rejected
        let threads_over = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true,
                "local": {
                    "threads": 129
                }
            }
        }"#;
        assert!(parse_config_json(threads_over).is_err());
    }

    #[test]
    fn test_ai_capabilities_require_ai_enabled() {
        for cap in ["ai.chat", "ai.index", "ai.search"] {
            // 1. Without ai section
            let json_no_ai =
                format!(r#"{{"appId": "test-ai", "capabilities": {{"{cap}": true}}}}"#);
            let err = parse_config_json(&json_no_ai).unwrap_err();
            match err {
                ValidationError::AiCapabilityRequiresAiEnabled(ref c) => {
                    assert_eq!(c, cap);
                    let msg = err.to_string();
                    assert!(msg.contains(&format!("in 'capabilities.{cap}'")));
                    assert!(
                        msg.contains(&format!("capability '{cap}' requires 'ai.enabled: true'"))
                    );
                    assert!(msg.contains("set 'ai: { enabled: true }' in wavical.config.js"));
                }
                other => {
                    panic!("Expected AiCapabilityRequiresAiEnabled for '{cap}', got: {other:?}")
                }
            }

            // 2. With ai.enabled = false
            let json_disabled_ai = format!(
                r#"{{"appId": "test-ai", "ai": {{"enabled": false}}, "capabilities": {{"{cap}": true}}}}"#
            );
            let err = parse_config_json(&json_disabled_ai).unwrap_err();
            assert!(matches!(
                err,
                ValidationError::AiCapabilityRequiresAiEnabled(_)
            ));
        }

        // 3. With ai.enabled = true, all three AI capabilities are accepted
        let json_enabled_ai = r#"{
            "appId": "test-ai",
            "ai": {
                "enabled": true
            },
            "capabilities": {
                "ai.chat": true,
                "ai.index": true,
                "ai.search": true
            }
        }"#;
        let config =
            parse_config_json(json_enabled_ai).expect("all AI capabilities valid when enabled");
        assert!(config.is_capability_granted("ai.chat"));
        assert!(config.is_capability_granted("ai.index"));
        assert!(config.is_capability_granted("ai.search"));
    }

    #[test]
    fn test_config_stdout_pollution_isolation() {
        let dir = tempdir().expect("tempdir");
        let config_file = dir.path().join("wavical.config.js");
        let mut file = std::fs::File::create(&config_file).expect("file create");
        file.write_all(
            b"console.log('diagnostic log 1');\nconsole.info('diagnostic info');\nexport default { appId: 'isolated-app' };\n",
        )
        .expect("write");

        let conf = validate_config(dir.path()).expect("validate_config should isolate stdout");
        assert_eq!(conf.app_id, "isolated-app");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(500))]

        #[test]
        fn proptest_random_app_config_roundtrips_cleanly(cfg in arb_app_config()) {
            let json = serde_json::to_string(&cfg).expect("Serialization should not fail");
            let deserialized: AppConfig = parse_config_json(&json).expect("Deserialization should succeed");

            prop_assert_eq!(&deserialized.app_id, &cfg.app_id);
            prop_assert_eq!(&deserialized.version, &cfg.version);
            prop_assert_eq!(&deserialized.window, &cfg.window);
            prop_assert_eq!(&deserialized.capabilities, &cfg.capabilities);
            prop_assert_eq!(&deserialized.dev, &cfg.dev);
            prop_assert_eq!(&deserialized.single_instance, &cfg.single_instance);
        }
    }
}
