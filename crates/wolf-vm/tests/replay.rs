use wolf_ir::{Instruction, Library, Op, Program, Service, Source, Target, PROFILE};
use wolf_vm::{
    replay::{Input, Recorder, Tape},
    runtime::{Completion, Config, Response, Token, Vm},
};

fn library() -> Library {
    Library::new(vec![Program {
        id: "main".into(),
        name: "main".into(),
        profile: PROFILE.into(),
        code: vec![
            Op::Request(Service::Message {
                text: "test".into(),
            }),
            Op::Halt,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, op)| Instruction {
            source: Source {
                asset: "synthetic".into(),
                content_blake3: "fixture".into(),
                event: 0,
                page: None,
                command: i,
                span: [i, i + 1],
            },
            raw: None,
            op,
        })
        .collect(),
    }])
    .unwrap()
}
fn record() -> (Vm, Tape) {
    let mut vm = Vm::new(library(), Config::default(), 7, 42).unwrap();
    vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    let mut recorder = Recorder::new(vm, 3).unwrap();
    recorder.tick(Input::default()).unwrap();
    let token = recorder.vm().requests().next().unwrap().token;
    let report = recorder
        .tick(Input {
            keyboard: None,
            globals: vec![(1, 17), (1, 18)],
            systems: vec![(2, 99)],
            completions: vec![Completion {
                token: Token {
                    session: 6,
                    sequence: token.sequence,
                },
                response: Response::Done,
            }],
        })
        .unwrap();
    assert_eq!(report.stale_completions, 1);
    recorder
        .tick(Input {
            completions: vec![Completion {
                token,
                response: Response::Done,
            }],
            ..Input::default()
        })
        .unwrap();
    let before = recorder.vm().state_hash().unwrap();
    assert!(recorder
        .tick(Input {
            globals: vec![(1, 999)],
            ..Input::default()
        })
        .is_err());
    assert_eq!(before, recorder.vm().state_hash().unwrap());
    recorder.finish()
}
#[test]
fn recorded_inputs_and_stale_completions_replay_across_sessions() {
    let (vm, tape) = record();
    let bytes = tape.to_bytes(1 << 20).unwrap();
    let restored = Tape::from_bytes(&bytes, 1 << 20, 3)
        .unwrap()
        .verify(library(), 8, 1 << 20)
        .unwrap();
    assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    assert_eq!(restored.state().world.globals.numbers[&1], 18);
    assert!(tape.verify(library(), 6, 1 << 20).is_err());
    assert!(tape.verify(library(), 7, 1 << 20).is_err());
    assert!(Tape::from_bytes(&bytes, bytes.len() - 1, 3).is_err());
    assert!(Tape::from_bytes(&bytes, bytes.len(), 2).is_err());
}
#[test]
fn modified_input_identifies_first_divergent_tick() {
    let (_, mut tape) = record();
    tape.frames[1].input.globals.push((1, -1));
    let error = tape.verify(library(), 8, 1 << 20).err().unwrap();
    assert!(error.contains("divergence at tick 2"), "{error}");
    tape.frames[1].tick = 4;
    let error = tape.verify(library(), 8, 1 << 20).err().unwrap();
    assert!(error.contains("nonconsecutive tick"), "{error}");
}

#[test]
fn failed_input_preserves_state_and_recorder_can_continue() {
    let vm = Vm::new(library(), Config::default(), 7, 42).unwrap();
    let initial_hash = vm.state_hash().unwrap();
    let mut recorder = Recorder::new(vm, 3).unwrap();
    assert!(recorder
        .tick(Input {
            globals: vec![(1, 18), (-1, 99)],
            ..Input::default()
        })
        .is_err());
    assert_eq!(recorder.vm().state_hash().unwrap(), initial_hash);
    recorder.tick(Input::default()).unwrap();
    let (vm, tape) = recorder.finish();
    assert_eq!(tape.frames.len(), 1);
    assert_eq!(
        tape.verify(library(), 8, 1 << 20)
            .unwrap()
            .state_hash()
            .unwrap(),
        vm.state_hash().unwrap()
    );
}

