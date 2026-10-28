#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ============================================================================
// Plan Section 1.5 Named Constants
// ============================================================================

// --- Chat Limits ---
/// Maximum number of messages allowed in a single chat request.
pub const MAX_CHAT_MESSAGES: usize = 50;

/// Maximum character count for a single message in a chat request.
pub const MAX_CHAT_MESSAGE_CHARS: usize = 32_000;

/// Maximum total combined character count across all messages in a chat request.
pub const MAX_CHAT_TOTAL_CHARS: usize = 128_000;

/// Hard ceiling for maxTokens requested per chat completion.
pub const MAX_CHAT_MAX_TOKENS_CEILING: u32 = 4_096;

/// Default maxTokens per answer.
pub const DEFAULT_CHAT_MAX_TOKENS: u32 = 1_024;

/// Minimum allowed sampling temperature.
pub const MIN_CHAT_TEMPERATURE: f64 = 0.0;

/// Maximum allowed sampling temperature.
pub const MAX_CHAT_TEMPERATURE: f64 = 2.0;

/// Maximum concurrent generation requests allowed per window.
pub const MAX_CONCURRENT_GENERATIONS_PER_WINDOW: usize = 1;

/// Wall-clock timeout in seconds for a chat completion request.
pub const CHAT_WALL_CLOCK_TIMEOUT_SECS: u64 = 120;

/// Maximum character count cap for streamed output from an answer.
pub const MAX_CHAT_STREAMED_OUTPUT_CHARS: usize = 100_000;

// --- Local Limits ---
/// Hard ceiling for context size in local engine configuration.
pub const MAX_LOCAL_CONTEXT_SIZE_CEILING: usize = 8_192;

/// Default local context size.
pub const DEFAULT_LOCAL_CONTEXT_SIZE: usize = 4_096;

// --- Index Limits ---
/// Minimum character length for an index collection name.
pub const MIN_COLLECTION_NAME_CHARS: usize = 1;

/// Maximum character length for an index collection name (`[a-z0-9._-]{1,64}`).
pub const MAX_COLLECTION_NAME_CHARS: usize = 64;

/// Maximum collections allowed per application.
pub const MAX_COLLECTIONS_PER_APP: usize = 16;

/// Minimum documents per `ai.index` call.
pub const MIN_DOCUMENTS_PER_INDEX_CALL: usize = 1;

/// Maximum documents allowed per `ai.index` call.
pub const MAX_DOCUMENTS_PER_INDEX_CALL: usize = 100;

/// Minimum character length for a document identifier.
pub const MIN_DOCUMENT_ID_CHARS: usize = 1;

/// Maximum character length for a document identifier (`[A-Za-z0-9._:-]{1,128}`).
pub const MAX_DOCUMENT_ID_CHARS: usize = 128;

/// Maximum character count allowed per document.
pub const MAX_DOCUMENT_CHARS: usize = 100_000;

/// Maximum serialized JSON bytes allowed for a document's metadata (4 KB).
pub const MAX_DOCUMENT_METADATA_BYTES: usize = 4_096;

/// Maximum chunks allowed per collection.
pub const MAX_CHUNKS_PER_COLLECTION: usize = 50_000;

/// Maximum storage file size allowed per application index (512 MB).
pub const MAX_INDEX_FILE_SIZE_BYTES: u64 = 512 * 1024 * 1024;

// --- Search Limits ---
/// Maximum character length for a search query.
pub const MAX_SEARCH_QUERY_CHARS: usize = 4_000;

/// Minimum topK hits returned by search.
pub const MIN_SEARCH_TOP_K: usize = 1;

/// Maximum topK hits returned by search.
pub const MAX_SEARCH_TOP_K: usize = 20;

/// Default topK hits if not specified.
pub const DEFAULT_SEARCH_TOP_K: usize = 5;

// --- Cloud Limits ---
/// Maximum request body size for cloud calls (1 MB).
pub const MAX_CLOUD_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// Maximum response body size for non-streamed cloud calls (4 MB).
pub const MAX_CLOUD_RESPONSE_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Cloud connection timeout in seconds.
pub const CLOUD_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Cloud total request timeout in seconds.
pub const CLOUD_TOTAL_TIMEOUT_SECS: u64 = 120;

/// Cloud redirects limit (none followed).
pub const CLOUD_MAX_REDIRECTS: usize = 0;

// --- Download Limits ---
/// Maximum concurrent model downloads allowed.
pub const MAX_CONCURRENT_DOWNLOADS: usize = 1;

// ============================================================================
// AI Method Names and Capabilities
// ============================================================================

/// Method name for chat completions.
pub const METHOD_AI_CHAT: &str = "ai.chat";

/// Method name for document indexing.
pub const METHOD_AI_INDEX: &str = "ai.index";

/// Method name for semantic document search.
pub const METHOD_AI_SEARCH: &str = "ai.search";

/// Array of all Phase 3 AI method names (sorted).
pub const AI_METHOD_NAMES: &[&str] = &[METHOD_AI_CHAT, METHOD_AI_INDEX, METHOD_AI_SEARCH];

// ============================================================================
// Tracing Event Names and Field Names (Section 1.1 Invariants 4 & 5)
// Prompts and outputs are never logged. Secrets never appear.
// ============================================================================

pub const EVENT_AI_CALL_STARTED: &str = "ai.call.started";
pub const EVENT_AI_CALL_FINISHED: &str = "ai.call.finished";
pub const EVENT_AI_CALL_DENIED: &str = "ai.call.denied";
pub const EVENT_AI_CALL_CANCELLED: &str = "ai.call.cancelled";
pub const EVENT_AI_CALL_FAILED: &str = "ai.call.failed";

