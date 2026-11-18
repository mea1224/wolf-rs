use wolf_ir::*;
use wolf_vm::runtime::*;
fn source(pc: usize) -> Source {
    Source {
        asset: "fixture".into(),
        content_blake3: "synthetic".into(),
        event: 1,
        page: Some(0),
        command: pc,
        span: [pc, pc + 1],
    }
}
fn program(id: &str, ops: Vec<Op>) -> Program {
    Program {
        id: id.into(),
        name: id.into(),
        profile: PROFILE.into(),
        code: ops
            .into_iter()
            .enumerate()
            .map(|(pc, op)| Instruction {
                source: source(pc),
                raw: None,
                op,
            })
            .collect(),
    }
}
fn compute(dst: Var, a: Value, binary: Binary, b: i32, update: Update) -> Op {
    Op::Compute {
        dst,
        lhs: a,
        binary,
        rhs: Value::Int(b),
        update,
        arithmetic: Arithmetic::Wrapping,
    }
}
fn set(n: i32, v: i32) -> Op {
    compute(Var::Global(n), Value::Int(v), Binary::Add, 0, Update::Set)
}
fn vm(programs: Vec<Program>, budget: u32) -> Vm {
    Vm::new(
        Library::new(programs).unwrap(),
        Config {
            instruction_budget: budget,
            ..Default::default()
        },
        7,
        42,
    )
    .unwrap()
}
fn start(vm: &mut Vm, id: &str) -> u64 {
    vm.spawn(Target::Id(id.into()), 10, vec![]).unwrap()
}

