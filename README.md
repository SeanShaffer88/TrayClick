# TrayClick

## TL;DR

A tiny Windows tray auto clicker. TrayClick sits in the notification area with no main window. Press the keybind (default **F6**) and it clicks at your cursor at the set interval (default **100 ms**), indefinitely, until you press the keybind again. Left- or right-click the tray icon for a menu with exactly six options: **Pause/Resume**, **Set Keybind**, **Set Interval**, **Start with Windows**, **Credits**, and **Exit**. That's the whole program.

- Windows 10 or later, x64
- One small executable, no installer, nothing to configure beyond the keybind and interval
- The keybind and interval you pick are remembered between runs (`%APPDATA%\TrayClick\config.json`)

## Download

Every [release](https://github.com/SeanShaffer88/TrayClick/releases) provides exactly two files:

- **Installer** — `TrayClick-Setup-v<version>-x64.exe`: per-user install (no admin), Start menu shortcut, uninstaller in Windows Settings. Your settings survive uninstall; they're in `%APPDATA%\TrayClick`.
- **Portable exe** — `TrayClick-v<version>-portable.exe`: the same program as a single file, no install — just run it.

The builds are unsigned, so Windows SmartScreen may warn on first run: "More info" → "Run anyway".

## How to use

1. Run TrayClick (installed shortcut, or the portable exe). It appears in the system tray (notification area) — you may need the `^` overflow arrow the first time.
2. Press **F6** to start auto-clicking at the cursor. Press **F6** again to stop.
3. Click the tray icon (left or right click — both work) for the menu:
   - **Pause** — stops clicking immediately and ignores the keybind. The item becomes **Resume**; resuming does not restart clicking until you press the keybind again.
   - **Set Keybind [F6]** — opens a small window; press any key to make it the new toggle key (`Esc` cancels). Mouse buttons can't be assigned, for obvious reasons. Assigning a key never starts clicking by itself — clicking starts when you press the new keybind again.
   - **Set Interval [100 ms]** — opens the settings window: **1 ms** and **100 ms** preset buttons or a custom interval (1–60000), the **mouse button** (left, right, or middle), **jitter** (varies each interval by up to ±10%), the **mode** (Click, or Hold — the keybind holds the button down until pressed again), the **target** (Follow the cursor, or Locked to where clicking started), an **auto-stop limit** (stop after N clicks per run; 0 = no limit), and **start on launch** (clicking begins when TrayClick starts, click mode only).
   - **Start with Windows** — checked when TrayClick is set to launch at sign-in (per-user, no admin). Uncheck to stop launching. If you use the portable exe, the entry points at wherever the exe is, so moving it means toggling this once more.
   - **Credits** — version, current settings, and how many clicks this session has fired.
   - **Exit** — removes the tray icon and quits.

While clicking is on in the default Follow target, clicking follows your cursor — it clicks wherever the pointer currently is, at the set interval, including if you move it. With the **Locked** target, each click jumps to the spot where clicking started, clicks, and puts your cursor back where you left it, so you can keep working elsewhere. In **Hold** mode the keybind holds the button down instead — and every way of stopping (keybind, Pause, a dialog, the failsafe, Exit) releases it. You can always tell it's on: **the tray icon blinks** (alternating with an inverted red icon) and its tooltip shows the click count or the held button.

**Safety:** while clicking is on, slam the mouse into the **top-left corner of the screen** to stop clicking instantly — handy at fast intervals, before you reach for the keybind.

## Notes

- Only one copy runs at a time; starting a second copy just tells you it's already running.
- TrayClick clicks with the standard Windows `SendInput` API at the current cursor position. It does not move the cursor, record anything, or phone home. It works completely offline.
- The click interval defaults to 100 ms and can be set to 1 ms, 100 ms, or any custom value from 1 to 60000 ms in the Set Interval window. Very small intervals click as fast as Windows can deliver them; the effective rate is bounded by the OS timer and message delivery.
- Settings — keybind, interval, mouse button, jitter, mode, target, auto-stop limit, and start-on-launch — are saved in `%APPDATA%\TrayClick\config.json` and survive restarts.

## Screenshots

The tray menu (captured on v0.3.1; v0.4.0 adds a sixth item, **Start with Windows**), with the current keybind and interval shown in brackets:

![TrayClick tray menu](docs/screenshots/tray-menu.png)

The settings window (captured on v0.3.1; it has grown since — 0.4.0 added the mouse-button row and jitter toggle, 0.5.0 the mode/target row, the auto-stop limit, and start-on-launch):

![TrayClick Set Interval window](docs/screenshots/set-interval.png)

More captures from a real run are on the way; see [docs/screenshots/CAPTURE.md](docs/screenshots/CAPTURE.md) for the exact shots still needed.

## Build from source

```powershell
cargo build --release
cargo test
```

Requires a current stable Rust toolchain on Windows. The icon in `assets/icon.ico` is embedded as a Windows resource at build time; regenerate it with `python3 assets/make_icon.py` (needs Pillow).

## License

MIT — see [LICENSE](LICENSE).