pub const FIELD_WINDOW_ID: &str = "window_id";
pub const FIELD_METHOD: &str = "method";
pub const FIELD_BACKEND: &str = "backend";
pub const FIELD_MODEL_ID: &str = "model_id";
pub const FIELD_INPUT_SIZE_CHARS: &str = "input_size_chars";
pub const FIELD_OUTPUT_SIZE_CHARS: &str = "output_size_chars";
pub const FIELD_DURATION_MS: &str = "duration_ms";
pub const FIELD_REASON: &str = "reason";

// ============================================================================
// Chat Types & Validation
// ============================================================================

/// Allowed roles for chat messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// A single message in a chat conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Request payload for `ai.chat`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,

    #[serde(
        default,
        rename = "maxTokens",
        alias = "max_tokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_tokens: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

// ============================================================================
// Index Types & Validation
// ============================================================================

/// Action to execute on an index collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexAction {
    Add,
    Replace,
    Delete,
}

/// Permitted metadata primitive values (string, number, boolean only; no nested objects or null).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetadataValue {
    String(String),
    Number(serde_json::Number),
    Bool(bool),
}

/// A document to add to or replace in a collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, MetadataValue>>,
}

/// Request payload for `ai.index`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexRequest {
    pub action: IndexAction,
    pub collection: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents: Option<Vec<Document>>,
}

// ============================================================================
// Search Types & Validation
// ============================================================================

fn default_top_k() -> usize {
    DEFAULT_SEARCH_TOP_K
}

/// Request payload for `ai.search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub collection: String,
    pub query: String,
    #[serde(default = "default_top_k", rename = "topK", alias = "top_k")]
    pub top_k: usize,
}

// ============================================================================
// Error Types
// ============================================================================

/// Detailed validation error returned by pure AI request validators.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum AiValidationError {
    #[error("chat request must contain at least one message")]
    EmptyMessages,

    #[error("chat request exceeds maximum messages limit of {max} (received {count})")]
    TooManyMessages { count: usize, max: usize },

    #[error(
        "message at index {index} exceeds maximum character limit of {max} (contains {chars} chars)"
    )]
    MessageTooLong {
        index: usize,
        chars: usize,
        max: usize,
    },

    #[error(
        "total chat request content exceeds maximum limit of {max} characters (contains {chars} chars)"
    )]
    TotalCharsTooLarge { chars: usize, max: usize },

    #[error("invalid maxTokens value {tokens}: must be between 1 and {max}")]
    InvalidMaxTokens { tokens: u32, max: u32 },

    #[error("invalid temperature value {temp}: must be between {min} and {max}")]
    InvalidTemperature { temp: f64, min: f64, max: f64 },

    #[error("collection name '{0}' is invalid: must match pattern [a-z0-9._-]{{1,64}}")]
    InvalidCollectionName(String),

    #[error("document id '{0}' is invalid: must match pattern [A-Za-z0-9._:-]{{1,128}}")]
    InvalidDocumentId(String),

    #[error("index request must specify at least one document for add/replace action")]
    MissingDocuments,

    #[error("delete collection action cannot specify document payloads")]
    DocumentsNotAllowedOnDelete,

    #[error("index request exceeds maximum documents per call limit of {max} (received {count})")]
    TooManyDocuments { count: usize, max: usize },

    #[error("document '{id}' text cannot be empty")]
    EmptyDocumentText { id: String },

    #[error(
        "document '{id}' text exceeds maximum character limit of {max} (contains {chars} chars)"
    )]
    DocumentTextTooLong {
        id: String,
        chars: usize,
        max: usize,
    },

    #[error(
        "document '{id}' metadata exceeds maximum limit of {max} bytes (serialized size: {bytes} bytes)"
    )]
    MetadataTooLarge {
        id: String,
        bytes: usize,
        max: usize,
    },

    #[error("search query cannot be empty")]
    EmptySearchQuery,

    #[error("search query exceeds maximum character limit of {max} (contains {chars} chars)")]
    SearchQueryTooLong { chars: usize, max: usize },

    #[error("invalid topK value {top_k}: must be between {min} and {max}")]
    InvalidTopK {
        top_k: usize,
        min: usize,
        max: usize,
    },

    #[error("catalog model id '{0}' is invalid: must match pattern [a-z0-9._-]{{1,64}}")]
    InvalidCatalogModelId(String),
}

// ============================================================================
// Validators
// ============================================================================

/// Validate a `ChatRequest` against all Phase 3 Section 1.5 limits.
pub fn validate_chat_request(req: &ChatRequest) -> Result<(), AiValidationError> {
    if req.messages.is_empty() {
        return Err(AiValidationError::EmptyMessages);
    }

    if req.messages.len() > MAX_CHAT_MESSAGES {
        return Err(AiValidationError::TooManyMessages {
            count: req.messages.len(),
            max: MAX_CHAT_MESSAGES,
        });
    }

    let mut total_chars: usize = 0;
    for (index, msg) in req.messages.iter().enumerate() {
        let chars = msg.content.chars().count();
        if chars > MAX_CHAT_MESSAGE_CHARS {
            return Err(AiValidationError::MessageTooLong {
                index,
                chars,
                max: MAX_CHAT_MESSAGE_CHARS,
            });
        }
        total_chars = total_chars.saturating_add(chars);
    }

    if total_chars > MAX_CHAT_TOTAL_CHARS {
        return Err(AiValidationError::TotalCharsTooLarge {
            chars: total_chars,
            max: MAX_CHAT_TOTAL_CHARS,
        });
    }

    if let Some(tokens) = req.max_tokens
        && (tokens == 0 || tokens > MAX_CHAT_MAX_TOKENS_CEILING)
    {
        return Err(AiValidationError::InvalidMaxTokens {
            tokens,
            max: MAX_CHAT_MAX_TOKENS_CEILING,
        });
    }

    if let Some(temp) = req.temperature
        && (temp.is_nan() || temp < MIN_CHAT_TEMPERATURE || temp > MAX_CHAT_TEMPERATURE)
    {
        return Err(AiValidationError::InvalidTemperature {
            temp,
            min: MIN_CHAT_TEMPERATURE,
            max: MAX_CHAT_TEMPERATURE,
        });
    }

    Ok(())
}

