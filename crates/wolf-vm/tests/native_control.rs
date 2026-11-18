//! Expected results follow the static GamePro handlers, not Rust enum discriminants.
use wolf_format::{RawCommand, Span};
use wolf_ir::{compile, Library, Source, Target};
use wolf_vm::runtime::{Config, Vm, Wait};

fn c(opcode: u32, args: &[i32], indent: u8) -> RawCommand {
    RawCommand {
        span: Span { start: 0, end: 1 },
        opcode,
        args: args.to_vec(),
        indent,
        strings: vec![],
        terminator: 0,
        movement: None,
        extension_length: 0,
        extension: vec![],
    }
}

#[test]
fn common_self_variables_are_persistent_per_common_event_and_alias_modulo_100() {
    let make = |event, raw: &[RawCommand]| {
        let src = Source {
            asset: "BasicData/CommonEvent.dat".into(),
            content_blake3: "synthetic".into(),
            event,
            page: None,
            command: 0,
            span: [0, 0],
        };
        compile(format!("common/{event}"), "".into(), src, raw).unwrap()
    };
    let library = Library::new(vec![
        make(
            1,
            &[
                c(121, &[1_600_000, 7, 0, 0], 0),
                c(210, &[500_010, 0], 0),
                c(210, &[500_010, 0], 0),
            ],
        ),
        make(10, &[c(121, &[1_600_100, 1, 0, 0x100], 0)]),
    ])
    .unwrap();
    let mut vm = Vm::new(library.clone(), Config::default(), 1, 1).unwrap();
    vm.spawn(Target::Id("common/1".into()), 42, vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
    assert_eq!(vm.state().world.commons.numbers[&100], 7);
    assert_eq!(vm.state().world.commons.numbers[&1000], 2);
    let mut restored = Vm::restore(library, &vm.save().unwrap(), 2, 1 << 20).unwrap();
    assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    restored
        .spawn(Target::Id("common/10".into()), 99, vec![])
        .unwrap();
    restored.tick(vec![]).unwrap();
    assert_eq!(restored.state().world.commons.numbers[&1000], 3);
}
fn execute(raw: &[RawCommand], globals: &[(i32, i32)]) -> Vm {
    let source = Source {
        asset: "native-control-fixture".into(),
        content_blake3: "synthetic".into(),
        event: 1,
        page: None,
        command: 0,
        span: [0, 0],
    };
    let p = compile("main".into(), "".into(), source, raw).unwrap();
    let mut vm = Vm::new(Library::new(vec![p]).unwrap(), Config::default(), 1, 1).unwrap();
    for (key, value) in globals {
        vm.set_global(*key, *value).unwrap();
    }
    vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
    vm
}
fn branch(mode: i32, lhs: i32, rhs: i32) -> Vec<RawCommand> {
    vec![
        c(111, &[1, lhs, rhs, mode], 0),
        c(401, &[1], 0),
        c(121, &[2_000_001, 1, 0, 0], 1),
        c(420, &[0], 0),
        c(121, &[2_000_001, 2, 0, 0], 1),
        c(499, &[], 0),
    ]
}

#[test]
fn title_condition_count_word_high_bit_does_not_change_matching() {
    for (lhs, expected) in [(0, 1), (1, 2)] {
        let mut raw = branch(2, lhs, 0);
        raw[0].args[0] = 0x11;
        let vm = execute(&raw, &[]);
        assert_eq!(vm.state().world.globals.numbers[&1], expected);
    }
}

#[test]
fn numeric_range_is_inclusive_and_rereads_aliasing_sources() {
    let vm = execute(
        &[c(121, &[2_000_010, 2_000_010, 1, 0x10000, 2], 0)],
        &[(10, 4), (13, 99)],
    );
    // First destination changes the source from 4 to 5; later destinations see 5.
    assert_eq!(vm.state().world.globals.numbers[&10], 5);
    assert_eq!(vm.state().world.globals.numbers[&11], 6);
    assert_eq!(vm.state().world.globals.numbers[&12], 6);
    assert_eq!(vm.state().world.globals.numbers[&13], 99);
}

#[test]
fn indirect_destination_uses_current_common_context_and_updates_pointed_value() {
    let vm = execute(
        &[
            c(121, &[1_600_035, 2_000_040, 0, 4], 0),
            c(121, &[1_600_012, 73, 0, 0], 0),
            // Common89 PC38's raw form: destination comes from cself35.
            c(121, &[1_600_035, 1_600_012, 0, 16], 0),
            c(121, &[1_600_035, 2, 0, 0x110], 0),
            c(121, &[1_600_035, 1_600_020, 0, 4], 0),
            c(121, &[1_600_035, 91, 0, 16], 0),
        ],
        &[],
    );
    assert_eq!(vm.state().world.globals.numbers[&40], 75);
    assert_eq!(vm.state().world.commons.numbers[&120], 91);
    assert_eq!(vm.state().world.commons.numbers[&135], 1_600_020);
}

#[test]
fn indirect_destination_self_alias_resolves_before_write() {
    let vm = execute(
        &[
            c(121, &[2_000_010, 2_000_010, 0, 4], 0),
            c(121, &[2_000_010, 7, 0, 16], 0),
        ],
        &[],
    );
    assert_eq!(vm.state().world.globals.numbers[&10], 7);
}

#[test]
fn title_common_self_range_and_negative_maximum_follow_native_bounds() {
    let vm = execute(
        &[
            c(121, &[1_600_010, 1, 0, 0x10000, 1], 0),
            c(121, &[1_600_020, 7, 0, 0x10000, -5], 0),
        ],
        &[],
    );
    assert_eq!(vm.state().world.commons.numbers[&110], 1);
    assert_eq!(vm.state().world.commons.numbers[&111], 1);
    assert_eq!(vm.state().world.commons.numbers[&120], 7);
    assert!(!vm.state().world.commons.numbers.contains_key(&121));
}

#[test]
fn numeric_condition_modes_match_native_jump_table_including_signed_and_bits() {
    // mode, lhs, rhs, whether branch 1 executes; includes equal boundaries and negative values.
    for (mode, lhs, rhs, yes) in [
        (0, -1, -2, true),
        (0, 2, 2, false),
        (1, 2, 2, true),
        (1, -3, -2, false),
        (2, -5, -5, true),
        (2, 1, 2, false),
        (3, 2, 2, true),
        (3, 3, 2, false),
        (4, -2, -1, true),
        (4, 2, 2, false),
        (5, 1, 2, true),
        (5, -1, -1, false),
        (6, 0b1101, 0b1001, true),
        (6, 0b1101, 0b0010, false),
    ] {
        let vm = execute(&branch(mode, lhs, rhs), &[]);
        assert_eq!(
            vm.state().world.globals.numbers[&1],
            if yes { 1 } else { 2 },
            "mode {mode}, {lhs}, {rhs}"
        );
    }
}

#[test]
fn rhs_literal_flag_bypasses_variable_resolution() {
    let vars = [(0, 2_000_002), (2, 7)];
    let literal = execute(&branch(0x12, 2_000_000, 2_000_002), &vars);
    let indirect = execute(&branch(2, 2_000_000, 2_000_002), &vars);
    assert_eq!(literal.state().world.globals.numbers[&1], 1);
    assert_eq!(indirect.state().world.globals.numbers[&1], 2);
}

#[test]
fn first_matching_numeric_branch_wins() {
    let vm = execute(
        &[
            c(111, &[2, 5, 3, 0, 5, 9, 4], 0),
            c(401, &[1], 0),
            c(121, &[2_000_000, 11, 0, 0], 1),
            c(401, &[2], 0),
            c(121, &[2_000_000, 22, 0, 0], 1),
            c(499, &[], 0),
        ],
        &[],
    );
    assert_eq!(vm.state().world.globals.numbers[&0], 11);
}

#[test]
fn continue_decrements_the_innermost_counted_loop_and_skips_remaining_body() {
    let vm = execute(
        &[
            c(179, &[2], 0),
            c(179, &[3], 1),
            c(121, &[2_000_000, 1, 0, 0x100], 2),
            c(176, &[], 2),
            c(121, &[2_000_000, 1000, 0, 0x100], 2),
            c(498, &[], 1),
            c(121, &[2_000_001, 1, 0, 0x100], 1),
            c(498, &[], 0),
        ],
        &[],
    );
    assert_eq!(vm.state().world.globals.numbers[&0], 6);
    assert_eq!(vm.state().world.globals.numbers[&1], 2);
}

#[test]
fn zero_count_in_real_common_event_skips_disabled_body() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../wolf-ir/tests/fixtures/control.json")).unwrap();
    let raw: Vec<RawCommand> =
        serde_json::from_value(fixtures["cases"][0]["commands"].clone()).unwrap();
    let vm = execute(&raw, &[(10, 17)]);
    assert_eq!(vm.state().world.globals.numbers[&10], 17);
    let vm = execute(
        &[
            c(179, &[-1], 0),
            c(121, &[2_000_010, 1, 0, 0], 1),
            c(498, &[], 0),
        ],
        &[(10, 17)],
    );
    assert_eq!(vm.state().world.globals.numbers[&10], 17);
}

