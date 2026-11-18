use wolf_format::{RawCommand, Span};
use wolf_ir::{compile, Library, Source, Target};
use wolf_vm::native_commands::{parse_lifecycle_request, LifecycleRequest, MapTransferRequest};
use wolf_vm::runtime::{Config, NativeContext, Vm, Wait};

fn command(opcode: u32, args: &[i32]) -> RawCommand {
    RawCommand {
        opcode,
        args: args.to_vec(),
        indent: 0,
        span: Span { start: 0, end: 1 },
        strings: vec![],
        terminator: 0,
        movement: None,
        extension_length: 0,
        extension: vec![],
    }
}

fn make_vm(raw: Vec<RawCommand>) -> Vm {
    let source = Source {
        asset: "lifecycle-fixture".into(),
        content_blake3: "synthetic".into(),
        event: 23,
        page: None,
        command: 0,
        span: [0, 1],
    };
    let main = compile("main".into(), String::new(), source.clone(), &raw).unwrap();
    let side = compile(
        "side".into(),
        String::new(),
        source,
        &[command(121, &[2_000_002, 42, 0, 0])],
    )
    .unwrap();
    let mut vm = Vm::new(
        Library::new(vec![main, side]).unwrap(),
        Config {
            run_until_yield: true,
            native_main_fiber: Some(1),
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    vm
}

#[test]
fn title_map_request_resolves_operands_then_yields_only_its_fiber() {
    let mut vm = make_vm(vec![
        command(130, &[-1, 2_000_003, 7, 1, 0, 0]),
        command(121, &[2_000_001, 99, 0, 0]),
    ]);
    vm.set_global(3, 13).unwrap();
    vm.spawn(Target::Id("side".into()), 2, vec![]).unwrap();
    let mut requests = vec![];
    let report = vm
        .tick_with_host(vec![], &mut |ctx| {
            requests.push(parse_lifecycle_request(ctx)?.unwrap());
            ctx.yield_once();
            Ok(true)
        })
        .unwrap();
    assert!(report.diagnostics.is_empty());
    assert!(!report.execution_invalidated);
    assert_eq!(
        requests,
        vec![LifecycleRequest::MapTransfer(MapTransferRequest {
            map_id: Some(1),
            x_half: 26,
            y_half: 14,
            direction: None,
            flags: 0,
        })]
    );
    assert_eq!(vm.state().fibers[&1].wait, Wait::Ticks(2));
    assert_eq!(vm.state().fibers[&1].frames[0].pc, 1);
    assert!(!vm.state().world.globals.numbers.contains_key(&1));
    assert_eq!(vm.state().world.globals.numbers[&2], 42);
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&1], 99);
}

#[test]
fn replacement_main_runs_before_older_common_and_world_requires_clear_execution() {
    let mut vm = make_vm(vec![command(175, &[])]);
    let world = vm.state().world.clone();
    assert!(vm.replace_world_for_restore(world.clone(), 123).is_err());
    vm.clear_execution();
    vm.replace_world_for_restore(world, 123).unwrap();
    assert_eq!(vm.state().native_rng, 123);
    vm.spawn(Target::Id("side".into()), 2, vec![]).unwrap();
    let main = vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    vm.set_native_main_fiber(Some(main)).unwrap();
    let report = vm
        .tick_with_host(vec![], &mut |ctx| {
            ctx.invalidate_execution();
            Ok(true)
        })
        .unwrap();
    assert!(report.execution_invalidated);
    assert!(!vm.state().world.globals.numbers.contains_key(&2));
}

#[test]
fn saved_world_patch_preserves_execution_and_rejects_invalid_world_atomically() {
    let mut vm = make_vm(vec![command(180, &[5])]);
    vm.tick(vec![]).unwrap();
    let fiber = serde_json::to_value(&vm.state().fibers).unwrap();
    let mut world = vm.state().world.clone();
    world.globals.numbers.insert(42, 7);
    vm.replace_saved_world(world.clone()).unwrap();
    assert_eq!(serde_json::to_value(&vm.state().fibers).unwrap(), fiber);
    world.globals.numbers.insert(-1, 9);
    assert!(vm.replace_saved_world(world).is_err());
    assert_eq!(vm.state().world.globals.numbers.get(&42), Some(&7));
    assert!(!vm.state().world.globals.numbers.contains_key(&-1));
}