/// Validate that a collection name matches `^[a-z0-9._-]{1,64}$`.
pub fn validate_collection_name(name: &str) -> Result<(), AiValidationError> {
    let char_count = name.chars().count();
    if !(MIN_COLLECTION_NAME_CHARS..=MAX_COLLECTION_NAME_CHARS).contains(&char_count) {
        return Err(AiValidationError::InvalidCollectionName(name.to_string()));
    }

    for c in name.chars() {
        let is_valid =
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-';
        if !is_valid {
            return Err(AiValidationError::InvalidCollectionName(name.to_string()));
        }
    }

    Ok(())
}

/// Validate that a document ID matches `^[A-Za-z0-9._:-]{1,128}$`.
pub fn validate_document_id(id: &str) -> Result<(), AiValidationError> {
    let char_count = id.chars().count();
    if !(MIN_DOCUMENT_ID_CHARS..=MAX_DOCUMENT_ID_CHARS).contains(&char_count) {
        return Err(AiValidationError::InvalidDocumentId(id.to_string()));
    }

    for c in id.chars() {
        let is_valid = c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == ':' || c == '-';
        if !is_valid {
            return Err(AiValidationError::InvalidDocumentId(id.to_string()));
        }
    }

    Ok(())
}

/// Validate metadata payload size (4 KB max).
pub fn validate_document_metadata(
    doc_id: &str,
    metadata: &HashMap<String, MetadataValue>,
) -> Result<(), AiValidationError> {
    let serialized =
        serde_json::to_vec(metadata).map_err(|_| AiValidationError::MetadataTooLarge {
            id: doc_id.to_string(),
            bytes: usize::MAX,
            max: MAX_DOCUMENT_METADATA_BYTES,
        })?;

    if serialized.len() > MAX_DOCUMENT_METADATA_BYTES {
        return Err(AiValidationError::MetadataTooLarge {
            id: doc_id.to_string(),
            bytes: serialized.len(),
            max: MAX_DOCUMENT_METADATA_BYTES,
        });
    }

    Ok(())
}

/// Validate an `IndexRequest` against all Phase 3 Section 1.5 limits.
pub fn validate_index_request(req: &IndexRequest) -> Result<(), AiValidationError> {
    validate_collection_name(&req.collection)?;

    match req.action {
        IndexAction::Delete => {
            if let Some(ref docs) = req.documents
                && !docs.is_empty()
            {
                return Err(AiValidationError::DocumentsNotAllowedOnDelete);
            }
        }
        IndexAction::Add | IndexAction::Replace => {
            let docs = req
                .documents
                .as_ref()
                .ok_or(AiValidationError::MissingDocuments)?;

            if docs.is_empty() {
                return Err(AiValidationError::MissingDocuments);
            }

            if docs.len() > MAX_DOCUMENTS_PER_INDEX_CALL {
                return Err(AiValidationError::TooManyDocuments {
                    count: docs.len(),
                    max: MAX_DOCUMENTS_PER_INDEX_CALL,
                });
            }

            for doc in docs {
                validate_document_id(&doc.id)?;

                let chars = doc.text.chars().count();
                if chars == 0 {
                    return Err(AiValidationError::EmptyDocumentText { id: doc.id.clone() });
                }
                if chars > MAX_DOCUMENT_CHARS {
                    return Err(AiValidationError::DocumentTextTooLong {
                        id: doc.id.clone(),
                        chars,
                        max: MAX_DOCUMENT_CHARS,
                    });
                }

                if let Some(ref meta) = doc.metadata {
                    validate_document_metadata(&doc.id, meta)?;
                }
            }
        }
    }

    Ok(())
}

/// Validate a `SearchRequest` against all Phase 3 Section 1.5 limits.
pub fn validate_search_request(req: &SearchRequest) -> Result<(), AiValidationError> {
    validate_collection_name(&req.collection)?;

    let chars = req.query.chars().count();
    if chars == 0 {
        return Err(AiValidationError::EmptySearchQuery);
    }
    if chars > MAX_SEARCH_QUERY_CHARS {
        return Err(AiValidationError::SearchQueryTooLong {
            chars,
            max: MAX_SEARCH_QUERY_CHARS,
        });
    }

    if req.top_k < MIN_SEARCH_TOP_K || req.top_k > MAX_SEARCH_TOP_K {
        return Err(AiValidationError::InvalidTopK {
            top_k: req.top_k,
            min: MIN_SEARCH_TOP_K,
            max: MAX_SEARCH_TOP_K,
        });
    }

    Ok(())
}

/// Approved Phase 3 model catalog identifiers.
pub const CATALOG_MODEL_IDS: &[&str] = &["bge-small-en-v1.5", "qwen2.5-1.5b-instruct"];

/// Check if a model identifier belongs to the approved Phase 3 catalog.
pub fn is_known_catalog_model(id: &str) -> bool {
    CATALOG_MODEL_IDS.contains(&id)
}

/// Validate catalog model identifier format (`^[a-z0-9._-]{1,64}$`).
pub fn validate_catalog_model_id(id: &str) -> Result<(), AiValidationError> {
    let char_count = id.chars().count();
    if !(MIN_COLLECTION_NAME_CHARS..=MAX_COLLECTION_NAME_CHARS).contains(&char_count) {
        return Err(AiValidationError::InvalidCatalogModelId(id.to_string()));
    }

    for c in id.chars() {
        let is_valid =
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-';
        if !is_valid {
            return Err(AiValidationError::InvalidCatalogModelId(id.to_string()));
        }
    }

    Ok(())
}

// ============================================================================
// Tracing Helpers & Formatting
// ============================================================================

