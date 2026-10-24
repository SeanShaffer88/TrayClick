# Changelog

## 0.3.1 — 2026-10-24

- **Polish:** the OEM punctuation keys now show by name in the menu and Credits instead of a hex code — `\` (which showed as "Key 0xDC"), `/`, `;`, `'`, `[`, `]`, `,`, `.`, `-`, `=`, and `` ` ``. Prompted by a field screenshot: SEAN's own keybind is the `\` key, and the menu called it "Key 0xDC".
- **Installer:** the "Launch TrayClick now" checkbox on the installer's final screen is now checked by default (it was opt-in before).

## 0.3.0 — 2026-10-24

- **New: you can now see when auto-clicking is on.** While clicking is active, the tray icon blinks — alternating every 500 ms between the normal icon and an inverted red "active" icon — and its tooltip reads "TrayClick — clicking ON". The moment clicking stops (keybind, Pause, or opening a settings window), the normal icon returns. No more guessing from the cursor alone.

## 0.2.2 — 2026-10-24

Internal review and optimization pass — no intended behaviour changes, four reliability fixes and a smaller binary:

- **Fix:** GDI object leak — the Set Keybind / Set Interval windows created a new background brush on every repaint and never released it. They now share one brush created once.
- **Fix:** click timing no longer drifts. Clicks are scheduled on absolute deadlines, so the time a click itself takes is no longer added to every interval, and the wait is precise to ~2 ms instead of overshooting by up to a Windows timer tick (~15 ms) — small intervals (like the 1 ms preset) now track the setting far more closely. After the PC sleeps, clicking resumes on schedule instead of firing catch-up clicks.
- **Fix:** the settings file is written atomically (temp file + rename), so a crash mid-save can no longer leave a torn config that resets your settings on the next launch.
- **Fix:** if Explorer restarts, the tray icon is re-added automatically instead of leaving TrayClick running invisibly.
- **Robustness:** the shared state lock now recovers instead of panicking (or silently ignoring the keybind) if it was ever poisoned.
- **Smaller download:** the release build now uses LTO, symbol stripping and size optimization, so the portable exe is noticeably smaller than 0.2.1's 217,088 bytes.

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
