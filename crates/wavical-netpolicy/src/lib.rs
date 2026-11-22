//! Pure network policy crate for Wavical Phase 4.5 (Network Modes).
//!
//! Contains allowlist parsing, parsed-URL origin matching, CSP generation,
//! policy decision functions, inline-script SHA-256 hashing, HTML scanner
//! result models, CDN URL version detection, and `vendor.lock.json` parsing.
//! Zero `tao`, `wry`, network, or OS dependencies.

#![forbid(unsafe_code)]

pub mod allowlist;
pub mod csp;
pub mod decisions;
pub mod diagnostics;
pub mod html;
pub mod limits;
pub mod origin;
pub mod vendor_lock;
pub mod version;

use serde::{Deserialize, Serialize};

/// The three Wavical network modes (Plan 1.1–1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkMode {
    Open,
    Safe,
    UltraSafe,
}

/// Policy for user-initiated external top-level link clicks (`window.externalLinks`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalLinksMode {
    #[default]
    Browser,
    InApp,
}
