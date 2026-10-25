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

/// Which mouse button the clicker sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClickButton {
    #[default]
    Left,
    Right,
    Middle,
}

impl ClickButton {
    /// The config-file spelling of this button.
    pub fn as_str(&self) -> &'static str {
        match self {
            ClickButton::Left => "left",
            ClickButton::Right => "right",
            ClickButton::Middle => "middle",
        }
    }

    /// Parse a config-file spelling; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "left" => Some(ClickButton::Left),
            "right" => Some(ClickButton::Right),
            "middle" => Some(ClickButton::Middle),
            _ => None,
        }
    }

    /// Human-readable name for menus and dialogs.
    pub fn label(&self) -> &'static str {
        match self {
            ClickButton::Left => "Left",
            ClickButton::Right => "Right",
            ClickButton::Middle => "Middle",
        }
    }
}

/// What the keybind does while active: repeated clicks, or holding
/// the button down until the keybind is pressed again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClickMode {
    #[default]
    Click,
    Hold,
}

impl ClickMode {
    /// The config-file spelling of this mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            ClickMode::Click => "click",
            ClickMode::Hold => "hold",
        }
    }

    /// Parse a config-file spelling; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "click" => Some(ClickMode::Click),
            "hold" => Some(ClickMode::Hold),
            _ => None,
        }
    }

    /// Human-readable name for menus and dialogs.
    pub fn label(&self) -> &'static str {
        match self {
            ClickMode::Click => "Click",
            ClickMode::Hold => "Hold",
        }
    }
}

/// What one activation cycle sends (in Click mode): a single click, a
/// double-click, a burst of several clicks spaced by a gap, mouse-wheel
/// scroll ticks, a repeated keystroke, or a drag between two points.
/// The first three are the *click actions* — the only ones Hold mode
/// and button alternation apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClickAction {
    #[default]
    Single,
    Double,
    Burst,
    Scroll,
    Key,
    Drag,
}

impl ClickAction {
    /// The config-file spelling of this action.
    pub fn as_str(&self) -> &'static str {
        match self {
            ClickAction::Single => "single",
            ClickAction::Double => "double",
            ClickAction::Burst => "burst",
            ClickAction::Scroll => "scroll",
            ClickAction::Key => "key",
            ClickAction::Drag => "drag",
        }
    }

    /// Parse a config-file spelling; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "single" => Some(ClickAction::Single),
            "double" => Some(ClickAction::Double),
            "burst" => Some(ClickAction::Burst),
            "scroll" => Some(ClickAction::Scroll),
            "key" => Some(ClickAction::Key),
            "drag" => Some(ClickAction::Drag),
            _ => None,
        }
    }

    /// Human-readable name for menus and dialogs.
    pub fn label(&self) -> &'static str {
        match self {
            ClickAction::Single => "Single",
            ClickAction::Double => "Double",
            ClickAction::Burst => "Burst",
            ClickAction::Scroll => "Scroll",
            ClickAction::Key => "Key",
            ClickAction::Drag => "Drag",
        }
    }

    /// True for the mouse-click actions (Single/Double/Burst) — the
    /// ones Hold mode, alternation, and the button choice apply to.
    pub fn is_click_action(&self) -> bool {
        matches!(self, Self::Single | Self::Double | Self::Burst)
    }
}

/// The default keystroke repeated by the Key action: Space.
pub const DEFAULT_REPEAT_VK: u32 = 0x20;

/// Wheel notches per Scroll cycle must be within ±10 and non-zero
/// (positive scrolls up, negative down).
pub fn is_valid_scroll_amount(n: i32) -> bool {
    n != 0 && (-10..=10).contains(&n)
}

/// Whether `vk` is an extended key (navigation cluster, arrows, …)
/// that needs `KEYEVENTF_EXTENDEDKEY` when synthesized.
pub fn is_extended_key(vk: u32) -> bool {
    matches!(
        vk,
        0x21..=0x2E // PgUp..Delete (incl. arrows, Home, End, Insert)
            | 0x5B..=0x5D // Win keys, Apps
            | 0x6F // Numpad divide
            | 0xA3..=0xA5 // Right Ctrl/Alt
            | 0x90 // Num Lock
    )
}

/// The interval partway through a speed ramp: linear from `from_ms`
/// to `to_ms` over `total_s` seconds, clamped at both ends. A total
/// of 0 means the ramp is instant (always `to_ms`).
pub fn ramped_interval_ms(from_ms: u32, to_ms: u32, elapsed_s: u64, total_s: u64) -> u32 {
    if total_s == 0 || elapsed_s >= total_s {
        return to_ms;
    }
    let from = from_ms as i64;
    let to = to_ms as i64;
    let v = from + (to - from) * (elapsed_s as i64) / (total_s as i64);
    v.clamp(MIN_INTERVAL_MS as i64, MAX_INTERVAL_MS as i64) as u32
}

/// The longest ramp duration the config accepts (24 h).
pub const MAX_RAMP_SECONDS: u32 = 86_400;

pub fn is_valid_ramp_seconds(n: u32) -> bool {
    (1..=MAX_RAMP_SECONDS).contains(&n)
}

/// Clicks per burst when the config does not say (and the burst default).
pub const DEFAULT_BURST_CLICKS: u32 = 3;
/// The most clicks one burst may contain.
pub const MAX_BURST_CLICKS: u32 = 100;

pub fn is_valid_burst_clicks(n: u32) -> bool {
    (1..=MAX_BURST_CLICKS).contains(&n)
}

/// Gap between the clicks of a burst (and of a double-click), default.
pub const DEFAULT_BURST_GAP_MS: u32 = 50;
/// The largest burst gap the config accepts.
pub const MAX_BURST_GAP_MS: u32 = 1_000;

pub fn is_valid_burst_gap(ms: u32) -> bool {
    (1..=MAX_BURST_GAP_MS).contains(&ms)
}

/// The longest a single click may hold its button down (0 = instant).
pub const MAX_PRESS_MS: u32 = 1_000;

pub fn is_valid_press_ms(ms: u32) -> bool {
    ms <= MAX_PRESS_MS
}

/// The button a click in a run uses. With alternation on, odd-numbered
/// clicks in the run use the base button's partner: Left ↔ Right, and
/// Middle pairs with Left.
pub fn effective_button(base: ClickButton, alternate: bool, run_clicks: u64) -> ClickButton {
    if !alternate || run_clicks.is_multiple_of(2) {
        return base;
    }
    match base {
        ClickButton::Left => ClickButton::Right,
        ClickButton::Right => ClickButton::Left,
        ClickButton::Middle => ClickButton::Left,
    }
}

/// The largest auto-stop limit (in clicks) the config accepts.
pub const MAX_LIMIT_CLICKS: u32 = 100_000_000;

