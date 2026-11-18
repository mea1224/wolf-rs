//! A053D0 captures the map; A058AB/A0560F select it only for map frames.
use wolf_format::{RawCommand, Span};
use wolf_ir::{compile, Library, Source, Target};
use wolf_vm::runtime::{Config, Vm, Wait};

fn command(opcode: u32, args: &[i32]) -> RawCommand {
    RawCommand { opcode, args: args.to_vec(), indent: 0, span: Span { start: 0, end: 1 },
        strings: vec![], terminator: 0, movement: None, extension_length: 0, extension: vec![] }
}

#[test]
fn transfer_preserves_map_frame_bank_while_nested_common_uses_current_map() {
    let source = |page| Source { asset: "map-context-fixture".into(), content_blake3: "fixture".into(),
        event: 7, page, command: 0, span: [0, 1] };
    let map = compile("map/old".into(), "old map event".into(), source(Some(0)), &[
        command(180, &[2]),
        command(210, &[500008, 0]),
        command(121, &[1100000, 31, 0, 0]),
        command(121, &[1000071, 32, 0, 0]),
        command(121, &[2000000, 1100000, 0, 0]),
    ]).unwrap();
    let mut common_source = source(None);
    common_source.event = 8;
    let common = compile("common/8".into(), "nested".into(), common_source, &[
        command(121, &[1100000, 51, 0, 0]),
        command(121, &[1000071, 52, 0, 0]),
        command(180, &[2]),
    ]).unwrap();
    let library = Library::new(vec![map, common]).unwrap();
    let mut vm = Vm::new(library.clone(), Config::default(), 1, 1).unwrap();
    vm.world_mut().switch_map(1).unwrap();
    vm.spawn(Target::Id("map/old".into()), 7, vec![]).unwrap();
    assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
    vm.world_mut().switch_map(2).unwrap();
    let mut restored = Vm::restore(library, &vm.save().unwrap(), 2, 1 << 20).unwrap();
    for _ in 0..10 {
        assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
        assert!(restored.tick(vec![]).unwrap().diagnostics.is_empty());
        assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    }
    assert_eq!(restored.state().fibers[&1].wait, Wait::Done);
    assert_eq!(restored.state().world.globals.numbers[&0], 31);
    assert_eq!(restored.state().world.events[&7].numbers[&0], 51);
    assert_eq!(restored.state().world.events[&7].numbers[&1], 52);
    assert_eq!(restored.state().world.map_events[&1][&7].numbers[&0], 31);
    assert_eq!(restored.state().world.map_events[&1][&7].numbers[&1], 32);
    restored.world_mut().switch_map(1).unwrap();
    assert_eq!(restored.state().world.events[&7].numbers[&0], 31);
    assert_eq!(restored.state().world.map_events[&2][&7].numbers[&0], 51);
}
