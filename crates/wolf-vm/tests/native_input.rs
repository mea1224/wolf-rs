//! Expectations from A14B50/A077D0/957140 and A1B9B0/A06570, not a simulated title screen.
use wolf_format::{RawCommand, Span};
use wolf_ir::{compile, Library, Source, Target};
use wolf_vm::{
    input::KeyboardSample,
    replay::{Input, Recorder},
    runtime::{Vm, Wait},
};

fn command(opcode: u32, args: &[i32]) -> RawCommand {
    RawCommand {
        span: Span { start: 10, end: 20 },
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
fn library(raw: &[RawCommand]) -> Library {
    Library::new(vec![compile(
        "main".into(),
        "".into(),
        Source {
            asset: "native-input-fixture".into(),
            content_blake3: "synthetic".into(),
            event: 26,
            page: None,
            command: 201,
            span: [0, 0],
        },
        raw,
    )
    .unwrap()])
    .unwrap()
}
fn vm(raw: &[RawCommand]) -> Vm {
    let mut vm = Vm::new(library(raw), Default::default(), 1, 1).unwrap();
    vm.spawn(Target::Id("main".into()), 9, vec![]).unwrap();
    vm
}
fn sample(indices: &[u8]) -> KeyboardSample {
    let mut sample = KeyboardSample::default();
    for &index in indices {
        sample.set(index, true);
    }
    sample
}
fn after() -> RawCommand {
    command(121, &[2_000_001, 99, 0, 0])
}

#[test]
fn input126_separates_event_and_player_gates_and_persists_in_save() {
    let raw = [
        command(126, &[0x1f7]),
        command(180, &[1]),
        command(126, &[0x201]),
        command(180, &[1]),
    ];
    let mut vm = vm(&raw);
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.input_policy().logical, 0xf7);
    assert_eq!(vm.input_policy().player, 0);
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.input_policy().logical, 0xf6);
    assert_eq!(vm.input_policy().player, 0);
    let saved = serde_json::to_value(vm.state()).unwrap();
    assert_eq!(saved["keyboard"]["policy"]["logical"], 0xf6);
    let mut policy = vm.input_policy();
    policy.update(1);
    assert_eq!(policy.logical, 0xf7);
    assert_eq!(policy.player, 1);
}

#[test]
fn native_zero_and_negative_waits_continue_but_positive_wait_returns() {
    for ticks in [-100, 0, 1, 3] {
        let mut vm = vm(&[command(180, &[ticks]), after()]);
        vm.tick(vec![]).unwrap();
        if ticks <= 0 {
            assert_eq!(vm.state().world.globals.numbers[&1], 99);
        } else {
            assert_eq!(vm.state().fibers[&1].frames[0].pc, 1);
            assert!(!vm.state().world.globals.numbers.contains_key(&1));
            for _ in 1..ticks {
                vm.tick(vec![]).unwrap();
                assert!(!vm.state().world.globals.numbers.contains_key(&1));
            }
            vm.tick(vec![]).unwrap();
            assert_eq!(vm.state().world.globals.numbers[&1], 99);
        }
    }
}

#[test]
fn scan_uses_native_order_exclusions_and_ignores_unused_third_argument() {
    // Common22 PC9's flags don't enable the third operand as a key filter.
    let mut vm = vm(&[command(123, &[2_000_000, 0x101, 100])]);
    vm.set_keyboard(sample(&[28, 57, 146, 5, 2]));
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 102);
    let mut vm = super_vm_scan();
    vm.set_keyboard(sample(&[1, 28, 42, 54, 14, 57, 58, 112, 147]));
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 0);
}
fn super_vm_scan() -> Vm {
    vm(&[command(123, &[2_000_000, 0x101, 100])])
}

#[test]
fn specified_keyboard_modes_use_current_pressed_and_released_states() {
    for (mode, before, now, expected) in [
        (0, true, true, 128),
        (1, true, true, 128),
        (2, false, true, 128),
        (2, true, true, 0),
        (3, true, false, 128),
        (3, false, true, 0),
    ] {
        let mut vm = vm(&[
            command(180, &[1]),
            command(123, &[2_000_000, 0x1100 | mode << 16, 128]),
        ]);
        vm.set_keyboard(sample(if before { &[28] } else { &[] }));
        vm.tick(vec![]).unwrap();
        vm.set_keyboard(sample(if now { &[28] } else { &[] }));
        vm.tick(vec![]).unwrap();
        assert_eq!(
            vm.state().world.globals.numbers[&0],
            expected,
            "mode {mode}"
        );
    }
}

