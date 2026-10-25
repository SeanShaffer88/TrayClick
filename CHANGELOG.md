# Changelog

## 0.8.0 — 2026-10-25

Slice 3 of the 50-enhancement program (docs/PROGRAM-50.md): targeting. The settings window is now paged — **Clicking**, **Action**, and **Targeting** tabs (it remembers which tab you left it on) — and the Target control cycles three ways: Follow, Locked, Sequence.

- **Crosshair picker (item 17).** "Pick lock point" (Targeting tab) closes the settings window and covers the screen: one click sets a **persistent** lock point that survives restarts. The old behaviour — a Locked run with no picked point locks to wherever clicking started — remains as the fallback.
- **Multi-point sequence (item 10).** With the Sequence target, clicks (and scrolls) rotate through up to 4 points, one per cycle. "Set sequence points" uses the same crosshair overlay: click each point, right-click to finish early, Esc cancels.
- **Named target slots (item 11).** Three slots on the Targeting tab: type a name, **Save** stores the lock point (or, if none is set, the cursor), **Use** makes that point the lock point and switches the target to Locked.
- **Click offset (item 12).** Shift every click by X/Y pixels (−200 to 200 per axis) from the cursor or the lock point, for targets whose real hot spot sits off the pointer.
- **Pixel check (item 13).** Sample a colour (read at the lock point, or the cursor), set a tolerance (0–64 per channel), and turn the check on: cycles then act only while the pixel at the click point still matches. Blocked cycles are skipped silently and do not count.
- **Region-change stop (item 14).** The region guard samples an 8×8 patch of pixels around the click point when a run starts and stops the run when that patch visibly changes — a "stop when the screen changes" failsafe next to the corner one. In Follow mode the patch anchors where the cursor was when the run started.
- **Window-relative lock (item 15).** With this on, picking a lock point also remembers the window under it; the point then follows that window when it moves (same offset from its top-left corner). If the window closes, the absolute point is used. The window itself is matched live and never saved.
- **Other-monitor preset (item 16).** "Other monitor centre" sets the lock point to the centre of your other monitor and switches the target to Locked. (With one monitor it just says so.)

Two quieter notes: the pixel check and region guard apply to the click and scroll actions (Key and Drag cycles have no target pixel to watch), and the scroll-amount box now accepts typing a minus sign (it previously used a digits-only style).

Config files from every earlier version load unchanged — the target defaults to Follow (or Locked when the old lock flag was set), and everything new starts unset/off.

## 0.7.0 — 2026-10-25

Slice 2 of the 50-enhancement program (docs/PROGRAM-50.md): three more actions, plus interval shaping.

- **Scroll action (item 2).** The Action cycle now continues past Burst to **Scroll**: each cycle sends mouse-wheel ticks (−10 to +10 notches; negative scrolls down) at the cursor — or, with a locked target, darts to the lock point, scrolls there, and darts back.
- **Key action (item 3).** **Key** repeats a keystroke instead of clicking. The "Repeat key" button in the settings window opens the same capture window as Set Keybind (default Space); the button-down time doubles as the key-down duration, and extended keys (arrows, Home, Delete, …) are synthesized correctly.
- **Drag action (item 4).** **Drag** presses the chosen button at point A, glides to point B over ~50 ms, pauses for the button-down time, and releases. "Set drag points" closes the settings window and covers the screen with a dark overlay: click A, click B, done (Esc cancels). A drag always completes its release once started. Until points are set, the Drag action does nothing.
- **Random interval range (item 6).** With Random on, each cycle's interval is picked uniformly between the base interval and "Max ms". (If the max is unset or below the base, the base is used.)
- **Speed ramp (item 8).** With Ramp on, the interval eases linearly from the base to "Ramp to ms" over the chosen number of seconds at the start of every run — wind a clicker up or down smoothly. Ramp and Random don't combine (ramp wins); jitter still applies on top of either.

One behaviour clarification: Hold mode applies to the mouse-click actions (Single/Double/Burst), as it always has in effect. With a Scroll, Key, or Drag action selected, both modes repeat cycles on the interval — there is no button to "hold" in those actions. The click counter counts every action sent (each scroll cycle, key press, and drag counts once).

Config files from every earlier version load unchanged — scroll defaults to +1 notch, the repeat key to Space, drag points unset, random and ramp off.

## 0.6.0 — 2026-10-25

Slice 1 of the 50-enhancement program (docs/PROGRAM-50.md): the DPI fix plus the click-action foundation.

