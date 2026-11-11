#![forbid(unsafe_code)]

//! Pure validators, frame serializers, frame deserializers, and buffers
//! for the backend IPC channel (Wavical Phase 4 WP5).
//!
//! Enforces:
//! - Exact method allowlist matching against `backend.methods` (no case folding, no trimming, no prefixes).
//! - 1 MB request params size ceiling (`MAX_BACKEND_REQUEST_PARAMS_BYTES`).
//! - 4 MB maximum response and frame size ceiling (`MAX_BACKEND_RESPONSE_BYTES`).
//! - JSON nesting depth limit (`MAX_JSON_DEPTH` = 32).
//! - `#[serde(deny_unknown_fields)]` on frame envelopes.
//! - Safe frame buffering with overflow prevention and 4-byte big-endian framing.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use crate::{
    MAX_BACKEND_CONCURRENT_REQUESTS_PER_WINDOW, MAX_BACKEND_DECLARED_METHODS,
    MAX_BACKEND_LOG_LINE_BYTES, MAX_BACKEND_LOG_RATE_PER_SEC, MAX_BACKEND_REQUEST_PARAMS_BYTES,
    MAX_BACKEND_RESPONSE_BYTES, MAX_BACKEND_RESTART_RETRIES, MAX_BACKEND_TIMEOUT_SECS,
};
use crate::{
    MAX_JSON_DEPTH, MAX_METHOD_NAME_LENGTH, MAX_REQUEST_ID, check_nesting_depth, check_value_depth,
};

/// Maximum allowable capacity for the frame accumulation buffer (4 MB + 4-byte length prefix).
pub const MAX_FRAME_BUFFER_BYTES: usize = MAX_BACKEND_RESPONSE_BYTES + 4;

/// Errors arising during backend frame serialization, framing, or parsing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BackendFrameError {
    #[error("Frame declared length {0} exceeds maximum limit of {1} bytes")]
    FrameTooLarge(usize, usize),

    #[error("Buffer size {0} exceeds maximum allowed frame buffer limit of {1} bytes")]
    BufferOverflow(usize, usize),

    #[error("JSON nesting depth exceeds maximum depth of {0}")]
    JsonDepthExceeded(usize),

    #[error("Malformed JSON or frame envelope: {0}")]
    MalformedEnvelope(String),

    #[error("Invalid request ID {0}: exceeds MAX_REQUEST_ID")]
    InvalidRequestId(u64),

    #[error("Invalid method name in frame")]
    InvalidMethodName,

    #[error("Response result size {0} exceeds limit of {1} bytes")]
    ResultTooLarge(usize, usize),
}

/// Errors arising during validation of `backend.invoke` parameters.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BackendValidationError {
    #[error("Capability 'backend.invoke' not granted")]
    CapabilityDenied,

    #[error("Method '{0}' is not permitted by backend.methods allowlist")]
    MethodNotAllowed(String),

    #[error("Backend request params size {0} exceeds limit of {1} bytes")]
    ParamsTooLarge(usize, usize),

    #[error("JSON nesting depth exceeds maximum depth of {0}")]
    JsonDepthExceeded(usize),

    #[error("Malformed invoke payload: {0}")]
    MalformedPayload(String),

    #[error("Window '{0}' has reached concurrent backend request limit ({1})")]
    ConcurrencyLimitReached(String, usize),
}