/// Structured representation of an AI tracing event.
/// Content (prompts, answers, documents, queries) and secrets (API keys) are strictly forbidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiTraceRecord {
    pub event_name: String,
    pub window_id: String,
    pub method: String,
    pub backend: Option<String>,
    pub model_id: Option<String>,
    pub input_size_chars: Option<usize>,
    pub output_size_chars: Option<usize>,
    pub duration_ms: Option<u64>,
    pub reason: Option<String>,
}

impl AiTraceRecord {
    /// Format the record into a log string.
    pub fn format_event(&self) -> String {
        let mut parts = vec![
            format!("event={}", self.event_name),
            format!("window_id={}", self.window_id),
            format!("method={}", self.method),
        ];

        if let Some(ref backend) = self.backend {
            parts.push(format!("backend={backend}"));
        }
        if let Some(ref model) = self.model_id {
            parts.push(format!("model_id={model}"));
        }
        if let Some(in_sz) = self.input_size_chars {
            parts.push(format!("input_size_chars={in_sz}"));
        }
        if let Some(out_sz) = self.output_size_chars {
            parts.push(format!("output_size_chars={out_sz}"));
        }
        if let Some(dur) = self.duration_ms {
            parts.push(format!("duration_ms={dur}"));
        }
        if let Some(ref reason) = self.reason {
            parts.push(format!("reason={reason}"));
        }

        parts.join(" ")
    }
}

/// Helper function to create a trace record for `ai.call.started`.
pub fn record_call_started(
    window_id: &str,
    method: &str,
    backend: &str,
    model_id: &str,
    input_size_chars: usize,
) -> AiTraceRecord {
    AiTraceRecord {
        event_name: EVENT_AI_CALL_STARTED.to_string(),
        window_id: window_id.to_string(),
        method: method.to_string(),
        backend: Some(backend.to_string()),
        model_id: Some(model_id.to_string()),
        input_size_chars: Some(input_size_chars),
        output_size_chars: None,
        duration_ms: None,
        reason: None,
    }
}

/// Helper function to create a trace record for `ai.call.finished`.
pub fn record_call_finished(
    window_id: &str,
    method: &str,
    backend: &str,
    model_id: &str,
    input_size_chars: usize,
    output_size_chars: usize,
    duration_ms: u64,
) -> AiTraceRecord {
    AiTraceRecord {
        event_name: EVENT_AI_CALL_FINISHED.to_string(),
        window_id: window_id.to_string(),
        method: method.to_string(),
        backend: Some(backend.to_string()),
        model_id: Some(model_id.to_string()),
        input_size_chars: Some(input_size_chars),
        output_size_chars: Some(output_size_chars),
        duration_ms: Some(duration_ms),
        reason: None,
    }
}

/// Helper function to create a trace record for `ai.call.denied`.
pub fn record_call_denied(window_id: &str, method: &str, reason: &str) -> AiTraceRecord {
    AiTraceRecord {
        event_name: EVENT_AI_CALL_DENIED.to_string(),
        window_id: window_id.to_string(),
        method: method.to_string(),
        backend: None,
        model_id: None,
        input_size_chars: None,
        output_size_chars: None,
        duration_ms: None,
        reason: Some(reason.to_string()),
    }
}

/// Helper function to create a trace record for `ai.call.cancelled`.
pub fn record_call_cancelled(window_id: &str, method: &str, duration_ms: u64) -> AiTraceRecord {
    AiTraceRecord {
        event_name: EVENT_AI_CALL_CANCELLED.to_string(),
        window_id: window_id.to_string(),
        method: method.to_string(),
        backend: None,
        model_id: None,
        input_size_chars: None,
        output_size_chars: None,
        duration_ms: Some(duration_ms),
        reason: None,
    }
}

/// Helper function to create a trace record for `ai.call.failed`.
pub fn record_call_failed(
    window_id: &str,
    method: &str,
    backend: &str,
    model_id: &str,
    duration_ms: u64,
    error_code: &str,
) -> AiTraceRecord {
    AiTraceRecord {
        event_name: EVENT_AI_CALL_FAILED.to_string(),
        window_id: window_id.to_string(),
        method: method.to_string(),
        backend: Some(backend.to_string()),
        model_id: Some(model_id.to_string()),
        input_size_chars: None,
        output_size_chars: None,
        duration_ms: Some(duration_ms),
        reason: Some(error_code.to_string()),
    }
}

