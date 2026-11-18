# wolf-player

Updated 2026-11-17. The default player entry resolves its startup map and
coordinates from `SysDatabase.dat`, maps IDs through the database map table,
and evaluates initial event pages with `wolf_ir::startup::initial_page`.
Pages are checked from highest to lowest; the first matching page is selected.
The player starts selected pages with automatic/parallel trigger values 1/2.
This replaces the old refusal to select any page or require an unconditional
single page. Later page reselection, page locks and the full automatic/parallel
common-event lifecycle remain incomplete.

There are no `--mode evidence`, `--mode similar`, or `--loose-single-page`
options in the current executable. `--program PROGRAM_ID` explicitly selects
a diagnostic event entry and bypasses automatic page selection; it is not the
default startup path. There is no command-line map override.

Logical resolution comes from `Game.dat`. Initial scale, monitor and window
mode combine its fields with `Game.ini`; the current configuration produces a
1280×720 client area at scale 1. Missing `Game.ini` uses the recovered defaults.
See [window configuration evidence](window-configuration-evidence.md) for the
native addresses, limits and remaining platform differences.

Unknown reachable instructions retain source context and fault in the VM.
The default entry now passes 1200 ticks with every fiber checked and rendering
on every tick. Enter through the keyboard adapter produces V3=10/V5=1 at tick302;
the original scripts continue through particles, BGM, mask transition, title
video and all five menu items. Down and Up change selection 0 → 1 → 0.
A 30-second release window run also passes. See the
[title acceptance report](../../docs-append/startup-title-acceptance.md) for
binary identities, screenshots, timings and remaining gaps. New game, full
gameplay and complete state restoration are outside this acceptance.

The playback clock starts after display initialization and permits one update
per callback, anchoring the next update 16,651 microseconds after the actual
start of the current one. This follows the recovered native high-rate branch
and removes startup/stall fast-forward. The native low-rate draw-skip path is
not yet reproduced. This machine's title composition remains slower than the
original; the recovered interval is not a claim of measured rate parity.

```powershell
cargo run -p wolf-player -- --root ..
cargo run -p wolf-player -- --root .. --media-probe --capture-tick 160 --capture validation/media-probe.png --smoke-seconds 5
```

Run these commands from `wolf-rs`. `--input-probe` isolates the real Common26
PC201 keyboard-wait instruction. `--media-probe` isolates Common23 PC45..55,
including the logo, SE and guideline image; configuration initialization,
the key guide and later startup logic are outside that probe. These two probes
and `--program` are mutually exclusive.

`--capture FILE.png`, `--capture-tick N` and `--smoke-seconds N` control capture
and bounded execution. `--probe-press-tick N` requires `--input-probe` and a
smoke duration; it is a diagnostic input injection, not ordinary startup input.
`--startup-confirm-tick N` requires default startup and a smoke duration. It
feeds physical Enter for three logical ticks through the keyboard adapter,
isolating other keyboard/pad input before mapping. It retains normal page
selection, Common22 and all fibers. `tools/verify_candidate_title.ps1` uses
tick300 confirmation, tick900 capture and 30 seconds of execution.
`--audio FILE` is a media backend diagnostic, not the game's startup audio
path. Escape exits the window. See [startup compatibility](startup.md),
[VM coverage](vm.md), [host services](host-services.md), and
[compatibility gaps](compatibility.md).
