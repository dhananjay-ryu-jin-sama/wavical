#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;
use thiserror::Error;

/// Hard ceiling for a single model file size: 50 GB.
pub const MAX_MODEL_SIZE_BYTES: u64 = 50 * 1024 * 1024 * 1024;

/// Permitted download host patterns for model downloads.
pub const ALLOWED_DOWNLOAD_HOSTS: &[&str] = &[
    "huggingface.co",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

/// Check whether a host is on the strict allowlist.
/// Supports exact match and `*.hf.co` subdomain matching.
pub fn is_host_allowed(host: &str) -> bool {
    let lower_host = host.to_ascii_lowercase();

    for &allowed in ALLOWED_DOWNLOAD_HOSTS {
        if lower_host == allowed {
            return true;
        }
    }

    if lower_host.ends_with(".hf.co") {
        let prefix = &lower_host[..lower_host.len() - ".hf.co".len()];
        if !prefix.is_empty() && !prefix.contains('/') && !prefix.contains('\\') {
            return true;
        }
    }

    false
}

/// Allowed model kind in catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Chat,
    Embedding,
}

impl std::fmt::Display for ModelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelKind::Chat => write!(f, "chat"),
            ModelKind::Embedding => write!(f, "embedding"),
        }
    }
}

/// A validated entry in the Wavical Model Catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCatalogEntry {
    /// Catalog identifier matching `^[a-z0-9._-]{1,64}$`.
    pub id: String,

    /// Human-readable display name.
    pub name: String,

    /// Primary model task (`chat` or `embedding`).
    pub kind: ModelKind,

    /// GGUF file name matching `^[a-z0-9._-]{1,64}$` ending in `.gguf`.
    pub file_name: String,

    /// Upstream HTTPS download URL on the allowlisted hosts.
    pub url: String,

    /// Exact size in bytes.
    pub size_bytes: u64,

    /// SHA-256 digest (64 lowercase hex characters).
    pub sha256: String,

    /// License identifier name (e.g. 'Apache-2.0', 'MIT').
    pub license_name: String,

    /// HTTPS link to the license terms.
    pub license_url: String,

    /// Minimum recommended system RAM in bytes.
    pub min_ram_bytes: u64,

    /// Optional notes or description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Complete Model Catalog definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCatalog {
    pub version: u32,
    pub entries: Vec<ModelCatalogEntry>,
}

impl ModelCatalog {
    /// Find a catalog entry by exact model ID.
    pub fn find_by_id(&self, id: &str) -> Option<&ModelCatalogEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Check if a model ID exists in this catalog.
    pub fn contains_id(&self, id: &str) -> bool {
        self.find_by_id(id).is_some()
    }
}

/// Catalog validation error details.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CatalogError {
    #[error("failed to parse catalog JSON: {0}")]
    JsonError(String),

    #[error("invalid model id '{0}': must match pattern [a-z0-9._-]{{1,64}}")]
    InvalidModelId(String),

    #[error("duplicate model id '{0}' found in catalog")]
    DuplicateModelId(String),

    #[error("model '{id}' has empty display name")]
    EmptyModelName { id: String },

    #[error(
        "invalid file name '{name}' for model '{id}': must match [a-z0-9._-]{{1,64}} and end with .gguf"
    )]
    InvalidFileName { id: String, name: String },

    #[error("duplicate file name '{0}' found in catalog")]
    DuplicateFileName(String),

    #[error("insecure or invalid URL '{url}' for model '{id}': must start with https://")]
    InsecureUrl { id: String, url: String },

    #[error("host '{host}' in URL for model '{id}' is not in the approved download allowlist")]
    DisallowedHost { id: String, host: String },

    #[error("URL '{url}' for model '{id}' contains forbidden userinfo credentials")]
    UserInfoInUrl { id: String, url: String },

    #[error("invalid size {size} bytes for model '{id}': must be > 0 and <= {max} bytes")]
    InvalidSize { id: String, size: u64, max: u64 },

    #[error(
        "invalid SHA-256 digest '{hash}' for model '{id}': must be exactly 64 lowercase hex characters"
    )]
    InvalidSha256 { id: String, hash: String },

    #[error("model '{id}' has empty license name")]
    EmptyLicenseName { id: String },

    #[error("insecure license URL '{url}' for model '{id}': must start with https://")]
    InsecureLicenseUrl { id: String, url: String },

    #[error("invalid minimum RAM {ram} bytes for model '{id}': must be > 0")]
    InvalidMinRam { id: String, ram: u64 },
}

/// Validate that an identifier string matches `^[a-z0-9._-]{1,64}$`.
pub fn validate_safe_id(id: &str) -> bool {
    let char_count = id.chars().count();
    if !(1..=64).contains(&char_count) {
        return false;
    }
    for c in id.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-';
        if !ok {
            return false;
        }
    }
    true
}

