#![forbid(unsafe_code)]

//! Wavical AI Subsystem (Phase 3).
//!
//! Provides model catalog management, safe verified downloads,
//! license confirmation flows, and model cache storage.

pub mod catalog;
pub mod channel;
pub mod client;
pub mod commands;
pub mod doctor;
pub mod downloader;
pub mod engine;
pub mod gguf;
pub mod license;
pub mod lifecycle_policy;
pub mod manager;
pub mod process;
pub mod sha256;
pub mod storage;

pub use catalog::*;
pub use channel::*;
pub use client::*;
pub use commands::*;
pub use doctor::*;
pub use downloader::*;
pub use engine::*;
pub use gguf::*;
pub use license::*;
pub use lifecycle_policy::*;
pub use manager::*;
pub use process::*;
pub use sha256::*;
pub use storage::*;
