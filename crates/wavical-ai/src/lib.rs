#![forbid(unsafe_code)]

//! Wavical AI Subsystem (Phase 3).
//!
//! Provides model catalog management, safe verified downloads,
//! license confirmation flows, and model cache storage.

pub mod catalog;
pub mod commands;
pub mod downloader;
pub mod gguf;
pub mod license;
pub mod sha256;
pub mod storage;

pub use catalog::*;
pub use commands::*;
pub use downloader::*;
pub use gguf::*;
pub use license::*;
pub use sha256::*;
pub use storage::*;