/// Validate that a GGUF file name is safe and contains no path traversal.
pub fn validate_safe_filename(name: &str) -> bool {
    if !validate_safe_id(name) {
        return false;
    }
    if !name.ends_with(".gguf") {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains(':') {
        return false;
    }
    true
}

/// Validate that a SHA-256 digest is exactly 64 lowercase hex characters.
pub fn validate_sha256_format(hash: &str) -> bool {
    if hash.len() != 64 {
        return false;
    }
    hash.chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Extract and validate host from an HTTPS URL.
pub fn extract_https_host(url: &str) -> Result<String, &'static str> {
    if !url.starts_with("https://") {
        return Err("URL must start with https://");
    }

    let after_scheme = &url["https://".len()..];
    let authority = after_scheme.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err("URL is missing host authority");
    }

    if authority.contains('@') {
        return Err("URL cannot contain userinfo credentials");
    }

    let host = authority.split(':').next().unwrap_or("");
    if host.is_empty() || host.contains(' ') || host.starts_with('.') || host.ends_with('.') {
        return Err("Invalid host in URL");
    }

    Ok(host.to_string())
}

/// Validate an entire model catalog.
pub fn validate_catalog(catalog: &ModelCatalog) -> Result<(), CatalogError> {
    let mut seen_ids = HashSet::new();
    let mut seen_filenames = HashSet::new();

    for entry in &catalog.entries {
        // ID validation
        if !validate_safe_id(&entry.id) {
            return Err(CatalogError::InvalidModelId(entry.id.clone()));
        }
        if !seen_ids.insert(entry.id.clone()) {
            return Err(CatalogError::DuplicateModelId(entry.id.clone()));
        }

        // Display name
        if entry.name.trim().is_empty() {
            return Err(CatalogError::EmptyModelName {
                id: entry.id.clone(),
            });
        }

        // File name validation
        if !validate_safe_filename(&entry.file_name) {
            return Err(CatalogError::InvalidFileName {
                id: entry.id.clone(),
                name: entry.file_name.clone(),
            });
        }
        if !seen_filenames.insert(entry.file_name.clone()) {
            return Err(CatalogError::DuplicateFileName(entry.file_name.clone()));
        }

        // URL validation
        let host = extract_https_host(&entry.url).map_err(|err| {
            if err.contains("userinfo") {
                CatalogError::UserInfoInUrl {
                    id: entry.id.clone(),
                    url: entry.url.clone(),
                }
            } else {
                CatalogError::InsecureUrl {
                    id: entry.id.clone(),
                    url: entry.url.clone(),
                }
            }
        })?;

        if !is_host_allowed(&host) {
            return Err(CatalogError::DisallowedHost {
                id: entry.id.clone(),
                host,
            });
        }

        // Size bounds
        if entry.size_bytes == 0 || entry.size_bytes > MAX_MODEL_SIZE_BYTES {
            return Err(CatalogError::InvalidSize {
                id: entry.id.clone(),
                size: entry.size_bytes,
                max: MAX_MODEL_SIZE_BYTES,
            });
        }

        // SHA-256 format
        if !validate_sha256_format(&entry.sha256) {
            return Err(CatalogError::InvalidSha256 {
                id: entry.id.clone(),
                hash: entry.sha256.clone(),
            });
        }

        // License
        if entry.license_name.trim().is_empty() {
            return Err(CatalogError::EmptyLicenseName {
                id: entry.id.clone(),
            });
        }
        if !entry.license_url.starts_with("https://") {
            return Err(CatalogError::InsecureLicenseUrl {
                id: entry.id.clone(),
                url: entry.license_url.clone(),
            });
        }

        // Min RAM
        if entry.min_ram_bytes == 0 {
            return Err(CatalogError::InvalidMinRam {
                id: entry.id.clone(),
                ram: entry.min_ram_bytes,
            });
        }
    }

    Ok(())
}

/// Parse and validate catalog JSON.
pub fn parse_catalog_json(json_str: &str) -> Result<ModelCatalog, CatalogError> {
    let catalog: ModelCatalog =
        serde_json::from_str(json_str).map_err(|e| CatalogError::JsonError(e.to_string()))?;
    validate_catalog(&catalog)?;
    Ok(catalog)
}

/// Raw embedded catalog JSON string.
pub const EMBEDDED_CATALOG_JSON: &str = include_str!("../../../models.json");

static GLOBAL_DEFAULT_CATALOG: OnceLock<ModelCatalog> = OnceLock::new();