#[test]
fn slot_and_completion_limits_reject_before_any_variable_write() {
    let config = Config {
        max_slots: 1,
        max_completions_per_tick: 1,
        ..Config::default()
    };
    let mut vm = Vm::new(library(), config, 7, 42).unwrap();
    vm.set_system(0, 1).unwrap();
    let mut recorder = Recorder::new(vm, 10).unwrap();
    let before = recorder.vm().state_hash().unwrap();
    for input in [
        Input {
            globals: vec![(0, 1)],
            systems: vec![(1, 2)],
            ..Input::default()
        },
        Input {
            globals: vec![(0, 1), (1, 2)],
            ..Input::default()
        },
        Input {
            globals: vec![(0, 1)],
            completions: vec![
                Completion {
                    token: Token {
                        session: 7,
                        sequence: 1
                    },
                    response: Response::Done
                };
                2
            ],
            ..Input::default()
        },
    ] {
        assert!(recorder.tick(input).is_err());
        assert_eq!(recorder.vm().state_hash().unwrap(), before);
    }
    recorder
        .tick(Input {
            globals: vec![(0, 1), (0, 2)],
            systems: vec![(0, 3)],
            ..Input::default()
        })
        .unwrap();
    assert_eq!(recorder.vm().state().world.globals.numbers[&0], 2);
    let (vm, tape) = recorder.finish();
    assert_eq!(
        tape.verify(library(), 8, 1 << 20)
            .unwrap()
            .state_hash()
            .unwrap(),
        vm.state_hash().unwrap()
    );
}

fn decode(value: &serde_json::Value) -> Result<Tape, String> {
    let bytes = serde_json::to_vec(value).unwrap();
    Tape::from_bytes(&bytes, 1 << 20, 100)
}

#[test]
fn decoder_rejects_unknown_missing_duplicate_and_invalid_hash_fields() {
    let (_, tape) = record();
    let original = serde_json::to_value(&tape).unwrap();
    for field in ["version", "initial_save", "initial_state_hash", "frames"] {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove(field);
        assert!(decode(&value).is_err(), "missing {field}");
    }
    let mut value = original.clone();
    value["extra"] = true.into();
    assert!(decode(&value).is_err());
    let mut value = original.clone();
    value["frames"][0]["extra"] = true.into();
    assert!(decode(&value).is_err());
    let mut value = original.clone();
    value["frames"][0]["input"]["extra"] = true.into();
    assert!(decode(&value).is_err());
    let mut value = original.clone();
    value["frames"][1]["input"]["completions"][0]["extra"] = true.into();
    assert!(decode(&value).is_err());
    let mut value = original.clone();
    value["frames"][1]["input"]["completions"][0]["token"]["extra"] = true.into();
    assert!(decode(&value).is_err());
    for hash in ["x", &"A".repeat(64), &"g".repeat(64)] {
        let mut value = original.clone();
        value["initial_state_hash"] = hash.into();
        assert!(decode(&value).is_err());
        value["initial_state_hash"] = original["initial_state_hash"].clone();
        value["frames"][0]["state_hash"] = hash.into();
        assert!(decode(&value).is_err());
    }
    let mut value = original.clone();
    value["version"] = 1.into();
    assert!(decode(&value).is_err());
    let bytes = tape.to_bytes(1 << 20).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let duplicate = text.replacen('{', "{\"version\":3,", 1);
    assert!(Tape::from_bytes(duplicate.as_bytes(), 1 << 20, 100)
        .unwrap_err()
        .contains("duplicate"));
    let trailing = format!("{text} {{}}");
    assert!(Tape::from_bytes(trailing.as_bytes(), 1 << 20, 100).is_err());
}

#[test]
fn byte_and_frame_limits_are_enforced_during_encoding_and_decoding() {
    let (_, tape) = record();
    let bytes = tape.to_bytes(1 << 20).unwrap();
    assert_eq!(tape.to_bytes(bytes.len()).unwrap(), bytes);
    assert!(tape
        .to_bytes(bytes.len() - 1)
        .unwrap_err()
        .contains("replay byte limit"));
    assert!(tape.to_bytes(0).is_err());
    // The fourth frame is intentionally malformed. The decoder must stop at the
    // caller's frame limit before trying to deserialize that element.
    let mut value = serde_json::to_value(&tape).unwrap();
    value["frames"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("not a frame"));
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(Tape::from_bytes(&bytes, 1 << 20, 3)
        .unwrap_err()
        .contains("replay collection limit"));
    assert!(Tape::from_bytes(&bytes, 1 << 20, 4)
        .unwrap_err()
        .contains("invalid type"));
    assert!(Recorder::new(
        Vm::new(library(), Config::default(), 7, 42).unwrap(),
        1_000_001
    )
    .is_err());
}

