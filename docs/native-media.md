# Script picture and native video service

2026-11-17 title continuation: opcode290 supports type3 triangle swing,
type2 independent axis offsets, type12 independent axis scale, type5 RGB pulse
with ordinary blend0 and channels -100..100, and type9 duration0 disable.
Effects act on existing delayed-create placeholders and survive activation.
Positive effect delays, nonzero type9 animation and unverified replacements
remain explicit errors. The player validates configuration[4] for native step1.
See [menu effects](../../docs-append/picture-menu-effects-static-closure.md),
[placeholders](../../docs-append/startup-picture-placeholder-closure.md), and
[title acceptance](../../docs-append/startup-title-acceptance.md).

The IR profile is now `wolf350-evidence-subset-v8`, which changes the library
identity used by VM save/restore and replay. VM JSON version6 stores the native
Park–Miller RNG separately from the synthetic RNG; replay envelope version2 is unchanged. Native
scene/media state, including swing phase, is still
outside those snapshots. These formats do not restore a complete native game.

The host supplies the logical dimensions from game configuration to
`NativeMedia::new(width, height)`. The service contains no startup list or title
filenames. The VM supplies the actual command, raw flags, resolved numeric
operands, and the expanded resource path. Those numeric slices exclude opcode.

## Picture subset and evidence

`native_media.rs` implements command 150 ordinary image creation/replacement,
opacity/position/uniform-scale interpolation, anchors 0..6, move, and timed erase.
Variable-path variant 1 is accepted after the caller resolves its string.
The 18-argument extension accepts neutral RGB `(100,100,100)` and a queued delay.
Queues preserve insertion order. The delay test occurs before decrement, so a
delay reaching zero applies on the next host tick. Relative movement preserves
the create anchor. `tick` must continue during VM waits.

Delete uses duration at operand2, optional delay at operand3, and inclusive
range end at operand5 for the six-operand range form. Endpoints are signed and
normalized; only objects present when issued receive a queued delete. Repeated
deletes do not restart an active fade. Completed deletion is hidden from queries
but retained while its queue is nonempty. Immediate delayed-create placeholders
report exists1 and dimensions0; they do not render before activation. Moving a
deleted object and general revival/replacement remain unsupported.

Sources: `docs-append/media-execution-static-report.md`, saved REA functions
`A18DF0`, `98A2F0`, `996470`, `996B80`, and the anchor jump table at `98C5DC`.
Helper `tools/inspect_native_media.py` saves raw anchor-table and PNG-dimension
facts in `validation/native-media/static-facts.json`. The current interpolation
and relative-move subset still requires comparison at identical original ticks;
unit tests validate the declared implementation, not native equivalence.

Commands 160/161/162 preserve capture independence, busy retry, host-timer and
draw-stage gate completion. Positive kinds resolve SystemDB type11 rowkind field0;
kind2 loads `SystemGraphic/tr_ink.png`. The native upload uses its red channel
as mask alpha. The recovered shader computes saturate((mask + offset) * 4),
with the piecewise native integer progress/offset mapping. Current masks must
match viewport dimensions. See [shader and upload evidence](../../docs-append/startup-transition-mask-closure.md).
Text/inline
icons, generated gradients, stretched nine-slice skins, and blend1/3 have separate
evidence-backed subsets. Non-neutral tint, rotation, divided sheets and unverified
extensions remain explicit errors.

## Windows video backend

`native_video.rs` uses Windows Media Foundation SourceReader and CPU RGB32
conversion. `NativeVideo::open(local_path, looping)` opens a local decoded archive
member. `frame_at(elapsed_100ns)` selects by sample PTS, retaining the current
frame and one future frame. It bounds dimensions to 16 megapixels, byte layout
to 64 MiB and advance work to 4096 samples. The caller updates the associated
picture through `replace_frame` without resetting its tween.

The loop policy is a separate script-facing choice: `native_loop_name` matches
exact `_loop.` or `_LOOP.` tokens, as established at `9851F0`. Presentation time
wraps by the container duration; wrap/backward seeks rewind the reader. This is
a deterministic implementation policy, not proof of exact original decoder EOS
latency. Other platforms return a capability error. Video audio is disabled;
script opcode 140 uses the separate audio output service described below.

COM and MF startup/shutdown use RAII, including open failures. SourceReader drops
before MF shutdown; the object is confined to its creator thread. No graphics
device, window, network request or audio output device is needed. MF's different
YUV conversion may differ by a few RGB levels from the original decoder and must
be measured during original-program comparison.

## Verification on 2026-11-17

`cargo test -p wolf-player --test native_media --offline`: six checks pass,
including real decoded intro PNGs in an arbitrary script order, delayed create
followed by relative move, capture independence and unsupported-mode errors.

`cargo test -p wolf-player --test native_video --offline`: real local
`title_loop.mp4` passes two complete decoder lifecycles, changes pixels between
0 and 500 ms, and returns exactly the first decoded pixels after a duration wrap.
The test saves `validation/native-media/title-video-{0,500}ms.png`. This host's
MF initialization was denied inside the restricted sandbox; the test passed
under the approved `cargo test` command. No game window was launched by these
tests. The original-game screen comparison is a separate host-level check.

## Connected VM host services

`native_services.rs` owns decoded image, archive and movie caches and is connected
through the player's `Game`/`Vm::tick_with_host` boundary. Archives are decoded
once per mount, with a 512 MiB aggregate limit; decoded PNGs have a 256 MiB limit.
Movie creation uses the real resource supplied by command150, writes a checked
content cache under `validation/media-cache`, and maintains movie PTS per picture
instance. The host does not select an introduction sequence.

The initial opcode140 path is file source, type2, operation0, eight
operands, zero delay/position/pan/extras and unchanged pitch. `96FB30` writes
file parameters into temporary SystemDB type3 records. `96BD80@96D1EF..96D3E2`
queues this category; `96D7A0@96EB0C` passes ECX=handle, EDX=1, stack=1 to
`88C920`, whose regular path supplies `loop=(a2==3)` to `88EE40`. This establishes
nonblocking, non-looping playback for the supported request. The service decodes
the actual Ogg/WAV and starts CPAL output at the post-VM host update. No audio
instruction is skipped in the introduction-block test.

The title continuation also resolves actual SystemDB BGM row0 and cursor/button
SE rows0/1. Common23 PC79 `140 [0,60,0,0]` starts title BGM with fade-in60;
it is not a stop command. NativeAudio uses configuration39 default-volume
behavior, duration+1 fades with apply-before-decrement, group volume sync,
and one retiring BGM slot. Audio device callbacks consume atomic gain updates.
Unverified switch/reuse shapes fault explicitly. Exact native amplitude
transfer remains an output-policy boundary, not a waveform-parity claim.
See [title audio evidence](../../docs-append/startup-title-audio-closure.md).

`cargo test -p wolf-player --test native_services --offline` passes three tests:
configured viewport, original Common23 PC50 through the actual VM host context,
and the unmodified PC45..55 command block for1000 host ticks. The latter executes
all five native requests `[150,140,150,150,150]` without diagnostics and ends on
RGBA pixels identical to the decoded `Picture/guideline.png`. This is a real
command-block integration test; full startup, original-window capture and
side-by-side timing comparisons remain distinct acceptance checks.

The older per-module results above describe their original test scope. Current
workspace checks pass 224 tests (8 ignored); the separately executed release
default-title acceptance passes 1200 ticks, video, five menu pictures and
Down/Up navigation without any fiber fault. Full commands and GUI measurements
are in the title acceptance report linked above.