#[test]
fn ordinary_integer_arithmetic_uses_native_operator_numbers_and_saturation() {
    // binary mode, lhs, rhs, expected. Literal bits bypass the native address classifier.
    for (mode, lhs, rhs, expected) in [
        (0, 1_900_000_000, 200_000_000, 2_000_000_000),
        (1, -1_900_000_000, 200_000_000, -2_000_000_000),
        (2, 1_000_000, 3000, 2_000_000_000),
        (3, -7, 0, -7),
        (3, -7, 3, -2),
        (4, -7, 3, -1),
        (4, 7, 0, 0),
        (5, 13, 10, 8),
        (7, 13, 10, 15),
        (8, 13, 10, 7),
        (9, 3, 33, 6),
        (9, -8, -1, -4),
        (9, 17, i32::MIN, 17),
    ] {
        let vm = execute(&[c(121, &[2_000_000, lhs, rhs, (mode << 12) | 12], 0)], &[]);
        assert_eq!(
            vm.state().world.globals.numbers[&0],
            expected,
            "binary mode {mode}"
        );
    }
}

#[test]
fn ordinary_integer_update_table_and_final_small_clamp_are_explicit() {
    for (mode, old, rhs, expected) in [
        (0, 12, 7, 7),
        (1, 1_900_000_000, 200_000_000, 2_000_000_000),
        (2, -1_900_000_000, 200_000_000, -2_000_000_000),
        (3, 1_000_000, 3000, 2_000_000_000),
        (4, -7, 0, -7),
        (5, -7, 3, -1),
        (6, 12, 7, 12),
        (7, 12, 7, 7),
        (8, 100, -7, 7),
    ] {
        let vm = execute(
            &[c(121, &[2_000_000, rhs, 0, (mode << 8) | 12], 0)],
            &[(0, old)],
        );
        assert_eq!(
            vm.state().world.globals.numbers[&0],
            expected,
            "update mode {mode}"
        );
    }
    let vm = execute(&[c(121, &[2_000_000, 1_000_000, 0, 13], 0)], &[]);
    assert_eq!(vm.state().world.globals.numbers[&0], 999_999);
}

