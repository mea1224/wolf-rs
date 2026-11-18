# Scene and offline media policies

These portable modules are executable Rust policies, not verified interpretations
of WOLF page flags, route commands, picture commands or sound commands.

## Movement and collision

`wolf_vm::scene::Grid` accepts an explicit bounded collision mask and actors.
Solid actors respect terrain and occupancy; non-solid actors still respect map
bounds. Actors advance in ascending ID order, one route step per logical tick.
Routes support cardinal movement and waits, with retry/skip/fail on collision.
Two actors cannot exchange occupied cells in one tick. Finished and blocked
routes produce distinct results. A zero-length route completes on its next tick.

`Movement` owns a grid and bridges `Service::Movement` requests to host actions.
The caller supplies a route decoder; unknown route schemas should return errors.
An already moving actor rejects a second request. Tokens are decoded once;
completion/failure produces a host action, and removed requests cancel routes.
Call `Movement::update` once after each VM tick and pass the actions immediately
to `Host::poll`; feed its completions into the next VM tick. Duplicate updates
at the same tick do not advance actors. Do not drop returned actions.

TileSetData collision interpretation, diagonal routes, footprints, speeds,
automatic routes and original route flags remain unresolved. The adapter must
not guess them from tile IDs or route opcode names.

## Page activation

`Pages` evaluates explicit global equality guards before each VM tick. The
highest matching rule wins. Changing pages cancels the old fiber's service
tokens. `Action` requires an explicit owner ID input; `Once` starts on activation;
`Parallel` restarts after normal completion, never after a fault. Existing fibers
are reused. These are named Rust policies, not decoded WOLF trigger values.
The original 37-byte condition blob remains preserved and uninterpreted.

## Offline media output

`wolf-media` has no device or VM dependency. `Pictures` composites validated
RGBA8 images into a caller-owned `Image`, with stable `(z, ID)` ordering,
clipping, nearest scaling and straight-alpha source-over in byte space.
Show replaces a picture ID; erase removes it. Render does not clear the target;
the caller supplies the background each frame.

`Mixer` mixes supplied stereo i16 PCM clips into caller-provided frames using
i64 accumulation and final clipping. Channels support replacement, stop, pause,
gain and looping; completion returns channel IDs. Pause preserves the cursor.
All clips must match the mixer's sample rate; no implicit resampling occurs.

This is actual CPU pixel/sample generation, not itself a WOLF command backend.
`wolf-media` also decodes bounded PNG, PCM16 WAV and mono/stereo OGG/Vorbis
inputs. `wolf-player` uploads a composed RGBA frame through wgpu to a desktop
surface and can play an explicitly selected decoded clip through the default
CPAL output device. This proves a Windows surface/device execution path for the
preview; it does not establish the original 140/150 parameter bindings, fonts,
pitch, fades, stream handling, or blend compatibility.

## Validation and save boundary

`crates/wolf-vm/tests/scene.rs` checks collision ordering, waits, blocked policies,
movement completion/cancellation through the host, page replacement and stale
callbacks. `crates/wolf-media/tests/backends.rs` checks exact pixel composition
and PCM samples, clipping, pause, looping, gain and sample-rate rejection.

The `wolf-player` preview reads an explicitly named image from `Picture.wolf`,
draws an explicitly synthetic VM/grid actor overlay, and optionally reads an
explicit BGM/SE member for audio. Space toggles the synthetic page, Escape exits.
It does not decode or execute a game's map startup path. See [player usage](player.md).

Scene/page/media state is not included in VM/host snapshots or replay hashes.
Do not treat a VM snapshot as a complete scene save. Original-engine traces and
platform device tests are still required before claiming game compatibility.
