use wolf_format::{RawCommand, Span, WolfString};
use wolf_ir::{compile, Binary, Op, Source, Update, Value, Var};

fn command(opcode: u32, args: &[i32], indent: u8, strings: &[&str]) -> RawCommand {
    RawCommand {
        span: Span { start: 0, end: 1 },
        opcode,
        args: args.to_vec(),
        indent,
        strings: strings
            .iter()
            .map(|s| WolfString {
                raw: [s.as_bytes(), &[0]].concat(),
                span: Span {
                    start: 0,
                    end: s.len() + 1,
                },
            })
            .collect(),
        terminator: 0,
        movement: None,
        extension_length: 0,
        extension: vec![],
    }
}

fn source() -> Source {
    Source {
        asset: "BasicData/CommonEvent.dat".into(),
        content_blake3: "corpus-fixture".into(),
        event: 1,
        page: None,
        command: 0,
        span: [0, 0],
    }
}

#[test]
fn menu_screenshot_and_slot_copy_survive_program_validation() {
    // Common9 PC0 and Common10 PC556 must retain native path expansion in
    // their raw records, and be accepted by both lowering and validation.
    let commands = [
        command(1000, &[0], 0, &["Save/ss_temp.jpg"]),
        command(1000, &[2], 0, &["Save/ss_temp.jpg", "Save/ss\\cself[15].jpg"]),
    ];
    let program = compile("screenshots".into(), "".into(), source(), &commands).unwrap();
    program.validate().unwrap();
    for (instruction, raw) in program.code.iter().zip(&commands) {
        assert!(matches!(instruction.op, Op::NativeHost));
        assert_eq!(instruction.raw.as_ref().unwrap().strings.iter().map(|text| &text.raw).collect::<Vec<_>>(), raw.strings.iter().map(|text| &text.raw).collect::<Vec<_>>());
    }
}

#[test]
fn lowers_observed_common_event_set_and_equality_branch_shapes() {
    // These argument forms occur in CommonEvent 1 at raw PCs 10 and 0 respectively.
    let set = command(121, &[2_000_013, 0, 0, 0], 0, &[]);
    let equal = command(111, &[1, 2_100_002, 1, 2], 0, &[]);
    let branch = command(401, &[1], 0, &[]);
    let end = command(499, &[], 0, &[]);
    let program = compile(
        "common/1".into(),
        "".into(),
        source(),
        &[set, equal, branch, end],
    )
    .unwrap();

    assert!(matches!(
        program.code[0].op,
        Op::Compute {
            dst: Var::Global(13),
            lhs: Value::Int(0),
            binary: Binary::Add,
            rhs: Value::Int(0),
            update: Update::Set,
            ..
        }
    ));
    assert!(matches!(program.code[1].op, Op::Branch { fallback: 4, .. }));
    assert!(matches!(program.code[2].op, Op::Jump(4)));
}

#[test]
fn literal_duplicate_labels_select_first_match_and_preserve_raw_records() {
    let commands = [
        command(213, &[], 0, &["duplicate"]),
        command(212, &[], 0, &["duplicate"]),
        command(0, &[], 0, &[]),
        command(212, &[], 0, &["duplicate"]),
        command(0, &[], 0, &[]),
    ];
    let program = compile("common/69".into(), "".into(), source(), &commands).unwrap();
    assert_eq!(program.code.len(), commands.len());
    assert!(matches!(program.code[0].op, Op::Jump(1)));
    assert!(matches!(program.code[1].op, Op::Nop));
    assert!(matches!(program.code[4].op, Op::Nop));
}

#[test]
fn label_jumps_rebuild_loop_ancestry_but_reject_unknown_expansion() {
    for commands in [
        vec![
            command(213, &[], 0, &["inside"]),
            command(170, &[], 0, &[]),
            command(212, &[], 1, &["inside"]),
            command(498, &[], 0, &[]),
        ],
        vec![
            command(213, &[], 0, &["\\cself[0]"]),
            command(212, &[], 0, &["\\cself[0]"]),
        ],
    ] {
        let p = compile("labels".into(), "".into(), source(), &commands).unwrap();
        assert_eq!(p.code.len(), commands.len());
        if commands[1].opcode == 170 {
            assert!(matches!(&p.code[0].op, Op::NativeLabelJump { target: 2, loops } if loops == &[1]));
        } else {
            assert!(matches!(p.code[0].op, Op::Unsupported { opcode: 213, .. }));
        }
    }
    let commands = [
        command(170, &[], 0, &[]),
        command(213, &[], 1, &["outside"]),
        command(498, &[], 0, &[]),
        command(212, &[], 0, &["outside"]),
    ];
    let p = compile("exit".into(), "".into(), source(), &commands).unwrap();
    assert!(matches!(&p.code[1].op, Op::NativeLabelJump { target: 3, loops } if loops.is_empty()));
}

#[test]
fn earlier_dynamic_label_cannot_be_skipped_by_a_literal_jump() {
    let commands = [
        command(213, &[], 0, &["target"]),
        command(212, &[], 0, &["\\cself[5]"]),
        command(212, &[], 0, &["target"]),
    ];
    let p = compile("dynamic-first".into(), "".into(), source(), &commands).unwrap();
    assert!(matches!(p.code[0].op, Op::Unsupported { opcode: 213, .. }));

    let commands = [
        commands[0].clone(),
        commands[2].clone(),
        commands[1].clone(),
    ];
    let p = compile("literal-first".into(), "".into(), source(), &commands).unwrap();
    assert!(matches!(p.code[0].op, Op::Jump(1)));
}

