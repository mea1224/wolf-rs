# Audio continuation audit, 2026-11-18

Audio choices remain data driven. Opcode140 resolves source, database row or expanded file path, volume, pitch, delay/fade duration, position, loop point, pan and group/channel from command operands and the system database. Initial loading and map transfer call `native_map_audio::requests` and `Services::map_audio`; map metadata selects keep, stop, database audio or an explicit path. Route46 uses its resolved database operand and the native group1 convention. No production map IDs, scene names or audio filenames select a track.

## Corrections

- A delayed SE group stop could leave later entries alive because `NativeAudio::tick` moved the pending list out before `stop_effect_group` inspected it. The tick now removes matching entries from both its remaining work queue and the already deferred queue. Regression checks both queue orders, a specific group, all groups, muted stops and survival of unrelated groups. Original `96D410` at `96D450..96D574` walks and erases all matching queued handles; evidence is `validation/native-title-audio/96d410.c`, `ev_1e464e6d6beff5075e3144e48dd73ca709a4db4671a23c61cd673fd7ef9ce787`.
- A resolved row of -1 in an SE unload command was accidentally treated as an immediate group0 stop. The immediate stop branch now requires action0 (play/stop). Original `A18A38..A18A74` skips variable-row action2 when its resolved row is -1; `A18BCA` distinguishes play from unload. The regression uses the actual Common69 PC10462 unload envelope with its variable operand resolved to -1, and verifies an already queued group0 sound survives.

## Verification

`tools/audit_audio_commands.py` regenerates `validation/audio-audit-20261118.json` from the complete current common/map corpus. It inventories 1,630 opcode140 records across 26 action/kind/source/operand-count combinations: 1,629 play/stop commands and one unload command. It records both source SHA256 and executable SHA256. The executable remains `13e7cf22b8b2e700f492794b4dd1b6f28ebe76d59165c1c188b95ac31b0c3b9d`; the corpus SHA256 is `c5fd80415cc41c4ed18ab6f663afa39a58450505c802123b3e3e77e1dc87892b`.

Actual checks after these corrections:

- `cargo test --offline --locked -p wolf-player --bin wolf-player audio -- --nocapture`: 31 passed, 1 ignored; process exit0. Log: `validation/audio-focused-20261118.log`. This includes 18 native command/snapshot/fade tests plus mixer, device-buffer policy and map metadata tests. Mixer checks include concurrent publication/stop/reclamation, pan, pitch, loop intro, position conversion, queue/PCM limits and explicit device failure.
- `cargo test --offline --locked -p wolf-player --bin wolf-player local_map_audio_settings_cover_all_map_rows -- --ignored --nocapture`: passed; process exit0. All54 system database map rows were resolved. Log: `validation/audio-map-settings-20261118.log`.

These are static inventory and focused Rust tests, not execution of every dynamic script branch or an audible comparison of every track. The callback ownership audit found no additional change necessary: the single consumer exclusively accesses READY/PLAYING PCM; it releases DONE only after ending references; publisher/reclaimer access is serialized and PCM is destroyed away from the callback. The focused tests do not open a physical output device.

Nearest-neighbor pitch/rate conversion and linear gain/pan remain host policies, without claimed parity with the native backend's exact amplitude/filter response. Ogg/WAV are implemented. Native MIDI and action3 cache sweeping remain unsupported; action3 does not occur in the inventoried opcode140 corpus. Existing host voice/cache limits remain explicit limits, not native format claims. Full startup/gameplay/settings and 60fps validation are tracked separately by the integration work.
