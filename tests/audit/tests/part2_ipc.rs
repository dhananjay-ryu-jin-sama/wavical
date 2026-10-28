// tests/audit/tests/part2_ipc.rs
//! Independent black-box security audit test suite for `wavical-ipc`.
//! Expectations are derived strictly from WP4 and Phase 0 build plan:
//! - docs/phase 0/WAVICAL_PHASE_0_BUILD_PLAN.md

use proptest::prelude::*;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use wavical_ipc::{
    Capabilities, GENERIC_HANDLER_ERROR, GENERIC_METHOD_ERROR, HandlerRegistry, IpcError,
    MAX_JSON_DEPTH, MAX_MESSAGE_SIZE_BYTES, Request, dispatch, dispatch_raw, parse_request,
    validate_method_name, validate_raw_message,
};

#[test]
fn test_part2_size_limits() {
    // 1. limit - 1
    let base_req = r#"{"id":1,"method":"core.ping","params":""}"#;
    let padding_len = MAX_MESSAGE_SIZE_BYTES - 1 - base_req.len();
    let padding = "a".repeat(padding_len);
    let req_limit_minus_1 = format!(r#"{{"id":1,"method":"core.ping","params":"{padding}"}}"#);
    assert_eq!(req_limit_minus_1.len(), MAX_MESSAGE_SIZE_BYTES - 1);
    assert!(parse_request(&req_limit_minus_1).is_ok());

    // 2. limit
    let padding_limit = "a".repeat(padding_len + 1);
    let req_limit = format!(r#"{{"id":1,"method":"core.ping","params":"{padding_limit}"}}"#);
    assert_eq!(req_limit.len(), MAX_MESSAGE_SIZE_BYTES);
    assert!(parse_request(&req_limit).is_ok());

    // 3. limit + 1
    let padding_limit_plus_1 = "a".repeat(padding_len + 2);
    let req_limit_plus_1 =
        format!(r#"{{"id":1,"method":"core.ping","params":"{padding_limit_plus_1}"}}"#);
    assert_eq!(req_limit_plus_1.len(), MAX_MESSAGE_SIZE_BYTES + 1);
    match parse_request(&req_limit_plus_1) {
        Err(IpcError::MessageTooLarge { size, max }) => {
            assert_eq!(size, MAX_MESSAGE_SIZE_BYTES + 1);
            assert_eq!(max, MAX_MESSAGE_SIZE_BYTES);
        }
        other => panic!("Expected MessageTooLarge, got {:?}", other),
    }

    // 4. 50 MB of garbage: oversize check must happen BEFORE JSON parsing and return in milliseconds
    let huge_garbage = "X".repeat(50 * 1024 * 1024);
    let start = Instant::now();
    let res = parse_request(&huge_garbage);
    let elapsed = start.elapsed();

    match res {
        Err(IpcError::MessageTooLarge { size, max }) => {
            assert_eq!(size, 50 * 1024 * 1024);
            assert_eq!(max, MAX_MESSAGE_SIZE_BYTES);
        }
        other => panic!("50 MB garbage must return MessageTooLarge, got {:?}", other),
    }

    assert!(
        elapsed.as_millis() < 50,
        "Oversize check took too long: {} ms (must return in milliseconds)",
        elapsed.as_millis()
    );
}

#[test]
fn test_part2_depth_limits() {
    let depths = [64, 128, 129, 1000, 100_000];

    for &d in &depths {
        // Top-level nesting
        let top_level = format!("{}{}", "[".repeat(d), "]".repeat(d));
        let res = parse_request(&top_level);
        match res {
            Err(IpcError::RecursionLimitExceeded { depth, max }) => {
                assert!(depth > max);
                assert_eq!(max, MAX_JSON_DEPTH);
            }
            Err(IpcError::InvalidJson(_)) => {}
            other => panic!(
                "Depth {} top-level should fail with clean error, got {:?}",
                d, other
            ),
        }

        // Inside params nesting
        let nested_brackets = format!("{}{}", "[".repeat(d), "]".repeat(d));
        let params_nested = format!(
            r#"{{"id":1,"method":"core.ping","params":{}}}"#,
            nested_brackets
        );
        let res_params = parse_request(&params_nested);
        match res_params {
            Err(IpcError::RecursionLimitExceeded { depth, max }) => {
                assert!(depth > max);
                assert_eq!(max, MAX_JSON_DEPTH);
            }
            Err(IpcError::InvalidJson(_)) => {}
            other => panic!(
                "Depth {} inside params should fail with clean error, got {:?}",
                d, other
            ),
        }
    }
}

#[test]
fn test_part2_shape_validation() {
    // Non-object top-level types
    assert!(parse_request("[]").is_err());
    assert!(parse_request(r#""hello""#).is_err());
    assert!(parse_request("123").is_err());
    assert!(parse_request("null").is_err());
    assert!(parse_request("true").is_err());

    // Empty object
    assert!(parse_request("{}").is_err());

    // Missing id
    assert!(parse_request(r#"{"method":"core.ping"}"#).is_err());

    // Missing method
    assert!(parse_request(r#"{"id":1}"#).is_err());

    // Extra field (deny_unknown_fields)
    assert!(parse_request(r#"{"id":1,"method":"core.ping","extra":"val"}"#).is_err());

    // Duplicate keys
    let dup_msg = r#"{"id":1,"method":"core.ping","method":"core.echo"}"#;
    assert!(parse_request(dup_msg).is_err());

    // UTF-8 BOM prefix
    let bom_msg = "\u{FEFF}{\"id\":1,\"method\":\"core.ping\"}";
    assert!(parse_request(bom_msg).is_err());

    // Trailing garbage
    assert!(parse_request(r#"{"id":1,"method":"core.ping"} garbage"#).is_err());

    // Leading and trailing whitespace
    let ws_req = parse_request(r#"   {"id":1,"method":"core.ping"}   "#);
    assert!(ws_req.is_ok());

    // params as every JSON type
    assert!(parse_request(r#"{"id":1,"method":"core.ping","params":null}"#).is_ok());
    assert!(parse_request(r#"{"id":1,"method":"core.ping","params":true}"#).is_ok());
    assert!(parse_request(r#"{"id":1,"method":"core.ping","params":42}"#).is_ok());
    assert!(parse_request(r#"{"id":1,"method":"core.ping","params":"text"}"#).is_ok());
    assert!(parse_request(r#"{"id":1,"method":"core.ping","params":[1,2,3]}"#).is_ok());
    assert!(parse_request(r#"{"id":1,"method":"core.ping","params":{"a":1}}"#).is_ok());
}

#[test]
fn test_part2_id_validation() {
    // 0: accepted
    assert!(parse_request(r#"{"id":0,"method":"core.ping"}"#).is_ok());

    // -1: rejected
    assert!(parse_request(r#"{"id":-1,"method":"core.ping"}"#).is_err());

    // 1: accepted
    assert!(parse_request(r#"{"id":1,"method":"core.ping"}"#).is_ok());

    // u32::MAX (4294967295): accepted
    assert!(parse_request(&format!(r#"{{"id":{},"method":"core.ping"}}"#, u32::MAX)).is_ok());

    // 2^53 (9007199254740992): rejected (exceeds MAX_REQUEST_ID = 2^53 - 1)
    let id_2_pow_53 = 9_007_199_254_740_992u64;
    assert!(parse_request(&format!(r#"{{"id":{id_2_pow_53},"method":"core.ping"}}"#)).is_err());

    // 2^53 + 1: rejected
    assert!(
        parse_request(&format!(
            r#"{{"id":{},"method":"core.ping"}}"#,
            id_2_pow_53 + 1
        ))
        .is_err()
    );

    // u64::MAX: rejected
    assert!(parse_request(&format!(r#"{{"id":{},"method":"core.ping"}}"#, u64::MAX)).is_err());

    // u64::MAX + 1: rejected
    assert!(parse_request(r#"{"id":18446744073709551616,"method":"core.ping"}"#).is_err());

    // 1.5: float rejected
    assert!(parse_request(r#"{"id":1.5,"method":"core.ping"}"#).is_err());

    // 1e3: scientific notation rejected
    assert!(parse_request(r#"{"id":1e3,"method":"core.ping"}"#).is_err());

    // "1": string id rejected
    assert!(parse_request(r#"{"id":"1","method":"core.ping"}"#).is_err());

    // null, true, [], {}: rejected
    assert!(parse_request(r#"{"id":null,"method":"core.ping"}"#).is_err());
    assert!(parse_request(r#"{"id":true,"method":"core.ping"}"#).is_err());
    assert!(parse_request(r#"{"id":[],"method":"core.ping"}"#).is_err());
    assert!(parse_request(r#"{"id":{},"method":"core.ping"}"#).is_err());
}

#[test]
fn test_part2_method_validation() {
    assert!(validate_method_name("").is_err());
    assert!(validate_method_name("a").is_ok());
    assert!(validate_method_name(&"a".repeat(64)).is_ok());
    assert!(validate_method_name(&"a".repeat(65)).is_err());

    assert!(validate_method_name("core.ping ").is_err());
    assert!(validate_method_name(" core.ping").is_err());
    assert!(validate_method_name("core.ping\0").is_err());
    assert!(validate_method_name("core..ping").is_err());
    assert!(validate_method_name(".core.ping").is_err());
    assert!(validate_method_name("core.ping.").is_err());
    assert!(validate_method_name("core/ping").is_err());
    assert!(validate_method_name("core.*").is_err());
    assert!(validate_method_name("*").is_err());
    assert!(validate_method_name("core").is_ok());
    assert!(validate_method_name("соre.ping").is_err());
    assert!(validate_method_name("core.ping🚀").is_err());
    assert!(validate_method_name(&"a".repeat(10_000)).is_err());
    assert!(validate_method_name("core.ping\n").is_err());
    assert!(validate_method_name("core.ping\t").is_err());
}

#[test]
fn test_part2_capabilities() {
    let mut caps = Capabilities::new();
    assert!(caps.is_empty());
    assert!(!caps.is_allowed("core.ping"));
    assert!(!caps.is_allowed("core.echo"));

    caps.grant("core.ping");
    assert!(caps.is_allowed("core.ping"));
    assert!(!caps.is_allowed("core.echo"));
    assert!(!caps.is_allowed("Core.Ping"));
    assert!(!caps.is_allowed("core.ping "));
    assert!(!caps.is_allowed("core.*"));

    let reg = HandlerRegistry::with_core_handlers();

    // Denial comparison:
    // (a) method exists in registry but ungranted: core.echo
    let req_ungranted = Request {
        id: 42,
        method: "core.echo".to_string(),
        params: json!({"test": "data"}),
    };
    let resp_ungranted = dispatch(&req_ungranted, &caps, &reg);

    // (b) method does not exist: nonexistent.foo
    let req_unknown = Request {
        id: 42,
        method: "nonexistent.foo".to_string(),
        params: json!({"test": "data"}),
    };
    let resp_unknown = dispatch(&req_unknown, &caps, &reg);

    // Assert BYTE-FOR-BYTE identical responses
    let json_ungranted = serde_json::to_string(&resp_ungranted).unwrap();
    let json_unknown = serde_json::to_string(&resp_unknown).unwrap();

    assert_eq!(
        json_ungranted, json_unknown,
        "Denial responses for ungranted and unknown methods MUST be byte-for-byte identical"
    );

    // Assert leak checks
    assert!(!json_ungranted.contains("core.echo"));
    assert!(!json_ungranted.contains("capability"));
    assert!(!json_ungranted.contains("HandlerRegistry"));
    assert!(!json_ungranted.contains("/"));
    assert!(!json_ungranted.contains("\\"));

    // Counting fake handler to prove ungranted method never reaches handler even with invalid params
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);

    let mut custom_reg = HandlerRegistry::new();
    custom_reg.register("guarded.action", move |_params| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        Ok(json!("executed"))
    });

    let guarded_req = Request {
        id: 99,
        method: "guarded.action".to_string(),
        params: json!("invalid_malformed_params"),
    };

    let empty_caps = Capabilities::new();
    let resp = dispatch(&guarded_req, &empty_caps, &custom_reg);
    assert!(!resp.ok);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "Ungranted method must NEVER reach the handler"
    );
}

#[test]
fn test_part2_response_shape_and_id_echo() {
    let mut caps = Capabilities::new();
    caps.grant("core.ping");
    let reg = HandlerRegistry::with_core_handlers();

    // 1. Success response echoes matching id
    let req_success = Request {
        id: 1234567,
        method: "core.ping".to_string(),
        params: json!(null),
    };
    let resp_success = dispatch(&req_success, &caps, &reg);
    assert_eq!(resp_success.id, 1234567);
    assert!(resp_success.ok);
    assert!(resp_success.result.is_some());
    assert!(resp_success.error.is_none());

    // 2. Error response echoes matching id
    let req_err = Request {
        id: 7654321,
        method: "unallowed.method".to_string(),
        params: json!(null),
    };
    let resp_err = dispatch(&req_err, &caps, &reg);
    assert_eq!(resp_err.id, 7654321);
    assert!(!resp_err.ok);
    assert!(resp_err.result.is_none());
    assert_eq!(resp_err.error.as_deref(), Some(GENERIC_METHOD_ERROR));

    // 3. What happens for malformed messages with no usable ID?
    // dispatch_raw returns Err(IpcError), so no response is generated (caller must drop or reject connection)
    let malformed_no_id = r#"{"method":"core.ping"}"#;
    let raw_res = dispatch_raw(malformed_no_id, &caps, &reg);
    assert!(matches!(raw_res, Err(IpcError::InvalidJson(_))));
}

/// Black-box audit expectation: ANY internal handler failure MUST be sanitized to a generic error
/// so internal symbols, stack traces, database schema names, and memory addresses are never revealed to the page.
/// FINDING-I1: `dispatch` only sanitizes errors containing '/', '\\', or ':'; arbitrary internal error strings pass unsanitized to the webview.
#[test]
fn test_part2_response_internal_leakage() {
    let mut caps = Capabilities::new();
    caps.grant("sensitive.action");

    let mut reg = HandlerRegistry::new();
    reg.register("sensitive.action", |_params| {
        Err("InternalDatabaseException in table user_secrets_v2 connection reset with code 0xDEADBEEF".to_string())
    });

    let req = Request {
        id: 101,
        method: "sensitive.action".to_string(),
        params: json!(null),
    };

    let resp = dispatch(&req, &caps, &reg);
    assert!(!resp.ok);
    let err_msg = resp.error.expect("error message must be present");

    assert_eq!(
        err_msg, GENERIC_HANDLER_ERROR,
        "Internal handler error leaked details to webview: {err_msg}"
    );
}

/// F-A5-04: Raw null byte pre-check in validate_raw_message must specifically return IpcError::NullByteDetected
#[test]
fn test_validate_raw_message_null_byte_rejected() {
    let null_payload = "{\"id\":1,\0\"method\":\"core.ping\"}";
    assert_eq!(
        validate_raw_message(null_payload),
        Err(IpcError::NullByteDetected),
        "F-A5-04: validate_raw_message must specifically return IpcError::NullByteDetected"
    );
}

/// F-A5-05: Empty raw message guard in validate_raw_message must specifically return IpcError::EmptyMessage
#[test]
fn test_validate_raw_message_empty_rejected() {
    assert_eq!(
        validate_raw_message(""),
        Err(IpcError::EmptyMessage),
        "F-A5-05: validate_raw_message(\"\") must specifically return IpcError::EmptyMessage"
    );
}

// =========================================================================
// Proptests: at least 10,000 cases each
// =========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// Property 1: Random byte strings never cause a panic in parse_request
    #[test]
    fn prop_ipc_random_strings_never_panic(raw in "\\PC*") {
        let _ = parse_request(&raw);
    }

    /// Property 2: Random JSON strings never cause a panic in parse_request
    #[test]
    fn prop_ipc_random_json_never_panic(
        id in any::<u64>(),
        method in "[a-zA-Z0-9_.-]{0,70}",
        params in ".*"
    ) {
        let raw = format!(r#"{{"id":{id},"method":"{method}","params":"{params}"}}"#);
        let _ = parse_request(&raw);
    }

    /// Property 3: Random method strings against random capabilities never execute an ungranted method
    #[test]
    fn prop_ipc_no_ungranted_method_ever_runs(
        method in "[a-zA-Z0-9_.-]{1,40}",
        grant_it in any::<bool>()
    ) {
        let execution_counter = Arc::new(AtomicUsize::new(0));
        let exec_clone = Arc::clone(&execution_counter);

        let mut reg = HandlerRegistry::new();
        reg.register(method.clone(), move |_| {
            exec_clone.fetch_add(1, Ordering::SeqCst);
            Ok(json!("executed"))
        });

        let mut caps = Capabilities::new();
        if grant_it {
            caps.grant(method.clone());
        }

        let req = Request {
            id: 1,
            method: method.clone(),
            params: json!(null),
        };

        let resp = dispatch(&req, &caps, &reg);

        if grant_it {
            prop_assert!(resp.ok);
            prop_assert_eq!(execution_counter.load(Ordering::SeqCst), 1);
        } else {
            prop_assert!(!resp.ok);
            prop_assert_eq!(execution_counter.load(Ordering::SeqCst), 0);
        }
    }
}
