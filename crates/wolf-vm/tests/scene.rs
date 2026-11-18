use wolf_ir::{Instruction, Library, Op, Program, Service, Source, Target, PROFILE};
use wolf_vm::{
    host::{Action, Host},
    runtime::{Config, Vm, Wait},
    scene::*,
};

fn library() -> Library {
    make_library(Op::Request(Service::Message {
        text: "test".into(),
    }))
}

fn make_library(first: Op) -> Library {
    Library::new(
        ["first", "second"]
            .into_iter()
            .map(|id| Program {
                id: id.into(),
                name: id.into(),
                profile: PROFILE.into(),
                code: vec![first.clone(), Op::Halt]
                    .into_iter()
                    .enumerate()
                    .map(|(pc, op)| Instruction {
                        source: Source {
                            asset: "synthetic".into(),
                            content_blake3: "fixture".into(),
                            event: 0,
                            page: None,
                            command: pc,
                            span: [pc, pc + 1],
                        },
                        raw: None,
                        op,
                    })
                    .collect(),
            })
            .collect(),
    )
    .unwrap()
}

#[test]
fn movement_bridge_completes_once_and_cancels_removed_requests() {
    let lib = make_library(Op::Request(Service::Movement {
        actor: 4,
        route: vec![],
    }));
    let mut vm = Vm::new(lib, Config::default(), 7, 1).unwrap();
    let fiber = vm.spawn(Target::Id("first".into()), 4, vec![]).unwrap();
    let mut grid = Grid::new(4, 1, vec![false; 4]).unwrap();
    grid.insert(4, Position { x: 0, y: 0 }, true).unwrap();
    let mut movement = Movement::new(grid);
    let mut host = Host::new(8, 8).unwrap();
    vm.tick(vec![]).unwrap();
    let actions = movement
        .update(&vm, |_| {
            Ok((vec![Step::Move(Direction::Right)], BlockedPolicy::Fail))
        })
        .unwrap();
    assert!(matches!(actions.as_slice(), [Action::MovementFinished(_)]));
    assert!(movement
        .update(&vm, |_| panic!("duplicate decode"))
        .unwrap()
        .is_empty());
    let poll = host.poll(&vm, &actions, |_| unreachable!()).unwrap();
    vm.tick(poll.completions).unwrap();
    assert_eq!(vm.state().fibers[&fiber].wait, Wait::Done);
    vm.replace_page(fiber, Target::Id("second".into())).unwrap();
    vm.tick(vec![]).unwrap();
    assert!(movement
        .update(&vm, |_| Ok((
            vec![Step::Wait(2), Step::Move(Direction::Right)],
            BlockedPolicy::Fail
        )))
        .unwrap()
        .is_empty());
    vm.cancel(fiber).unwrap();
    vm.tick(vec![]).unwrap();
    movement.update(&vm, |_| unreachable!()).unwrap();
    vm.tick(vec![]).unwrap();
    movement.update(&vm, |_| unreachable!()).unwrap();
    assert_eq!(movement.position(4), Some(Position { x: 1, y: 0 }));
}

#[test]
fn collision_is_stable_with_waits_retry_skip_and_bounds() {
    let mut grid = Grid::new(3, 2, vec![false, false, false, true, false, false]).unwrap();
    grid.insert(2, Position { x: 2, y: 0 }, true).unwrap();
    grid.insert(1, Position { x: 0, y: 0 }, true).unwrap();
    grid.route(1, vec![Step::Move(Direction::Right)], BlockedPolicy::Fail)
        .unwrap();
    grid.route(2, vec![Step::Move(Direction::Left)], BlockedPolicy::Retry)
        .unwrap();
    assert_eq!(grid.tick().unwrap(), vec![RouteResult::Finished(1)]);
    assert_eq!(grid.position(1), Some(Position { x: 1, y: 0 }));
    assert_eq!(grid.position(2), Some(Position { x: 2, y: 0 }));
    grid.route(
        1,
        vec![Step::Wait(2), Step::Move(Direction::Down)],
        BlockedPolicy::Fail,
    )
    .unwrap();
    assert!(grid.tick().unwrap().is_empty());
    assert!(grid.tick().unwrap().is_empty());
    assert_eq!(
        grid.tick().unwrap(),
        vec![RouteResult::Finished(1), RouteResult::Finished(2)]
    );
    grid.route(1, vec![Step::Move(Direction::Left)], BlockedPolicy::Fail)
        .unwrap();
    assert_eq!(grid.tick().unwrap(), vec![RouteResult::Blocked(1)]);
    grid.route(1, vec![Step::Move(Direction::Down)], BlockedPolicy::Skip)
        .unwrap();
    assert_eq!(grid.tick().unwrap(), vec![RouteResult::Finished(1)]);
    assert_eq!(grid.position(1), Some(Position { x: 1, y: 1 }));
}

#[test]
fn page_switch_cancels_old_ui_and_action_page_waits_for_trigger() {
    let mut vm = Vm::new(library(), Config::default(), 7, 1).unwrap();
    let mut pages = Pages::new();
    pages
        .insert(
            10,
            vec![
                PageRule {
                    target: Target::Id("first".into()),
                    equals: vec![],
                    trigger: Trigger::Once,
                },
                PageRule {
                    target: Target::Id("second".into()),
                    equals: vec![(0, 1)],
                    trigger: Trigger::Action,
                },
            ],
        )
        .unwrap();
    assert!(pages.update(&mut vm, &[]).is_empty());
    vm.tick(vec![]).unwrap();
    let old = vm.requests().next().unwrap().token;
    vm.set_global(0, 1).unwrap();
    assert!(pages.update(&mut vm, &[]).is_empty());
    assert_eq!(vm.requests().count(), 0);
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
    pages.update(&mut vm, &[10]);
    vm.tick(vec![]).unwrap();
    let new = vm.requests().next().unwrap().token;
    assert_ne!(new, old);
    let mut host = Host::new(8, 8).unwrap();
    let reply = host
        .poll(
            &vm,
            &[Action::Acknowledge(old), Action::Acknowledge(new)],
            |_| unreachable!(),
        )
        .unwrap();
    assert_eq!(reply.rejected_actions, 1);
    vm.tick(reply.completions).unwrap();
    pages.update(&mut vm, &[]);
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
}
