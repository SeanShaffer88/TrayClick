#![cfg_attr(windows, windows_subsystem = "windows")]

//! TrayClick — a Windows tray auto clicker.
//!
//! Lives entirely in the notification area. Left- or right-clicking the
//! tray icon opens a menu with exactly six items:
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
//! (click mode only).
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
        absolute_coord, format_count, ico_image_for_size, in_failsafe_corner, is_valid_interval,
        is_valid_keybind, is_valid_limit, jittered_interval_ms, key_name, next_click_deadline,
        ClickButton, ClickMode, Config, Engine, KeyPressDetector, MAX_INTERVAL_MS, MIN_INTERVAL_MS,
        VK_ESCAPE_CODE,
    };

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{
        GetLastError, COLORREF, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT,
        WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        CreateSolidBrush, SetBkMode, SetTextColor, HBRUSH, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, REG_VALUE_TYPE,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, SetFocus, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
        MOUSEINPUT, MOUSE_EVENT_FLAGS,
    };
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW,
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
    /// Whether clicks lock to the position captured when clicking
    /// started instead of following the cursor.
    static LOCK_POS: AtomicBool = AtomicBool::new(false);
    /// The captured lock point and whether one is set for this run.
    static LOCK_X: AtomicI32 = AtomicI32::new(0);
    static LOCK_Y: AtomicI32 = AtomicI32::new(0);
    static LOCK_SET: AtomicBool = AtomicBool::new(false);
    /// Auto-stop limit in clicks per run; 0 = no limit.
    static LIMIT_CLICKS: AtomicU32 = AtomicU32::new(0);
    /// Clicks sent in the current run (reset on every activation).
    static RUN_COUNT: AtomicU64 = AtomicU64::new(0);
    /// Whether clicking starts when TrayClick launches (click mode).
    static START_ON_LAUNCH: AtomicBool = AtomicBool::new(false);
    /// Whether the mouse button is currently held down (hold mode).
    /// Every stop path funnels through `refresh_indicator`, which calls
    /// `sync_hold_state` — so every stop path releases the button.
    static HELD: AtomicBool = AtomicBool::new(false);
    static MAIN_HWND: AtomicI32 = AtomicI32::new(0);
    /// True while a dialog window (Set Keybind or Set Interval) is open;
    /// the polling thread must not treat keys pressed there as toggles.
    static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);
    /// Set when the capture window closes: the polling thread resyncs its
    /// press detector, so the key still held from choosing it does not
    /// count as a press. Clicking starts only on the next fresh press.
    static RESYNC_DETECTOR: AtomicBool = AtomicBool::new(false);
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

    fn config_path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("TrayClick").join("config.json")
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
        }
    }

    fn current_mode() -> ClickMode {
        match MODE.load(Ordering::SeqCst) {
            1 => ClickMode::Hold,
            _ => ClickMode::Click,
        }
    }

    /// The live settings as a `Config`, for persisting after a change.
    fn current_config() -> Config {
        Config {
            keybind_vk: KEYBIND_VK.load(Ordering::SeqCst),
            interval_ms: INTERVAL_MS.load(Ordering::SeqCst),
            button: current_button(),
            jitter: JITTER_ON.load(Ordering::SeqCst),
            mode: current_mode(),
            lock_position: LOCK_POS.load(Ordering::SeqCst),
            limit_clicks: LIMIT_CLICKS.load(Ordering::SeqCst),
            start_on_launch: START_ON_LAUNCH.load(Ordering::SeqCst),
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

    /// Press (`down = true`) or release the chosen button once — the
    /// hold mode primitives.
    fn send_button(down: bool) {
        let (d, u) = button_flags(current_button());
        let inputs = [mouse_input(0, 0, if down { d } else { u })];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn send_click() {
        let (down, up) = button_flags(current_button());
        if LOCK_POS.load(Ordering::SeqCst) && LOCK_SET.load(Ordering::SeqCst) {
            send_click_locked(down, up);
        } else {
            let inputs = [mouse_input(0, 0, down), mouse_input(0, 0, up)];
            unsafe {
                SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            }
        }
        CLICK_COUNT.fetch_add(1, Ordering::Relaxed);
        RUN_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    /// Click at the locked point, then put the cursor back where the
    /// user left it — all in one SendInput batch, so the pointer only
    /// leaves for the instant of the click and the user can keep
    /// working elsewhere.
    fn send_click_locked(down: MOUSE_EVENT_FLAGS, up: MOUSE_EVENT_FLAGS) {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let abs_to = |x: i32, y: i32| {
                mouse_input(
                    absolute_coord(x, vx, vw) as i32,
                    absolute_coord(y, vy, vh) as i32,
                    MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                )
            };
            let target = abs_to(LOCK_X.load(Ordering::SeqCst), LOCK_Y.load(Ordering::SeqCst));
            let mut cur = POINT::default();
            if GetCursorPos(&mut cur).is_ok() {
                let back = abs_to(cur.x, cur.y);
                let inputs = [target, mouse_input(0, 0, down), mouse_input(0, 0, up), back];
                SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            } else {
                let inputs = [target, mouse_input(0, 0, down), mouse_input(0, 0, up)];
                SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            }
        }
    }

    /// Whether the current run has reached the auto-stop limit.
    fn limit_reached() -> bool {
        let limit = LIMIT_CLICKS.load(Ordering::SeqCst);
        limit > 0 && RUN_COUNT.load(Ordering::Relaxed) >= limit as u64
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

    /// Capture the cursor as the lock point for this run.
    fn capture_lock_point() {
        unsafe {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                LOCK_X.store(pt.x, Ordering::SeqCst);
                LOCK_Y.store(pt.y, Ordering::SeqCst);
                LOCK_SET.store(true, Ordering::SeqCst);
            }
        }
    }

    /// Release the button if it is held, unconditionally.
    fn sync_hold_state_released() {
        if HELD.swap(false, Ordering::SeqCst) {
            send_button(false);
        }
    }

    /// Keep the physical button in sync with the engine: in hold mode
    /// the button is down exactly while the engine is active. Called at
    /// the end of `refresh_indicator`, which every state change on the
    /// UI thread funnels through — so the keybind, Pause, the failsafe,
    /// the limit, opening a dialog, and Exit all release a held button.
    fn sync_hold_state() {
        let want = lock_engine().is_clicking_now() && current_mode() == ClickMode::Hold;
        let held = HELD.load(Ordering::SeqCst);
        if want && !held {
            send_button(true);
            HELD.store(true, Ordering::SeqCst);
        } else if !want && held {
            send_button(false);
            HELD.store(false, Ordering::SeqCst);
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

    /// The interval for the next cycle: the configured value, varied by
    /// up to ±10% when jitter is on.
    fn effective_interval(rng: &mut XorShift) -> Duration {
        let base = INTERVAL_MS.load(Ordering::SeqCst);
        let ms = if JITTER_ON.load(Ordering::SeqCst) {
            jittered_interval_ms(base, rng.next())
        } else {
            base
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

    /// Stop clicking because the failsafe tripped, and tell the UI
    /// thread so it can settle the tray indicator.
    fn trip_failsafe() {
        lock_engine().clicking = false;
        post_main(WM_APP_FAILSAFE);
    }

    /// Wait until `deadline`, waking early if clicking stops.
    ///
    /// The bulk is slept in short slices; the final ~2 ms are spin-waited
    /// because an OS sleep can overshoot by a whole timer tick (up to
    /// ~15 ms at the default Windows resolution) — at a 1 ms interval
    /// that overshoot would collapse the click rate. The spin runs only
    /// while clicking, and only for the last slice of each interval.
    fn wait_until(deadline: Instant) {
        // The failsafe check costs a `GetCursorPos` per call — cheap in
        // the sleep phase (iterations are milliseconds apart) but pure
        // overhead in the spin tail, which can loop thousands of times
        // in the last 2 ms. Throttle it to every 500 µs there: a mouse
        // slam takes far longer than that to reach the corner, so stop
        // latency is unchanged in practice.
        let mut last_failsafe_check = Instant::now() - Duration::from_millis(1);
        loop {
            let now = Instant::now();
            if now >= deadline || !lock_engine().is_clicking_now() {
                return;
            }
            if now.duration_since(last_failsafe_check) >= Duration::from_micros(500) {
                last_failsafe_check = now;
                if cursor_in_failsafe() {
                    trip_failsafe();
                    return;
                }
            }
            let remaining = deadline - now;
            if remaining > Duration::from_millis(2) {
                let slice = (remaining - Duration::from_millis(2)).min(Duration::from_millis(5));
                std::thread::sleep(slice);
            } else {
                std::hint::spin_loop();
            }
        }
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
        loop {
            if !lock_engine().is_clicking_now() {
                deadline = None;
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if current_mode() == ClickMode::Hold {
                // Nothing to schedule — the button is simply down. Just
                // keep watch for the failsafe.
                if cursor_in_failsafe() {
                    trip_failsafe();
                }
                deadline = None;
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if cursor_in_failsafe() {
                trip_failsafe();
                deadline = None;
                continue;
            }
            match deadline {
                None => {
                    send_click();
                    deadline = Some(Instant::now() + effective_interval(&mut rng));
                }
                Some(d) => {
                    if Instant::now() >= d {
                        send_click();
                        deadline = Some(next_click_deadline(
                            d,
                            effective_interval(&mut rng),
                            Instant::now(),
                        ));
                    } else {
                        wait_until(d);
                    }
                }
            }
            if limit_reached() {
                lock_engine().clicking = false;
                post_main(WM_APP_LIMIT);
                deadline = None;
            }
        }
    }

    fn keybind_thread() {
        let mut detector = KeyPressDetector::new();
        loop {
            std::thread::sleep(Duration::from_millis(8));
            if DIALOG_OPEN.load(Ordering::SeqCst) {
                continue;
            }
            if RESYNC_DETECTOR.swap(false, Ordering::SeqCst) {
                detector.resync();
            }
            let vk = KEYBIND_VK.load(Ordering::SeqCst) as i32;
            let down = unsafe { GetAsyncKeyState(vk) } & (0x8000u16 as i16) != 0;
            if detector.sample(down) {
                post_main(WM_APP_TOGGLE);
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
        let target = if LOCK_POS.load(Ordering::SeqCst) {
            "Locked position"
        } else {
            "Follow cursor"
        };
        let text = wide(&format!(
            "TrayClick {}\r\n\r\n\
             Created by SEAN (SeanShaffer88).\r\n\
             Built with Rust and the Windows API.\r\n\r\n\
             Keybind: {keybind} — start or stop auto clicking at the cursor.\r\n\
             Interval: {interval} ms (set it from the tray menu).\r\n\
             Button: {button}\r\n\
             Mode: {mode}\r\n\
             Target: {target}\r\n\
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
                if vk == VK_ESCAPE_CODE {
                    let _ = DestroyWindow(hwnd);
                } else if is_valid_keybind(vk) {
                    KEYBIND_VK.store(vk, Ordering::SeqCst);
                    save_config(current_config());
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
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn open_capture_window(parent: HWND) {
        if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
            return; // already open
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
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("TrayClickCapture"),
                w!("TrayClick — Set Keybind"),
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
            let current = key_name(KEYBIND_VK.load(Ordering::SeqCst));
            let label = wide(&format!(
                "Press a key to set the toggle keybind.\r\n\r\nCurrent: {current}\r\nEsc to cancel."
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
                    ID_IV_LAUNCH => apply_launch(hwnd),
                    // IDCANCEL (2) arrives when Esc is pressed via the
                    // dialog message loop.
                    2 => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
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

    fn pos_caption() -> &'static str {
        if LOCK_POS.load(Ordering::SeqCst) {
            "Position: Locked"
        } else {
            "Position: Follow"
        }
    }

    fn launch_caption() -> &'static str {
        if START_ON_LAUNCH.load(Ordering::SeqCst) {
            "Start on launch: On"
        } else {
            "Start on launch: Off"
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
                let t = wide(pos_caption());
                let _ = SetWindowTextW(pb, PCWSTR(t.as_ptr()));
            }
            if let Ok(lb) = GetDlgItem(Some(hwnd), ID_IV_LAUNCH as i32) {
                let t = wide(launch_caption());
                let _ = SetWindowTextW(lb, PCWSTR(t.as_ptr()));
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
            ClickMode::Hold => ClickMode::Click,
        };
        MODE.store(mode_u32(next), Ordering::SeqCst);
        save_config(current_config());
        relabel_interval_controls(hwnd);
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
    }

    fn apply_pos(hwnd: HWND) {
        let on = !LOCK_POS.load(Ordering::SeqCst);
        LOCK_POS.store(on, Ordering::SeqCst);
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
                    hbrBackground: CreateSolidBrush(COLORREF(0x00181818)),
                    lpszClassName: w!("TrayClickInterval"),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            }
            let width = 396;
            let height = 356;
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
            let label_style = WS_CHILD | WS_VISIBLE;
            let _ = child(
                w!("STATIC"),
                "Interval between clicks (milliseconds):",
                label_style,
                16,
                14,
                360,
                20,
                0,
            );
            let btn_style = WS_CHILD | WS_VISIBLE;
            let _ = child(w!("BUTTON"), "1 ms", btn_style, 16, 44, 80, 30, ID_IV_1MS);
            let _ = child(
                w!("BUTTON"),
                "100 ms",
                btn_style,
                106,
                44,
                80,
                30,
                ID_IV_100MS,
            );
            // WS_BORDER | ES_CENTER | ES_NUMBER on top of child/visible.
            let edit_style =
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0080_0000 | 0x0000_0001 | 0x0000_2000);
            let current = INTERVAL_MS.load(Ordering::SeqCst).to_string();
            let _ = child(
                w!("EDIT"),
                &current,
                edit_style,
                200,
                44,
                76,
                30,
                ID_IV_EDIT,
            );
            // BS_DEFPUSHBUTTON (0x1) so Enter applies via the dialog loop.
            let apply_style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0000_0001);
            let _ = child(
                w!("BUTTON"),
                "Apply",
                apply_style,
                290,
                44,
                74,
                30,
                ID_IV_APPLY,
            );
            let _ = child(
                w!("STATIC"),
                "Mouse button:",
                label_style,
                16,
                88,
                360,
                20,
                0,
            );
            let cur_btn = current_button();
            let left_cap = button_caption(ClickButton::Left, cur_btn);
            let _ = child(
                w!("BUTTON"),
                &left_cap,
                btn_style,
                16,
                112,
                80,
                30,
                ID_IV_BTN_LEFT,
            );
            let right_cap = button_caption(ClickButton::Right, cur_btn);
            let _ = child(
                w!("BUTTON"),
                &right_cap,
                btn_style,
                106,
                112,
                80,
                30,
                ID_IV_BTN_RIGHT,
            );
            let middle_cap = button_caption(ClickButton::Middle, cur_btn);
            let _ = child(
                w!("BUTTON"),
                &middle_cap,
                btn_style,
                196,
                112,
                80,
                30,
                ID_IV_BTN_MIDDLE,
            );
            let _ = child(
                w!("BUTTON"),
                jitter_caption(),
                btn_style,
                290,
                112,
                74,
                30,
                ID_IV_JITTER,
            );
            let _ = child(
                w!("STATIC"),
                "Mode and target:",
                label_style,
                16,
                156,
                360,
                20,
                0,
            );
            let mode_cap = mode_caption();
            let _ = child(
                w!("BUTTON"),
                &mode_cap,
                btn_style,
                16,
                180,
                150,
                30,
                ID_IV_MODE,
            );
            let _ = child(
                w!("BUTTON"),
                pos_caption(),
                btn_style,
                176,
                180,
                188,
                30,
                ID_IV_POS,
            );
            let _ = child(
                w!("STATIC"),
                "Stop after (clicks, 0 = no limit):",
                label_style,
                16,
                224,
                216,
                20,
                0,
            );
            let limit_now = LIMIT_CLICKS.load(Ordering::SeqCst).to_string();
            let _ = child(
                w!("EDIT"),
                &limit_now,
                edit_style,
                238,
                218,
                62,
                30,
                ID_IV_LIMIT_EDIT,
            );
            let _ = child(
                w!("BUTTON"),
                "Set",
                btn_style,
                308,
                218,
                56,
                30,
                ID_IV_LIMIT_SET,
            );
            let _ = child(
                w!("BUTTON"),
                launch_caption(),
                btn_style,
                16,
                256,
                348,
                30,
                ID_IV_LAUNCH,
            );
            let _ = child(
                w!("STATIC"),
                "Presets apply at once. Custom interval: type a value and click Apply (range 1–60000). Everything else applies at once. Esc cancels.",
                label_style,
                16,
                296,
                360,
                36,
                0,
            );
            let _ = SetFocus(Some(hwnd));
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
            WM_TRAYICON => {
                let event = lparam.0 as u32;
                if event == WM_LBUTTONUP || event == WM_RBUTTONUP {
                    show_menu(hwnd);
                }
                LRESULT(0)
            }
            m if m == WM_APP_TOGGLE => {
                lock_engine().press_keybind();
                if lock_engine().is_clicking_now() {
                    RUN_COUNT.store(0, Ordering::Relaxed);
                    if current_mode() == ClickMode::Click && LOCK_POS.load(Ordering::SeqCst) {
                        capture_lock_point();
                    }
                } else {
                    LOCK_SET.store(false, Ordering::SeqCst);
                }
                refresh_indicator(hwnd);
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
                        sync_hold_state_released();
                        tray_remove(hwnd);
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // Belt and braces: never exit with a button held down.
                sync_hold_state_released();
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    pub fn run() {
        // Single instance: a second copy would fight over the keybind.
        // The handle is held in this binding until `run` returns at
        // process exit. (In the `windows` crate `HANDLE` is a plain
        // `Copy` value with no `Drop`, so an earlier draft of this code
        // was reviewed as if the handle closed when its block ended —
        // it never did; keeping the binding simply makes the lifetime
        // explicit and guarantees nothing closes it early.)
        let instance_mutex = unsafe { CreateMutexW(None, true, w!("TrayClickSingleInstance")) };
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
        LOCK_POS.store(cfg.lock_position, Ordering::SeqCst);
        LIMIT_CLICKS.store(cfg.limit_clicks, Ordering::SeqCst);
        START_ON_LAUNCH.store(cfg.start_on_launch, Ordering::SeqCst);
        if cfg.start_on_launch && cfg.mode == ClickMode::Click {
            lock_engine().clicking = true;
            if cfg.lock_position {
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
            refresh_indicator(hwnd);

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
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            tray_remove(hwnd);
        }
    }
}
