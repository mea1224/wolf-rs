# Settings branch acceptance — 2026-11-18

This run extends the existing settings integration test; it does not add menu-specific behavior to the runtime. All menu changes are physical keyboard/pad samples processed by Common22, Common15, Common16 and Common8. Expected setting types, ranges and defaults come from the loaded User database. Test fixtures copy configuration and a valid save into unique directories under `validation/settings-extended/` before the first tick. Original saves/configuration remain untouched.

## Inputs and evidence

- `GamePro.exe` SHA-256: `13e7cf22b8b2e700f492794b4dd1b6f28ebe76d59165c1c188b95ac31b0c3b9d`.
- `Data/BasicData.wolf`: `231002c9ce9746661b1adc7b4076fe6bff4373371ff325d3e2b145774602a1a7`.
- `Data/MapData.wolf`: `8526fcf63dfc96b9b107306aa42e321b14ba54224b3611db9573c3496b0bbb7f`.
- Original command/native evidence: `validation/settings-semantics-20261117/evidence-map.md`, complete Common15/16 listings and `docs-append/title-database-settings-closure.md`.
- Edited implementation test: `crates/wolf-player/src/native_settings_tests.rs`. No production VM, IR, audio, media or menu handler was changed in this settings follow-up.

## Verified branches

| Original branch | Acceptance |
| --- | --- |
| Common15 rows 0–6 and 8–11 | Every slider and enum is edited by left/right input. All sliders traverse held-key repetition to both 0 and 100 clamps. Every enum visits all database-defined values and wraparound. |
| Common15 both tabs | First/last row navigation wraps. Tab switching works before and after key editing, including the newly assigned tab key. |
| BGM/SE/ambience rows | Database values propagate to system variables 100/102/101. The real audio host executes the script's sound requests during the tests. |
| Window row 6 and quality row 11 | Values propagate to sys138/sys33. Window options 0, 1 and 2 execute; the quality enum uses the actual database range, not an assumed count. |
| Voice, text speed, brightness, dash, skip, information | Slider/enum values and bounds are exercised, and each final value survives a fresh startup. |
| Language row 7 | The change creates Common18's message dialog, updates the language value, and closes settings via Common15 PC563. Fresh startup restores the changed value. |
| Common16 nine binding rows | All nine accept distinct physical keyboard keys. Reserved A is rejected while the editor continues waiting. |
| Common16 duplicate handling | Assigning an existing keyboard key exchanges the old binding with its former owner. Assigning an existing pad button exchanges the pad bindings and updates sys55. All nine keyboard and pad rows survive fresh startup. |
| Common15 reset | Cancel preserves the changed volume. Confirm restores all 13 database defaults (language follows sys141) and exits through PC393's cross-loop label jump. |
| Common16 reset | Cancel preserves the changed key. Confirm restores all nine keyboard/pad defaults, restarts through PC508's backward jump, and remains navigable. |
| In-game settings | A separate run completes the opening from the default entry, opens settings in game, switches tabs, closes settings and returns to gameplay. |

Common15's original Japanese/English reset dispatch is asymmetric: PC385 selects language, while the language-1 body at PC397–399 is empty. The acceptance above exercises the implemented language-0 reset body; no synthetic English reset behavior has been added.

## Runs

Built with:

```powershell
cargo test --offline --locked --release -p wolf-player --bin wolf-player --no-run
```

The resulting test executable was copied to `validation/settings-final-tests-20261118.exe` before execution so other agents could rebuild without Windows locking the Cargo output:

```powershell
& validation/settings-final-tests-20261118.exe game::settings_tests::local_settings_ --ignored --nocapture --test-threads=1
$env:WOLF_TAKEOVER_CASE = 'settings'
& validation/settings-final-tests-20261118.exe local_takeover_interaction_and_save_roundtrip --ignored --nocapture
cargo clippy --offline --locked -p wolf-player --bin wolf-player --tests -- -D warnings
```

- `validation/settings-resume-20261118-final.log`: **2 passed**, 149.20 seconds, including every bound/enum value, all binding rows, both reset flows and persistence.
- `validation/settings-resume-20261118-takeover.log`: **1 passed**, 60.63 seconds, fresh opening and in-game settings (no checkpoint restore).
- `validation/settings-resume-20261118-clippy.log`: passed with warnings denied.

The final test log's summary printed the earlier temporary key value 119; assertions and saved configuration verify the final exchanged row-0 value 123. The diagnostic print was corrected afterward; Clippy includes that correction.

These are script/runtime tests with render boundaries and real loaded assets. They do not measure desktop window-mode transitions, original-engine pixel equality, speaker output latency, sustained presentation frame rate or every gameplay branch. Those remain separate integration/performance acceptance work.
