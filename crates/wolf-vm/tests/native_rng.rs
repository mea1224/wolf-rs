//! Vectors reconstructed from 942820 and A0F554; see validation/native-rng/manifest.json.
use wolf_format::{RawCommand, Span};
use wolf_ir::{compile, Library, Source, Target};
use wolf_vm::runtime::{Config, Vm, Wait};

fn command(opcode: u32, args: &[i32]) -> RawCommand {
    RawCommand {
        span: Span { start: 0, end: 1 },
        opcode,
        args: args.to_vec(),
        indent: 0,
        strings: vec![],
        terminator: 0,
        movement: None,
        extension_length: 0,
        extension: vec![],
    }
}
fn setup(raw: &[RawCommand], seed: u32, config: Config) -> (Vm, Library) {
    let source = Source {
        asset: "native-rng-fixture".into(),
        content_blake3: "synthetic".into(),
        event: 17,
        page: None,
        command: 40,
        span: [0, 0],
    };
    let program = compile("main".into(), "".into(), source, raw).unwrap();
    let library = Library::new(vec![program]).unwrap();
    let mut vm = Vm::new(library.clone(), config, 1, seed).unwrap();
    vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    (vm, library)
}

#[test]
fn native_random_vectors_cover_rejection_reverse_degenerate_negative_and_wrapping_ranges() {
    for (seed, a, b, expected, state) in [
        (1, 1, 3, 2, 16_807),
        (1, 600, 400, 524, 16_807),
        (1, -200, -500, -248, 16_807),
        (1, 0, 0, 0, 16_807),
        (120_000, -30, 30, 0, 1_147_995_752),
        (u32::MAX, -100, 100, 24, 16_807),
        (1, i32::MIN, i32::MAX, -2_147_483_647, 16_807),
        (1, 0, i32::MIN, -2_147_466_841, 16_807),
    ] {
        // 0xC forces both operands literal, including integer extremes.
        let (mut vm, _) = setup(
            &[command(121, &[2_000_000, a, b, 0x600C])],
            seed,
            Config::default(),
        );
        assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
        assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
        assert_eq!(vm.state().world.globals.numbers[&0], expected);
        assert_eq!(vm.state().native_rng, state);
        assert_eq!(vm.state().rng, seed.max(1));
    }
}

#[test]
fn ranged_random_draws_once_per_destination_and_rereads_aliasing_operand() {
    let (mut vm, _) = setup(
        &[command(121, &[2_000_000, 2_000_000, 20, 0x16000, 2])],
        1,
        Config::default(),
    );
    vm.set_global(0, 10).unwrap();
    assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
    // First draw: 10+16807%11=20. Subsequent reads now see range20..20,
    // but the RNG still advances for each of the remaining destinations.
    assert_eq!(vm.state().world.globals.numbers[&0], 20);
    assert_eq!(vm.state().world.globals.numbers[&1], 20);
    assert_eq!(vm.state().world.globals.numbers[&2], 20);
    assert_eq!(vm.state().native_rng, 1_622_650_073);
}

#[test]
fn indirect_random_applies_update_and_clamp_without_extra_draws() {
    let (mut vm, _) = setup(
        &[command(121, &[2_000_000, 1, 3, 0x6111])],
        1,
        Config::default(),
    );
    vm.set_global(0, 2_000_001).unwrap();
    vm.set_global(1, 999_998).unwrap();
    assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
    assert_eq!(vm.state().world.globals.numbers[&1], 999_999);
    assert_eq!(vm.state().native_rng, 16_807);
}

#[test]
fn replay_restores_native_sequence_including_rejection_and_requires_new_save_field() {
    let mut raw = Vec::new();
    for id in 0..12 {
        raw.push(command(121, &[2_000_000 + id, 0, 999_999, 0x6000]));
        raw.push(command(180, &[1]));
    }
    let (mut vm, library) = setup(&raw, 1, Config::default());
    for _ in 0..5 {
        assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
    }
    let save = vm.save().unwrap();
    let mut restored = Vm::restore(library.clone(), &save, 2, 1 << 20).unwrap();
    for _ in 0..10 {
        assert!(vm.tick(vec![]).unwrap().diagnostics.is_empty());
        assert!(restored.tick(vec![]).unwrap().diagnostics.is_empty());
        assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    }
    assert_eq!(vm.state().native_rng, 1_784_484_492);
    assert_eq!(vm.state().world.globals.numbers[&9], 564_440);
    let mut value: serde_json::Value = serde_json::from_slice(&save).unwrap();
    value.as_object_mut().unwrap().remove("native_rng");
    assert!(Vm::restore(library, &serde_json::to_vec(&value).unwrap(), 2, 1 << 20).is_err());
}

#[test]
fn unbound_clock_reseed_is_explicit_and_does_not_replace_seed() {
    for seed in [0, 2_147_483_647, 4_294_967_294] {
        let (mut vm, _) = setup(
            &[command(121, &[2_000_000, 1, 3, 0x6000])],
            seed,
            Config::default(),
        );
        let report = vm.tick(vec![]).unwrap();
        assert!(report.diagnostics[0]
            .message
            .contains("GetTickCount reseed"));
        assert_eq!(report.diagnostics[0].source.as_ref().unwrap().command, 40);
        assert_eq!(vm.state().native_rng, seed);
        assert_eq!(vm.state().fibers[&1].frames[0].pc, 0);
        assert!(!vm.state().world.globals.numbers.contains_key(&0));
    }
}

#[test]
fn failed_destination_does_not_commit_random_draw() {
    let config = Config {
        max_slots: 1,
        ..Config::default()
    };
    let (mut vm, _) = setup(&[command(121, &[2_000_001, 1, 3, 0x6000])], 1, config);
    vm.set_global(0, 10).unwrap();
    assert!(!vm.tick(vec![]).unwrap().diagnostics.is_empty());
    assert_eq!(vm.state().native_rng, 1);
    assert!(!vm.state().world.globals.numbers.contains_key(&1));
}
