#![forbid(unsafe_code)]

//! AI Chat streaming handler, concurrency management, cancellation, and safe delivery.
//!
//! Enforces:
//! - Strict capability check: caller window must have `ai.chat` granted.
//! - Pure validation from WP2 before calling engine.
//! - Single generation per window concurrency limit (extra requests rejected, not queued).
//! - 120-second wall-clock timeout.
//! - Clamped `maxTokens` to config value and hard ceiling (4,096).
//! - Streamed output capped at 100,000 characters, terminating with `"truncated"` status.
//! - Window-isolated cancellation: only the initiating window can cancel by request ID.
//! - Engine failures mid-stream return failed status and never hang the page.
//! - Double JSON-escaped safe script delivery (Phase 0 safe delivery path).
//! - Zero content or prompt logging: only window ID, method, sizes, duration, and generic reasons.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

use reqwest::blocking::Client;
use wavical_ipc::ChatRequest;
use wavical_ipc::ai::{
    CHAT_WALL_CLOCK_TIMEOUT_SECS, EVENT_AI_CALL_CANCELLED, EVENT_AI_CALL_DENIED,
    EVENT_AI_CALL_FAILED, EVENT_AI_CALL_FINISHED, EVENT_AI_CALL_STARTED,
    MAX_CHAT_MAX_TOKENS_CEILING, MAX_CHAT_STREAMED_OUTPUT_CHARS, METHOD_AI_CHAT,
    validate_chat_request,
};

use crate::channel::EngineChannel;

/// Generic user-facing error message for all AI chat failures.
pub const GENERIC_AI_CHAT_ERROR: &str = "AI chat operation failed";

/// Terminal status for a streamed chat completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatStreamStatus {
    /// Stream completed normally.
    Done,
    /// Stream was cancelled by the originating window.
    Cancelled,
    /// Stream failed due to timeout, engine crash, or network error.
    Failed,
    /// Stream reached the maximum character cap (100,000 chars) and stopped cleanly.
    Truncated,
}

impl ChatStreamStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Truncated => "truncated",
        }
    }
}

/// Errors raised during chat initiation, validation, or cancellation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChatError {
    #[error("concurrency limit exceeded: only one generation allowed per window")]
    ConcurrencyLimit,

    #[error("capability denied: window lacks 'ai.chat' capability")]
    CapabilityDenied,

    #[error("chat request validation failed: {0}")]
    ValidationFailed(String),

    #[error("unknown or unauthorized request id")]
    UnknownOrUnauthorizedRequest,

    #[error("engine unavailable or model not loaded")]
    EngineUnavailable,

    #[error("engine request failed: {0}")]
    EngineError(String),
}

/// Active generation session tracking.
#[derive(Debug)]
pub struct ActiveChatSession {
    pub request_id: u64,
    pub cancel_flag: Arc<AtomicBool>,
    pub start_time: Instant,
}

/// Session manager tracking active chat generations per window.
#[derive(Debug, Default, Clone)]
pub struct ChatSessionManager {
    sessions: Arc<Mutex<HashMap<String, ActiveChatSession>>>,
}

