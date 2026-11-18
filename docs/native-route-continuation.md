# Native route continuation evidence

Target: GamePro.exe SHA-256 `13e7cf22b8b2e700f492794b4dd1b6f28ebe76d59165c1c188b95ac31b0c3b9d`.
This document records static consumer evidence and implementation scope, not a complete gameplay acceptance result.

## Serialized movement header is reordered

`56ECE0` reads the six movement bytes into object offsets **+1,+2,+3,+0,+4,+5**.
`9F8E14..9F8E30` copies those in-memory fields into actor movement type, animation speed, walking speed, and frequency.
Therefore the four raw `Page.movement` bytes are **animation speed, walking speed, frequency, autonomous type**.
They are not already laid out in native object order. For example `[2,3,0,1]` means animation 2, speed 3, frequency 0, custom route.
Evidence: `validation/native-scene-movement/route-header-load.asm` and `page-movement-binding.asm`.

`9F9A7C` dispatches an autonomous event only after idle ticks exceed `20 * frequency²`.
Native autonomous types are 0 stationary, 1 custom route (`A018A0`), 2 random, 3 approach.
The current archive's raw fourth byte contains only 0/1. `9F8F65` installs a new page route into the backup when a forced route remains active, otherwise into the current program and resets its cursor.

## Route host boundary

`route_host.rs` connects the interpreter to live world variable banks, original RNG, scene geometry, tile passability, actor collision, and target positions. It does not look up story/common-event identities.

`A05B08..A05C27` binds installed forced-route operands: opcode 28/55 first operand remains a destination address; contextual event-self is relocated to the target actor; absolute event-self and global operands remain live; common-self/system/database operands are resolved at installation. `A03320..A033A5` additionally relocates event-self at execution and treats common-self as zero when no call frame exists.

`A0345A` assigns a variable; `A038AA` adds to it. Both continue control dispatch. Face operations consume one update. Movement retries when blocked unless route flag 2 permits skipping; flag 1 repeats. `A04534` restores the displaced route when a non-repeating forced route finishes. Interpreter state, page movement type, cursor, and interpolation are part of the host snapshot.

Ordinary walking shares the existing Game.dat speed tables and quarter-pixel interpolation. `9F1C00` and `9F1EA0` choose approach axes with original `942820` RNG for equal distances, and try the secondary axis if the first cannot move. Opcodes 53/54 mean approach an actor / tile coordinate, not instantaneous teleportation.

## Verification and remaining boundaries

Added focused tests for raw page-header field ordering, autonomous route dispatch, page changes during a forced route, blocked walking retry, and execution-time variable reads after interpolation. The parent agent owns running Cargo and real-game tests; test existence is not a passing result.

The route corpus contains 12 opcode-60 and 35 opcode-160 sites. Their handler `A042F8` uses `9F4FB0` persistent search state, `9F4600` search, and `429370` search execution. **60 retains its target and queue and continues at A03262 until arrival; 160 invokes 9F5930 to clear the search after one movement attempt.** Both resolve coordinates from script operands. This corrects the earlier reversed interpretation of 9F5930: its assembly stores -1 at actor+244/+248 and clears both deques.

`route/path.rs` implements those two current-corpus forms. Search uses the extracted clockwise neighbor order (up, right, down, left; eight-direction adds intervening diagonals), native cardinal cost 100/diagonal cost 101, unscaled Manhattan/squared-distance heuristic from 429110, signed 64-bit priority with the native heap's tie behavior, looped-map neighbors, and the search budget `2*(dx²+dy²)+500` (local search subtracts 300). CA83B0 is 500.0, not the previously inferred 300.0. Constants and binary identity are retained in `path-constants.json`; aligned evidence is `path-neighbors.asm`, `path-search-core.asm` and `path-reconstruct.asm`.

The persistent queue searches terrain first. Mode 1 examines up to ten nearby nodes, then runs a temporary search to the sixth remaining node using actor obstacles; failed local search falls back to the retained route. Reconstructed search removes its starting node at 9F4EFF. Actual movement still runs the ordinary collision and interpolation consumer, so a path does not teleport or bypass blocking actors. Opcode 60 preserves the captured destination across future variable changes and checkpoints. Route replacement, page replacement without a forced route, and relocation clear stale search state. Failure retries the instruction unless route flag 2 permits skipping; a blocked persistent movement also respects that flag.

Added `path_60_detours_and_restores_its_queue_before_following_command`, `path_160_only_moves_once_then_advances_the_program`, `path_failure_and_dynamic_blocking_retry_or_skip_from_route_flags`, and `path_60_captures_destination_until_arrival`. They exercise a wall detour, independent checkpoint continuation, different instruction completion rules, invalid destinations, dynamic blocking, and destination capture across variable changes. These are synthetic interpreter tests; original-process route trajectory comparison remains a separate validation task. Temporary searches disregard the destination occupant while actual movement retains its collision, matching the purpose of 9F46A0..9F4B31. Exact native obstacle-cell footprints and occupied odd-coordinate start selection still need trajectory comparison against native movement; current scene collision remains authoritative when a selected path step is attempted.

General autonomous random/approach page modes (absent from current raw page fourth-byte corpus), eight-direction graphic facing, follower targets, and native saved route string operands require further consumer closure. These are not silently treated as completed movement.
