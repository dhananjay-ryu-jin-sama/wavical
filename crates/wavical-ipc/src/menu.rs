#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

/// Maximum number of menu items allowed in a single menu.
pub const MAX_MENU_ITEMS: usize = 200;

/// Maximum length of a menu item label in characters.
pub const MAX_MENU_LABEL_CHARS: usize = 100;

/// Maximum length of a menu item ID in characters.
pub const MAX_MENU_ITEM_ID_CHARS: usize = 64;

/// Maximum length of a tray tooltip in characters.
pub const MAX_TRAY_TOOLTIP_CHARS: usize = 128;

/// Generic error message returned to the page on menu failures.
pub const GENERIC_MENU_ERROR: &str = "Menu operation failed";

fn default_true() -> bool {
    true
}

/// A validated menu item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub separator: bool,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
            separator: false,
        }
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn separator(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            enabled: true,
            separator: true,
        }
    }
}

/// Parameters for `tray.set`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraySetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    pub items: Vec<MenuItem>,
}

/// Parameters for `menu.showContextMenu`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMenuParams {
    pub items: Vec<MenuItem>,
}

/// Detailed validation error for menu items and tray parameters.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MenuValidationError {
    #[error("Menu items list cannot be empty")]
    EmptyList,

    #[error("Menu item count ({count}) exceeds maximum allowed ({max})")]
    TooManyItems { count: usize, max: usize },

    #[error("Menu item ID cannot be empty")]
    EmptyItemId,

    #[error("Menu item ID '{id}' exceeds maximum length of {max} characters")]
    ItemIdTooLong { id: String, max: usize },

    #[error("Menu item ID '{id}' contains invalid character '{char}'; must only match [a-z0-9._-]")]
    InvalidItemIdCharacter { id: String, char: char },

    #[error("Duplicate menu item ID detected: '{0}'")]
    DuplicateItemId(String),

    #[error("Menu item '{id}' label exceeds maximum length of {max} characters")]
    LabelTooLong { id: String, max: usize },

    #[error("Tray tooltip exceeds maximum length of {max} characters")]
    TooltipTooLong { max: usize },

    #[error("Failed to parse menu parameters: {0}")]
    ParseError(String),
}

/// Check if a character is valid in a menu item ID: [a-z0-9._-]
pub fn is_valid_item_id_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '-')
}

/// Pure validation of a slice of menu items against all security rules.
pub fn validate_menu_items(items: &[MenuItem]) -> Result<(), MenuValidationError> {
    if items.is_empty() {
        return Err(MenuValidationError::EmptyList);
    }

    if items.len() > MAX_MENU_ITEMS {
        return Err(MenuValidationError::TooManyItems {
            count: items.len(),
            max: MAX_MENU_ITEMS,
        });
    }

    let mut seen_ids = BTreeSet::new();

    for item in items {
        // Validate ID
        if item.id.is_empty() {
            return Err(MenuValidationError::EmptyItemId);
        }

        if item.id.chars().count() > MAX_MENU_ITEM_ID_CHARS {
            return Err(MenuValidationError::ItemIdTooLong {
                id: item.id.clone(),
                max: MAX_MENU_ITEM_ID_CHARS,
            });
        }

        for c in item.id.chars() {
            if !is_valid_item_id_char(c) {
                return Err(MenuValidationError::InvalidItemIdCharacter {
                    id: item.id.clone(),
                    char: c,
                });
            }
        }

        if !seen_ids.insert(&item.id) {
            return Err(MenuValidationError::DuplicateItemId(item.id.clone()));
        }

        // Validate label length
        if item.label.chars().count() > MAX_MENU_LABEL_CHARS {
            return Err(MenuValidationError::LabelTooLong {
                id: item.id.clone(),
                max: MAX_MENU_LABEL_CHARS,
            });
        }
    }

    Ok(())
}

