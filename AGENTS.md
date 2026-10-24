# AGENTS.md — TrayClick

TrayClick is governed by SEAN's Universal Windows Desktop Master Prompt
(`~/workspace/your_files/windows-desktop-master-prompt.md`), which applies to
all of his Windows desktop projects. Standing defaults: Rust, black/red
family look where a surface exists, Windows 10+ when feasible, MIT,
README opens with `## TL;DR` and uses only real screenshots from SEAN's
machine, no competitor mentions in public docs.

Project invariants — do not change without SEAN's direction:

- The tray menu has exactly four items: Pause/Resume, Set Keybind, Credits, Exit. Both left and right click on the tray icon open it.
- The keybind toggles clicking; while on, the mouse left-clicks at the cursor every 100 ms, indefinitely, until the keybind is pressed again. The interval is fixed.
- Core state machine and config logic live in `src/lib.rs` (platform-independent, unit-tested). `src/main.rs` is Win32 only.
- Gate before publishing: `cargo fmt --all`, `cargo clippy --all-targets`, full `cargo test`, and a Windows-target check/build.
