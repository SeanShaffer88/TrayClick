#![cfg_attr(windows, windows_subsystem = "windows")]

//! TrayClick — a Windows tray auto clicker.
//!
//! Lives entirely in the notification area. Right-clicking the
//! tray icon opens a menu with exactly six items; left-clicking it
//! opens the settings window (the Set Interval window below):
//!
//! - **Pause / Resume** — pause stops clicking and ignores the keybind
//!   until resumed.
//! - **Set Keybind** — shows the current keybind; opens a small window
//!   that captures the next key press as the keybind. The press
//!   that assigns the key never starts clicking; clicking starts on the
//!   next fresh press.
//! - **Set Interval** — shows the current interval; opens a small window
//!   with 1 ms and 100 ms presets plus a custom value in milliseconds,
//!   the mouse-button choice (left/right/middle), and the jitter toggle.
//! - **Start with Windows** — checkable; toggles the per-user Run key.
//! - **Credits** — about/credits dialog.
//! - **Exit** — removes the tray icon and quits.
//!
//! Safety: while clicking, slamming the mouse into the top-left corner
//! of the screen stops clicking immediately (the corner failsafe).
//!
//! The Set Interval window also carries the 0.5.0 settings: the mode
//! (Click, or Hold — the keybind holds the button down until pressed
//! again; every stop path releases it), the target (follow the cursor,
//! or lock to the position where clicking started — each click moves
//! to the locked point, clicks, and puts the cursor back), an
//! auto-stop limit in clicks per run (0 = none), and start-on-launch
//! (click mode only). The 0.6.0 slice (50-enhancement program) added
//! the click action — single, double, or a burst of clicks — plus a
//! per-click button-down duration and alternating buttons, and made
//! the process per-monitor DPI aware so locked clicks land exactly
//! on scaled displays. The 0.7.0 slice added the Scroll, Key, and
//! Drag actions (wheel ticks, a repeated keystroke, a two-point drag
//! captured on a full-screen overlay), a random interval range, and
//! a per-run speed ramp. Hold mode applies to the mouse-click
//! actions only; Scroll/Key/Drag repeat on the interval either way.
//!
//! The 0.8.0 slice is all about targeting. The target is now three-way
//! (follow the cursor, one locked point, or a rotating sequence of up
//! to four points); the lock point is persistent and can be picked
//! with a crosshair overlay, saved into three named slots, jumped to
//! the other monitor's centre, and followed window-relatively (it
//! tracks the window it was picked on). Clicks can be offset by a few
//! pixels, gated by a pixel check (click only while a watched pixel
//! keeps its colour), and guarded by a region check (the run stops
//! when the patch of screen around the point changes). The Set
//! Interval window is paged (Clicking / Action / Targeting tabs) and
//! remembers its page.
//!
//! The 0.9.0 slice is triggers and automation. The mode gains
//! Momentary (click only while the keybind is held); the mouse side
//! buttons X1/X2 can be the keybind or the new stop-only key; an
//! optional RegisterHotKey mode replaces polling where it can
//! (keyboard bindings only, never for momentary); a delayed start
//! begins a run in N minutes; a run gate pauses cycles while the
//! chosen foreground app is not in front, or while a fullscreen
//! window is; and a second launch can forward --start, --stop,
//! --toggle, or --profile <name> to the running instance instead of
//! starting a copy.
//!
//! The 0.10.0 work is conformance, not features: settings moved
//! from `%APPDATA%` to a `data` folder next to the executable (a
//! config from an older version is copied over automatically on
//! first run), the build links the C runtime statically, releases
//! add an MSI installer next to the Inno one, and the CLI gains
//! local maintenance commands — `--help`, `--version`, `--clean`,
//! and `--uninstall` — that run before any window exists.
//!
//! The 0.11.0 work adds a second auto-stop limit — elapsed seconds
//! per run — beside the click limit (whichever is reached first
//! stops the run, in Hold mode too), rearranges the settings
//! window into a charcoal window with a fixed top control bar
//! (keybind, interval, Start/Stop), grouped pages whose buttons
//! sit in a footer row, and makes a left-click on the tray icon
//! open the settings window (right-click keeps the menu).
//!
//! Pressing the keybind (default **F6**) toggles auto left-clicking at the
//! cursor position at the configured interval (default 100 ms),
//! indefinitely, until the keybind is pressed again. While clicking is
//! on, the tray icon blinks (alternating with an inverted red icon every
//! 500 ms) and its tooltip shows the running click count (or, in hold
//! mode, which button is held), so the state is visible at a glance.
//!
//! All state-machine logic lives in the platform-independent library
//! (`lib.rs`) and is unit-tested there; this file is the Win32 shell.

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("TrayClick is a Windows program.");
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::{Duration, Instant};

    use trayclick::{
        absolute_coord, auto_stop_reached, choose_config_file, colour_matches, effective_button,
        format_count, ico_image_for_size, in_failsafe_corner, is_extended_key, is_newer_version,
        is_valid_burst_clicks, is_valid_burst_gap, is_valid_delay_minutes, is_valid_interval,
        is_valid_keybind, is_valid_limit, is_valid_limit_seconds, is_valid_offset,
        is_valid_pixel_tol, is_valid_press_ms, is_valid_ramp_seconds, is_valid_repeat_key,
        is_valid_scroll_amount, jittered_interval_ms, key_name, next_click_deadline,
        other_monitor_centre, parse_cli_full, parse_latest_release, perform_press,
        ramped_interval_ms, rect_is_fullscreen, region_changed, sanitize_slot_name,
        wait_for_action, ActionClock, ActionInput, ActionRun, CliCommand, CliOptions, ClickAction,
        ClickButton, ClickMode, Config, ConfigChoice, Engine, InputOwner, KeyPressDetector,
        KeyReleaseDetector, PressBatch, TargetMode, DEFAULT_PIXEL_TOL, LATEST_RELEASE_URL,
        MAX_INTERVAL_MS, MIN_INTERVAL_MS, USAGE, VK_ESCAPE_CODE,
    };

    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, COLORREF, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HWND, LPARAM,
        LRESULT, POINT, RECT, WPARAM,
    };
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
    use windows::Win32::Graphics::Gdi::{
        CreateSolidBrush, DrawTextW, EnumDisplayMonitors, FillRect, FrameRect, GetDC,
        GetMonitorInfoW, GetPixel, InvalidateRect, MonitorFromWindow, ReleaseDC, SetBkColor,
        SetBkMode, SetTextColor, DT_CENTER, DT_SINGLELINE, DT_VCENTER, HBRUSH, HDC, HMONITOR,
        MONITORINFO, MONITOR_DEFAULTTONEAREST, TRANSPARENT,
    };
    use windows::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT,
        WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE,
    };
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, REG_VALUE_TYPE,
    };
    use windows::Win32::System::Threading::{
        CreateMutexW, GetCurrentThread, OpenProcess, QueryFullProcessImageNameW, SetThreadPriority,
        PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, THREAD_PRIORITY_BELOW_NORMAL,
    };
    use windows::Win32::UI::Controls::DRAWITEMSTRUCT;
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, RegisterHotKey, SendInput, SetFocus, UnregisterHotKey, INPUT, INPUT_0,
        INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, MOD_NOREPEAT, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
        MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
    };
    use windows::Win32::UI::Shell::{
        ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
        NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    /// Tray icon callback message.
    const WM_TRAYICON: u32 = WM_USER + 1;
    /// Posted by the keybind polling thread on a keybind press edge.
    const WM_APP_TOGGLE: u32 = WM_APP + 1;
    /// Posted by the click thread when the corner failsafe trips, so the
    /// UI thread can settle the tray indicator.
    const WM_APP_FAILSAFE: u32 = WM_APP + 2;
    /// Posted by the click thread when the auto-stop limit is reached.
    const WM_APP_LIMIT: u32 = WM_APP + 3;
    /// Posted by the click thread when the region guard stops a run.
    const WM_APP_REGION: u32 = WM_APP + 4;
    /// Posted when the stop-only key is pressed (poll thread or the
    /// CLI forwarder): stop clicking, whatever started it.
    const WM_APP_STOP: u32 = WM_APP + 5;
    /// Posted by a second launch carrying a CLI command; `wparam` is
    /// 1 = start, 2 = stop, 3 = toggle.
    const WM_APP_COMMAND: u32 = WM_APP + 6;
    /// Posted when the run gate (foreground-app rule / fullscreen
    /// pause) flips, so the UI thread can settle the held button.
    const WM_APP_GATE: u32 = WM_APP + 7;
    /// Posted after a capture assigns a key, to (re)register hotkeys.
    const WM_APP_REHOTKEY: u32 = WM_APP + 8;
    /// Posted by the launch-time update check when a newer release
    /// exists; the tag and release URL wait in `update_offer`.
    const WM_APP_UPDATE: u32 = WM_APP + 9;
    /// The delayed-start countdown timer on the main window.
    const ID_DELAY_TIMER: usize = 2;
    /// RegisterHotKey ids (their own numbering space).
    const ID_HOTKEY_KEYBIND: i32 = 1;
    const ID_HOTKEY_STOP: i32 = 2;

    const TRAY_ID: u32 = 1;

    /// Timer that blinks the tray icon while clicking is on.
    const ID_BLINK_TIMER: usize = 1;
    const BLINK_MS: u32 = 500;

    const ID_PAUSE_RESUME: usize = 1001;
    const ID_SET_KEYBIND: usize = 1002;
    const ID_SET_INTERVAL: usize = 1003;
    const ID_START_WITH_WINDOWS: usize = 1006;
    const ID_CREDITS: usize = 1004;
    const ID_EXIT: usize = 1005;

    // Control IDs inside the Set Interval window.
    const ID_IV_1MS: usize = 2001;
    const ID_IV_100MS: usize = 2002;
    const ID_IV_APPLY: usize = 2003;
    const ID_IV_EDIT: usize = 2004;
    const ID_IV_BTN_LEFT: usize = 2005;
    const ID_IV_BTN_RIGHT: usize = 2006;
    const ID_IV_BTN_MIDDLE: usize = 2007;
    const ID_IV_JITTER: usize = 2008;
    const ID_IV_MODE: usize = 2009;
    const ID_IV_POS: usize = 2010;
    const ID_IV_LIMIT_EDIT: usize = 2011;
    const ID_IV_LIMIT_SET: usize = 2012;
    const ID_IV_LAUNCH: usize = 2013;
    const ID_IV_ACTION: usize = 2014;
    const ID_IV_ALT: usize = 2015;
    const ID_IV_BURST_EDIT: usize = 2016;
    const ID_IV_GAP_EDIT: usize = 2017;
    const ID_IV_PRESS_EDIT: usize = 2018;
    const ID_IV_NUM_SET: usize = 2019;
    const ID_IV_REPEAT: usize = 2020;
    const ID_IV_DRAG: usize = 2021;
    const ID_IV_SCROLL_EDIT: usize = 2022;
    const ID_IV_RANDOM: usize = 2023;
    const ID_IV_MAX_EDIT: usize = 2024;
    const ID_IV_RAMP: usize = 2025;
    const ID_IV_RAMP_TO_EDIT: usize = 2026;
    const ID_IV_RAMP_SECS_EDIT: usize = 2027;
    const ID_IV_TIMING_SET: usize = 2028;
    // Settings-window page buttons (0.8.0 — the window is paged).
    const ID_IV_PAGE_CLICK: usize = 2101;
    const ID_IV_PAGE_ACTION: usize = 2102;
    const ID_IV_PAGE_TARGET: usize = 2103;
    // Targeting page controls (0.8.0).
    const ID_IV_PICK_LOCK: usize = 2104;
    const ID_IV_OTHER_MON: usize = 2105;
    const ID_IV_WINREL: usize = 2106;
    const ID_IV_REGION: usize = 2107;
    const ID_IV_SEQ_SET: usize = 2108;
    const ID_IV_SEQ_CLEAR: usize = 2109;
    const ID_IV_SLOT1_NAME: usize = 2110;
    const ID_IV_SLOT1_SAVE: usize = 2111;
    const ID_IV_SLOT1_USE: usize = 2112;
    const ID_IV_SLOT1_COORDS: usize = 2113;
    const ID_IV_SLOT2_NAME: usize = 2114;
    const ID_IV_SLOT2_SAVE: usize = 2115;
    const ID_IV_SLOT2_USE: usize = 2116;
    const ID_IV_SLOT2_COORDS: usize = 2117;
    const ID_IV_SLOT3_NAME: usize = 2118;
    const ID_IV_SLOT3_SAVE: usize = 2119;
    const ID_IV_SLOT3_USE: usize = 2120;
    const ID_IV_SLOT3_COORDS: usize = 2121;
    const ID_IV_OFF_X_EDIT: usize = 2122;
    const ID_IV_OFF_Y_EDIT: usize = 2123;
    const ID_IV_OFF_SET: usize = 2124;
    const ID_IV_PIXEL: usize = 2125;
    const ID_IV_PIXEL_SAMPLE: usize = 2126;
    const ID_IV_TOL_EDIT: usize = 2127;
    const ID_IV_TOL_SET: usize = 2128;
    const ID_IV_FOOTER: usize = 2129;
    // Clicking-page trigger controls (0.9.0).
    const ID_IV_STOPKEY: usize = 2130;
    const ID_IV_HOTKEY: usize = 2131;
    const ID_IV_DELAY_EDIT: usize = 2132;
    const ID_IV_DELAY: usize = 2133;
    const ID_IV_FS: usize = 2134;
    const ID_IV_FG: usize = 2135;
    const ID_IV_FG_LABEL: usize = 2136;
    const ID_IV_FG_PICK: usize = 2137;
    // Auto-stop seconds row and the top control bar (0.11.0).
    const ID_IV_LIMIT_SEC_EDIT: usize = 2138;
    const ID_IV_LIMIT_SEC_SET: usize = 2139;
    const ID_IV_TOP_KEYNAME: usize = 2140;
    const ID_IV_TOP_KEYBIND: usize = 2141;
    const ID_IV_TOP_START: usize = 2142;
    const ID_IV_TOP_STOP: usize = 2143;
    // App-picker window controls (0.9.0).
    const ID_AP_LIST: usize = 3001;
    const ID_AP_SELECT: usize = 3002;
    // Update prompt window controls (1.0.0).
    const ID_UP_DOWNLOAD: usize = 3101;
    const ID_UP_LATER: usize = 3102;

    static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
    static KEYBIND_VK: AtomicU32 = AtomicU32::new(trayclick::DEFAULT_KEYBIND_VK);
    static INTERVAL_MS: AtomicU32 = AtomicU32::new(trayclick::DEFAULT_INTERVAL_MS);
    /// Which mouse button is clicked: 0 = left, 1 = right, 2 = middle.
    static BUTTON: AtomicU32 = AtomicU32::new(0);
    /// Whether interval jitter (±10%) is on.
    static JITTER_ON: AtomicBool = AtomicBool::new(false);
    /// Clicks sent since the process started (shown in the tooltip and
    /// in Credits).
    static CLICK_COUNT: AtomicU64 = AtomicU64::new(0);
    /// The mode: 0 = Click, 1 = Hold.
    static MODE: AtomicU32 = AtomicU32::new(0);
    /// The target mode: 0 = Follow cursor, 1 = Locked point,
    /// 2 = Sequence of points (0.8.0; replaces the pre-0.8 lock flag).
    static TARGET_MODE: AtomicU32 = AtomicU32::new(0);
    /// The persistent lock point (crosshair-picked, a slot, or the
    /// other-monitor preset) and whether one is set. Persisted.
    static LOCK_X: AtomicI32 = AtomicI32::new(0);
    static LOCK_Y: AtomicI32 = AtomicI32::new(0);
    static LOCK_SET: AtomicBool = AtomicBool::new(false);
    /// Window-relative lock: while on and the picked window is alive,
    /// the lock point follows it at this offset from its top-left.
    /// The window handle itself is matched live, never persisted.
    static LOCK_REL: AtomicBool = AtomicBool::new(false);
    static LOCK_RX: AtomicI32 = AtomicI32::new(0);
    static LOCK_RY: AtomicI32 = AtomicI32::new(0);
    static LOCK_HWND: AtomicI32 = AtomicI32::new(0);
    /// The fallback lock point captured when a run starts with no
    /// persistent point set (the pre-0.8 behaviour), per run only.
    static RUN_X: AtomicI32 = AtomicI32::new(0);
    static RUN_Y: AtomicI32 = AtomicI32::new(0);
    static RUN_SET: AtomicBool = AtomicBool::new(false);
    /// The multi-point sequence: points and how many are in use,
    /// plus the rotation index (reset when a run starts).
    static SEQ_COUNT: AtomicU32 = AtomicU32::new(0);
    static SEQ_X: [AtomicI32; 4] = [
        AtomicI32::new(0),
        AtomicI32::new(0),
        AtomicI32::new(0),
        AtomicI32::new(0),
    ];
    static SEQ_Y: [AtomicI32; 4] = [
        AtomicI32::new(0),
        AtomicI32::new(0),
        AtomicI32::new(0),
        AtomicI32::new(0),
    ];
    static SEQ_IDX: AtomicU32 = AtomicU32::new(0);
    /// Three named target slots (points + names; names behind a lock
    /// because they are Strings).
    static SLOT_X: [AtomicI32; 3] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
    static SLOT_Y: [AtomicI32; 3] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
    static SLOT_SET: [AtomicBool; 3] = [
        AtomicBool::new(false),
        AtomicBool::new(false),
        AtomicBool::new(false),
    ];
    static SLOT_NAMES: OnceLock<Mutex<[String; 3]>> = OnceLock::new();
    /// Click offset from the cursor/lock point, in pixels.
    static OFFSET_X: AtomicI32 = AtomicI32::new(0);
    static OFFSET_Y: AtomicI32 = AtomicI32::new(0);
    /// The pixel check: click only while the pixel at the click
    /// point matches the sampled colour (packed 0xRRGGBB).
    static PIXEL_CHECK: AtomicBool = AtomicBool::new(false);
    static PIXEL_SET: AtomicBool = AtomicBool::new(false);
    static PIXEL_RGB: AtomicU32 = AtomicU32::new(0);
    static PIXEL_TOL: AtomicU32 = AtomicU32::new(DEFAULT_PIXEL_TOL);
    /// The region guard: stop the run when the patch of screen
    /// around the click point changes. The baseline signature is
    /// sampled at the run's first cycle.
    static REGION_GUARD: AtomicBool = AtomicBool::new(false);
    static REGION_BASE: OnceLock<Mutex<Option<[u32; 64]>>> = OnceLock::new();
    /// Where the region baseline was sampled in Follow mode (the
    /// region is a fixed patch of screen, even when clicks follow
    /// the cursor).
    static REGION_AX: AtomicI32 = AtomicI32::new(0);
    static REGION_AY: AtomicI32 = AtomicI32::new(0);
    static REGION_ANCHOR_SET: AtomicBool = AtomicBool::new(false);
    /// The settings window's current page (0 = Clicking, 1 = Action,
    /// 2 = Targeting) and its paged controls' handles.
    static CURRENT_PAGE: AtomicU32 = AtomicU32::new(0);
    static PAGE_CONTROLS: OnceLock<Mutex<Vec<(i32, u32)>>> = OnceLock::new();
    /// The stop-only key's virtual-key code (0 = unset), and whether
    /// RegisterHotKey mode is on. The OK flags record which bindings
    /// are actually registered right now (registration can fail, and
    /// mouse buttons can't register at all — those keep polling).
    static STOP_VK: AtomicU32 = AtomicU32::new(0);
    static HOTKEY_MODE: AtomicBool = AtomicBool::new(false);
    static KB_HOTKEY_OK: AtomicBool = AtomicBool::new(false);
    static STOP_HOTKEY_OK: AtomicBool = AtomicBool::new(false);
    /// The run gate: cycles and the held button are live only while
    /// this is open. The click thread recomputes it from the
    /// foreground-app rule and the fullscreen pause.
    static GATE_OPEN: AtomicBool = AtomicBool::new(true);
    static FG_RULE: AtomicBool = AtomicBool::new(false);
    static FG_EXE: OnceLock<Mutex<String>> = OnceLock::new();
    static FS_PAUSE: AtomicBool = AtomicBool::new(false);
    /// When a delayed start fires (one-shot, never persisted).
    static DELAY_END: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    /// A profile name forwarded by a CLI launch. Profiles land in
    /// v1.0.0; until then the name is only remembered here.
    static PENDING_PROFILE: OnceLock<Mutex<String>> = OnceLock::new();
    /// The app picker's process names, parallel to its listbox rows.
    static APP_PICK_EXES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    /// The update on offer, when the launch-time check found one:
    /// (release tag, release page URL), written by the check thread
    /// before it posts `WM_APP_UPDATE`, read by the prompt window.
    static UPDATE_OFFER: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();
    /// Auto-stop limit in clicks per run; 0 = no limit.
    static LIMIT_CLICKS: AtomicU32 = AtomicU32::new(0);
    /// Auto-stop limit in seconds per run; 0 = no limit (0.11.0).
    static LIMIT_SECONDS: AtomicU32 = AtomicU32::new(0);
    /// Clicks sent in the current run (reset on every activation).
    static RUN_COUNT: AtomicU64 = AtomicU64::new(0);
    /// Whether clicking starts when TrayClick launches (click mode).
    static START_ON_LAUNCH: AtomicBool = AtomicBool::new(false);
    /// The click action: 0 = Single, 1 = Double, 2 = Burst.
    static ACTION: AtomicU32 = AtomicU32::new(0);
    /// Clicks per burst (burst action) and the gap between them.
    static BURST_CLICKS: AtomicU32 = AtomicU32::new(trayclick::DEFAULT_BURST_CLICKS);
    static BURST_GAP_MS: AtomicU32 = AtomicU32::new(trayclick::DEFAULT_BURST_GAP_MS);
    /// How long each click holds its button down (0 = instant).
    static PRESS_MS: AtomicU32 = AtomicU32::new(0);
    /// Whether consecutive clicks alternate between button partners.
    static ALTERNATE: AtomicBool = AtomicBool::new(false);
    /// Wheel notches per Scroll cycle (negative = down).
    static SCROLL_AMT: AtomicI32 = AtomicI32::new(1);
    /// The keystroke the Key action repeats.
    static REPEAT_VK: AtomicU32 = AtomicU32::new(trayclick::DEFAULT_REPEAT_VK);
    /// The Drag action's two points and whether they have been set.
    static DRAG_AX: AtomicI32 = AtomicI32::new(0);
    static DRAG_AY: AtomicI32 = AtomicI32::new(0);
    static DRAG_BX: AtomicI32 = AtomicI32::new(0);
    static DRAG_BY: AtomicI32 = AtomicI32::new(0);
    static DRAG_SET: AtomicBool = AtomicBool::new(false);
    /// Random-interval mode: each cycle picks within [interval, max].
    static RANDOM_ON: AtomicBool = AtomicBool::new(false);
    static INTERVAL_MAX_MS: AtomicU32 = AtomicU32::new(0);
    /// Speed ramp: the interval eases from the base to `ramp_to_ms`
    /// over `ramp_seconds` of each run.
    static RAMP_ON: AtomicBool = AtomicBool::new(false);
    static RAMP_TO_MS: AtomicU32 = AtomicU32::new(0);
    static RAMP_SECONDS: AtomicU32 = AtomicU32::new(60);
    /// Whether the mouse button is currently held down (hold mode).
    /// Every stop path funnels through `refresh_indicator`, which calls
    /// `sync_hold_state` — so every stop path releases the button.
    static HELD: AtomicU64 = AtomicU64::new(0);
    static RUN_ID: AtomicU64 = AtomicU64::new(0);
    static ACTION_RUN: Mutex<Option<ActionRun>> = Mutex::new(None);
    fn input_owner() -> &'static Mutex<InputOwner<INPUT>> {
        static OWNER: OnceLock<Mutex<InputOwner<INPUT>>> = OnceLock::new();
        OWNER.get_or_init(|| Mutex::new(InputOwner::default()))
    }

    fn action_run() -> Option<ActionRun> {
        *ACTION_RUN.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn same_run(run: ActionRun) -> bool {
        action_run().is_some_and(|current| current.id == run.id)
    }

    fn action_allowed_locked(
        run: ActionRun,
        engine: &mut Engine,
        gate: bool,
        failsafe: bool,
    ) -> bool {
        if !same_run(run) || !engine.is_clicking_now() {
            return false;
        }
        if run.expired(Instant::now()) || limit_reached(0) {
            engine.clicking = false;
            post_main(WM_APP_LIMIT);
            return false;
        }
        if failsafe {
            engine.clicking = false;
            post_main(WM_APP_FAILSAFE);
            return false;
        }
        gate
    }

    struct ActionWait;
    impl ActionClock for ActionWait {
        fn now(&self) -> Instant {
            Instant::now()
        }
        fn allowed(&mut self, run: ActionRun) -> bool {
            let gate = gate_should_be_open();
            let failsafe = cursor_in_failsafe();
            action_allowed_locked(run, &mut lock_engine(), gate, failsafe)
        }
        fn pause(&mut self, duration: Duration) {
            if duration > Duration::from_millis(2) {
                std::thread::sleep(duration - Duration::from_millis(2));
            } else {
                std::hint::spin_loop();
            }
        }
    }

    struct PressInput {
        vk: u32,
        down: INPUT,
        up: INPUT,
        drag: Option<((i32, i32), (i32, i32))>,
        target: Option<(i32, i32)>,
    }

    impl ActionClock for PressInput {
        fn now(&self) -> Instant {
            Instant::now()
        }
        fn allowed(&mut self, run: ActionRun) -> bool {
            ActionWait.allowed(run)
        }
        fn pause(&mut self, duration: Duration) {
            ActionWait.pause(duration);
        }
    }

    impl ActionInput for PressInput {
        fn press(&mut self, run: ActionRun) -> bool {
            press_targeted(
                run,
                self.vk,
                self.down,
                self.up,
                self.target,
                false,
                self.drag.is_none(),
            )
        }
        fn release(&mut self, run: ActionRun) {
            release_owned(run.id);
        }
        fn drag_step(&mut self, run: ActionRun, step: u32) {
            if let Some(((ax, ay), (bx, by))) = self.drag {
                let i = step as i32;
                let mut engine = lock_engine();
                let mut owner = input_owner().lock().unwrap_or_else(|p| p.into_inner());
                if action_allowed_locked(
                    run,
                    &mut engine,
                    gate_should_be_open(),
                    cursor_in_failsafe(),
                ) {
                    owner.move_sampled(
                        Some((run.id, self.vk)),
                        abs_move_input(ax + (bx - ax) * i / 8, ay + (by - ay) * i / 8),
                        button_held,
                        send_one,
                    );
                }
            }
        }
    }

    fn button_held(vk: u32) -> bool {
        unsafe { GetAsyncKeyState(vk as i32) < 0 }
    }

    fn send_one(input: INPUT) -> bool {
        unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) == 1 }
    }

    fn press_owned(run: ActionRun, vk: u32, down: INPUT, up: INPUT) -> bool {
        press_targeted(run, vk, down, up, None, false, false)
    }

    fn press_targeted(
        run: ActionRun,
        vk: u32,
        down: INPUT,
        up: INPUT,
        target: Option<(i32, i32)>,
        complete: bool,
        restore_after: bool,
    ) -> bool {
        let mut engine = lock_engine();
        let mut owner = input_owner().lock().unwrap_or_else(|p| p.into_inner());
        let gate = gate_should_be_open();
        let failsafe = cursor_in_failsafe();
        if !action_allowed_locked(run, &mut engine, gate, failsafe) {
            return false;
        }
        let back = target
            .and_then(|_| cursor_pos())
            .map(|(x, y)| abs_move_input(x, y));
        let target = target.map(|(x, y)| abs_move_input(x, y));
        let down_index = usize::from(target.is_some());
        let event_count =
            down_index + 1 + usize::from(complete) * (1 + usize::from(back.is_some()));
        let accepted = owner.press_sampled(
            run.id,
            PressBatch {
                target,
                down,
                up,
                back,
                complete,
                restore_after,
            },
            action_allowed_locked(run, &mut engine, gate, failsafe),
            vk,
            button_held,
            |batch| unsafe { SendInput(batch, std::mem::size_of::<INPUT>() as i32) as usize },
        );
        if accepted > 0
            && ((complete && accepted < event_count) || (!complete && accepted <= down_index))
            && !owner.release_sampled(run.id, button_held, send_one)
        {
            eprintln!("TrayClick: owned input release failed; ownership retained");
        }
        accepted > down_index
    }

    fn release_owned(id: u64) -> bool {
        let released = input_owner()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .release_sampled(id, button_held, send_one);
        if !released {
            eprintln!("TrayClick: owned input release failed; ownership retained");
        } else if current_mode() == ClickMode::Hold && HELD.load(Ordering::SeqCst) == 0 {
            // A restarted Hold run may have waited for the old action's cleanup.
            post_main(WM_APP_GATE);
        }
        released
    }

    fn button_vk(button: ClickButton) -> u32 {
        match button {
            ClickButton::Left => 1,
            ClickButton::Right => 2,
            ClickButton::Middle => 4,
        }
    }

    fn action_wait(run: ActionRun, duration: Duration) -> bool {
        wait_for_action(&mut ActionWait, run, Instant::now() + duration)
    }

    fn count_for(run: ActionRun) {
        let _engine = lock_engine();
        CLICK_COUNT.fetch_add(1, Ordering::Relaxed);
        if same_run(run) {
            RUN_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    static MAIN_HWND: AtomicI32 = AtomicI32::new(0);
    /// The update prompt window while it is open (0 = not open), so
    /// the check can never stack a second one.
    static UPDATE_HWND: AtomicI32 = AtomicI32::new(0);
    /// True while a dialog window (Set Keybind or Set Interval) is open;
    /// the polling thread must not treat keys pressed there as toggles.
    static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);
    /// Set when the capture window closes: the polling thread resyncs its
    /// press detector, so the key still held from choosing it does not
    /// count as a press. Clicking starts only on the next fresh press.
    static RESYNC_DETECTOR: AtomicBool = AtomicBool::new(false);
    /// What the capture window is assigning: 0 = the toggle keybind,
    /// 1 = the Key action's repeat key.
    static CAPTURE_FOR: AtomicU32 = AtomicU32::new(0);
    /// The registered `TaskbarCreated` message id (0 until `run` sets
    /// it). Explorer broadcasts it when the taskbar is (re)created; the
    /// tray icon must then be re-added or it stays gone after an
    /// Explorer restart.
    static TASKBAR_CREATED_MSG: AtomicU32 = AtomicU32::new(0);
    /// Which blink phase is on screen: `true` while the normal icon is
    /// showing, `false` while the active icon is. Only touched on the
    /// UI thread (timer + state-change handlers).
    static BLINK_NORMAL_SHOWN: AtomicBool = AtomicBool::new(true);

    fn engine() -> &'static Mutex<Engine> {
        ENGINE.get_or_init(|| Mutex::new(Engine::new()))
    }

    /// Lock the engine, recovering the state if a previous holder
    /// panicked while holding it. `Engine` is two bools — always safe to
    /// keep using — and panicking inside a window proc (or silently
    /// ignoring the user's keypress) would be strictly worse.
    fn lock_engine() -> MutexGuard<'static, Engine> {
        engine()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    // ---------- config persistence ----------

    /// Where every version before 0.10.0 kept the config:
    /// `%APPDATA%\TrayClick\config.json`.
    fn legacy_config_path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("TrayClick").join("config.json")
    }

    /// The directory the running executable lives in.
    fn exe_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// The portable config location since 0.10.0: `data\config.json`
    /// next to the executable, for portable and installed builds
    /// alike (the installer is per-user, so the folder is writable).
    fn portable_config_path() -> PathBuf {
        exe_dir().join("data").join("config.json")
    }

    /// The config file this launch uses, resolved once: see
    /// [`choose_config_file`] for the decision. A legacy file the
    /// choice says to migrate is copied over here, best-effort,
    /// before anything reads the chosen path.
    fn config_choice() -> &'static ConfigChoice {
        static CHOICE: OnceLock<ConfigChoice> = OnceLock::new();
        CHOICE.get_or_init(|| {
            let portable = portable_config_path();
            let legacy = legacy_config_path();
            let portable_exists = portable.exists();
            // Creating the data dir is the writability probe; it is
            // skipped when a portable config already exists (the
            // choice ignores writability in that case anyway).
            let portable_dir_writable = portable_exists
                || portable
                    .parent()
                    .map(|d| std::fs::create_dir_all(d).is_ok())
                    .unwrap_or(false);
            let choice = choose_config_file(
                &portable,
                &legacy,
                portable_exists,
                legacy.exists(),
                portable_dir_writable,
            );
            if let Some(from) = &choice.migrate_from {
                let _ = std::fs::copy(from, &choice.path);
            }
            choice
        })
    }

    fn config_path() -> PathBuf {
        config_choice().path.clone()
    }

    fn load_config() -> Config {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            // A malformed file falls back to defaults and is left untouched.
            Ok(text) => Config::from_json(&text).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    fn save_config(cfg: Config) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Write to a temp file and rename it over the real one, so a
        // crash mid-write can never leave a torn config behind (a torn
        // file would make the next launch silently reset both settings
        // to defaults). `fs::rename` replaces an existing destination
        // on Windows. If the rename fails, fall back to a direct write.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, cfg.to_json()).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
            return;
        }
        let _ = std::fs::write(&path, cfg.to_json());
    }

    fn button_u32(b: ClickButton) -> u32 {
        match b {
            ClickButton::Left => 0,
            ClickButton::Right => 1,
            ClickButton::Middle => 2,
        }
    }

    fn current_button() -> ClickButton {
        match BUTTON.load(Ordering::SeqCst) {
            1 => ClickButton::Right,
            2 => ClickButton::Middle,
            _ => ClickButton::Left,
        }
    }

    fn mode_u32(m: ClickMode) -> u32 {
        match m {
            ClickMode::Click => 0,
            ClickMode::Hold => 1,
            ClickMode::Momentary => 2,
        }
    }

    fn current_mode() -> ClickMode {
        match MODE.load(Ordering::SeqCst) {
            1 => ClickMode::Hold,
            2 => ClickMode::Momentary,
            _ => ClickMode::Click,
        }
    }

    fn action_u32(a: ClickAction) -> u32 {
        match a {
            ClickAction::Single => 0,
            ClickAction::Double => 1,
            ClickAction::Burst => 2,
            ClickAction::Scroll => 3,
            ClickAction::Key => 4,
            ClickAction::Drag => 5,
        }
    }

    fn current_action() -> ClickAction {
        match ACTION.load(Ordering::SeqCst) {
            1 => ClickAction::Double,
            2 => ClickAction::Burst,
            3 => ClickAction::Scroll,
            4 => ClickAction::Key,
            5 => ClickAction::Drag,
            _ => ClickAction::Single,
        }
    }

    fn target_u32(t: TargetMode) -> u32 {
        match t {
            TargetMode::Follow => 0,
            TargetMode::Locked => 1,
            TargetMode::Sequence => 2,
        }
    }

    fn target_mode() -> TargetMode {
        match TARGET_MODE.load(Ordering::SeqCst) {
            1 => TargetMode::Locked,
            2 => TargetMode::Sequence,
            _ => TargetMode::Follow,
        }
    }

    fn slot_names() -> &'static Mutex<[String; 3]> {
        SLOT_NAMES.get_or_init(|| Mutex::new([String::new(), String::new(), String::new()]))
    }

    fn fg_exe() -> &'static Mutex<String> {
        FG_EXE.get_or_init(|| Mutex::new(String::new()))
    }

    fn delay_end() -> &'static Mutex<Option<Instant>> {
        DELAY_END.get_or_init(|| Mutex::new(None))
    }

    fn pending_profile() -> &'static Mutex<String> {
        PENDING_PROFILE.get_or_init(|| Mutex::new(String::new()))
    }

    fn update_offer() -> &'static Mutex<Option<(String, String)>> {
        UPDATE_OFFER.get_or_init(|| Mutex::new(None))
    }

    fn app_pick_exes() -> &'static Mutex<Vec<String>> {
        APP_PICK_EXES.get_or_init(|| Mutex::new(Vec::new()))
    }

    fn region_base() -> &'static Mutex<Option<[u32; 64]>> {
        REGION_BASE.get_or_init(|| Mutex::new(None))
    }

    fn page_controls() -> &'static Mutex<Vec<(i32, u32)>> {
        PAGE_CONTROLS.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// The live settings as a `Config`, for persisting after a change.
    fn current_config() -> Config {
        let names = slot_names()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Config {
            keybind_vk: KEYBIND_VK.load(Ordering::SeqCst),
            interval_ms: INTERVAL_MS.load(Ordering::SeqCst),
            button: current_button(),
            jitter: JITTER_ON.load(Ordering::SeqCst),
            mode: current_mode(),
            lock_position: target_mode() != TargetMode::Follow,
            limit_clicks: LIMIT_CLICKS.load(Ordering::SeqCst),
            limit_seconds: LIMIT_SECONDS.load(Ordering::SeqCst),
            start_on_launch: START_ON_LAUNCH.load(Ordering::SeqCst),
            action: current_action(),
            burst_clicks: BURST_CLICKS.load(Ordering::SeqCst),
            burst_gap_ms: BURST_GAP_MS.load(Ordering::SeqCst),
            press_ms: PRESS_MS.load(Ordering::SeqCst),
            alternate: ALTERNATE.load(Ordering::SeqCst),
            scroll_amount: SCROLL_AMT.load(Ordering::SeqCst),
            repeat_vk: REPEAT_VK.load(Ordering::SeqCst),
            drag_ax: DRAG_AX.load(Ordering::SeqCst),
            drag_ay: DRAG_AY.load(Ordering::SeqCst),
            drag_bx: DRAG_BX.load(Ordering::SeqCst),
            drag_by: DRAG_BY.load(Ordering::SeqCst),
            drag_set: DRAG_SET.load(Ordering::SeqCst),
            random_interval: RANDOM_ON.load(Ordering::SeqCst),
            interval_max_ms: INTERVAL_MAX_MS.load(Ordering::SeqCst),
            ramp: RAMP_ON.load(Ordering::SeqCst),
            ramp_to_ms: RAMP_TO_MS.load(Ordering::SeqCst),
            ramp_seconds: RAMP_SECONDS.load(Ordering::SeqCst),
            target: target_mode(),
            lock_x: LOCK_X.load(Ordering::SeqCst),
            lock_y: LOCK_Y.load(Ordering::SeqCst),
            lock_set: LOCK_SET.load(Ordering::SeqCst),
            lock_rel: LOCK_REL.load(Ordering::SeqCst),
            lock_rx: LOCK_RX.load(Ordering::SeqCst),
            lock_ry: LOCK_RY.load(Ordering::SeqCst),
            seq_count: SEQ_COUNT.load(Ordering::SeqCst),
            seq_x: std::array::from_fn(|i| SEQ_X[i].load(Ordering::SeqCst)),
            seq_y: std::array::from_fn(|i| SEQ_Y[i].load(Ordering::SeqCst)),
            slot_x: std::array::from_fn(|i| SLOT_X[i].load(Ordering::SeqCst)),
            slot_y: std::array::from_fn(|i| SLOT_Y[i].load(Ordering::SeqCst)),
            slot_set: std::array::from_fn(|i| SLOT_SET[i].load(Ordering::SeqCst)),
            slot_names: names.clone(),
            offset_x: OFFSET_X.load(Ordering::SeqCst),
            offset_y: OFFSET_Y.load(Ordering::SeqCst),
            pixel_check: PIXEL_CHECK.load(Ordering::SeqCst),
            pixel_set: PIXEL_SET.load(Ordering::SeqCst),
            pixel_rgb: PIXEL_RGB.load(Ordering::SeqCst),
            pixel_tol: PIXEL_TOL.load(Ordering::SeqCst),
            region_guard: REGION_GUARD.load(Ordering::SeqCst),
            stop_vk: STOP_VK.load(Ordering::SeqCst),
            hotkey_mode: HOTKEY_MODE.load(Ordering::SeqCst),
            fg_rule: FG_RULE.load(Ordering::SeqCst),
            fg_exe: fg_exe()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            fs_pause: FS_PAUSE.load(Ordering::SeqCst),
        }
    }

    fn apply_interval(ms: u32) {
        INTERVAL_MS.store(ms, Ordering::SeqCst);
        save_config(current_config());
    }

    // ---------- click + keybind worker threads ----------

    /// The SendInput down/up flags for a button.
    fn button_flags(b: ClickButton) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
        match b {
            ClickButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            ClickButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            ClickButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        }
    }

    fn mouse_input(dx: i32, dy: i32, dw_flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: 0,
                    dwFlags: dw_flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// The cursor's current position, if it can be read.
    fn cursor_pos() -> Option<(i32, i32)> {
        unsafe {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                Some((pt.x, pt.y))
            } else {
                None
            }
        }
    }

    /// The locked point, resolved live: the persistent point when one
    /// is set (following its window while window-relative is on and
    /// that window is alive), else the point captured when the run
    /// started, else wherever the cursor is now.
    fn locked_point() -> (i32, i32) {
        if LOCK_SET.load(Ordering::SeqCst) {
            let hwnd_raw = LOCK_HWND.load(Ordering::SeqCst);
            if LOCK_REL.load(Ordering::SeqCst) && hwnd_raw != 0 {
                unsafe {
                    let hwnd = HWND(hwnd_raw as isize as *mut c_void);
                    if IsWindow(Some(hwnd)).as_bool() {
                        let mut rc = RECT::default();
                        if GetWindowRect(hwnd, &mut rc).is_ok() {
                            return (
                                rc.left + LOCK_RX.load(Ordering::SeqCst),
                                rc.top + LOCK_RY.load(Ordering::SeqCst),
                            );
                        }
                    }
                }
            }
            (LOCK_X.load(Ordering::SeqCst), LOCK_Y.load(Ordering::SeqCst))
        } else if RUN_SET.load(Ordering::SeqCst) {
            (RUN_X.load(Ordering::SeqCst), RUN_Y.load(Ordering::SeqCst))
        } else {
            cursor_pos().unwrap_or((0, 0))
        }
    }

    /// The point this cycle acts on, or `None` to act at the cursor.
    /// The configured offset applies to every fixed point; in Follow
    /// mode with an offset set, the point is the cursor plus the
    /// offset (the click darts there and back, like a locked click).
    fn cycle_point() -> Option<(i32, i32)> {
        let ox = OFFSET_X.load(Ordering::SeqCst);
        let oy = OFFSET_Y.load(Ordering::SeqCst);
        match target_mode() {
            TargetMode::Follow => {
                if ox == 0 && oy == 0 {
                    None
                } else {
                    cursor_pos().map(|(x, y)| (x + ox, y + oy))
                }
            }
            TargetMode::Locked => {
                let (x, y) = locked_point();
                Some((x + ox, y + oy))
            }
            TargetMode::Sequence => {
                let n = SEQ_COUNT.load(Ordering::SeqCst);
                if n == 0 {
                    None
                } else {
                    let i = (SEQ_IDX.load(Ordering::SeqCst) % n) as usize;
                    Some((
                        SEQ_X[i].load(Ordering::SeqCst) + ox,
                        SEQ_Y[i].load(Ordering::SeqCst) + oy,
                    ))
                }
            }
        }
    }

    /// One accepted click owns its captured release, even after cancellation.
    fn click_once(run: ActionRun, btn: ClickButton, point: Option<(i32, i32)>) -> bool {
        let (down, up) = button_flags(btn);
        let press = PRESS_MS.load(Ordering::SeqCst);
        if press == 0 {
            let accepted = press_targeted(
                run,
                button_vk(btn),
                mouse_input(0, 0, down),
                mouse_input(0, 0, up),
                point,
                true,
                true,
            );
            if accepted {
                count_for(run);
            }
            return accepted;
        }
        let mut input = PressInput {
            vk: button_vk(btn),
            down: mouse_input(0, 0, down),
            up: mouse_input(0, 0, up),
            drag: None,
            target: point,
        };
        let accepted = perform_press(&mut input, run, Duration::from_millis(press as u64), false);
        if accepted {
            count_for(run);
        }
        accepted
    }

    /// An absolute mouse-move input to a screen point (virtual-desktop
    /// coordinates), used by the Drag action.
    fn abs_move_input(x: i32, y: i32) -> INPUT {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            mouse_input(
                absolute_coord(x, vx, vw) as i32,
                absolute_coord(y, vy, vh) as i32,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            )
        }
    }

    /// One Scroll cycle: wheel ticks at the cursor — or, when the
    /// cycle resolved a point, a dart to it, the ticks, and a dart
    /// back, in one batch (the wheel goes to the window under the
    /// pointer).
    fn scroll_once(run: ActionRun, point: Option<(i32, i32)>) -> bool {
        let amt = SCROLL_AMT.load(Ordering::SeqCst);
        let mut engine = lock_engine();
        let mut owner = input_owner().lock().unwrap_or_else(|p| p.into_inner());
        if !action_allowed_locked(
            run,
            &mut engine,
            gate_should_be_open(),
            cursor_in_failsafe(),
        ) {
            return false;
        }
        let mut wheel = mouse_input(0, 0, MOUSEEVENTF_WHEEL);
        wheel.Anonymous.mi.mouseData = (amt * 120) as u32;
        let back = point
            .and_then(|_| cursor_pos())
            .map(|(x, y)| abs_move_input(x, y));
        let accepted = owner.scroll_sampled(
            run.id,
            point.map(|(x, y)| abs_move_input(x, y)),
            wheel,
            back,
            button_held,
            |batch| unsafe { SendInput(batch, std::mem::size_of::<INPUT>() as i32) as usize },
        );
        if owner.owner() == Some(run.id) {
            owner.release_sampled(run.id, button_held, send_one);
        }
        drop(owner);
        drop(engine);
        if accepted {
            count_for(run);
        }
        accepted
    }

    /// Convert a GDI `COLORREF` (`0x00BBGGRR`) to packed `0xRRGGBB`.
    fn rgb_of(c: COLORREF) -> u32 {
        let r = c.0 & 0xFF;
        let g = (c.0 >> 8) & 0xFF;
        let b = (c.0 >> 16) & 0xFF;
        (r << 16) | (g << 8) | b
    }

    /// Read one screen pixel as packed `0xRRGGBB`; `None` when the
    /// read fails (`GetPixel` returns `CLR_INVALID`, e.g. the point
    /// is off-screen).
    fn sample_pixel(x: i32, y: i32) -> Option<u32> {
        unsafe {
            let dc: HDC = GetDC(None);
            if dc.0.is_null() {
                return None;
            }
            let c = GetPixel(dc, x, y);
            let _ = ReleaseDC(None, dc);
            if c.0 == u32::MAX {
                None
            } else {
                Some(rgb_of(c))
            }
        }
    }

    /// The pixel check: while it is on (and a colour is sampled), a
    /// cycle may act only while the pixel at its point still matches
    /// the sampled colour within the tolerance.
    fn pixel_allows(point: Option<(i32, i32)>) -> bool {
        if !PIXEL_CHECK.load(Ordering::SeqCst) || !PIXEL_SET.load(Ordering::SeqCst) {
            return true;
        }
        let (x, y) = match point {
            Some(p) => p,
            None => match cursor_pos() {
                Some(p) => p,
                None => return false,
            },
        };
        match sample_pixel(x, y) {
            Some(rgb) => colour_matches(
                rgb,
                PIXEL_RGB.load(Ordering::SeqCst),
                PIXEL_TOL.load(Ordering::SeqCst),
            ),
            None => false,
        }
    }

    /// Sample an 8×8 grid of pixels (±14 px around the point, every
    /// 4 px) as a region signature. Unreadable pixels read as black.
    fn sample_region(cx: i32, cy: i32) -> [u32; 64] {
        let mut out = [0u32; 64];
        unsafe {
            let dc: HDC = GetDC(None);
            if dc.0.is_null() {
                return out;
            }
            let mut i = 0;
            for gy in 0..8 {
                for gx in 0..8 {
                    let c = GetPixel(dc, cx - 14 + gx * 4, cy - 14 + gy * 4);
                    out[i] = if c.0 == u32::MAX { 0 } else { rgb_of(c) };
                    i += 1;
                }
            }
            let _ = ReleaseDC(None, dc);
        }
        out
    }

    /// The region guard: the first call of a run samples the
    /// baseline; later calls report whether the region around the
    /// point has changed since. In Follow mode the patch anchors at
    /// the cursor position of that first call — the region is a
    /// fixed patch of screen even when clicks follow the cursor.
    fn region_tripped(point: Option<(i32, i32)>) -> bool {
        if !REGION_GUARD.load(Ordering::SeqCst) {
            return false;
        }
        let (x, y) = match point {
            Some(p) => p,
            None => {
                if REGION_ANCHOR_SET.load(Ordering::SeqCst) {
                    (
                        REGION_AX.load(Ordering::SeqCst),
                        REGION_AY.load(Ordering::SeqCst),
                    )
                } else {
                    match cursor_pos() {
                        Some(p) => {
                            REGION_AX.store(p.0, Ordering::SeqCst);
                            REGION_AY.store(p.1, Ordering::SeqCst);
                            REGION_ANCHOR_SET.store(true, Ordering::SeqCst);
                            p
                        }
                        None => return false,
                    }
                }
            }
        };
        let now = sample_region(x, y);
        let mut base = region_base()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *base {
            None => {
                *base = Some(now);
                false
            }
            Some(b) => region_changed(&b, &now),
        }
    }

    /// Stop clicking because the watched region changed, and tell
    /// the UI thread so it can settle the tray indicator.
    fn stop_run(run: ActionRun, message: u32) {
        let mut engine = lock_engine();
        if same_run(run) {
            engine.clicking = false;
            post_main(message);
        }
    }

    fn key_input(vk: u32, up: bool) -> INPUT {
        let mut flags = KEYBD_EVENT_FLAGS::default();
        if is_extended_key(vk) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk as u16),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// One Key cycle: press and release the repeat key, honouring the
    /// button-down duration as the key-down duration.
    fn key_once(run: ActionRun) {
        let vk = REPEAT_VK.load(Ordering::SeqCst);
        let press = PRESS_MS.load(Ordering::SeqCst);
        let mut input = PressInput {
            vk,
            down: key_input(vk, false),
            up: key_input(vk, true),
            drag: None,
            target: None,
        };
        if perform_press(&mut input, run, Duration::from_millis(press as u64), false) {
            count_for(run);
        }
    }

    /// A cancelled drag stops moving promptly, then releases only its accepted DOWN.
    fn drag_once(run: ActionRun) {
        let button = current_button();
        let (down, up) = button_flags(button);
        let ax = DRAG_AX.load(Ordering::SeqCst);
        let ay = DRAG_AY.load(Ordering::SeqCst);
        let bx = DRAG_BX.load(Ordering::SeqCst);
        let by = DRAG_BY.load(Ordering::SeqCst);
        let mut input = PressInput {
            vk: button_vk(button),
            down: mouse_input(0, 0, down),
            up: mouse_input(0, 0, up),
            drag: Some(((ax, ay), (bx, by))),
            target: Some((ax, ay)),
        };
        let press = Duration::from_millis(PRESS_MS.load(Ordering::SeqCst) as u64);
        if perform_press(&mut input, run, press, true) {
            count_for(run);
        }
    }

    /// One scheduled cycle, with the slice-3 guards applied first:
    /// the region guard may stop the run, and the pixel check may
    /// skip the cycle (silently — the run continues). Both apply
    /// only to the point-acting actions (clicks and scroll); Key and
    /// Drag cycles have no target pixel to watch.
    fn run_cycle(run: ActionRun) {
        if !ActionWait.allowed(run) {
            return;
        }
        let action = current_action();
        let pointed = action.is_click_action() || action == ClickAction::Scroll;
        let point = if pointed { cycle_point() } else { None };
        if pointed {
            if region_tripped(point) {
                stop_run(run, WM_APP_REGION);
                return;
            }
            if !pixel_allows(point) {
                return;
            }
        }
        let accepted = perform_cycle(run, point);
        let _engine = lock_engine();
        if accepted && pointed && same_run(run) && target_mode() == TargetMode::Sequence {
            SEQ_IDX.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// One activation cycle, dispatched on the configured action.
    /// The click actions (single, double, burst of clicks spaced by
    /// the burst gap) stop early if clicking was switched off, the
    /// failsafe trips, or the run hits its auto-stop limit — a long
    /// burst never outlives any of them. The clicker's own limit
    /// check settles the stop afterwards.
    fn perform_cycle(run: ActionRun, point: Option<(i32, i32)>) -> bool {
        match current_action() {
            ClickAction::Scroll => {
                if ActionWait.allowed(run) {
                    return scroll_once(run, point);
                }
                return false;
            }
            ClickAction::Key => {
                key_once(run);
                return false;
            }
            ClickAction::Drag => {
                if DRAG_SET.load(Ordering::SeqCst) {
                    drag_once(run);
                }
                return false;
            }
            _ => {}
        }
        let n = match current_action() {
            ClickAction::Single => 1,
            ClickAction::Double => 2,
            _ => BURST_CLICKS.load(Ordering::SeqCst),
        };
        let gap = Duration::from_millis(BURST_GAP_MS.load(Ordering::SeqCst) as u64);
        let mut accepted = false;
        for i in 0..n {
            if i > 0 && !action_wait(run, gap) {
                return accepted;
            }
            let btn = effective_button(
                current_button(),
                ALTERNATE.load(Ordering::SeqCst),
                RUN_COUNT.load(Ordering::Relaxed),
            );
            if !click_once(run, btn, point) {
                return accepted;
            }
            accepted = true;
            if limit_reached(0) {
                return accepted;
            }
        }
        accepted
    }

    /// Whether the current run has reached an auto-stop limit:
    /// the click limit or the elapsed-seconds limit, whichever is
    /// reached first (a limit of 0 is off).
    fn limit_reached(elapsed_s: u64) -> bool {
        auto_stop_reached(
            RUN_COUNT.load(Ordering::Relaxed),
            LIMIT_CLICKS.load(Ordering::SeqCst),
            elapsed_s,
            LIMIT_SECONDS.load(Ordering::SeqCst),
        )
    }

    /// Post a message to the main window (used by the worker threads).
    fn post_main(msg: u32) {
        let raw = MAIN_HWND.load(Ordering::SeqCst);
        if raw != 0 {
            let hwnd = HWND(raw as isize as *mut c_void);
            unsafe {
                let _ = PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn main_hwnd() -> Option<HWND> {
        let raw = MAIN_HWND.load(Ordering::SeqCst);
        if raw != 0 {
            Some(HWND(raw as isize as *mut c_void))
        } else {
            None
        }
    }

    /// Cancel a pending delayed start, if one is armed.
    fn cancel_delay() {
        let was = delay_end()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if was.is_some() {
            if let Some(hwnd) = main_hwnd() {
                unsafe {
                    let _ = KillTimer(Some(hwnd), ID_DELAY_TIMER);
                }
            }
        }
    }

    /// Side effects shared by every path that starts a run: reset
    /// the per-run counters, cancel any pending delayed start, and
    /// arm the locked-target fallback point when one is needed.
    fn on_run_started() {
        let started = Instant::now();
        let seconds = LIMIT_SECONDS.load(Ordering::SeqCst);
        *ACTION_RUN.lock().unwrap_or_else(|p| p.into_inner()) = Some(ActionRun {
            id: RUN_ID.fetch_add(1, Ordering::SeqCst) + 1,
            started,
            deadline: (seconds != 0).then(|| started + Duration::from_secs(seconds as u64)),
        });
        RUN_COUNT.store(0, Ordering::Relaxed);
        SEQ_IDX.store(0, Ordering::SeqCst);
        RUN_SET.store(false, Ordering::SeqCst);
        cancel_delay();
        // A Locked run with no persistent point falls back to the
        // position where it was started (not in Hold mode, which
        // never uses a point).
        if current_mode() != ClickMode::Hold
            && target_mode() == TargetMode::Locked
            && !LOCK_SET.load(Ordering::SeqCst)
        {
            capture_lock_point();
        }
    }

    /// One press of the toggle keybind (from the poll thread's
    /// message, a registered hotkey, or the CLI toggle command).
    fn handle_keybind_press(hwnd: HWND) {
        {
            let mut engine = lock_engine();
            if current_mode() == ClickMode::Momentary {
                if !engine.is_clicking_now() {
                    engine.clicking = true;
                    on_run_started();
                }
            } else {
                engine.press_keybind();
                if engine.is_clicking_now() {
                    on_run_started();
                } else {
                    RUN_SET.store(false, Ordering::SeqCst);
                }
            }
        }
        refresh_indicator(hwnd);
    }

    /// Stop clicking, whatever started it (stop key, CLI --stop).
    fn engine_stop(hwnd: HWND) {
        lock_engine().clicking = false;
        RUN_SET.store(false, Ordering::SeqCst);
        refresh_indicator(hwnd);
    }

    /// Start clicking unconditionally (CLI --start, delayed start):
    /// clears a pause as well as starting.
    fn engine_start(hwnd: HWND) {
        {
            let mut e = lock_engine();
            e.paused = false;
            if !e.clicking {
                e.clicking = true;
                on_run_started();
            }
        }
        refresh_indicator(hwnd);
    }

    /// (Re)register the system hotkeys from the current bindings.
    /// Mouse-button bindings cannot be hotkeys (they keep polling),
    /// and a failed registration falls back to polling too — the OK
    /// flags record what is actually live, and the poll thread reads
    /// them every iteration.
    fn register_hotkeys(hwnd: HWND) {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), ID_HOTKEY_KEYBIND);
            let _ = UnregisterHotKey(Some(hwnd), ID_HOTKEY_STOP);
        }
        KB_HOTKEY_OK.store(false, Ordering::SeqCst);
        STOP_HOTKEY_OK.store(false, Ordering::SeqCst);
        if !HOTKEY_MODE.load(Ordering::SeqCst) {
            return;
        }
        let kb = KEYBIND_VK.load(Ordering::SeqCst);
        if kb >= 0x08 {
            let ok =
                unsafe { RegisterHotKey(Some(hwnd), ID_HOTKEY_KEYBIND, MOD_NOREPEAT, kb).is_ok() };
            KB_HOTKEY_OK.store(ok, Ordering::SeqCst);
        }
        let sv = STOP_VK.load(Ordering::SeqCst);
        if sv != 0 && sv >= 0x08 {
            let ok =
                unsafe { RegisterHotKey(Some(hwnd), ID_HOTKEY_STOP, MOD_NOREPEAT, sv).is_ok() };
            STOP_HOTKEY_OK.store(ok, Ordering::SeqCst);
        }
    }

    /// Capture the cursor as this run's fallback lock point (used
    /// for a Locked run when no persistent lock point is set).
    fn capture_lock_point() {
        if let Some((x, y)) = cursor_pos() {
            RUN_X.store(x, Ordering::SeqCst);
            RUN_Y.store(y, Ordering::SeqCst);
            RUN_SET.store(true, Ordering::SeqCst);
        }
    }

    /// Release the button if it is held, unconditionally.
    fn sync_hold_state_released() {
        let held = HELD.load(Ordering::SeqCst);
        if held != 0 && release_owned(held) {
            HELD.store(0, Ordering::SeqCst);
        }
    }

    fn release_inputs_on_exit() {
        lock_engine().clicking = false;
        sync_hold_state_released();
        let owner = input_owner()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .owner();
        if let Some(id) = owner {
            release_owned(id);
        }
    }

    /// Hold cleanup remains on every refresh path and retains the accepted button.
    fn sync_hold_state() {
        let run = action_run();
        let want = run.is_some_and(|run| ActionWait.allowed(run))
            && current_mode() == ClickMode::Hold
            && current_action().is_click_action();
        let held = HELD.load(Ordering::SeqCst);
        if held != 0 && (!want || run.is_none_or(|run| run.id != held)) {
            sync_hold_state_released();
        }
        if want && HELD.load(Ordering::SeqCst) == 0 {
            if let Some(run) = run {
                let button = current_button();
                let (down, up) = button_flags(button);
                if press_owned(
                    run,
                    button_vk(button),
                    mouse_input(0, 0, down),
                    mouse_input(0, 0, up),
                ) {
                    HELD.store(run.id, Ordering::SeqCst);
                }
            }
        }
    }

    /// A tiny xorshift PRNG for interval jitter — good enough to vary
    /// a cadence; not cryptographic, and nothing here needs it to be.
    struct XorShift(u32);

    impl XorShift {
        fn next(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
    }

    /// The interval for the next cycle. The base value is reshaped by
    /// at most one of the ramp (interval eases toward a target over
    /// the run) or the random range (each cycle picks within
    /// [base, max]); jitter, when on, then varies the result by up to
    /// ±10% as always.
    fn effective_interval(rng: &mut XorShift, run_start: Option<Instant>) -> Duration {
        let base = INTERVAL_MS.load(Ordering::SeqCst);
        let ms = if RAMP_ON.load(Ordering::SeqCst) && RAMP_TO_MS.load(Ordering::SeqCst) != 0 {
            let elapsed = run_start.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            ramped_interval_ms(
                base,
                RAMP_TO_MS.load(Ordering::SeqCst),
                elapsed,
                RAMP_SECONDS.load(Ordering::SeqCst) as u64,
            )
        } else if RANDOM_ON.load(Ordering::SeqCst) {
            let max = INTERVAL_MAX_MS.load(Ordering::SeqCst).max(base);
            if max > base {
                base + rng.next() % (max - base + 1)
            } else {
                base
            }
        } else {
            base
        };
        let ms = if JITTER_ON.load(Ordering::SeqCst) {
            jittered_interval_ms(ms, rng.next())
        } else {
            ms
        };
        Duration::from_millis(ms as u64)
    }

    /// The virtual screen's top-left corner, read once. The failsafe
    /// check runs in the clicker's hot loops, and the origin only moves
    /// when a monitor is added or removed — in that rare mid-run case
    /// the corner follows the layout as it was when first read, until
    /// TrayClick restarts. (The check failing safe — stopping — is the
    /// worst outcome of a stale origin.)
    fn failsafe_origin() -> (i32, i32) {
        static ORIGIN: OnceLock<(i32, i32)> = OnceLock::new();
        *ORIGIN.get_or_init(|| unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
            )
        })
    }

    /// True when the cursor is in the top-left corner failsafe zone.
    fn cursor_in_failsafe() -> bool {
        unsafe {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                let (vx, vy) = failsafe_origin();
                return in_failsafe_corner(pt.x, pt.y, vx, vy);
            }
            false
        }
    }

    /// Scheduling uses the same bounded cancellation checks as action waits.
    fn wait_until(run: ActionRun, deadline: Instant) {
        wait_for_action(&mut ActionWait, run, deadline);
    }

    /// The foreground window's process image name (basename) and
    /// whether the window covers its whole monitor. `(None, false)`
    /// when there is no ordinary foreground window — the desktop
    /// shell or one of TrayClick's own windows; a process whose name
    /// cannot be read also reports `None`.
    /// A process's image name (basename, e.g. `game.exe`), or
    /// `None` when it cannot be read (protected processes).
    fn process_exe_name(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 260];
            let mut len = buf.len() as u32;
            let r = QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_FORMAT(0),
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(h);
            r.ok()?;
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().map(|s| s.to_string())
        }
    }

    fn foreground_info() -> (Option<String>, bool) {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_invalid() || fg == GetShellWindow() {
                return (None, false);
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            if pid == 0 || pid == std::process::id() {
                return (None, false);
            }
            let name = process_exe_name(pid);
            let mut fullscreen = false;
            let mut rc = RECT::default();
            if GetWindowRect(fg, &mut rc).is_ok() {
                let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if GetMonitorInfoW(mon, &mut mi).as_bool() {
                    let m = mi.rcMonitor;
                    fullscreen = rect_is_fullscreen(
                        (rc.left, rc.top, rc.right, rc.bottom),
                        (m.left, m.top, m.right, m.bottom),
                    );
                }
            }
            (name, fullscreen)
        }
    }

    /// Whether the run gate should be open right now: the
    /// foreground-app rule (when on and an app is picked) requires
    /// that app in the foreground, and the fullscreen pause (when
    /// on) closes the gate while a fullscreen window is foreground.
    /// A foreground process whose name can't be read counts as "not
    /// the app" — such apps can't be matched by the rule.
    fn gate_should_be_open() -> bool {
        let fg_rule = FG_RULE.load(Ordering::SeqCst);
        let fs_pause = FS_PAUSE.load(Ordering::SeqCst);
        if !fg_rule && !fs_pause {
            return true;
        }
        let want = if fg_rule {
            fg_exe()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        } else {
            String::new()
        };
        let matching = fg_rule && !want.is_empty();
        if !matching && !fs_pause {
            return true;
        }
        let (exe, fullscreen) = foreground_info();
        if matching {
            match exe {
                Some(e) if e.eq_ignore_ascii_case(&want) => {}
                _ => return false,
            }
        }
        if fs_pause && fullscreen {
            return false;
        }
        true
    }

    fn clicker_thread() {
        // Clicks are scheduled on absolute deadlines (see
        // `next_click_deadline`), so the time a click itself takes does
        // not accumulate as drift the way sleep-after-click did.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0x9E37_79B9)
            | 1;
        let mut rng = XorShift(seed);
        let mut deadline: Option<Instant> = None;
        let mut run_start: Option<Instant> = None;
        let mut observed_run = 0;
        let mut last_gate = Instant::now() - Duration::from_millis(200);
        loop {
            if HELD.load(Ordering::SeqCst) == 0 {
                let pending = input_owner()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .cleanup_owner();
                if let Some(id) = pending {
                    release_owned(id);
                }
            }
            // Sample the run gate (foreground-app rule + fullscreen
            // pause) at most every 100 ms, whatever the engine state,
            // so flips are noticed mid-run and mid-idle alike.
            if last_gate.elapsed() >= Duration::from_millis(100) {
                last_gate = Instant::now();
                let open = gate_should_be_open();
                if open != GATE_OPEN.load(Ordering::SeqCst) {
                    GATE_OPEN.store(open, Ordering::SeqCst);
                    post_main(WM_APP_GATE);
                }
            }
            if !lock_engine().is_clicking_now() {
                deadline = None;
                run_start = None;
                // A stopped run forgets its region baseline; the
                // next run samples a fresh one.
                *region_base()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                REGION_ANCHOR_SET.store(false, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let Some(run) = action_run() else {
                continue;
            };
            if observed_run != run.id {
                observed_run = run.id;
                deadline = None;
                run_start = Some(run.started);
            }
            if current_mode() == ClickMode::Hold && current_action().is_click_action() {
                // Nothing to schedule — the button is simply down. Just
                // keep watch for the failsafe, and for the auto-stop
                // time limit: the hold's start is tracked so an elapsed
                // limit can end the hold like any other run (the stop
                // funnels through refresh_indicator, which releases the
                // held button). The click limit cannot advance here —
                // a hold performs no clicks.
                if cursor_in_failsafe() {
                    stop_run(run, WM_APP_FAILSAFE);
                }
                deadline = None;
                if run_start.is_none() {
                    run_start = Some(Instant::now());
                }
                let elapsed = run_start.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                if limit_reached(elapsed) {
                    stop_run(run, WM_APP_LIMIT);
                    run_start = None;
                }
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if !ActionWait.allowed(run) {
                deadline = None;
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            match deadline {
                None => {
                    if run_start.is_none() {
                        run_start = Some(Instant::now());
                    }
                    if GATE_OPEN.load(Ordering::SeqCst) {
                        run_cycle(run);
                    }
                    deadline = Some(Instant::now() + effective_interval(&mut rng, run_start));
                }
                Some(d) => {
                    if Instant::now() >= d {
                        if GATE_OPEN.load(Ordering::SeqCst) {
                            run_cycle(run);
                        }
                        deadline = Some(next_click_deadline(
                            d,
                            effective_interval(&mut rng, run_start),
                            Instant::now(),
                        ));
                    } else {
                        wait_until(run, d);
                    }
                }
            }
            let elapsed = run_start.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            if same_run(run) && limit_reached(elapsed) {
                stop_run(run, WM_APP_LIMIT);
                deadline = None;
            }
        }
    }

    fn keybind_thread() {
        let mut kb_press = KeyPressDetector::new();
        let mut kb_release = KeyReleaseDetector::new();
        let mut stop_press = KeyPressDetector::new();
        loop {
            std::thread::sleep(Duration::from_millis(8));
            if DIALOG_OPEN.load(Ordering::SeqCst) {
                continue;
            }
            if RESYNC_DETECTOR.swap(false, Ordering::SeqCst) {
                kb_press.resync();
                kb_release.resync();
                stop_press.resync();
            }
            let down_of = |vk: u32| {
                let st = unsafe { GetAsyncKeyState(vk as i32) };
                st & (0x8000u16 as i16) != 0
            };
            // The keybind. While a registered hotkey delivers it,
            // polling stands down — except in momentary mode, which
            // needs the release edge only polling can see (hotkey
            // mode therefore never engages for momentary).
            let kb_vk = KEYBIND_VK.load(Ordering::SeqCst);
            let kb_via_hotkey = HOTKEY_MODE.load(Ordering::SeqCst)
                && KB_HOTKEY_OK.load(Ordering::SeqCst)
                && current_mode() != ClickMode::Momentary;
            if !kb_via_hotkey {
                let down = down_of(kb_vk);
                if kb_press.sample(down) {
                    post_main(WM_APP_TOGGLE);
                }
                if current_mode() == ClickMode::Momentary && kb_release.sample(down) {
                    post_main(WM_APP_STOP);
                }
            }
            // The stop-only key (unset = 0 = skipped). A registered
            // hotkey delivers it when there is one.
            let stop_vk = STOP_VK.load(Ordering::SeqCst);
            if stop_vk != 0 {
                let stop_via_hotkey =
                    HOTKEY_MODE.load(Ordering::SeqCst) && STOP_HOTKEY_OK.load(Ordering::SeqCst);
                if !stop_via_hotkey && stop_press.sample(down_of(stop_vk)) {
                    post_main(WM_APP_STOP);
                }
            }
        }
    }

    // ---------- tray icon ----------

    // Resource IDs are passed to LoadIconW as integer-as-pointer values
    // (the MAKEINTRESOURCE convention) — not a real, dangling pointer.
    #[allow(clippy::manual_dangling_ptr)]
    fn app_icon() -> HICON {
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            // Resource ID 1 is the icon embedded by build.rs.
            if let Ok(icon) = LoadIconW(Some(instance.into()), PCWSTR(1 as *const u16)) {
                return icon;
            }
            LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
        }
    }

    fn tray_add(hwnd: HWND) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_ID;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = app_icon();
            let tip = wide("TrayClick");
            let n = tip.len().min(nid.szTip.len());
            nid.szTip[..n].copy_from_slice(&tip[..n]);
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }

    fn tray_remove(hwnd: HWND) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_ID;
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    /// The "clicking ON" icon (inverted red target), built once from the
    /// embedded `assets/icon_active.ico` bytes. Falls back to the normal
    /// icon if the embedded data ever fails to parse, so the worst case
    /// is a steady icon, not a broken one.
    fn active_icon() -> HICON {
        static ICON: OnceLock<isize> = OnceLock::new();
        let raw = *ICON.get_or_init(|| {
            let bytes = include_bytes!("../assets/icon_active.ico");
            let desired = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
            let created = ico_image_for_size(bytes, desired).and_then(|image| unsafe {
                CreateIconFromResourceEx(
                    image,
                    true,
                    0x0003_0000,
                    desired as i32,
                    desired as i32,
                    LR_DEFAULTCOLOR,
                )
                .ok()
            });
            created.map(|h| h.0 as isize).unwrap_or(0)
        });
        if raw == 0 {
            app_icon()
        } else {
            HICON(raw as *mut c_void)
        }
    }

    /// Swap the tray icon and tooltip in place.
    fn tray_modify(hwnd: HWND, icon: HICON, tip_text: &str) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_ID;
            nid.uFlags = NIF_ICON | NIF_TIP;
            nid.hIcon = icon;
            let tip = wide(tip_text);
            let n = tip.len().min(nid.szTip.len());
            nid.szTip[..n].copy_from_slice(&tip[..n]);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// The tooltip shown while active: the click count in click mode,
    /// the held button in hold mode.
    fn on_tooltip() -> String {
        if current_mode() == ClickMode::Hold {
            format!("TrayClick — holding {}", current_button().label())
        } else {
            format!(
                "TrayClick — clicking ON · {} clicks",
                format_count(CLICK_COUNT.load(Ordering::Relaxed))
            )
        }
    }

    /// Reflect the engine state in the tray. While clicking is on, the
    /// icon blinks between the normal and active icons (see the
    /// `WM_TIMER` handler) and the tooltip says so; when off, the timer
    /// stops and the normal icon is restored. Called after every engine
    /// state change, which all happen on the UI thread.
    fn refresh_indicator(hwnd: HWND) {
        if lock_engine().is_clicking_now() {
            BLINK_NORMAL_SHOWN.store(false, Ordering::SeqCst);
            tray_modify(hwnd, active_icon(), &on_tooltip());
            unsafe {
                let _ = SetTimer(Some(hwnd), ID_BLINK_TIMER, BLINK_MS, None);
            }
        } else {
            unsafe {
                let _ = KillTimer(Some(hwnd), ID_BLINK_TIMER);
            }
            BLINK_NORMAL_SHOWN.store(true, Ordering::SeqCst);
            tray_modify(hwnd, app_icon(), "TrayClick");
        }
        sync_hold_state();
    }

    // ---------- Start with Windows (HKCU Run key) ----------

    /// The Run value this build would write: the quoted current exe path.
    fn expected_autostart_value() -> String {
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "trayclick.exe".to_string());
        format!("\"{exe}\"")
    }

    /// The current Run value for TrayClick, if one is present.
    fn autostart_value() -> Option<String> {
        unsafe {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                None,
                KEY_READ,
                &mut hkey,
            ) != ERROR_SUCCESS
            {
                return None;
            }
            let mut ty = REG_VALUE_TYPE::default();
            let mut size: u32 = 0;
            let st = RegQueryValueExW(
                hkey,
                w!("TrayClick"),
                None,
                Some(&mut ty),
                None,
                Some(&mut size),
            );
            let mut value = None;
            if st == ERROR_SUCCESS && ty == REG_SZ && size >= 2 {
                let mut buf = vec![0u8; size as usize];
                let st2 = RegQueryValueExW(
                    hkey,
                    w!("TrayClick"),
                    None,
                    None,
                    Some(buf.as_mut_ptr()),
                    Some(&mut size),
                );
                if st2 == ERROR_SUCCESS {
                    let units: Vec<u16> = buf
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| u16::from_le_bytes(*c))
                        .collect();
                    value = Some(
                        String::from_utf16_lossy(&units)
                            .trim_end_matches('\0')
                            .to_string(),
                    );
                }
            }
            let _ = RegCloseKey(hkey);
            value
        }
    }

    /// True when the Run key already points at this exe.
    fn autostart_enabled() -> bool {
        autostart_value().as_deref() == Some(expected_autostart_value().as_str())
    }

    /// Write or remove the Run value. All failures are silent: the menu
    /// re-reads the state on every open, so a failed change simply shows
    /// up unchanged there.
    fn set_autostart(enable: bool) {
        unsafe {
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                None,
                KEY_SET_VALUE,
                &mut hkey,
            ) != ERROR_SUCCESS
            {
                return;
            }
            if enable {
                let val = wide(&expected_autostart_value());
                let bytes: &[u8] =
                    std::slice::from_raw_parts(val.as_ptr() as *const u8, val.len() * 2);
                let _ = RegSetValueExW(hkey, w!("TrayClick"), None, REG_SZ, Some(bytes));
            } else {
                let _ = RegDeleteValueW(hkey, w!("TrayClick"));
            }
            let _ = RegCloseKey(hkey);
        }
    }

    // ---------- tray menu ----------

    /// Left-click on the tray icon: open the settings window by the
    /// same path as the menu's Set Interval item. If the settings
    /// window is already open, bring it to the foreground instead of
    /// opening a second one (the open path itself no-ops while any
    /// dialog is up).
    fn open_settings_from_tray(hwnd: HWND) {
        let raw = INTERVAL_HWND.load(Ordering::SeqCst);
        if raw != 0 {
            let existing = HWND(raw as isize as *mut c_void);
            unsafe {
                if IsWindow(Some(existing)).as_bool() {
                    let _ = SetForegroundWindow(existing);
                    return;
                }
            }
        }
        open_interval_window(hwnd);
    }

    fn show_menu(hwnd: HWND) {
        unsafe {
            let menu = match CreatePopupMenu() {
                Ok(m) => m,
                Err(_) => return,
            };
            let (paused, vk) = {
                let e = lock_engine();
                (e.paused, KEYBIND_VK.load(Ordering::SeqCst))
            };
            let pause_label = if paused { "Resume" } else { "Pause" };
            let pause_w = wide(pause_label);
            let keybind_label = format!("Set Keybind [{}]", key_name(vk));
            let keybind_w = wide(&keybind_label);
            let interval_label =
                format!("Set Interval [{} ms]", INTERVAL_MS.load(Ordering::SeqCst));
            let interval_w = wide(&interval_label);
            let credits_w = wide("Credits");
            let exit_w = wide("Exit");
            let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE_RESUME, PCWSTR(pause_w.as_ptr()));
            let _ = AppendMenuW(menu, MF_STRING, ID_SET_KEYBIND, PCWSTR(keybind_w.as_ptr()));
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_SET_INTERVAL,
                PCWSTR(interval_w.as_ptr()),
            );
            let sw_w = wide("Start with Windows");
            let sw_flags = MF_STRING
                | if autostart_enabled() {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                };
            let _ = AppendMenuW(menu, sw_flags, ID_START_WITH_WINDOWS, PCWSTR(sw_w.as_ptr()));
            let _ = AppendMenuW(menu, MF_STRING, ID_CREDITS, PCWSTR(credits_w.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, PCWSTR(exit_w.as_ptr()));

            let mut pt = windows::Win32::Foundation::POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd);
            let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, Some(0), hwnd, None);
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
        }
    }

    fn show_credits(hwnd: HWND) {
        let keybind = key_name(KEYBIND_VK.load(Ordering::SeqCst));
        let interval = INTERVAL_MS.load(Ordering::SeqCst);
        let button = current_button().label();
        let jitter = if JITTER_ON.load(Ordering::SeqCst) {
            "on (±10%)"
        } else {
            "off"
        };
        let clicks = format_count(CLICK_COUNT.load(Ordering::Relaxed));
        let mode = current_mode().label();
        let action = match current_action() {
            ClickAction::Burst => format!("Burst ×{}", BURST_CLICKS.load(Ordering::SeqCst)),
            ClickAction::Scroll => {
                format!("Scroll ({:+} notches)", SCROLL_AMT.load(Ordering::SeqCst))
            }
            ClickAction::Key => {
                format!("Key [{}]", key_name(REPEAT_VK.load(Ordering::SeqCst)))
            }
            a => a.label().to_string(),
        };
        let target = match target_mode() {
            TargetMode::Locked => "Locked position".to_string(),
            TargetMode::Sequence => {
                format!("Sequence ({} points)", SEQ_COUNT.load(Ordering::SeqCst))
            }
            TargetMode::Follow => "Follow cursor".to_string(),
        };
        let stop_vk = STOP_VK.load(Ordering::SeqCst);
        let stop = if stop_vk == 0 {
            "not set".to_string()
        } else {
            key_name(stop_vk)
        };
        let mut rules = String::new();
        if FG_RULE.load(Ordering::SeqCst) {
            let exe = fg_label_caption();
            rules.push_str(&format!("\r\n{exe}"));
        }
        if FS_PAUSE.load(Ordering::SeqCst) {
            rules.push_str("\r\nFullscreen pause: on");
        }
        let text = wide(&format!(
            "TrayClick {}\r\n\r\n\
             Created by SEAN (SeanShaffer88).\r\n\
             Built with Rust and the Windows API.\r\n\r\n\
             Keybind: {keybind} — start or stop auto clicking at the cursor.\r\n\
             Stop key: {stop}\r\n\
             Interval: {interval} ms (set it from the tray menu).\r\n\
             Button: {button}\r\n\
             Mode: {mode}\r\n\
             Action: {action}\r\n\
             Target: {target}{rules}\r\n\
             Jitter: {jitter}\r\n\
             Clicks this session: {clicks}\r\n\r\n\
             Safety: while clicking, slam the mouse into the top-left corner\
             of the screen to stop instantly.\r\n\r\n\
             MIT License — see LICENSE.",
            env!("CARGO_PKG_VERSION")
        ));
        let title = wide("TrayClick — Credits");
        unsafe {
            MessageBoxW(
                Some(hwnd),
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }

    /// The dialogs' dark background brush, created once and shared.
    ///
    /// A brush returned for `WM_CTLCOLORSTATIC` stays the app's
    /// responsibility — creating a new one per repaint (as this code
    /// used to) leaks a GDI object every time, toward the per-process
    /// limit of 10,000. This one lives for the process lifetime.
    fn dialog_bg_brush() -> HBRUSH {
        static BRUSH: OnceLock<isize> = OnceLock::new();
        let raw =
            *BRUSH.get_or_init(|| unsafe { CreateSolidBrush(COLORREF(0x00181818)).0 } as isize);
        HBRUSH(raw as *mut c_void)
    }

    /// A process-lifetime solid brush from the settings window's
    /// charcoal/red palette, created once per colour. Like
    /// `dialog_bg_brush`, a brush handed to a `WM_CTLCOLOR*` handler
    /// stays the app's responsibility, so it is never created per
    /// repaint.
    fn palette_brush(slot: &'static OnceLock<isize>, rgb: u32) -> HBRUSH {
        let raw = *slot.get_or_init(|| unsafe { CreateSolidBrush(COLORREF(rgb)).0 } as isize);
        HBRUSH(raw as *mut c_void)
    }

    /// The settings window's charcoal background (#1B1B1B).
    fn settings_bg_brush() -> HBRUSH {
        static BRUSH: OnceLock<isize> = OnceLock::new();
        palette_brush(&BRUSH, 0x001B_1B1B)
    }

    /// Edit background in the settings window (#262626).
    fn edit_bg_brush() -> HBRUSH {
        static BRUSH: OnceLock<isize> = OnceLock::new();
        palette_brush(&BRUSH, 0x0026_2626)
    }

    /// The top bar Start button's red fill (#8F1113).
    fn start_fill_brush() -> HBRUSH {
        static BRUSH: OnceLock<isize> = OnceLock::new();
        palette_brush(&BRUSH, 0x0013_118F)
    }

    /// The red accent (#E5484D): the Start/Stop borders and the
    /// Stop button's caption.
    fn accent_brush() -> HBRUSH {
        static BRUSH: OnceLock<isize> = OnceLock::new();
        palette_brush(&BRUSH, 0x004D_48E5)
    }

    // ---------- Set Keybind capture window ----------

    unsafe extern "system" fn capture_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = wparam.0 as u32;
                if vk == VK_ESCAPE_CODE || assign_capture_key(hwnd, vk) {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_XBUTTONDOWN => {
                // Mouse side buttons can be the keybind or the stop
                // key (never the repeat key — it must be sendable as
                // a keyboard event). GET_XBUTTON_WPARAM: 1 = X1,
                // 2 = X2, mapping to VK 0x05 / 0x06.
                let xb = (wparam.0 >> 16) & 0xFFFF;
                let vk = 0x04 + xb as u32;
                if (vk == 0x05 || vk == 0x06) && assign_capture_key(hwnd, vk) {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, COLORREF(0x00F2F2F2));
                SetBkMode(hdc, TRANSPARENT);
                LRESULT(dialog_bg_brush().0 as isize)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                DIALOG_OPEN.store(false, Ordering::SeqCst);
                // The assigning keypress is still physically held; make
                // the poll thread treat it as the baseline, not a press.
                RESYNC_DETECTOR.store(true, Ordering::SeqCst);
                // A new assignment may change what needs registering.
                post_main(WM_APP_REHOTKEY);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Assign a captured key to the capture window's current target
    /// (0 = keybind, 1 = repeat key, 2 = stop key), when the key is
    /// valid for that target. Returns whether it assigned.
    fn assign_capture_key(_hwnd: HWND, vk: u32) -> bool {
        match CAPTURE_FOR.load(Ordering::SeqCst) {
            1 => {
                if !is_valid_repeat_key(vk) {
                    return false;
                }
                REPEAT_VK.store(vk, Ordering::SeqCst);
            }
            2 => {
                if !is_valid_keybind(vk) {
                    return false;
                }
                STOP_VK.store(vk, Ordering::SeqCst);
            }
            _ => {
                if !is_valid_keybind(vk) {
                    return false;
                }
                KEYBIND_VK.store(vk, Ordering::SeqCst);
            }
        }
        save_config(current_config());
        true
    }

    fn open_capture_window(parent: HWND) {
        open_capture(parent, 0);
    }

    /// Open the key capture window for a target: 0 = the toggle
    /// keybind, 1 = the Key action's repeat key, 2 = the stop-only
    /// key. The capture mechanics (and the resync-on-close that
    /// keeps the assigning press from counting as a use) are
    /// identical for all three.
    fn open_capture(parent: HWND, target: u32) {
        if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
            return; // already open
        }
        CAPTURE_FOR.store(target, Ordering::SeqCst);
        // Stop any clicking in progress so stray clicks can't hit the dialog.
        lock_engine().clicking = false;
        refresh_indicator(parent);
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            static REGISTERED: AtomicBool = AtomicBool::new(false);
            if !REGISTERED.swap(true, Ordering::SeqCst) {
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(capture_wndproc),
                    hInstance: instance.into(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    hbrBackground: CreateSolidBrush(COLORREF(0x00181818)),
                    lpszClassName: w!("TrayClickCapture"),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            }
            let width = 360;
            let height = 150;
            let x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
            let y = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;
            let title = wide(match target {
                1 => "TrayClick — Set Repeat Key",
                2 => "TrayClick — Set Stop Key",
                _ => "TrayClick — Set Keybind",
            });
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("TrayClickCapture"),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                x,
                y,
                width,
                height,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(h) => h,
                Err(_) => {
                    DIALOG_OPEN.store(false, Ordering::SeqCst);
                    return;
                }
            };
            let current = key_name(match target {
                1 => REPEAT_VK.load(Ordering::SeqCst),
                2 => STOP_VK.load(Ordering::SeqCst),
                _ => KEYBIND_VK.load(Ordering::SeqCst),
            });
            let what = match target {
                1 => "the key repeated by the Key action",
                2 => "the stop-only key",
                _ => "the toggle keybind",
            };
            let hint = if target == 1 {
                ""
            } else {
                " A mouse side button works too."
            };
            let label = wide(&format!(
                "Press a key to set {what}.{hint}\r\n\r\nCurrent: {current}\r\nEsc to cancel."
            ));
            let _ = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR(label.as_ptr()),
                // WS_CHILD | WS_VISIBLE | SS_CENTER (0x0001 static style)
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0000_0001),
                16,
                24,
                width - 48,
                height - 80,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            );
            let _ = SetFocus(Some(hwnd));
        }
    }

    // ---------- Drag-point capture overlay ----------

    /// Which drag point the overlay is waiting for (0 = A, 1 = B), and
    /// its instruction label (so the text can change after point A).
    static DRAG_STEP: AtomicU32 = AtomicU32::new(0);
    static DRAG_LABEL: AtomicI32 = AtomicI32::new(0);

    unsafe extern "system" fn drag_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_LBUTTONDOWN => {
                unsafe {
                    let mut pt = POINT::default();
                    if GetCursorPos(&mut pt).is_ok() {
                        if DRAG_STEP.load(Ordering::SeqCst) == 0 {
                            DRAG_AX.store(pt.x, Ordering::SeqCst);
                            DRAG_AY.store(pt.y, Ordering::SeqCst);
                            DRAG_STEP.store(1, Ordering::SeqCst);
                            let label_raw = DRAG_LABEL.load(Ordering::SeqCst);
                            if label_raw != 0 {
                                let label = HWND(label_raw as isize as *mut c_void);
                                let t = wide(
                                    "Point A set. Now click point B — where the drag ends. Esc cancels.",
                                );
                                let _ = SetWindowTextW(label, PCWSTR(t.as_ptr()));
                            }
                        } else {
                            DRAG_BX.store(pt.x, Ordering::SeqCst);
                            DRAG_BY.store(pt.y, Ordering::SeqCst);
                            DRAG_SET.store(true, Ordering::SeqCst);
                            save_config(current_config());
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wparam.0 as u32 == VK_ESCAPE_CODE {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, COLORREF(0x00F2F2F2));
                SetBkMode(hdc, TRANSPARENT);
                LRESULT(dialog_bg_brush().0 as isize)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                DIALOG_OPEN.store(false, Ordering::SeqCst);
                RESYNC_DETECTOR.store(true, Ordering::SeqCst);
                DRAG_LABEL.store(0, Ordering::SeqCst);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Cover the virtual screen with a dark overlay and record the
    /// user's next two clicks as the Drag action's points A and B.
    /// Opened from the settings window (which closes first, so no
    /// dialog is up); Esc cancels without changing the points.
    fn open_drag_overlay(parent: HWND) {
        if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
            return; // a dialog is already open
        }
        DRAG_STEP.store(0, Ordering::SeqCst);
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            static REGISTERED: AtomicBool = AtomicBool::new(false);
            if !REGISTERED.swap(true, Ordering::SeqCst) {
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(drag_wndproc),
                    hInstance: instance.into(),
                    hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
                    hbrBackground: CreateSolidBrush(COLORREF(0x00181818)),
                    lpszClassName: w!("TrayClickDragPick"),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            }
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0x0000_0008), // WS_EX_TOPMOST
                w!("TrayClickDragPick"),
                w!("TrayClick — Set Drag Points"),
                WS_POPUP | WS_VISIBLE,
                vx,
                vy,
                vw,
                vh,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(h) => h,
                Err(_) => {
                    DIALOG_OPEN.store(false, Ordering::SeqCst);
                    return;
                }
            };
            let label = wide("Click point A — where the drag starts. Esc cancels.");
            if let Ok(lh) = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR(label.as_ptr()),
                WS_CHILD | WS_VISIBLE,
                32,
                32,
                900,
                30,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            ) {
                DRAG_LABEL.store(lh.0 as i32, Ordering::SeqCst);
            }
            let _ = SetFocus(Some(hwnd));
        }
    }

    // ---------- Crosshair point-pick overlay ----------

    /// What the pick overlay is capturing: 0 = one lock point,
    /// 1 = a multi-point sequence (up to 4 clicks).
    static PICK_MODE: AtomicU32 = AtomicU32::new(0);
    /// Points picked so far in this overlay session (sequence mode).
    static PICK_POINTS: OnceLock<Mutex<Vec<(i32, i32)>>> = OnceLock::new();
    static PICK_LABEL: AtomicI32 = AtomicI32::new(0);

    fn pick_points() -> &'static Mutex<Vec<(i32, i32)>> {
        PICK_POINTS.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// Commit the lock point just picked: store it, and remember the
    /// window it was picked on when window-relative is on, so later
    /// cycles can follow that window. Callers destroy the overlay
    /// first, so `WindowFromPoint` sees the real window under the
    /// point, not the overlay.
    fn commit_lock_point(x: i32, y: i32) {
        LOCK_X.store(x, Ordering::SeqCst);
        LOCK_Y.store(y, Ordering::SeqCst);
        LOCK_SET.store(true, Ordering::SeqCst);
        if LOCK_REL.load(Ordering::SeqCst) {
            unsafe {
                let pt = POINT { x, y };
                let hwnd = WindowFromPoint(pt);
                if !hwnd.is_invalid() {
                    let root = GetAncestor(hwnd, GET_ANCESTOR_FLAGS(2)); // GA_ROOT
                    if !root.is_invalid() {
                        let mut rc = RECT::default();
                        if GetWindowRect(root, &mut rc).is_ok() {
                            LOCK_HWND.store(root.0 as i32, Ordering::SeqCst);
                            LOCK_RX.store(x - rc.left, Ordering::SeqCst);
                            LOCK_RY.store(y - rc.top, Ordering::SeqCst);
                        }
                    }
                }
            }
        }
        save_config(current_config());
    }

    /// Commit the sequence points picked so far (at least one).
    fn commit_sequence() {
        let points = pick_points()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let n = points.len().min(4);
        for i in 0..n {
            SEQ_X[i].store(points[i].0, Ordering::SeqCst);
            SEQ_Y[i].store(points[i].1, Ordering::SeqCst);
        }
        SEQ_COUNT.store(n as u32, Ordering::SeqCst);
        save_config(current_config());
    }

    unsafe extern "system" fn pick_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_LBUTTONDOWN => {
                unsafe {
                    let mut pt = POINT::default();
                    if GetCursorPos(&mut pt).is_ok() {
                        if PICK_MODE.load(Ordering::SeqCst) == 0 {
                            // Destroy first: the window-relative
                            // commit below looks through the point,
                            // and it must not find this overlay.
                            let _ = DestroyWindow(hwnd);
                            commit_lock_point(pt.x, pt.y);
                        } else {
                            let mut points = pick_points()
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            if points.len() < 4 {
                                points.push((pt.x, pt.y));
                                let count = points.len();
                                let label_raw = PICK_LABEL.load(Ordering::SeqCst);
                                if label_raw != 0 {
                                    let label = HWND(label_raw as isize as *mut c_void);
                                    let t = wide(&format!(
                                        "Point {count} set. Click another (up to 4), or right-click to finish. Esc cancels."
                                    ));
                                    let _ = SetWindowTextW(label, PCWSTR(t.as_ptr()));
                                }
                                if count >= 4 {
                                    drop(points);
                                    commit_sequence();
                                    let _ = DestroyWindow(hwnd);
                                }
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                if PICK_MODE.load(Ordering::SeqCst) == 1 {
                    let has_points = !pick_points()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .is_empty();
                    if has_points {
                        commit_sequence();
                        unsafe {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wparam.0 as u32 == VK_ESCAPE_CODE {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, COLORREF(0x00F2F2F2));
                SetBkMode(hdc, TRANSPARENT);
                LRESULT(dialog_bg_brush().0 as isize)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                DIALOG_OPEN.store(false, Ordering::SeqCst);
                RESYNC_DETECTOR.store(true, Ordering::SeqCst);
                PICK_LABEL.store(0, Ordering::SeqCst);
                let mut points = pick_points()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                points.clear();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Cover the virtual screen with a dark crosshair overlay and
    /// record the user's clicks as the lock point (one click) or the
    /// multi-point sequence (up to four, right-click finishes early).
    /// Opened from the settings window (which closes first, so no
    /// dialog is up); Esc cancels without changing anything.
    fn open_pick_overlay(parent: HWND, for_sequence: bool) {
        if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
            return; // a dialog is already open
        }
        PICK_MODE.store(if for_sequence { 1 } else { 0 }, Ordering::SeqCst);
        pick_points()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            static REGISTERED: AtomicBool = AtomicBool::new(false);
            if !REGISTERED.swap(true, Ordering::SeqCst) {
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(pick_wndproc),
                    hInstance: instance.into(),
                    hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
                    hbrBackground: CreateSolidBrush(COLORREF(0x00181818)),
                    lpszClassName: w!("TrayClickPointPick"),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            }
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let title = if for_sequence {
                w!("TrayClick — Set Sequence Points")
            } else {
                w!("TrayClick — Pick Lock Point")
            };
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0x0000_0008), // WS_EX_TOPMOST
                w!("TrayClickPointPick"),
                title,
                WS_POPUP | WS_VISIBLE,
                vx,
                vy,
                vw,
                vh,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(h) => h,
                Err(_) => {
                    DIALOG_OPEN.store(false, Ordering::SeqCst);
                    return;
                }
            };
            let label = wide(if for_sequence {
                "Click point 1 of up to 4 — right-click to finish. Esc cancels."
            } else {
                "Click anywhere to set the lock point. Esc cancels."
            });
            if let Ok(lh) = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR(label.as_ptr()),
                WS_CHILD | WS_VISIBLE,
                32,
                32,
                900,
                30,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            ) {
                PICK_LABEL.store(lh.0 as i32, Ordering::SeqCst);
            }
            let _ = SetFocus(Some(hwnd));
        }
    }

    // ---------- Other-monitor quick lock ----------

    /// Collect this process's monitor rects (left, top, right, bottom).
    fn monitor_rects() -> Vec<(i32, i32, i32, i32)> {
        let mut rects: Vec<(i32, i32, i32, i32)> = Vec::new();
        unsafe extern "system" fn cb(
            _mon: HMONITOR,
            _hdc: HDC,
            rect: *mut RECT,
            data: LPARAM,
        ) -> windows::core::BOOL {
            if rect.is_null() {
                return windows::core::BOOL(1);
            }
            let rects = &mut *(data.0 as *mut Vec<(i32, i32, i32, i32)>);
            let r = &*rect;
            rects.push((r.left, r.top, r.right, r.bottom));
            windows::core::BOOL(1)
        }
        unsafe {
            let _ = EnumDisplayMonitors(
                None,
                None,
                Some(cb),
                LPARAM(&mut rects as *mut Vec<(i32, i32, i32, i32)> as isize),
            );
        }
        rects
    }

    /// Set the lock point to the centre of the first monitor that
    /// does not contain the current lock point (or the cursor, when
    /// no point is set), and switch the target to Locked. With one
    /// monitor there is nowhere to jump — that just shows a note.
    fn lock_other_monitor(hwnd: HWND) {
        let current = if LOCK_SET.load(Ordering::SeqCst) {
            (LOCK_X.load(Ordering::SeqCst), LOCK_Y.load(Ordering::SeqCst))
        } else {
            match cursor_pos() {
                Some(p) => p,
                None => return,
            }
        };
        match other_monitor_centre(&monitor_rects(), current) {
            Some((x, y)) => {
                LOCK_X.store(x, Ordering::SeqCst);
                LOCK_Y.store(y, Ordering::SeqCst);
                LOCK_SET.store(true, Ordering::SeqCst);
                TARGET_MODE.store(target_u32(TargetMode::Locked), Ordering::SeqCst);
                save_config(current_config());
                relabel_interval_controls(hwnd);
            }
            None => unsafe {
                let text =
                    wide("Only one monitor was found, so there is no other monitor to lock to.");
                let title = wide("TrayClick — Set Interval");
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_OK | MB_ICONINFORMATION,
                );
            },
        }
    }

    // ---------- Set Interval window ----------

    static INTERVAL_HWND: AtomicI32 = AtomicI32::new(0);

    unsafe extern "system" fn interval_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_COMMAND => {
                match wparam.0 & 0xFFFF {
                    ID_IV_1MS => {
                        apply_interval(1);
                        let _ = DestroyWindow(hwnd);
                    }
                    ID_IV_100MS => {
                        apply_interval(100);
                        let _ = DestroyWindow(hwnd);
                    }
                    ID_IV_APPLY => {
                        // Read the custom value from the edit control.
                        let mut applied = false;
                        if let Ok(edit) = GetDlgItem(Some(hwnd), ID_IV_EDIT as i32) {
                            let mut buf = [0u16; 16];
                            let n = GetWindowTextW(edit, &mut buf);
                            if n > 0 {
                                let text = String::from_utf16_lossy(&buf[..n as usize]);
                                if let Ok(ms) = text.trim().parse::<u32>() {
                                    if is_valid_interval(ms) {
                                        apply_interval(ms);
                                        applied = true;
                                    }
                                }
                            }
                        }
                        if applied {
                            let _ = DestroyWindow(hwnd);
                        } else {
                            let text = wide(&format!(
                                "Enter a whole number from {MIN_INTERVAL_MS} to {MAX_INTERVAL_MS} (milliseconds)."
                            ));
                            let title = wide("TrayClick — Set Interval");
                            MessageBoxW(
                                Some(hwnd),
                                PCWSTR(text.as_ptr()),
                                PCWSTR(title.as_ptr()),
                                MB_OK | MB_ICONWARNING,
                            );
                        }
                    }
                    ID_IV_BTN_LEFT => apply_button(hwnd, ClickButton::Left),
                    ID_IV_BTN_RIGHT => apply_button(hwnd, ClickButton::Right),
                    ID_IV_BTN_MIDDLE => apply_button(hwnd, ClickButton::Middle),
                    ID_IV_JITTER => apply_jitter(hwnd),
                    ID_IV_MODE => apply_mode(hwnd),
                    ID_IV_POS => apply_pos(hwnd),
                    ID_IV_LIMIT_SET => apply_limit(hwnd),
                    ID_IV_LIMIT_SEC_SET => apply_limit_seconds(hwnd),
                    ID_IV_TOP_KEYBIND => {
                        // Mirror the repeat-key capture: close this
                        // window first, then open the keybind capture
                        // from the main window.
                        let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
                        let _ = DestroyWindow(hwnd);
                        open_capture(parent, 0);
                    }
                    ID_IV_TOP_START => {
                        // Start the run and close: a run started under
                        // an open dialog would click the dialog itself.
                        let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
                        engine_start(parent);
                        let _ = DestroyWindow(hwnd);
                    }
                    ID_IV_TOP_STOP => {
                        let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
                        engine_stop(parent);
                        relabel_interval_controls(hwnd);
                        let _ = unsafe { SetFocus(Some(hwnd)) };
                    }
                    ID_IV_LAUNCH => apply_launch(hwnd),
                    ID_IV_STOPKEY => apply_stopkey(hwnd),
                    ID_IV_HOTKEY => apply_hotkey(hwnd),
                    ID_IV_DELAY => apply_delay(hwnd),
                    ID_IV_FS => apply_fs(hwnd),
                    ID_IV_FG => apply_fg(hwnd),
                    ID_IV_FG_PICK => apply_fg_pick(hwnd),
                    ID_IV_ACTION => apply_action(hwnd),
                    ID_IV_ALT => apply_alt(hwnd),
                    ID_IV_NUM_SET => apply_numbers(hwnd),
                    ID_IV_RANDOM => apply_random(hwnd),
                    ID_IV_RAMP => apply_ramp(hwnd),
                    ID_IV_TIMING_SET => apply_timing(hwnd),
                    ID_IV_REPEAT => {
                        // Capture windows open with no dialog up, so
                        // close this window first, then open the
                        // repeat-key capture from the main window.
                        let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
                        let _ = DestroyWindow(hwnd);
                        open_capture(parent, 1);
                    }
                    ID_IV_DRAG => {
                        let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
                        let _ = DestroyWindow(hwnd);
                        open_drag_overlay(parent);
                    }
                    ID_IV_PAGE_CLICK => show_page(hwnd, 0),
                    ID_IV_PAGE_ACTION => show_page(hwnd, 1),
                    ID_IV_PAGE_TARGET => show_page(hwnd, 2),
                    ID_IV_PICK_LOCK => {
                        let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
                        let _ = DestroyWindow(hwnd);
                        open_pick_overlay(parent, false);
                    }
                    ID_IV_SEQ_SET => {
                        let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();
                        let _ = DestroyWindow(hwnd);
                        open_pick_overlay(parent, true);
                    }
                    ID_IV_OTHER_MON => lock_other_monitor(hwnd),
                    ID_IV_WINREL => apply_winrel(hwnd),
                    ID_IV_REGION => apply_region(hwnd),
                    ID_IV_SEQ_CLEAR => apply_seq_clear(hwnd),
                    ID_IV_SLOT1_SAVE => apply_slot_save(hwnd, 0),
                    ID_IV_SLOT2_SAVE => apply_slot_save(hwnd, 1),
                    ID_IV_SLOT3_SAVE => apply_slot_save(hwnd, 2),
                    ID_IV_SLOT1_USE => apply_slot_use(hwnd, 0),
                    ID_IV_SLOT2_USE => apply_slot_use(hwnd, 1),
                    ID_IV_SLOT3_USE => apply_slot_use(hwnd, 2),
                    ID_IV_OFF_SET => apply_offset(hwnd),
                    ID_IV_PIXEL => apply_pixel(hwnd),
                    ID_IV_PIXEL_SAMPLE => apply_pixel_sample(hwnd),
                    ID_IV_TOL_SET => apply_tol(hwnd),
                    // IDCANCEL (2) arrives when Esc is pressed via the
                    // dialog message loop.
                    2 => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                // Charcoal theme: light text on the window's charcoal
                // background. Push buttons ignore the colours, but
                // group boxes take their caption and frame from them.
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, COLORREF(0x00F2F2F2));
                SetBkMode(hdc, TRANSPARENT);
                LRESULT(settings_bg_brush().0 as isize)
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, COLORREF(0x00F2F2F2));
                SetBkColor(hdc, COLORREF(0x00262626));
                LRESULT(edit_bg_brush().0 as isize)
            }
            WM_DRAWITEM => {
                let dis = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
                draw_run_button(dis);
                LRESULT(1)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                DIALOG_OPEN.store(false, Ordering::SeqCst);
                RESYNC_DETECTOR.store(true, Ordering::SeqCst);
                INTERVAL_HWND.store(0, Ordering::SeqCst);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// The caption of a mouse-button choice; the current one is ticked.
    fn button_caption(b: ClickButton, current: ClickButton) -> String {
        if b == current {
            format!("✓ {}", b.label())
        } else {
            b.label().to_string()
        }
    }

    fn jitter_caption() -> &'static str {
        if JITTER_ON.load(Ordering::SeqCst) {
            "Jitter: On"
        } else {
            "Jitter: Off"
        }
    }

    fn mode_caption() -> String {
        format!("Mode: {}", current_mode().label())
    }

    fn stopkey_caption() -> String {
        let vk = STOP_VK.load(Ordering::SeqCst);
        if vk == 0 {
            "Stop key: none".to_string()
        } else {
            format!("Stop key: {}", key_name(vk))
        }
    }

    fn hotkey_caption() -> String {
        if HOTKEY_MODE.load(Ordering::SeqCst) {
            "Hotkeys: On".to_string()
        } else {
            "Hotkeys: Off".to_string()
        }
    }

    /// The top bar's Start button caption: ticked while a run is on.
    fn top_start_caption() -> String {
        if lock_engine().is_clicking_now() {
            "Start ✓".to_string()
        } else {
            "Start".to_string()
        }
    }

    fn delay_caption() -> String {
        let end = *delay_end()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match end {
            Some(end) => {
                let secs = end.saturating_duration_since(Instant::now()).as_secs();
                format!("Starts in ~{} min (cancel)", (secs / 60).max(1))
            }
            None => "Start timer".to_string(),
        }
    }

    fn fs_caption() -> String {
        if FS_PAUSE.load(Ordering::SeqCst) {
            "Fullscreen pause: On".to_string()
        } else {
            "Fullscreen pause: Off".to_string()
        }
    }

    fn fg_caption() -> String {
        if FG_RULE.load(Ordering::SeqCst) {
            "App rule: On".to_string()
        } else {
            "App rule: Off".to_string()
        }
    }

    fn fg_label_caption() -> String {
        let exe = fg_exe()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if exe.is_empty() {
            "Foreground app: (none picked)".to_string()
        } else {
            format!("Foreground app: {exe}")
        }
    }

    fn pos_caption() -> String {
        format!("Target: {}", target_mode().label())
    }

    fn launch_caption() -> &'static str {
        if START_ON_LAUNCH.load(Ordering::SeqCst) {
            "Start on launch: On"
        } else {
            "Start on launch: Off"
        }
    }

    fn action_caption() -> String {
        format!("Action: {}", current_action().label())
    }

    fn alt_caption() -> &'static str {
        if ALTERNATE.load(Ordering::SeqCst) {
            "Alternate buttons: On"
        } else {
            "Alternate buttons: Off"
        }
    }

    fn repeat_caption() -> String {
        format!("Repeat key: {}", key_name(REPEAT_VK.load(Ordering::SeqCst)))
    }

    fn drag_caption() -> &'static str {
        if DRAG_SET.load(Ordering::SeqCst) {
            "Drag points set ✓"
        } else {
            "Set drag points"
        }
    }

    fn random_caption() -> &'static str {
        if RANDOM_ON.load(Ordering::SeqCst) {
            "Random: On"
        } else {
            "Random: Off"
        }
    }

    fn ramp_caption() -> &'static str {
        if RAMP_ON.load(Ordering::SeqCst) {
            "Ramp: On"
        } else {
            "Ramp: Off"
        }
    }

    /// The settings window's outer width on every page.
    const SETTINGS_WIDTH: i32 = 440;

    /// The settings window's outer height for a page (Clicking
    /// carries the trigger controls, Targeting the targeting rows).
    fn page_outer_height(page: u32) -> i32 {
        match page {
            0 => 690,
            2 => 610,
            _ => 428,
        }
    }

    fn page_caption(page: u32) -> String {
        let name = match page {
            0 => "Clicking",
            1 => "Action",
            _ => "Targeting",
        };
        if CURRENT_PAGE.load(Ordering::SeqCst) == page {
            format!("✓ {name}")
        } else {
            name.to_string()
        }
    }

    fn pick_caption() -> String {
        if LOCK_SET.load(Ordering::SeqCst) {
            format!(
                "Pick lock point ✓ ({}, {})",
                LOCK_X.load(Ordering::SeqCst),
                LOCK_Y.load(Ordering::SeqCst)
            )
        } else {
            "Pick lock point…".to_string()
        }
    }

    fn winrel_caption() -> &'static str {
        if LOCK_REL.load(Ordering::SeqCst) {
            "Window-relative: On"
        } else {
            "Window-relative: Off"
        }
    }

    fn region_caption() -> &'static str {
        if REGION_GUARD.load(Ordering::SeqCst) {
            "Region guard: On"
        } else {
            "Region guard: Off"
        }
    }

    fn seq_caption() -> String {
        let n = SEQ_COUNT.load(Ordering::SeqCst);
        if n > 0 {
            format!("Set sequence ✓ ({n} points)")
        } else {
            "Set sequence points".to_string()
        }
    }

    fn slot_coords_caption(i: usize) -> String {
        if SLOT_SET[i].load(Ordering::SeqCst) {
            format!(
                "{}, {}",
                SLOT_X[i].load(Ordering::SeqCst),
                SLOT_Y[i].load(Ordering::SeqCst)
            )
        } else {
            "— empty —".to_string()
        }
    }

    fn pixel_caption() -> &'static str {
        if PIXEL_CHECK.load(Ordering::SeqCst) {
            "Pixel check: On"
        } else {
            "Pixel check: Off"
        }
    }

    fn sample_caption() -> String {
        if PIXEL_SET.load(Ordering::SeqCst) {
            format!("Sampled #{:06X}", PIXEL_RGB.load(Ordering::SeqCst))
        } else {
            "Sample colour".to_string()
        }
    }

    /// Repaint the button-row and jitter captions after a change.
    fn relabel_interval_controls(hwnd: HWND) {
        unsafe {
            for (id, b) in [
                (ID_IV_BTN_LEFT, ClickButton::Left),
                (ID_IV_BTN_RIGHT, ClickButton::Right),
                (ID_IV_BTN_MIDDLE, ClickButton::Middle),
            ] {
                if let Ok(btn) = GetDlgItem(Some(hwnd), id as i32) {
                    let t = wide(&button_caption(b, current_button()));
                    let _ = SetWindowTextW(btn, PCWSTR(t.as_ptr()));
                }
            }
            if let Ok(jb) = GetDlgItem(Some(hwnd), ID_IV_JITTER as i32) {
                let t = wide(jitter_caption());
                let _ = SetWindowTextW(jb, PCWSTR(t.as_ptr()));
            }
            if let Ok(mb) = GetDlgItem(Some(hwnd), ID_IV_MODE as i32) {
                let t = wide(&mode_caption());
                let _ = SetWindowTextW(mb, PCWSTR(t.as_ptr()));
            }
            if let Ok(pb) = GetDlgItem(Some(hwnd), ID_IV_POS as i32) {
                let t = wide(&pos_caption());
                let _ = SetWindowTextW(pb, PCWSTR(t.as_ptr()));
            }
            if let Ok(lb) = GetDlgItem(Some(hwnd), ID_IV_LAUNCH as i32) {
                let t = wide(launch_caption());
                let _ = SetWindowTextW(lb, PCWSTR(t.as_ptr()));
            }
            if let Ok(sb) = GetDlgItem(Some(hwnd), ID_IV_STOPKEY as i32) {
                let t = wide(&stopkey_caption());
                let _ = SetWindowTextW(sb, PCWSTR(t.as_ptr()));
            }
            if let Ok(hb) = GetDlgItem(Some(hwnd), ID_IV_HOTKEY as i32) {
                let t = wide(&hotkey_caption());
                let _ = SetWindowTextW(hb, PCWSTR(t.as_ptr()));
            }
            if let Ok(db) = GetDlgItem(Some(hwnd), ID_IV_DELAY as i32) {
                let t = wide(&delay_caption());
                let _ = SetWindowTextW(db, PCWSTR(t.as_ptr()));
            }
            if let Ok(fb) = GetDlgItem(Some(hwnd), ID_IV_FS as i32) {
                let t = wide(&fs_caption());
                let _ = SetWindowTextW(fb, PCWSTR(t.as_ptr()));
            }
            if let Ok(fb) = GetDlgItem(Some(hwnd), ID_IV_FG as i32) {
                let t = wide(&fg_caption());
                let _ = SetWindowTextW(fb, PCWSTR(t.as_ptr()));
            }
            if let Ok(fl) = GetDlgItem(Some(hwnd), ID_IV_FG_LABEL as i32) {
                let t = wide(&fg_label_caption());
                let _ = SetWindowTextW(fl, PCWSTR(t.as_ptr()));
            }
            if let Ok(ab) = GetDlgItem(Some(hwnd), ID_IV_ACTION as i32) {
                let t = wide(&action_caption());
                let _ = SetWindowTextW(ab, PCWSTR(t.as_ptr()));
            }
            if let Ok(tb) = GetDlgItem(Some(hwnd), ID_IV_ALT as i32) {
                let t = wide(alt_caption());
                let _ = SetWindowTextW(tb, PCWSTR(t.as_ptr()));
            }
            if let Ok(rb) = GetDlgItem(Some(hwnd), ID_IV_REPEAT as i32) {
                let t = wide(&repeat_caption());
                let _ = SetWindowTextW(rb, PCWSTR(t.as_ptr()));
            }
            if let Ok(db) = GetDlgItem(Some(hwnd), ID_IV_DRAG as i32) {
                let t = wide(drag_caption());
                let _ = SetWindowTextW(db, PCWSTR(t.as_ptr()));
            }
            if let Ok(rb) = GetDlgItem(Some(hwnd), ID_IV_RANDOM as i32) {
                let t = wide(random_caption());
                let _ = SetWindowTextW(rb, PCWSTR(t.as_ptr()));
            }
            if let Ok(rb) = GetDlgItem(Some(hwnd), ID_IV_RAMP as i32) {
                let t = wide(ramp_caption());
                let _ = SetWindowTextW(rb, PCWSTR(t.as_ptr()));
            }
            for (id, page) in [
                (ID_IV_PAGE_CLICK, 0),
                (ID_IV_PAGE_ACTION, 1),
                (ID_IV_PAGE_TARGET, 2),
            ] {
                if let Ok(pb) = GetDlgItem(Some(hwnd), id as i32) {
                    let t = wide(&page_caption(page));
                    let _ = SetWindowTextW(pb, PCWSTR(t.as_ptr()));
                }
            }
            if let Ok(kn) = GetDlgItem(Some(hwnd), ID_IV_TOP_KEYNAME as i32) {
                let t = wide(&key_name(KEYBIND_VK.load(Ordering::SeqCst)));
                let _ = SetWindowTextW(kn, PCWSTR(t.as_ptr()));
            }
            if let Ok(sb) = GetDlgItem(Some(hwnd), ID_IV_TOP_START as i32) {
                let t = wide(&top_start_caption());
                let _ = SetWindowTextW(sb, PCWSTR(t.as_ptr()));
                let _ = InvalidateRect(Some(sb), None, true);
            }
            if let Ok(sb) = GetDlgItem(Some(hwnd), ID_IV_TOP_STOP as i32) {
                let _ = InvalidateRect(Some(sb), None, true);
            }
            if let Ok(pb) = GetDlgItem(Some(hwnd), ID_IV_PICK_LOCK as i32) {
                let t = wide(&pick_caption());
                let _ = SetWindowTextW(pb, PCWSTR(t.as_ptr()));
            }
            if let Ok(wb) = GetDlgItem(Some(hwnd), ID_IV_WINREL as i32) {
                let t = wide(winrel_caption());
                let _ = SetWindowTextW(wb, PCWSTR(t.as_ptr()));
            }
            if let Ok(gb) = GetDlgItem(Some(hwnd), ID_IV_REGION as i32) {
                let t = wide(region_caption());
                let _ = SetWindowTextW(gb, PCWSTR(t.as_ptr()));
            }
            if let Ok(sb) = GetDlgItem(Some(hwnd), ID_IV_SEQ_SET as i32) {
                let t = wide(&seq_caption());
                let _ = SetWindowTextW(sb, PCWSTR(t.as_ptr()));
            }
            for (id, i) in [
                (ID_IV_SLOT1_COORDS, 0),
                (ID_IV_SLOT2_COORDS, 1),
                (ID_IV_SLOT3_COORDS, 2),
            ] {
                if let Ok(cb) = GetDlgItem(Some(hwnd), id as i32) {
                    let t = wide(&slot_coords_caption(i));
                    let _ = SetWindowTextW(cb, PCWSTR(t.as_ptr()));
                }
            }
            if let Ok(xb) = GetDlgItem(Some(hwnd), ID_IV_PIXEL as i32) {
                let t = wide(pixel_caption());
                let _ = SetWindowTextW(xb, PCWSTR(t.as_ptr()));
            }
            if let Ok(sb) = GetDlgItem(Some(hwnd), ID_IV_PIXEL_SAMPLE as i32) {
                let t = wide(&sample_caption());
                let _ = SetWindowTextW(sb, PCWSTR(t.as_ptr()));
            }
        }
    }

    /// Apply a mouse-button choice: persists at once, the window stays
    /// open (unlike the interval presets, which close it).
    fn apply_button(hwnd: HWND, b: ClickButton) {
        BUTTON.store(button_u32(b), Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_jitter(hwnd: HWND) {
        let on = !JITTER_ON.load(Ordering::SeqCst);
        JITTER_ON.store(on, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_mode(hwnd: HWND) {
        let next = match current_mode() {
            ClickMode::Click => ClickMode::Hold,
            ClickMode::Hold => ClickMode::Momentary,
            ClickMode::Momentary => ClickMode::Click,
        };
        MODE.store(mode_u32(next), Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    /// Cycle the target mode: Follow → Locked → Sequence → Follow.
    fn apply_pos(hwnd: HWND) {
        let next = match target_mode() {
            TargetMode::Follow => TargetMode::Locked,
            TargetMode::Locked => TargetMode::Sequence,
            TargetMode::Sequence => TargetMode::Follow,
        };
        TARGET_MODE.store(target_u32(next), Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_launch(hwnd: HWND) {
        let on = !START_ON_LAUNCH.load(Ordering::SeqCst);
        START_ON_LAUNCH.store(on, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    /// Stop key: capture a key (or mouse side button) for it, using
    /// the same capture window as the keybind.
    fn apply_stopkey(hwnd: HWND) {
        unsafe {
            let parent = GetParent(hwnd).unwrap_or(hwnd);
            let _ = DestroyWindow(hwnd);
            open_capture(parent, 2);
        }
    }

    fn apply_hotkey(hwnd: HWND) {
        HOTKEY_MODE.fetch_not(Ordering::SeqCst);
        save_config(current_config());
        if let Some(main) = main_hwnd() {
            register_hotkeys(main);
        }
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    /// Delayed start: arm the one-shot timer from the minutes edit,
    /// or cancel it when one is already counting down.
    fn apply_delay(hwnd: HWND) {
        let armed = delay_end()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some();
        if armed {
            cancel_delay();
            relabel_interval_controls(hwnd);
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
            return;
        }
        let parsed = read_edit_u32(hwnd, ID_IV_DELAY_EDIT);
        match parsed {
            Some(mins) if is_valid_delay_minutes(mins) => {
                *delay_end()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some(Instant::now() + Duration::from_secs(u64::from(mins) * 60));
                if let Some(main) = main_hwnd() {
                    unsafe {
                        let _ = SetTimer(
                            Some(main),
                            ID_DELAY_TIMER,
                            mins.saturating_mul(60_000),
                            None,
                        );
                    }
                }
                relabel_interval_controls(hwnd);
                unsafe {
                    let _ = SetFocus(Some(hwnd));
                }
            }
            _ => {
                warn(
                    hwnd,
                    "Enter a delay of 1 to 1440 minutes (whole numbers only).",
                );
                unsafe {
                    let _ = SetFocus(Some(hwnd));
                }
            }
        }
    }

    fn apply_fs(hwnd: HWND) {
        FS_PAUSE.fetch_not(Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_fg(hwnd: HWND) {
        FG_RULE.fetch_not(Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    /// Pick the foreground-rule app from the running windows.
    fn apply_fg_pick(hwnd: HWND) {
        unsafe {
            let parent = GetParent(hwnd).unwrap_or(hwnd);
            let _ = DestroyWindow(hwnd);
            open_app_picker(parent);
        }
    }

    fn apply_action(hwnd: HWND) {
        let next = match current_action() {
            ClickAction::Single => ClickAction::Double,
            ClickAction::Double => ClickAction::Burst,
            ClickAction::Burst => ClickAction::Scroll,
            ClickAction::Scroll => ClickAction::Key,
            ClickAction::Key => ClickAction::Drag,
            ClickAction::Drag => ClickAction::Single,
        };
        ACTION.store(action_u32(next), Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_alt(hwnd: HWND) {
        let on = !ALTERNATE.load(Ordering::SeqCst);
        ALTERNATE.store(on, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    /// Read one numeric edit control in the settings window.
    fn read_edit_u32(hwnd: HWND, id: usize) -> Option<u32> {
        let edit = unsafe { GetDlgItem(Some(hwnd), id as i32) }.ok()?;
        let mut buf = [0u16; 16];
        let n = unsafe { GetWindowTextW(edit, &mut buf) };
        if n <= 0 {
            return None;
        }
        String::from_utf16_lossy(&buf[..n as usize])
            .trim()
            .parse::<u32>()
            .ok()
    }

    /// Read one signed numeric edit control (for the scroll amount,
    /// which may be negative).
    fn read_edit_i32(hwnd: HWND, id: usize) -> Option<i32> {
        let edit = unsafe { GetDlgItem(Some(hwnd), id as i32) }.ok()?;
        let mut buf = [0u16; 16];
        let n = unsafe { GetWindowTextW(edit, &mut buf) };
        if n <= 0 {
            return None;
        }
        String::from_utf16_lossy(&buf[..n as usize])
            .trim()
            .parse::<i32>()
            .ok()
    }

    /// Apply the four numeric action fields (burst clicks, burst gap,
    /// button-down time, scroll notches) at once; all must be valid,
    /// or nothing changes and a warning explains the ranges.
    fn apply_numbers(hwnd: HWND) {
        let burst = read_edit_u32(hwnd, ID_IV_BURST_EDIT);
        let gap = read_edit_u32(hwnd, ID_IV_GAP_EDIT);
        let press = read_edit_u32(hwnd, ID_IV_PRESS_EDIT);
        let scroll = read_edit_i32(hwnd, ID_IV_SCROLL_EDIT);
        let valid = matches!(burst, Some(n) if is_valid_burst_clicks(n))
            && matches!(gap, Some(n) if is_valid_burst_gap(n))
            && matches!(press, Some(n) if is_valid_press_ms(n))
            && matches!(scroll, Some(n) if is_valid_scroll_amount(n));
        if valid {
            BURST_CLICKS.store(burst.unwrap_or_default(), Ordering::SeqCst);
            BURST_GAP_MS.store(gap.unwrap_or_default(), Ordering::SeqCst);
            PRESS_MS.store(press.unwrap_or_default(), Ordering::SeqCst);
            SCROLL_AMT.store(scroll.unwrap_or_default(), Ordering::SeqCst);
            save_config(current_config());
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        } else {
            let text = wide(
                "Burst: 1 to 100 clicks. Gap: 1 to 1000 ms. Down: 0 to 1000 ms (0 = instant). Scroll: -10 to 10 notches, not 0.",
            );
            let title = wide("TrayClick — Set Interval");
            unsafe {
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }

    fn apply_random(hwnd: HWND) {
        let on = !RANDOM_ON.load(Ordering::SeqCst);
        RANDOM_ON.store(on, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_ramp(hwnd: HWND) {
        let on = !RAMP_ON.load(Ordering::SeqCst);
        RAMP_ON.store(on, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    /// Apply the timing fields (random-range max, ramp target, ramp
    /// duration) at once; all must be valid, or nothing changes.
    fn apply_timing(hwnd: HWND) {
        let max = read_edit_u32(hwnd, ID_IV_MAX_EDIT);
        let to = read_edit_u32(hwnd, ID_IV_RAMP_TO_EDIT);
        let secs = read_edit_u32(hwnd, ID_IV_RAMP_SECS_EDIT);
        let valid = matches!(max, Some(n) if n == 0 || is_valid_interval(n))
            && matches!(to, Some(n) if n == 0 || is_valid_interval(n))
            && matches!(secs, Some(n) if is_valid_ramp_seconds(n));
        if valid {
            INTERVAL_MAX_MS.store(max.unwrap_or_default(), Ordering::SeqCst);
            RAMP_TO_MS.store(to.unwrap_or_default(), Ordering::SeqCst);
            RAMP_SECONDS.store(secs.unwrap_or_default(), Ordering::SeqCst);
            save_config(current_config());
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        } else {
            let text = wide(
                "Max and Ramp-to: 0 (unset) or 1 to 60000 ms. Ramp duration: 1 to 86400 seconds.",
            );
            let title = wide("TrayClick — Set Interval");
            unsafe {
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }

    /// Set the auto-stop limit from the edit control (0 = no limit).
    /// Like the other toggles this applies at once and keeps the
    /// window open; only a bad value interrupts, with a warning.
    fn apply_limit(hwnd: HWND) {
        let mut ok = false;
        if let Ok(edit) = unsafe { GetDlgItem(Some(hwnd), ID_IV_LIMIT_EDIT as i32) } {
            let mut buf = [0u16; 16];
            let n = unsafe { GetWindowTextW(edit, &mut buf) };
            if n > 0 {
                let text = String::from_utf16_lossy(&buf[..n as usize]);
                if let Ok(v) = text.trim().parse::<u32>() {
                    if is_valid_limit(v) {
                        LIMIT_CLICKS.store(v, Ordering::SeqCst);
                        save_config(current_config());
                        ok = true;
                    }
                }
            }
        }
        if ok {
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        } else {
            let text = wide("Enter a whole number of clicks from 0 (no limit) to 100000000.");
            let title = wide("TrayClick — Set Interval");
            unsafe {
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }

    /// Set the auto-stop time limit from the edit control (0 = no
    /// limit). Mirrors `apply_limit`: applies at once and keeps the
    /// window open; only a bad value interrupts, with a warning.
    fn apply_limit_seconds(hwnd: HWND) {
        let mut ok = false;
        if let Ok(edit) = unsafe { GetDlgItem(Some(hwnd), ID_IV_LIMIT_SEC_EDIT as i32) } {
            let mut buf = [0u16; 16];
            let n = unsafe { GetWindowTextW(edit, &mut buf) };
            if n > 0 {
                let text = String::from_utf16_lossy(&buf[..n as usize]);
                if let Ok(v) = text.trim().parse::<u32>() {
                    if is_valid_limit_seconds(v) {
                        LIMIT_SECONDS.store(v, Ordering::SeqCst);
                        save_config(current_config());
                        ok = true;
                    }
                }
            }
        }
        if ok {
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        } else {
            let text = wide("Enter a whole number of seconds from 0 (no limit) to 604800.");
            let title = wide("TrayClick — Set Interval");
            unsafe {
                MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }

    /// Owner-draw the top bar's Start/Stop buttons: Start is a red
    /// fill with a brighter red border and white text; Stop is the
    /// charcoal background with a red border and red text.
    fn draw_run_button(dis: &DRAWITEMSTRUCT) {
        let start = dis.CtlID as usize == ID_IV_TOP_START;
        unsafe {
            let hdc = dis.hDC;
            let rc = dis.rcItem;
            if start {
                FillRect(hdc, &rc, start_fill_brush());
            } else {
                FillRect(hdc, &rc, settings_bg_brush());
            }
            FrameRect(hdc, &rc, accent_brush());
            let mut buf = [0u16; 32];
            let n = GetWindowTextW(dis.hwndItem, &mut buf);
            if n > 0 {
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(
                    hdc,
                    if start {
                        COLORREF(0x00FF_FFFF)
                    } else {
                        COLORREF(0x004D_48E5)
                    },
                );
                let mut text_rc = rc;
                DrawTextW(
                    hdc,
                    &mut buf[..n as usize],
                    &mut text_rc,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
    }

    /// Read a free-text edit control (slot names allow letters and
    /// spaces, so the numeric readers do not apply).
    fn read_edit_text(hwnd: HWND, id: usize) -> String {
        let Ok(edit) = (unsafe { GetDlgItem(Some(hwnd), id as i32) }) else {
            return String::new();
        };
        let mut buf = [0u16; 40];
        let n = unsafe { GetWindowTextW(edit, &mut buf) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn warn(hwnd: HWND, msg: &str) {
        let text = wide(msg);
        let title = wide("TrayClick — Set Interval");
        unsafe {
            MessageBoxW(
                Some(hwnd),
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONWARNING,
            );
        }
    }

    fn apply_winrel(hwnd: HWND) {
        let on = !LOCK_REL.load(Ordering::SeqCst);
        LOCK_REL.store(on, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_region(hwnd: HWND) {
        let on = !REGION_GUARD.load(Ordering::SeqCst);
        REGION_GUARD.store(on, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_pixel(hwnd: HWND) {
        let on = !PIXEL_CHECK.load(Ordering::SeqCst);
        PIXEL_CHECK.store(on, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    /// Sample the colour the pixel check watches: the pixel at the
    /// lock point when one is resolved, else at the cursor. The
    /// settings window is up when this runs, so in practice the
    /// cursor is over this window — pick a lock point first, then
    /// sample (the sample reads the lock point, under the window).
    fn apply_pixel_sample(hwnd: HWND) {
        let (x, y) = if LOCK_SET.load(Ordering::SeqCst) || target_mode() != TargetMode::Follow {
            locked_point()
        } else {
            cursor_pos().unwrap_or((0, 0))
        };
        match sample_pixel(x, y) {
            Some(rgb) => {
                PIXEL_RGB.store(rgb, Ordering::SeqCst);
                PIXEL_SET.store(true, Ordering::SeqCst);
                save_config(current_config());
                relabel_interval_controls(hwnd);
                unsafe {
                    let _ = SetFocus(Some(hwnd));
                }
            }
            None => warn(hwnd, "Could not read a pixel at that point."),
        }
    }

    fn apply_tol(hwnd: HWND) {
        match read_edit_u32(hwnd, ID_IV_TOL_EDIT) {
            Some(n) if is_valid_pixel_tol(n) => {
                PIXEL_TOL.store(n, Ordering::SeqCst);
                save_config(current_config());
                unsafe {
                    let _ = SetFocus(Some(hwnd));
                }
            }
            _ => warn(hwnd, "Tolerance: a whole number from 0 to 64."),
        }
    }

    fn apply_offset(hwnd: HWND) {
        let x = read_edit_i32(hwnd, ID_IV_OFF_X_EDIT);
        let y = read_edit_i32(hwnd, ID_IV_OFF_Y_EDIT);
        let valid = matches!(x, Some(v) if is_valid_offset(v))
            && matches!(y, Some(v) if is_valid_offset(v));
        if valid {
            OFFSET_X.store(x.unwrap_or_default(), Ordering::SeqCst);
            OFFSET_Y.store(y.unwrap_or_default(), Ordering::SeqCst);
            save_config(current_config());
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        } else {
            warn(hwnd, "Offset: whole numbers from -200 to 200 on each axis.");
        }
    }

    fn apply_seq_clear(hwnd: HWND) {
        SEQ_COUNT.store(0, Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    const SLOT_NAME_IDS: [usize; 3] = [ID_IV_SLOT1_NAME, ID_IV_SLOT2_NAME, ID_IV_SLOT3_NAME];

    /// Save a slot: the current lock point when one is set, else the
    /// cursor position, under the (sanitized) name in its edit box.
    fn apply_slot_save(hwnd: HWND, i: usize) {
        let point = if LOCK_SET.load(Ordering::SeqCst) {
            Some((LOCK_X.load(Ordering::SeqCst), LOCK_Y.load(Ordering::SeqCst)))
        } else {
            cursor_pos()
        };
        let Some((x, y)) = point else { return };
        let name = sanitize_slot_name(&read_edit_text(hwnd, SLOT_NAME_IDS[i]));
        SLOT_X[i].store(x, Ordering::SeqCst);
        SLOT_Y[i].store(y, Ordering::SeqCst);
        SLOT_SET[i].store(true, Ordering::SeqCst);
        {
            let mut names = slot_names()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            names[i] = name.clone();
        }
        save_config(current_config());
        // Show the sanitized name back, so the box always matches
        // what is stored.
        unsafe {
            if let Ok(edit) = GetDlgItem(Some(hwnd), SLOT_NAME_IDS[i] as i32) {
                let t = wide(&name);
                let _ = SetWindowTextW(edit, PCWSTR(t.as_ptr()));
            }
            let _ = SetFocus(Some(hwnd));
        }
        relabel_interval_controls(hwnd);
    }

    /// Use a slot: it becomes the lock point and the target switches
    /// to Locked. An empty slot just explains itself.
    fn apply_slot_use(hwnd: HWND, i: usize) {
        if !SLOT_SET[i].load(Ordering::SeqCst) {
            warn(
                hwnd,
                "That slot is empty. Save the lock point (or the cursor position) into it first.",
            );
            return;
        }
        LOCK_X.store(SLOT_X[i].load(Ordering::SeqCst), Ordering::SeqCst);
        LOCK_Y.store(SLOT_Y[i].load(Ordering::SeqCst), Ordering::SeqCst);
        LOCK_SET.store(true, Ordering::SeqCst);
        LOCK_REL.store(false, Ordering::SeqCst);
        LOCK_HWND.store(0, Ordering::SeqCst);
        TARGET_MODE.store(target_u32(TargetMode::Locked), Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    /// Show one settings page: hide the other pages' controls, size
    /// the window for the page, and park the footer text and the
    /// page buttons at the bottom. The window remembers its page
    /// across opens (item 49, partial — remembering the window
    /// position itself lands in slice 7).
    fn show_page(hwnd: HWND, page: u32) {
        CURRENT_PAGE.store(page, Ordering::SeqCst);
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                SETTINGS_WIDTH,
                page_outer_height(page),
                SWP_NOMOVE | SWP_NOZORDER,
            );
            {
                let controls = page_controls()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                for (raw, p) in controls.iter() {
                    let h = HWND(*raw as isize as *mut c_void);
                    let _ = ShowWindow(h, if *p == page { SW_SHOW } else { SW_HIDE });
                }
            }
            let mut rc = RECT::default();
            if GetClientRect(hwnd, &mut rc).is_ok() {
                if let Ok(footer) = GetDlgItem(Some(hwnd), ID_IV_FOOTER as i32) {
                    let _ = SetWindowPos(footer, None, 16, rc.bottom - 70, 392, 28, SWP_NOZORDER);
                }
                // The page buttons form a footer row (0.11.0; they
                // were tabs at the top before).
                for (id, x, w) in [
                    (ID_IV_PAGE_CLICK, 16, 120),
                    (ID_IV_PAGE_ACTION, 142, 120),
                    (ID_IV_PAGE_TARGET, 268, 140),
                ] {
                    if let Ok(btn) = GetDlgItem(Some(hwnd), id as i32) {
                        let _ = SetWindowPos(btn, None, x, rc.bottom - 34, w, 26, SWP_NOZORDER);
                    }
                }
            }
        }
        relabel_interval_controls(hwnd);
    }

    // ---------- App picker (foreground-rule target) ----------

    /// EnumWindows callback: collect (title, exe) for visible,
    /// titled windows belonging to other processes.
    unsafe extern "system" fn app_pick_enum(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let out = &mut *(lparam.0 as *mut Vec<(String, String)>);
        let keep = windows::core::BOOL(1);
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return keep;
        }
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len <= 0 {
            return keep;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 || pid == std::process::id() {
            return keep;
        }
        let mut tbuf = vec![0u16; len as usize + 1];
        let n = unsafe { GetWindowTextW(hwnd, &mut tbuf) };
        if n <= 0 {
            return keep;
        }
        let title = String::from_utf16_lossy(&tbuf[..n as usize]);
        if title == "Program Manager" {
            return keep;
        }
        if let Some(exe) = process_exe_name(pid) {
            out.push((title, exe));
        }
        keep
    }

    unsafe extern "system" fn app_pick_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        const LB_GETCURSEL: usize = 0x0188;
        const LBN_DBLCLK: usize = 2;
        match msg {
            WM_COMMAND => {
                let id = wparam.0 & 0xFFFF;
                let code = (wparam.0 >> 16) & 0xFFFF;
                if id == ID_AP_SELECT || (id == ID_AP_LIST && code == LBN_DBLCLK) {
                    let list = GetDlgItem(Some(hwnd), ID_AP_LIST as i32).unwrap_or_default();
                    let idx =
                        SendMessageW(list, LB_GETCURSEL as u32, Some(WPARAM(0)), Some(LPARAM(0))).0;
                    if idx >= 0 {
                        let exes = app_pick_exes()
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .clone();
                        if let Some(exe) = exes.get(idx as usize) {
                            *fg_exe()
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner()) = exe.clone();
                            FG_RULE.store(true, Ordering::SeqCst);
                            save_config(current_config());
                        }
                    }
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, COLORREF(0x00F2F2F2));
                SetBkMode(hdc, TRANSPARENT);
                LRESULT(dialog_bg_brush().0 as isize)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                DIALOG_OPEN.store(false, Ordering::SeqCst);
                RESYNC_DETECTOR.store(true, Ordering::SeqCst);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// List the running apps' windows; picking one sets it as the
    /// foreground-rule app (and turns the rule on).
    fn open_app_picker(parent: HWND) {
        if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
            return; // already open
        }
        // Stop any clicking in progress so stray clicks can't hit it.
        lock_engine().clicking = false;
        refresh_indicator(parent);
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            static REGISTERED: AtomicBool = AtomicBool::new(false);
            if !REGISTERED.swap(true, Ordering::SeqCst) {
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(app_pick_wndproc),
                    hInstance: instance.into(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    hbrBackground: CreateSolidBrush(COLORREF(0x00181818)),
                    lpszClassName: w!("TrayClickAppPick"),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            }
            let mut found: Vec<(String, String)> = Vec::new();
            let _ = EnumWindows(
                Some(app_pick_enum),
                LPARAM(&mut found as *mut Vec<(String, String)> as isize),
            );
            found.sort_by(|a, b| {
                a.1.to_lowercase()
                    .cmp(&b.1.to_lowercase())
                    .then_with(|| a.0.cmp(&b.0))
            });
            found.dedup();
            let width = 480;
            let height = 400;
            let x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
            let y = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("TrayClickAppPick"),
                w!("TrayClick — Pick Foreground App"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                x,
                y,
                width,
                height,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(h) => h,
                Err(_) => {
                    DIALOG_OPEN.store(false, Ordering::SeqCst);
                    return;
                }
            };
            let label = wide("Pick the app the clicks should follow:");
            let _ = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR(label.as_ptr()),
                WS_CHILD | WS_VISIBLE,
                12,
                10,
                440,
                20,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            );
            // LISTBOX with LBS_NOTIFY (0x0001) + border + vscroll.
            let list = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("LISTBOX"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | WINDOW_STYLE(0x0000_0001),
                12,
                36,
                440,
                288,
                Some(hwnd),
                Some(HMENU(ID_AP_LIST as *mut c_void)),
                Some(instance.into()),
                None,
            );
            const LB_ADDSTRING: usize = 0x0180;
            let mut exes: Vec<String> = Vec::new();
            if let Ok(list) = list {
                for (title, exe) in &found {
                    let row = wide(&format!("{title}  —  {exe}"));
                    let _ = SendMessageW(
                        list,
                        LB_ADDSTRING as u32,
                        Some(WPARAM(0)),
                        Some(LPARAM(row.as_ptr() as isize)),
                    );
                    exes.push(exe.clone());
                }
            }
            *app_pick_exes()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = exes;
            let _ = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("Choose"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                332,
                330,
                120,
                30,
                Some(hwnd),
                Some(HMENU(ID_AP_SELECT as *mut c_void)),
                Some(instance.into()),
                None,
            );
        }
    }

    fn open_interval_window(parent: HWND) {
        if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
            return; // a dialog is already open
        }
        // Stop any clicking in progress so stray clicks can't hit the dialog.
        lock_engine().clicking = false;
        refresh_indicator(parent);
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            static REGISTERED: AtomicBool = AtomicBool::new(false);
            if !REGISTERED.swap(true, Ordering::SeqCst) {
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(interval_wndproc),
                    hInstance: instance.into(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    hbrBackground: CreateSolidBrush(COLORREF(0x001B_1B1B)),
                    lpszClassName: w!("TrayClickInterval"),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            }
            let width = SETTINGS_WIDTH;
            let height = page_outer_height(CURRENT_PAGE.load(Ordering::SeqCst));
            let x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
            let y = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("TrayClickInterval"),
                w!("TrayClick — Set Interval"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                x,
                y,
                width,
                height,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(h) => h,
                Err(_) => {
                    DIALOG_OPEN.store(false, Ordering::SeqCst);
                    return;
                }
            };
            INTERVAL_HWND.store(hwnd.0 as i32, Ordering::SeqCst);
            // Dark title bar to match the charcoal client area
            // (Windows 10 1903+; ignored where unsupported).
            let dark: i32 = 1;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );

            page_controls()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
            let child = |class: PCWSTR,
                         text: &str,
                         style: WINDOW_STYLE,
                         cx: i32,
                         cy: i32,
                         cw: i32,
                         ch: i32,
                         id: usize|
             -> windows::core::Result<HWND> {
                let t = wide(text);
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    PCWSTR(t.as_ptr()),
                    style,
                    cx,
                    cy,
                    cw,
                    ch,
                    Some(hwnd),
                    Some(HMENU(id as *mut c_void)),
                    Some(instance.into()),
                    None,
                )
            };
            // A child that belongs to one settings page: registered
            // so `show_page` can show/hide it with its page.
            let child_p = |page: u32,
                           class: PCWSTR,
                           text: &str,
                           style: WINDOW_STYLE,
                           cx: i32,
                           cy: i32,
                           cw: i32,
                           ch: i32,
                           id: usize|
             -> windows::core::Result<HWND> {
                let r = child(class, text, style, cx, cy, cw, ch, id);
                if let Ok(h) = &r {
                    page_controls()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push((h.0 as i32, page));
                }
                r
            };
            let label_style = WS_CHILD | WS_VISIBLE;
            let btn_style = WS_CHILD | WS_VISIBLE;
            // WS_BORDER | ES_CENTER | ES_NUMBER on top of child/visible.
            let edit_style =
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0080_0000 | 0x0000_0001 | 0x0000_2000);
            // Signed edits (scroll notches, click offsets) cannot use
            // ES_NUMBER — it refuses the minus key.
            let edit_signed_style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0080_0000 | 0x0000_0001);
            // Free-text edits (slot names): border only.
            let edit_text_style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0080_0000);
            // BS_DEFPUSHBUTTON (0x1) so Enter applies via the dialog loop.
            let apply_style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0000_0001);

            // BS_GROUPBOX (0x7) for the page group frames, and
            // BS_OWNERDRAW (0xB) for the top bar's Start/Stop buttons.
            let group_style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0000_0007);
            let owner_style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0000_000B);

            // Top control bar (always visible, 0.11.0): the keybind
            // and its Change button, the interval edit (moved up from
            // page 0; Apply and the presets stay on that page), and
            // the owner-drawn Start/Stop run buttons.
            let _ = child(w!("STATIC"), "Hotkey", label_style, 16, 13, 40, 20, 0);
            let keyname_now = key_name(KEYBIND_VK.load(Ordering::SeqCst)).to_string();
            let _ = child(
                w!("STATIC"),
                &keyname_now,
                label_style,
                58,
                13,
                34,
                20,
                ID_IV_TOP_KEYNAME,
            );
            let _ = child(
                w!("BUTTON"),
                "Change",
                btn_style,
                96,
                8,
                58,
                28,
                ID_IV_TOP_KEYBIND,
            );
            let _ = child(w!("STATIC"), "Interval", label_style, 160, 13, 44, 20, 0);
            let current = INTERVAL_MS.load(Ordering::SeqCst).to_string();
            let _ = child(w!("EDIT"), &current, edit_style, 206, 8, 50, 28, ID_IV_EDIT);
            let _ = child(w!("STATIC"), "ms", label_style, 260, 13, 18, 20, 0);
            let _ = child(
                w!("BUTTON"),
                &top_start_caption(),
                owner_style,
                284,
                8,
                58,
                28,
                ID_IV_TOP_START,
            );
            let _ = child(
                w!("BUTTON"),
                "Stop",
                owner_style,
                348,
                8,
                58,
                28,
                ID_IV_TOP_STOP,
            );

            // Page buttons (always visible). They live in a footer
            // row now; `show_page` parks them at the window's bottom,
            // so these coordinates are only a starting point.
            let _ = child(
                w!("BUTTON"),
                &page_caption(0),
                btn_style,
                16,
                8,
                108,
                28,
                ID_IV_PAGE_CLICK,
            );
            let _ = child(
                w!("BUTTON"),
                &page_caption(1),
                btn_style,
                132,
                8,
                108,
                28,
                ID_IV_PAGE_ACTION,
            );
            let _ = child(
                w!("BUTTON"),
                &page_caption(2),
                btn_style,
                248,
                8,
                116,
                28,
                ID_IV_PAGE_TARGET,
            );

            // ----- Page 0: Clicking -----
            // Group: Auto-stop — the click limit and (0.11.0) the
            // elapsed-seconds limit; whichever is reached first
            // stops the run.
            let _ = child_p(
                0,
                w!("BUTTON"),
                "Auto-stop",
                group_style,
                16,
                46,
                392,
                96,
                0,
            );
            let _ = child_p(
                0,
                w!("STATIC"),
                "Stop after (clicks, 0 = no limit):",
                label_style,
                32,
                73,
                200,
                20,
                0,
            );
            let limit_now = LIMIT_CLICKS.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                0,
                w!("EDIT"),
                &limit_now,
                edit_style,
                238,
                67,
                62,
                30,
                ID_IV_LIMIT_EDIT,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                "Set",
                btn_style,
                308,
                67,
                56,
                30,
                ID_IV_LIMIT_SET,
            );
            let _ = child_p(
                0,
                w!("STATIC"),
                "Stop after (seconds, 0 = no limit):",
                label_style,
                32,
                109,
                200,
                20,
                0,
            );
            let limit_sec_now = LIMIT_SECONDS.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                0,
                w!("EDIT"),
                &limit_sec_now,
                edit_style,
                238,
                103,
                62,
                30,
                ID_IV_LIMIT_SEC_EDIT,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                "Set",
                btn_style,
                308,
                103,
                56,
                30,
                ID_IV_LIMIT_SEC_SET,
            );

            // Group: Click options.
            let _ = child_p(
                0,
                w!("BUTTON"),
                "Click options",
                group_style,
                16,
                152,
                392,
                346,
                0,
            );
            let _ = child_p(
                0,
                w!("STATIC"),
                "Interval presets:",
                label_style,
                32,
                176,
                180,
                18,
                0,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                "1 ms",
                btn_style,
                32,
                196,
                80,
                28,
                ID_IV_1MS,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                "100 ms",
                btn_style,
                120,
                196,
                80,
                28,
                ID_IV_100MS,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                "Apply",
                apply_style,
                208,
                196,
                96,
                28,
                ID_IV_APPLY,
            );
            let _ = child_p(
                0,
                w!("STATIC"),
                "Mouse button:",
                label_style,
                32,
                232,
                200,
                18,
                0,
            );
            let cur_btn = current_button();
            let left_cap = button_caption(ClickButton::Left, cur_btn);
            let _ = child_p(
                0,
                w!("BUTTON"),
                &left_cap,
                btn_style,
                32,
                252,
                80,
                28,
                ID_IV_BTN_LEFT,
            );
            let right_cap = button_caption(ClickButton::Right, cur_btn);
            let _ = child_p(
                0,
                w!("BUTTON"),
                &right_cap,
                btn_style,
                120,
                252,
                80,
                28,
                ID_IV_BTN_RIGHT,
            );
            let middle_cap = button_caption(ClickButton::Middle, cur_btn);
            let _ = child_p(
                0,
                w!("BUTTON"),
                &middle_cap,
                btn_style,
                208,
                252,
                80,
                28,
                ID_IV_BTN_MIDDLE,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                jitter_caption(),
                btn_style,
                296,
                252,
                96,
                28,
                ID_IV_JITTER,
            );
            let mode_cap = mode_caption();
            let _ = child_p(
                0,
                w!("BUTTON"),
                &mode_cap,
                btn_style,
                32,
                288,
                360,
                28,
                ID_IV_MODE,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                launch_caption(),
                btn_style,
                32,
                322,
                360,
                28,
                ID_IV_LAUNCH,
            );
            let sk_cap = stopkey_caption();
            let _ = child_p(
                0,
                w!("BUTTON"),
                &sk_cap,
                btn_style,
                32,
                356,
                176,
                28,
                ID_IV_STOPKEY,
            );
            let hk_cap = hotkey_caption();
            let _ = child_p(
                0,
                w!("BUTTON"),
                &hk_cap,
                btn_style,
                216,
                356,
                176,
                28,
                ID_IV_HOTKEY,
            );
            let _ = child_p(
                0,
                w!("STATIC"),
                "Start in (min):",
                label_style,
                32,
                396,
                104,
                18,
                0,
            );
            let _ = child_p(
                0,
                w!("EDIT"),
                "10",
                edit_signed_style,
                140,
                390,
                52,
                28,
                ID_IV_DELAY_EDIT,
            );
            let dl_cap = delay_caption();
            let _ = child_p(
                0,
                w!("BUTTON"),
                &dl_cap,
                btn_style,
                200,
                390,
                192,
                28,
                ID_IV_DELAY,
            );
            let fs_cap = fs_caption();
            let _ = child_p(
                0,
                w!("BUTTON"),
                &fs_cap,
                btn_style,
                32,
                424,
                176,
                28,
                ID_IV_FS,
            );
            let fg_cap = fg_caption();
            let _ = child_p(
                0,
                w!("BUTTON"),
                &fg_cap,
                btn_style,
                216,
                424,
                176,
                28,
                ID_IV_FG,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                "Pick app...",
                btn_style,
                32,
                458,
                140,
                28,
                ID_IV_FG_PICK,
            );
            let fgl_cap = fg_label_caption();
            let _ = child_p(
                0,
                w!("STATIC"),
                &fgl_cap,
                label_style,
                180,
                463,
                212,
                18,
                ID_IV_FG_LABEL,
            );

            // Group: Cursor position.
            let _ = child_p(
                0,
                w!("BUTTON"),
                "Cursor position",
                group_style,
                16,
                508,
                392,
                62,
                0,
            );
            let _ = child_p(
                0,
                w!("BUTTON"),
                &pos_caption(),
                btn_style,
                32,
                528,
                360,
                28,
                ID_IV_POS,
            );
            // ----- Page 1: Action -----
            let _ = child_p(
                1,
                w!("STATIC"),
                "Click action:",
                label_style,
                16,
                48,
                360,
                20,
                0,
            );
            let act_cap = action_caption();
            let _ = child_p(
                1,
                w!("BUTTON"),
                &act_cap,
                btn_style,
                16,
                72,
                150,
                30,
                ID_IV_ACTION,
            );
            let _ = child_p(
                1,
                w!("BUTTON"),
                alt_caption(),
                btn_style,
                176,
                72,
                188,
                30,
                ID_IV_ALT,
            );
            let rep_cap = repeat_caption();
            let _ = child_p(
                1,
                w!("BUTTON"),
                &rep_cap,
                btn_style,
                16,
                110,
                170,
                30,
                ID_IV_REPEAT,
            );
            let _ = child_p(
                1,
                w!("BUTTON"),
                drag_caption(),
                btn_style,
                196,
                110,
                168,
                30,
                ID_IV_DRAG,
            );
            let _ = child_p(
                1,
                w!("STATIC"),
                "Burst count / gap / down (ms) / scroll notches:",
                label_style,
                16,
                152,
                360,
                20,
                0,
            );
            let burst_now = BURST_CLICKS.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                1,
                w!("EDIT"),
                &burst_now,
                edit_style,
                16,
                176,
                52,
                30,
                ID_IV_BURST_EDIT,
            );
            let gap_now = BURST_GAP_MS.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                1,
                w!("EDIT"),
                &gap_now,
                edit_style,
                76,
                176,
                52,
                30,
                ID_IV_GAP_EDIT,
            );
            let press_now = PRESS_MS.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                1,
                w!("EDIT"),
                &press_now,
                edit_style,
                136,
                176,
                52,
                30,
                ID_IV_PRESS_EDIT,
            );
            let scroll_now = SCROLL_AMT.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                1,
                w!("EDIT"),
                &scroll_now,
                edit_signed_style,
                196,
                176,
                52,
                30,
                ID_IV_SCROLL_EDIT,
            );
            let _ = child_p(
                1,
                w!("BUTTON"),
                "Set",
                btn_style,
                256,
                176,
                108,
                30,
                ID_IV_NUM_SET,
            );
            let _ = child_p(
                1,
                w!("STATIC"),
                "Random interval and speed ramp:",
                label_style,
                16,
                218,
                360,
                20,
                0,
            );
            let _ = child_p(
                1,
                w!("BUTTON"),
                random_caption(),
                btn_style,
                16,
                242,
                120,
                30,
                ID_IV_RANDOM,
            );
            let _ = child_p(1, w!("STATIC"), "Max ms:", label_style, 144, 248, 52, 20, 0);
            let max_now = INTERVAL_MAX_MS.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                1,
                w!("EDIT"),
                &max_now,
                edit_style,
                200,
                242,
                56,
                30,
                ID_IV_MAX_EDIT,
            );
            let _ = child_p(
                1,
                w!("BUTTON"),
                ramp_caption(),
                btn_style,
                264,
                242,
                100,
                30,
                ID_IV_RAMP,
            );
            let _ = child_p(
                1,
                w!("STATIC"),
                "Ramp to ms:",
                label_style,
                16,
                286,
                68,
                20,
                0,
            );
            let to_now = RAMP_TO_MS.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                1,
                w!("EDIT"),
                &to_now,
                edit_style,
                88,
                280,
                56,
                30,
                ID_IV_RAMP_TO_EDIT,
            );
            let _ = child_p(
                1,
                w!("STATIC"),
                "over secs:",
                label_style,
                152,
                286,
                60,
                20,
                0,
            );
            let secs_now = RAMP_SECONDS.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                1,
                w!("EDIT"),
                &secs_now,
                edit_style,
                216,
                280,
                52,
                30,
                ID_IV_RAMP_SECS_EDIT,
            );
            let _ = child_p(
                1,
                w!("BUTTON"),
                "Set",
                btn_style,
                276,
                280,
                88,
                30,
                ID_IV_TIMING_SET,
            );

            // ----- Page 2: Targeting -----
            let _ = child_p(
                2,
                w!("STATIC"),
                "Lock point (crosshair pick):",
                label_style,
                16,
                48,
                360,
                20,
                0,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                &pick_caption(),
                btn_style,
                16,
                72,
                176,
                30,
                ID_IV_PICK_LOCK,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                "Other monitor centre",
                btn_style,
                196,
                72,
                168,
                30,
                ID_IV_OTHER_MON,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                winrel_caption(),
                btn_style,
                16,
                110,
                176,
                30,
                ID_IV_WINREL,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                region_caption(),
                btn_style,
                196,
                110,
                168,
                30,
                ID_IV_REGION,
            );
            let _ = child_p(
                2,
                w!("STATIC"),
                "Multi-point sequence (clicks rotate through the points):",
                label_style,
                16,
                148,
                348,
                20,
                0,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                &seq_caption(),
                btn_style,
                16,
                172,
                240,
                30,
                ID_IV_SEQ_SET,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                "Clear",
                btn_style,
                264,
                172,
                100,
                30,
                ID_IV_SEQ_CLEAR,
            );
            let _ = child_p(
                2,
                w!("STATIC"),
                "Click offset in pixels (cursor and lock points):",
                label_style,
                16,
                210,
                348,
                20,
                0,
            );
            let _ = child_p(2, w!("STATIC"), "X:", label_style, 16, 240, 20, 20, 0);
            let off_x_now = OFFSET_X.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                2,
                w!("EDIT"),
                &off_x_now,
                edit_signed_style,
                40,
                234,
                52,
                30,
                ID_IV_OFF_X_EDIT,
            );
            let _ = child_p(2, w!("STATIC"), "Y:", label_style, 100, 240, 20, 20, 0);
            let off_y_now = OFFSET_Y.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                2,
                w!("EDIT"),
                &off_y_now,
                edit_signed_style,
                124,
                234,
                52,
                30,
                ID_IV_OFF_Y_EDIT,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                "Set",
                btn_style,
                184,
                234,
                80,
                30,
                ID_IV_OFF_SET,
            );
            let _ = child_p(
                2,
                w!("STATIC"),
                "-200..200",
                label_style,
                272,
                240,
                92,
                20,
                0,
            );
            let _ = child_p(
                2,
                w!("STATIC"),
                "Named slots - Save stores the lock point (or cursor):",
                label_style,
                16,
                272,
                348,
                20,
                0,
            );
            let slot_name_ids = [ID_IV_SLOT1_NAME, ID_IV_SLOT2_NAME, ID_IV_SLOT3_NAME];
            let slot_save_ids = [ID_IV_SLOT1_SAVE, ID_IV_SLOT2_SAVE, ID_IV_SLOT3_SAVE];
            let slot_use_ids = [ID_IV_SLOT1_USE, ID_IV_SLOT2_USE, ID_IV_SLOT3_USE];
            let slot_coords_ids = [ID_IV_SLOT1_COORDS, ID_IV_SLOT2_COORDS, ID_IV_SLOT3_COORDS];
            let names_now = slot_names()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            for i in 0..3 {
                let sy = 296 + (i as i32) * 32;
                let _ = child_p(
                    2,
                    w!("EDIT"),
                    &names_now[i],
                    edit_text_style,
                    16,
                    sy,
                    128,
                    28,
                    slot_name_ids[i],
                );
                let _ = child_p(
                    2,
                    w!("STATIC"),
                    &slot_coords_caption(i),
                    label_style,
                    152,
                    sy + 5,
                    100,
                    20,
                    slot_coords_ids[i],
                );
                let _ = child_p(
                    2,
                    w!("BUTTON"),
                    "Save",
                    btn_style,
                    258,
                    sy,
                    50,
                    28,
                    slot_save_ids[i],
                );
                let _ = child_p(
                    2,
                    w!("BUTTON"),
                    "Use",
                    btn_style,
                    312,
                    sy,
                    52,
                    28,
                    slot_use_ids[i],
                );
            }
            let _ = child_p(
                2,
                w!("STATIC"),
                "Pixel check - click only while the target pixel keeps its colour:",
                label_style,
                16,
                400,
                348,
                20,
                0,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                pixel_caption(),
                btn_style,
                16,
                424,
                176,
                30,
                ID_IV_PIXEL,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                &sample_caption(),
                btn_style,
                196,
                424,
                168,
                30,
                ID_IV_PIXEL_SAMPLE,
            );
            let _ = child_p(
                2,
                w!("STATIC"),
                "Tolerance (0-64):",
                label_style,
                16,
                468,
                118,
                20,
                0,
            );
            let tol_now = PIXEL_TOL.load(Ordering::SeqCst).to_string();
            let _ = child_p(
                2,
                w!("EDIT"),
                &tol_now,
                edit_style,
                140,
                462,
                52,
                30,
                ID_IV_TOL_EDIT,
            );
            let _ = child_p(
                2,
                w!("BUTTON"),
                "Set",
                btn_style,
                200,
                462,
                88,
                30,
                ID_IV_TOL_SET,
            );

            // Footer (always visible; show_page parks it and the
            // page buttons at the bottom).
            let _ = child(
                w!("STATIC"),
                "Pages group the settings. Presets and toggles apply at once; numbers apply with their Set buttons. Esc closes.",
                label_style,
                16,
                380,
                392,
                28,
                ID_IV_FOOTER,
            );
            show_page(hwnd, CURRENT_PAGE.load(Ordering::SeqCst));
            let _ = SetFocus(Some(hwnd));
        }
    }

    // ---------- update check (1.0.0) ----------

    /// The launch-time update check, on its own thread so startup
    /// never waits for the network. It runs at every launch — there
    /// is deliberately no setting for it — and its only visible trace
    /// is the prompt window when GitHub's latest release is newer
    /// than this build. Every failure along the way (offline, DNS,
    /// TLS, a non-200 reply, a body that does not parse) simply ends
    /// the thread: no window, no retry, nothing written anywhere.
    /// This check is the program's only network use.
    fn update_check_thread() {
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
        let Some(body) = fetch_latest_release() else {
            return;
        };
        let Some((tag, url)) = parse_latest_release(&body) else {
            return;
        };
        if !is_newer_version(env!("CARGO_PKG_VERSION"), &tag) {
            return;
        }
        *update_offer()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((tag, url));
        post_main(WM_APP_UPDATE);
    }

    /// GET the latest-release JSON from GitHub over WinHTTP, with the
    /// user agent GitHub's API requires. `None` on any failure.
    fn fetch_latest_release() -> Option<String> {
        let (_, after_scheme) = LATEST_RELEASE_URL.split_once("://")?;
        let (host, path) = after_scheme.split_once('/')?;
        let agent = wide("TrayClick");
        let host_w = wide(host);
        let path_w = wide(&format!("/{path}"));
        unsafe {
            let session = WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            );
            if session.is_null() {
                return None;
            }
            // Resolve/connect/send/receive each get ~10 s; an
            // unreachable network must not linger past that.
            let _ = WinHttpSetTimeouts(session, 10_000, 10_000, 10_000, 10_000);
            let connect = WinHttpConnect(
                session,
                PCWSTR(host_w.as_ptr()),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            );
            if connect.is_null() {
                let _ = WinHttpCloseHandle(session);
                return None;
            }
            let request = WinHttpOpenRequest(
                connect,
                w!("GET"),
                PCWSTR(path_w.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            );
            if request.is_null() {
                let _ = WinHttpCloseHandle(connect);
                let _ = WinHttpCloseHandle(session);
                return None;
            }
            let body = send_and_read_release(request);
            let _ = WinHttpCloseHandle(request);
            let _ = WinHttpCloseHandle(connect);
            let _ = WinHttpCloseHandle(session);
            body
        }
    }

    /// Send the prepared request and read its body, requiring an
    /// HTTP 200. Split out of `fetch_latest_release` so that function
    /// only juggles handle lifetimes.
    fn send_and_read_release(request: *mut c_void) -> Option<String> {
        unsafe {
            WinHttpSendRequest(request, None, None, 0, 0, 0).ok()?;
            WinHttpReceiveResponse(request, std::ptr::null_mut()).ok()?;
            // The status code is a pseudo-header: it is queried
            // with no header name (NULL) and no header index
            // (WINHTTP_NO_HEADER_INDEX, which is NULL) — passing
            // any other pointer value here makes WinHTTP read from
            // that address and crash inside winhttp.dll (field
            // report 2026-11-02: 1.0.0 died at launch, exception
            // 0xc0000005 in winhttp.dll).
            let mut status: u32 = 0;
            let mut len = std::mem::size_of::<u32>() as u32;
            WinHttpQueryHeaders(
                request,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut status as *mut u32 as *mut c_void),
                &mut len,
                std::ptr::null_mut(),
            )
            .ok()?;
            if status != 200 {
                return None;
            }
            let mut bytes: Vec<u8> = Vec::new();
            loop {
                let mut available: u32 = 0;
                WinHttpQueryDataAvailable(request, &mut available).ok()?;
                if available == 0 {
                    break;
                }
                // The release JSON is a few KB; anything past 1 MiB
                // is not it, whatever answered.
                if bytes.len() + available as usize > 1_048_576 {
                    return None;
                }
                let mut chunk = vec![0u8; available as usize];
                let mut read: u32 = 0;
                WinHttpReadData(
                    request,
                    chunk.as_mut_ptr() as *mut c_void,
                    available,
                    &mut read,
                )
                .ok()?;
                if read == 0 {
                    break;
                }
                chunk.truncate(read as usize);
                bytes.extend_from_slice(&chunk);
            }
            String::from_utf8(bytes).ok()
        }
    }

    /// Owner-draw the prompt's Download button, matching the settings
    /// window's Start button: red fill, brighter red border, white
    /// text (see `draw_run_button`).
    fn draw_download_button(dis: &DRAWITEMSTRUCT) {
        unsafe {
            let hdc = dis.hDC;
            let rc = dis.rcItem;
            FillRect(hdc, &rc, start_fill_brush());
            FrameRect(hdc, &rc, accent_brush());
            let mut buf = [0u16; 32];
            let n = GetWindowTextW(dis.hwndItem, &mut buf);
            if n > 0 {
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00FF_FFFF));
                let mut text_rc = rc;
                DrawTextW(
                    hdc,
                    &mut buf[..n as usize],
                    &mut text_rc,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
    }

    unsafe extern "system" fn update_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_COMMAND => {
                match wparam.0 & 0xFFFF {
                    ID_UP_DOWNLOAD => {
                        // Open the release page in the browser; the
                        // download and install stay the user's own
                        // steps — nothing is fetched or run for them.
                        let url = update_offer()
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .as_ref()
                            .map(|(_, url)| url.clone());
                        if let Some(url) = url {
                            let url_w = wide(&url);
                            let _ = ShellExecuteW(
                                Some(hwnd),
                                w!("open"),
                                PCWSTR(url_w.as_ptr()),
                                PCWSTR::null(),
                                PCWSTR::null(),
                                SW_SHOWNORMAL,
                            );
                        }
                        let _ = DestroyWindow(hwnd);
                    }
                    // Later — and IDCANCEL (2), which the dialog
                    // message loop sends for Esc — just closes.
                    ID_UP_LATER | 2 => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                // The settings window's charcoal theme.
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, COLORREF(0x00F2F2F2));
                SetBkMode(hdc, TRANSPARENT);
                LRESULT(settings_bg_brush().0 as isize)
            }
            WM_DRAWITEM => {
                let dis = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
                draw_download_button(dis);
                LRESULT(1)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                UPDATE_HWND.store(0, Ordering::SeqCst);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Open the update prompt, or raise it if it is somehow already
    /// open. Called from the main window on `WM_APP_UPDATE`; styled
    /// exactly like the settings window (charcoal background, dark
    /// title bar, red accent on the primary button).
    fn open_update_window(parent: HWND) {
        let raw = UPDATE_HWND.load(Ordering::SeqCst);
        if raw != 0 {
            let existing = HWND(raw as isize as *mut c_void);
            unsafe {
                if IsWindow(Some(existing)).as_bool() {
                    let _ = SetForegroundWindow(existing);
                    return;
                }
            }
        }
        let Some((tag, _url)) = update_offer()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            return;
        };
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            static REGISTERED: AtomicBool = AtomicBool::new(false);
            if !REGISTERED.swap(true, Ordering::SeqCst) {
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(update_wndproc),
                    hInstance: instance.into(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    hbrBackground: CreateSolidBrush(COLORREF(0x001B_1B1B)),
                    lpszClassName: w!("TrayClickUpdate"),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            }
            // Fixed layout in physical pixels, like the settings
            // window (the process is per-monitor DPI aware, so no
            // scaling helper exists to reuse), centred on the screen.
            let width = 440;
            let height = 172;
            let x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
            let y = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("TrayClickUpdate"),
                w!("TrayClick — Update available"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                x,
                y,
                width,
                height,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(h) => h,
                Err(_) => return,
            };
            UPDATE_HWND.store(hwnd.0 as i32, Ordering::SeqCst);
            // Dark title bar to match the charcoal client area.
            let dark: i32 = 1;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
            let label_style = WS_CHILD | WS_VISIBLE;
            let btn_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
            // BS_OWNERDRAW (0xB) for the Download button.
            let owner_style = btn_style | WINDOW_STYLE(0x0000_000B);
            let version = tag.strip_prefix('v').unwrap_or(&tag);
            let line1 = wide(&format!("TrayClick {version} is available."));
            let _ = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR(line1.as_ptr()),
                label_style,
                20,
                18,
                384,
                20,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            );
            let line2 = wide(&format!(
                "You have {}. Download it from the release page?",
                env!("CARGO_PKG_VERSION")
            ));
            let _ = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR(line2.as_ptr()),
                label_style,
                20,
                44,
                384,
                20,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            );
            let _ = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("Download"),
                owner_style,
                192,
                84,
                108,
                32,
                Some(hwnd),
                Some(HMENU(ID_UP_DOWNLOAD as *mut c_void)),
                Some(instance.into()),
                None,
            );
            let later = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("Later"),
                btn_style,
                308,
                84,
                108,
                32,
                Some(hwnd),
                Some(HMENU(ID_UP_LATER as *mut c_void)),
                Some(instance.into()),
                None,
            );
            // Focus rests on Later, so a stray keypress while the
            // prompt appears can only dismiss it, never open a page.
            if let Ok(later) = later {
                let _ = SetFocus(Some(later));
            }
        }
    }

    // ---------- main (hidden) window ----------

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Explorer (re)created the taskbar: re-add the tray icon, which
        // does not survive an Explorer restart on its own.
        let taskbar_msg = TASKBAR_CREATED_MSG.load(Ordering::SeqCst);
        if taskbar_msg != 0 && msg == taskbar_msg {
            tray_add(hwnd);
            refresh_indicator(hwnd);
            return LRESULT(0);
        }
        match msg {
            m if m == WM_APP_FAILSAFE => {
                // The clicker already stopped; just settle the icon.
                refresh_indicator(hwnd);
                LRESULT(0)
            }
            m if m == WM_APP_LIMIT => {
                // The clicker stopped at the auto-stop limit; settle.
                refresh_indicator(hwnd);
                LRESULT(0)
            }
            m if m == WM_APP_REGION => {
                // The region guard stopped the run; settle the icon.
                refresh_indicator(hwnd);
                LRESULT(0)
            }
            WM_TRAYICON => {
                // Left-click opens the settings window; right-click
                // opens the menu (0.11.0 — before, both opened it).
                let event = lparam.0 as u32;
                if event == WM_RBUTTONUP {
                    show_menu(hwnd);
                } else if event == WM_LBUTTONUP {
                    open_settings_from_tray(hwnd);
                }
                LRESULT(0)
            }
            m if m == WM_APP_TOGGLE => {
                handle_keybind_press(hwnd);
                LRESULT(0)
            }
            m if m == WM_APP_STOP => {
                engine_stop(hwnd);
                LRESULT(0)
            }
            m if m == WM_APP_COMMAND => {
                // A forwarded CLI command: 1 = start, 2 = stop, 3 = toggle.
                match wparam.0 {
                    1 => engine_start(hwnd),
                    2 => engine_stop(hwnd),
                    3 => handle_keybind_press(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            m if m == WM_APP_GATE => {
                // The run gate flipped; settle the held button + icon.
                refresh_indicator(hwnd);
                LRESULT(0)
            }
            m if m == WM_APP_REHOTKEY => {
                register_hotkeys(hwnd);
                LRESULT(0)
            }
            m if m == WM_APP_UPDATE => {
                // The launch-time check found a newer release.
                open_update_window(hwnd);
                LRESULT(0)
            }
            WM_HOTKEY => {
                if wparam.0 as i32 == ID_HOTKEY_KEYBIND {
                    handle_keybind_press(hwnd);
                } else if wparam.0 as i32 == ID_HOTKEY_STOP {
                    engine_stop(hwnd);
                }
                LRESULT(0)
            }
            WM_COPYDATA => {
                // A forwarded CLI --profile: remember the name.
                // (Profiles themselves arrive in v1.0.0.)
                let cds = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
                if cds.dwData == 1 && cds.cbData > 0 && !cds.lpData.is_null() {
                    let bytes = unsafe {
                        std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize)
                    };
                    let name = String::from_utf8_lossy(bytes).into_owned();
                    *pending_profile()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = name;
                }
                LRESULT(1)
            }
            WM_TIMER if wparam.0 == ID_DELAY_TIMER => {
                // The delayed start fires: kill the one-shot timer
                // and start clicking.
                unsafe {
                    let _ = KillTimer(Some(hwnd), ID_DELAY_TIMER);
                }
                *delay_end()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                engine_start(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == ID_BLINK_TIMER => {
                if lock_engine().is_clicking_now() {
                    // Alternate normal <-> active icon.
                    let show_normal = !BLINK_NORMAL_SHOWN.load(Ordering::SeqCst);
                    BLINK_NORMAL_SHOWN.store(show_normal, Ordering::SeqCst);
                    let icon = if show_normal {
                        app_icon()
                    } else {
                        active_icon()
                    };
                    tray_modify(hwnd, icon, &on_tooltip());
                } else {
                    // State changed without a refresh; settle the icon.
                    refresh_indicator(hwnd);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                match wparam.0 & 0xFFFF {
                    ID_PAUSE_RESUME => {
                        lock_engine().toggle_pause();
                        refresh_indicator(hwnd);
                    }
                    ID_SET_KEYBIND => open_capture_window(hwnd),
                    ID_SET_INTERVAL => open_interval_window(hwnd),
                    ID_START_WITH_WINDOWS => set_autostart(!autostart_enabled()),
                    ID_CREDITS => show_credits(hwnd),
                    ID_EXIT => {
                        release_inputs_on_exit();
                        tray_remove(hwnd);
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // Finish accepted input cleanup before the worker can be terminated.
                release_inputs_on_exit();
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Forward a CLI command to the already-running instance, if
    /// there is one: start/stop/toggle travel as window messages,
    /// the profile name as WM_COPYDATA (UTF-8). Returns true when a
    /// running window took the command (this launch should exit).
    fn forward_cli(cmd: &CliCommand) -> bool {
        unsafe {
            let hwnd = match FindWindowW(w!("TrayClickMain"), PCWSTR::null()) {
                Ok(h) if !h.is_invalid() => h,
                _ => return false,
            };
            match cmd {
                CliCommand::Start => {
                    let _ = PostMessageW(Some(hwnd), WM_APP_COMMAND, WPARAM(1), LPARAM(0));
                }
                CliCommand::Stop => {
                    let _ = PostMessageW(Some(hwnd), WM_APP_COMMAND, WPARAM(2), LPARAM(0));
                }
                CliCommand::Toggle => {
                    let _ = PostMessageW(Some(hwnd), WM_APP_COMMAND, WPARAM(3), LPARAM(0));
                }
                CliCommand::Profile(name) => {
                    let bytes = name.as_bytes();
                    let cds = COPYDATASTRUCT {
                        dwData: 1,
                        cbData: bytes.len() as u32,
                        lpData: bytes.as_ptr() as *mut c_void,
                    };
                    // SendMessage (not Post): the data buffer must
                    // stay valid until the receiver has copied it.
                    let _ = SendMessageW(
                        hwnd,
                        WM_COPYDATA,
                        Some(WPARAM(0)),
                        Some(LPARAM(&cds as *const COPYDATASTRUCT as isize)),
                    );
                }
                // Maintenance commands never reach forwarding: `run`
                // handles them before this is called.
                _ => return false,
            }
            true
        }
    }

    // ---------- CLI maintenance commands (0.10.0) ----------

    /// The single-instance mutex name. Changed in 0.10.0 from the
    /// plain "TrayClickSingleInstance" to a GUID-form name so it
    /// cannot collide with another program's mutex.
    const INSTANCE_MUTEX_NAME: &str = "TrayClick-{62C29D04-266D-4455-AFE3-C19063EF7C82}";

    /// True when another copy of TrayClick is already running (it
    /// holds the single-instance mutex). This is only a probe — the
    /// handle is closed again before returning; the real instance
    /// claim happens in `run`.
    fn another_instance_running() -> bool {
        let name = wide(INSTANCE_MUTEX_NAME);
        let probe = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) };
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if let Ok(handle) = probe {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
        already
    }

    /// Print a maintenance command's output: to the parent console
    /// when there is one (the normal case for a command-line
    /// launch), else in a message box — unless `--silent` was
    /// given, which keeps everything on the console channel (and so
    /// shows nothing when there is no console).
    fn cli_output(text: &str, opts: &CliOptions) {
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        use std::io::Write;
        if let Ok(mut con) = std::fs::File::options().write(true).open("CONOUT$") {
            let _ = writeln!(con, "{text}");
            let _ = con.flush();
            return;
        }
        if !opts.silent {
            let text_w = wide(text);
            let title_w = wide("TrayClick");
            unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(text_w.as_ptr()),
                    PCWSTR(title_w.as_ptr()),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
    }

    /// Minimal JSON string escaping for the small objects the
    /// maintenance commands print with `--json`.
    fn json_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out
    }

    /// The `--version` output, as a text line or a JSON object.
    fn version_text(json: bool) -> String {
        let version = env!("CARGO_PKG_VERSION");
        if json {
            format!("{{\"name\":\"TrayClick\",\"version\":\"{version}\"}}")
        } else {
            format!("TrayClick {version}")
        }
    }

    /// Delete one file or folder if it exists, recording its path
    /// in `removed`. With `dry_run`, record without deleting.
    fn remove_target(
        path: &std::path::Path,
        is_dir: bool,
        dry_run: bool,
        removed: &mut Vec<String>,
    ) {
        if !path.exists() {
            return;
        }
        if !dry_run {
            let deleted = if is_dir {
                std::fs::remove_dir_all(path).is_ok()
            } else {
                std::fs::remove_file(path).is_ok()
            };
            if !deleted {
                return;
            }
        }
        removed.push(path.display().to_string());
    }

    /// Shared body of `--clean` and `--uninstall`: delete the
    /// settings files at both locations, the `data` folder, the
    /// legacy settings folder, and the Start with Windows entry.
    /// Returns the process exit code: 1 when refused because another
    /// copy is running (a live copy's config is never touched), else
    /// 0. `--uninstall` differs only in its closing note — program
    /// files placed by the installer are removed through Windows
    /// Settings, not from here.
    fn run_cleanup(uninstall: bool, dry_run: bool, opts: &CliOptions) -> i32 {
        let command = if uninstall { "uninstall" } else { "clean" };
        if another_instance_running() {
            if opts.json {
                cli_output(
                    &format!("{{\"command\":\"{command}\",\"error\":\"already-running\"}}"),
                    opts,
                );
            } else {
                cli_output(
                    "TrayClick is already running. Quit it from the tray, then try again.",
                    opts,
                );
            }
            return 1;
        }

        let mut removed: Vec<String> = Vec::new();
        let portable = portable_config_path();
        if let Some(data_dir) = portable.parent() {
            remove_target(&portable, false, dry_run, &mut removed);
            remove_target(data_dir, true, dry_run, &mut removed);
        }
        let legacy = legacy_config_path();
        if let Some(legacy_dir) = legacy.parent() {
            remove_target(&legacy, false, dry_run, &mut removed);
            remove_target(legacy_dir, true, dry_run, &mut removed);
        }
        if autostart_value().is_some() {
            if !dry_run {
                set_autostart(false);
            }
            removed.push("Start with Windows entry (HKCU Run value)".to_string());
        }

        if opts.json {
            let items: Vec<String> = removed
                .iter()
                .map(|r| format!("\"{}\"", json_escape(r)))
                .collect();
            cli_output(
                &format!(
                    "{{\"command\":\"{command}\",\"dry_run\":{dry_run},\"removed\":[{}]}}",
                    items.join(",")
                ),
                opts,
            );
            return 0;
        }

        let mut text = String::new();
        if dry_run {
            text.push_str("Dry run - nothing was changed.\n");
        }
        if removed.is_empty() {
            text.push_str("Nothing to remove: no TrayClick settings or sign-in entry found.");
        } else {
            text.push_str(if dry_run {
                "Would remove:\n"
            } else {
                "Removed:\n"
            });
            for item in &removed {
                text.push_str("  ");
                text.push_str(item);
                text.push('\n');
            }
            text = text.trim_end().to_string();
        }
        if uninstall {
            text.push_str(
                "\nProgram files installed by the installer are not removed here; \
                 uninstall TrayClick from Windows Settings > Apps to remove them.",
            );
        }
        cli_output(&text, opts);
        0
    }

    pub fn run() {
        // Per-monitor DPI awareness (v2), set before any window exists:
        // without it Windows virtualizes coordinates on scaled displays,
        // and locked-position clicks land off target. Failure (very old
        // Windows) is non-fatal — the default awareness simply stays.
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }

        // Command-line control (item 25): a launch carrying a command
        // forwards it to the running instance and exits. With no
        // instance running, --start/--toggle/--profile launch the app
        // (starting clicking for start/toggle); --stop does nothing.
        let args: Vec<String> = std::env::args().skip(1).collect();
        let (cli, cli_opts) = parse_cli_full(&args);
        // Maintenance commands (0.10.0) act locally and exit, before
        // anything is forwarded and before any window is created.
        match &cli {
            Some(CliCommand::Help) => {
                cli_output(USAGE, &cli_opts);
                return;
            }
            Some(CliCommand::Version) => {
                cli_output(&version_text(cli_opts.json), &cli_opts);
                return;
            }
            Some(CliCommand::Clean { dry_run }) => {
                std::process::exit(run_cleanup(false, *dry_run, &cli_opts));
            }
            Some(CliCommand::Uninstall { dry_run }) => {
                std::process::exit(run_cleanup(true, *dry_run, &cli_opts));
            }
            _ => {}
        }
        let mut cli_start = false;
        if let Some(cmd) = &cli {
            if forward_cli(cmd) {
                return;
            }
            match cmd {
                CliCommand::Stop => return,
                CliCommand::Start | CliCommand::Toggle => cli_start = true,
                CliCommand::Profile(name) => {
                    *pending_profile()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = name.clone();
                }
                // Maintenance commands returned or exited in the match
                // above; they never reach this one.
                CliCommand::Help
                | CliCommand::Version
                | CliCommand::Clean { .. }
                | CliCommand::Uninstall { .. } => {}
            }
        }

        // Single instance: a second copy would fight over the keybind.
        // The handle is held in this binding until `run` returns at
        // process exit. (In the `windows` crate `HANDLE` is a plain
        // `Copy` value with no `Drop`, so an earlier draft of this code
        // was reviewed as if the handle closed when its block ended —
        // it never did; keeping the binding simply makes the lifetime
        // explicit and guarantees nothing closes it early.)
        let mutex_name = wide(INSTANCE_MUTEX_NAME);
        let instance_mutex = unsafe { CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr())) };
        // Whether the call itself reports Ok or Err, ERROR_ALREADY_EXISTS
        // is the signal that another copy holds the name.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let text = wide("TrayClick is already running — check the tray.");
            let title = wide("TrayClick");
            unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            return;
        }
        let _instance_mutex = instance_mutex;

        let cfg = load_config();
        KEYBIND_VK.store(cfg.keybind_vk, Ordering::SeqCst);
        INTERVAL_MS.store(cfg.interval_ms, Ordering::SeqCst);
        BUTTON.store(button_u32(cfg.button), Ordering::SeqCst);
        JITTER_ON.store(cfg.jitter, Ordering::SeqCst);
        MODE.store(mode_u32(cfg.mode), Ordering::SeqCst);
        TARGET_MODE.store(target_u32(cfg.target), Ordering::SeqCst);
        LOCK_X.store(cfg.lock_x, Ordering::SeqCst);
        LOCK_Y.store(cfg.lock_y, Ordering::SeqCst);
        LOCK_SET.store(cfg.lock_set, Ordering::SeqCst);
        LOCK_REL.store(cfg.lock_rel, Ordering::SeqCst);
        LOCK_RX.store(cfg.lock_rx, Ordering::SeqCst);
        LOCK_RY.store(cfg.lock_ry, Ordering::SeqCst);
        SEQ_COUNT.store(cfg.seq_count, Ordering::SeqCst);
        for i in 0..4 {
            SEQ_X[i].store(cfg.seq_x[i], Ordering::SeqCst);
            SEQ_Y[i].store(cfg.seq_y[i], Ordering::SeqCst);
        }
        for i in 0..3 {
            SLOT_X[i].store(cfg.slot_x[i], Ordering::SeqCst);
            SLOT_Y[i].store(cfg.slot_y[i], Ordering::SeqCst);
            SLOT_SET[i].store(cfg.slot_set[i], Ordering::SeqCst);
        }
        *slot_names()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = cfg.slot_names.clone();
        OFFSET_X.store(cfg.offset_x, Ordering::SeqCst);
        OFFSET_Y.store(cfg.offset_y, Ordering::SeqCst);
        PIXEL_CHECK.store(cfg.pixel_check, Ordering::SeqCst);
        PIXEL_SET.store(cfg.pixel_set, Ordering::SeqCst);
        PIXEL_RGB.store(cfg.pixel_rgb, Ordering::SeqCst);
        PIXEL_TOL.store(cfg.pixel_tol, Ordering::SeqCst);
        REGION_GUARD.store(cfg.region_guard, Ordering::SeqCst);
        STOP_VK.store(cfg.stop_vk, Ordering::SeqCst);
        HOTKEY_MODE.store(cfg.hotkey_mode, Ordering::SeqCst);
        FG_RULE.store(cfg.fg_rule, Ordering::SeqCst);
        *fg_exe()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = cfg.fg_exe.clone();
        FS_PAUSE.store(cfg.fs_pause, Ordering::SeqCst);
        LIMIT_CLICKS.store(cfg.limit_clicks, Ordering::SeqCst);
        LIMIT_SECONDS.store(cfg.limit_seconds, Ordering::SeqCst);
        START_ON_LAUNCH.store(cfg.start_on_launch, Ordering::SeqCst);
        ACTION.store(action_u32(cfg.action), Ordering::SeqCst);
        BURST_CLICKS.store(cfg.burst_clicks, Ordering::SeqCst);
        BURST_GAP_MS.store(cfg.burst_gap_ms, Ordering::SeqCst);
        PRESS_MS.store(cfg.press_ms, Ordering::SeqCst);
        ALTERNATE.store(cfg.alternate, Ordering::SeqCst);
        SCROLL_AMT.store(cfg.scroll_amount, Ordering::SeqCst);
        REPEAT_VK.store(cfg.repeat_vk, Ordering::SeqCst);
        DRAG_AX.store(cfg.drag_ax, Ordering::SeqCst);
        DRAG_AY.store(cfg.drag_ay, Ordering::SeqCst);
        DRAG_BX.store(cfg.drag_bx, Ordering::SeqCst);
        DRAG_BY.store(cfg.drag_by, Ordering::SeqCst);
        DRAG_SET.store(cfg.drag_set, Ordering::SeqCst);
        RANDOM_ON.store(cfg.random_interval, Ordering::SeqCst);
        INTERVAL_MAX_MS.store(cfg.interval_max_ms, Ordering::SeqCst);
        RAMP_ON.store(cfg.ramp, Ordering::SeqCst);
        RAMP_TO_MS.store(cfg.ramp_to_ms, Ordering::SeqCst);
        RAMP_SECONDS.store(cfg.ramp_seconds, Ordering::SeqCst);
        if cfg.start_on_launch && cfg.mode == ClickMode::Click {
            lock_engine().clicking = true;
            on_run_started();
            if cfg.target == TargetMode::Locked && !cfg.lock_set {
                capture_lock_point();
            }
        }

        std::thread::spawn(clicker_thread);
        std::thread::spawn(keybind_thread);

        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("TrayClickMain"),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("TrayClickMain"),
                w!("TrayClick"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            ) {
                Ok(h) => h,
                Err(_) => return,
            };
            MAIN_HWND.store(hwnd.0 as i32, Ordering::SeqCst);
            let taskbar_id = RegisterWindowMessageW(w!("TaskbarCreated"));
            if taskbar_id != 0 {
                TASKBAR_CREATED_MSG.store(taskbar_id, Ordering::SeqCst);
            }
            tray_add(hwnd);
            // The launch-time update check (1.0.0): one background
            // thread on the normal tray startup path only — launches
            // that forwarded a command or ran a maintenance command
            // have already returned above. It posts WM_APP_UPDATE
            // only when a newer release exists.
            std::thread::spawn(update_check_thread);
            refresh_indicator(hwnd);
            register_hotkeys(hwnd);
            if cli_start {
                engine_start(hwnd);
            }

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                // Give the Set Interval window dialog keyboard behaviour
                // (Tab, Enter = Apply, Esc = cancel) while it is open.
                let iv = INTERVAL_HWND.load(Ordering::SeqCst);
                if iv != 0 {
                    let iv_hwnd = HWND(iv as isize as *mut c_void);
                    if IsDialogMessageW(iv_hwnd, &msg).as_bool() {
                        continue;
                    }
                }
                // Same dialog behaviour for the update prompt while
                // it is open (Tab between its buttons, Esc closes).
                let up = UPDATE_HWND.load(Ordering::SeqCst);
                if up != 0 {
                    let up_hwnd = HWND(up as isize as *mut c_void);
                    if IsDialogMessageW(up_hwnd, &msg).as_bool() {
                        continue;
                    }
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            tray_remove(hwnd);
        }
    }
}
