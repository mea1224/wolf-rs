# Startup configurable action keys

Common8 PC417/418 and PC421/422 write system52/55 and53/56 using values
from the key configuration database. These are the first two logical action
bindings, not ordinary numeric storage or volume settings.

The 9E8230 setter dispatches system52..54 to 9BD760 with action indices
0..2 and keyboard mode1 (`9E870D`, `9E8722`, `9E8737`); system55..57 use
pad mode0 (`9E874C`, `9E8761`, `9E8776`). Exports are in
`docs-append/evidence/startup-native/9E8230.c` and `9BD760.c`; the latter's
REA evidence is `ev_e7369003c003ba9ec8b852a904f8d7dfefc63c187832d6d87c251dd5c102a1b6`.

Keyboard codes100..355 are stored at core DWORDs634..636. Pad codes301..320
are transformed into masks `1 << (code - 297)` at DWORDs637..639. Invalid
values leave the bindings unchanged in the release mode. Assigning a code
already used by another action exchanges that action's binding with the
destination's old binding. `9BDA93..9BDB1E` and `9BD8F8..9BD983` implement
the ordered exchanges. The VM reproduces this order and stores the public
codes exposed by the getter, rather than native memory masks.

`9BDC40.c` (REA evidence
`ev_31485cc690c36e218f787e6813230fe4f4b81518cd302d28f9b7749d99c421d5`)
returns keyboard codes directly and translates exactly twenty pad masks to
301..320. An unrecognized pad mask reads zero.

Constructor9605F0, `960CCF..960D01`, initializes keyboard144/145/146 and
pad masks10h/20h/40h, so the host initializes systems52..57 to
144/145/146/301/302/303. See
`validation/native-values/core-constructor.asm`. The desktop adapter consumes
these bindings alongside the permanent aliases proven at `9BC848..9BC925`,
then applies the existing logical input permission mask. Rebinding therefore
affects actual logical action10/11/12 input on the next sample.

Tests cover all six collision directions in both banks, repeated exchanges,
valid range boundaries, release invalid-value no-ops, profile gating, a
configured keyboard action, permanent Enter fallback, and the twentieth pad
mask. This does not verify physical hardware support for every native pad
button or all key configuration UI behavior.