#[test]
fn same_map_flags_directions_and_old_coordinate_form_remain_distinct() {
    for (args, expected) in [
        (
            vec![-1, 13, 7, -1, 33, 8],
            MapTransferRequest {
                map_id: None,
                x_half: 13,
                y_half: 7,
                direction: Some(8),
                flags: 33,
            },
        ),
        (
            vec![-1, 13, 7, -1, 0, 5],
            MapTransferRequest {
                map_id: None,
                x_half: 26,
                y_half: 14,
                direction: None,
                flags: 0,
            },
        ),
        (
            vec![-1, 13, 7, 4],
            MapTransferRequest {
                map_id: Some(4),
                x_half: 26,
                y_half: 14,
                direction: None,
                flags: 0,
            },
        ),
    ] {
        let mut vm = make_vm(vec![command(130, &args)]);
        let mut observed = None;
        let report = vm
            .tick_with_host(vec![], &mut |ctx| {
                observed = parse_lifecycle_request(ctx)?;
                Ok(true)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty());
        assert_eq!(observed, Some(LifecycleRequest::MapTransfer(expected)));
    }
}

#[test]
fn busy_map_transaction_retries_same_pc_without_publishing_a_second_request() {
    let mut vm = make_vm(vec![command(130, &[-1, 13, 7, 1, 0, 0])]);
    let mut busy = true;
    let mut accepted = 0;
    let mut host = |ctx: &mut NativeContext<'_>| {
        if busy {
            busy = false;
            return Ok(false);
        }
        assert!(matches!(
            parse_lifecycle_request(ctx)?,
            Some(LifecycleRequest::MapTransfer(_))
        ));
        accepted += 1;
        ctx.yield_once();
        Ok(true)
    };
    vm.tick_with_host(vec![], &mut host).unwrap();
    assert_eq!(vm.state().fibers[&1].frames[0].pc, 0);
    vm.tick_with_host(vec![], &mut host).unwrap();
    assert_eq!(vm.state().fibers[&1].frames[0].pc, 1);
    vm.tick_with_host(vec![], &mut host).unwrap();
    assert_eq!(accepted, 1);
}

#[test]
fn exit_or_restore_stops_old_tree_and_requires_commit_before_tick_or_save() {
    for lifecycle in [command(175, &[]), command(220, &[1, 1])] {
        let mut vm = make_vm(vec![lifecycle, command(121, &[2_000_001, 99, 0, 0])]);
        vm.spawn(Target::Id("side".into()), 2, vec![]).unwrap();
        let report = vm
            .tick_with_host(vec![], &mut |ctx| {
                if ctx.instruction.raw.as_ref().unwrap().opcode == 175 {
                    assert_eq!(parse_lifecycle_request(ctx)?, Some(LifecycleRequest::Exit));
                }
                ctx.invalidate_execution();
                Ok(true)
            })
            .unwrap();
        assert!(report.execution_invalidated);
        assert_eq!(report.instructions, 1);
        assert!(!vm.state().world.globals.numbers.contains_key(&1));
        assert!(!vm.state().world.globals.numbers.contains_key(&2));
        assert!(vm.save().is_err());
        assert!(vm.tick(vec![]).is_err());
        assert_eq!(vm.state().tick, 1);
        vm.clear_execution();
        let id = vm.spawn(Target::Id("side".into()), 2, vec![]).unwrap();
        assert!(id > 2);
        vm.set_native_main_fiber(Some(id)).unwrap();
        assert_eq!(vm.config().native_main_fiber, Some(id));
        vm.tick(vec![]).unwrap();
        assert_eq!(vm.state().world.globals.numbers[&2], 42);
        assert!(vm.save().is_ok());
    }
}

#[test]
fn unknown_actor_transfer_is_a_contextual_diagnostic_not_a_map_request() {
    let mut vm = make_vm(vec![command(130, &[-2, 13, 7, 1, 0, 0])]);
    let report = vm
        .tick_with_host(vec![], &mut |ctx| {
            parse_lifecycle_request(ctx)?;
            Ok(true)
        })
        .unwrap();
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(vm.state().fibers[&1].frames[0].pc, 0);
    assert_eq!(report.diagnostics[0].source.as_ref().unwrap().event, 23);
    assert!(report.diagnostics[0]
        .message
        .contains("actor or database adapter"));
}

#[test]
fn scheduler_absolute_page_self_reads_event_id_and_slot_boundaries() {
    let mut vm = make_vm(vec![]);
    let mut world = vm.state().world.clone();
    world.events.entry(0).or_default().numbers.insert(0, 11);
    world.events.entry(17).or_default().numbers.insert(9, 22);
    world.events.entry(9999).or_default().numbers.insert(9, 33);
    vm.replace_saved_world(world).unwrap();
    assert_eq!(vm.native_number(1_000_000).unwrap(), 11);
    assert_eq!(vm.native_number(1_000_179).unwrap(), 22);
    assert_eq!(vm.native_number(1_099_999).unwrap(), 33);
    assert_eq!(vm.native_number(1_000_169).unwrap(), 0);
    // The contextual range retains its existing owner-zero interpretation.
    assert_eq!(vm.native_number(1_100_000).unwrap(), 11);
}
