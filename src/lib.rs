//! TrayClick core logic: the click engine state machine, the keybind
//! model, and config persistence formatting.
//!
//! This module is platform-independent so it can be unit-tested anywhere.
//! All Win32 code lives in `main.rs`.

/// How often the mouse is clicked while clicking is on.
pub const CLICK_INTERVAL_MS: u64 = 100;

/// Default toggle key: F6 (`VK_F6`).
pub const DEFAULT_KEYBIND_VK: u32 = 0x75;

/// `VK_ESCAPE` — reserved in the capture window to mean "cancel", so it
/// can never be assigned as the keybind.
pub const VK_ESCAPE_CODE: u32 = 0x1B;

/// The click engine's state.
///
/// Two independent switches:
/// - `clicking` — toggled by the keybind; the mouse clicks every
///   [`CLICK_INTERVAL_MS`] ms while this is on.
/// - `paused` — toggled from the tray menu. Pausing stops any clicking in
///   progress and makes the keybind do nothing until resumed. Resuming
///   does **not** restart clicking on its own; press the keybind again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Engine {
    pub clicking: bool,
    pub paused: bool,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle one press of the keybind. Ignored while paused.
    pub fn press_keybind(&mut self) {
        if self.paused {
            return;
        }
        self.clicking = !self.clicking;
    }

    /// Handle the tray menu's Pause/Resume item. Pausing always stops
    /// clicking; resuming leaves clicking off until the keybind is pressed.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if self.paused {
            self.clicking = false;
        }
    }

    /// True when the click thread should actually be sending clicks.
    pub fn is_clicking_now(&self) -> bool {
        self.clicking && !self.paused
    }
}

/// True if `vk` may be assigned as the keybind.
///
/// Rejects mouse buttons (the clicker itself would retrigger them),
/// `Esc` (it cancels the capture window), and out-of-range codes.
pub fn is_valid_keybind(vk: u32) -> bool {
    if vk == VK_ESCAPE_CODE {
        return false;
    }
    // 0x01..=0x06 are the mouse buttons and the reserved "unassigned" code.
    if vk < 0x08 {
        return false;
    }
    vk <= 0xFE
}

/// A human-readable name for a virtual-key code, e.g. `F6`, `A`, `Space`.
pub fn key_name(vk: u32) -> String {
    match vk {
        0x08 => "Backspace".to_string(),
        0x09 => "Tab".to_string(),
        0x0D => "Enter".to_string(),
        0x10 => "Shift".to_string(),
        0x11 => "Ctrl".to_string(),
        0x12 => "Alt".to_string(),
        0x14 => "Caps Lock".to_string(),
        0x20 => "Space".to_string(),
        0x21 => "Page Up".to_string(),
        0x22 => "Page Down".to_string(),
        0x23 => "End".to_string(),
        0x24 => "Home".to_string(),
        0x25 => "Left".to_string(),
        0x26 => "Up".to_string(),
        0x27 => "Right".to_string(),
        0x28 => "Down".to_string(),
        0x2D => "Insert".to_string(),
        0x2E => "Delete".to_string(),
        0x5B => "Left Win".to_string(),
        0x5C => "Right Win".to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x30..=0x39 => ((vk as u8) as char).to_string(),
        0x41..=0x5A => ((vk as u8) as char).to_string(),
        _ => format!("Key 0x{vk:02X}"),
    }
}

/// Persisted settings: just the keybind, everything else is fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub keybind_vk: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keybind_vk: DEFAULT_KEYBIND_VK,
        }
    }
}

impl Config {
    /// Serialize as a small hand-rolled JSON object (no dependencies).
    pub fn to_json(&self) -> String {
        format!("{{\n  \"keybind_vk\": {}\n}}\n", self.keybind_vk)
    }

    /// Parse a config written by [`Config::to_json`]. Returns `None` for
    /// anything malformed or with an invalid keybind, so the caller can
    /// fall back to defaults without touching the user's file.
    pub fn from_json(text: &str) -> Option<Self> {
        let key = "\"keybind_vk\"";
        let start = text.find(key)? + key.len();
        let rest = text[start..].trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let vk: u32 = digits.parse().ok()?;
        if !is_valid_keybind(vk) {
            return None;
        }
        Some(Self { keybind_vk: vk })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keybind_toggles_clicking_on_and_off() {
        let mut e = Engine::new();
        assert!(!e.is_clicking_now());
        e.press_keybind();
        assert!(e.clicking && e.is_clicking_now());
        e.press_keybind();
        assert!(!e.clicking && !e.is_clicking_now());
    }

    #[test]
    fn pause_stops_clicking_and_blocks_keybind() {
        let mut e = Engine::new();
        e.press_keybind();
        assert!(e.is_clicking_now());
        e.toggle_pause();
        assert!(e.paused && !e.clicking && !e.is_clicking_now());
        // Keybind does nothing while paused.
        e.press_keybind();
        assert!(!e.clicking && !e.is_clicking_now());
    }

    #[test]
    fn resume_does_not_restart_clicking_by_itself() {
        let mut e = Engine::new();
        e.press_keybind();
        e.toggle_pause();
        e.toggle_pause();
        assert!(!e.paused);
        assert!(!e.is_clicking_now());
        e.press_keybind();
        assert!(e.is_clicking_now());
    }

    #[test]
    fn keybind_validation() {
        assert!(is_valid_keybind(0x75)); // F6
        assert!(is_valid_keybind(0x41)); // A
        assert!(is_valid_keybind(0x20)); // Space
        assert!(!is_valid_keybind(VK_ESCAPE_CODE));
        assert!(!is_valid_keybind(0x01)); // left mouse button
        assert!(!is_valid_keybind(0x02)); // right mouse button
        assert!(!is_valid_keybind(0x04)); // middle mouse button
        assert!(!is_valid_keybind(0x00));
        assert!(!is_valid_keybind(0xFF));
    }

    #[test]
    fn key_names() {
        assert_eq!(key_name(0x75), "F6");
        assert_eq!(key_name(0x70), "F1");
        assert_eq!(key_name(0x87), "F24");
        assert_eq!(key_name(0x41), "A");
        assert_eq!(key_name(0x35), "5");
        assert_eq!(key_name(0x20), "Space");
        assert_eq!(key_name(0x0D), "Enter");
        assert_eq!(key_name(0x2E), "Delete");
    }

    #[test]
    fn config_round_trip() {
        let cfg = Config { keybind_vk: 0x77 }; // F8
        let json = cfg.to_json();
        assert_eq!(Config::from_json(&json), Some(cfg));
    }

    #[test]
    fn config_default_is_f6() {
        assert_eq!(Config::default().keybind_vk, DEFAULT_KEYBIND_VK);
        assert_eq!(key_name(DEFAULT_KEYBIND_VK), "F6");
    }

    #[test]
    fn config_rejects_garbage() {
        assert_eq!(Config::from_json(""), None);
        assert_eq!(Config::from_json("{}"), None);
        assert_eq!(Config::from_json("{\"keybind_vk\": }"), None);
        assert_eq!(Config::from_json("{\"keybind_vk\": 27}"), None); // Esc
        assert_eq!(Config::from_json("{\"keybind_vk\": 1}"), None); // mouse
        assert_eq!(Config::from_json("{\"keybind_vk\": 99999}"), None);
    }

    #[test]
    fn interval_is_100ms() {
        assert_eq!(CLICK_INTERVAL_MS, 100);
    }
}
