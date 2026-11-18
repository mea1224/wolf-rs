# Native string conditions (raw 112)

Static evidence: `GamePro.exe` dispatch A091B5 calls A0D330. Full REA export is
`docs-append/evidence/startup-native/A0D330.c`, evidence
`ev_83cc6fb85667df71373bdfe963a7e86de23d1717982c1376b1fc748762b01f6b`.
Instruction bytes are retained in `validation/native/string-condition-handler*.asm`.

A0D3AB reads the low nibble of argument 0 as condition count. A0D426/A0D430
read each packed argument and mask its low 24 bits for the left string address.
Common self addresses are resolved against the calling common event. A0D618
tests bits 24–27: zero selects the corresponding literal string; a nonzero
value selects the RHS address at `1 + count + case`. The implementation accepts
the evidenced mode 1 and rejects other nonzero encodings. Both ordinary RHS
paths call 94FC50 (A0DAC0/A0DC7A). The existing bounded plain-text expander
handles supported variable tokens, including nested substitutions.

A0DF32 selects the comparison from bits 28–31:

| Mode | Native site | Behavior |
| --- | --- | --- |
| 0 | A0DF45 | Exact equality, including length |
| 1 | A0DFD7 | Inequality; 56B8C0 negates 439CB0 byte equality |
| 2 | A0DFF8 | RHS occurs anywhere in LHS |
| 3 | A0E02A | RHS starts at offset zero |
| 4 | A0E05B | RHS does not occur |
| 5 | A0E08D | RHS starts at `lhs.length - rhs.length` |
| 6 | A0E0CA | Separate regular expression path, not implemented |

A0E2F7 exits after a match, and A0E320 stores the one-based branch selector.
The IR therefore evaluates conditions in order and selects the first matching
401 branch, otherwise the 420 default or block end. This includes Common8 PC91:
`[17,1600005]`, strings `["0","","",""]`, testing the file-existence result.

Tests in `crates/wolf-vm/tests/native_string_condition.rs` execute the lowered
branches for file results 0/1, all six comparisons, empty and Unicode strings,
RHS variables and expansion, first-match ordering, and malformed/regex rejection.
Four tests pass. This verifies the supported plain-string paths; native lazy
text-variable metadata, map-event strings, UI formatting escapes, debug-mode
conversions and regular expressions remain outside this implementation.