#[test]
fn empty_tape_checks_initial_save_hash_and_restore_validation() {
    let vm = Vm::new(library(), Config::default(), 7, 42).unwrap();
    let (_, tape) = Recorder::new(vm, 0).unwrap().finish();
    let original = serde_json::to_value(&tape).unwrap();
    let saved: Vec<u8> = serde_json::from_value(original["initial_save"].clone()).unwrap();
    let mut saved: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    saved["rng"] = 43.into();
    let mut value = original.clone();
    value["initial_save"] = serde_json::to_value(serde_json::to_vec(&saved).unwrap()).unwrap();
    assert!(decode(&value)
        .unwrap()
        .verify(library(), 8, 1 << 20)
        .err()
        .unwrap()
        .contains("initial state hash mismatch"));
    saved["next_token"] = 0.into();
    value["initial_save"] = serde_json::to_value(serde_json::to_vec(&saved).unwrap()).unwrap();
    assert!(decode(&value)
        .unwrap()
        .verify(library(), 8, 1 << 20)
        .is_err());
    value["initial_save"] = serde_json::json!([255]);
    assert!(decode(&value)
        .unwrap()
        .verify(library(), 8, 1 << 20)
        .is_err());
    assert!(tape.verify(library(), 8, 0).is_err());
}

#[test]
fn new_session_must_not_reactivate_a_recorded_future_stale_token() {
    let mut vm = Vm::new(library(), Config::default(), 7, 42).unwrap();
    vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    let mut recorder = Recorder::new(vm, 3).unwrap();
    recorder.tick(Input::default()).unwrap();
    let token = recorder.vm().requests().next().unwrap().token;
    let report = recorder
        .tick(Input {
            completions: vec![Completion {
                token: Token {
                    session: 8,
                    ..token
                },
                response: Response::Done,
            }],
            ..Input::default()
        })
        .unwrap();
    assert_eq!(report.stale_completions, 1);
    recorder
        .tick(Input {
            completions: vec![Completion {
                token,
                response: Response::Done,
            }],
            ..Input::default()
        })
        .unwrap();
    let (vm, tape) = recorder.finish();
    assert!(tape
        .verify(library(), 8, 1 << 20)
        .err()
        .unwrap()
        .contains("session collides"));
    assert_eq!(
        tape.verify(library(), 9, 1 << 20)
            .unwrap()
            .state_hash()
            .unwrap(),
        vm.state_hash().unwrap()
    );
}

#[test]
fn duplicate_completions_replay_as_rejections() {
    let mut vm = Vm::new(library(), Config::default(), 7, 42).unwrap();
    vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    let mut recorder = Recorder::new(vm, 3).unwrap();
    recorder.tick(Input::default()).unwrap();
    let token = recorder.vm().requests().next().unwrap().token;
    let report = recorder
        .tick(Input {
            completions: vec![
                Completion {
                    token,
                    response: Response::Done
                };
                2
            ],
            ..Input::default()
        })
        .unwrap();
    assert_eq!(report.rejected_completions, 2);
    assert_eq!(recorder.vm().requests().count(), 1);
    let (vm, tape) = recorder.finish();
    assert_eq!(
        tape.verify(library(), 8, 1 << 20)
            .unwrap()
            .state_hash()
            .unwrap(),
        vm.state_hash().unwrap()
    );
}

#[test]
fn exhausted_tick_rejects_before_input_changes() {
    let vm = Vm::new(library(), Config::default(), 7, 42).unwrap();
    let mut saved: serde_json::Value = serde_json::from_slice(&vm.save().unwrap()).unwrap();
    saved["tick"] = u64::MAX.into();
    let vm = Vm::restore(library(), &serde_json::to_vec(&saved).unwrap(), 8, 1 << 20).unwrap();
    let before = vm.state_hash().unwrap();
    let mut recorder = Recorder::new(vm, 1).unwrap();
    assert!(recorder
        .tick(Input {
            globals: vec![(1, 123)],
            ..Input::default()
        })
        .unwrap_err()
        .contains("tick exhausted"));
    assert_eq!(recorder.vm().state_hash().unwrap(), before);
    let (_, tape) = recorder.finish();
    assert!(tape.frames.is_empty());
    assert_eq!(
        tape.verify(library(), 9, 1 << 20)
            .unwrap()
            .state_hash()
            .unwrap(),
        before
    );
}

#[test]
fn completion_wire_variants_roundtrip_without_changing_representation() {
    let (_, mut tape) = record();
    tape.frames[1].input.completions = [
        Response::Done,
        Response::Choice(3),
        Response::Cancel,
        Response::Failed("\"错误\"\n\\".into()),
    ]
    .into_iter()
    .enumerate()
    .map(|(sequence, response)| Completion {
        token: Token {
            session: 6,
            sequence: sequence as u64,
        },
        response,
    })
    .collect();
    let bytes = tape.to_bytes(1 << 20).unwrap();
    let decoded = Tape::from_bytes(&bytes, 1 << 20, 3).unwrap();
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        serde_json::to_value(&tape).unwrap()
    );
}
