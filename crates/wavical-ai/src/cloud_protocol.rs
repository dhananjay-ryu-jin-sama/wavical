#![forbid(unsafe_code)]

//! Cloud OpenAI-compatible chat completion protocol handler (Phase 3 WP7).
//!
//! Enforces:
//! - Pure request validation from WP2 before calling cloud.
//! - Request body bounded to at most 1 MB (`MAX_CLOUD_REQUEST_BODY_BYTES`).
//! - Non-streamed response bounded to at most 4 MB (`MAX_CLOUD_RESPONSE_BODY_BYTES`).
//! - Streamed response output cap bounded to 100,000 characters (`MAX_CHAT_STREAMED_OUTPUT_CHARS`).
//! - 120-second wall-clock timeout.
//! - Cancellation polling.
//! - Redirect refusal (fails closed if 3xx received).
//! - All provider errors mapped to generic user-facing error (`"AI chat operation failed"`).
//! - Logging includes ONLY sizes, host, status code, duration, IDs; NEVER prompt text, output text, or API key.

use reqwest::blocking::Client;
use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use thiserror::Error;

use wavical_ipc::ChatRequest;
use wavical_ipc::ai::{
    EVENT_AI_CALL_CANCELLED, EVENT_AI_CALL_FAILED, EVENT_AI_CALL_FINISHED, EVENT_AI_CALL_STARTED,
    MAX_CLOUD_REQUEST_BODY_BYTES, MAX_CLOUD_RESPONSE_BODY_BYTES, METHOD_AI_CHAT,
    validate_chat_request,
};

use crate::chat::{
    ChatStreamStatus, GENERIC_AI_CHAT_ERROR, SseEvent, StreamChatConfig, clamp_max_tokens,
    compute_input_chars, parse_sse_line,
};
use crate::cloud_client::{
    CloudClientError, build_target_cloud_url, validate_and_extract_cloud_host,
};
use crate::credentials::CloudApiKey;