/// Parse and validate `tray.set` parameters from JSON.
pub fn parse_and_validate_tray_params(
    value: &serde_json::Value,
) -> Result<TraySetParams, MenuValidationError> {
    let params: TraySetParams = if value.is_array() {
        let items: Vec<MenuItem> = serde_json::from_value(value.clone())
            .map_err(|e| MenuValidationError::ParseError(e.to_string()))?;
        TraySetParams {
            tooltip: None,
            items,
        }
    } else {
        serde_json::from_value(value.clone())
            .map_err(|e| MenuValidationError::ParseError(e.to_string()))?
    };

    if let Some(ref tip) = params.tooltip
        && tip.chars().count() > MAX_TRAY_TOOLTIP_CHARS
    {
        return Err(MenuValidationError::TooltipTooLong {
            max: MAX_TRAY_TOOLTIP_CHARS,
        });
    }

    validate_menu_items(&params.items)?;
    Ok(params)
}

/// Parse and validate `menu.showContextMenu` parameters from JSON.
pub fn parse_and_validate_context_menu_params(
    value: &serde_json::Value,
) -> Result<ContextMenuParams, MenuValidationError> {
    let params: ContextMenuParams = if value.is_array() {
        let items: Vec<MenuItem> = serde_json::from_value(value.clone())
            .map_err(|e| MenuValidationError::ParseError(e.to_string()))?;
        ContextMenuParams { items }
    } else {
        serde_json::from_value(value.clone())
            .map_err(|e| MenuValidationError::ParseError(e.to_string()))?
    };

    validate_menu_items(&params.items)?;
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_menu_items() {
        let items = vec![
            MenuItem::new("item.1", "First Item"),
            MenuItem::new("item_2", "Second Item").with_enabled(false),
            MenuItem::separator("sep-1"),
        ];
        assert!(validate_menu_items(&items).is_ok());
    }

    #[test]
    fn test_validation_empty_list_rejected() {
        let err = validate_menu_items(&[]).expect_err("Empty menu list must be rejected");
        assert_eq!(err, MenuValidationError::EmptyList);
    }

    #[test]
    fn test_validation_max_items_limit() {
        // Exactly 200 items -> Ok
        let items_200: Vec<MenuItem> = (0..200)
            .map(|i| MenuItem::new(format!("item_{i}"), format!("Label {i}")))
            .collect();
        assert!(validate_menu_items(&items_200).is_ok());

        // 201 items -> TooManyItems
        let items_201: Vec<MenuItem> = (0..201)
            .map(|i| MenuItem::new(format!("item_{i}"), format!("Label {i}")))
            .collect();
        let err = validate_menu_items(&items_201).expect_err("201 items must be rejected");
        match err {
            MenuValidationError::TooManyItems { count, max } => {
                assert_eq!(count, 201);
                assert_eq!(max, 200);
            }
            other => panic!("Unexpected error: {other:?}"),
        }
    }

    #[test]
    fn test_validation_duplicate_item_ids_rejected() {
        let items = vec![
            MenuItem::new("duplicate.id", "Item A"),
            MenuItem::new("unique.id", "Item B"),
            MenuItem::new("duplicate.id", "Item C"),
        ];
        let err = validate_menu_items(&items).expect_err("Duplicate IDs must be rejected");
        assert_eq!(
            err,
            MenuValidationError::DuplicateItemId("duplicate.id".to_string())
        );
    }

    #[test]
    fn test_validation_bad_characters_in_id_rejected() {
        // Uppercase not allowed [a-z0-9._-]
        let items_upper = vec![MenuItem::new("ItemBad", "Label")];
        let err = validate_menu_items(&items_upper).expect_err("Uppercase must be rejected");
        assert!(matches!(
            err,
            MenuValidationError::InvalidItemIdCharacter { char: 'I', .. }
        ));

        // Space not allowed
        let items_space = vec![MenuItem::new("item bad", "Label")];
        let err2 = validate_menu_items(&items_space).expect_err("Space must be rejected");
        assert!(matches!(
            err2,
            MenuValidationError::InvalidItemIdCharacter { char: ' ', .. }
        ));

        // Special symbols like @ or ! or / not allowed
        let items_symbol = vec![MenuItem::new("item/bad", "Label")];
        let err3 = validate_menu_items(&items_symbol).expect_err("Slash must be rejected");
        assert!(matches!(
            err3,
            MenuValidationError::InvalidItemIdCharacter { char: '/', .. }
        ));
    }

    #[test]
    fn test_validation_empty_item_id_rejected() {
        let items = vec![MenuItem::new("", "Label")];
        let err = validate_menu_items(&items).expect_err("Empty item ID must be rejected");
        assert_eq!(err, MenuValidationError::EmptyItemId);
    }

    #[test]
    fn test_validation_item_id_max_length() {
        // Exactly 64 chars -> Ok
        let id_64 = "a".repeat(64);
        let items_ok = vec![MenuItem::new(id_64, "Label")];
        assert!(validate_menu_items(&items_ok).is_ok());

        // 65 chars -> ItemIdTooLong
        let id_65 = "a".repeat(65);
        let items_bad = vec![MenuItem::new(id_65.clone(), "Label")];
        let err = validate_menu_items(&items_bad).expect_err("65 chars ID must be rejected");
        match err {
            MenuValidationError::ItemIdTooLong { id, max } => {
                assert_eq!(id, id_65);
                assert_eq!(max, 64);
            }
            other => panic!("Unexpected error: {other:?}"),
        }
    }

    #[test]
    fn test_validation_label_max_length() {
        // Exactly 100 chars -> Ok
        let label_100 = "L".repeat(100);
        let items_ok = vec![MenuItem::new("item_1", label_100)];
        assert!(validate_menu_items(&items_ok).is_ok());

        // 101 chars -> LabelTooLong
        let label_101 = "L".repeat(101);
        let items_bad = vec![MenuItem::new("item_1", label_101)];
        let err = validate_menu_items(&items_bad).expect_err("101 chars label must be rejected");
        assert!(matches!(
            err,
            MenuValidationError::LabelTooLong { max: 100, .. }
        ));
    }

    #[test]
    fn test_tray_params_parsing_object_and_array() {
        // Array format
        let json_arr = serde_json::json!([
            {"id": "item.one", "label": "One"},
            {"id": "item.two", "label": "Two"}
        ]);
        let parsed =
            parse_and_validate_tray_params(&json_arr).expect("Array parsing should succeed");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.tooltip, None);

        // Object format with tooltip
        let json_obj = serde_json::json!({
            "tooltip": "My App Tooltip",
            "items": [
                {"id": "quit", "label": "Quit App"}
            ]
        });
        let parsed_obj =
            parse_and_validate_tray_params(&json_obj).expect("Object parsing should succeed");
        assert_eq!(parsed_obj.items.len(), 1);
        assert_eq!(parsed_obj.tooltip.as_deref(), Some("My App Tooltip"));

        // Oversized tooltip > 128 chars
        let long_tip = "T".repeat(129);
        let json_long_tip = serde_json::json!({
            "tooltip": long_tip,
            "items": [{"id": "item", "label": "Item"}]
        });
        assert!(parse_and_validate_tray_params(&json_long_tip).is_err());
    }

    #[test]
    fn test_context_menu_params_parsing() {
        let json_arr = serde_json::json!([
            {"id": "copy", "label": "Copy"},
            {"id": "paste", "label": "Paste"}
        ]);
        let parsed =
            parse_and_validate_context_menu_params(&json_arr).expect("Parsing should succeed");
        assert_eq!(parsed.items.len(), 2);

        let json_obj = serde_json::json!({
            "items": [
                {"id": "inspect", "label": "Inspect Element"}
            ]
        });
        let parsed_obj =
            parse_and_validate_context_menu_params(&json_obj).expect("Parsing should succeed");
        assert_eq!(parsed_obj.items.len(), 1);
    }
}