impl ChatSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Attempt to register a new active chat generation for a window.
    ///
    /// Returns `ChatError::ConcurrencyLimit` if the window already has an ongoing generation.
    pub fn start_session(
        &self,
        window_id: &str,
        request_id: u64,
    ) -> Result<Arc<AtomicBool>, ChatError> {
        let mut map = self.sessions.lock().unwrap();

        if map.contains_key(window_id) {
            tracing::warn!(
                event = EVENT_AI_CALL_DENIED,
                window_id = %window_id,
                method = METHOD_AI_CHAT,
                reason = "concurrency limit exceeded: active generation in progress",
                "AI chat request rejected: window already has an active generation"
            );
            return Err(ChatError::ConcurrencyLimit);
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        map.insert(
            window_id.to_string(),
            ActiveChatSession {
                request_id,
                cancel_flag: Arc::clone(&cancel_flag),
                start_time: Instant::now(),
            },
        );

        Ok(cancel_flag)
    }

    /// Cancel an ongoing chat generation for a window by request ID.
    ///
    /// Enforces: only the window that started the request can cancel it.
    /// Forged or non-existent IDs return `ChatError::UnknownOrUnauthorizedRequest`.
    pub fn cancel_session(&self, window_id: &str, request_id: u64) -> Result<(), ChatError> {
        let mut map = self.sessions.lock().unwrap();

        if let Some(session) = map.get(window_id).filter(|s| s.request_id == request_id) {
            session.cancel_flag.store(true, Ordering::SeqCst);
            map.remove(window_id);
            tracing::info!(
                event = EVENT_AI_CALL_CANCELLED,
                window_id = %window_id,
                method = METHOD_AI_CHAT,
                reason = "cancelled by window request",
                "AI chat generation successfully cancelled"
            );
            return Ok(());
        }

        tracing::warn!(
            event = EVENT_AI_CALL_DENIED,
            window_id = %window_id,
            method = METHOD_AI_CHAT,
            reason = "unknown or unauthorized request id for cancellation",
            "AI chat cancel request rejected: unknown or forged request id"
        );
        Err(ChatError::UnknownOrUnauthorizedRequest)
    }

    /// Complete an active session for a window if request ID matches.
    pub fn finish_session(&self, window_id: &str, request_id: u64) {
        let mut map = self.sessions.lock().unwrap();
        if map.get(window_id).map(|s| s.request_id) == Some(request_id) {
            map.remove(window_id);
        }
    }

    /// Check whether a window currently has an active chat generation.
    pub fn has_active_session(&self, window_id: &str) -> bool {
        let map = self.sessions.lock().unwrap();
        map.contains_key(window_id)
    }
}

static GLOBAL_CHAT_SESSIONS: std::sync::OnceLock<ChatSessionManager> = std::sync::OnceLock::new();

/// Return a reference to the global chat session manager singleton.
pub fn global_chat_sessions() -> &'static ChatSessionManager {
    GLOBAL_CHAT_SESSIONS.get_or_init(ChatSessionManager::new)
}

/// Clamp requested `maxTokens` against configured limits and the hard ceiling (4,096).
pub fn clamp_max_tokens(requested: Option<u32>, config_default: u32) -> u32 {
    let base = requested.unwrap_or(config_default);
    base.min(config_default).min(MAX_CHAT_MAX_TOKENS_CEILING)
}

/// Compute total character count of a chat request's messages.
pub fn compute_input_chars(request: &ChatRequest) -> usize {
    request
        .messages
        .iter()
        .map(|m| m.content.chars().count())
        .sum()
}

/// Format safe JavaScript evaluation code to deliver an incremental chunk to the page.
///
/// Uses double JSON serialization to produce a strictly escaped JS string literal passed
/// to `JSON.parse(...)`, preventing any script injection, markup execution, or UTF-8 breakage.
pub fn build_chat_chunk_script(id: u64, chunk: &str) -> Result<String, serde_json::Error> {
    let payload = serde_json::json!({
        "id": id,
        "ok": true,
        "chunk": chunk,
    });
    let json_str = serde_json::to_string(&payload)?;
    let escaped_js_literal = serde_json::to_string(&json_str)?;
    let safe_js_literal = escaped_js_literal
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    Ok(format!(
        "window.__wavical_receive__(JSON.parse({safe_js_literal}));"
    ))
}

/// Format safe JavaScript evaluation code to deliver the final completion message to the page.
pub fn build_chat_final_script(
    id: u64,
    status: ChatStreamStatus,
    generic_error: Option<&str>,
) -> Result<String, serde_json::Error> {
    let is_ok = status == ChatStreamStatus::Done || status == ChatStreamStatus::Truncated;
    let status_str = status.as_str();
    let payload = serde_json::json!({
        "id": id,
        "ok": is_ok,
        "status": status_str,
        "result": {
            "status": status_str,
        },
        "error": generic_error,
    });
    let json_str = serde_json::to_string(&payload)?;
    let escaped_js_literal = serde_json::to_string(&json_str)?;
    let safe_js_literal = escaped_js_literal
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    Ok(format!(
        "window.__wavical_receive__(JSON.parse({safe_js_literal}));"
    ))
}

/// SSE parse result for a line of text.
#[derive(Debug, PartialEq, Eq)]
pub enum SseEvent {
    /// Incremental content delta from the model.
    Chunk(String),
    /// SSE stream termination marker `[DONE]`.
    Done,
    /// Keepalive ping or empty whitespace line.
    Empty,
}

