//! TrayClick core logic: the click engine state machine, the keybind
//! model, click scheduling, and config persistence formatting.
//!
//! This module is platform-independent so it can be unit-tested anywhere.
//! All Win32 code lives in `main.rs`.

use std::time::{Duration, Instant};

/// Default click interval: the mouse clicks every 100 ms while on.
pub const DEFAULT_INTERVAL_MS: u32 = 100;

/// Smallest interval the user may set (1 ms).
pub const MIN_INTERVAL_MS: u32 = 1;

/// Largest interval the user may set (60 s).
pub const MAX_INTERVAL_MS: u32 = 60_000;

/// True if `ms` may be used as the click interval.
pub fn is_valid_interval(ms: u32) -> bool {
    (MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&ms)
}

/// Compute the deadline for the click after `previous`, on a fixed
/// cadence of one click every `interval`.
///
/// Scheduling from the previous deadline (instead of sleeping
/// `interval` after each click) keeps the long-run rate exact: the time
/// a click itself takes no longer accumulates as drift. If the schedule
/// has already fallen a full interval or more behind `now` — the PC
/// slept, or the process was stalled — the next deadline is rebased to
/// `now + interval` rather than firing catch-up clicks in a burst.
pub fn next_click_deadline(previous: Instant, interval: Duration, now: Instant) -> Instant {
    let next = previous + interval;
    if next <= now {
        now + interval
    } else {
        next
    }
}

/// Default toggle key: F6 (`VK_F6`).
pub const DEFAULT_KEYBIND_VK: u32 = 0x75;

/// `VK_ESCAPE` — reserved in the capture window to mean "cancel", so it
/// can never be assigned as the keybind.
pub const VK_ESCAPE_CODE: u32 = 0x1B;

/// The click engine's state.
///
/// Two independent switches:
/// - `clicking` — toggled by the keybind; while this is on, the mouse
///   clicks at the configured interval (default [`DEFAULT_INTERVAL_MS`]).
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

/// Turns a polled key state into toggle presses (down-edges).
///
/// The detector can be *resynced*: the next sample only sets the baseline
/// and never counts as a press. This matters when the Set Keybind window
/// closes — the user is still physically holding the key they just
/// assigned, and that keypress must not toggle clicking. Clicking may
/// only start when the key is released and pressed again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPressDetector {
    was_down: bool,
    resync_pending: bool,
}

impl Default for KeyPressDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyPressDetector {
    /// A new detector resyncs on its first sample, so a key already held
    /// when polling starts does not count as a press either.
    pub fn new() -> Self {
        Self {
            was_down: false,
            resync_pending: true,
        }
    }

    /// The next [`KeyPressDetector::sample`] sets the baseline only.
    pub fn resync(&mut self) {
        self.resync_pending = true;
    }

    /// Feed the key's current physical state; returns `true` exactly once
    /// per genuine press (up → down transition outside a resync).
    pub fn sample(&mut self, down: bool) -> bool {
        if self.resync_pending {
            self.resync_pending = false;
            self.was_down = down;
            return false;
        }
        let pressed = down && !self.was_down;
        self.was_down = down;
        pressed
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

/// Persisted settings: the keybind and the click interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub keybind_vk: u32,
    pub interval_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keybind_vk: DEFAULT_KEYBIND_VK,
            interval_ms: DEFAULT_INTERVAL_MS,
        }
    }
}