#[test]
fn wait_advances_pc_once_preserves_zero_result_and_resumes_on_following_tick() {
    let mut vm = vm(&[command(123, &[2_000_000, 0x180, 100]), after()]);
    vm.set_global(0, 77).unwrap();
    vm.tick(vec![]).unwrap();
    for _ in 0..3 {
        vm.tick(vec![]).unwrap();
        assert_eq!(vm.state().fibers[&1].frames[0].pc, 1);
        assert_eq!(vm.state().world.globals.numbers[&0], 77);
    }
    vm.set_keyboard(sample(&[5]));
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 105);
    assert_eq!(vm.state().fibers[&1].wait, Wait::Ready);
    assert!(!vm.state().world.globals.numbers.contains_key(&1));
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&1], 99);
}

#[test]
fn blocked_wait_writes_zero_without_unblocking_or_accepting_a_held_key() {
    let mut vm = vm(&[command(123, &[2_000_000, 0x1180, 128]), after()]);
    vm.tick(vec![]).unwrap();
    vm.set_global(0, 77).unwrap();
    let mut down = sample(&[28]);
    down.blocked = true;
    vm.set_keyboard(down);
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 0);
    assert!(matches!(vm.state().fibers[&1].wait, Wait::Keyboard { .. }));
    down.blocked = false;
    vm.set_keyboard(down);
    vm.tick(vec![]).unwrap();
    assert!(matches!(vm.state().fibers[&1].wait, Wait::Keyboard { .. }));
    vm.set_keyboard(sample(&[]));
    vm.tick(vec![]).unwrap();
    vm.set_keyboard(down);
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 128);
    assert_eq!(vm.state().fibers[&1].wait, Wait::Ready);
}

#[test]
fn wait_initialization_does_not_reaccept_the_key_that_started_it() {
    let mut vm = vm(&[command(123, &[2_000_000, 0x180, 100])]);
    vm.set_keyboard(sample(&[5]));
    vm.tick(vec![]).unwrap();
    assert_eq!(
        vm.state().fibers[&1].wait,
        Wait::Keyboard { key: 0, memory: 5 }
    );
    vm.tick(vec![]).unwrap();
    assert!(matches!(vm.state().fibers[&1].wait, Wait::Keyboard { .. }));
    vm.set_keyboard(sample(&[]));
    vm.tick(vec![]).unwrap();
    vm.set_keyboard(sample(&[5]));
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 105);
}

#[test]
fn filter_is_resolved_once_and_saved_wait_restores_without_reinitialization() {
    let raw = [command(123, &[2_000_000, 0x1180, 2_000_002]), after()];
    let mut vm = vm(&raw);
    vm.set_global(2, 128).unwrap();
    vm.tick(vec![]).unwrap();
    vm.set_global(2, 142).unwrap();
    let saved = vm.save().unwrap();
    let mut restored = Vm::restore(library(&raw), &saved, 2, 1 << 20).unwrap();
    assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    for instance in [&mut vm, &mut restored] {
        instance.set_keyboard(sample(&[28]));
        instance.tick(vec![]).unwrap();
        assert_eq!(instance.state().world.globals.numbers[&0], 128);
    }
    assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
    let mut corrupt: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    corrupt["fibers"]["1"]["frames"][0]["pc"] = 0.into();
    assert!(Vm::restore(
        library(&raw),
        &serde_json::to_vec(&corrupt).unwrap(),
        2,
        1 << 20
    )
    .is_err());
}

#[test]
fn replacing_page_cancels_input_wait_and_does_not_write_old_destination() {
    let mut vm = vm(&[command(123, &[2_000_000, 0x180])]);
    vm.tick(vec![]).unwrap();
    vm.replace_page(1, Target::Id("main".into())).unwrap();
    vm.set_keyboard(sample(&[5]));
    vm.tick(vec![]).unwrap();
    assert!(!vm.state().world.globals.numbers.contains_key(&0));
    vm.cancel(1).unwrap();
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
}

#[test]
fn keyboard_history_is_shared_across_fibers_and_advances_once_per_tick() {
    let mut vm = vm(&[command(123, &[1_100_000, 0x21100, 128])]);
    vm.spawn(Target::Id("main".into()), 10, vec![]).unwrap();
    vm.set_keyboard(sample(&[28]));
    vm.tick(vec![]).unwrap();
    for owner in [9, 10] {
        assert_eq!(vm.state().world.events[&owner].numbers[&0], 128);
    }
}

#[test]
fn unsupported_devices_counter_mode_and_unknown_flags_still_fault_at_source() {
    for flags in [0x300, 0x40090, 0x40100, 0x200100, 0x102] {
        let mut vm = vm(&[command(123, &[2_000_000, flags, 128])]);
        vm.tick(vec![]).unwrap();
        let fiber = &vm.state().fibers[&1];
        assert_eq!(fiber.wait, Wait::Fault);
        assert_eq!(fiber.frames[0].pc, 0);
        assert_eq!(
            fiber
                .fault
                .as_ref()
                .unwrap()
                .source
                .as_ref()
                .unwrap()
                .command,
            201
        );
    }
}