/// Incoming invocation parameters passed to `backend.invoke({ method, params })`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendInvokeParams {
    /// Exact target method name declared in `backend.methods`.
    pub method: String,

    /// Parameters passed to the backend handler (defaults to null if omitted).
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Request frame sent across the dedicated IPC Named Pipe from host shell to backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendRequestFrame {
    /// Unique invocation identifier.
    pub id: u64,

    /// Target backend handler method name.
    pub method: String,

    /// Handler arguments.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Response frame sent across the dedicated IPC Named Pipe from backend to host shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendResponseFrame {
    /// Matching invocation identifier.
    pub id: u64,

    /// Whether backend handler execution succeeded.
    pub ok: bool,

    /// Handler return value if successful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,

    /// Handler failure explanation if execution failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Validates that a requested backend method EXACTLY matches an entry in `backend.methods`.
///
/// Security rules:
/// - Exact string equality only.
/// - No case-folding (e.g., `Hello.Greet` must NOT match `hello.greet`).
/// - No trimming (e.g., `hello.greet ` must NOT match `hello.greet`).
/// - No prefix matching (e.g., `hello` or `hello.greet.x` must NOT match `hello.greet`).
pub fn validate_backend_invoke_method(
    method: &str,
    allowlist: &[String],
) -> Result<(), BackendValidationError> {
    if method.is_empty() || method.len() > MAX_METHOD_NAME_LENGTH {
        return Err(BackendValidationError::MethodNotAllowed(method.to_string()));
    }

    if allowlist.iter().any(|allowed| allowed == method) {
        Ok(())
    } else {
        Err(BackendValidationError::MethodNotAllowed(method.to_string()))
    }
}

/// Validates invocation params against the 1 MB size limit and 32-level JSON depth limit.
pub fn validate_backend_invoke_params(
    params: &serde_json::Value,
) -> Result<(), BackendValidationError> {
    let serialized = serde_json::to_vec(params)
        .map_err(|e| BackendValidationError::MalformedPayload(e.to_string()))?;

    if serialized.len() > MAX_BACKEND_REQUEST_PARAMS_BYTES {
        return Err(BackendValidationError::ParamsTooLarge(
            serialized.len(),
            MAX_BACKEND_REQUEST_PARAMS_BYTES,
        ));
    }

    check_value_depth(params, 1, MAX_JSON_DEPTH)
        .map_err(|_| BackendValidationError::JsonDepthExceeded(MAX_JSON_DEPTH))?;

    Ok(())
}

/// Validates a response frame before returning it to the caller.
pub fn validate_backend_response_frame(
    frame: &BackendResponseFrame,
) -> Result<(), BackendFrameError> {
    if frame.id > MAX_REQUEST_ID {
        return Err(BackendFrameError::InvalidRequestId(frame.id));
    }

    if let Some(ref res) = frame.result {
        let serialized = serde_json::to_vec(res)
            .map_err(|e| BackendFrameError::MalformedEnvelope(e.to_string()))?;
        if serialized.len() > MAX_BACKEND_RESPONSE_BYTES {
            return Err(BackendFrameError::ResultTooLarge(
                serialized.len(),
                MAX_BACKEND_RESPONSE_BYTES,
            ));
        }
    }

    Ok(())
}

/// Serializes and frames a `BackendRequestFrame` with a 4-byte big-endian length prefix.
pub fn encode_request_frame(req: &BackendRequestFrame) -> Result<Vec<u8>, BackendFrameError> {
    if req.id > MAX_REQUEST_ID {
        return Err(BackendFrameError::InvalidRequestId(req.id));
    }
    if req.method.is_empty() || req.method.len() > MAX_METHOD_NAME_LENGTH {
        return Err(BackendFrameError::InvalidMethodName);
    }

    let json_bytes =
        serde_json::to_vec(req).map_err(|e| BackendFrameError::MalformedEnvelope(e.to_string()))?;

    if json_bytes.len() > MAX_BACKEND_RESPONSE_BYTES {
        return Err(BackendFrameError::FrameTooLarge(
            json_bytes.len(),
            MAX_BACKEND_RESPONSE_BYTES,
        ));
    }

    let mut out = Vec::with_capacity(4 + json_bytes.len());
    out.extend_from_slice(&(json_bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(&json_bytes);
    Ok(out)
}

/// Serializes and frames a `BackendResponseFrame` with a 4-byte big-endian length prefix.
pub fn encode_response_frame(res: &BackendResponseFrame) -> Result<Vec<u8>, BackendFrameError> {
    if res.id > MAX_REQUEST_ID {
        return Err(BackendFrameError::InvalidRequestId(res.id));
    }

    validate_backend_response_frame(res)?;

    let json_bytes =
        serde_json::to_vec(res).map_err(|e| BackendFrameError::MalformedEnvelope(e.to_string()))?;

    if json_bytes.len() > MAX_BACKEND_RESPONSE_BYTES {
        return Err(BackendFrameError::FrameTooLarge(
            json_bytes.len(),
            MAX_BACKEND_RESPONSE_BYTES,
        ));
    }

    let mut out = Vec::with_capacity(4 + json_bytes.len());
    out.extend_from_slice(&(json_bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(&json_bytes);
    Ok(out)
}

/// Accumulating buffer for partial stream reads, enforcing frame size and buffer limits.
#[derive(Debug, Clone, Default)]
pub struct BackendFrameBuffer {
    buffer: Vec<u8>,
}

impl BackendFrameBuffer {
    /// Create an empty frame accumulation buffer.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Return current byte length of buffered data.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Return whether buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Discard all buffered data.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Append raw bytes into the buffer, rejecting buffer overflows beyond `MAX_FRAME_BUFFER_BYTES`.
    pub fn push_bytes(&mut self, chunk: &[u8]) -> Result<(), BackendFrameError> {
        if self.buffer.len() + chunk.len() > MAX_FRAME_BUFFER_BYTES {
            return Err(BackendFrameError::BufferOverflow(
                self.buffer.len() + chunk.len(),
                MAX_FRAME_BUFFER_BYTES,
            ));
        }
        self.buffer.extend_from_slice(chunk);
        Ok(())
    }

    /// Attempt to pop a complete, validated `BackendResponseFrame`.
    /// Returns `Ok(None)` if more data is needed to complete the frame.
    pub fn pop_response_frame(
        &mut self,
    ) -> Result<Option<BackendResponseFrame>, BackendFrameError> {
        if self.buffer.len() < 4 {
            return Ok(None);
        }

        let frame_len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if frame_len > MAX_BACKEND_RESPONSE_BYTES {
            return Err(BackendFrameError::FrameTooLarge(
                frame_len,
                MAX_BACKEND_RESPONSE_BYTES,
            ));
        }

        if self.buffer.len() < 4 + frame_len {
            return Ok(None);
        }

        let payload = &self.buffer[4..4 + frame_len];
        let payload_str = std::str::from_utf8(payload)
            .map_err(|e| BackendFrameError::MalformedEnvelope(e.to_string()))?;

        if check_nesting_depth(payload_str, MAX_JSON_DEPTH).is_err() {
            return Err(BackendFrameError::JsonDepthExceeded(MAX_JSON_DEPTH));
        }

        let frame: BackendResponseFrame = serde_json::from_str(payload_str)
            .map_err(|e| BackendFrameError::MalformedEnvelope(e.to_string()))?;

        validate_backend_response_frame(&frame)?;

        self.buffer.drain(..4 + frame_len);
        Ok(Some(frame))
    }

    /// Attempt to pop a complete, validated `BackendRequestFrame`.
    /// Returns `Ok(None)` if more data is needed to complete the frame.
    pub fn pop_request_frame(&mut self) -> Result<Option<BackendRequestFrame>, BackendFrameError> {
        if self.buffer.len() < 4 {
            return Ok(None);
        }

        let frame_len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if frame_len > MAX_BACKEND_RESPONSE_BYTES {
            return Err(BackendFrameError::FrameTooLarge(
                frame_len,
                MAX_BACKEND_RESPONSE_BYTES,
            ));
        }

        if self.buffer.len() < 4 + frame_len {
            return Ok(None);
        }

        let payload = &self.buffer[4..4 + frame_len];
        let payload_str = std::str::from_utf8(payload)
            .map_err(|e| BackendFrameError::MalformedEnvelope(e.to_string()))?;

        if check_nesting_depth(payload_str, MAX_JSON_DEPTH).is_err() {
            return Err(BackendFrameError::JsonDepthExceeded(MAX_JSON_DEPTH));
        }

        let frame: BackendRequestFrame = serde_json::from_str(payload_str)
            .map_err(|e| BackendFrameError::MalformedEnvelope(e.to_string()))?;

        if frame.id > MAX_REQUEST_ID {
            return Err(BackendFrameError::InvalidRequestId(frame.id));
        }
        if frame.method.is_empty() || frame.method.len() > MAX_METHOD_NAME_LENGTH {
            return Err(BackendFrameError::InvalidMethodName);
        }

        self.buffer.drain(..4 + frame_len);
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_match_spec_1_10() {
        assert_eq!(MAX_BACKEND_REQUEST_PARAMS_BYTES, 1024 * 1024);
        assert_eq!(MAX_BACKEND_RESPONSE_BYTES, 4 * 1024 * 1024);
        assert_eq!(MAX_BACKEND_TIMEOUT_SECS, 300);
        assert_eq!(MAX_BACKEND_CONCURRENT_REQUESTS_PER_WINDOW, 16);
        assert_eq!(MAX_BACKEND_DECLARED_METHODS, 256);
        assert_eq!(MAX_BACKEND_LOG_LINE_BYTES, 4096);
        assert_eq!(MAX_BACKEND_LOG_RATE_PER_SEC, 200);
        assert_eq!(MAX_BACKEND_RESTART_RETRIES, 3);
    }

    #[test]
    fn test_method_exact_match_rule() {
        let allowlist = vec!["hello.greet".to_string(), "todos.list".to_string()];

        // Exact matches succeed
        assert!(validate_backend_invoke_method("hello.greet", &allowlist).is_ok());
        assert!(validate_backend_invoke_method("todos.list", &allowlist).is_ok());

        // Case mismatch rejected
        assert_eq!(
            validate_backend_invoke_method("Hello.Greet", &allowlist),
            Err(BackendValidationError::MethodNotAllowed(
                "Hello.Greet".to_string()
            ))
        );

        // Trailing whitespace rejected
        assert_eq!(
            validate_backend_invoke_method("hello.greet ", &allowlist),
            Err(BackendValidationError::MethodNotAllowed(
                "hello.greet ".to_string()
            ))
        );

        // Prefix match rejected
        assert_eq!(
            validate_backend_invoke_method("hello", &allowlist),
            Err(BackendValidationError::MethodNotAllowed(
                "hello".to_string()
            ))
        );

        // Prefix extension rejected
        assert_eq!(
            validate_backend_invoke_method("hello.greet.x", &allowlist),
            Err(BackendValidationError::MethodNotAllowed(
                "hello.greet.x".to_string()
            ))
        );

        // Unknown method rejected
        assert_eq!(
            validate_backend_invoke_method("unknown.method", &allowlist),
            Err(BackendValidationError::MethodNotAllowed(
                "unknown.method".to_string()
            ))
        );
    }

    #[test]
    fn test_params_size_and_depth_limits() {
        // Valid params
        let ok_val = serde_json::json!({"name": "world"});
        assert!(validate_backend_invoke_params(&ok_val).is_ok());

        // 1 MB boundary tests
        let small_val = serde_json::json!({"data": "A".repeat(1000)});
        assert!(validate_backend_invoke_params(&small_val).is_ok());

        // 1 MB + 1 byte payload must be rejected
        let oversize_str = "x".repeat(MAX_BACKEND_REQUEST_PARAMS_BYTES + 1);
        let oversize_val = serde_json::Value::String(oversize_str);
        assert!(matches!(
            validate_backend_invoke_params(&oversize_val),
            Err(BackendValidationError::ParamsTooLarge(len, max)) if len > max
        ));

        // Depth bomb test (> 32 levels)
        let mut deep = serde_json::json!("leaf");
        for _ in 0..35 {
            deep = serde_json::json!([deep]);
        }
        assert!(matches!(
            validate_backend_invoke_params(&deep),
            Err(BackendValidationError::JsonDepthExceeded(32))
        ));
    }

    #[test]
    fn test_frame_encoding_and_decoding_roundtrip() {
        let req = BackendRequestFrame {
            id: 42,
            method: "hello.greet".to_string(),
            params: serde_json::json!({"user": "Alice"}),
        };

        let encoded = encode_request_frame(&req).expect("encode request");
        let expected_json_len = serde_json::to_vec(&req).unwrap().len();
        assert_eq!(encoded.len(), 4 + expected_json_len);

        let mut buf = BackendFrameBuffer::new();
        buf.push_bytes(&encoded).expect("push bytes");
        let popped = buf
            .pop_request_frame()
            .expect("pop frame")
            .expect("must be Some");
        assert_eq!(popped, req);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_response_frame_deny_unknown_fields() {
        // Envelope containing unknown field must be rejected
        let malformed = r#"{"id":1,"ok":true,"result":null,"extra":"malicious"}"#;
        let mut buf = BackendFrameBuffer::new();
        let len = (malformed.len() as u32).to_be_bytes();
        buf.push_bytes(&len).unwrap();
        buf.push_bytes(malformed.as_bytes()).unwrap();

        let err = buf.pop_response_frame().unwrap_err();
        assert!(matches!(err, BackendFrameError::MalformedEnvelope(_)));
    }

    #[test]
    fn test_oversize_response_frame_rejected() {
        let mut buf = BackendFrameBuffer::new();
        let oversize_len = (MAX_BACKEND_RESPONSE_BYTES as u32) + 10;
        buf.push_bytes(&oversize_len.to_be_bytes()).unwrap();

        let err = buf.pop_response_frame().unwrap_err();
        assert!(matches!(
            err,
            BackendFrameError::FrameTooLarge(len, max) if len > max
        ));
    }

    #[test]
    fn test_response_result_size_limit_4mb_plus_1() {
        let big_res = BackendResponseFrame {
            id: 1,
            ok: true,
            result: Some(serde_json::Value::String(
                "A".repeat(MAX_BACKEND_RESPONSE_BYTES + 1),
            )),
            error: None,
        };

        let err = encode_response_frame(&big_res).unwrap_err();
        assert!(matches!(
            err,
            BackendFrameError::ResultTooLarge(len, max) if len > max
        ));
    }

    #[test]
    fn test_fragmented_frame_read_1_byte_at_a_time() {
        let resp = BackendResponseFrame {
            id: 99,
            ok: true,
            result: Some(serde_json::json!({"status": "ready"})),
            error: None,
        };

        let bytes = encode_response_frame(&resp).expect("encode");
        let mut buf = BackendFrameBuffer::new();

        // Feed one byte at a time
        for i in 0..bytes.len() - 1 {
            buf.push_bytes(&bytes[i..i + 1]).expect("push 1 byte");
            assert!(
                buf.pop_response_frame()
                    .expect("pop response should not error")
                    .is_none(),
                "should be None before complete frame is received"
            );
        }

        // Final byte completes the frame
        buf.push_bytes(&bytes[bytes.len() - 1..])
            .expect("push last");
        let popped = buf
            .pop_response_frame()
            .expect("pop frame")
            .expect("must be Some");
        assert_eq!(popped, resp);
        assert!(buf.is_empty());
    }
}
