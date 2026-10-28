#![forbid(unsafe_code)]

//! Engine HTTP client, health checks, and version handshakes.
//!
//! Provides a safe loopback HTTP interface for querying `/health` and checking
//! engine responsiveness before sending user inference requests.

use reqwest::blocking::Client;
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::channel::EngineChannel;

/// Default timeout for health check handshakes during engine startup.
pub const DEFAULT_HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(
        "Local AI engine is currently unavailable. Please verify model files or run 'wavical ai doctor'."
    )]
    EngineUnavailable,

    #[error("HTTP request to engine failed: {0}")]
    HttpError(String),
}

/// Create a pre-configured blocking HTTP client for talking to the local engine.
pub fn create_engine_http_client() -> Result<Client, ClientError> {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ClientError::HttpError(e.to_string()))
}

/// Query the `/health` endpoint of the engine once.
pub fn check_engine_health(client: &Client, channel: &EngineChannel) -> bool {
    let url = format!("{}/health", channel.base_url());
    let resp = client
        .get(&url)
        .header("Authorization", channel.auth_header_value())
        .send();

    match resp {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

/// Poll the engine's `/health` endpoint until it responds with HTTP 200 or timeout expires.
pub fn wait_for_engine_ready(
    client: &Client,
    channel: &EngineChannel,
    timeout: Duration,
) -> Result<(), ClientError> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(50);

    while start.elapsed() < timeout {
        if check_engine_health(client, channel) {
            return Ok(());
        }
        std::thread::sleep(poll_interval);
    }

    Err(ClientError::EngineUnavailable)
}
