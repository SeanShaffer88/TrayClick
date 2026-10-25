# TrayClick — 50-Enhancement Program

Adopted by SEAN 2026-10-25 ("ok" to Muse's suggested 50, in the
recommended order). Executed in gated slices, PortableForge-style: each
slice is implemented, fully gated (fmt, clippy host + windows-msvc
`-D warnings`, FULL test suite), published per-file with a blob audit,
and released only on green CI. Runtime behaviour stays labelled
Unverified until SEAN's on-machine checks.

Standing constraints for every slice:

- The tray menu stays at exactly six items; new controls live in the
  settings window (or behind CLI flags), never as new menu items.
- The config file stays backward compatible: every new field defaults
  to the pre-program behaviour when absent; a garbled field rejects
  the file (caller falls back to defaults), never a partial load.
- Binaries stay unsigned, offline, dependency-free (windows crate only).

## Slice plan

| Slice | Release | Items | Theme |
|---|---|---|---|
| 1 | v0.6.0 | 46, 1, 5, 7, 9 | DPI fix + click-action foundation |
| 2 | v0.7.0 | 2, 3, 4, 6, 8 | Scroll / keyboard / drag actions, random range, speed ramp |
| 3 | v0.8.0 | 10–17 | Targeting (settings window gains pages here) |
| 4 | v0.9.0 | 18–25 | Triggers & automation (incl. CLI control) |
| 5 | v0.10.0 | 26–31 | Safety & limits |
| 6 | v0.11.0 | 32–38 | Feedback & visibility (stats storage lands here) |
| 7 | v1.0.0 | 39–45, 47–50 | Profiles, data portability, ARM64, polish |

## The 50

### Click power
1. Double-click mode — each cycle sends a double-click. *(Slice 1)*
2. Scroll mode — repeat wheel ticks (direction + notches). *(Slice 2)*
3. Keyboard mode — repeat a chosen keystroke. *(Slice 2)*
4. Drag mode — press at A, move to B, release per cycle. *(Slice 2)*
5. Burst clicking — N clicks per trigger, configurable gap. *(Slice 1)*
6. Random interval range — each cycle random within min–max ms. *(Slice 2)*
7. Hold-duration control — ms the button stays down per click. *(Slice 1)*
8. Speed ramp — interval eases start→end over the run. *(Slice 2)*
9. Alternating buttons — cycles alternate a button pair. *(Slice 1)*

### Targeting
10. Multi-point sequence — up to 4 lock points, rotated. *(Slice 3)*
11. Named target slots — 3 saved lock points. *(Slice 3)*
12. Cursor offset — click X/Y px away from cursor/lock. *(Slice 3)*
13. Pixel check — click only while target pixel matches a colour. *(Slice 3)*
14. Region-change stop — stop when target region changes. *(Slice 3)*
15. Window-relative lock — survive the target window moving. *(Slice 3)*
16. Other-monitor quick lock preset. *(Slice 3)*
17. Crosshair picker — next click anywhere becomes the lock point. *(Slice 3)*

### Triggers & automation
18. Momentary mode — click only while keybind held. *(Slice 4)*
19. Mouse side buttons (X1/X2) allowed as keybind. *(Slice 4)*
20. Separate stop-only key. *(Slice 4)*
21. RegisterHotKey option (near-zero idle CPU; polling stays default). *(Slice 4)*
22. Delayed start — start in N minutes. *(Slice 4)*
23. Foreground-app rule — run only while a chosen app is foreground. *(Slice 4)*
24. Fullscreen auto-pause. *(Slice 4)*
25. Command-line control — --start/--stop/--toggle/--profile forwarded to the running instance. *(Slice 4)*

### Safety & limits
26. Time limit per run (minutes). *(Slice 5)*
27. Mouse-move guard — user mouse movement pauses the run. *(Slice 5)*
28. Any-corner failsafe choice. *(Slice 5)*
29. Daily session click cap. *(Slice 5)*
30. Hard-stop key combo, works with dialogs open. *(Slice 5)*
31. One-time note the first time start-on-launch activates. *(Slice 5)*

### Feedback & visibility
32. Stop-reason notification (limit / failsafe / time / manual). *(Slice 6)*
33. Tooltip countdown to limit. *(Slice 6)*
34. Sound cues on start/stop. *(Slice 6)*
35. Steady-icon option (no blink). *(Slice 6)*
36. On-screen pill indicator with live count. *(Slice 6)*
37. Run history (start, duration, clicks, stop reason). *(Slice 6)*
38. Lifetime stats in Credits. *(Slice 6)*

### Settings, profiles & data
39. Named profiles (full settings sets). *(Slice 7)*
40. Profile quick-switch keys. *(Slice 7)*
41. Export / import settings JSON. *(Slice 7)*
42. Reset settings button (with confirmation). *(Slice 7)*
43. Portable settings mode (portable.txt beside exe). *(Slice 7)*
44. More interval presets (10 / 250 / 1000 ms). *(Slice 7)*
45. Separate Pause keybind. *(Slice 7)*

### Platform & polish
46. Per-monitor DPI awareness — locked clicks land exactly on scaled displays. *(Slice 1)*
47. ARM64 builds in CI + release. *(Slice 7)*
48. Installer: optional desktop shortcut. *(Slice 7)*
49. Settings window pages + remembers position/page. *(Pages land in Slice 3; position memory in Slice 7)*
50. Manual "Check for updates" in Credits (opt-in, opens releases page). *(Slice 7)*

## Status log

- 2026-10-25 — program adopted; Slice 1 in progress.
- 2026-10-25 — **Slice 1 DONE (v0.6.0):** items 46 (PerMonitorV2 DPI awareness at startup), 1 (Double action), 5 (Burst 1–100 × gap 1–1000 ms, early-stop on toggle/failsafe/limit; limit counts physical clicks), 7 (button-down time 0–1000 ms), 9 (alternating buttons, lib `effective_button`). Config gains action/burst_clicks/burst_gap_ms/press_ms/alternate, all defaulted for older files. Gate: fmt + clippy both targets clean, 37/37 tests.
