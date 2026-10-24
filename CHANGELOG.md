# Changelog

## 0.1.0 — 2026-10-24

Initial version.

- Lives in the system tray with no main window; left- or right-clicking the tray icon opens the menu.
- Menu contains exactly: Pause/Resume, Set Keybind (shows the current key), Credits, Exit.
- Keybind (default F6) toggles auto left-clicking at the cursor every 100 ms, indefinitely, until pressed again.
- Pause stops clicking and blocks the keybind; Resume re-arms the keybind without restarting clicking on its own.
- Set Keybind captures the next key press in a small window (Esc cancels); the choice persists in `%APPDATA%\TrayClick\config.json`.
- Single-instance guard; works offline; clicks via `SendInput` at the live cursor position.