/// Parse a single Server-Sent Events (SSE) line from llama-server's `/v1/chat/completions`.
pub fn parse_sse_line(line: &str) -> SseEvent {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return SseEvent::Empty;
    }

    if let Some(data) = trimmed.strip_prefix("data:") {
        let payload = data.trim();
        if payload == "[DONE]" {
            return SseEvent::Done;
        }

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(payload) {
            // Standard OpenAI-compatible SSE delta or legacy completion format
            let chunk_opt = val
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|choices| choices.first())
                .and_then(|first| {
                    first
                        .get("delta")
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                        .or_else(|| first.get("text").and_then(|t| t.as_str()))
                });

            if let Some(text) = chunk_opt {
                return SseEvent::Chunk(text.to_string());
            }
        }
    }

    SseEvent::Empty
}

/// Stream execution parameters.
#[derive(Debug, Clone)]
pub struct StreamChatConfig {
    pub wall_clock_timeout: Duration,
    pub max_streamed_chars: usize,
    pub config_max_tokens: u32,
}

impl Default for StreamChatConfig {
    fn default() -> Self {
        Self {
            wall_clock_timeout: Duration::from_secs(CHAT_WALL_CLOCK_TIMEOUT_SECS),
            max_streamed_chars: MAX_CHAT_STREAMED_OUTPUT_CHARS,
            config_max_tokens: 1_024,
        }
    }
}

