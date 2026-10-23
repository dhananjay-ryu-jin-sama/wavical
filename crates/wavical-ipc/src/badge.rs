#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum single-digit / double-digit badge count displayed numerically.
pub const MAX_BADGE_NUMERIC_COUNT: u32 = 99;

/// Parsed and validated taskbar badge display value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BadgeDisplay {
    /// Count is 0: clear the badge overlay.
    Clear,
    /// Count between 1 and 99: render the exact count string.
    Count(u32),
    /// Count above 99: render "99+".
    Overflow,
}

impl BadgeDisplay {
    /// Return the textual representation rendered on the taskbar badge, or None if cleared.
    pub fn render(&self) -> Option<String> {
        match self {
            Self::Clear => None,
            Self::Count(n) => Some(n.to_string()),
            Self::Overflow => Some("99+".to_string()),
        }
    }
}

/// Errors occurring during badge parameter validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BadgeValidationError {
    #[error("badge count must be non-negative, got {0}")]
    NegativeCount(i64),

    #[error("invalid badge parameters: {0}")]
    InvalidParams(String),
}

/// Validate raw integer badge count.
/// 0 clears the badge, 1..=99 shows count, >99 shows "99+", negative rejected.
pub fn validate_badge_count(count: i64) -> Result<BadgeDisplay, BadgeValidationError> {
    if count < 0 {
        return Err(BadgeValidationError::NegativeCount(count));
    }
    if count == 0 {
        Ok(BadgeDisplay::Clear)
    } else if count <= MAX_BADGE_NUMERIC_COUNT as i64 {
        Ok(BadgeDisplay::Count(count as u32))
    } else {
        Ok(BadgeDisplay::Overflow)
    }
}

/// Parse and validate badge parameters from JSON-RPC parameters.
/// Accepts either an integer (e.g. `5`), or an object with a `count` field (`{"count": 5}`).
pub fn parse_and_validate_badge_params(
    params: Option<&serde_json::Value>,
) -> Result<BadgeDisplay, BadgeValidationError> {
    let val = params.ok_or_else(|| {
        BadgeValidationError::InvalidParams("missing badge parameters".to_string())
    })?;

    let count = if let Some(n) = val.as_i64() {
        n
    } else if let Some(obj) = val.as_object() {
        obj.get("count").and_then(|v| v.as_i64()).ok_or_else(|| {
            BadgeValidationError::InvalidParams(
                "expected integer 'count' field in object".to_string(),
            )
        })?
    } else {
        return Err(BadgeValidationError::InvalidParams(
            "expected integer count or object with 'count'".to_string(),
        ));
    };

    validate_badge_count(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_badge_zero_clears() {
        let display = validate_badge_count(0).unwrap();
        assert_eq!(display, BadgeDisplay::Clear);
        assert_eq!(display.render(), None);
    }

    #[test]
    fn test_badge_numeric_range() {
        let d1 = validate_badge_count(1).unwrap();
        assert_eq!(d1, BadgeDisplay::Count(1));
        assert_eq!(d1.render().as_deref(), Some("1"));

        let d5 = validate_badge_count(5).unwrap();
        assert_eq!(d5, BadgeDisplay::Count(5));
        assert_eq!(d5.render().as_deref(), Some("5"));

        let d99 = validate_badge_count(99).unwrap();
        assert_eq!(d99, BadgeDisplay::Count(99));
        assert_eq!(d99.render().as_deref(), Some("99"));
    }

    #[test]
    fn test_badge_overflow_renders_99_plus() {
        let d100 = validate_badge_count(100).unwrap();
        assert_eq!(d100, BadgeDisplay::Overflow);
        assert_eq!(d100.render().as_deref(), Some("99+"));

        let d150 = validate_badge_count(150).unwrap();
        assert_eq!(d150, BadgeDisplay::Overflow);
        assert_eq!(d150.render().as_deref(), Some("99+"));

        let d9999 = validate_badge_count(9999).unwrap();
        assert_eq!(d9999, BadgeDisplay::Overflow);
        assert_eq!(d9999.render().as_deref(), Some("99+"));
    }

    #[test]
    fn test_badge_negative_rejected() {
        assert_eq!(
            validate_badge_count(-1),
            Err(BadgeValidationError::NegativeCount(-1))
        );
        assert_eq!(
            validate_badge_count(-100),
            Err(BadgeValidationError::NegativeCount(-100))
        );
    }

    #[test]
    fn test_parse_and_validate_badge_json() {
        // Raw integer
        assert_eq!(
            parse_and_validate_badge_params(Some(&serde_json::json!(5))).unwrap(),
            BadgeDisplay::Count(5)
        );

        // Object with count
        assert_eq!(
            parse_and_validate_badge_params(Some(&serde_json::json!({ "count": 150 }))).unwrap(),
            BadgeDisplay::Overflow
        );
        assert_eq!(
            parse_and_validate_badge_params(Some(&serde_json::json!({ "count": 0 }))).unwrap(),
            BadgeDisplay::Clear
        );

        // Negative in object
        assert_eq!(
            parse_and_validate_badge_params(Some(&serde_json::json!({ "count": -5 }))),
            Err(BadgeValidationError::NegativeCount(-5))
        );

        // Invalid types
        assert!(matches!(
            parse_and_validate_badge_params(Some(&serde_json::json!("invalid"))),
            Err(BadgeValidationError::InvalidParams(_))
        ));
        assert!(matches!(
            parse_and_validate_badge_params(Some(&serde_json::json!(null))),
            Err(BadgeValidationError::InvalidParams(_))
        ));
        assert!(matches!(
            parse_and_validate_badge_params(None),
            Err(BadgeValidationError::InvalidParams(_))
        ));
        assert!(matches!(
            parse_and_validate_badge_params(Some(&serde_json::json!({ "wrong_field": 5 }))),
            Err(BadgeValidationError::InvalidParams(_))
        ));
    }
}
