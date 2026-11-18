# Actor light and sprite validation

The native actor draw consumer `9CAAB0` copies opacity from actor+0x48 and blend from actor+0x4A (`9CABB1..9CABC3`). Its blend switch at `9CAC5A` selects normal 17/1, additive 18/2, subtractive 19/3, and multiply 11. The backend mode tables retained in `docs-append/evidence/startup-native/8C4320.c` and the FA3690 analysis in `docs-append/native-generated-picture-report.md` establish additive source/destination factors ONE/ONE for premultiplied input and SRC_ALPHA/ONE for straight input.

`native_scene_render::blit` previously clamped source + destination before applying source alpha. A source 200 over destination 200 at alpha 128 became 228 instead of saturating to 255. Add/subtract now scale the source first, combine with the destination, then saturate. The regression uses partially transparent RGB channels covering saturation, non-saturation and subtract-to-zero. Ordinary source-over and multiply retain their previous calculation.

Scene caching stores straight RGBA. The special-mode intermediate alpha remains source-over coverage, so fully opaque map pixels have the verified additive/subtractive color calculation. This does not establish native separate-alpha render-target parity on transparent holes, gamma-space equivalence, or exact GPU UNORM rounding. Multiply preprocessing, bush opacity division, actor flash/tint, map zoom and additional actor render layers remain separate consumer/evidence obligations; an additive-light fix is not proof of all actor effects.

Validation on 2026-11-17, rerun on 2026-11-18:

- `cargo test --offline --locked -p wolf-player native_scene::route::`: 15 passed, including four path tests.
- `cargo test --offline --locked -p wolf-player local_stationary_effect_sprites_advance -- --ignored --nocapture`: passed using real archived resources. Map001 event10 first shows frames 1/2/0 at ticks 0/9/27; Map031 event3 and Map000 event48 at ticks 0/14/42. Each produces three different raster images under `validation/native-scene-movement/effect-animation/`.
- These sprite timings are runtime fixture results, not synchronized original-process observations. They prove that frame changes reach rendered atlas cells, including the light effect, without asserting story reachability or full visual parity.
- `cargo test --offline --locked --workspace`: 391 passed, 0 failed, 14 ignored. This includes the new translucent additive/subtractive actor test, 15 route tests, six picture-cache tests and screen-tone/flash tests. Full log: `validation/integration-workspace-tests.log`.
- `cargo clippy --offline --locked --workspace --all-targets -- -D warnings`: passed, process exit 0. The separately rerun real archived sprite fixture also exited 0; log: `validation/integration-stationary-effects.log`.
