#![deny(unsafe_code)]

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use std::collections::HashMap;
use std::sync::Mutex;
use wavical_ipc::{Modifier, ParsedShortcut};

/// Convert a validated `ParsedShortcut` into a `global_hotkey::hotkey::HotKey`.
pub fn parsed_shortcut_to_hotkey(parsed: &ParsedShortcut) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    for m in &parsed.modifiers {
        match m {
            Modifier::Control => mods |= Modifiers::CONTROL,
            Modifier::Alt => mods |= Modifiers::ALT,
            Modifier::Shift => mods |= Modifiers::SHIFT,
            Modifier::Super => mods |= Modifiers::META,
        }
    }

    let code = match parsed.key.as_str() {
        "A" => Code::KeyA,
        "B" => Code::KeyB,
        "C" => Code::KeyC,
        "D" => Code::KeyD,
        "E" => Code::KeyE,
        "F" => Code::KeyF,
        "G" => Code::KeyG,
        "H" => Code::KeyH,
        "I" => Code::KeyI,
        "J" => Code::KeyJ,
        "K" => Code::KeyK,
        "L" => Code::KeyL,
        "M" => Code::KeyM,
        "N" => Code::KeyN,
        "O" => Code::KeyO,
        "P" => Code::KeyP,
        "Q" => Code::KeyQ,
        "R" => Code::KeyR,
        "S" => Code::KeyS,
        "T" => Code::KeyT,
        "U" => Code::KeyU,
        "V" => Code::KeyV,
        "W" => Code::KeyW,
        "X" => Code::KeyX,
        "Y" => Code::KeyY,
        "Z" => Code::KeyZ,
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "Space" => Code::Space,
        "Enter" => Code::Enter,
        "Tab" => Code::Tab,
        "Escape" => Code::Escape,
        "Backspace" => Code::Backspace,
        "Delete" => Code::Delete,
        "Insert" => Code::Insert,
        "ArrowUp" => Code::ArrowUp,
        "ArrowDown" => Code::ArrowDown,
        "ArrowLeft" => Code::ArrowLeft,
        "ArrowRight" => Code::ArrowRight,
        "Home" => Code::Home,
        "End" => Code::End,
        "PageUp" => Code::PageUp,
        "PageDown" => Code::PageDown,
        _ => return None,
    };

    Some(HotKey::new(Some(mods), code))
}

/// Global hotkey manager wrapper that maps hotkey IDs to canonical shortcut strings.
pub struct ShortcutManager {
    manager: global_hotkey::GlobalHotKeyManager,
    registered: Mutex<HashMap<u32, String>>,
}

impl ShortcutManager {
    /// Initialize a new shortcut manager with an active Win32 message loop.
    pub fn new() -> Result<Self, String> {
        let manager = global_hotkey::GlobalHotKeyManager::new()
            .map_err(|e| format!("Failed to initialize GlobalHotKeyManager: {e}"))?;
        Ok(Self {
            manager,
            registered: Mutex::new(HashMap::new()),
        })
    }

    /// Register a list of parsed shortcuts. If a shortcut conflicts with another app,
    /// a generic error is returned and a tracing event is logged without panicking.
    pub fn register_all(&self, shortcuts: &[ParsedShortcut]) -> Result<(), String> {
        let mut to_register = Vec::new();

        for parsed in shortcuts {
            let hotkey = parsed_shortcut_to_hotkey(parsed)
                .ok_or_else(|| format!("Unsupported shortcut key mapping: {}", parsed.canonical))?;
            to_register.push((hotkey, parsed.canonical.clone()));
        }

        // Register with global-hotkey manager
        for (hotkey, canonical) in to_register {
            match self.manager.register(hotkey) {
                Ok(_) => {
                    if let Ok(mut map) = self.registered.lock() {
                        map.insert(hotkey.id(), canonical);
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        shortcut = %canonical,
                        error = %err,
                        "Global shortcut registration conflict with another application"
                    );
                    return Err(format!(
                        "Global shortcut '{canonical}' could not be registered: shortcut is in use by another application"
                    ));
                }
            }
        }

        Ok(())
    }

    /// Lookup the canonical shortcut string for a triggered hotkey ID.
    pub fn get_shortcut_name(&self, hotkey_id: u32) -> Option<String> {
        self.registered.lock().ok()?.get(&hotkey_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsed_shortcut_to_hotkey_conversion() {
        let parsed = wavical_ipc::parse_and_validate_shortcut("Ctrl+Shift+K").unwrap();
        let hotkey = parsed_shortcut_to_hotkey(&parsed).expect("valid conversion");
        assert_eq!(hotkey.key, Code::KeyK);
        let mods = hotkey.mods;
        assert!(mods.contains(Modifiers::CONTROL));
        assert!(mods.contains(Modifiers::SHIFT));
    }
}
