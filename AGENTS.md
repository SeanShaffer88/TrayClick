# AGENTS.md — TrayClick

TrayClick is governed by SEAN's Universal Windows Desktop Master Prompt
(`~/workspace/your_files/windows-desktop-master-prompt.md`), which applies to
all of his Windows desktop projects. Standing defaults: Rust, black/red
family look where a surface exists, Windows 10+ when feasible, MIT,
README opens with `## TL;DR` and uses only real screenshots from SEAN's
machine, no competitor mentions in public docs.

Project invariants — do not change without SEAN's direction:

- The tray menu has exactly six items: Pause/Resume, Set Keybind, Set Interval, Start with Windows, Credits, Exit. (Five until v0.3.1; SEAN directed adding Start with Windows in 0.4.0.) Both left and right click on the tray icon open it.
- The keybind toggles clicking; while on, the chosen mouse button (left/right/middle, default left) clicks at the cursor at the configured interval, indefinitely, until the keybind is pressed again. The interval defaults to 100 ms and is settable to 1 ms, 100 ms, or a custom 1–60000 ms in the Set Interval window; interval, button, and optional jitter (±10%) persist in the config file. Older config files must keep loading (missing fields default).
- The keypress that assigns a new keybind must never start clicking (the press detector resyncs when the capture window closes); clicking starts only on the next fresh press.
- Core state machine and config logic live in `src/lib.rs` (platform-independent, unit-tested). `src/main.rs` is Win32 only.
- The corner failsafe (stop clicking when the cursor enters the top-left corner zone while clicking) and the click counter (tooltip + Credits) are directed 0.4.0 features; do not remove them as "extras". Start with Windows uses the HKCU Run key only — never HKLM, never a scheduled task.
- Directed 0.5.0 features, same standing: Hold mode (every stop path must release the held button — the release funnels through `refresh_indicator` → `sync_hold_state`), Locked target (clicks go to the point captured at activation, cursor restored after each click), the per-run auto-stop limit, and start-on-launch (click mode only, defaults off).
- The 50-enhancement program (adopted 2026-10-25, tracker `docs/PROGRAM-50.md`) runs in gated slices; Slice 1 (0.6.0) directed features, same standing: PerMonitorV2 DPI awareness (set first thing in `run()`, before any window exists), click actions (Single/Double/Burst via `perform_cycle`; the auto-stop limit counts physical clicks, and a burst must stop early on toggle-off, failsafe, or limit), per-click button-down time, and alternating buttons (`effective_button` in lib.rs). Slice 2 (0.7.0), same standing: Scroll/Key/Drag actions (Hold mode applies ONLY to click actions — `ClickAction::is_click_action` gates both the clicker's hold branch and `sync_hold_state`; a started drag always completes its release; drag points are captured on the full-screen overlay, never typed), random interval range + speed ramp in `effective_interval` (ramp wins over random; jitter applies last).
- Gate before publishing: `cargo fmt --all`, `cargo clippy --all-targets`, full `cargo test`, and a Windows-target check/build.
