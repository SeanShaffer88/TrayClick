# TrayClick

## TL;DR

A tiny Windows tray auto clicker. TrayClick sits in the notification area with no main window. Press the keybind (default **F6**) and it left-clicks at your cursor every **100 ms**, indefinitely, until you press the keybind again. Left- or right-click the tray icon for a menu with exactly four options: **Pause/Resume**, **Set Keybind**, **Credits**, and **Exit**. That's the whole program.

- Windows 10 or later, x64
- One small executable, no installer, nothing to configure beyond the keybind
- The keybind you pick is remembered between runs (`%APPDATA%\TrayClick\config.json`)

## How to use

1. Run `trayclick.exe`. It appears in the system tray (notification area) — you may need the `^` overflow arrow the first time.
2. Press **F6** to start auto-clicking at the cursor. Press **F6** again to stop.
3. Click the tray icon (left or right click — both work) for the menu:
   - **Pause** — stops clicking immediately and ignores the keybind. The item becomes **Resume**; resuming does not restart clicking until you press the keybind again.
   - **Set Keybind [F6]** — opens a small window; press any key to make it the new toggle key (`Esc` cancels). Mouse buttons can't be assigned, for obvious reasons.
   - **Credits** — version and credits.
   - **Exit** — removes the tray icon and quits.

While clicking is on, clicking follows your cursor — it clicks wherever the pointer currently is, every 100 ms, including if you move it.

## Notes

- Only one copy runs at a time; starting a second copy just tells you it's already running.
- TrayClick clicks with the standard Windows `SendInput` API at the current cursor position. It does not move the cursor, record anything, or phone home. It works completely offline.
- The click interval is fixed at 100 ms by design — this program's whole point is doing one thing simply.

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