- **Per-monitor DPI awareness (item 46).** TrayClick now declares PerMonitorV2 awareness at startup. On scaled displays (125%, 150%, …) Windows otherwise virtualizes coordinates, which could put locked-position clicks a few pixels off target; they now land exactly.
- **Click actions (items 1, 5).** A new Action setting in the Set Interval window: **Single** (as always), **Double** (each cycle sends a double-click), or **Burst** (each cycle sends several clicks — 1 to 100, spaced by a configurable gap of 1–1000 ms). A burst stops early if you toggle clicking off, trip the corner failsafe, or hit the auto-stop limit; the limit counts physical clicks, so a burst of 5 counts 5.
- **Button-down time (item 7).** "Down" in the numbers row: each click can hold its button down for 0–1000 ms before releasing (0 = instant, as before) — for games and apps that miss very short clicks.
- **Alternating buttons (item 9).** Consecutive clicks alternate between button partners: Left ↔ Right, Middle pairs with Left.

Config files from every earlier version load unchanged — action defaults to Single, burst to 3 clicks at a 50 ms gap, down time to 0, alternation off.

## 0.5.1 — 2026-10-24

Internal review and optimization pass (the second senior review, after 0.2.2). No features added or removed, and no intended behaviour changes:

- **Cheaper failsafe watching.** The click wait loop checked the cursor position on every spin iteration — thousands of redundant `GetCursorPos` calls in the final 2 ms of each interval. The check is now throttled to every 500 µs in that phase (a mouse slam takes far longer to reach the corner, so stop latency is unchanged in practice), and the virtual-screen origin is read once instead of on every check. One nuance: if you hot-plug a monitor mid-run, the failsafe corner follows the layout as it was when first read until TrayClick restarts.
- **Deduplicated internals.** The keybind thread now posts through the same helper as the other worker threads, and the config parser's three optional booleans share one code path (with a new test pinning that a garbled one still rejects the file).
- **Refreshed stale descriptions** (the crate description and a module comment still described the 0.1/0.3 feature set).

Everything else the review re-checked came back clean and is unchanged: the hold-release funnel, the atomic config save, the icon/brush handle lifetimes, and the size-tuned release profile.

## 0.5.0 — 2026-10-24

Four more additions, SEAN's second suggestion round:

- **New: locked-position clicking.** A new Position toggle in the Set Interval window: Follow (the cursor, as always) or Locked — clicking then stays on the spot where you pressed the keybind. Each click darts to the locked point, clicks, and puts your cursor back where you left it, all in one input batch, so you can keep working elsewhere while it clicks. Ideal for a second monitor.
- **New: Hold mode.** Mode: Click or Hold. In Hold, the keybind presses the chosen button down and keeps it down until you press the keybind again. Every stop path releases it — Pause, the corner failsafe, opening a settings window, and Exit — so the button can never be left stuck down.
- **New: auto-stop limit.** "Stop after (clicks, 0 = no limit)" in the settings window: clicking stops by itself after that many clicks in a run (up to 100,000,000). Pairs with Start with Windows and start-on-launch for unattended runs that can't run away.
- **New: start clicking on launch.** An opt-in toggle: when TrayClick starts, clicking starts with it. Click mode only, off by default.

Config files from every earlier version load unchanged — mode defaults to Click, target to Follow, limit to 0 (none), start-on-launch off.

## 0.4.0 — 2026-10-24

Five additions, all from SEAN's suggestion round:

- **New: corner failsafe.** While clicking is on, slamming the mouse into the top-left corner of the screen stops clicking immediately. At a 1 ms interval the clicker can do damage fast if the pointer lands somewhere bad; this is the fastest possible stop, no keybind hunt required.
- **New: Start with Windows.** A checkable sixth menu item toggles the per-user Run-key entry (no admin). With the portable exe the entry points at the exe's current location — move the exe, toggle it once, done.
- **New: choose which mouse button clicks.** Left, right, or middle, chosen in the Set Interval window (which keeps its name and its menu slot), saved in the config like everything else.
- **New: click counter.** The tooltip while clicking reads "TrayClick — clicking ON · N clicks", and Credits shows the session total.
- **New: interval jitter.** An optional toggle in the Set Interval window varies each interval by up to ±10%, so the cadence isn't metronome-perfect. Off by default.

The tray menu is now exactly six items (this amends the five-item rule at SEAN's direction). Config files from every earlier version load unchanged — the new fields default to left button, jitter off.

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
