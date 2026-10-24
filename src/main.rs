#![cfg_attr(windows, windows_subsystem = "windows")]

//! TrayClick — a Windows tray auto clicker.
//!
//! Lives entirely in the notification area. Left- or right-clicking the
//! tray icon opens a menu with exactly four items:
//!
//! - **Pause / Resume** — pause stops clicking and ignores the keybind
//!   until resumed.
//! - **Set Keybind** — shows the current keybind; opens a small window
//!   that captures the next key press as the new toggle key.
//! - **Credits** — about/credits dialog.
//! - **Exit** — removes the tray icon and quits.
//!
//! Pressing the keybind (default **F6**) toggles auto left-clicking at the
//! cursor position every 100 ms, indefinitely, until the keybind is
//! pressed again.
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
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use trayclick::{
        is_valid_keybind, key_name, Config, Engine, CLICK_INTERVAL_MS, VK_ESCAPE_CODE,
    };

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{
        GetLastError, COLORREF, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{CreateSolidBrush, SetBkMode, SetTextColor, TRANSPARENT};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, SetFocus, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEINPUT,
    };
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    /// Tray icon callback message.
    const WM_TRAYICON: u32 = WM_USER + 1;
    /// Posted by the keybind polling thread on a keybind press edge.
    const WM_APP_TOGGLE: u32 = WM_APP + 1;

    const TRAY_ID: u32 = 1;

    const ID_PAUSE_RESUME: usize = 1001;
    const ID_SET_KEYBIND: usize = 1002;
    const ID_CREDITS: usize = 1003;
    const ID_EXIT: usize = 1004;

    static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
    static KEYBIND_VK: AtomicU32 = AtomicU32::new(trayclick::DEFAULT_KEYBIND_VK);
    static MAIN_HWND: AtomicI32 = AtomicI32::new(0);
    /// True while the Set Keybind capture window is open; the polling
    /// thread must not treat the captured key as a toggle press.
    static CAPTURING: AtomicBool = AtomicBool::new(false);

    fn engine() -> &'static Mutex<Engine> {
        ENGINE.get_or_init(|| Mutex::new(Engine::new()))
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
        let _ = std::fs::write(&path, cfg.to_json());
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

    fn clicker_thread() {
        loop {
            let active = engine()
                .lock()
                .map(|e| e.is_clicking_now())
                .unwrap_or(false);
            if active {
                send_left_click();
                std::thread::sleep(Duration::from_millis(CLICK_INTERVAL_MS));
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn keybind_thread() {
        let mut was_down = false;
        loop {
            std::thread::sleep(Duration::from_millis(8));
            if CAPTURING.load(Ordering::SeqCst) {
                was_down = false;
                continue;
            }
            let vk = KEYBIND_VK.load(Ordering::SeqCst) as i32;
            let down = unsafe { GetAsyncKeyState(vk) } & (0x8000u16 as i16) != 0;
            if down && !was_down {
                let raw = MAIN_HWND.load(Ordering::SeqCst);
                if raw != 0 {
                    let hwnd = HWND(raw as isize as *mut c_void);
                    unsafe {
                        let _ = PostMessageW(Some(hwnd), WM_APP_TOGGLE, WPARAM(0), LPARAM(0));
                    }
                }
            }
            was_down = down;
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

    // ---------- tray menu ----------

    fn show_menu(hwnd: HWND) {
        unsafe {
            let menu = match CreatePopupMenu() {
                Ok(m) => m,
                Err(_) => return,
            };
            let (paused, vk) = {
                let e = engine().lock().unwrap();
                (e.paused, KEYBIND_VK.load(Ordering::SeqCst))
            };
            let pause_label = if paused { "Resume" } else { "Pause" };
            let pause_w = wide(pause_label);
            let keybind_label = format!("Set Keybind [{}]", key_name(vk));
            let keybind_w = wide(&keybind_label);
            let credits_w = wide("Credits");
            let exit_w = wide("Exit");
            let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE_RESUME, PCWSTR(pause_w.as_ptr()));
            let _ = AppendMenuW(menu, MF_STRING, ID_SET_KEYBIND, PCWSTR(keybind_w.as_ptr()));
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
        let text = wide(concat!(
            "TrayClick 0.1.0\r\n\r\n",
            "Created by SEAN (SeanShaffer88).\r\n",
            "Built with Rust and the Windows API.\r\n\r\n",
            "Press the keybind (default F6) to start or stop auto left-clicking ",
            "every 100 ms at the cursor.\r\n\r\n",
            "MIT License — see LICENSE."
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
                    save_config(Config { keybind_vk: vk });
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, COLORREF(0x00F2F2F2));
                SetBkMode(hdc, TRANSPARENT);
                LRESULT(CreateSolidBrush(COLORREF(0x00181818)).0 as isize)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                CAPTURING.store(false, Ordering::SeqCst);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn open_capture_window(parent: HWND) {
        if CAPTURING.swap(true, Ordering::SeqCst) {
            return; // already open
        }
        // Stop any clicking in progress so stray clicks can't hit the dialog.
        if let Ok(mut e) = engine().lock() {
            e.clicking = false;
        }
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
                    CAPTURING.store(false, Ordering::SeqCst);
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

    // ---------- main (hidden) window ----------

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TRAYICON => {
                let event = lparam.0 as u32;
                if event == WM_LBUTTONUP || event == WM_RBUTTONUP {
                    show_menu(hwnd);
                }
                LRESULT(0)
            }
            m if m == WM_APP_TOGGLE => {
                if let Ok(mut e) = engine().lock() {
                    e.press_keybind();
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                match wparam.0 & 0xFFFF {
                    ID_PAUSE_RESUME => {
                        if let Ok(mut e) = engine().lock() {
                            e.toggle_pause();
                        }
                    }
                    ID_SET_KEYBIND => open_capture_window(hwnd),
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
        unsafe {
            let _mutex = CreateMutexW(None, true, w!("TrayClickSingleInstance"));
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let text = wide("TrayClick is already running — check the tray.");
                let title = wide("TrayClick");
                MessageBoxW(
                    None,
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_OK | MB_ICONINFORMATION,
                );
                return;
            }
        }

        let cfg = load_config();
        KEYBIND_VK.store(cfg.keybind_vk, Ordering::SeqCst);

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
            tray_add(hwnd);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            tray_remove(hwnd);
        }
    }
}
