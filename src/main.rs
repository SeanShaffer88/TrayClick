#![cfg_attr(windows, windows_subsystem = "windows")]

//! TrayClick — a Windows tray auto clicker.
//!
//! Lives entirely in the notification area. Left- or right-clicking the
//! tray icon opens a menu with exactly five items:
//!
//! - **Pause / Resume** — pause stops clicking and ignores the keybind
//!   until resumed.
//! - **Set Keybind** — shows the current keybind; opens a small window
//!   that captures the next key press as the new toggle key. The press
//!   that assigns the key never starts clicking; clicking starts on the
//!   next fresh press.
//! - **Set Interval** — shows the current interval; opens a small window
//!   with 1 ms and 100 ms presets plus a custom value in milliseconds.
//! - **Credits** — about/credits dialog.
//! - **Exit** — removes the tray icon and quits.
//!
//! Pressing the keybind (default **F6**) toggles auto left-clicking at the
//! cursor position at the configured interval (default 100 ms),
//! indefinitely, until the keybind is pressed again. While clicking is
//! on, the tray icon blinks (alternating with an inverted red icon every
//! 500 ms) and its tooltip reads "TrayClick — clicking ON", so the state
//! is visible at a glance.
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
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::{Duration, Instant};

    use trayclick::{
        ico_image_for_size, is_valid_interval, is_valid_keybind, key_name, next_click_deadline,
        Config, Engine, KeyPressDetector, MAX_INTERVAL_MS, MIN_INTERVAL_MS, VK_ESCAPE_CODE,
    };

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{
        GetLastError, COLORREF, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        CreateSolidBrush, SetBkMode, SetTextColor, HBRUSH, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, SetFocus, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEINPUT,
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

    const TRAY_ID: u32 = 1;

    /// Timer that blinks the tray icon while clicking is on.
    const ID_BLINK_TIMER: usize = 1;
    const BLINK_MS: u32 = 500;

    const ID_PAUSE_RESUME: usize = 1001;
    const ID_SET_KEYBIND: usize = 1002;
    const ID_SET_INTERVAL: usize = 1003;
    const ID_CREDITS: usize = 1004;
    const ID_EXIT: usize = 1005;

    // Control IDs inside the Set Interval window.
    const ID_IV_1MS: usize = 2001;
    const ID_IV_100MS: usize = 2002;
    const ID_IV_APPLY: usize = 2003;
    const ID_IV_EDIT: usize = 2004;

    static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
    static KEYBIND_VK: AtomicU32 = AtomicU32::new(trayclick::DEFAULT_KEYBIND_VK);
    static INTERVAL_MS: AtomicU32 = AtomicU32::new(trayclick::DEFAULT_INTERVAL_MS);
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

    /// The live settings as a `Config`, for persisting after a change.
    fn current_config() -> Config {
        Config {
            keybind_vk: KEYBIND_VK.load(Ordering::SeqCst),
            interval_ms: INTERVAL_MS.load(Ordering::SeqCst),
        }
    }

    fn apply_interval(ms: u32) {
        INTERVAL_MS.store(ms, Ordering::SeqCst);
        save_config(current_config());
    }

    // ---------- click + keybind worker threads ----------

    fn send_left_click() {
        let inputs = [
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTDOWN,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: 0,
                        dwFlags: MOUSEEVENTF_LEFTUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            },
        ];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn interval_dur() -> Duration {
        Duration::from_millis(INTERVAL_MS.load(Ordering::SeqCst) as u64)
    }

    /// Wait until `deadline`, waking early if clicking stops.
    ///
    /// The bulk is slept in short slices; the final ~2 ms are spin-waited
    /// because an OS sleep can overshoot by a whole timer tick (up to
    /// ~15 ms at the default Windows resolution) — at a 1 ms interval
    /// that overshoot would collapse the click rate. The spin runs only
    /// while clicking, and only for the last slice of each interval.
    fn wait_until(deadline: Instant) {
        loop {
            let now = Instant::now();
            if now >= deadline || !lock_engine().is_clicking_now() {
                return;
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
        let mut deadline: Option<Instant> = None;
        loop {
            if !lock_engine().is_clicking_now() {
                deadline = None;
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            match deadline {
                None => {
                    send_left_click();
                    deadline = Some(Instant::now() + interval_dur());
                }
                Some(d) => {
                    if Instant::now() >= d {
                        send_left_click();
                        deadline = Some(next_click_deadline(d, interval_dur(), Instant::now()));
                    } else {
                        wait_until(d);
                    }
                }
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
                let raw = MAIN_HWND.load(Ordering::SeqCst);
                if raw != 0 {
                    let hwnd = HWND(raw as isize as *mut c_void);
                    unsafe {
                        let _ = PostMessageW(Some(hwnd), WM_APP_TOGGLE, WPARAM(0), LPARAM(0));
                    }
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

    /// Reflect the engine state in the tray. While clicking is on, the
    /// icon blinks between the normal and active icons (see the
    /// `WM_TIMER` handler) and the tooltip says so; when off, the timer
    /// stops and the normal icon is restored. Called after every engine
    /// state change, which all happen on the UI thread.
    fn refresh_indicator(hwnd: HWND) {
        if lock_engine().is_clicking_now() {
            BLINK_NORMAL_SHOWN.store(false, Ordering::SeqCst);
            tray_modify(hwnd, active_icon(), "TrayClick — clicking ON");
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
        let text = wide(&format!(
            "TrayClick {}\r\n\r\n\
             Created by SEAN (SeanShaffer88).\r\n\
             Built with Rust and the Windows API.\r\n\r\n\
             Keybind: {keybind} — start or stop auto left-clicking at the cursor.\r\n\
             Interval: {interval} ms (set it from the tray menu).\r\n\r\n\
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
            let height = 176;
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
                "Presets apply at once. Custom: type a value and click Apply (range 1–60000). Esc cancels.",
                label_style,
                16,
                88,
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
            WM_TRAYICON => {
                let event = lparam.0 as u32;
                if event == WM_LBUTTONUP || event == WM_RBUTTONUP {
                    show_menu(hwnd);
                }
                LRESULT(0)
            }
            m if m == WM_APP_TOGGLE => {
                lock_engine().press_keybind();
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
                    tray_modify(hwnd, icon, "TrayClick — clicking ON");
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
                    ID_CREDITS => show_credits(hwnd),
                    ID_EXIT => {
                        tray_remove(hwnd);
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
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
