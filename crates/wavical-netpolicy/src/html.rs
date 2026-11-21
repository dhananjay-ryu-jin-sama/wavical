//! Inline-script SHA-256 CSP hashing and HTML scanner result model (Plan 1.10).

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

/// A SHA-256 CSP hash token (e.g. `'sha256-...'`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CspHash {
    pub base64_digest: String,
}

impl CspHash {
    /// Returns the CSP source expression token, e.g. `'sha256-abc...'`.
    #[must_use]
    pub fn as_csp_token(&self) -> String {
        format!("'sha256-{}'", self.base64_digest)
    }
}

/// Computes the exact-byte SHA-256 CSP hash of an inline `<script>` body.
#[must_use]
pub fn hash_inline_script(script_bytes: &[u8]) -> CspHash {
    let digest = Sha256::digest(script_bytes);
    CspHash {
        base64_digest: STANDARD.encode(digest),
    }
}

/// Category of an external resource reference found in HTML/CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExternalRefCategory {
    Script,
    Style,
    Font,
    Image,
    Media,
    Frame,
}

/// An inline `<script>` block found by the HTML scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineScriptBlock {
    pub index: usize,
    pub line: usize,
    pub byte_len: usize,
    pub hash: CspHash,
    pub content: String,
}

/// An inline event handler attribute (`onclick=`, `onerror=`, etc.) found by the HTML scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineHandlerFinding {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub tag: String,
    pub attribute: String,
}

/// An inline `style="..."` attribute found by the HTML scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleAttrFinding {
    pub file: String,
    pub line: usize,
    pub tag: String,
}

/// An external resource reference (`src=`, `href=`, etc.) found by the HTML scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRef {
    pub category: ExternalRefCategory,
    pub url: String,
    pub line: usize,
    pub has_integrity: bool,
}

/// Structured output populated by the CLI HTML/CSS scanner for `safe` hashing,
/// `ultra-safe` extraction, `wavical vendor`, and build warnings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HtmlScanResult {
    pub file: String,
    pub inline_scripts: Vec<InlineScriptBlock>,
    pub inline_handlers: Vec<InlineHandlerFinding>,
    pub style_attributes: Vec<StyleAttrFinding>,
    pub external_refs: Vec<ExternalRef>,
}