#[test]
fn common22_pad_scan_selects_native_order_and_ignores_disabled_filter() {
    let mut vm = vm(&[command(123, &[2_000_000, 0x200, 100])]);
    let mut device = sample(&[]);
    device.pad_buttons = (1 << 18) | (1 << 5) | (1 << 1);
    vm.set_keyboard(device);
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 302);
}

#[test]
fn native_pad_wait_remembers_initial_buttons_and_preserves_poll_yield() {
    let raw = [command(123, &[2_000_000, 0x280]), after()];
    let mut vm = vm(&raw);
    let mut device = sample(&[]);
    device.pad_buttons = 1 << 4;
    vm.set_keyboard(device);
    vm.tick(vec![]).unwrap();
    let saved = vm.save().unwrap();
    let mut restored = Vm::restore(library(&raw), &saved, 2, 1 << 20).unwrap();
    for instance in [&mut vm, &mut restored] {
        instance.tick(vec![]).unwrap();
        assert!(matches!(
            instance.state().fibers[&1].wait,
            Wait::NativeInput { .. }
        ));
        device.pad_buttons = 0;
        instance.set_keyboard(device);
        instance.tick(vec![]).unwrap();
        device.pad_buttons = 1 << 4;
        instance.set_keyboard(device);
        instance.tick(vec![]).unwrap();
        assert_eq!(instance.state().world.globals.numbers[&0], 301);
        assert!(!instance.state().world.globals.numbers.contains_key(&1));
        instance.tick(vec![]).unwrap();
        assert_eq!(instance.state().world.globals.numbers[&1], 99);
    }
    assert_eq!(vm.state_hash().unwrap(), restored.state_hash().unwrap());
}

#[test]
fn native_logical_input_uses_published_selector_and_action_priority() {
    let mut vm = vm(&[
        command(123, &[2_000_000, 1]),
        command(123, &[2_000_001, 0x71]),
    ]);
    let mut device = sample(&[]);
    device.logical.directions[0] = 8;
    device.logical.buttons = 7;
    vm.set_keyboard(device);
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 8);
    assert_eq!(vm.state().world.globals.numbers[&1], 10);
}

#[test]
fn logical_title_wait_does_not_accept_already_held_confirm() {
    let mut vm = vm(&[command(123, &[2_000_000, 0x10090]), after()]);
    let mut device = sample(&[]);
    device.logical.buttons = 1;
    vm.set_keyboard(device);
    vm.tick(vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    assert!(matches!(
        vm.state().fibers[&1].wait,
        Wait::NativeInput { .. }
    ));
    device.logical.buttons = 0;
    vm.set_keyboard(device);
    vm.tick(vec![]).unwrap();
    device.logical.buttons = 1;
    vm.set_keyboard(device);
    vm.tick(vec![]).unwrap();
    assert_eq!(vm.state().world.globals.numbers[&0], 10);
    assert!(!vm.state().world.globals.numbers.contains_key(&1));
}

#[test]
fn native_pad_edges_and_out_of_table_explicit_codes() {
    for (mode, old, now, key, expected) in [
        (2, 0, 1 << 4, 301, 301),
        (2, 1 << 4, 1 << 4, 301, 0),
        (3, 1 << 4, 0, 301, 301),
        (0, 0, u32::MAX, 316, 0),
    ] {
        let mut vm = vm(&[
            command(180, &[1]),
            command(123, &[2_000_000, 0x1200 | mode << 16, key]),
        ]);
        let mut device = sample(&[]);
        device.pad_buttons = old;
        vm.set_keyboard(device);
        vm.tick(vec![]).unwrap();
        device.pad_buttons = now;
        vm.set_keyboard(device);
        vm.tick(vec![]).unwrap();
        assert_eq!(vm.state().world.globals.numbers[&0], expected);
    }
}

#[test]
fn input_wait_replays_to_identical_hashes() {
    let raw = [command(123, &[2_000_000, 0x180]), after()];
    let mut recorder = Recorder::new(vm(&raw), 4).unwrap();
    recorder.tick(Input::default()).unwrap();
    recorder.tick(Input::default()).unwrap();
    recorder
        .tick(Input {
            keyboard: Some(sample(&[5])),
            ..Default::default()
        })
        .unwrap();
    recorder.tick(Input::default()).unwrap();
    let (vm, tape) = recorder.finish();
    let replay = tape.verify(library(&raw), 2, 1 << 20).unwrap();
    assert_eq!(vm.state_hash().unwrap(), replay.state_hash().unwrap());
}
