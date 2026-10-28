#![forbid(unsafe_code)]

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use crate::channel::EngineChannel;

/// Default timeout for embedding requests (30 seconds).
pub const EMBEDDING_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Errors originating during embedding generation against the engine.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EmbeddingError {
    #[error("network connection to engine failed: {0}")]
    NetworkError(String),

    #[error("engine returned HTTP error status {status}")]
    EngineError { status: u16 },

    #[error("engine returned invalid embedding response format: {0}")]
    InvalidResponse(String),

    #[error("inconsistent vector dimensions returned by engine")]
    InconsistentDimensions,

    #[error("empty embedding vector returned for text chunk")]
    EmptyVector,
}

#[derive(Serialize)]
struct EmbeddingRequestPayload<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbeddingDataEntry {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbeddingResponsePayload {
    data: Vec<EmbeddingDataEntry>,
}

/// Request embeddings from the engine for a batch of text chunks.
///
/// Ensures:
/// - Exact model ID passed in request body.
/// - Bearer authorization header with ephemeral session token.
/// - Output vectors sorted matching input slice ordering.
/// - Every vector has consistent, non-zero dimension.
pub fn compute_embeddings(
    client: &Client,
    channel: &EngineChannel,
    model_id: &str,
    texts: &[&str],
) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    let url = format!("{}/v1/embeddings", channel.base_url());
    let payload = EmbeddingRequestPayload {
        model: model_id,
        input: texts,
    };

    let response = client
        .post(&url)
        .header(reqwest::header::AUTHORIZATION, channel.auth_header_value())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .timeout(EMBEDDING_REQUEST_TIMEOUT)
        .json(&payload)
        .send()
        .map_err(|e| EmbeddingError::NetworkError(e.to_string()))?;

    if !response.status().is_success() {
        return Err(EmbeddingError::EngineError {
            status: response.status().as_u16(),
        });
    }

    let parsed: EmbeddingResponsePayload = response
        .json()
        .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

    if parsed.data.len() != texts.len() {
        return Err(EmbeddingError::InvalidResponse(format!(
            "Expected {} embedding vectors, but engine returned {}",
            texts.len(),
            parsed.data.len()
        )));
    }

    // Sort by returned index
    let mut sorted_entries = parsed.data;
    sorted_entries.sort_by_key(|e| e.index);

    let mut result = Vec::with_capacity(sorted_entries.len());
    let mut expected_dim: Option<usize> = None;

    for entry in sorted_entries {
        if entry.embedding.is_empty() {
            return Err(EmbeddingError::EmptyVector);
        }

        if let Some(dim) = expected_dim {
            if entry.embedding.len() != dim {
                return Err(EmbeddingError::InconsistentDimensions);
            }
        } else {
            expected_dim = Some(entry.embedding.len());
        }

        result.push(entry.embedding);
    }

    Ok(result)
}
