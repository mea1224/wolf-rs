//! Call binding follows the native read/store loop; synthetic setup only seeds
//! persistent strings because opcode 122 expansion is a separate adapter.
use wolf_format::{RawCommand, Span, WolfString};
use wolf_ir::{compile, Library, Op, Program, Source, Target, Text, Var};
use wolf_vm::runtime::{Config, Vm, Wait};

fn c(opcode: u32, args: &[i32], indent: u8, strings: &[&str]) -> RawCommand {
    RawCommand {
        opcode,
        args: args.to_vec(),
        indent,
        span: Span { start: 0, end: 1 },
        strings: strings
            .iter()
            .map(|s| WolfString {
                raw: [s.as_bytes(), &[0]].concat(),
                span: Span { start: 0, end: 1 },
            })
            .collect(),
        terminator: 0,
        movement: None,
        extension_length: 0,
        extension: vec![],
    }
}
fn program(id: u32, raw: &[RawCommand]) -> Program {
    compile(
        format!("common/{id}"),
        format!("event{id}"),
        Source {
            asset: "call-test".into(),
            content_blake3: "synthetic".into(),
            event: id,
            page: None,
            command: 0,
            span: [0, 0],
        },
        raw,
    )
    .unwrap()
}

#[test]
fn native_return_waits_for_completion_and_survives_save_restore() {
    let caller = program(
        23,
        &[
            c(
                300,
                &[0, 0x1001012, 0, 1, 0, 1600024],
                0,
                &["event18", "Quit?"],
            ),
            c(121, &[2000000, 1600024, 0, 0], 0, &[]),
        ],
    );
    let callee = program(
        18,
        &[c(121, &[1600027, 1, 0, 0], 0, &[]), c(180, &[2], 0, &[])],
    );
    let library = Library::new(vec![caller, callee]).unwrap();
    let mut vm = Vm::new(
        library.clone(),
        Config {
            run_until_yield: true,
            native_common_returns: [(18, 27)].into(),
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("common/23".into()), 7, vec![]).unwrap();
    let tick = vm.tick(vec![]).unwrap();
    assert!(tick.diagnostics.is_empty(), "{:?}", tick.diagnostics);
    assert!(!vm.state().world.commons.numbers.contains_key(&2324));
    let saved = vm.save().unwrap();
    let mut vm = Vm::restore(library, &saved, 2, 1 << 20).unwrap();
    for _ in 0..3 {
        assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
    }
    assert_eq!(vm.state().world.commons.numbers.get(&2324), Some(&1));
    assert_eq!(vm.state().world.globals.numbers.get(&0), Some(&1));
}

#[test]
fn native_string_return_uses_metadata_and_disabled_return_preserves_destination() {
    for slot in [5, -1] {
        let caller = program(23, &[c(210, &[500018, 0x1000000, 1600006], 0, &[])]);
        let mut callee = program(18, &[c(0, &[], 0, &[])]);
        callee.code[0].op = Op::SetText {
            dst: Var::Common(1805),
            value: Text::Literal("answer".into()),
            append: false,
        };
        let mut vm = Vm::new(
            Library::new(vec![caller, callee]).unwrap(),
            Config {
                run_until_yield: true,
                native_common_returns: [(18, slot)].into(),
                ..Default::default()
            },
            1,
            1,
        )
        .unwrap();
        vm.spawn(Target::Id("common/23".into()), 7, vec![]).unwrap();
        let report = vm.tick(vec![]).unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert_eq!(
            vm.state()
                .world
                .commons
                .strings
                .get(&2306)
                .map(String::as_str),
            if slot == -1 { None } else { Some("answer") }
        );
    }
}

#[test]
fn recursive_numeric_and_string_aliases_bind_in_order_and_restore_active_stack() {
    let mut p = program(
        10,
        &[
            c(111, &[1, 1600000, 0, 2], 0, &[]),
            c(401, &[1], 0, &[]),
            c(121, &[1600000, 7, 0, 0], 1, &[]),
            c(0, &[], 1, &[]),                    // synthetic initial string
            c(121, &[1600010, 77, 0, 0], 1, &[]), // omitted self slot survives
            // 2 numeric + 2 strings; string0 literal, string1 references self5.
            c(
                210,
                &[500010, 0x1022, 9, 1600000, 0, 1600005],
                1,
                &["", "new", ""],
            ),
            c(0, &[], 1, &[]),
            c(499, &[], 0, &[]),
            c(180, &[2], 0, &[]),
        ],
    );
    p.code[3].op = Op::SetText {
        dst: Var::Common(1005),
        value: Text::Literal("old".into()),
        append: false,
    };
    let library = Library::new(vec![p]).unwrap();
    let mut vm = Vm::new(library.clone(), Config::default(), 1, 1).unwrap();
    vm.spawn(Target::Id("common/10".into()), 42, vec![])
        .unwrap();
    vm.tick(vec![]).unwrap();
    let common = &vm.state().world.commons;
    assert_eq!(common.numbers[&1000], 9);
    assert_eq!(
        common.numbers[&1001], 9,
        "second argument reads first argument's new self0"
    );
    assert_eq!(common.numbers[&1010], 77);
    assert_eq!(common.strings[&1005], "new");
    assert_eq!(
        common.strings[&1006], "new",
        "reference sees first string's new self5"
    );
    assert_eq!(vm.state().fibers[&1].frames.len(), 2);
    assert!(vm.state().fibers[&1].frames[1].locals.numbers.is_empty());
    let mut restored = Vm::restore(library, &vm.save().unwrap(), 2, 1 << 20).unwrap();
    for _ in 0..8 {
        vm.tick(vec![]).unwrap();
        restored.tick(vec![]).unwrap();
        assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    }
    assert_eq!(restored.state().fibers[&1].wait, Wait::Done);
}

#[test]
fn named_calls_keep_map_owner_and_callee_common_context() {
    let caller = program(2, &[c(300, &[0, 1, 6], 0, &["event8"])]);
    let callee = program(8, &[c(121, &[1100000, 1600000, 0, 0], 0, &[])]);
    let mut vm = Vm::new(
        Library::new(vec![caller, callee]).unwrap(),
        Config::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("common/2".into()), 42, vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.events[&42].numbers[&0], 6);
    assert_eq!(vm.state().world.commons.numbers[&800], 6);
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
}

#[test]
fn saved_named_common_call_ignores_map_event_with_the_same_label() {
    let caller = program(2, &[c(300, &[0, 1, 6], 0, &["event8"])]);
    let callee = program(8, &[c(180, &[3], 0, &[]), c(121, &[1100000, 1600000, 0, 0], 0, &[])]);
    let mut map = program(99, &[c(0, &[], 0, &[])]);
    map.id = "map/1/0/0".into();
    map.name = "event8".into();
    let library = Library::new(vec![caller, callee, map]).unwrap();
    assert!(library.resolve(&Target::Name("event8".into())).is_none());
    let mut vm = Vm::new(library.clone(), Config::default(), 1, 1).unwrap();
    vm.spawn(Target::Id("common/2".into()), 42, vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().fibers[&1].frames.len(), 2);
    let saved = vm.save().unwrap();
    let mut wrong_callee: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    wrong_callee["fibers"]["1"]["frames"][1]["program"] = serde_json::json!("common/2");
    let error = Vm::restore(library.clone(), &serde_json::to_vec(&wrong_callee).unwrap(), 2, 1 << 20).err().expect("wrong callee accepted");
    assert!(error.contains("saved call stack does not match program"), "{error}");
    let mut restored = Vm::restore(library, &saved, 2, 1 << 20).unwrap();
    for _ in 0..6 { restored.tick(vec![]).unwrap(); }
    assert_eq!(restored.state().world.events[&42].numbers[&0], 6);
    assert_eq!(restored.state().fibers[&1].wait, Wait::Done);
}

#[test]
fn unsupported_expansion_faults_at_call_without_entering_callee() {
    let caller = program(2, &[c(210, &[500008, 0x1010, 0], 0, &["", "\\unknown[5]"])]);
    let callee = program(8, &[c(121, &[2000000, 1, 0, 0], 0, &[])]);
    let mut vm = Vm::new(
        Library::new(vec![caller, callee]).unwrap(),
        Config::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("common/2".into()), 42, vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    let fiber = &vm.state().fibers[&1];
    assert_eq!(fiber.wait, Wait::Fault);
    assert_eq!(fiber.frames.len(), 1);
    assert_eq!(fiber.frames[0].pc, 0);
    assert!(fiber.fault.as_ref().unwrap().message.contains("expansion"));
    assert!(!vm.state().world.globals.numbers.contains_key(&0));
}

#[test]
fn native_call_expands_strings_in_caller_common_context() {
    let caller = program(
        2,
        &[
            c(122, &[1600005, 0, 0], 0, &["caller text"]),
            c(210, &[500008, 0x1010, 0], 0, &["", "\\cself[5]"]),
        ],
    );
    let callee = program(8, &[c(121, &[2000000, 1, 0, 0], 0, &[])]);
    let mut vm = Vm::new(
        Library::new(vec![caller, callee]).unwrap(),
        Config::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("common/2".into()), 42, vec![]).unwrap();
    vm.tick_with_host(vec![], &mut wolf_vm::native_commands::execute_values)
        .unwrap();
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
    assert_eq!(vm.state().world.commons.strings[&805], "caller text");
    assert_eq!(vm.state().world.globals.numbers[&0], 1);
}