// ============================================================================
// Unit & Boundary Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_chat_request_boundaries() {
        // 1. Empty messages rejected
        let empty_req = ChatRequest {
            messages: vec![],
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&empty_req).unwrap_err(),
            AiValidationError::EmptyMessages
        );

        // 2. Exactly MAX_CHAT_MESSAGES (50) accepted
        let msg_50: Vec<ChatMessage> = (0..MAX_CHAT_MESSAGES)
            .map(|_| ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
            })
            .collect();
        let valid_50 = ChatRequest {
            messages: msg_50,
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert!(validate_chat_request(&valid_50).is_ok());

        // 3. MAX_CHAT_MESSAGES + 1 (51) rejected
        let msg_51: Vec<ChatMessage> = (0..=MAX_CHAT_MESSAGES)
            .map(|_| ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
            })
            .collect();
        let invalid_51 = ChatRequest {
            messages: msg_51,
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&invalid_51).unwrap_err(),
            AiValidationError::TooManyMessages {
                count: 51,
                max: MAX_CHAT_MESSAGES,
            }
        );

        // 4. Single message exact 32,000 ASCII chars accepted
        let content_32k = "a".repeat(MAX_CHAT_MESSAGE_CHARS);
        let req_32k = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: content_32k,
            }],
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert!(validate_chat_request(&req_32k).is_ok());

        // 5. Single message exact 32,001 ASCII chars rejected
        let content_32k_plus_1 = "a".repeat(MAX_CHAT_MESSAGE_CHARS + 1);
        let req_32k_plus_1 = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: content_32k_plus_1,
            }],
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&req_32k_plus_1).unwrap_err(),
            AiValidationError::MessageTooLong {
                index: 0,
                chars: MAX_CHAT_MESSAGE_CHARS + 1,
                max: MAX_CHAT_MESSAGE_CHARS,
            }
        );

        // 6. Multibyte UTF-8 boundary (32,000 multibyte characters)
        // '🦀' is 4 bytes, 32,000 chars = 128,000 bytes. Char count should be 32,000.
        let multibyte_32k: String = "🦀".repeat(MAX_CHAT_MESSAGE_CHARS);
        assert_eq!(multibyte_32k.chars().count(), MAX_CHAT_MESSAGE_CHARS);
        assert_eq!(multibyte_32k.len(), MAX_CHAT_MESSAGE_CHARS * 4);
        let req_multibyte_32k = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: multibyte_32k,
            }],
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert!(validate_chat_request(&req_multibyte_32k).is_ok());

        // 7. Multibyte UTF-8 boundary + 1 char (32,001 chars) rejected
        let multibyte_32k_plus_1: String = "🦀".repeat(MAX_CHAT_MESSAGE_CHARS + 1);
        let req_multibyte_plus_1 = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: multibyte_32k_plus_1,
            }],
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&req_multibyte_plus_1).unwrap_err(),
            AiValidationError::MessageTooLong {
                index: 0,
                chars: MAX_CHAT_MESSAGE_CHARS + 1,
                max: MAX_CHAT_MESSAGE_CHARS,
            }
        );

        // 8. Total chars boundary: exactly 128,000 chars across 4 messages accepted
        let chunk_32k = "b".repeat(32_000);
        let req_total_128k = ChatRequest {
            messages: vec![
                ChatMessage {
                    role: ChatRole::System,
                    content: chunk_32k.clone(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: chunk_32k.clone(),
                },
                ChatMessage {
                    role: ChatRole::Assistant,
                    content: chunk_32k.clone(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: chunk_32k.clone(),
                },
            ],
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert!(validate_chat_request(&req_total_128k).is_ok());

        // 9. Total chars boundary + 1 char (128,001 chars) rejected
        let req_total_128k_plus_1 = ChatRequest {
            messages: vec![
                ChatMessage {
                    role: ChatRole::System,
                    content: chunk_32k.clone(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: chunk_32k.clone(),
                },
                ChatMessage {
                    role: ChatRole::Assistant,
                    content: chunk_32k.clone(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: chunk_32k.clone(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: "x".to_string(),
                },
            ],
            max_tokens: None,
            temperature: None,
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&req_total_128k_plus_1).unwrap_err(),
            AiValidationError::TotalCharsTooLarge {
                chars: 128_001,
                max: MAX_CHAT_TOTAL_CHARS,
            }
        );

        // 10. maxTokens boundaries
        let req_tok_1 = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: Some(1),
            temperature: None,
            stream: None,
        };
        assert!(validate_chat_request(&req_tok_1).is_ok());

        let req_tok_max = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: Some(MAX_CHAT_MAX_TOKENS_CEILING),
            temperature: None,
            stream: None,
        };
        assert!(validate_chat_request(&req_tok_max).is_ok());

        let req_tok_zero = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: Some(0),
            temperature: None,
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&req_tok_zero).unwrap_err(),
            AiValidationError::InvalidMaxTokens {
                tokens: 0,
                max: MAX_CHAT_MAX_TOKENS_CEILING,
            }
        );

        let req_tok_over = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: Some(MAX_CHAT_MAX_TOKENS_CEILING + 1),
            temperature: None,
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&req_tok_over).unwrap_err(),
            AiValidationError::InvalidMaxTokens {
                tokens: MAX_CHAT_MAX_TOKENS_CEILING + 1,
                max: MAX_CHAT_MAX_TOKENS_CEILING,
            }
        );

        // 11. Temperature boundaries (0.0 to 2.0)
        let req_temp_0 = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: None,
            temperature: Some(0.0),
            stream: None,
        };
        assert!(validate_chat_request(&req_temp_0).is_ok());

        let req_temp_2 = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: None,
            temperature: Some(2.0),
            stream: None,
        };
        assert!(validate_chat_request(&req_temp_2).is_ok());

        let req_temp_neg = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: None,
            temperature: Some(-0.01),
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&req_temp_neg).unwrap_err(),
            AiValidationError::InvalidTemperature {
                temp: -0.01,
                min: MIN_CHAT_TEMPERATURE,
                max: MAX_CHAT_TEMPERATURE,
            }
        );

        let req_temp_over = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: None,
            temperature: Some(2.01),
            stream: None,
        };
        assert_eq!(
            validate_chat_request(&req_temp_over).unwrap_err(),
            AiValidationError::InvalidTemperature {
                temp: 2.01,
                min: MIN_CHAT_TEMPERATURE,
                max: MAX_CHAT_TEMPERATURE,
            }
        );

        let req_temp_nan = ChatRequest {
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            max_tokens: None,
            temperature: Some(f64::NAN),
            stream: None,
        };
        assert!(matches!(
            validate_chat_request(&req_temp_nan).unwrap_err(),
            AiValidationError::InvalidTemperature { .. }
        ));

        // 12. Unknown fields rejected via serde
        let bad_json = r#"{"messages": [{"role": "user", "content": "hi"}], "unknown_field": 123}"#;
        assert!(serde_json::from_str::<ChatRequest>(bad_json).is_err());

        let bad_msg_json = r#"{"messages": [{"role": "user", "content": "hi", "extra": true}]}"#;
        assert!(serde_json::from_str::<ChatRequest>(bad_msg_json).is_err());

        // 13. Invalid role rejected
        let bad_role_json = r#"{"messages": [{"role": "superuser", "content": "hi"}]}"#;
        assert!(serde_json::from_str::<ChatRequest>(bad_role_json).is_err());
    }

    #[test]
    fn test_index_request_boundaries() {
        // 1. Collection name boundaries: 1 char, 64 chars, 0 chars, 65 chars, charset
        assert!(validate_collection_name("a").is_ok());
        let name_64 = "a".repeat(MAX_COLLECTION_NAME_CHARS);
        assert!(validate_collection_name(&name_64).is_ok());

        assert_eq!(
            validate_collection_name("").unwrap_err(),
            AiValidationError::InvalidCollectionName("".to_string())
        );

        let name_65 = "a".repeat(MAX_COLLECTION_NAME_CHARS + 1);
        assert_eq!(
            validate_collection_name(&name_65).unwrap_err(),
            AiValidationError::InvalidCollectionName(name_65)
        );

        // Valid chars: [a-z0-9._-]
        assert!(validate_collection_name("kb-docs_2026.v1").is_ok());
        // Invalid chars: uppercase, spaces, slashes
        assert!(validate_collection_name("Kb-Docs").is_err());
        assert!(validate_collection_name("kb docs").is_err());
        assert!(validate_collection_name("kb/docs").is_err());

        // 2. Document ID boundaries: 1 char, 128 chars, 0 chars, 129 chars, charset
        assert!(validate_document_id("1").is_ok());
        let id_128 = "A".repeat(MAX_DOCUMENT_ID_CHARS);
        assert!(validate_document_id(&id_128).is_ok());

        assert_eq!(
            validate_document_id("").unwrap_err(),
            AiValidationError::InvalidDocumentId("".to_string())
        );

        let id_129 = "A".repeat(MAX_DOCUMENT_ID_CHARS + 1);
        assert_eq!(
            validate_document_id(&id_129).unwrap_err(),
            AiValidationError::InvalidDocumentId(id_129)
        );

        // Valid chars: [A-Za-z0-9._:-]
        assert!(validate_document_id("doc:section_1.2-alpha").is_ok());
        // Invalid chars: spaces, slashes, symbols
        assert!(validate_document_id("doc 1").is_err());
        assert!(validate_document_id("doc/1").is_err());
        assert!(validate_document_id("doc#1").is_err());

        // 3. Delete action: documents disallowed
        let del_req_clean = IndexRequest {
            action: IndexAction::Delete,
            collection: "docs".to_string(),
            documents: None,
        };
        assert!(validate_index_request(&del_req_clean).is_ok());

        let del_req_empty_vec = IndexRequest {
            action: IndexAction::Delete,
            collection: "docs".to_string(),
            documents: Some(vec![]),
        };
        assert!(validate_index_request(&del_req_empty_vec).is_ok());

        let del_req_with_docs = IndexRequest {
            action: IndexAction::Delete,
            collection: "docs".to_string(),
            documents: Some(vec![Document {
                id: "doc1".to_string(),
                text: "sample".to_string(),
                metadata: None,
            }]),
        };
        assert_eq!(
            validate_index_request(&del_req_with_docs).unwrap_err(),
            AiValidationError::DocumentsNotAllowedOnDelete
        );

        // 4. Add/Replace: missing or empty documents
        let add_req_none = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: None,
        };
        assert_eq!(
            validate_index_request(&add_req_none).unwrap_err(),
            AiValidationError::MissingDocuments
        );

        let add_req_empty = IndexRequest {
            action: IndexAction::Replace,
            collection: "docs".to_string(),
            documents: Some(vec![]),
        };
        assert_eq!(
            validate_index_request(&add_req_empty).unwrap_err(),
            AiValidationError::MissingDocuments
        );

        // 5. Document count boundaries: 1 doc, 100 docs, 101 docs
        let docs_100: Vec<Document> = (0..MAX_DOCUMENTS_PER_INDEX_CALL)
            .map(|i| Document {
                id: format!("doc_{i}"),
                text: "sample text".to_string(),
                metadata: None,
            })
            .collect();
        let add_req_100 = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(docs_100),
        };
        assert!(validate_index_request(&add_req_100).is_ok());

        let docs_101: Vec<Document> = (0..=MAX_DOCUMENTS_PER_INDEX_CALL)
            .map(|i| Document {
                id: format!("doc_{i}"),
                text: "sample text".to_string(),
                metadata: None,
            })
            .collect();
        let add_req_101 = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(docs_101),
        };
        assert_eq!(
            validate_index_request(&add_req_101).unwrap_err(),
            AiValidationError::TooManyDocuments {
                count: 101,
                max: MAX_DOCUMENTS_PER_INDEX_CALL,
            }
        );

        // 6. Document text boundaries: empty, 100,000 chars, 100,001 chars (multibyte included)
        let doc_empty_text = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(vec![Document {
                id: "doc1".to_string(),
                text: "".to_string(),
                metadata: None,
            }]),
        };
        assert_eq!(
            validate_index_request(&doc_empty_text).unwrap_err(),
            AiValidationError::EmptyDocumentText {
                id: "doc1".to_string(),
            }
        );

        let text_100k = "c".repeat(MAX_DOCUMENT_CHARS);
        let doc_100k = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(vec![Document {
                id: "doc1".to_string(),
                text: text_100k,
                metadata: None,
            }]),
        };
        assert!(validate_index_request(&doc_100k).is_ok());

        let text_100k_plus_1 = "c".repeat(MAX_DOCUMENT_CHARS + 1);
        let doc_100k_plus_1 = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(vec![Document {
                id: "doc1".to_string(),
                text: text_100k_plus_1,
                metadata: None,
            }]),
        };
        assert_eq!(
            validate_index_request(&doc_100k_plus_1).unwrap_err(),
            AiValidationError::DocumentTextTooLong {
                id: "doc1".to_string(),
                chars: MAX_DOCUMENT_CHARS + 1,
                max: MAX_DOCUMENT_CHARS,
            }
        );

        // Multibyte 100,000 chars ('日')
        let mb_100k: String = "日".repeat(MAX_DOCUMENT_CHARS);
        assert_eq!(mb_100k.chars().count(), MAX_DOCUMENT_CHARS);
        assert_eq!(mb_100k.len(), MAX_DOCUMENT_CHARS * 3);
        let doc_mb_100k = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(vec![Document {
                id: "doc-mb".to_string(),
                text: mb_100k,
                metadata: None,
            }]),
        };
        assert!(validate_index_request(&doc_mb_100k).is_ok());

        // 7. Metadata boundaries: primitives allowed, size limit 4 KB (4096 bytes)
        let mut meta = HashMap::new();
        meta.insert(
            "author".to_string(),
            MetadataValue::String("Alice".to_string()),
        );
        meta.insert(
            "views".to_string(),
            MetadataValue::Number(serde_json::Number::from(42)),
        );
        meta.insert("published".to_string(), MetadataValue::Bool(true));

        let doc_with_meta = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(vec![Document {
                id: "doc_meta".to_string(),
                text: "content".to_string(),
                metadata: Some(meta),
            }]),
        };
        assert!(validate_index_request(&doc_with_meta).is_ok());

        // Construct metadata payload around 4 KB
        let mut big_meta = HashMap::new();
        // Serialized format: {"val":"<string>"} -> overhead ~10 bytes
        let big_str = "x".repeat(4080);
        big_meta.insert("val".to_string(), MetadataValue::String(big_str));
        let serialized_len = serde_json::to_vec(&big_meta).unwrap().len();
        assert!(serialized_len <= MAX_DOCUMENT_METADATA_BYTES);

        let doc_big_meta = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(vec![Document {
                id: "doc_big".to_string(),
                text: "content".to_string(),
                metadata: Some(big_meta),
            }]),
        };
        assert!(validate_index_request(&doc_big_meta).is_ok());

        // Oversized metadata (> 4096 bytes)
        let mut huge_meta = HashMap::new();
        huge_meta.insert("val".to_string(), MetadataValue::String("x".repeat(5000)));
        let doc_huge_meta = IndexRequest {
            action: IndexAction::Add,
            collection: "docs".to_string(),
            documents: Some(vec![Document {
                id: "doc_huge".to_string(),
                text: "content".to_string(),
                metadata: Some(huge_meta),
            }]),
        };
        assert!(matches!(
            validate_index_request(&doc_huge_meta).unwrap_err(),
            AiValidationError::MetadataTooLarge { .. }
        ));

        // 8. Unknown fields rejected
        let bad_json = r#"{"action": "add", "collection": "docs", "documents": [], "extra": 1}"#;
        assert!(serde_json::from_str::<IndexRequest>(bad_json).is_err());

        let bad_doc_json = r#"{"action": "add", "collection": "docs", "documents": [{"id": "1", "text": "a", "extra": 2}]}"#;
        assert!(serde_json::from_str::<IndexRequest>(bad_doc_json).is_err());
    }

    #[test]
    fn test_search_request_boundaries() {
        // 1. Collection name validation in search
        let bad_col = SearchRequest {
            collection: "Invalid/Col".to_string(),
            query: "rust".to_string(),
            top_k: 5,
        };
        assert_eq!(
            validate_search_request(&bad_col).unwrap_err(),
            AiValidationError::InvalidCollectionName("Invalid/Col".to_string())
        );

        // 2. Query length boundaries: 0 chars, 4,000 chars, 4,001 chars (multibyte included)
        let empty_query = SearchRequest {
            collection: "docs".to_string(),
            query: "".to_string(),
            top_k: 5,
        };
        assert_eq!(
            validate_search_request(&empty_query).unwrap_err(),
            AiValidationError::EmptySearchQuery
        );

        let query_4k = "q".repeat(MAX_SEARCH_QUERY_CHARS);
        let req_4k = SearchRequest {
            collection: "docs".to_string(),
            query: query_4k,
            top_k: 5,
        };
        assert!(validate_search_request(&req_4k).is_ok());

        let query_4k_plus_1 = "q".repeat(MAX_SEARCH_QUERY_CHARS + 1);
        let req_4k_plus_1 = SearchRequest {
            collection: "docs".to_string(),
            query: query_4k_plus_1,
            top_k: 5,
        };
        assert_eq!(
            validate_search_request(&req_4k_plus_1).unwrap_err(),
            AiValidationError::SearchQueryTooLong {
                chars: MAX_SEARCH_QUERY_CHARS + 1,
                max: MAX_SEARCH_QUERY_CHARS,
            }
        );

        // Multibyte 4,000 chars ('🔍')
        let mb_4k: String = "🔍".repeat(MAX_SEARCH_QUERY_CHARS);
        assert_eq!(mb_4k.chars().count(), MAX_SEARCH_QUERY_CHARS);
        let req_mb_4k = SearchRequest {
            collection: "docs".to_string(),
            query: mb_4k,
            top_k: 5,
        };
        assert!(validate_search_request(&req_mb_4k).is_ok());

        // 3. topK boundaries: 1, 20, 0, 21, and default
        let req_top1 = SearchRequest {
            collection: "docs".to_string(),
            query: "find".to_string(),
            top_k: 1,
        };
        assert!(validate_search_request(&req_top1).is_ok());

        let req_top20 = SearchRequest {
            collection: "docs".to_string(),
            query: "find".to_string(),
            top_k: 20,
        };
        assert!(validate_search_request(&req_top20).is_ok());

        let req_top0 = SearchRequest {
            collection: "docs".to_string(),
            query: "find".to_string(),
            top_k: 0,
        };
        assert_eq!(
            validate_search_request(&req_top0).unwrap_err(),
            AiValidationError::InvalidTopK {
                top_k: 0,
                min: MIN_SEARCH_TOP_K,
                max: MAX_SEARCH_TOP_K,
            }
        );

        let req_top21 = SearchRequest {
            collection: "docs".to_string(),
            query: "find".to_string(),
            top_k: 21,
        };
        assert_eq!(
            validate_search_request(&req_top21).unwrap_err(),
            AiValidationError::InvalidTopK {
                top_k: 21,
                min: MIN_SEARCH_TOP_K,
                max: MAX_SEARCH_TOP_K,
            }
        );

        // Default topK when omitted in JSON
        let json_omitted_top_k = r#"{"collection": "docs", "query": "hello"}"#;
        let parsed: SearchRequest = serde_json::from_str(json_omitted_top_k).unwrap();
        assert_eq!(parsed.top_k, 5);
        assert!(validate_search_request(&parsed).is_ok());

        // 4. Unknown fields rejected
        let bad_json = r#"{"collection": "docs", "query": "hello", "extra": 42}"#;
        assert!(serde_json::from_str::<SearchRequest>(bad_json).is_err());
    }

    #[test]
    fn test_catalog_model_id_hook() {
        assert!(validate_catalog_model_id("qwen2.5-1.5b-instruct").is_ok());
        assert!(validate_catalog_model_id("bge-small-en-v1.5").is_ok());
        assert!(validate_catalog_model_id("a").is_ok());
        let id_64 = "a".repeat(64);
        assert!(validate_catalog_model_id(&id_64).is_ok());

        let id_65 = "a".repeat(65);
        assert_eq!(
            validate_catalog_model_id(&id_65).unwrap_err(),
            AiValidationError::InvalidCatalogModelId(id_65)
        );

        assert_eq!(
            validate_catalog_model_id("").unwrap_err(),
            AiValidationError::InvalidCatalogModelId("".to_string())
        );

        assert!(validate_catalog_model_id("Qwen2.5").is_err());
        assert!(validate_catalog_model_id("model name").is_err());
        assert!(validate_catalog_model_id("model/id").is_err());
    }

    #[test]
    fn test_tracing_events_never_leak_content_or_secrets() {
        let marker_prompt = "MARKER_PROMPT_SECRET_98765";
        let marker_key = "sk-test-LEAK-MARKER-123";

        let events = vec![
            record_call_started(
                "win-main",
                METHOD_AI_CHAT,
                "local",
                "qwen2.5-1.5b-instruct",
                256,
            ),
            record_call_finished(
                "win-main",
                METHOD_AI_CHAT,
                "local",
                "qwen2.5-1.5b-instruct",
                256,
                128,
                450,
            ),
            record_call_denied("win-main", METHOD_AI_CHAT, "permission_denied"),
            record_call_cancelled("win-main", METHOD_AI_CHAT, 1200),
            record_call_failed(
                "win-main",
                METHOD_AI_CHAT,
                "local",
                "qwen2.5-1.5b-instruct",
                800,
                "engine_exit_1",
            ),
        ];

        for event in events {
            let formatted = event.format_event();

            // Strict invariant: neither the secret key nor the prompt content must appear
            assert!(
                !formatted.contains(marker_prompt),
                "Event string leaked marker prompt: {formatted}"
            );
            assert!(
                !formatted.contains(marker_key),
                "Event string leaked secret key marker: {formatted}"
            );

            // Assert event has required metadata fields
            assert!(formatted.contains("event="));
            assert!(formatted.contains("window_id=win-main"));
            assert!(formatted.contains("method=ai.chat"));

            // Verify no prompt or output payload field exists in the formatted event
            assert!(!formatted.contains("prompt="));
            assert!(!formatted.contains("output="));
            assert!(!formatted.contains("secret="));
            assert!(!formatted.contains("key="));
        }
    }

    struct CountingAiHandler {
        call_count: AtomicUsize,
    }

    impl CountingAiHandler {
        fn new() -> Self {
            Self {
                call_count: AtomicUsize::new(0),
            }
        }

        fn count(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn test_counting_handler_proves_ungranted_ai_methods_never_reach_handler() {
        use crate::{Capabilities, GENERIC_METHOD_ERROR, HandlerRegistry, Request, dispatch};

        let handler = Arc::new(CountingAiHandler::new());
        let mut registry = HandlerRegistry::new();

        let h1 = Arc::clone(&handler);
        registry.register(METHOD_AI_CHAT, move |_| {
            h1.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"status": "ok"}))
        });

        let h2 = Arc::clone(&handler);
        registry.register(METHOD_AI_INDEX, move |_| {
            h2.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"status": "ok"}))
        });

        let h3 = Arc::clone(&handler);
        registry.register(METHOD_AI_SEARCH, move |_| {
            h3.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"status": "ok"}))
        });

        // 1. Default-deny: empty capabilities
        let empty_caps = Capabilities::new();

        for &method in AI_METHOD_NAMES {
            let req = Request {
                id: 10,
                method: method.to_string(),
                params: serde_json::json!({"query": "unauthorized attempt"}),
            };
            let resp = dispatch(&req, &empty_caps, &registry);
            assert!(!resp.ok);
            assert_eq!(resp.error.as_deref(), Some(GENERIC_METHOD_ERROR));
            assert_eq!(
                handler.count(),
                0,
                "Handler was reached for ungranted AI method '{method}'!"
            );
        }

        // 2. Grant each capability one by one and prove handler is called exactly once
        let mut expected = 0;
        for &method in AI_METHOD_NAMES {
            let mut single_cap = Capabilities::new();
            single_cap.grant(method);

            let req = Request {
                id: 20 + expected as u64,
                method: method.to_string(),
                params: serde_json::json!({}),
            };
            let resp = dispatch(&req, &single_cap, &registry);
            assert!(resp.ok, "Granted AI call '{method}' failed unexpectedly");
            expected += 1;
            assert_eq!(
                handler.count(),
                expected,
                "Handler count did not increment for granted AI method '{method}'"
            );
        }
    }
}