/// Whether an auto-stop limit is acceptable: 0 means "no limit".
pub fn is_valid_limit(n: u32) -> bool {
    n <= MAX_LIMIT_CLICKS
}

/// Where cycles land: follow the cursor, one locked point, or a
/// rotating sequence of points. (Slice 3 of the 50-enhancement
/// program; replaces the pre-0.8 `lock_position` boolean, which the
/// config parser still honours for older files.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetMode {
    #[default]
    Follow,
    Locked,
    Sequence,
}

impl TargetMode {
    /// The config-file spelling of this target mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetMode::Follow => "follow",
            TargetMode::Locked => "locked",
            TargetMode::Sequence => "sequence",
        }
    }

    /// Parse a config-file spelling; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "follow" => Some(TargetMode::Follow),
            "locked" => Some(TargetMode::Locked),
            "sequence" => Some(TargetMode::Sequence),
            _ => None,
        }
    }

    /// Human-readable name for menus and dialogs.
    pub fn label(&self) -> &'static str {
        match self {
            TargetMode::Follow => "Follow",
            TargetMode::Locked => "Locked",
            TargetMode::Sequence => "Sequence",
        }
    }
}

/// The most points a multi-point sequence may hold.
pub const MAX_SEQ_POINTS: u32 = 4;

/// The number of named target slots.
pub const MAX_SLOTS: usize = 3;

/// Click offsets (px from the cursor/lock point) must stay within
/// ±200 on each axis.
pub const MAX_OFFSET_PX: i32 = 200;

pub fn is_valid_offset(v: i32) -> bool {
    (-MAX_OFFSET_PX..=MAX_OFFSET_PX).contains(&v)
}

/// The pixel-check tolerance: the largest per-channel difference
/// (0–255 scale) that still counts as "the same colour".
pub const MAX_PIXEL_TOL: u32 = 64;
pub const DEFAULT_PIXEL_TOL: u32 = 16;

pub fn is_valid_pixel_tol(n: u32) -> bool {
    n <= MAX_PIXEL_TOL
}

/// True when two packed `0xRRGGBB` colours are within `tol` on every
/// channel — the pixel check's "same colour" test.
pub fn colour_matches(sample: u32, target: u32, tol: u32) -> bool {
    let chan = |c: u32, shift: u32| ((c >> shift) & 0xFF) as i32;
    (chan(sample, 16) - chan(target, 16)).abs() <= tol as i32
        && (chan(sample, 8) - chan(target, 8)).abs() <= tol as i32
        && (chan(sample, 0) - chan(target, 0)).abs() <= tol as i32
}

/// How many samples of a region signature differ between two reads,
/// comparing packed colours with a small fixed tolerance (lighting
/// noise must not count as change). Slices of different lengths
/// compare over their common prefix.
pub fn region_diff_count(baseline: &[u32], current: &[u32]) -> usize {
    baseline
        .iter()
        .zip(current.iter())
        .filter(|(b, c)| !colour_matches(**b, **c, 8))
        .count()
}

/// The region-change stop: the region counts as changed when more
/// than a quarter of its samples differ from the run-start read.
pub fn region_changed(baseline: &[u32], current: &[u32]) -> bool {
    if baseline.is_empty() || baseline.len() != current.len() {
        return false;
    }
    region_diff_count(baseline, current) * 4 > baseline.len()
}

/// The centre of the first monitor that does not contain `current`
/// — the "other monitor" quick-lock preset. `monitors` are
/// `(left, top, right, bottom)` rects in virtual-screen coordinates.
/// `None` when there is no other monitor to jump to.
pub fn other_monitor_centre(
    monitors: &[(i32, i32, i32, i32)],
    current: (i32, i32),
) -> Option<(i32, i32)> {
    if monitors.len() < 2 {
        return None;
    }
    let contains = |m: &(i32, i32, i32, i32)| {
        current.0 >= m.0 && current.0 < m.2 && current.1 >= m.1 && current.1 < m.3
    };
    let home = monitors.iter().position(contains).unwrap_or(0);
    let other = monitors[(home + 1) % monitors.len()];
    Some(((other.0 + other.2) / 2, (other.1 + other.3) / 2))
}

/// The longest slot name the config accepts.
pub const MAX_SLOT_NAME: usize = 20;

/// Reduce arbitrary text to a slot name: letters, digits, spaces and
/// `-_'.,` survive, everything else is dropped, and the result is
/// truncated to [`MAX_SLOT_NAME`] chars and trimmed. The restricted
/// charset keeps names safe for the hand-rolled JSON writer (no
/// quotes or backslashes can occur).
pub fn sanitize_slot_name(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " -_'.,".contains(*c))
        .take(MAX_SLOT_NAME)
        .collect();
    kept.trim().to_string()
}

/// True when `name` is a slot name the config accepts as-is.
pub fn is_valid_slot_name(name: &str) -> bool {
    name.chars().count() <= MAX_SLOT_NAME && sanitize_slot_name(name) == name
}

/// Map a point on an axis to the 0..=65535 absolute coordinate that
/// `SendInput` expects for that axis of the virtual screen, given the
/// axis `origin` (which may be negative) and `extent` in pixels.
/// Out-of-range points clamp to the nearest edge.
pub fn absolute_coord(pos: i32, origin: i32, extent: i32) -> u16 {
    if extent <= 1 {
        return 0;
    }
    let scaled = (pos as i64 - origin as i64) * 65535 / (extent as i64 - 1);
    scaled.clamp(0, 65535) as u16
}

/// Apply up to ±10% jitter to an interval, using `rand` as the random
/// source (any u32; only its value mod 201 is used, mapping to a
/// per-mille factor of 900..=1100). A 1 ms interval is returned
/// unchanged — there is no room below it.
pub fn jittered_interval_ms(base_ms: u32, rand: u32) -> u32 {
    if base_ms <= MIN_INTERVAL_MS {
        return base_ms;
    }
    let factor = 900 + (rand % 201) as u64; // per-mille: 0.900 ..= 1.100
    let scaled = (base_ms as u64 * factor) / 1000;
    (scaled as u32).max(MIN_INTERVAL_MS)
}

/// Format a count with thousands separators, e.g. `1,234,567`.
pub fn format_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The corner failsafe: true when the cursor is in the top-left corner
/// zone of the virtual screen (within `CORNER_PX` of the corner whose
/// coordinates are `vx`, `vy`). Slamming the mouse to the top-left is
/// the classic panic gesture; while clicking, it stops the clicker.
pub const CORNER_PX: i32 = 5;

