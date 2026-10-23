//! Pure window state persistence parser and resolution.
//!
//! Enforces:
//! - Rejects corrupt JSON with explicit error.
//! - Ignores unknown fields (forward compatibility).
//! - Falls back to defaults on missing fields.
//! - Validates position against connected screen bounds; falls back to default if off-screen.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default framework window width (1200px).
pub const DEFAULT_SAVED_WIDTH: u32 = 1200;

/// Default framework window height (800px).
pub const DEFAULT_SAVED_HEIGHT: u32 = 800;

/// Error returned when window state JSON is corrupt or unparseable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WindowStateError {
    #[error("Corrupt JSON window state: {0}")]
    CorruptJson(String),
}

/// Representation of a connected display monitor's physical bounds in virtual screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenBounds {
    /// Construct a new screen bounding rectangle.
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Check whether a window rectangle overlaps this screen with grabable visible area.
    pub fn intersects(&self, win_x: i32, win_y: i32, win_w: u32, win_h: u32) -> bool {
        let win_right = win_x.saturating_add(win_w.max(1) as i32);
        let win_bottom = win_y.saturating_add(win_h.max(1) as i32);
        let screen_right = self.x.saturating_add(self.width as i32);
        let screen_bottom = self.y.saturating_add(self.height as i32);

        win_x < screen_right && win_right > self.x && win_y < screen_bottom && win_bottom > self.y
    }
}

/// Deserialized window layout state (position, dimensions, maximized).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
    /// Window X position on virtual desktop (None indicates default/centered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,

    /// Window Y position on virtual desktop (None indicates default/centered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,

    /// Window width in pixels.
    pub width: u32,

    /// Window height in pixels.
    pub height: u32,

    /// Whether the window was maximized.
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: DEFAULT_SAVED_WIDTH,
            height: DEFAULT_SAVED_HEIGHT,
            maximized: false,
        }
    }
}

/// Intermediate lenient deserializer that ignores unknown fields and accepts missing fields.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawWindowState {
    #[serde(default)]
    x: Option<i64>,
    #[serde(default)]
    y: Option<i64>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
    #[serde(default)]
    maximized: Option<bool>,
}

/// Parse a raw JSON string into a `WindowState`.
///
/// Rules:
/// - Rejects corrupt/invalid JSON syntax with `WindowStateError::CorruptJson`.
/// - Ignores unknown fields completely.
/// - Missing fields fall back to default values (width: 1200, height: 800, maximized: false, x/y: None).
pub fn parse_window_state(raw_json: &str) -> Result<WindowState, WindowStateError> {
    let trimmed = raw_json.trim();
    if trimmed.is_empty() {
        return Err(WindowStateError::CorruptJson(
            "JSON string is empty".to_string(),
        ));
    }

    let val: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| WindowStateError::CorruptJson(e.to_string()))?;

    let map = match val {
        serde_json::Value::Object(m) => m,
        _ => {
            return Err(WindowStateError::CorruptJson(
                "Window state JSON must be an object".to_string(),
            ));
        }
    };

    let raw: RawWindowState = serde_json::from_value(serde_json::Value::Object(map))
        .map_err(|e| WindowStateError::CorruptJson(e.to_string()))?;

    let width = match raw.width {
        Some(w) if (100.0..=32768.0).contains(&w) => w as u32,
        _ => DEFAULT_SAVED_WIDTH,
    };

    let height = match raw.height {
        Some(h) if (100.0..=32768.0).contains(&h) => h as u32,
        _ => DEFAULT_SAVED_HEIGHT,
    };

    let x = match raw.x {
        Some(val) => i32::try_from(val).ok(),
        None => None,
    };

    let y = match raw.y {
        Some(val) => i32::try_from(val).ok(),
        None => None,
    };

    let maximized = raw.maximized.unwrap_or(false);

    Ok(WindowState {
        x,
        y,
        width,
        height,
        maximized,
    })
}

/// Check whether the specified window rectangle is visible on at least one connected display screen.
/// If `screens` is empty (e.g. headless environment), returns `true`.
pub fn is_position_on_any_screen(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    screens: &[ScreenBounds],
) -> bool {
    if screens.is_empty() {
        return true;
    }
    screens
        .iter()
        .any(|screen| screen.intersects(x, y, width, height))
}

