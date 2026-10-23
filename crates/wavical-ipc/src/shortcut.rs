#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

/// Maximum number of global shortcuts an application can register concurrently.
pub const MAX_APP_SHORTCUTS: usize = 32;

/// Maximum length of a single shortcut specification string.
pub const MAX_SHORTCUT_STRING_CHARS: usize = 64;

/// Recognized shortcut keyboard modifier keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Modifier {
    Control,
    Alt,
    Shift,
    Super,
}

impl Modifier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Control => "Ctrl",
            Self::Alt => "Alt",
            Self::Shift => "Shift",
            Self::Super => "Super",
        }
    }
}

/// Parsed and validated representation of a single global keyboard shortcut.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ParsedShortcut {
    pub modifiers: Vec<Modifier>,
    pub key: String,
    pub canonical: String,
}

/// Errors during shortcut specification parsing and validation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ShortcutValidationError {
    #[error("shortcut string cannot be empty")]
    Empty,

    #[error(
        "shortcut string exceeds maximum length of {MAX_SHORTCUT_STRING_CHARS} characters (got {0})"
    )]
    TooLong(usize),

    #[error("shortcut must include at least one modifier key (Ctrl, Alt, Shift, Super)")]
    MissingModifier,

    #[error("shortcut missing base key specification")]
    MissingKey,

    #[error("duplicate modifier '{0}' within shortcut")]
    DuplicateModifier(String),

    #[error("unknown or invalid modifier '{0}' (expected Ctrl, Alt, Shift, or Super)")]
    InvalidModifier(String),

    #[error("unknown or invalid key '{0}'")]
    InvalidKey(String),

    #[error("too many registered shortcuts (got {0}, maximum allowed is {MAX_APP_SHORTCUTS})")]
    TooManyShortcuts(usize),

    #[error("duplicate shortcut '{0}' in registration list")]
    DuplicateShortcut(String),

    #[error("invalid shortcut parameter payload: {0}")]
    InvalidParams(String),
}

fn parse_modifier(token: &str) -> Option<Modifier> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Modifier::Control),
        "alt" => Some(Modifier::Alt),
        "shift" => Some(Modifier::Shift),
        "super" | "meta" | "cmd" | "command" | "win" => Some(Modifier::Super),
        _ => None,
    }
}

fn normalize_key(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Single character A-Z or 0-9
    if trimmed.len() == 1 {
        let c = trimmed.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(c.to_ascii_uppercase().to_string());
        }
        if c.is_ascii_digit() {
            return Some(c.to_string());
        }
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "f1" => Some("F1".to_string()),
        "f2" => Some("F2".to_string()),
        "f3" => Some("F3".to_string()),
        "f4" => Some("F4".to_string()),
        "f5" => Some("F5".to_string()),
        "f6" => Some("F6".to_string()),
        "f7" => Some("F7".to_string()),
        "f8" => Some("F8".to_string()),
        "f9" => Some("F9".to_string()),
        "f10" => Some("F10".to_string()),
        "f11" => Some("F11".to_string()),
        "f12" => Some("F12".to_string()),
        "space" => Some("Space".to_string()),
        "enter" | "return" => Some("Enter".to_string()),
        "tab" => Some("Tab".to_string()),
        "escape" | "esc" => Some("Escape".to_string()),
        "backspace" => Some("Backspace".to_string()),
        "delete" | "del" => Some("Delete".to_string()),
        "insert" | "ins" => Some("Insert".to_string()),
        "arrowup" | "up" => Some("ArrowUp".to_string()),
        "arrowdown" | "down" => Some("ArrowDown".to_string()),
        "arrowleft" | "left" => Some("ArrowLeft".to_string()),
        "arrowright" | "right" => Some("ArrowRight".to_string()),
        "home" => Some("Home".to_string()),
        "end" => Some("End".to_string()),
        "pageup" => Some("PageUp".to_string()),
        "pagedown" => Some("PageDown".to_string()),
        _ => None,
    }
}

