use wolf_ir::{Instruction, Library, Op, Program, Service, Source, Target, PROFILE};
use wolf_vm::{
    host::{Action, Host},
    runtime::{Config, Vm, Wait},
};

fn setup(services: Vec<Service>) -> (Library, Vm) {
    let library = Library::new(vec![Program {
        id: "main".into(),
        name: "main".into(),
        profile: PROFILE.into(),
        code: services
            .into_iter()
            .map(Op::Request)
            .chain([Op::Halt])
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
    }])
    .unwrap();
    let mut vm = Vm::new(library.clone(), Config::default(), 7, 42).unwrap();
    vm.spawn(Target::Id("main".into()), 0, vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    (library, vm)
}
#[test]
fn dialogue_asset_and_transition_execute_without_synthetic_vm_completions() {
    let (_, mut vm) = setup(vec![
        Service::Message {
            text: "hello".into(),
        },
        Service::Asset {
            path: "MapData/map.mps".into(),
        },
        Service::Transition { style: 0, ticks: 3 },
    ]);
    let mut host = Host::new(10, 10).unwrap();
    let mut loads = 0;
    let mut loader = |path: &str| {
        assert_eq!(path, "MapData/map.mps");
        loads += 1;
        Ok(())
    };
    let first = host.poll(&vm, &[], &mut loader).unwrap();
    assert!(first.completions.is_empty());
    let token = host.requests().next().unwrap().token;
    let poll = host
        .poll(
            &vm,
            &[Action::Acknowledge(token), Action::Acknowledge(token)],
            &mut loader,
        )
        .unwrap();
    assert_eq!(poll.rejected_actions, 1);
    vm.tick(poll.completions).unwrap();
    let poll = host.poll(&vm, &[], &mut loader).unwrap();
    // Repeated polling must neither duplicate loading nor lose the unconsumed reply.
    assert_eq!(
        host.poll(&vm, &[], &mut loader).unwrap().completions.len(),
        1
    );
    vm.tick(poll.completions).unwrap();
    for _ in 0..2 {
        let poll = host.poll(&vm, &[], &mut loader).unwrap();
        assert!(poll.completions.is_empty());
        vm.tick(poll.completions).unwrap();
    }
    let poll = host.poll(&vm, &[], &mut loader).unwrap();
    assert_eq!(poll.completions.len(), 1);
    vm.tick(poll.completions).unwrap();
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
    assert_eq!(loads, 1);
}

#[test]
fn choices_validate_cancel_index_and_restored_session() {
    let (library, vm) = setup(vec![Service::Choice {
        options: vec!["yes".into()],
        targets: vec![1],
        cancel: None,
    }]);
    let old = vm.requests().next().unwrap().token;
    let mut restored = Vm::restore(library, &vm.save().unwrap(), 8, 1 << 20).unwrap();
    let current = restored.requests().next().unwrap().token;
    let mut host = Host::new(4, 4).unwrap();
    let poll = host
        .poll(
            &restored,
            &[
                Action::Select(old, 0),
                Action::Select(current, 1),
                Action::Cancel(current),
            ],
            |_| panic!("not an asset"),
        )
        .unwrap();
    assert_eq!(poll.rejected_actions, 3);
    assert!(poll.completions.is_empty());
    let poll = host
        .poll(&restored, &[Action::Select(current, 0)], |_| unreachable!())
        .unwrap();
    restored.tick(poll.completions).unwrap();
    assert_eq!(restored.state().fibers[&1].wait, Wait::Done);
}

#[test]
fn cancellation_limits_and_loading_failure_are_explicit() {
    let (_, mut vm) = setup(vec![Service::Movement {
        actor: 1,
        route: vec![],
    }]);
    let token = vm.requests().next().unwrap().token;
    let mut host = Host::new(1, 1).unwrap();
    assert!(host
        .poll(
            &vm,
            &[Action::Cancel(token), Action::Cancel(token)],
            |_| Ok(())
        )
        .is_err());
    assert_eq!(host.requests().count(), 0);
    assert!(host
        .poll(&vm, &[], |_| Ok(()))
        .unwrap()
        .completions
        .is_empty());
    vm.cancel(1).unwrap();
    assert_eq!(
        host.poll(&vm, &[Action::MovementFinished(token)], |_| Ok(()))
            .unwrap()
            .rejected_actions,
        1
    );
    assert_eq!(host.requests().count(), 0);
    let (_, mut vm) = setup(vec![Service::Asset {
        path: "missing".into(),
    }]);
    let poll = host
        .poll(&vm, &[], |_| Err("asset missing".into()))
        .unwrap();
    vm.tick(poll.completions).unwrap();
    assert_eq!(vm.state().fibers[&1].wait, Wait::Fault);
}

#[test]
fn paired_restore_preserves_remaining_transition_and_rejects_wrong_state() {
    let (library, mut vm) = setup(vec![Service::Transition { style: 0, ticks: 5 }]);
    let mut host = Host::new(8, 8).unwrap();
    for _ in 0..2 {
        let poll = host.poll(&vm, &[], |_| unreachable!()).unwrap();
        vm.tick(poll.completions).unwrap();
    }
    host.poll(&vm, &[], |_| unreachable!()).unwrap();
    let bytes = host.save(&vm, 1 << 20).unwrap();
    assert!(host.save(&vm, 10).is_err());
    let mut restored = Vm::restore(library, &vm.save().unwrap(), 8, 1 << 20).unwrap();
    let mut restored_host = Host::restore(&bytes, &restored, 1 << 20).unwrap();
    assert!(Host::restore(&bytes, &restored, bytes.len() - 1).is_err());
    for _ in 0..3 {
        let a = host.poll(&vm, &[], |_| unreachable!()).unwrap();
        let b = restored_host
            .poll(&restored, &[], |_| unreachable!())
            .unwrap();
        assert_eq!(a.completions.len(), b.completions.len());
        vm.tick(a.completions).unwrap();
        restored.tick(b.completions).unwrap();
        assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    }
    assert_eq!(restored.state().fibers[&1].wait, Wait::Done);
    assert!(Host::restore(&bytes, &restored, 1 << 20).is_err());
}

#[test]
fn queued_asset_reply_survives_restore_without_reloading() {
    let (library, vm) = setup(vec![Service::Asset {
        path: "fixture".into(),
    }]);
    let mut host = Host::new(8, 8).unwrap();
    host.poll(&vm, &[], |_| Ok(())).unwrap();
    let bytes = host.save(&vm, 1 << 20).unwrap();
    let mut restored = Vm::restore(library, &vm.save().unwrap(), 8, 1 << 20).unwrap();
    let mut restored_host = Host::restore(&bytes, &restored, 1 << 20).unwrap();
    let poll = restored_host
        .poll(&restored, &[], |_| panic!("already completed"))
        .unwrap();
    restored.tick(poll.completions).unwrap();
    assert_eq!(restored.state().fibers[&1].wait, Wait::Done);
}
