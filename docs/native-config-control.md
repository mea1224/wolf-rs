# Startup configuration control and numeric string conversion

## Raw 106 in release mode

`GamePro.exe` A09193 dispatches 106 to A0B3E0. REA export:
`docs-append/evidence/startup-native/A0B3E0.c`, evidence
`ev_2cf09f3f9c8996ba6e21c2299699a4b3fbd665cff7430c08d36cae05e5071465`.

This command is not unconditionally a no-op. The native handler first handles
the exact leading strings `MessageBox:`, `PictureWatch:`, `ValWatch:`,
`MoveWatch:`, and `PerformanceMonitor:`. Those branches can display UI or change
debug monitor state. The current host rejects them explicitly.

The ordinary text branch at A0CB49 returns success immediately when the debug
flag at 14DC660 is not 1. It does not expand the text, wait, or mutate VM
variables. The shipped release startup therefore accepts the zero-argument,
one-string ordinary form with no effects. Debug-mode output is not implemented.
The host test includes a deliberately unsupported escape that must remain
unevaluated and checks all five special prefixes are rejected.

## Raw 121 string operands

`validation/native/arithmetic-handler.asm` A0F12A/A0F18D calls 9E2260 to classify
each operand. Numeric operands use A05800; string operands use A058D0 followed
by BA6B62 at A0F15C/A0F1C1. Bits 4/8 (values 0x04/0x08) bypass conversion and
use the literal integers. The string classifier is also exported as
`docs-append/evidence/startup-native/9E2260.c`, evidence
`ev_ae846690e6b264391e5152b7d47210589ece5e607f80cec51d428b3bae955884`.

IR `Value::StringNumber` preserves the distinction between a numeric slot and
a string parsed as a decimal integer. Lowering is limited to raw 121's two
ordinary operands and the proven common/global/system string banks. Numeric
destination decoding still rejects string addresses. The VM reuses the same
bounded atoi implementation as native string-to-number assignments: leading C
whitespace, optional sign, decimal prefix, zero if no digits. Integer overflow
remains an explicit error rather than an asserted compatibility result.

Tests cover both operands, signs and trailing text, empty/nonnumeric values,
decimal-point termination, subsequent WOLF integer clamping, and literal flags.
The real Common8 PC193 now executes; the next main-fiber diagnostic reached
Common8 PC227 `[121,9000100,1600013,0,0]`, a platform system-variable write.
This is a progress boundary, not a full startup acceptance result.

## Raw121 indirect numeric destination

The subsequent main diagnostic reached Common89 PC38
`[1600035,1600012,0,16]`. Bit0x10 means that the first argument names a
variable containing the destination address. In
`validation/native/arithmetic-handler.asm`, `A0EE79` selects this path,
`A0EF5A` resolves the address value, `A0F2EE..A0F308` selects and reads the
pointed numeric slot, and `A10194..A101AC` commits through that address via
A05440. Both pointer and pointed address retain current-common addressing.

IR `ComputeIndirect` preserves the pointer expression. Runtime resolves it
before evaluating and committing the ordinary numeric computation. Tests use
the actual Common89 argument form, verify assignment and addition to the
pointed global slot, current-common destination resolution, and a pointer
whose destination aliases itself. Indirect destination ranges, string
destinations and indirect source flags0x20/0x40 remain explicit unsupported
forms; no broader command compatibility is asserted.