/// Resolve window state from raw JSON, validating against screen monitors and falling back to defaults.
///
/// If:
/// - JSON is corrupt -> returns full defaults.
/// - Fields are missing -> defaults for missing fields.
/// - Position is off every connected screen -> falls back to default position (x: None, y: None).
pub fn resolve_window_state(
    raw_json: &str,
    default_width: u32,
    default_height: u32,
    screens: &[ScreenBounds],
) -> WindowState {
    let mut state = match parse_window_state(raw_json) {
        Ok(parsed) => parsed,
        Err(_) => {
            return WindowState {
                x: None,
                y: None,
                width: default_width,
                height: default_height,
                maximized: false,
            };
        }
    };

    // If both x and y are present, verify visibility against connected monitors
    if let (Some(x), Some(y)) = (state.x, state.y) {
        let is_off_screen = !is_position_on_any_screen(x, y, state.width, state.height, screens);
        if is_off_screen {
            tracing::warn!(
                x = x,
                y = y,
                width = state.width,
                height = state.height,
                "Saved window position is off every connected display; falling back to default position"
            );
            state.x = None;
            state.y = None;
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_full_window_state() {
        let json = r#"{
            "x": 150,
            "y": 200,
            "width": 1440,
            "height": 900,
            "maximized": true
        }"#;

        let state = parse_window_state(json).expect("should parse valid state");
        assert_eq!(state.x, Some(150));
        assert_eq!(state.y, Some(200));
        assert_eq!(state.width, 1440);
        assert_eq!(state.height, 900);
        assert!(state.maximized);
    }

    #[test]
    fn test_parse_rejects_corrupt_json() {
        let bad_json_cases = [
            "",
            "   ",
            "not a json",
            "{ corrupt: true",
            "{\"x\": 100, \"y\": }",
            "[1, 2, 3]",
            "null",
        ];

        for &bad in &bad_json_cases {
            let res = parse_window_state(bad);
            assert!(
                res.is_err(),
                "Corrupt JSON '{bad}' must return WindowStateError::CorruptJson"
            );
            match res.unwrap_err() {
                WindowStateError::CorruptJson(msg) => {
                    assert!(!msg.is_empty());
                }
            }
        }
    }

    #[test]
    fn test_parse_ignores_unknown_fields() {
        let json = r#"{
            "x": 50,
            "y": 60,
            "width": 1000,
            "height": 700,
            "maximized": false,
            "unknownField": "ignored value",
            "futureSetting": 42,
            "nestedUnknown": { "foo": "bar" }
        }"#;

        let state = parse_window_state(json).expect("should ignore unknown fields");
        assert_eq!(state.x, Some(50));
        assert_eq!(state.y, Some(60));
        assert_eq!(state.width, 1000);
        assert_eq!(state.height, 700);
        assert!(!state.maximized);
    }

    #[test]
    fn test_parse_falls_back_to_defaults_on_missing_fields() {
        // Completely empty JSON object
        let empty_json = "{}";
        let state = parse_window_state(empty_json).expect("should parse empty object");
        assert_eq!(state.x, None);
        assert_eq!(state.y, None);
        assert_eq!(state.width, DEFAULT_SAVED_WIDTH);
        assert_eq!(state.height, DEFAULT_SAVED_HEIGHT);
        assert!(!state.maximized);

        // Only width provided
        let partial_json = r#"{"width": 800}"#;
        let p_state = parse_window_state(partial_json).expect("should parse partial");
        assert_eq!(p_state.x, None);
        assert_eq!(p_state.y, None);
        assert_eq!(p_state.width, 800);
        assert_eq!(p_state.height, DEFAULT_SAVED_HEIGHT);
        assert!(!p_state.maximized);
    }

    #[test]
    fn test_resolve_falls_back_when_off_screen() {
        let screens = [
            ScreenBounds::new(0, 0, 1920, 1080),
            ScreenBounds::new(1920, 0, 1920, 1080),
        ];

        // Valid on primary screen
        let valid_primary = r#"{"x": 100, "y": 100, "width": 800, "height": 600}"#;
        let resolved = resolve_window_state(valid_primary, 1200, 800, &screens);
        assert_eq!(resolved.x, Some(100));
        assert_eq!(resolved.y, Some(100));

        // Valid on secondary screen
        let valid_secondary = r#"{"x": 2000, "y": 100, "width": 800, "height": 600}"#;
        let resolved_sec = resolve_window_state(valid_secondary, 1200, 800, &screens);
        assert_eq!(resolved_sec.x, Some(2000));
        assert_eq!(resolved_sec.y, Some(100));

        // Way off screen (e.g. disconnected monitor at x: 8000)
        let offscreen_json = r#"{"x": 8000, "y": 8000, "width": 800, "height": 600}"#;
        let resolved_off = resolve_window_state(offscreen_json, 1200, 800, &screens);
        assert_eq!(
            resolved_off.x, None,
            "Off-screen X must fall back to default None"
        );
        assert_eq!(
            resolved_off.y, None,
            "Off-screen Y must fall back to default None"
        );
        assert_eq!(resolved_off.width, 800);
        assert_eq!(resolved_off.height, 600);

        // Completely off top/left
        let negative_off = r#"{"x": -5000, "y": -5000, "width": 800, "height": 600}"#;
        let resolved_neg = resolve_window_state(negative_off, 1200, 800, &screens);
        assert_eq!(resolved_neg.x, None);
        assert_eq!(resolved_neg.y, None);
    }

    #[test]
    fn test_resolve_corrupt_json_falls_back_to_defaults() {
        let screens = [ScreenBounds::new(0, 0, 1920, 1080)];
        let corrupt = "{ this is completely broken json";

        let resolved = resolve_window_state(corrupt, 1024, 768, &screens);
        assert_eq!(resolved.width, 1024);
        assert_eq!(resolved.height, 768);
        assert_eq!(resolved.x, None);
        assert_eq!(resolved.y, None);
        assert!(!resolved.maximized);
    }
}
