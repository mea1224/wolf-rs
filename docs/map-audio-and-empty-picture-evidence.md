# Map audio and empty-slot picture commands

Evidence scope: the local GamePro.exe SHA256
`13e7cf22b8b2e700f492794b4dd1b6f28ebe76d59165c1c188b95ac31b0c3b9d`.
This is static native evidence, not a claim of whole-game acceptance.

## Map audio

The `.mps` tile/event header is not the sound configuration. Native `9FE550`
queries SystemDB type 0, row=current map ID, field 1 for BGM and field 2 for
BGS through `946830`. `-2` preserves playback and fade state, `-1` stops,
nonnegative values select an audio database row, and `-3` uses the explicit
file in logical field 5 (BGM) or 6 (BGS), queried through `946F40`.

`9FE608/9FE753` supply file parameters volume=100, pitch=100, loop=0.
`9FE624/9FE8DE/9FE789/9FE98D` pass fade duration 5 to `96BD80`.
`9FE638..9FE67C` then sets the incoming BGM fade timer to remaining=0,total=1
and target=source-volume*master/100. The previous voice retains its five-step
fade. The BGS branch does the equivalent for channel 0 and first stops all
nonzero BGS channels. Keep (`-2`) bypasses those side effects entirely.

Direct call sites `A00D77` and `A4172B` enter `9FE550`. The latter follows
the map loader `9FD1D0`. Existing startup/transfer orchestration must invoke
the map configuration consumer at map entry, before map event playback.

Implementation: `native_map_audio::requests` resolves the configuration;
`Services::map_audio`/`NativeAudio::map_change` share ordinary audio parsing,
channel management and playback. There are no map IDs, event IDs, or music
filenames encoded in production rules. Unit tests distinguish row zero from
stop, preserve from stop, and both explicit-path fields. The ignored archive
audit lists every map setting row; test execution is owned by the parent
integration run.

## Empty-slot Shift

Common10 action13 reaches PC728. Only a valid selected slot opens the memo
editor (PC733..762). The empty branch plays its invalid-action SE, but both
branches then run picture moves/deletes at PC767..771.

`A1AA79..A1AC84` implements move by iterating existing picture objects in the
requested inclusive ID range. A missing ID receives no operation and no
delayed command. `A1ACBA..A1AD72` likewise deletes only existing objects.
The previous Rust single-ID move returned an error where the range form
already did nothing. It now returns Applied after validating arguments and
before queueing delayed work. This prevents both the UI fault and a delayed
command accidentally affecting an unrelated later create of the same ID.

The regression repeats immediate/delayed moves and deletes against missing
IDs, then creates the same ID and verifies it remains at its authored position
and opacity. The distinct case of an already-deleted object retained because
it owns queued commands is not silently changed by this patch; it requires
additional lifecycle evidence.

Artifacts: `validation/native-map-audio/9fe550-rea.json` retains complete REA
evidence. `tools/inspect_map_audio.py` exports bounded objdump listings and a
manifest with hashes. E8 byte matches are candidates, not automatically
instruction-boundary-proven call edges.