/// Return a reference to the validated default model catalog embedded in Wavical.
pub fn default_catalog() -> &'static ModelCatalog {
    GLOBAL_DEFAULT_CATALOG.get_or_init(|| {
        parse_catalog_json(EMBEDDED_CATALOG_JSON)
            .expect("embedded models.json must be a valid, verified catalog")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_catalog_is_valid() {
        let cat = default_catalog();
        assert_eq!(cat.version, 1);
        assert_eq!(cat.entries.len(), 2);

        let qwen = cat
            .find_by_id("qwen2.5-1.5b-instruct")
            .expect("qwen must exist");
        assert_eq!(qwen.kind, ModelKind::Chat);
        assert_eq!(qwen.file_name, "qwen2.5-1.5b-instruct-q4_k_m.gguf");
        assert_eq!(qwen.size_bytes, 1117320736);
        assert_eq!(
            qwen.sha256,
            "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e"
        );
        assert_eq!(qwen.license_name, "Apache-2.0");

        let bge = cat.find_by_id("bge-small-en-v1.5").expect("bge must exist");
        assert_eq!(bge.kind, ModelKind::Embedding);
        assert_eq!(bge.file_name, "bge-small-en-v1.5-q4_k_m.gguf");
        assert_eq!(bge.size_bytes, 24808576);
        assert_eq!(
            bge.sha256,
            "363a0a4855dff6c653e06efe3209157debcf7f74e52d0d7c71e2747cd523043e"
        );
        assert_eq!(bge.license_name, "MIT");
    }

    #[test]
    fn test_catalog_deny_unknown_fields() {
        let bad_json = r#"{
            "version": 1,
            "extra_field": "hacker",
            "entries": []
        }"#;
        assert!(matches!(
            parse_catalog_json(bad_json).unwrap_err(),
            CatalogError::JsonError(_)
        ));

        let bad_entry_json = r#"{
            "version": 1,
            "entries": [{
                "id": "model1",
                "name": "Model 1",
                "kind": "chat",
                "fileName": "model1.gguf",
                "url": "https://huggingface.co/model1.gguf",
                "sizeBytes": 1000,
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "licenseName": "MIT",
                "licenseUrl": "https://opensource.org/licenses/MIT",
                "minRamBytes": 1024,
                "extraField": 123
            }]
        }"#;
        assert!(matches!(
            parse_catalog_json(bad_entry_json).unwrap_err(),
            CatalogError::JsonError(_)
        ));
    }

    #[test]
    fn test_catalog_id_validation_and_traversal_rejection() {
        assert!(validate_safe_id("qwen2.5-1.5b-instruct"));
        assert!(validate_safe_id("bge-small-en-v1.5"));
        assert!(validate_safe_id("a"));

        // Reject uppercase, spaces, and path traversal characters
        assert!(!validate_safe_id("Qwen2.5"));
        assert!(!validate_safe_id("model id"));
        assert!(!validate_safe_id("../traversal"));
        assert!(!validate_safe_id("..\\traversal"));
        assert!(!validate_safe_id("model/id"));
        assert!(!validate_safe_id("model\\id"));
        assert!(!validate_safe_id("C:model"));
        assert!(!validate_safe_id(""));
        assert!(!validate_safe_id(&"a".repeat(65)));
    }

    #[test]
    fn test_catalog_filename_validation_and_traversal_rejection() {
        assert!(validate_safe_filename("model-q4.gguf"));
        assert!(validate_safe_filename("qwen2.5-1.5b-instruct-q4_k_m.gguf"));

        // Must end with .gguf
        assert!(!validate_safe_filename("model-q4.bin"));
        assert!(!validate_safe_filename("model-q4"));

        // Reject path traversal attempts in filename
        assert!(!validate_safe_filename("../model.gguf"));
        assert!(!validate_safe_filename("..\\model.gguf"));
        assert!(!validate_safe_filename("folder/model.gguf"));
        assert!(!validate_safe_filename("folder\\model.gguf"));
        assert!(!validate_safe_filename("C:model.gguf"));
    }

    #[test]
    fn test_catalog_url_and_host_allowlist() {
        assert!(is_host_allowed("huggingface.co"));
        assert!(is_host_allowed("us.aws.cdn.hf.co"));
        assert!(is_host_allowed("cdn-lfs.hf.co"));
        assert!(is_host_allowed("github.com"));
        assert!(is_host_allowed("release-assets.githubusercontent.com"));
        assert!(is_host_allowed("objects.githubusercontent.com"));

        assert!(!is_host_allowed("malicious.com"));
        assert!(!is_host_allowed("hf.co.attacker.com"));
        assert!(!is_host_allowed("evil-huggingface.co"));
        assert!(!is_host_allowed("api.openai.com")); // Not a model download host
    }

    #[test]
    fn test_catalog_sha256_format() {
        assert!(validate_sha256_format(
            "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e"
        ));
        // Uppercase rejected
        assert!(!validate_sha256_format(
            "6A1A2EB6D15622BF3C96857206351BA97E1AF16C30D7A74EE38970E434E9407E"
        ));
        // Wrong length
        assert!(!validate_sha256_format("6a1a2eb6"));
        // Invalid chars
        assert!(!validate_sha256_format(&"g".repeat(64)));
    }
}
