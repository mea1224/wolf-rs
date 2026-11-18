use wolf_format::{RawCommand, Span, WolfString};
use wolf_ir::{Instruction, Library, Op, Program, Source, Target, PROFILE};
use wolf_vm::native_commands::{execute_values, split_native_line, transform_plain_string};
use wolf_vm::runtime::{Config, Vm};

#[test]
fn plain_text_transform_follows_native_delimiter_and_replacement_rules() {
    assert_eq!(
        transform_plain_string(0x800, "aababa", "ab", None).unwrap(),
        "aa"
    );
    assert_eq!(
        transform_plain_string(0x900, "a=a", "a", Some("aa")).unwrap(),
        "aa=aa"
    );
    assert_eq!(
        transform_plain_string(0xA00, "言語=日本語=1", "=", None).unwrap(),
        "言語"
    );
    assert_eq!(
        transform_plain_string(0xB00, "言語=日本語=1", "=", None).unwrap(),
        "日本語=1"
    );
    assert_eq!(
        transform_plain_string(0xB00, "plain", "=", None).unwrap(),
        "<<NotFound>>"
    );
    assert!(transform_plain_string(0x800, "plain", "", None).is_err());
    assert_eq!(
        split_native_line("language=1\nvolume=80\n"),
        ("language=1", "volume=80\n")
    );
    assert_eq!(split_native_line("last"), ("last", ""));
}

fn string_instruction(pc: usize, args: &[i32], strings: &[&str]) -> Instruction {
    let span = Span { start: 0, end: 1 };
    Instruction {
        source: Source {
            asset: "native-values-test".into(),
            content_blake3: "synthetic".into(),
            event: 8,
            page: None,
            command: pc,
            span: [0, 1],
        },
        raw: Some(RawCommand {
            span,
            opcode: 122,
            args: args.to_vec(),
            indent: 0,
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
        }),
        op: Op::NativeHost,
    }
}

#[test]
fn config_line_read_consumes_source_and_keeps_destination_common_scope() {
    let program = Program {
        id: "common/8".into(),
        name: "config".into(),
        profile: PROFILE.into(),
        code: vec![
            string_instruction(0, &[1600006, 0], &["language=1\nvolume=80\n"]),
            // Actual Common8 pc177 form: write the first line to self7 and consume self6.
            string_instruction(1, &[1600007, 0x301, 1600006], &[]),
            string_instruction(2, &[1600008, 1, 1600007], &[]),
            string_instruction(3, &[1600008, 0xB00, 0], &["="]),
            // Text assignments to a numeric self slot follow native atoi.
            string_instruction(4, &[1600013, 1, 1600008], &[]),
        ],
    };
    let mut vm = Vm::new(
        Library::new(vec![program]).unwrap(),
        Config::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("common/8".into()), 1, vec![]).unwrap();
    let report = vm.tick_with_host(vec![], &mut execute_values).unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let slots = &vm.state().world.commons;
    assert_eq!(slots.strings[&806], "volume=80\n");
    assert_eq!(slots.strings[&807], "language=1");
    assert_eq!(slots.strings[&808], "1");
    assert_eq!(slots.numbers[&813], 1);
}

#[test]
fn dialogue_character_consumer_handles_utf8_empty_and_source_destination_alias() {
    let program = Program {
        id: "common/8".into(),
        name: "characters".into(),
        profile: PROFILE.into(),
        code: vec![
            string_instruction(0, &[1600006, 0], &["…🙂A"]),
            string_instruction(1, &[1600007, 0x401, 1600006], &[]),
            string_instruction(2, &[1600008, 0x401, 1600006], &[]),
            string_instruction(3, &[1600006, 0x401, 1600006], &[]),
            string_instruction(4, &[1600009, 0x400, 0], &[""]),
        ],
    };
    let mut vm = Vm::new(
        Library::new(vec![program]).unwrap(),
        Config::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("common/8".into()), 1, vec![]).unwrap();
    let report = vm.tick_with_host(vec![], &mut execute_values).unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let strings = &vm.state().world.commons.strings;
    assert_eq!(strings[&807], "…");
    assert_eq!(strings[&808], "🙂");
    assert_eq!(strings[&806], "A");
    assert_eq!(strings[&809], "");
}