/// Errors raised during cloud chat execution.
#[derive(Debug, Error)]
pub enum CloudProtocolError {
    #[error("cloud client error: {0}")]
    Client(#[from] CloudClientError),

    #[error("chat request validation failed: {0}")]
    Validation(String),

    #[error("request body exceeds limit of 1 MB: {0} bytes")]
    RequestBodyTooLarge(usize),

    #[error("response body exceeds limit of 4 MB: {0} bytes")]
    ResponseBodyTooLarge(usize),

    #[error("redirect refused")]
    RedirectRefused,

    #[error("provider returned error status: {0}")]
    ProviderError(u16),

    #[error("network transport error: {0}")]
    Transport(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("wall-clock timeout exceeded")]
    Timeout,
}

/// Execute a streaming chat completion against a cloud provider endpoint.
#[allow(clippy::too_many_arguments)]
pub fn execute_cloud_chat_stream<F, G>(
    client: &Client,
    base_url: &str,
    provider_model: &str,
    api_key: &CloudApiKey,
    window_id: &str,
    request_id: u64,
    request: &ChatRequest,
    stream_config: &StreamChatConfig,
    cancel_flag: &Arc<AtomicBool>,
    mut on_chunk: F,
    on_final: G,
) where
    F: FnMut(&str),
    G: FnOnce(ChatStreamStatus, Option<String>),
{
    let start_time = Instant::now();
    let input_chars = compute_input_chars(request);

    // 1. Validate host and base URL
    let host = match validate_and_extract_cloud_host(base_url) {
        Ok(h) => h,
        Err(_err) => {
            tracing::warn!(
                event = EVENT_AI_CALL_FAILED,
                window_id = %window_id,
                method = METHOD_AI_CHAT,
                backend = "cloud",
                error_code = "invalid_base_url",
                duration_ms = start_time.elapsed().as_millis() as u64,
                "Cloud base URL validation failed"
            );
            on_final(
                ChatStreamStatus::Failed,
                Some(GENERIC_AI_CHAT_ERROR.to_string()),
            );
            return;
        }
    };

    let target_url = match build_target_cloud_url(base_url, "chat/completions") {
        Ok(u) => u,
        Err(_) => {
            tracing::warn!(
                event = EVENT_AI_CALL_FAILED,
                window_id = %window_id,
                method = METHOD_AI_CHAT,
                backend = "cloud",
                host = %host,
                error_code = "invalid_endpoint_url",
                duration_ms = start_time.elapsed().as_millis() as u64,
                "Cloud endpoint URL construction failed"
            );
            on_final(
                ChatStreamStatus::Failed,
                Some(GENERIC_AI_CHAT_ERROR.to_string()),
            );
            return;
        }
    };

    tracing::info!(
        event = EVENT_AI_CALL_STARTED,
        window_id = %window_id,
        request_id = request_id,
        method = METHOD_AI_CHAT,
        backend = "cloud",
        host = %host,
        model_id = %provider_model,
        input_size_chars = input_chars,
        "Cloud AI chat call started"
    );

    // 2. Pure validation from WP2
    if let Err(err) = validate_chat_request(request) {
        tracing::warn!(
            event = EVENT_AI_CALL_FAILED,
            window_id = %window_id,
            method = METHOD_AI_CHAT,
            backend = "cloud",
            host = %host,
            model_id = %provider_model,
            duration_ms = start_time.elapsed().as_millis() as u64,
            reason = "chat request validation failed",
            "Chat request failed schema validation"
        );
        on_final(
            ChatStreamStatus::Failed,
            Some(format!("Validation failed: {err}")),
        );
        return;
    }

    // 3. Early cancellation check
    if cancel_flag.load(Ordering::SeqCst) {
        tracing::info!(
            event = EVENT_AI_CALL_CANCELLED,
            window_id = %window_id,
            method = METHOD_AI_CHAT,
            backend = "cloud",
            host = %host,
            model_id = %provider_model,
            duration_ms = start_time.elapsed().as_millis() as u64,
            reason = "cancelled before first token",
            "Cloud AI chat call cancelled before connection"
        );
        on_final(
            ChatStreamStatus::Cancelled,
            Some("Operation cancelled".to_string()),
        );
        return;
    }

    // 4. Build payload and enforce <= 1 MB request body
    let effective_max_tokens =
        clamp_max_tokens(request.max_tokens, stream_config.config_max_tokens);

    let cloud_payload = serde_json::json!({
        "model": provider_model,
        "messages": request.messages,
        "max_tokens": effective_max_tokens,
        "temperature": request.temperature.unwrap_or(0.7),
        "stream": true,
    });

    let body_bytes = match serde_json::to_vec(&cloud_payload) {
        Ok(b) => b,
        Err(_) => {
            tracing::warn!(
                event = EVENT_AI_CALL_FAILED,
                window_id = %window_id,
                backend = "cloud",
                host = %host,
                error_code = "serialization_failed",
                "Cloud request body serialization failed"
            );
            on_final(
                ChatStreamStatus::Failed,
                Some(GENERIC_AI_CHAT_ERROR.to_string()),
            );
            return;
        }
    };

    if body_bytes.len() > MAX_CLOUD_REQUEST_BODY_BYTES {
        tracing::warn!(
            event = EVENT_AI_CALL_FAILED,
            window_id = %window_id,
            backend = "cloud",
            host = %host,
            body_bytes = body_bytes.len(),
            limit_bytes = MAX_CLOUD_REQUEST_BODY_BYTES,
            error_code = "request_body_too_large",
            "Cloud request body exceeds 1 MB limit"
        );
        on_final(
            ChatStreamStatus::Failed,
            Some(GENERIC_AI_CHAT_ERROR.to_string()),
        );
        return;
    }

    // 5. Send HTTP request
    let response_result = client
        .post(&target_url)
        .header(
            "Authorization",
            format!("Bearer {}", api_key.expose_secret()),
        )
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .body(body_bytes)
        .send();

    let response = match response_result {
        Ok(r) => {
            // Check for redirect refusal
            if r.status().is_redirection() {
                tracing::warn!(
                    event = EVENT_AI_CALL_FAILED,
                    window_id = %window_id,
                    backend = "cloud",
                    host = %host,
                    status_code = %r.status().as_u16(),
                    error_code = "redirect_refused",
                    duration_ms = start_time.elapsed().as_millis() as u64,
                    "Cloud request attempted HTTP redirect; redirects are strictly refused"
                );
                on_final(
                    ChatStreamStatus::Failed,
                    Some(GENERIC_AI_CHAT_ERROR.to_string()),
                );
                return;
            }

            // Check for provider error status
            if !r.status().is_success() {
                tracing::warn!(
                    event = EVENT_AI_CALL_FAILED,
                    window_id = %window_id,
                    backend = "cloud",
                    host = %host,
                    status_code = %r.status().as_u16(),
                    error_code = "provider_error",
                    duration_ms = start_time.elapsed().as_millis() as u64,
                    "Cloud provider returned error status"
                );
                on_final(
                    ChatStreamStatus::Failed,
                    Some(GENERIC_AI_CHAT_ERROR.to_string()),
                );
                return;
            }
            r
        }
        Err(_err) => {
            tracing::warn!(
                event = EVENT_AI_CALL_FAILED,
                window_id = %window_id,
                backend = "cloud",
                host = %host,
                duration_ms = start_time.elapsed().as_millis() as u64,
                error_code = "transport_error",
                "Cloud network transport error"
            );
            on_final(
                ChatStreamStatus::Failed,
                Some(GENERIC_AI_CHAT_ERROR.to_string()),
            );
            return;
        }
    };

    // 6. Stream reading loop
    let mut reader = BufReader::new(response);
    let mut accumulated_chars: usize = 0;
    let mut line_buf = Vec::new();

    loop {
        // A. Check cancellation
        if cancel_flag.load(Ordering::SeqCst) {
            tracing::info!(
                event = EVENT_AI_CALL_CANCELLED,
                window_id = %window_id,
                method = METHOD_AI_CHAT,
                backend = "cloud",
                host = %host,
                model_id = %provider_model,
                duration_ms = start_time.elapsed().as_millis() as u64,
                reason = "cancelled mid-stream",
                "Cloud AI chat call cancelled by user"
            );
            on_final(
                ChatStreamStatus::Cancelled,
                Some("Operation cancelled".to_string()),
            );
            return;
        }

        // B. Check wall-clock timeout (120 s)
        if start_time.elapsed() >= stream_config.wall_clock_timeout {
            tracing::warn!(
                event = EVENT_AI_CALL_FAILED,
                window_id = %window_id,
                method = METHOD_AI_CHAT,
                backend = "cloud",
                host = %host,
                model_id = %provider_model,
                duration_ms = start_time.elapsed().as_millis() as u64,
                error_code = "timeout",
                "Cloud AI chat call timed out"
            );
            on_final(
                ChatStreamStatus::Failed,
                Some(GENERIC_AI_CHAT_ERROR.to_string()),
            );
            return;
        }

        // C. Read line
        line_buf.clear();
        match reader.read_until(b'\n', &mut line_buf) {
            Ok(0) => {
                // EOF reached
                break;
            }
            Ok(_) => {
                let line_str = String::from_utf8_lossy(&line_buf);
                match parse_sse_line(&line_str) {
                    SseEvent::Done => {
                        tracing::info!(
                            event = EVENT_AI_CALL_FINISHED,
                            window_id = %window_id,
                            method = METHOD_AI_CHAT,
                            backend = "cloud",
                            host = %host,
                            model_id = %provider_model,
                            output_size_chars = accumulated_chars,
                            duration_ms = start_time.elapsed().as_millis() as u64,
                            reason = "done",
                            "Cloud AI chat completion finished normally"
                        );
                        on_final(ChatStreamStatus::Done, None);
                        return;
                    }
                    SseEvent::Chunk(chunk_text) => {
                        let chunk_char_count = chunk_text.chars().count();
                        if accumulated_chars + chunk_char_count >= stream_config.max_streamed_chars
                        {
                            let allowed = stream_config
                                .max_streamed_chars
                                .saturating_sub(accumulated_chars);
                            if allowed > 0 {
                                let clamped_chunk: String =
                                    chunk_text.chars().take(allowed).collect();
                                on_chunk(&clamped_chunk);
                                accumulated_chars += allowed;
                            }

                            tracing::info!(
                                event = EVENT_AI_CALL_FINISHED,
                                window_id = %window_id,
                                method = METHOD_AI_CHAT,
                                backend = "cloud",
                                host = %host,
                                model_id = %provider_model,
                                output_size_chars = accumulated_chars,
                                duration_ms = start_time.elapsed().as_millis() as u64,
                                reason = "truncated",
                                "Cloud AI chat stream output reached maximum character cap"
                            );
                            on_final(ChatStreamStatus::Truncated, None);
                            return;
                        }

                        on_chunk(&chunk_text);
                        accumulated_chars += chunk_char_count;
                    }
                    SseEvent::Empty => {}
                }
            }
            Err(_) => {
                tracing::warn!(
                    event = EVENT_AI_CALL_FAILED,
                    window_id = %window_id,
                    method = METHOD_AI_CHAT,
                    backend = "cloud",
                    host = %host,
                    model_id = %provider_model,
                    duration_ms = start_time.elapsed().as_millis() as u64,
                    error_code = "stream_read_error",
                    "Cloud AI chat stream read error"
                );
                on_final(
                    ChatStreamStatus::Failed,
                    Some(GENERIC_AI_CHAT_ERROR.to_string()),
                );
                return;
            }
        }
    }

    // Finished stream
    tracing::info!(
        event = EVENT_AI_CALL_FINISHED,
        window_id = %window_id,
        method = METHOD_AI_CHAT,
        backend = "cloud",
        host = %host,
        model_id = %provider_model,
        output_size_chars = accumulated_chars,
        duration_ms = start_time.elapsed().as_millis() as u64,
        reason = "stream_eof",
        "Cloud AI chat stream ended on EOF"
    );
    on_final(ChatStreamStatus::Done, None);
}

/// Execute a non-streamed chat completion against a cloud endpoint, bounded by 4 MB response limit.
pub fn execute_cloud_chat_non_streamed(
    client: &Client,
    base_url: &str,
    provider_model: &str,
    api_key: &CloudApiKey,
    request: &ChatRequest,
) -> Result<String, CloudProtocolError> {
    let _host = validate_and_extract_cloud_host(base_url)?;
    let target_url = build_target_cloud_url(base_url, "chat/completions")?;

    validate_chat_request(request).map_err(|e| CloudProtocolError::Validation(e.to_string()))?;

    let payload = serde_json::json!({
        "model": provider_model,
        "messages": request.messages,
        "temperature": request.temperature.unwrap_or(0.7),
        "stream": false,
    });

    let body_bytes =
        serde_json::to_vec(&payload).map_err(|e| CloudProtocolError::Transport(e.to_string()))?;

    if body_bytes.len() > MAX_CLOUD_REQUEST_BODY_BYTES {
        return Err(CloudProtocolError::RequestBodyTooLarge(body_bytes.len()));
    }

    let response = client
        .post(&target_url)
        .header(
            "Authorization",
            format!("Bearer {}", api_key.expose_secret()),
        )
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .map_err(|e| CloudProtocolError::Transport(e.to_string()))?;

    if response.status().is_redirection() {
        return Err(CloudProtocolError::RedirectRefused);
    }

    if !response.status().is_success() {
        return Err(CloudProtocolError::ProviderError(
            response.status().as_u16(),
        ));
    }

    // Bounded read of response up to 4 MB + 1 byte
    let mut reader = response.take((MAX_CLOUD_RESPONSE_BODY_BYTES + 1) as u64);
    let mut resp_bytes = Vec::new();
    reader
        .read_to_end(&mut resp_bytes)
        .map_err(|e| CloudProtocolError::Transport(e.to_string()))?;

    if resp_bytes.len() > MAX_CLOUD_RESPONSE_BODY_BYTES {
        return Err(CloudProtocolError::ResponseBodyTooLarge(resp_bytes.len()));
    }

    let val: serde_json::Value = serde_json::from_slice(&resp_bytes)
        .map_err(|e| CloudProtocolError::Transport(e.to_string()))?;

    let content = val
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|f| f.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    Ok(content.to_string())
}