#[test]
fn native_label_exit_preserves_outer_count_and_entered_loop_survives_save() {
    let p = program("labels", vec![
        Op::Loop { count: Some(Value::Int(3)), end: 8 },
        Op::Loop { count: None, end: 5 },
        Op::NativeLabelJump { target: 5, loops: vec![0] },
        set(9, -1),
        Op::Next { start: 1 },
        compute(Var::Global(0), Value::Int(1), Binary::Add, 0, Update::Add),
        Op::Wait(Value::Int(1)),
        Op::Next { start: 0 },
        Op::NativeLabelJump { target: 10, loops: vec![9] },
        Op::Loop { count: Some(Value::Int(99)), end: 13 },
        Op::Wait(Value::Int(1)),
        compute(Var::Global(1), Value::Int(1), Binary::Add, 0, Update::Add),
        Op::Next { start: 9 },
        Op::Halt,
    ]);
    let library = Library::new(vec![p.clone()]).unwrap();
    let mut runner = vm(vec![p], 100);
    start(&mut runner, "labels");
    for tick in 0..60 {
        runner.tick(vec![]).unwrap();
        assert!(runner.state().fibers.values().all(|f| f.fault.is_none()));
        runner = Vm::restore(library.clone(), &runner.save().unwrap(), tick+100, 1<<20).unwrap();
    }
    assert_eq!(runner.state().world.globals.numbers.get(&0), Some(&3));
    assert_eq!(runner.state().world.globals.numbers.get(&1), Some(&1));
    assert!(!runner.state().world.globals.numbers.contains_key(&9));
}
#[test]
fn native_slot_order_runs_main_until_wait_before_parallel_observers() {
    let library = Library::new(vec![
        program(
            "main",
            vec![
                set(0, 1),
                set(0, 2),
                Op::Wait(Value::Int(1)),
                set(0, 3),
                Op::Halt,
            ],
        ),
        program(
            "parallel",
            vec![
                compute(
                    Var::Global(1),
                    Value::Var(Var::Global(0)),
                    Binary::Add,
                    0,
                    Update::Set,
                ),
                compute(
                    Var::Global(2),
                    Value::Var(Var::System(13)),
                    Binary::Add,
                    0,
                    Update::Set,
                ),
                Op::Halt,
            ],
        ),
    ])
    .unwrap();
    let mut vm = Vm::new(
        library,
        Config {
            run_until_yield: true,
            native_main_fiber: Some(1),
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    start(&mut vm, "main");
    let parallel = start(&mut vm, "parallel");
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&1], 2);
    assert_eq!(vm.state().world.globals.numbers[&2], 1);
    vm.replace_page(parallel, Target::Id("parallel".into()))
        .unwrap();
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&1], 3);
    assert_eq!(vm.state().world.globals.numbers[&2], 0);
    assert_eq!(vm.state().fibers[&parallel].page_generation, 1);
}
#[test]
fn counted_loop_calls_locals_and_waits_survive_save_restore() {
    let programs = vec![
        program(
            "main",
            vec![
                set(0, 0),
                Op::Loop {
                    count: Some(Value::Int(3)),
                    end: 4,
                },
                Op::Call {
                    target: Target::Name("callee".into()),
                    args: vec![Value::Int(4)],
                },
                Op::Next { start: 1 },
                Op::Halt,
            ],
        ),
        program(
            "callee",
            vec![
                Op::Wait(Value::Int(2)),
                compute(
                    Var::Global(0),
                    Value::Var(Var::Local(0)),
                    Binary::Add,
                    0,
                    Update::Add,
                ),
                Op::Return,
            ],
        ),
    ];
    let library = Library::new(programs.clone()).unwrap();
    let mut a = vm(programs, 100);
    start(&mut a, "main");
    a.tick(vec![]).unwrap();
    assert_eq!(a.state().fibers[&1].frames.len(), 2);
    assert!(matches!(a.state().fibers[&1].wait, Wait::Ticks(3)));
    let mut b = Vm::restore(library, &a.save().unwrap(), 8, 1 << 20).unwrap();
    assert_eq!(a.state_hash().unwrap(), b.state_hash().unwrap());
    for _ in 0..6 {
        a.tick(vec![]).unwrap();
        b.tick(vec![]).unwrap();
        assert_eq!(a.state_hash().unwrap(), b.state_hash().unwrap());
    }
    assert_eq!(a.state().world.globals.numbers[&0], 12);
    assert_eq!(a.state().fibers[&1].wait, Wait::Done);
}
#[test]
fn requests_issue_once_and_stale_cancelled_or_restored_completions_cannot_wake() {
    let programs = vec![program(
        "main",
        vec![
            Op::Request(Service::Message {
                text: "hello".into(),
            }),
            set(0, 9),
            Op::Halt,
        ],
    )];
    let library = Library::new(programs.clone()).unwrap();
    let mut a = vm(programs, 100);
    let id = start(&mut a, "main");
    a.tick(vec![]).unwrap();
    let first = a.requests().next().unwrap().token;
    for _ in 0..3 {
        assert_eq!(a.tick(vec![]).unwrap().instructions, 0);
        assert_eq!(a.requests().count(), 1);
    }
    let mut b = Vm::restore(library, &a.save().unwrap(), 8, 1 << 20).unwrap();
    assert_eq!(
        b.tick(vec![Completion {
            token: first,
            response: Response::Done
        }])
        .unwrap()
        .stale_completions,
        1
    );
    let new = b.requests().next().unwrap().token;
    b.tick(vec![Completion {
        token: new,
        response: Response::Done,
    }])
    .unwrap();
    assert_eq!(b.state().world.globals.numbers[&0], 9);
    a.replace_page(id, Target::Id("main".into())).unwrap();
    a.tick(vec![]).unwrap();
    assert_ne!(a.requests().next().unwrap().token, first);
    assert_eq!(
        a.tick(vec![Completion {
            token: first,
            response: Response::Done
        }])
        .unwrap()
        .stale_completions,
        1
    );
}
#[test]
fn choice_validates_reply_and_jumps_to_compiled_target() {
    let mut a = vm(
        vec![program(
            "main",
            vec![
                Op::Request(Service::Choice {
                    options: vec!["a".into(), "b".into()],
                    targets: vec![1, 3],
                    cancel: None,
                }),
                set(0, 1),
                Op::Halt,
                set(0, 2),
                Op::Halt,
            ],
        )],
        100,
    );
    start(&mut a, "main");
    a.tick(vec![]).unwrap();
    let token = a.requests().next().unwrap().token;
    assert_eq!(
        a.tick(vec![Completion {
            token,
            response: Response::Choice(9)
        }])
        .unwrap()
        .rejected_completions,
        1
    );
    let duplicate = vec![
        Completion {
            token,
            response: Response::Choice(0),
        },
        Completion {
            token,
            response: Response::Choice(1),
        },
    ];
    assert_eq!(a.tick(duplicate).unwrap().rejected_completions, 2);
    a.tick(vec![Completion {
        token,
        response: Response::Choice(1),
    }])
    .unwrap();
    assert_eq!(a.state().world.globals.numbers[&0], 2);
}
#[test]
fn global_budget_round_robin_prevents_starvation_and_reports_hot_loop() {
    let mut a = vm(
        vec![program(
            "main",
            vec![
                compute(Var::Event(0), Value::Int(1), Binary::Add, 0, Update::Add),
                Op::Jump(0),
            ],
        )],
        1,
    );
    a.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    a.spawn(Target::Id("main".into()), 2, vec![]).unwrap();
    let mut hot = false;
    for _ in 0..120 {
        let r = a.tick(vec![]).unwrap();
        assert_eq!(r.instructions, 1);
        assert!(r.budget_exhausted);
        hot |= !r.diagnostics.is_empty();
    }
    assert!(hot);
    assert_eq!(a.state().world.events[&1].numbers[&0], 30);
    assert_eq!(a.state().world.events[&2].numbers[&0], 30);
}
#[test]
fn unsupported_and_arithmetic_faults_preserve_pc_and_other_fibers_progress() {
    let mut a = vm(
        vec![
            program(
                "bad",
                vec![compute(
                    Var::Global(0),
                    Value::Int(1),
                    Binary::Div,
                    0,
                    Update::Set,
                )],
            ),
            program(
                "unknown",
                vec![Op::Unsupported {
                    opcode: 242,
                    reason: "fixture".into(),
                }],
            ),
            program("good", vec![set(1, 7), Op::Halt]),
        ],
        100,
    );
    start(&mut a, "bad");
    start(&mut a, "unknown");
    start(&mut a, "good");
    let report = a.tick(vec![]).unwrap();
    assert_eq!(report.diagnostics.len(), 2);
    assert_eq!(a.state().fibers[&1].frames[0].pc, 0);
    assert_eq!(a.state().fibers[&2].frames[0].pc, 0);
    assert_eq!(a.state().world.globals.numbers[&1], 7);
    assert!(!a.state().world.globals.numbers.contains_key(&0));
    assert_eq!(report.diagnostics[1].source.as_ref().unwrap().command, 0);
}
#[test]
fn random_overflow_indirect_strings_and_conditions_have_explicit_policy() {
    let code = vec![
        set(0, i32::MAX),
        compute(Var::Global(0), Value::Int(1), Binary::Add, 0, Update::Add),
        set(1, 2_000_000),
        compute(
            Var::Global(2),
            Value::Indirect(Box::new(Value::Var(Var::Global(1)))),
            Binary::Add,
            0,
            Update::Set,
        ),
        Op::SetText {
            dst: Var::Local(0),
            value: Text::Literal("hello".into()),
            append: false,
        },
        Op::SetText {
            dst: Var::Local(0),
            value: Text::Literal("!".into()),
            append: true,
        },
        Op::Branch {
            cases: vec![(
                Condition::Text {
                    lhs: Text::Var(Var::Local(0)),
                    cmp: Compare::Eq,
                    rhs: Text::Literal("hello!".into()),
                },
                8,
            )],
            fallback: 7,
        },
        Op::Unsupported {
            opcode: 255,
            reason: "wrong branch".into(),
        },
        compute(
            Var::Global(3),
            Value::Int(-100),
            Binary::RandomInclusive,
            100,
            Update::Set,
        ),
        Op::Halt,
    ];
    let programs = vec![program("main", code)];
    let mut a = vm(programs.clone(), 100);
    let mut b = vm(programs, 100);
    start(&mut a, "main");
    start(&mut b, "main");
    assert!(a.tick(vec![]).unwrap().diagnostics.is_empty());
    b.tick(vec![]).unwrap();
    assert_eq!(a.state_hash().unwrap(), b.state_hash().unwrap());
    assert_eq!(a.state().world.globals.numbers[&2], i32::MIN);
    assert!((-100..=100).contains(&a.state().world.globals.numbers[&3]));
}
#[test]
fn reservations_begin_next_tick_and_stack_overflow_faults_in_place() {
    let mut a = vm(
        vec![
            program(
                "main",
                vec![
                    Op::Reserve {
                        target: Target::Id("child".into()),
                        args: vec![Value::Int(17)],
                    },
                    Op::Halt,
                ],
            ),
            program(
                "child",
                vec![
                    compute(
                        Var::Global(0),
                        Value::Var(Var::Local(0)),
                        Binary::Add,
                        0,
                        Update::Set,
                    ),
                    Op::Halt,
                ],
            ),
        ],
        100,
    );
    start(&mut a, "main");
    a.tick(vec![]).unwrap();
    assert!(a.state().world.globals.numbers.is_empty());
    a.tick(vec![]).unwrap();
    assert_eq!(a.state().world.globals.numbers[&0], 17);
    let lib = Library::new(vec![program(
        "recursive",
        vec![Op::Call {
            target: Target::Id("recursive".into()),
            args: vec![],
        }],
    )])
    .unwrap();
    let mut a = Vm::new(
        lib,
        Config {
            max_stack: 3,
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    start(&mut a, "recursive");
    a.tick(vec![]).unwrap();
    assert_eq!(a.state().fibers[&1].frames.len(), 3);
    assert_eq!(a.state().fibers[&1].frames[2].pc, 0);
}
#[test]
fn restore_rejects_corrupt_program_counter_and_library_identity() {
    let p = vec![program("main", vec![Op::Wait(Value::Int(4)), Op::Halt])];
    let mut a = vm(p.clone(), 100);
    start(&mut a, "main");
    a.tick(vec![]).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&a.save().unwrap()).unwrap();
    assert_eq!(json["version"], 11);
    json["version"] = 10.into();
    assert!(Vm::restore(
        Library::new(p.clone()).unwrap(),
        &serde_json::to_vec(&json).unwrap(),
        8,
        1 << 20
    )
    .is_err());
    json["version"] = 11.into();
    json["fibers"]["1"]["frames"][0]["pc"] = 1000.into();
    assert!(Vm::restore(
        Library::new(p).unwrap(),
        &serde_json::to_vec(&json).unwrap(),
        8,
        1 << 20
    )
    .is_err());
    assert!(Vm::restore(
        Library::new(vec![]).unwrap(),
        &a.save().unwrap(),
        8,
        1 << 20
    )
    .is_err());
}
#[test]
fn clock_preserves_backlog() {
    let mut c = FixedClock::new(60).unwrap();
    assert_eq!(c.advance(1_000_000_000, 4).unwrap(), (4, 56));
    assert_eq!(c.advance(0, 60).unwrap(), (56, 0));
}

#[test]
fn external_spawn_cannot_steal_a_reserved_slot_or_lose_its_arguments() {
    let library = Library::new(vec![
        program(
            "main",
            vec![
                Op::Reserve {
                    target: Target::Id("child".into()),
                    args: vec![Value::Int(37)],
                },
                Op::Halt,
            ],
        ),
        program(
            "child",
            vec![
                compute(
                    Var::Global(0),
                    Value::Var(Var::Local(0)),
                    Binary::Add,
                    0,
                    Update::Set,
                ),
                Op::Halt,
            ],
        ),
    ])
    .unwrap();
    let mut a = Vm::new(
        library.clone(),
        Config {
            max_fibers: 2,
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    start(&mut a, "main");
    a.tick(vec![]).unwrap();
    let before = a.state_hash().unwrap();
    assert!(a.spawn(Target::Id("child".into()), 99, vec![]).is_err());
    assert_eq!(a.state_hash().unwrap(), before);
    let mut b = Vm::restore(library, &a.save().unwrap(), 2, 1 << 20).unwrap();
    a.tick(vec![]).unwrap();
    b.tick(vec![]).unwrap();
    assert_eq!(a.state().world.globals.numbers[&0], 37);
    assert_eq!(a.state().fibers.len(), 2);
    assert_eq!(a.state_hash().unwrap(), b.state_hash().unwrap());
}

#[test]
fn rejected_tick_batch_is_atomic_and_does_not_consume_a_completion() {
    let library = Library::new(vec![program(
        "main",
        vec![
            Op::Request(Service::Message {
                text: "hello".into(),
            }),
            Op::Halt,
        ],
    )])
    .unwrap();
    let mut a = Vm::new(
        library,
        Config {
            max_completions_per_tick: 1,
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    start(&mut a, "main");
    a.tick(vec![]).unwrap();
    let token = a.requests().next().unwrap().token;
    let before = a.state_hash().unwrap();
    let completion = Completion {
        token,
        response: Response::Done,
    };
    assert!(a
        .tick(vec![completion.clone(), completion.clone()])
        .is_err());
    assert_eq!(a.state_hash().unwrap(), before);
    a.tick(vec![completion]).unwrap();
    assert_eq!(a.state().fibers[&1].wait, Wait::Done);
}

#[test]
fn string_and_variable_limits_fault_without_partial_writes() {
    let library = Library::new(vec![
        program(
            "long",
            vec![Op::SetText {
                dst: Var::Event(7),
                value: Text::Literal("12345".into()),
                append: false,
            }],
        ),
        program("slots", vec![set(0, 1), set(0, 2), set(1, 3)]),
    ])
    .unwrap();
    let mut a = Vm::new(
        library.clone(),
        Config {
            max_string_bytes: 4,
            max_slots: 1,
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    start(&mut a, "long");
    start(&mut a, "slots");
    let report = a.tick(vec![]).unwrap();
    assert_eq!(report.diagnostics.len(), 2);
    assert!(a.state().world.events.is_empty());
    assert_eq!(a.state().world.globals.numbers.len(), 1);
    assert_eq!(a.state().world.globals.numbers[&0], 2);
    assert_eq!(a.state().fibers[&1].frames[0].pc, 0);
    assert_eq!(a.state().fibers[&2].frames[0].pc, 2);
    let before = a.state_hash().unwrap();
    assert!(a.set_global(1, 3).is_err());
    assert!(a.set_system(-1, 2).is_err());
    assert_eq!(a.state_hash().unwrap(), before);
    // A cancelled fault has no stale diagnostic and remains a valid saved state.
    a.cancel(1).unwrap();
    assert!(a.state().fibers[&1].fault.is_none());
    Vm::restore(library, &a.save().unwrap(), 2, 1 << 20).unwrap();
}

#[test]
fn movement_asset_and_transition_requests_survive_restore_and_service_failure() {
    let services = [
        Service::Movement {
            actor: 3,
            route: vec![(1, vec![2])],
        },
        Service::Asset {
            path: "MapData/Map001.mps".into(),
        },
        Service::Transition {
            style: 0,
            ticks: 10,
        },
    ];
    let mut code: Vec<_> = services.iter().cloned().map(Op::Request).collect();
    code.push(Op::Halt);
    let programs = vec![program("main", code)];
    let library = Library::new(programs.clone()).unwrap();
    let mut a = vm(programs, 100);
    start(&mut a, "main");
    a.tick(vec![]).unwrap();
    for (i, service) in services.iter().enumerate() {
        let old = a.requests().next().unwrap().clone();
        assert_eq!(&old.service, service);
        let before = a.state_hash().unwrap();
        a = Vm::restore(library.clone(), &a.save().unwrap(), i as u64 + 8, 1 << 20).unwrap();
        assert_eq!(a.state_hash().unwrap(), before);
        let new = a.requests().next().unwrap().token;
        let report = a
            .tick(vec![Completion {
                token: old.token,
                response: Response::Done,
            }])
            .unwrap();
        assert_eq!(report.stale_completions, 1);
        assert_eq!(report.instructions, 0);
        a.tick(vec![Completion {
            token: new,
            response: if i == 2 {
                Response::Failed("unavailable".into())
            } else {
                Response::Done
            },
        }])
        .unwrap();
    }
    assert_eq!(a.state().fibers[&1].wait, Wait::Fault);
    assert!(a.requests().next().is_none());
    assert_eq!(a.state().fibers[&1].frames[0].pc, 3);
    assert_eq!(
        a.state().fibers[&1]
            .fault
            .as_ref()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .command,
        2
    );
}

#[test]
fn restore_rejects_forged_loop_request_and_resource_state() {
    let programs = vec![program(
        "main",
        vec![
            Op::Loop {
                count: Some(Value::Int(2)),
                end: 3,
            },
            Op::Request(Service::Message {
                text: "hello".into(),
            }),
            Op::Next { start: 0 },
            Op::Halt,
        ],
    )];
    let library = Library::new(programs.clone()).unwrap();
    let mut a = vm(programs, 100);
    start(&mut a, "main");
    a.tick(vec![]).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&a.save().unwrap()).unwrap();
    let corruptions: Vec<(&str, serde_json::Value)> = vec![
        (
            "/fibers/1/frames/0/loops/0/remaining",
            serde_json::Value::Null,
        ),
        ("/fibers/1/frames/0/pc", 0.into()),
        ("/requests/1/service/Message/text", "forged".into()),
        ("/requests/1/page_generation", 99.into()),
        ("/next_token", 0.into()),
        ("/config/max_stack", 1025.into()),
        ("/config/max_save_bytes", 1.into()),
        (
            "/world/databases/numeric_fields",
            serde_json::json!({"Mutable": []}),
        ),
    ];
    for (path, value) in corruptions {
        let mut bad = original.clone();
        *bad.pointer_mut(path).unwrap() = value;
        assert!(
            Vm::restore(
                library.clone(),
                &serde_json::to_vec(&bad).unwrap(),
                8,
                1 << 20
            )
            .is_err(),
            "{path}"
        );
    }
    let mut bad = original.clone();
    bad["world"]["globals"]["numbers"]["-1"] = 4.into();
    assert!(Vm::restore(library, &serde_json::to_vec(&bad).unwrap(), 8, 1 << 20).is_err());
}

#[test]
fn save_size_and_exhausted_tick_are_bounded_without_state_changes() {
    let library = Library::new(vec![program("main", vec![Op::Nop])]).unwrap();
    let a = Vm::new(
        library.clone(),
        Config {
            max_save_bytes: 32,
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    assert!(a.save().is_err());
    let a = Vm::new(library.clone(), Config::default(), 1, 1).unwrap();
    let mut save: serde_json::Value = serde_json::from_slice(&a.save().unwrap()).unwrap();
    save["tick"] = u64::MAX.into();
    let mut a = Vm::restore(library, &serde_json::to_vec(&save).unwrap(), 2, 1 << 20).unwrap();
    let before = a.state_hash().unwrap();
    assert!(a.tick(vec![]).is_err());
    assert_eq!(a.state_hash().unwrap(), before);
}
