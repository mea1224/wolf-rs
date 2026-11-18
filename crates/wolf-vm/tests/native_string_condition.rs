//! Raw 112 expectations come from A0D330/A0DF1A and the native branch selectors.
use wolf_format::{RawCommand, Span, WolfString};
use wolf_ir::{compile, Library, Op, Source, Target, Text, Var};
use wolf_vm::runtime::{Config, Vm, Wait};

fn c(opcode: u32, args: &[i32], indent: u8, strings: &[&str]) -> RawCommand {
    let span = Span { start: 0, end: 1 };
    RawCommand {
        span,
        opcode,
        args: args.to_vec(),
        indent,
        strings: strings
            .iter()
            .map(|s| WolfString {
                span,
                raw: [s.as_bytes(), &[0]].concat(),
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
        asset: "native-string-condition".into(),
        content_blake3: "synthetic".into(),
        event: 8,
        page: None,
        command: 0,
        span: [0, 1],
    }
}
fn branch(args: &[i32], strings: &[&str], count: usize) -> Vec<RawCommand> {
    let mut raw = vec![
        c(0, &[], 0, &[]),
        c(0, &[], 0, &[]),
        c(112, args, 0, strings),
    ];
    for n in 1..=count {
        raw.push(c(401, &[n as i32], 0, &[]));
        raw.push(c(121, &[2_000_001, n as i32, 0, 0], 1, &[]));
    }
    raw.push(c(420, &[0], 0, &[]));
    raw.push(c(121, &[2_000_001, 99, 0, 0], 1, &[]));
    raw.push(c(499, &[], 0, &[]));
    raw
}
fn execute(raw: &[RawCommand], lhs: &str, rhs: &str) -> Vm {
    let mut program = compile("common/8".into(), "".into(), source(), raw).unwrap();
    for (i, value) in [lhs, rhs].iter().enumerate() {
        program.code[i].op = Op::SetText {
            dst: Var::Common(805 + i as i32),
            value: Text::Literal((*value).into()),
            append: false,
        };
    }
    let mut vm = Vm::new(
        Library::new(vec![program]).unwrap(),
        Config::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("common/8".into()), 1, vec![]).unwrap();
    let report = vm.tick(vec![]).unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
    vm
}

#[test]
fn file_query_zero_and_one_select_the_actual_common8_branch() {
    let raw = branch(&[17, 1_600_005], &["0", "", "", ""], 1);
    for (lhs, expected) in [("0", 1), ("1", 99)] {
        assert_eq!(
            execute(&raw, lhs, "").state().world.globals.numbers[&1],
            expected
        );
    }
}

#[test]
fn native_modes_cover_empty_unicode_and_nonmatches() {
    for (mode, lhs, rhs, matched) in [
        (0, "日本語", "日本語", true),
        (0, "0", "1", false),
        (1, "abc", "abd", true),
        (1, "", "", false),
        (2, "a日本語b", "日本語", true),
        (2, "abc", "z", false),
        (3, "abcabc", "abc", true),
        (3, "zabc", "abc", false),
        (4, "abc", "z", true),
        (4, "abc", "", false),
        (5, "abcabc", "abc", true),
        (5, "abcz", "abc", false),
        (2, "abc", "", true),
        (3, "", "", true),
        (5, "abc", "", true),
    ] {
        let raw = branch(&[1, (mode << 28) | 1_600_005], &[rhs], 1);
        let vm = execute(&raw, lhs, "");
        assert_eq!(
            vm.state().world.globals.numbers[&1],
            if matched { 1 } else { 99 },
            "mode={mode}, lhs={lhs:?}, rhs={rhs:?}"
        );
    }
}

#[test]
fn rhs_variable_is_expanded_and_first_matching_branch_wins() {
    let raw = branch(
        &[0x12, 0x0100_0000 | 1_600_005, 1_600_005, 1_600_006, 0],
        &["unused", "abc"],
        2,
    );
    let vm = execute(&raw, "abc", r"\cself[5]");
    assert_eq!(vm.state().world.globals.numbers[&1], 1);
    let vm = execute(&raw, "abc", "different");
    assert_eq!(vm.state().world.globals.numbers[&1], 2);
    let raw = branch(&[1, 1_600_005], &[r"\cself[6]"], 1);
    assert_eq!(
        execute(&raw, "abc", "abc").state().world.globals.numbers[&1],
        1
    );
}

#[test]
fn malformed_operands_and_unimplemented_regex_remain_explicit_faults() {
    for args in [
        vec![1, 0x6000_0000 | 1_600_005],
        vec![1, 0x0100_0000 | 1_600_005],
        vec![1, 2_000_000],
        vec![0x21, 1_600_005],
    ] {
        let p = compile(
            "common/8".into(),
            "".into(),
            source(),
            &branch(&args, &["0"], 1),
        )
        .unwrap();
        assert!(matches!(p.code[2].op, Op::Unsupported { opcode: 112, .. }));
    }
}

#[test]
fn arithmetic_reads_string_operands_as_native_atoi_but_literal_flags_bypass_it() {
    for (lhs, rhs, expected) in [
        ("80", "20", 100),
        (" \t-12trailing", "+2", -10),
        ("nonnumeric", "", 0),
        ("3.9", "4", 7),
        ("2147483647", "0", 2_000_000_000),
    ] {
        let raw = [
            c(0, &[], 0, &[]),
            c(0, &[], 0, &[]),
            c(121, &[2_000_001, 1_600_005, 1_600_006, 0], 0, &[]),
        ];
        let vm = execute(&raw, lhs, rhs);
        assert_eq!(
            vm.state().world.globals.numbers[&1],
            expected,
            "{lhs:?} + {rhs:?}"
        );
    }
    let raw = [
        c(0, &[], 0, &[]),
        c(0, &[], 0, &[]),
        c(121, &[2_000_001, 1_600_005, 1_600_006, 12], 0, &[]),
    ];
    assert_eq!(
        execute(&raw, "80", "20").state().world.globals.numbers[&1],
        3_200_011
    );
}
