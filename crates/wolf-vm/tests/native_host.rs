use wolf_format::{RawCommand, Span};
use wolf_ir::{Library, Source, Target};
use wolf_vm::runtime::{Vm, Wait};

fn vm() -> Vm {
    let raw = RawCommand {
        opcode: 270,
        args: vec![0, 1],
        indent: 0,
        span: Span { start: 0, end: 1 },
        strings: vec![],
        terminator: 0,
        movement: None,
        extension_length: 0,
        extension: vec![],
    };
    let source = Source {
        asset: "host-test".into(),
        content_blake3: "synthetic".into(),
        event: 1,
        page: Some(0),
        command: 0,
        span: [0, 1],
    };
    let program = wolf_ir::compile("test".into(), String::new(), source, &[raw]).unwrap();
    let mut vm = Vm::new(
        Library::new(vec![program]).unwrap(),
        Default::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(Target::Id("test".into()), 1, vec![]).unwrap();
    vm
}

#[test]
fn busy_host_retries_on_next_tick_and_applies_once() {
    let mut vm = vm();
    let mut calls = 0;
    let mut host = |ctx: &mut wolf_vm::runtime::NativeContext<'_>| {
        calls += 1;
        if calls == 1 {
            return Ok(false);
        }
        ctx.set_number(2_000_003, 42)?;
        Ok(true)
    };
    vm.tick_with_host(vec![], &mut host).unwrap();
    assert_eq!(vm.state().fibers[&1].frames[0].pc, 0);
    assert_eq!(vm.state().fibers[&1].wait, Wait::Ticks(2));
    assert!(!vm.state().world.globals.numbers.contains_key(&3));
    vm.tick_with_host(vec![], &mut host).unwrap();
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
    assert_eq!(vm.state().world.globals.numbers[&3], 42);
    vm.tick_with_host(vec![], &mut host).unwrap();
    assert_eq!(calls, 2);
}

#[test]
fn missing_host_fault_preserves_source_and_pc() {
    let mut vm = vm();
    vm.tick(vec![]).unwrap();
    let fiber = &vm.state().fibers[&1];
    assert_eq!(fiber.wait, Wait::Fault);
    assert_eq!(fiber.frames[0].pc, 0);
    assert_eq!(
        fiber.fault.as_ref().unwrap().source.as_ref().unwrap().asset,
        "host-test"
    );
}
