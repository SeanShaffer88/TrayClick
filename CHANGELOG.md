# Changelog

## 0.2.1 — 2026-10-24

- **Release downloads are now exactly two files:** an installer (`TrayClick-Setup-v0.2.1-x64.exe`, per-user Inno Setup install with Start menu shortcut and uninstaller; your settings in `%APPDATA%\TrayClick` are kept on uninstall) and a portable exe (`TrayClick-v0.2.1-portable.exe` — the same program, no install). The zip and SHA256SUMS.txt assets were dropped, and were also removed from the v0.1.0 and v0.2.0 releases.
- No program changes.

## 0.2.0 — 2026-10-24

- **New menu item: Set Interval [100 ms]** — opens a small window with **1 ms** and **100 ms** preset buttons plus a custom value in milliseconds (1–60000). The interval is remembered between runs (`interval_ms` in the config file; v0.1.x config files without it default to 100 ms).
- **Fix:** the keypress that assigns a new keybind no longer starts clicking. The press detector resyncs when the Set Keybind window closes, so clicking starts only on the next fresh press of the new keybind.
- Credits dialog now shows the live keybind and interval and takes its version from the package version.

## 0.1.0 — 2026-10-24

Initial version.

- Lives in the system tray with no main window; left- or right-clicking the tray icon opens the menu.
- Menu contains exactly: Pause/Resume, Set Keybind (shows the current key), Credits, Exit.
- Keybind (default F6) toggles auto left-clicking at the cursor every 100 ms, indefinitely, until pressed again.
- Pause stops clicking and blocks the keybind; Resume re-arms the keybind without restarting clicking on its own.
- Set Keybind captures the next key press in a small window (Esc cancels); the choice persists in `%APPDATA%\TrayClick\config.json`.
- Single-instance guard; works offline; clicks via `SendInput` at the live cursor position.