#[test]
fn malformed_end_does_not_discard_later_well_formed_branches() {
    let commands = [
        command(111, &[1, 2_100_002, 1, 2], 0, &[]),
        command(401, &[1], 0, &[]),
        command(0, &[], 1, &[]),
        command(499, &[], 0, &[]),
        command(498, &[], 0, &[]),
        command(111, &[1, 2_100_002, 1, 2], 0, &[]),
        command(401, &[1], 0, &[]),
        command(0, &[], 1, &[]),
        command(499, &[], 0, &[]),
    ];
    let program = compile("common/100".into(), "".into(), source(), &commands).unwrap();
    assert_eq!(program.code.len(), commands.len());
    assert!(matches!(
        program.code[4].op,
        Op::Unsupported { opcode: 498, .. }
    ));
    assert!(matches!(program.code[5].op, Op::Branch { .. }));
    assert!(matches!(program.code[8].op, Op::Nop));
}

#[test]
fn unsupported_loop_forms_and_unclosed_blocks_keep_all_raw_records() {
    let raw = [
        command(179, &[1, 2], 0, &[]),
        command(171, &[], 1, &[]),
        command(498, &[], 0, &[]),
        command(111, &[1, 2_000_000, 1, 2], 0, &[]),
        command(401, &[1], 0, &[]),
    ];
    let p = compile("bad".into(), "".into(), source(), &raw).unwrap();
    assert_eq!(p.code.len(), raw.len());
    assert!(p
        .code
        .iter()
        .all(|i| matches!(i.op, Op::Unsupported { .. })));
    assert!(p.code.iter().all(|i| i.raw.is_some()));
}

#[test]
fn extensions_and_route_data_are_never_implicitly_executed() {
    let mut raw = command(0, &[], 0, &[]);
    raw.extension_length = 1;
    raw.extension = vec![5];
    let p = compile("extension".into(), "".into(), source(), &[raw]).unwrap();
    assert!(matches!(p.code[0].op, Op::Unsupported { opcode: 0, .. }));
}

#[test]
fn independent_real_command_fixtures_keep_source_and_raw_records() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/control.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let raw: Vec<RawCommand> = serde_json::from_value(case["commands"].clone()).unwrap();
        let mut src = source();
        src.event = case["event"].as_u64().unwrap() as u32;
        src.command = case["first_command"].as_u64().unwrap() as usize;
        let p = compile("fixture".into(), "".into(), src.clone(), &raw).unwrap();
        assert_eq!(p.code.len(), raw.len());
        for (n, ins) in p.code.iter().enumerate() {
            assert_eq!(ins.source.command, src.command + n);
            assert_eq!(ins.source.span, [raw[n].span.start, raw[n].span.end]);
            assert_eq!(
                serde_json::to_value(&ins.raw).unwrap(),
                serde_json::to_value(&raw[n]).unwrap()
            );
            if raw[n].opcode == 99 {
                // CommonEvent 1 PC 58 carries an integer, ignored by the native dispatcher.
                assert!(!raw[n].args.is_empty());
                assert!(matches!(ins.op, Op::Nop));
            }
        }
        if src.event == 100 {
            // Native 499 is a state reset; CommonEvent 100 PC 385 is legal without an opener.
            assert!(matches!(p.code[385 - src.command].op, Op::Nop));
        }
    }
}

#[test]
fn unsupported_loop_end_traps_locally_and_preserves_following_commands() {
    let mut end = command(498, &[], 0, &[]);
    end.extension_length = 1;
    end.extension = vec![1];
    let raw = [
        command(170, &[], 0, &[]),
        command(176, &[], 1, &[]),
        end,
        command(0, &[], 0, &[]),
    ];
    let p = compile("extension".into(), "".into(), source(), &raw).unwrap();
    assert!(p.code[..3]
        .iter()
        .all(|i| matches!(i.op, Op::Unsupported { .. })));
    assert!(matches!(p.code[3].op, Op::Nop));
}

#[test]
fn unknown_condition_flags_and_duplicate_markers_are_explicit_traps() {
    for mode in [7, 0x20, -1] {
        let raw = [
            command(111, &[1, 2_000_000, 0, mode], 0, &[]),
            command(401, &[1], 0, &[]),
            command(499, &[], 0, &[]),
        ];
        let p = compile("bad-condition".into(), "".into(), source(), &raw).unwrap();
        assert!(matches!(p.code[0].op, Op::Unsupported { opcode: 111, .. }));
    }
    let raw = [
        command(111, &[1, 0, 0, 2], 0, &[]),
        command(401, &[1], 0, &[]),
        command(401, &[1], 0, &[]),
        command(499, &[], 0, &[]),
    ];
    let p = compile("duplicate".into(), "".into(), source(), &raw).unwrap();
    assert!(matches!(p.code[0].op, Op::Unsupported { opcode: 111, .. }));
}

#[test]
fn numeric_range_validates_shape_addresses_and_resource_bound() {
    for args in [
        vec![2_000_000, 1, 0, 0x10000],
        vec![2_000_000, 1, 0, 0, 2],
        vec![2_000_000, 1, 0, 0x10000, 10_000],
    ] {
        let raw = [command(121, &args, 0, &[])];
        let p = compile("invalid-range".into(), "".into(), source(), &raw).unwrap();
        assert!(matches!(p.code[0].op, Op::Unsupported { opcode: 121, .. }));
    }
    let p = compile("mixed-range".into(), "".into(), source(),
        &[command(121, &[1_600_004, 1, 0, 0x10000, 1], 0, &[])]).unwrap();
    assert!(matches!(&p.code[0].op, Op::ComputeRange { destinations, .. } if destinations.len() == 1));
    let p = compile("text-only-range".into(), "".into(), source(),
        &[command(121, &[1_600_005, 1, 0, 0x10000, 4], 0, &[])]).unwrap();
    assert!(matches!(&p.code[0].op, Op::Nop));
}
