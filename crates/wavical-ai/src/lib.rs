#![forbid(unsafe_code)]

//! Wavical AI Subsystem (Phase 3).
//!
//! Provides model catalog management, safe verified downloads,
//! license confirmation flows, and model cache storage.

pub mod catalog;
pub mod channel;
pub mod chat;
pub mod chunker;
pub mod client;
pub mod cloud_client;
pub mod cloud_protocol;
pub mod commands;
pub mod consent;
pub mod credentials;
pub mod doctor;
pub mod downloader;
pub mod embeddings;
pub mod engine;
pub mod gguf;
pub mod license;
pub mod lifecycle_policy;
pub mod manager;
pub mod process;
pub mod rag;
pub mod sha256;
pub mod storage;
pub mod store;

pub use catalog::*;
pub use channel::*;
pub use chat::*;
pub use chunker::*;
pub use client::*;
pub use cloud_client::*;
pub use cloud_protocol::*;
pub use commands::*;
pub use consent::*;
pub use credentials::*;
pub use doctor::*;
pub use downloader::*;
pub use embeddings::*;
pub use engine::*;
pub use gguf::*;
pub use license::*;
pub use lifecycle_policy::*;
pub use manager::*;
pub use process::*;
pub use rag::*;
pub use sha256::*;
pub use storage::*;
pub use store::*;