#[test]
fn floating_point_modes_remain_contextual_faults() {
    {
        let flags = 2;
        let raw = [c(121, &[2_000_000, 1, 2, flags], 0)];
        let src = Source {
            asset: "native-control-fixture".into(),
            content_blake3: "synthetic".into(),
            event: 7,
            page: None,
            command: 12,
            span: [0, 0],
        };
        let p = compile("main".into(), "".into(), src, &raw).unwrap();
        let mut vm = Vm::new(Library::new(vec![p]).unwrap(), Config::default(), 1, 1).unwrap();
        vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
        vm.tick(vec![]).unwrap();
        let f = &vm.state().fibers[&1];
        assert_eq!(f.wait, Wait::Fault);
        assert_eq!(f.frames[0].pc, 0);
        assert_eq!(
            f.fault.as_ref().unwrap().source.as_ref().unwrap().command,
            12
        );
    }
}

#[test]
fn native_indirect_operands_resolve_numbers_strings_and_common_context() {
    let raw = [
        c(121, &[2_000_010, 1_600_000, 2_000_001, 0x60], 0),
        c(121, &[2_000_011, 2_000_002, 0, 0x20], 0),
        c(121, &[2_000_012, 3_000_007, 0, 0x24], 0),
    ];
    let source = Source { asset: "native121".into(), content_blake3: "fixture".into(), event: 30, page: None, command: 0, span: [0,0] };
    let p = compile("main".into(), "".into(), source, &raw).unwrap();
    let mut vm = Vm::new(Library::new(vec![p]).unwrap(), Config::default(), 1, 1).unwrap();
    let w = vm.world_mut();
    w.commons.numbers.extend([(3000, 1_600_010), (3010, 17)]);
    w.globals.numbers.extend([(1, 3_000_007), (2, 15_003_005)]);
    w.globals.strings.insert(7, " -12suffix".into());
    w.commons.strings.insert(3005, "35suffix".into());
    vm.spawn(Target::Id("main".into()), 42, vec![]).unwrap();
    let report = vm.tick(vec![]).unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(vm.state().world.globals.numbers[&10], 5);
    assert_eq!(vm.state().world.globals.numbers[&11], 35);
    assert_eq!(vm.state().world.globals.numbers[&12], -12);
}