pub fn in_failsafe_corner(x: i32, y: i32, vx: i32, vy: i32) -> bool {
    x <= vx + CORNER_PX && y <= vy + CORNER_PX
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

/// Pick the image bytes of the best-matching entry in an `.ico` file.
///
/// `desired` is the wanted icon width in pixels (e.g. the system's
/// small-icon size). The chosen entry is the smallest one at least
/// `desired` wide, or the largest available when all are smaller — so
/// the shell never has to upscale a tiny image or shrink the 256 px
/// one into a tray slot. The returned slice is the entry's image data
/// (a PNG for our generated icons), in the resource format
/// `CreateIconFromResourceEx` expects. Returns `None` for malformed
/// data: wrong header, truncated directory, or an image range outside
/// the file.
pub fn ico_image_for_size(data: &[u8], desired: u32) -> Option<&[u8]> {
    if data.len() < 6 {
        return None;
    }
    let reserved = u16::from_le_bytes([data[0], data[1]]);
    let kind = u16::from_le_bytes([data[2], data[3]]);
    let count = u16::from_le_bytes([data[4], data[5]]) as usize;
    if reserved != 0 || kind != 1 || count == 0 {
        return None;
    }
    let mut best_fit: Option<(u32, &[u8])> = None; // smallest width >= desired
    let mut largest: Option<(u32, &[u8])> = None; // largest width overall
    for i in 0..count {
        let base = 6 + 16 * i;
        let entry = data.get(base..base + 16)?;
        // A width byte of 0 means 256.
        let width = match entry[0] {
            0 => 256,
            w => w as u32,
        };
        let size = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        let image = data.get(offset..offset.checked_add(size)?)?;
        if largest.is_none_or(|(w, _)| width > w) {
            largest = Some((width, image));
        }
        if width >= desired && best_fit.is_none_or(|(w, _)| width < w) {
            best_fit = Some((width, image));
        }
    }
    best_fit.or(largest).map(|(_, image)| image)
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
        // OEM punctuation keys (US layout): VK_OEM_1/2/3/4/5/6/7,
        // VK_OEM_PLUS, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD.
        0xBA => ";".to_string(),
        0xBB => "=".to_string(),
        0xBC => ",".to_string(),
        0xBD => "-".to_string(),
        0xBE => ".".to_string(),
        0xBF => "/".to_string(),
        0xC0 => "`".to_string(),
        0xDB => "[".to_string(),
        0xDC => "\\".to_string(),
        0xDD => "]".to_string(),
        0xDE => "'".to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x30..=0x39 => ((vk as u8) as char).to_string(),
        0x41..=0x5A => ((vk as u8) as char).to_string(),
        _ => format!("Key 0x{vk:02X}"),
    }
}

/// Persisted settings: the keybind, the click interval, which mouse
/// button is clicked, whether interval jitter is on, the mode (click
/// or hold), whether clicks lock to the position where clicking
/// started, the auto-stop limit in clicks (0 = no limit), and whether
/// clicking starts when TrayClick launches (click mode only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub keybind_vk: u32,
    pub interval_ms: u32,
    pub button: ClickButton,
    pub jitter: bool,
    pub mode: ClickMode,
    pub lock_position: bool,
    pub limit_clicks: u32,
    pub start_on_launch: bool,
    pub action: ClickAction,
    pub burst_clicks: u32,
    pub burst_gap_ms: u32,
    pub press_ms: u32,
    pub alternate: bool,
    pub scroll_amount: i32,
    pub repeat_vk: u32,
    pub drag_ax: i32,
    pub drag_ay: i32,
    pub drag_bx: i32,
    pub drag_by: i32,
    pub drag_set: bool,
    pub random_interval: bool,
    pub interval_max_ms: u32,
    pub ramp: bool,
    pub ramp_to_ms: u32,
    pub ramp_seconds: u32,
    // Fields added in 0.8.0 (50-enhancement program, slice 3 —
    // targeting). `lock_position` above is kept in sync with `target`
    // (true unless Follow) so pre-0.8 builds can still read the file.
    pub target: TargetMode,
    /// The persistent lock point, picked with the crosshair overlay
    /// (slice-2 runs captured one per activation; that fallback stays
    /// for when no persistent point is set).
    pub lock_x: i32,
    pub lock_y: i32,
    pub lock_set: bool,
    /// Window-relative lock: the point follows the window it was
    /// picked on, at this offset from the window's top-left corner.
    /// (The window itself is matched live, never persisted.)
    pub lock_rel: bool,
    pub lock_rx: i32,
    pub lock_ry: i32,
    /// The multi-point sequence (first `seq_count` points are used).
    pub seq_count: u32,
    pub seq_x: [i32; 4],
    pub seq_y: [i32; 4],
    /// Three named target slots.
    pub slot_x: [i32; 3],
    pub slot_y: [i32; 3],
    pub slot_set: [bool; 3],
    pub slot_names: [String; 3],
    /// Click offset from the cursor/lock point, in pixels.
    pub offset_x: i32,
    pub offset_y: i32,
    /// The pixel check: only click while the pixel at the click
    /// point matches `pixel_rgb` (packed 0xRRGGBB) within tolerance.
    pub pixel_check: bool,
    pub pixel_set: bool,
    pub pixel_rgb: u32,
    pub pixel_tol: u32,
    /// Stop the run when the region around the click point changes.
    pub region_guard: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keybind_vk: DEFAULT_KEYBIND_VK,
            interval_ms: DEFAULT_INTERVAL_MS,
            button: ClickButton::Left,
            jitter: false,
            mode: ClickMode::Click,
            lock_position: false,
            limit_clicks: 0,
            start_on_launch: false,
            action: ClickAction::Single,
            burst_clicks: DEFAULT_BURST_CLICKS,
            burst_gap_ms: DEFAULT_BURST_GAP_MS,
            press_ms: 0,
            alternate: false,
            scroll_amount: 1,
            repeat_vk: DEFAULT_REPEAT_VK,
            drag_ax: 0,
            drag_ay: 0,
            drag_bx: 0,
            drag_by: 0,
            drag_set: false,
            random_interval: false,
            interval_max_ms: 0,
            ramp: false,
            ramp_to_ms: 0,
            ramp_seconds: 60,
            target: TargetMode::Follow,
            lock_x: 0,
            lock_y: 0,
            lock_set: false,
            lock_rel: false,
            lock_rx: 0,
            lock_ry: 0,
            seq_count: 0,
            seq_x: [0; 4],
            seq_y: [0; 4],
            slot_x: [0; 3],
            slot_y: [0; 3],
            slot_set: [false; 3],
            slot_names: [String::new(), String::new(), String::new()],
            offset_x: 0,
            offset_y: 0,
            pixel_check: false,
            pixel_set: false,
            pixel_rgb: 0,
            pixel_tol: DEFAULT_PIXEL_TOL,
            region_guard: false,
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