/// Execute a streaming chat completion against the engine.
///
/// Handles:
/// - Validation check.
/// - Clamping `maxTokens`.
/// - SSE stream reading.
/// - Incremental chunk delivery callback.
/// - Cancellation flag polling.
/// - 120s wall-clock timeout enforcement.
/// - Streamed output character cap (100,000 chars) resulting in `"truncated"`.
/// - Engine failures mid-stream returning `"failed"` status.
#[allow(clippy::too_many_arguments)]
pub fn execute_chat_stream<F, G>(
    client: &Client,
    channel: &EngineChannel,
    window_id: &str,
    request_id: u64,
    model_id: &str,
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

    tracing::info!(
        event = EVENT_AI_CALL_STARTED,
        window_id = %window_id,
        request_id = request_id,
        method = METHOD_AI_CHAT,
        backend = "local",
        model_id = %model_id,
        input_size_chars = input_chars,
        "AI chat call started"
    );

    // 1. Pure validation from WP2
    if let Err(err) = validate_chat_request(request) {
        tracing::warn!(
            event = EVENT_AI_CALL_DENIED,
            window_id = %window_id,
            method = METHOD_AI_CHAT,
            reason = %err,
            "AI chat request validation failed"
        );
        on_final(
            ChatStreamStatus::Failed,
            Some(GENERIC_AI_CHAT_ERROR.to_string()),
        );
        return;
    }

    // 2. Early cancellation check before connecting
    if cancel_flag.load(Ordering::SeqCst) {
        tracing::info!(
            event = EVENT_AI_CALL_CANCELLED,
            window_id = %window_id,
            method = METHOD_AI_CHAT,
            backend = "local",
            model_id = %model_id,
            duration_ms = start_time.elapsed().as_millis() as u64,
            reason = "cancelled before first token",
            "AI chat call cancelled before connection"
        );
        on_final(
            ChatStreamStatus::Cancelled,
            Some("Operation cancelled".to_string()),
        );
        return;
    }

    // 3. Clamp max_tokens
    let effective_max_tokens =
        clamp_max_tokens(request.max_tokens, stream_config.config_max_tokens);

    // 4. Build engine request payload
    let engine_payload = serde_json::json!({
        "messages": request.messages,
        "max_tokens": effective_max_tokens,
        "temperature": request.temperature.unwrap_or(0.7),
        "stream": true,
    });

    let completions_url = format!("{}/v1/chat/completions", channel.base_url());
    let response_result = client
        .post(&completions_url)
        .header("Authorization", channel.auth_header_value())
        .header("Accept", "text/event-stream")
        .json(&engine_payload)
        .send();

    let response = match response_result {
        Ok(r) => {
            if !r.status().is_success() {
                tracing::warn!(
                    event = EVENT_AI_CALL_FAILED,
                    window_id = %window_id,
                    method = METHOD_AI_CHAT,
                    backend = "local",
                    model_id = %model_id,
                    status_code = %r.status(),
                    duration_ms = start_time.elapsed().as_millis() as u64,
                    reason = "engine returned non-success HTTP status",
                    "AI engine chat request failed"
                );
                on_final(
                    ChatStreamStatus::Failed,
                    Some(GENERIC_AI_CHAT_ERROR.to_string()),
                );
                return;
            }
            r
        }
        Err(err) => {
            tracing::warn!(
                event = EVENT_AI_CALL_FAILED,
                window_id = %window_id,
                method = METHOD_AI_CHAT,
                backend = "local",
                model_id = %model_id,
                error_detail = %err,
                duration_ms = start_time.elapsed().as_millis() as u64,
                reason = "network error contacting local engine",
                "AI engine connection failed"
            );
            on_final(
                ChatStreamStatus::Failed,
                Some(GENERIC_AI_CHAT_ERROR.to_string()),
            );
            return;
        }
    };

    // 5. Read SSE stream incrementally
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
                backend = "local",
                model_id = %model_id,
                duration_ms = start_time.elapsed().as_millis() as u64,
                reason = "cancelled mid-stream",
                "AI chat call cancelled by user"
            );
            on_final(
                ChatStreamStatus::Cancelled,
                Some("Operation cancelled".to_string()),
            );
            return;
        }

        // B. Check wall-clock timeout
        if start_time.elapsed() >= stream_config.wall_clock_timeout {
            tracing::warn!(
                event = EVENT_AI_CALL_FAILED,
                window_id = %window_id,
                method = METHOD_AI_CHAT,
                backend = "local",
                model_id = %model_id,
                duration_ms = start_time.elapsed().as_millis() as u64,
                reason = "wall-clock timeout exceeded",
                "AI chat call timed out"
            );
            on_final(
                ChatStreamStatus::Failed,
                Some(GENERIC_AI_CHAT_ERROR.to_string()),
            );
            return;
        }

        // C. Read line with safe UTF-8 decoding
        line_buf.clear();
        match reader.read_until(b'\n', &mut line_buf) {
            Ok(0) => {
                // EOF reached cleanly
                break;
            }
            Ok(_) => {
                let line_str = String::from_utf8_lossy(&line_buf);
                match parse_sse_line(&line_str) {
                    SseEvent::Done => {
                        // Normal completion
                        tracing::info!(
                            event = EVENT_AI_CALL_FINISHED,
                            window_id = %window_id,
                            method = METHOD_AI_CHAT,
                            backend = "local",
                            model_id = %model_id,
                            output_size_chars = accumulated_chars,
                            duration_ms = start_time.elapsed().as_millis() as u64,
                            reason = "done",
                            "AI chat completion finished normally"
                        );
                        on_final(ChatStreamStatus::Done, None);
                        return;
                    }
                    SseEvent::Chunk(chunk_text) => {
                        let chunk_char_count = chunk_text.chars().count();
                        if accumulated_chars + chunk_char_count >= stream_config.max_streamed_chars
                        {
                            // Cap reached: deliver prefix up to limit, then truncate
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
                                backend = "local",
                                model_id = %model_id,
                                output_size_chars = accumulated_chars,
                                duration_ms = start_time.elapsed().as_millis() as u64,
                                reason = "truncated",
                                "AI chat stream output reached maximum character cap"
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
            Err(err) => {
                tracing::warn!(
                    event = EVENT_AI_CALL_FAILED,
                    window_id = %window_id,
                    method = METHOD_AI_CHAT,
                    backend = "local",
                    model_id = %model_id,
                    error_detail = %err,
                    duration_ms = start_time.elapsed().as_millis() as u64,
                    reason = "engine stream read error mid-stream",
                    "AI chat stream abruptly interrupted"
                );
                on_final(
                    ChatStreamStatus::Failed,
                    Some(GENERIC_AI_CHAT_ERROR.to_string()),
                );
                return;
            }
        }
    }

    // Reached EOF without [DONE] marker
    tracing::info!(
        event = EVENT_AI_CALL_FINISHED,
        window_id = %window_id,
        method = METHOD_AI_CHAT,
        backend = "local",
        model_id = %model_id,
        output_size_chars = accumulated_chars,
        duration_ms = start_time.elapsed().as_millis() as u64,
        reason = "done",
        "AI chat stream closed by server"
    );
    on_final(ChatStreamStatus::Done, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_max_tokens() {
        // Hard ceiling is 4096
        assert_eq!(clamp_max_tokens(Some(10_000), 1024), 1024);
        assert_eq!(clamp_max_tokens(Some(500), 1024), 500);
        assert_eq!(clamp_max_tokens(None, 1024), 1024);
        assert_eq!(clamp_max_tokens(Some(5000), 8000), 4096);
        assert_eq!(clamp_max_tokens(None, 8000), 4096);
    }

    #[test]
    fn test_parse_sse_line() {
        assert_eq!(parse_sse_line(""), SseEvent::Empty);
        assert_eq!(parse_sse_line(": keepalive"), SseEvent::Empty);
        assert_eq!(parse_sse_line("data: [DONE]"), SseEvent::Done);

        let standard_chunk = r#"data: {"choices":[{"delta":{"content":"Hello world!"}}]}"#;
        assert_eq!(
            parse_sse_line(standard_chunk),
            SseEvent::Chunk("Hello world!".to_string())
        );

        let legacy_chunk = r#"data: {"choices":[{"text":"Legacy chunk"}]}"#;
        assert_eq!(
            parse_sse_line(legacy_chunk),
            SseEvent::Chunk("Legacy chunk".to_string())
        );
    }

    #[test]
    fn test_session_manager_concurrency_and_cancel() {
        let manager = ChatSessionManager::new();

        // 1. Start session for window1
        let cancel1 = manager
            .start_session("win1", 101)
            .expect("win1 session starts");
        assert!(manager.has_active_session("win1"));

        // 2. Second request from win1 rejected (concurrency limit)
        let err = manager.start_session("win1", 102).unwrap_err();
        assert_eq!(err, ChatError::ConcurrencyLimit);

        // 3. Different window can start session concurrently
        let cancel2 = manager
            .start_session("win2", 201)
            .expect("win2 session starts");
        assert!(manager.has_active_session("win2"));

        // 4. Cancel with wrong window ID fails
        let forged_err = manager.cancel_session("win1", 201).unwrap_err();
        assert_eq!(forged_err, ChatError::UnknownOrUnauthorizedRequest);
        assert!(!cancel2.load(Ordering::SeqCst));

        // 5. Cancel with unknown request ID fails
        let unknown_err = manager.cancel_session("win1", 999).unwrap_err();
        assert_eq!(unknown_err, ChatError::UnknownOrUnauthorizedRequest);

        // 6. Legitimate cancel succeeds
        manager
            .cancel_session("win1", 101)
            .expect("legitimate cancel succeeds");
        assert!(cancel1.load(Ordering::SeqCst));
        assert!(!manager.has_active_session("win1"));

        // 7. After finish/cancel, new request can start
        let cancel3 = manager
            .start_session("win1", 103)
            .expect("new session starts");
        assert!(!cancel3.load(Ordering::SeqCst));

        // 8. Cancel after finish fails
        manager.finish_session("win1", 103);
        let finished_err = manager.cancel_session("win1", 103).unwrap_err();
        assert_eq!(finished_err, ChatError::UnknownOrUnauthorizedRequest);
    }

    #[test]
    fn test_safe_script_generation_escaping() {
        // Test dangerous payloads: </script>, HTML, quotes, U+2028 line separator
        let dangerous_payloads = [
            "</script><script>alert(1)</script>",
            r#"<img src=x onerror="alert('xss')">"#,
            "Line 1\u{2028}Line 2\u{2029}Line 3",
            "Quotes: \"hello\" 'world' `backtick` $dollar",
        ];

        for &payload in &dangerous_payloads {
            let chunk_script = build_chat_chunk_script(42, payload).expect("must serialize");
            assert!(
                chunk_script.starts_with("window.__wavical_receive__(JSON.parse(\""),
                "script must strictly use window.__wavical_receive__(JSON.parse(...))"
            );
            assert!(
                !chunk_script.contains("</script>"),
                "script must never contain raw unescaped </script>"
            );

            let final_script =
                build_chat_final_script(42, ChatStreamStatus::Done, None).expect("must serialize");
            assert!(final_script.contains("done"));

            // Verify that stripping the JS wrapper and parsing JSON recovers the exact original payload
            let prefix = "window.__wavical_receive__(JSON.parse(";
            let suffix = "));";
            assert!(chunk_script.starts_with(prefix));
            assert!(chunk_script.ends_with(suffix));
            let js_literal = &chunk_script[prefix.len()..chunk_script.len() - suffix.len()];
            let inner_json: String =
                serde_json::from_str(js_literal).expect("must parse JS literal");
            let parsed_obj: serde_json::Value =
                serde_json::from_str(&inner_json).expect("must parse payload JSON");
            assert_eq!(parsed_obj["chunk"], payload, "payload must match exactly");
        }
    }
}