/// Parse and validate a single shortcut string according to the fixed grammar.
pub fn parse_and_validate_shortcut(s: &str) -> Result<ParsedShortcut, ShortcutValidationError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(ShortcutValidationError::Empty);
    }
    if trimmed.chars().count() > MAX_SHORTCUT_STRING_CHARS {
        return Err(ShortcutValidationError::TooLong(trimmed.chars().count()));
    }

    let parts: Vec<&str> = trimmed.split('+').map(|p| p.trim()).collect();
    if parts.is_empty() {
        return Err(ShortcutValidationError::Empty);
    }
    if parts.len() < 2 {
        if parse_modifier(parts[0]).is_some() {
            return Err(ShortcutValidationError::MissingKey);
        }
        return Err(ShortcutValidationError::MissingModifier);
    }

    let key_token = parts[parts.len() - 1];
    let key = normalize_key(key_token)
        .ok_or_else(|| ShortcutValidationError::InvalidKey(key_token.to_string()))?;

    let mut modifier_set = BTreeSet::new();
    for &mod_token in &parts[..parts.len() - 1] {
        if mod_token.is_empty() {
            return Err(ShortcutValidationError::InvalidModifier(
                mod_token.to_string(),
            ));
        }
        let modifier = parse_modifier(mod_token)
            .ok_or_else(|| ShortcutValidationError::InvalidModifier(mod_token.to_string()))?;

        if !modifier_set.insert(modifier) {
            return Err(ShortcutValidationError::DuplicateModifier(
                mod_token.to_string(),
            ));
        }
    }

    if modifier_set.is_empty() {
        return Err(ShortcutValidationError::MissingModifier);
    }

    let modifiers: Vec<Modifier> = modifier_set.into_iter().collect();

    // Canonical representation: Ctrl+Alt+Shift+Super+Key
    let mut canonical_parts: Vec<&str> = modifiers.iter().map(|m| m.as_str()).collect();
    canonical_parts.push(&key);
    let canonical = canonical_parts.join("+");

    Ok(ParsedShortcut {
        modifiers,
        key,
        canonical,
    })
}

/// Validate a list of shortcut strings. Enforces maximum count and rejects duplicates.
pub fn validate_shortcut_list(
    shortcuts: &[String],
) -> Result<Vec<ParsedShortcut>, ShortcutValidationError> {
    if shortcuts.is_empty() {
        return Err(ShortcutValidationError::Empty);
    }
    if shortcuts.len() > MAX_APP_SHORTCUTS {
        return Err(ShortcutValidationError::TooManyShortcuts(shortcuts.len()));
    }

    let mut parsed_list = Vec::with_capacity(shortcuts.len());
    let mut seen_canonical = BTreeSet::new();

    for raw in shortcuts {
        let parsed = parse_and_validate_shortcut(raw)?;
        if !seen_canonical.insert(parsed.canonical.clone()) {
            return Err(ShortcutValidationError::DuplicateShortcut(parsed.canonical));
        }
        parsed_list.push(parsed);
    }

    Ok(parsed_list)
}