/// Read one signed-integer field out of the hand-rolled JSON object.
fn json_i32(text: &str, key: &str) -> Option<i32> {
    let start = text.find(key)? + key.len();
    let rest = text[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let (neg, rest) = match rest.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let mag: i32 = digits.parse().ok()?;
    Some(if neg { -mag } else { mag })
}

/// Read one string field out of the hand-rolled JSON object.
fn json_str(text: &str, key: &str) -> Option<String> {
    let start = text.find(key)? + key.len();
    let rest = text[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Read one boolean field out of the hand-rolled JSON object.
fn json_bool(text: &str, key: &str) -> Option<bool> {
    let start = text.find(key)? + key.len();
    let rest = text[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

impl Config {
    /// Serialize as a small hand-rolled JSON object (no dependencies).
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"keybind_vk\": {},\n  \"interval_ms\": {},\n  \"button\": \"{}\",\n  \"jitter\": {},\n  \"mode\": \"{}\",\n  \"lock_position\": {},\n  \"limit_clicks\": {},\n  \"start_on_launch\": {},\n  \"action\": \"{}\",\n  \"burst_clicks\": {},\n  \"burst_gap_ms\": {},\n  \"press_ms\": {},\n  \"alternate\": {},\n  \"scroll_amount\": {},\n  \"repeat_vk\": {},\n  \"drag_ax\": {},\n  \"drag_ay\": {},\n  \"drag_bx\": {},\n  \"drag_by\": {},\n  \"drag_set\": {},\n  \"random_interval\": {},\n  \"interval_max_ms\": {},\n  \"ramp\": {},\n  \"ramp_to_ms\": {},\n  \"ramp_seconds\": {},\n  \"target\": \"{}\",\n  \"lock_x\": {},\n  \"lock_y\": {},\n  \"lock_set\": {},\n  \"lock_rel\": {},\n  \"lock_rx\": {},\n  \"lock_ry\": {},\n  \"seq_count\": {},\n  \"seq_x1\": {},\n  \"seq_y1\": {},\n  \"seq_x2\": {},\n  \"seq_y2\": {},\n  \"seq_x3\": {},\n  \"seq_y3\": {},\n  \"seq_x4\": {},\n  \"seq_y4\": {},\n  \"slot1_x\": {},\n  \"slot1_y\": {},\n  \"slot1_set\": {},\n  \"slot1_name\": \"{}\",\n  \"slot2_x\": {},\n  \"slot2_y\": {},\n  \"slot2_set\": {},\n  \"slot2_name\": \"{}\",\n  \"slot3_x\": {},\n  \"slot3_y\": {},\n  \"slot3_set\": {},\n  \"slot3_name\": \"{}\",\n  \"offset_x\": {},\n  \"offset_y\": {},\n  \"pixel_check\": {},\n  \"pixel_set\": {},\n  \"pixel_rgb\": {},\n  \"pixel_tol\": {},\n  \"region_guard\": {}\n}}\n",
            self.keybind_vk,
            self.interval_ms,
            self.button.as_str(),
            self.jitter,
            self.mode.as_str(),
            self.lock_position,
            self.limit_clicks,
            self.start_on_launch,
            self.action.as_str(),
            self.burst_clicks,
            self.burst_gap_ms,
            self.press_ms,
            self.alternate,
            self.scroll_amount,
            self.repeat_vk,
            self.drag_ax,
            self.drag_ay,
            self.drag_bx,
            self.drag_by,
            self.drag_set,
            self.random_interval,
            self.interval_max_ms,
            self.ramp,
            self.ramp_to_ms,
            self.ramp_seconds,
            self.target.as_str(),
            self.lock_x,
            self.lock_y,
            self.lock_set,
            self.lock_rel,
            self.lock_rx,
            self.lock_ry,
            self.seq_count,
            self.seq_x[0],
            self.seq_y[0],
            self.seq_x[1],
            self.seq_y[1],
            self.seq_x[2],
            self.seq_y[2],
            self.seq_x[3],
            self.seq_y[3],
            self.slot_x[0],
            self.slot_y[0],
            self.slot_set[0],
            self.slot_names[0],
            self.slot_x[1],
            self.slot_y[1],
            self.slot_set[1],
            self.slot_names[1],
            self.slot_x[2],
            self.slot_y[2],
            self.slot_set[2],
            self.slot_names[2],
            self.offset_x,
            self.offset_y,
            self.pixel_check,
            self.pixel_set,
            self.pixel_rgb,
            self.pixel_tol,
            self.region_guard
        )
    }

    /// Parse a config written by [`Config::to_json`]. Returns `None` for
    /// anything malformed or with invalid values, so the caller can fall
    /// back to defaults without touching the user's file. Missing fields
    /// from older config versions default: `interval_ms` (pre-0.2) to
    /// [`DEFAULT_INTERVAL_MS`], `button` (pre-0.4) to Left, `jitter`
    /// (pre-0.4) to off.
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
        let button = match json_str(text, "\"button\"") {
            Some(name) => ClickButton::from_name(&name)?,
            None => ClickButton::Left,
        };
        let jitter = opt_bool(text, "\"jitter\"")?;
        // Fields added in 0.5.0; absent in older files, defaults apply.
        let mode = match json_str(text, "\"mode\"") {
            Some(name) => ClickMode::from_name(&name)?,
            None => ClickMode::Click,
        };
        let lock_position = opt_bool(text, "\"lock_position\"")?;
        let limit_clicks = match json_u32(text, "\"limit_clicks\"") {
            Some(n) => {
                if !is_valid_limit(n) {
                    return None;
                }
                n
            }
            None => 0,
        };
        let start_on_launch = opt_bool(text, "\"start_on_launch\"")?;
        // Fields added in 0.6.0 (50-enhancement program, slice 1);
        // absent in older files, defaults apply.
        let action = match json_str(text, "\"action\"") {
            Some(name) => ClickAction::from_name(&name)?,
            None => ClickAction::Single,
        };
        let burst_clicks = match json_u32(text, "\"burst_clicks\"") {
            Some(n) => {
                if !is_valid_burst_clicks(n) {
                    return None;
                }
                n
            }
            None => DEFAULT_BURST_CLICKS,
        };
        let burst_gap_ms = match json_u32(text, "\"burst_gap_ms\"") {
            Some(n) => {
                if !is_valid_burst_gap(n) {
                    return None;
                }
                n
            }
            None => DEFAULT_BURST_GAP_MS,
        };
        let press_ms = match json_u32(text, "\"press_ms\"") {
            Some(n) => {
                if !is_valid_press_ms(n) {
                    return None;
                }
                n
            }
            None => 0,
        };
        let alternate = opt_bool(text, "\"alternate\"")?;
        // Fields added in 0.7.0 (50-enhancement program, slice 2);
        // absent in older files, defaults apply.
        let scroll_amount = match json_i32(text, "\"scroll_amount\"") {
            Some(n) => {
                if !is_valid_scroll_amount(n) {
                    return None;
                }
                n
            }
            None => 1,
        };
        let repeat_vk = match json_u32(text, "\"repeat_vk\"") {
            Some(v) => {
                if !is_valid_keybind(v) {
                    return None;
                }
                v
            }
            None => DEFAULT_REPEAT_VK,
        };
        let drag_ax = opt_i32(text, "\"drag_ax\"")?;
        let drag_ay = opt_i32(text, "\"drag_ay\"")?;
        let drag_bx = opt_i32(text, "\"drag_bx\"")?;
        let drag_by = opt_i32(text, "\"drag_by\"")?;
        let drag_set = opt_bool(text, "\"drag_set\"")?;
        let random_interval = opt_bool(text, "\"random_interval\"")?;
        let interval_max_ms = match json_u32(text, "\"interval_max_ms\"") {
            Some(n) => {
                if n != 0 && !is_valid_interval(n) {
                    return None;
                }
                n
            }
            None => 0,
        };
        let ramp = opt_bool(text, "\"ramp\"")?;
        let ramp_to_ms = match json_u32(text, "\"ramp_to_ms\"") {
            Some(n) => {
                if n != 0 && !is_valid_interval(n) {
                    return None;
                }
                n
            }
            None => 0,
        };
        let ramp_seconds = match json_u32(text, "\"ramp_seconds\"") {
            Some(n) => {
                if !is_valid_ramp_seconds(n) {
                    return None;
                }
                n
            }
            None => 60,
        };
        // Fields added in 0.8.0 (50-enhancement program, slice 3 —
        // targeting); absent in older files, defaults apply. The
        // `target` string wins; older files carry only the
        // `lock_position` boolean (true → Locked, false → Follow).
        let target = match json_str(text, "\"target\"") {
            Some(name) => TargetMode::from_name(&name)?,
            None => {
                if lock_position {
                    TargetMode::Locked
                } else {
                    TargetMode::Follow
                }
            }
        };
        let lock_x = opt_i32(text, "\"lock_x\"")?;
        let lock_y = opt_i32(text, "\"lock_y\"")?;
        let lock_set = opt_bool(text, "\"lock_set\"")?;
        let lock_rel = opt_bool(text, "\"lock_rel\"")?;
        let lock_rx = opt_i32(text, "\"lock_rx\"")?;
        let lock_ry = opt_i32(text, "\"lock_ry\"")?;
        let seq_count = match json_u32(text, "\"seq_count\"") {
            Some(n) => {
                if n > MAX_SEQ_POINTS {
                    return None;
                }
                n
            }
            None => 0,
        };
        let mut seq_x = [0; 4];
        let mut seq_y = [0; 4];
        for i in 0..4 {
            seq_x[i] = opt_i32(text, &format!("\"seq_x{}\"", i + 1))?;
            seq_y[i] = opt_i32(text, &format!("\"seq_y{}\"", i + 1))?;
        }
        let mut slot_x = [0; 3];
        let mut slot_y = [0; 3];
        let mut slot_set = [false; 3];
        let mut slot_names: [String; 3] = [String::new(), String::new(), String::new()];
        for i in 0..3 {
            slot_x[i] = opt_i32(text, &format!("\"slot{}_x\"", i + 1))?;
            slot_y[i] = opt_i32(text, &format!("\"slot{}_y\"", i + 1))?;
            slot_set[i] = opt_bool(text, &format!("\"slot{}_set\"", i + 1))?;
            slot_names[i] = match json_str(text, &format!("\"slot{}_name\"", i + 1)) {
                Some(name) => {
                    if !is_valid_slot_name(&name) {
                        return None;
                    }
                    name
                }
                None => String::new(),
            };
        }
        let offset_x = opt_i32(text, "\"offset_x\"")?;
        let offset_y = opt_i32(text, "\"offset_y\"")?;
        if !is_valid_offset(offset_x) || !is_valid_offset(offset_y) {
            return None;
        }
        let pixel_check = opt_bool(text, "\"pixel_check\"")?;
        let pixel_set = opt_bool(text, "\"pixel_set\"")?;
        let pixel_rgb = match json_u32(text, "\"pixel_rgb\"") {
            Some(n) => {
                if n > 0x00FF_FFFF {
                    return None;
                }
                n
            }
            None => 0,
        };
        let pixel_tol = match json_u32(text, "\"pixel_tol\"") {
            Some(n) => {
                if !is_valid_pixel_tol(n) {
                    return None;
                }
                n
            }
            None => DEFAULT_PIXEL_TOL,
        };
        let region_guard = opt_bool(text, "\"region_guard\"")?;
        Some(Self {
            keybind_vk: vk,
            interval_ms,
            button,
            jitter,
            mode,
            limit_clicks,
            start_on_launch,
            action,
            burst_clicks,
            burst_gap_ms,
            press_ms,
            alternate,
            scroll_amount,
            repeat_vk,
            drag_ax,
            drag_ay,
            drag_bx,
            drag_by,
            drag_set,
            random_interval,
            interval_max_ms,
            ramp,
            ramp_to_ms,
            ramp_seconds,
            lock_position: target != TargetMode::Follow,
            target,
            lock_x,
            lock_y,
            lock_set,
            lock_rel,
            lock_rx,
            lock_ry,
            seq_count,
            seq_x,
            seq_y,
            slot_x,
            slot_y,
            slot_set,
            slot_names,
            offset_x,
            offset_y,
            pixel_check,
            pixel_set,
            pixel_rgb,
            pixel_tol,
            region_guard,
        })
    }
}

/// Read an optional signed-integer field: absent → `Some(0)`, present
/// and valid → its value, present but garbled → `None`.
fn opt_i32(text: &str, key: &str) -> Option<i32> {
    match json_i32(text, key) {
        Some(v) => Some(v),
        None => {
            if text.contains(key) {
                None
            } else {
                Some(0)
            }
        }
    }
}

/// Read an optional boolean field: absent → `Some(false)`, present
/// and valid → its value, present but garbled → `None`.
fn opt_bool(text: &str, key: &str) -> Option<bool> {
    match json_bool(text, key) {
        Some(b) => Some(b),
        None => {
            if text.contains(key) {
                None
            } else {
                Some(false)
            }
        }
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
    fn key_names_oem_punctuation() {
        assert_eq!(key_name(0xBA), ";");
        assert_eq!(key_name(0xBB), "=");
        assert_eq!(key_name(0xBC), ",");
        assert_eq!(key_name(0xBD), "-");
        assert_eq!(key_name(0xBE), ".");
        assert_eq!(key_name(0xBF), "/");
        assert_eq!(key_name(0xC0), "`");
        assert_eq!(key_name(0xDB), "[");
        assert_eq!(key_name(0xDC), "\\");
        assert_eq!(key_name(0xDD), "]");
        assert_eq!(key_name(0xDE), "'");
    }

    #[test]
    fn config_round_trip() {
        let cfg = Config {
            keybind_vk: 0x77, // F8
            interval_ms: 250,
            button: ClickButton::Right,
            jitter: true,
            mode: ClickMode::Hold,
            lock_position: true,
            limit_clicks: 5_000,
            start_on_launch: true,
            action: ClickAction::Burst,
            burst_clicks: 5,
            burst_gap_ms: 80,
            press_ms: 25,
            alternate: true,
            scroll_amount: -3,
            repeat_vk: 0x52, // R
            drag_ax: -100,
            drag_ay: 200,
            drag_bx: 300,
            drag_by: -400,
            drag_set: true,
            random_interval: true,
            interval_max_ms: 500,
            ramp: true,
            ramp_to_ms: 20,
            ramp_seconds: 120,
            target: TargetMode::Sequence,
            lock_x: -500,
            lock_y: 700,
            lock_set: true,
            lock_rel: true,
            lock_rx: 40,
            lock_ry: -15,
            seq_count: 3,
            seq_x: [10, -20, 30, 0],
            seq_y: [100, 200, -300, 0],
            slot_x: [1, 2, 3],
            slot_y: [4, 5, 6],
            slot_set: [true, false, true],
            slot_names: [
                "Boss button".to_string(),
                String::new(),
                "Craft, mk.2".to_string(),
            ],
            offset_x: -25,
            offset_y: 40,
            pixel_check: true,
            pixel_set: true,
            pixel_rgb: 0x00FF_8040,
            pixel_tol: 24,
            region_guard: true,
        };
        let json = cfg.to_json();
        assert_eq!(Config::from_json(&json), Some(cfg));
    }

    #[test]
    fn config_from_070_gets_slice3_defaults() {
        // A v0.7.0 config has lock_position but no target or slice-3
        // fields: the boolean maps (true → Locked), all else defaults.
        let cfg = Config::from_json("{\"keybind_vk\": 117, \"lock_position\": true}").unwrap();
        assert_eq!(cfg.target, TargetMode::Locked);
        assert!(cfg.lock_position);
        assert!(!cfg.lock_set);
        assert_eq!(cfg.seq_count, 0);
        assert_eq!(cfg.offset_x, 0);
        assert!(!cfg.pixel_check);
        assert_eq!(cfg.pixel_tol, DEFAULT_PIXEL_TOL);
        assert!(!cfg.region_guard);
        let cfg = Config::from_json("{\"keybind_vk\": 117}").unwrap();
        assert_eq!(cfg.target, TargetMode::Follow);
        assert!(!cfg.lock_position);
    }

    #[test]
    fn config_rejects_bad_slice3_fields() {
        for bad in [
            "{\"keybind_vk\": 117, \"target\": \"orbit\"}",
            "{\"keybind_vk\": 117, \"seq_count\": 5}",
            "{\"keybind_vk\": 117, \"offset_x\": 201}",
            "{\"keybind_vk\": 117, \"offset_y\": -201}",
            "{\"keybind_vk\": 117, \"pixel_rgb\": 16777216}",
            "{\"keybind_vk\": 117, \"pixel_tol\": 65}",
            "{\"keybind_vk\": 117, \"slot1_name\": \"has \\\"quote\\\"\"}",
            "{\"keybind_vk\": 117, \"slot2_name\": \"this name is far too long\"}",
            "{\"keybind_vk\": 117, \"seq_x2\": \"left\"}",
        ] {
            assert_eq!(Config::from_json(bad), None, "{bad}");
        }
        // Boundary values are legitimate.
        assert!(Config::from_json(
            "{\"keybind_vk\": 117, \"seq_count\": 4, \"offset_x\": -200, \"pixel_tol\": 64}"
        )
        .is_some());
    }

    #[test]
    fn target_mode_names() {
        for t in [TargetMode::Follow, TargetMode::Locked, TargetMode::Sequence] {
            assert_eq!(TargetMode::from_name(t.as_str()), Some(t));
        }
        assert_eq!(TargetMode::from_name("lock"), None);
    }

    #[test]
    fn colour_matching() {
        assert!(colour_matches(0xFF8040, 0xFF8040, 0));
        assert!(colour_matches(0xFF8040, 0xFE7F41, 1));
        assert!(!colour_matches(0xFF8040, 0xFD8040, 1)); // red off by 2
        assert!(colour_matches(0x000000, 0x404040, 64));
        assert!(!colour_matches(0x000000, 0x410000, 64)); // one channel over
    }

    #[test]
    fn region_change_detection() {
        let base = [0x101010; 8];
        assert!(!region_changed(&base, &base));
        // One of eight samples fully different: 1*4 <= 8, not a change.
        let mut slight = base;
        slight[3] = 0xFFFFFF;
        assert!(!region_changed(&base, &slight));
        // Three of eight: 3*4 > 8, a change.
        let mut moved = base;
        moved[0] = 0xFFFFFF;
        moved[1] = 0xFFFFFF;
        moved[2] = 0xFFFFFF;
        assert!(region_changed(&base, &moved));
        // Lighting noise within the fixed tolerance is not a change.
        let noisy = [0x141414; 8];
        assert!(!region_changed(&base, &noisy));
        assert_eq!(region_diff_count(&base, &moved), 3);
    }

    #[test]
    fn other_monitor_centre_picks_the_other_one() {
        let dual = [(0, 0, 1920, 1080), (1920, 0, 3840, 1080)];
        assert_eq!(other_monitor_centre(&dual, (100, 100)), Some((2880, 540)));
        assert_eq!(other_monitor_centre(&dual, (2000, 100)), Some((960, 540)));
        // One monitor: nowhere to jump.
        assert_eq!(other_monitor_centre(&dual[..1], (100, 100)), None);
        // Point outside every monitor: the first counts as home.
        assert_eq!(other_monitor_centre(&dual, (9999, 9999)), Some((2880, 540)));
        // A monitor left of the primary (negative coordinates).
        let neg = [(-1920, 0, 0, 1080), (0, 0, 1920, 1080)];
        assert_eq!(other_monitor_centre(&neg, (100, 100)), Some((-960, 540)));
    }

    #[test]
    fn slot_name_rules() {
        assert_eq!(sanitize_slot_name("Boss button!"), "Boss button");
        assert_eq!(sanitize_slot_name("  Craft, mk.2  "), "Craft, mk.2");
        assert_eq!(sanitize_slot_name("a\"b\\c"), "abc");
        assert!(is_valid_slot_name(""));
        assert!(is_valid_slot_name("Boss button"));
        assert!(!is_valid_slot_name("Boss button!"));
        assert!(!is_valid_slot_name("trailing "));
        assert!(!is_valid_slot_name("this name is far too long"));
    }

    #[test]
    fn config_from_060_gets_slice2_defaults() {
        // A v0.6.0 config file has none of the slice-2 fields.
        let cfg =
            Config::from_json("{\"keybind_vk\": 117, \"action\": \"burst\", \"burst_clicks\": 4}")
                .unwrap();
        assert_eq!(cfg.scroll_amount, 1);
        assert_eq!(cfg.repeat_vk, DEFAULT_REPEAT_VK);
        assert!(!cfg.drag_set);
        assert!(!cfg.random_interval);
        assert_eq!(cfg.interval_max_ms, 0);
        assert!(!cfg.ramp);
        assert_eq!(cfg.ramp_to_ms, 0);
        assert_eq!(cfg.ramp_seconds, 60);
    }

    #[test]
    fn config_rejects_bad_slice2_fields() {
        for bad in [
            "{\"keybind_vk\": 117, \"scroll_amount\": 0}",
            "{\"keybind_vk\": 117, \"scroll_amount\": 11}",
            "{\"keybind_vk\": 117, \"scroll_amount\": -11}",
            "{\"keybind_vk\": 117, \"repeat_vk\": 27}", // Esc
            "{\"keybind_vk\": 117, \"repeat_vk\": 1}",  // mouse
            "{\"keybind_vk\": 117, \"interval_max_ms\": 60001}",
            "{\"keybind_vk\": 117, \"ramp_to_ms\": 60001}",
            "{\"keybind_vk\": 117, \"ramp_seconds\": 0}",
            "{\"keybind_vk\": 117, \"drag_ax\": \"left\"}",
        ] {
            assert_eq!(Config::from_json(bad), None, "{bad}");
        }
        // Negative coordinates and scroll are legitimate values.
        assert!(Config::from_json(
            "{\"keybind_vk\": 117, \"drag_ax\": -1920, \"scroll_amount\": -10}"
        )
        .is_some());
    }

    #[test]
    fn action_names_include_slice2() {
        for a in [ClickAction::Scroll, ClickAction::Key, ClickAction::Drag] {
            assert_eq!(ClickAction::from_name(a.as_str()), Some(a));
            assert!(!a.is_click_action());
        }
        for a in [ClickAction::Single, ClickAction::Double, ClickAction::Burst] {
            assert!(a.is_click_action());
        }
    }

    #[test]
    fn ramp_interpolates_and_clamps() {
        assert_eq!(ramped_interval_ms(100, 10, 0, 60), 100);
        assert_eq!(ramped_interval_ms(100, 10, 30, 60), 55);
        assert_eq!(ramped_interval_ms(100, 10, 60, 60), 10);
        assert_eq!(ramped_interval_ms(100, 10, 600, 60), 10);
        assert_eq!(ramped_interval_ms(100, 10, 0, 0), 10);
        assert_eq!(ramped_interval_ms(10, 100, 30, 60), 55);
        assert!(is_valid_ramp_seconds(1) && is_valid_ramp_seconds(MAX_RAMP_SECONDS));
        assert!(!is_valid_ramp_seconds(0));
    }

    #[test]
    fn extended_key_set() {
        assert!(is_extended_key(0x25)); // Left arrow
        assert!(is_extended_key(0x2E)); // Delete
        assert!(is_extended_key(0x24)); // Home
        assert!(is_extended_key(0x5B)); // Left Win
        assert!(!is_extended_key(0x41)); // A
        assert!(!is_extended_key(0x20)); // Space
        assert!(!is_extended_key(0x70)); // F1
    }

    #[test]
    fn config_from_051_gets_action_defaults() {
        // A v0.5.1 config file has none of the slice-1 fields.
        let cfg = Config::from_json(
            "{\"keybind_vk\": 117, \"interval_ms\": 50, \"mode\": \"hold\", \"limit_clicks\": 10}",
        )
        .unwrap();
        assert_eq!(cfg.action, ClickAction::Single);
        assert_eq!(cfg.burst_clicks, DEFAULT_BURST_CLICKS);
        assert_eq!(cfg.burst_gap_ms, DEFAULT_BURST_GAP_MS);
        assert_eq!(cfg.press_ms, 0);
        assert!(!cfg.alternate);
    }

    #[test]
    fn config_rejects_bad_action_fields() {
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"action\": \"triple\"}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"burst_clicks\": 0}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"burst_clicks\": 101}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"burst_gap_ms\": 0}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"press_ms\": 1001}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"alternate\": \"yes\"}"),
            None
        );
        assert!(is_valid_burst_clicks(1) && is_valid_burst_clicks(MAX_BURST_CLICKS));
        assert!(is_valid_press_ms(0) && is_valid_press_ms(MAX_PRESS_MS));
    }

    #[test]
    fn action_names_round_trip() {
        for a in [ClickAction::Single, ClickAction::Double, ClickAction::Burst] {
            assert_eq!(ClickAction::from_name(a.as_str()), Some(a));
        }
        assert_eq!(ClickAction::from_name("Single"), None);
        assert_eq!(ClickAction::default(), ClickAction::Single);
    }

    #[test]
    fn alternation_flips_partner_on_odd_clicks() {
        assert_eq!(
            effective_button(ClickButton::Left, false, 1),
            ClickButton::Left
        );
        assert_eq!(
            effective_button(ClickButton::Left, true, 0),
            ClickButton::Left
        );
        assert_eq!(
            effective_button(ClickButton::Left, true, 1),
            ClickButton::Right
        );
        assert_eq!(
            effective_button(ClickButton::Right, true, 3),
            ClickButton::Left
        );
        assert_eq!(
            effective_button(ClickButton::Middle, true, 1),
            ClickButton::Left
        );
        assert_eq!(
            effective_button(ClickButton::Middle, true, 2),
            ClickButton::Middle
        );
    }

    #[test]
    fn config_from_040_gets_mode_defaults() {
        // A v0.4.x config file has none of the 0.5.0 fields.
        let cfg = Config::from_json(
            "{\"keybind_vk\": 117, \"interval_ms\": 50, \"button\": \"middle\", \"jitter\": true}",
        )
        .unwrap();
        assert_eq!(cfg.mode, ClickMode::Click);
        assert!(!cfg.lock_position);
        assert_eq!(cfg.limit_clicks, 0);
        assert!(!cfg.start_on_launch);
    }

    #[test]
    fn config_rejects_bad_mode_and_limit() {
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"mode\": \"smash\"}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"limit_clicks\": 100000001}"),
            None
        );
        assert!(is_valid_limit(0));
        assert!(is_valid_limit(MAX_LIMIT_CLICKS));
        assert!(!is_valid_limit(MAX_LIMIT_CLICKS + 1));
    }

    #[test]
    fn config_rejects_garbled_optional_bools() {
        // Present-but-garbled optional booleans reject the whole file
        // (the caller then falls back to defaults); absent ones default.
        for field in ["jitter", "lock_position", "start_on_launch"] {
            let garbled = format!("{{\"keybind_vk\": 117, \"{field}\": \"yes\"}}");
            assert_eq!(Config::from_json(&garbled), None, "{field}");
            let absent = Config::from_json("{\"keybind_vk\": 117}").unwrap();
            assert!(!absent.jitter && !absent.lock_position && !absent.start_on_launch);
        }
    }

    #[test]
    fn mode_names_round_trip() {
        for m in [ClickMode::Click, ClickMode::Hold] {
            assert_eq!(ClickMode::from_name(m.as_str()), Some(m));
        }
        assert_eq!(ClickMode::from_name("Click"), None);
        assert_eq!(ClickMode::default(), ClickMode::Click);
    }

    #[test]
    fn absolute_coord_maps_virtual_screen() {
        // Single 1920-wide screen starting at 0.
        assert_eq!(absolute_coord(0, 0, 1920), 0);
        assert_eq!(absolute_coord(1919, 0, 1920), 65535);
        let mid = absolute_coord(960, 0, 1920);
        assert!((32700..=32900).contains(&mid), "mid was {mid}");
        // A second monitor to the left: origin -1920, extent 3840.
        assert_eq!(absolute_coord(-1920, -1920, 3840), 0);
        assert_eq!(absolute_coord(1919, -1920, 3840), 65535);
        // Out-of-range clamps instead of wrapping.
        assert_eq!(absolute_coord(99999, 0, 1920), 65535);
        assert_eq!(absolute_coord(-99999, 0, 1920), 0);
        // Degenerate extents are safe.
        assert_eq!(absolute_coord(5, 0, 1), 0);
        assert_eq!(absolute_coord(5, 0, 0), 0);
    }

    #[test]
    fn config_from_030_gets_button_and_jitter_defaults() {
        // A v0.3.x config file has no button/jitter fields.
        let cfg = Config::from_json("{\"keybind_vk\": 117, \"interval_ms\": 50}").unwrap();
        assert_eq!(cfg.button, ClickButton::Left);
        assert!(!cfg.jitter);
    }

    #[test]
    fn config_rejects_unknown_button() {
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"button\": \"pinky\"}"),
            None
        );
        assert_eq!(
            Config::from_json("{\"keybind_vk\": 117, \"jitter\": \"yes\"}"),
            None
        );
    }

    #[test]
    fn button_names_round_trip() {
        for b in [ClickButton::Left, ClickButton::Right, ClickButton::Middle] {
            assert_eq!(ClickButton::from_name(b.as_str()), Some(b));
        }
        assert_eq!(ClickButton::from_name("LEFT"), None);
        assert_eq!(ClickButton::default(), ClickButton::Left);
    }

    #[test]
    fn jitter_stays_within_ten_percent() {
        for rand in [0u32, 1, 100, 200, 201, 999, u32::MAX] {
            let j = jittered_interval_ms(100, rand);
            assert!((90..=110).contains(&j), "rand {rand} gave {j}");
            let big = jittered_interval_ms(60_000, rand);
            assert!((54_000..=66_000).contains(&big), "rand {rand} gave {big}");
        }
        // The floor does not move: 1 ms stays 1 ms for every rand.
        for rand in [0u32, 100, u32::MAX] {
            assert_eq!(jittered_interval_ms(1, rand), 1);
        }
        // Both extremes are reachable.
        assert_eq!(jittered_interval_ms(100, 0), 90);
        assert_eq!(jittered_interval_ms(100, 200), 110);
    }

    #[test]
    fn count_formatting() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(42), "42");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1,000");
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(format_count(u64::MAX), "18,446,744,073,709,551,615");
    }

    #[test]
    fn failsafe_corner_is_top_left_only() {
        assert!(in_failsafe_corner(0, 0, 0, 0));
        assert!(in_failsafe_corner(CORNER_PX, CORNER_PX, 0, 0));
        assert!(!in_failsafe_corner(CORNER_PX + 1, 0, 0, 0));
        assert!(!in_failsafe_corner(0, CORNER_PX + 1, 0, 0));
        // Other corners do not trigger it.
        assert!(!in_failsafe_corner(1920, 1080, 0, 0));
        // A virtual screen starting at (0,0) with a monitor to the left
        // has a negative origin; the zone follows the origin.
        assert!(in_failsafe_corner(-1920, 0, -1920, 0));
        assert!(!in_failsafe_corner(0, 0, -1920, 0));
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

    /// Build a synthetic multi-entry .ico for the picker tests. Each
    /// entry is (width byte, image bytes); images are appended in order.
    fn fake_ico(entries: &[(u8, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * entries.len();
        let mut blobs = Vec::new();
        for (w, img) in entries {
            let mut e = [0u8; 16];
            e[0] = *w;
            e[1] = *w;
            e[8..12].copy_from_slice(&(img.len() as u32).to_le_bytes());
            e[12..16].copy_from_slice(&(offset as u32).to_le_bytes());
            offset += img.len();
            out.extend_from_slice(&e);
            blobs.extend_from_slice(img);
        }
        out.extend_from_slice(&blobs);
        out
    }

    #[test]
    fn ico_picker_chooses_smallest_adequate_entry() {
        let data = fake_ico(&[(16, b"aa"), (32, b"bbb"), (48, b"cccc"), (0, b"ddddd")]);
        assert_eq!(ico_image_for_size(&data, 16), Some(&b"aa"[..]));
        assert_eq!(ico_image_for_size(&data, 20), Some(&b"bbb"[..]));
        assert_eq!(ico_image_for_size(&data, 48), Some(&b"cccc"[..]));
        // The width byte 0 entry is 256 px wide.
        assert_eq!(ico_image_for_size(&data, 100), Some(&b"ddddd"[..]));
        // Nothing is big enough: fall back to the largest entry.
        assert_eq!(ico_image_for_size(&data, 512), Some(&b"ddddd"[..]));
    }

    #[test]
    fn ico_picker_rejects_malformed_data() {
        assert_eq!(ico_image_for_size(b"", 16), None);
        assert_eq!(ico_image_for_size(b"\x00\x00\x02\x00\x01\x00", 16), None); // not type 1
        let truncated = fake_ico(&[(16, b"aa")]);
        assert_eq!(ico_image_for_size(&truncated[..10], 16), None); // short directory
        let mut bad_offset = fake_ico(&[(16, b"aa")]);
        bad_offset[6 + 12] = 0xFF; // image offset far past the end
        bad_offset[6 + 13] = 0xFF;
        assert_eq!(ico_image_for_size(&bad_offset, 16), None);
    }

    #[test]
    fn shipped_icons_parse_and_have_png_entries() {
        for bytes in [
            include_bytes!("../assets/icon.ico").as_slice(),
            include_bytes!("../assets/icon_active.ico").as_slice(),
        ] {
            let img = ico_image_for_size(bytes, 16).expect("icon entry");
            assert!(img.starts_with(b"\x89PNG"));
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
