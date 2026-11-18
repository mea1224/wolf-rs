use wolf_format::{RawCommand, Span, WolfString};
use wolf_ir::{native_startup::*, Source, Target, Text, Value, Var};

fn source(common: u32) -> Source {
    Source {
        asset: "BasicData/CommonEvent.dat".into(),
        content_blake3: String::new(),
        event: common,
        page: None,
        command: 0,
        span: [0, 0],
    }
}
fn command(opcode: u32, args: Vec<i32>, strings: &[&str]) -> RawCommand {
    RawCommand {
        opcode,
        args,
        indent: 0,
        span: Span { start: 0, end: 0 },
        strings: strings
            .iter()
            .map(|s| WolfString {
                raw: [s.as_bytes(), &[0]].concat(),
                span: Span { start: 0, end: 0 },
            })
            .collect(),
        extension: vec![],
        extension_length: 0,
        terminator: 0,
        movement: None,
    }
}

#[test]
fn map_page_name_does_not_shadow_common_call_target() {
    let common = wolf_ir::compile(
        "common/23".into(),
        "title".into(),
        source(23),
        &[command(0, vec![], &[])],
    )
    .unwrap();
    let mut map_source = source(1);
    map_source.page = Some(0);
    map_source.asset = "MapData/Map002.mps".into();
    let map = wolf_ir::compile(
        "MapData/Map002.mps/1/0".into(),
        "title".into(),
        map_source,
        &[command(0, vec![], &[])],
    )
    .unwrap();
    let library = wolf_ir::Library::new(vec![map, common]).unwrap();
    assert_eq!(
        library
            .resolve_common(&Target::Name("title".into()))
            .unwrap()
            .id,
        "common/23"
    );
}

#[test]
fn real_title_calls_preserve_target_and_caller_context() {
    // Common23 PC32 invokes config loader Common8 by name with numeric mode 1.
    let call = numeric_common_call(
        &command(300, vec![0, 1, 1], &["システムデータ"]),
        &source(23),
    )
    .unwrap();
    assert_eq!(call.target, Target::Name("システムデータ".into()));
    assert_eq!(call.arguments, vec![Value::Int(1)]);
    assert_eq!(
        numeric_common_parameter_slots(8, 1).unwrap(),
        vec![Var::Common(800)]
    );
    // A named call reading its caller's self slot must not resolve against callee.
    let call = numeric_common_call(
        &command(300, vec![0, 1, 1600010], &["タイトル画面"]),
        &source(22),
    )
    .unwrap();
    assert_eq!(call.arguments, vec![Value::Var(Var::Common(2210))]);
}

#[test]
fn real_guideline_key_guide_has_two_numbers_and_two_literal_strings() {
    let call = common_call(
        &command(
            300,
            vec![0, 12322, 10, 0, 0, 0],
            &["★☆キーガイド表示", "[決定]OK", "[決定]OK"],
        ),
        &source(23),
    )
    .unwrap();
    assert_eq!(call.numbers, vec![Value::Int(10), Value::Int(0)]);
    assert_eq!(
        call.strings,
        vec![
            Text::Literal("[決定]OK".into()),
            Text::Literal("[決定]OK".into())
        ]
    );
    assert_eq!(
        string_common_parameter_slots(89, 2).unwrap(),
        vec![Var::Common(8905), Var::Common(8906)]
    );
}

#[test]
fn indexed_string_reference_uses_numeric_operand_and_preserves_caller() {
    let call = common_call(
        &command(210, vec![500017, 0x11, 10, 1600005], &[]),
        &source(23),
    )
    .unwrap();
    assert_eq!(call.strings, vec![Text::Var(Var::Common(2305))]);
    assert!(common_call(
        &command(210, vec![500017, 0x2010, 0], &["", "a"]),
        &source(23)
    )
    .is_err());
}

#[test]
fn no_arguments_do_not_clear_persistent_self() {
    let call =
        numeric_common_call(&command(300, vec![0, 0], &["タイトル画面"]), &source(23)).unwrap();
    assert!(call.arguments.is_empty());
    assert!(numeric_common_parameter_slots(23, 0).unwrap().is_empty());
}

#[test]
fn rejects_strings_return_values_reserve_and_numeric_over_string_slots() {
    for (op, args) in [
        (211, vec![500008, 1, 1]),
        (210, vec![500008, 0x11, 1, 3000000]),
        (210, vec![500008, 0x100000, 2000000]),
        (210, vec![500008, 6, 1, 2, 3, 4, 5, 6]),
    ] {
        assert!(numeric_common_call(&command(op, args, &[]), &source(23)).is_err());
    }
    assert!(numeric_common_call(&command(300, vec![0, 0], &["\\cself[5]"]), &source(23)).is_err());
    assert!(numeric_common_parameter_slots(10_000, 1).is_err());
}

#[test]
fn string_slots_are_distinct_from_numeric_spaces_and_require_context() {
    assert_eq!(
        stored_text_address(1600005, &source(8)).unwrap(),
        Var::Common(805)
    );
    assert_eq!(
        stored_text_address(1600105, &source(8)).unwrap(),
        Var::Common(805)
    );
    assert_eq!(
        stored_text_address(15000805, &source(23)).unwrap(),
        Var::Common(805)
    );
    assert_eq!(
        stored_text_address(3000000, &source(8)).unwrap(),
        Var::Global(0)
    );
    assert!(stored_text_address(1600010, &source(8)).is_err());
    assert!(stored_text_address(9900071, &source(8)).is_err());
    let mut map = source(8);
    map.page = Some(0);
    assert!(stored_text_address(1600005, &map).is_err());
}