/// Parse and validate shortcut payload from JSON-RPC parameters.
pub fn parse_and_validate_shortcut_params(
    params: Option<&serde_json::Value>,
) -> Result<Vec<ParsedShortcut>, ShortcutValidationError> {
    let val = params.ok_or_else(|| {
        ShortcutValidationError::InvalidParams("missing shortcut parameters".to_string())
    })?;

    if let Some(s) = val.as_str() {
        let parsed = parse_and_validate_shortcut(s)?;
        return Ok(vec![parsed]);
    }

    if let Some(arr) = val.as_array() {
        let mut list = Vec::new();
        for item in arr {
            let s = item.as_str().ok_or_else(|| {
                ShortcutValidationError::InvalidParams("shortcut array item must be string".into())
            })?;
            list.push(s.to_string());
        }
        return validate_shortcut_list(&list);
    }

    if let Some(obj) = val.as_object() {
        if let Some(s) = obj.get("shortcut").and_then(|v| v.as_str()) {
            let parsed = parse_and_validate_shortcut(s)?;
            return Ok(vec![parsed]);
        }
        if let Some(arr) = obj.get("shortcuts").and_then(|v| v.as_array()) {
            let mut list = Vec::new();
            for item in arr {
                let s = item.as_str().ok_or_else(|| {
                    ShortcutValidationError::InvalidParams(
                        "shortcuts array item must be string".into(),
                    )
                })?;
                list.push(s.to_string());
            }
            return validate_shortcut_list(&list);
        }
    }

    Err(ShortcutValidationError::InvalidParams(
        "expected shortcut string or list of shortcut strings".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_shortcuts_parsing() {
        let s1 = parse_and_validate_shortcut("Ctrl+Shift+K").unwrap();
        assert_eq!(s1.canonical, "Ctrl+Shift+K");
        assert_eq!(s1.modifiers, vec![Modifier::Control, Modifier::Shift]);
        assert_eq!(s1.key, "K");

        // Case insensitivity & aliases
        let s2 = parse_and_validate_shortcut("control+alt+k").unwrap();
        assert_eq!(s2.canonical, "Ctrl+Alt+K");

        let s3 = parse_and_validate_shortcut("Cmd+Shift+P").unwrap();
        assert_eq!(s3.canonical, "Shift+Super+P");

        let s4 = parse_and_validate_shortcut("Alt+F4").unwrap();
        assert_eq!(s4.canonical, "Alt+F4");

        let s5 = parse_and_validate_shortcut("Ctrl+Space").unwrap();
        assert_eq!(s5.canonical, "Ctrl+Space");

        let s6 = parse_and_validate_shortcut("Super+Left").unwrap();
        assert_eq!(s6.canonical, "Super+ArrowLeft");
    }

    #[test]
    fn test_missing_modifier_rejected() {
        assert_eq!(
            parse_and_validate_shortcut("K"),
            Err(ShortcutValidationError::MissingModifier)
        );
        assert_eq!(
            parse_and_validate_shortcut("Space"),
            Err(ShortcutValidationError::MissingModifier)
        );
    }

    #[test]
    fn test_missing_key_rejected() {
        assert_eq!(
            parse_and_validate_shortcut("Ctrl+"),
            Err(ShortcutValidationError::InvalidKey("".into()))
        );
        assert_eq!(
            parse_and_validate_shortcut("Ctrl"),
            Err(ShortcutValidationError::MissingKey)
        );
    }

    #[test]
    fn test_invalid_modifier_rejected() {
        assert_eq!(
            parse_and_validate_shortcut("Hyper+K"),
            Err(ShortcutValidationError::InvalidModifier("Hyper".into()))
        );
    }

    #[test]
    fn test_invalid_key_rejected() {
        assert_eq!(
            parse_and_validate_shortcut("Ctrl+InvalidKey"),
            Err(ShortcutValidationError::InvalidKey("InvalidKey".into()))
        );
    }

    #[test]
    fn test_duplicate_modifier_in_shortcut_rejected() {
        assert_eq!(
            parse_and_validate_shortcut("Ctrl+Ctrl+K"),
            Err(ShortcutValidationError::DuplicateModifier("Ctrl".into()))
        );
        assert_eq!(
            parse_and_validate_shortcut("Control+Ctrl+K"),
            Err(ShortcutValidationError::DuplicateModifier("Ctrl".into()))
        );
    }

    #[test]
    fn test_empty_and_too_long() {
        assert_eq!(
            parse_and_validate_shortcut(""),
            Err(ShortcutValidationError::Empty)
        );
        let long = format!("Ctrl+Alt+Shift+{}", "A".repeat(60));
        assert!(matches!(
            parse_and_validate_shortcut(&long),
            Err(ShortcutValidationError::TooLong(_))
        ));
    }

    #[test]
    fn test_validate_shortcut_list_duplicates_rejected() {
        let list = vec!["Ctrl+K".to_string(), "ctrl+k".to_string()];
        assert_eq!(
            validate_shortcut_list(&list),
            Err(ShortcutValidationError::DuplicateShortcut(
                "Ctrl+K".to_string()
            ))
        );
    }

    #[test]
    fn test_validate_shortcut_list_max_32() {
        let mut list = Vec::new();
        // Generate 32 valid unique shortcuts
        for i in 0..26 {
            let c = (b'A' + i) as char;
            list.push(format!("Ctrl+{c}"));
        }
        for i in 1..=6 {
            list.push(format!("Ctrl+F{i}"));
        }
        assert_eq!(list.len(), 32);
        assert!(validate_shortcut_list(&list).is_ok());

        // 33rd shortcut exceeds MAX_APP_SHORTCUTS
        list.push("Ctrl+F7".to_string());
        assert_eq!(
            validate_shortcut_list(&list),
            Err(ShortcutValidationError::TooManyShortcuts(33))
        );
    }

    #[test]
    fn test_parse_shortcut_json_params() {
        let p1 =
            parse_and_validate_shortcut_params(Some(&serde_json::json!("Ctrl+Shift+P"))).unwrap();
        assert_eq!(p1.len(), 1);
        assert_eq!(p1[0].canonical, "Ctrl+Shift+P");

        let p2 = parse_and_validate_shortcut_params(Some(&serde_json::json!({
            "shortcuts": ["Ctrl+Alt+Delete", "Alt+F4"]
        })))
        .unwrap();
        assert_eq!(p2.len(), 2);
    }
}
