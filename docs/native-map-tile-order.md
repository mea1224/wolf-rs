# Map103 tile file order

The decoded tile payload is `layer -> x -> y`, with contiguous signed i32
values for y. For width W and height H the linear index is
`(layer * W + x) * H + y`.

`docs-append/evidence/rea-vm-9fd1d0.c` (REA evidence
`ev_16e28150d29ac9179678d68fd40e847b74584cb78bff3e43344dd5b4714304ea`)
calls `669970` at `9FD301`, passing map core +1640 as this. A decompilation
request for 669970 returned "Not function"; this conclusion instead uses the
original executable's disassembly saved in
`validation/native-scene/map103-file-reader.asm` and the short
`map103-tile-loop.asm` extract.

`66DEDB/66DEE3` select the tile vector at this+1440h. Width and height are
this+1434h and this+1438h. `67278D` defaults the layer count to three;
`6727DA` selects the >=102 format and `673273` reads its explicit layer count.
The preceding i32 read at `672E2E` is a separate field; it is not a fourth
dimension. `67329C..6732A7` temporarily supplies the layer count to allocator
570640 then restores that field.

`673AFA/673B0A` start the layer loop; `673D0C/673D16` start x and compare
against width. `6741AE/6741B4` calculate H*4. `6741BD..6741C8` select the
decoded source buffer and cursor; `6741C9..6741D4` select tiles[layer][x].data.
`6741D7` copies H*4 bytes via B99A50, `6741F3..6741FC` advance the source
cursor, `674207..674214` advance x, and `67422B..67423F` advance the layer.
The opaque outer wrappers evaluate to one iteration and do not change the
tile dimensions. This closes the file-order evidence independently of the
9FBB00 save reader's matching in-memory layer/x/y traversal.
