//! A1CE70 writes a single common slot; A05DB0 consumes it on the idle main.
use wolf_format::{RawCommand, Span};
use wolf_ir::{compile, Library, Source, Target};
use wolf_vm::runtime::{Config, Vm, Wait};

fn program(id: u32, commands: &[(u32, &[i32])]) -> wolf_ir::Program {
    let raw: Vec<_> = commands.iter().map(|(opcode,args)| RawCommand {
        opcode: *opcode, args: args.to_vec(), indent: 0, span: Span { start: 0, end: 1 },
        strings: vec![], terminator: 0, movement: None, extension_length: 0, extension: vec![],
    }).collect();
    compile(format!("common/{id}"), format!("fixture{id}"), Source {
        asset: "reservation-fixture".into(), content_blake3: "fixture".into(), event: id,
        page: None, command: 0, span: [0,1],
    }, &raw).unwrap()
}

#[test]
fn common_reservation_coalesces_waits_for_main_and_restores() {
    let library = Library::new(vec![
        program(0, &[(211,&[500001,0]),(211,&[500002,0]),(211,&[500002,0])]),
        program(1, &[(121,&[2000001,1,0,0])]),
        program(2, &[(121,&[2000002,9000013,0,0]),(180,&[2])]),
        program(3, &[(180,&[3])]),
    ]).unwrap();
    let mut vm = Vm::new(library.clone(), Config::default(), 1, 1).unwrap();
    vm.spawn(Target::Id("common/0".into()), 90, vec![]).unwrap();
    let main = vm.spawn(Target::Id("common/3".into()), 7, vec![]).unwrap();
    vm.set_native_main_fiber(Some(main)).unwrap();
    vm.tick(vec![]).unwrap();
    assert!(!vm.start_native_reserved_main().unwrap());
    let mut restored = Vm::restore(library, &vm.save().unwrap(), 2, 1<<20).unwrap();
    for _ in 0..10 {
        assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
        assert!(restored.tick(vec![]).unwrap().diagnostics.is_empty());
        assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    }
    assert!(!vm.state().world.globals.numbers.contains_key(&1));
    assert_eq!(vm.state().world.globals.numbers[&2], 1);
    assert_eq!(vm.state().fibers.len(), 3);
    assert!(vm.state().fibers.values().all(|f| f.wait == Wait::Done));
    assert_eq!(vm.state().fibers[&3].owner, u64::MAX);
}

#[test]
fn dynamic_and_relative_reservations_use_common_self_and_clamp() {
    let library = Library::new(vec![
        program(0, &[(121,&[2000000,71,0,0])]),
        program(1, &[(121,&[1600011,600099,0,0]),(211,&[1600011,0])]),
        program(2, &[(211,&[699999,0]),(180,&[2])]),
        program(3, &[(121,&[2000003,73,0,0])]),
    ]).unwrap();
    let mut vm = Vm::new(library, Config::default(), 1, 1).unwrap();
    vm.spawn(Target::Id("common/1".into()), 1, vec![]).unwrap();
    for _ in 0..4 { assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty()); }
    assert_eq!(vm.state().world.globals.numbers[&0],71);
    vm.spawn(Target::Id("common/2".into()), 1, vec![]).unwrap();
    for _ in 0..4 { assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty()); }
    assert_eq!(vm.state().world.globals.numbers[&3],73);
    // The main common slot is reused instead of creating an unbounded fiber stream.
    assert_eq!(vm.state().fibers.len(),3);
}

#[test]
fn map_reservations_are_flags_bound_to_the_current_map_and_saved() {
    let library = Library::new(vec![program(0, &[(211,&[7,0]),(211,&[7,0]),(211,&[9,0])])]).unwrap();
    let mut vm = Vm::new(library.clone(),Config::default(),1,1).unwrap();
    vm.world_mut().switch_map(1).unwrap();
    vm.spawn(Target::Id("common/0".into()),u64::MAX,vec![]).unwrap();
    assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
    let mut restored = Vm::restore(library,&vm.save().unwrap(),2,1<<20).unwrap();
    assert_eq!(restored.take_native_map_reservations(),vec![7,9]);
    vm.world_mut().switch_map(2).unwrap();
    assert!(vm.take_native_map_reservations().is_empty());
}
