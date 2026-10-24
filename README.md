# TrayClick

## TL;DR

A tiny Windows tray auto clicker. TrayClick sits in the notification area with no main window. Press the keybind (default **F6**) and it left-clicks at your cursor at the set interval (default **100 ms**), indefinitely, until you press the keybind again. Left- or right-click the tray icon for a menu with exactly five options: **Pause/Resume**, **Set Keybind**, **Set Interval**, **Credits**, and **Exit**. That's the whole program.

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
   - **Set Interval [100 ms]** — opens a small window with **1 ms** and **100 ms** preset buttons, or type any custom interval in milliseconds (1–60000) and click Apply.
   - **Credits** — version and credits.
   - **Exit** — removes the tray icon and quits.

While clicking is on, clicking follows your cursor — it clicks wherever the pointer currently is, at the set interval, including if you move it. You can always tell it's on: **the tray icon blinks** (alternating with an inverted red icon) and its tooltip reads "TrayClick — clicking ON".

## Notes

- Only one copy runs at a time; starting a second copy just tells you it's already running.
- TrayClick clicks with the standard Windows `SendInput` API at the current cursor position. It does not move the cursor, record anything, or phone home. It works completely offline.
- The click interval defaults to 100 ms and can be set to 1 ms, 100 ms, or any custom value from 1 to 60000 ms in the Set Interval window. Very small intervals click as fast as Windows can deliver them; the effective rate is bounded by the OS timer and message delivery.

## Screenshots

Captures from a real run will go here. See [docs/screenshots/CAPTURE.md](docs/screenshots/CAPTURE.md) for the exact shots needed.

## Build from source

```powershell
cargo build --release
cargo test
```

Requires a current stable Rust toolchain on Windows. The icon in `assets/icon.ico` is embedded as a Windows resource at build time; regenerate it with `python3 assets/make_icon.py` (needs Pillow).

## License

MIT — see [LICENSE](LICENSE).