/// Read one unsigned-integer field out of the hand-rolled JSON object.
fn json_u32(text: &str, key: &str) -> Option<u32> {
    let start = text.find(key)? + key.len();
    let rest = text[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

impl Config {
    /// Serialize as a small hand-rolled JSON object (no dependencies).
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"keybind_vk\": {},\n  \"interval_ms\": {}\n}}\n",
            self.keybind_vk, self.interval_ms
        )
    }

    /// Parse a config written by [`Config::to_json`]. Returns `None` for
    /// anything malformed or with invalid values, so the caller can fall
    /// back to defaults without touching the user's file. A missing
    /// `interval_ms` (a v0.1.x config file) defaults to
    /// [`DEFAULT_INTERVAL_MS`].
    pub fn from_json(text: &str) -> Option<Self> {
        let vk = json_u32(text, "\"keybind_vk\"")?;
        if !is_valid_keybind(vk) {
            return None;
        }
        let interval_ms = match json_u32(text, "\"interval_ms\"") {
            Some(ms) => {
                if !is_valid_interval(ms) {
                    return None;
                }
                ms
            }
            None => DEFAULT_INTERVAL_MS,
        };
        Some(Self {
            keybind_vk: vk,
            interval_ms,
        })
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
        let cfg = Config {
            keybind_vk: 0x77, // F8
            interval_ms: 250,
        };
        let json = cfg.to_json();
        assert_eq!(Config::from_json(&json), Some(cfg));
    }

    #[test]
    fn config_without_interval_uses_default() {
        // A v0.1.x config file has no interval_ms field.
        let cfg = Config::from_json("{\"keybind_vk\": 119}").unwrap();
        assert_eq!(cfg.keybind_vk, 0x77);
        assert_eq!(cfg.interval_ms, DEFAULT_INTERVAL_MS);
    }

    #[test]
    fn interval_validation() {
        assert!(is_valid_interval(1));
        assert!(is_valid_interval(100));
        assert!(is_valid_interval(60_000));
        assert!(!is_valid_interval(0));
        assert!(!is_valid_interval(60_001));
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"interval_ms\": 0}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"interval_ms\": 60001}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"interval_ms\": 1}")
                .unwrap()
                .interval_ms,
            1
        );
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
    fn default_interval_is_100ms() {
        assert_eq!(DEFAULT_INTERVAL_MS, 100);
        assert_eq!(Config::default().interval_ms, 100);
    }

    #[test]
    fn detector_fires_once_per_press() {
        let mut d = KeyPressDetector::new();
        assert!(!d.sample(false)); // baseline
        assert!(d.sample(true)); // press
        assert!(!d.sample(true)); // held — no refire
        assert!(!d.sample(false)); // release
        assert!(d.sample(true)); // second press
    }

    #[test]
    fn detector_ignores_key_held_at_startup() {
        let mut d = KeyPressDetector::new();
        assert!(!d.sample(true)); // already held: baseline, not a press
        assert!(!d.sample(true));
        assert!(!d.sample(false));
        assert!(d.sample(true));
    }

    #[test]
    fn detector_capture_keypress_does_not_toggle() {
        // The v0.1.0 bug: the key held to SET the keybind counted as a
        // press of it the moment the capture window closed.
        let mut d = KeyPressDetector::new();
        assert!(!d.sample(false)); // baseline, idle
                                   // User picks the new key in the capture window; on close the
                                   // app resyncs while the key is still physically down.
        d.resync();
        assert!(!d.sample(true)); // still held from capture — no toggle
        assert!(!d.sample(true));
        assert!(!d.sample(false)); // user lets go
        assert!(d.sample(true)); // a fresh press toggles, as expected
    }

    #[test]
    fn schedule_advances_one_interval() {
        let t0 = Instant::now();
        let iv = Duration::from_millis(100);
        // Half an interval after the previous click: next is t0 + 100 ms.
        let next = next_click_deadline(t0, iv, t0 + Duration::from_millis(50));
        assert_eq!(next, t0 + iv);
    }

    #[test]
    fn schedule_does_not_play_catch_up() {
        let t0 = Instant::now();
        let iv = Duration::from_millis(100);
        // The process was stalled for 3.5 intervals (e.g. the PC slept):
        // the deadline rebases to now + interval — no burst of clicks.
        let now = t0 + Duration::from_millis(350);
        assert_eq!(next_click_deadline(t0, iv, now), now + iv);
        // Landing exactly on a deadline also steps to the next one.
        let exact = t0 + iv;
        assert_eq!(next_click_deadline(t0, iv, exact), exact + iv);
    }

    #[test]
    fn schedule_chains_without_drift() {
        let t0 = Instant::now();
        let iv = Duration::from_millis(100);
        // Each click happens 3 ms "late" (its own cost); the deadlines
        // must stay on the 100 ms grid regardless.
        let mut prev = t0;
        for k in 1..=10u32 {
            let now = prev + Duration::from_millis(3);
            prev = next_click_deadline(prev, iv, now);
            assert_eq!(prev, t0 + iv * k);
        }
    }

    #[test]
    fn detector_resync_while_key_up_is_harmless() {
        let mut d = KeyPressDetector::new();
        assert!(!d.sample(false));
        d.resync();
        assert!(!d.sample(false));
        assert!(d.sample(true));
    }
}
