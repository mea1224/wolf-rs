# Script audio and save continuation, 2026-11-17

Target `GamePro.exe` SHA256 rechecked: `13e7cf22b8b2e700f492794b4dd1b6f28ebe76d59165c1c188b95ac31b0c3b9d`.

## Actual script coverage

An inventory of **both** `current-startup-corpus.json.common.events` and all map pages finds 1,630 opcode140 commands: 1,629 play/stop commands and one unload command. This is static inventory, not a claim that all branches ran. Examples that previously failed in the host:

- Common28 PC650: file CV, group3, pan89.
- Common84 PC39: file SE with variable volume and pan.
- Common27 PC43/57: file SE with randomly selected pitch75..125; Common70 PC184/214 uses pitch104.
- BGM/BGS positions include constants (48246) and variables. Source, volume, pitch, loop point, delay, group and position remain command/database values; no scene IDs or track names select production behavior.

## Native evidence and changes

`validation/native-title-audio/a18940.c`, evidence `ev_0587c0aafc87e3bd51157bf7c2ce0dcb69bae9c980f3c38e4b5737998a8a6968`, resolves optional argument3 as playback position and argument7 as stereo balance. `96bd80.c`, evidence `ev_2c0045dad9d71cb27a41975a6a6383aa25777af5734194aa5e748e4461afbbaf`, distinguishes BGM/BGS and grouped/delayed SE.

- Saved disassembly `seek-position.asm` and `seek-units.asm`: 88DC30 calls8A2E20, which multiplies position by the source sample rate and divides by1000. Positions are milliseconds, not simulation ticks. Replaying a database BGM row with a nonzero position seeks while preserving the existing fade.
- `pan-clamp.asm`: 88CE90 clamps balance to−255..255. 88D870 uses the same range for queued SE. The host mixer now applies stereo balance, preserves it in snapshots, and interpolates background pan using the existing fade clock (96D7A0/96DB94).
- `loop-position.asm`: 88E080 converts the loop point via8A2E20 and then calls88E520. The host preserves the intro and repeats from the configured loop point. Pitch changes PCM sample selection and playback length rather than rejecting the command.
- `96BD80` stops a delayed SE group before its muted-playback gate. The host no longer drops delayed stop requests at master volume zero.
- `restore-bgs.asm`: A40681..A4092F walks the saved channel→row map, uses each active channel's path/volume/pitch/loop/pan, and starts a20-tick fade. The parser now retains system+2272 instead of skipping it; stopped channels' metadata does not revive their sound.
- `validation/native-map-audio/map-config-audio.asm`: 9FE550 consumes system database map BGM/BGS configuration. `Services::map_audio` routes it through the same command audio adapter:−2 keeps,−1 stops,−3 uses the configured file. New map audio receives total1/remaining0 while outgoing audio fades for5ticks. BGS changes stop other channels.

Ordinary play previously read a decode cache but never populated it. It now caches decoded PCM within a128MiB budget. `Clip` shares immutable PCM so replaying a cached sound does not copy its entire sample buffer. The output thread remains persistent.

## Save boundary fixes

Pending SE stop commands legitimately have no path. Capturing one worked, but restoring it failed with `saved SE missing source`; all-group stops were also incorrectly rejected. Both are restored as control commands and tested across capture/restore.

Real takeover checkpoint restoration exposed another deterministic save failure: `NativeCommonCall` executes through `Library::resolve_common`, but snapshot validation used the generic `resolve`. A map event sharing the common event's label made a legitimate active call stack fail validation. Restoration now uses the same namespace as execution. A regression saves an active named common call alongside a map event with the same label, restores and completes it, and still rejects an altered callee. All7native-call tests pass.

Application snapshot version4 and VM version10 record the new movement state and map-scoped variable context. The envelope version is checked before decoding Scene, so older snapshots get an explicit incompatibility diagnostic rather than a missing-field error. The selected Save root is installed before restoring service resources. Original game save files are not rewritten by these tests.

## Menu screenshot dependency

The real takeover/settings test next reached Common9 PC0 and failed on opcode1000. Inventory of the complete corpus finds10commands, all mode0 (capture) or mode2 (copy). `validation/native-title-audio/screenshot-copy.asm` records the native handler A2D9F0: mode0 expands the path through94FC50, then A2DBC1 passes JPEG quality85 to768AF0; mode2 expands both paths and A2E280 calls CopyFileW with overwrite enabled. Common10 PC556 copies the pre-menu screenshot to its script-selected slot path.

IR now routes1000 to Services. Capture renders the current composed image immediately and the separate Windows GDI+ encoder writes JPEG85. Copy uses both expanded filenames; Save paths use the configured isolated store. Other unverified modes still fail with command context. The encoder is a Windows platform adapter; non-Windows screenshot encoding remains unsupported. Native byte-identical JPEG compression is not claimed.

## Validation and limits

Initial focused runs:26 audio tests passed; complete application sidecar/Scene restore test passed including rejection of old version2; wolf-media7tests passed with1manual benchmark ignored. JPEG screenshot tests pass for isolated paths, JPEG signature, dimensions, color channels and replacing an existing image. The new physical-input opening reached takeover and captured a version4/10 checkpoint; `validation/takeover-settings-v10-full.log` then advanced through Common9's screenshot and identified an independent opcode290 third-parameter40 effect gap atPC401. This log is a failure diagnosis, not settings completion. Subsequent map audio and real UI regressions are tracked by the task's validation logs.

Final focused audio rerun:16 `native_audio` tests passed, including map metadata changes and real route sound grouping (`validation/native-audio-focused-final.log`). The IR screenshot/copy test also passes through both command lowering and program validation. Refreshed whole-corpus audit inventories81,924commands,78,022ordinary-entry candidates and1,723definite shape-gap sites under its current conservative rules;769candidates remain IR unsupported. Settings has no shape gap under these rules, which does not prove execution success or validate every resolved parameter. The audit records source hashes and reported no source changes during the final run.

Nearest-neighbor rate conversion and linear gain/balance are host output policies; exact native backend amplitude/filter parity is not claimed. Ogg/WAV are the implemented codecs. Native MIDI/backend-specific behavior and action3 cache sweeping are not established by these tests. Static inventory alone does not establish whole-game completion.