#[test]
fn native_indirect_invalid_address_faults_before_destination_write() {
    let source = Source { asset: "native121".into(), content_blake3: "fixture".into(), event: 30, page: None, command: 0, span: [0,0] };
    let p = compile("main".into(), "".into(), source, &[c(121, &[2_000_000, 17, 0, 0x20], 0)]).unwrap();
    let mut vm = Vm::new(Library::new(vec![p]).unwrap(), Config::default(), 1, 1).unwrap();
    vm.set_global(0, 99).unwrap();
    vm.spawn(Target::Id("main".into()), 42, vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 99);
    assert_eq!(vm.state().fibers[&1].wait, Wait::Fault);
}

#[test]
fn common_numeric_range_preserves_embedded_string_slots_and_reloads_operands() {
    let source = Source { asset: "CommonEvent.dat".into(), content_blake3: "fixture".into(), event: 81, page: None, command: 284, span: [0,0] };
    let p = compile("main".into(), "".into(), source,
        &[c(121, &[1_600_000, 1_600_000, 1, 0x10000, 53], 0)]).unwrap();
    let mut vm = Vm::new(Library::new(vec![p]).unwrap(), Config::default(), 1, 1).unwrap();
    for slot in 0..=54 { vm.world_mut().commons.numbers.insert(8100+slot, 90); }
    for slot in 5..=9 { vm.world_mut().commons.strings.insert(8100+slot, format!("keep{slot}")); }
    vm.spawn(Target::Id("main".into()), 42, vec![]).unwrap();
    let report = vm.tick(vec![]).unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let w = &vm.state().world;
    assert_eq!(w.commons.numbers[&8100], 91);
    for slot in 1..=53 {
        assert_eq!(w.commons.numbers[&(8100+slot)], if (5..=9).contains(&slot) {90} else {92});
    }
    assert_eq!(w.commons.numbers[&8154], 90);
    for slot in 5..=9 { assert_eq!(w.commons.strings[&(8100+slot)], format!("keep{slot}")); }
}

#[test]
fn native_angle_and_scaled_functions_follow_command_operands_and_units() {
    for (flags, x, y, expected) in [
        (0xf900, 1, 0, 0), (0xf900, 0, 1, 900), (0xf900, -1, 0, 1800),
        (0xf900, 0, -1, 2700), (0xf900, 1, 1, 450), (0xf900, 0, 0, 0),
        (0xa00, 0, 0, 0), (0xa00, 900, 0, 1000), (0xa00, -900, 0, -1000),
        (0xb00, 0, 0, 1000), (0xb00, 1800, 0, -1000),
        (0xc00, 9, 0, 3000), (0xc00, 2, 0, 1414), (0xc00, -1, 0, 0),
    ] {
        let vm = execute(&[c(121, &[2_000_000, x, y, flags], 0)], &[]);
        assert_eq!(vm.state().world.globals.numbers[&0], expected, "flags={flags:#x} x={x} y={y}");
    }
    let vm = execute(&[c(121, &[2_000_000, 2_000_001, 0, 0xc01], 0)], &[(1, 4_000_000)]);
    assert_eq!(vm.state().world.globals.numbers[&0], 999_999);
}
