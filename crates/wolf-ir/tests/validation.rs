use wolf_ir::*;

fn program(ops: Vec<Op>) -> Program {
    Program {
        id: "invalid-ir".into(),
        name: "".into(),
        profile: PROFILE.into(),
        code: ops
            .into_iter()
            .enumerate()
            .map(|(pc, op)| Instruction {
                op,
                raw: None,
                source: Source {
                    asset: "synthetic".into(),
                    content_blake3: "synthetic".into(),
                    event: 0,
                    page: None,
                    command: pc,
                    span: [pc, pc + 1],
                },
            })
            .collect(),
    }
}

#[test]
fn previous_profile_cannot_bypass_native_semantic_version() {
    let mut previous = program(vec![Op::Halt]);
    previous.profile = "wolf350-evidence-subset-v11".into();
    assert!(Library::new(vec![previous]).is_err());
    assert_eq!(PROFILE, "wolf350-evidence-subset-v12");
}
#[test]
fn rejects_crossing_loops_or_references_to_unrelated_ends() {
    for ops in [
        vec![
            Op::Loop {
                count: None,
                end: 4,
            },
            Op::Loop {
                count: None,
                end: 5,
            },
            Op::Nop,
            Op::Next { start: 0 },
            Op::Next { start: 1 },
        ],
        vec![
            Op::Loop {
                count: None,
                end: 3,
            },
            Op::Next { start: 0 },
            Op::Nop,
        ],
        vec![
            Op::Loop {
                count: None,
                end: 4,
            },
            Op::Break { start: 0, end: 3 },
            Op::Nop,
            Op::Next { start: 0 },
        ],
    ] {
        assert!(Library::new(vec![program(ops)]).is_err());
    }
}
#[test]
fn rejects_unbounded_expressions_and_invalid_payloads_before_execution() {
    let mut v = Value::Int(0);
    for _ in 0..18 {
        v = Value::Indirect(Box::new(v));
    }
    let mut condition = Condition::Number {
        lhs: Value::Int(0),
        cmp: Compare::Eq,
        rhs: Value::Int(0),
    };
    for _ in 0..34 {
        condition = Condition::All(vec![condition]);
    }
    for op in [
        Op::Wait(v),
        Op::Branch {
            cases: vec![(condition, 1)],
            fallback: 1,
        },
        Op::SetText {
            dst: Var::Global(-1),
            value: Text::Literal("".into()),
            append: false,
        },
        Op::Request(Service::Choice {
            options: vec!["one".into()],
            targets: vec![],
            cancel: None,
        }),
        Op::Jump(2),
    ] {
        assert!(Library::new(vec![program(vec![op])]).is_err());
    }
    let mut p = program(vec![Op::Nop]);
    p.code[0].source.span = [2, 1];
    assert!(p.validate().is_err());
}
#[test]
fn numeric_address_decoder_rejects_known_string_classes() {
    for n in [
        1_600_005, 1_600_009, 3_000_000, 9_100_009, 9_199_999, 9_900_000, 15_001_005,
    ] {
        assert!(
            value(n).is_err(),
            "numeric adapter accepted string address {n}"
        );
    }
    assert!(value(1_600_004).is_err());
    assert_eq!(value(15_001_004).unwrap(), Value::Var(Var::Common(1004)));
    assert_eq!(value(1_100_101).unwrap(), Value::Var(Var::Event(1)));
    assert!(value(9_100_000).is_err());
}
