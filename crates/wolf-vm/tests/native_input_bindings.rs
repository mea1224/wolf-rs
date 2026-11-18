use wolf_format::{RawCommand, Span};
use wolf_ir::{compile, Library, Source, Target};
use wolf_vm::runtime::{Config, Vm, Wait};

fn run(writes: &[(i32, i32)], native: bool) -> Vm {
    let raw: Vec<_> = writes
        .iter()
        .map(|&(slot, value)| RawCommand {
            span: Span { start: 0, end: 1 },
            opcode: 121,
            args: vec![9_000_000 + slot, value, 0, 4],
            indent: 0,
            strings: vec![],
            terminator: 0,
            movement: None,
            extension_length: 0,
            extension: vec![],
        })
        .collect();
    let source = Source {
        asset: "binding-test".into(),
        content_blake3: "synthetic".into(),
        event: 8,
        page: None,
        command: 0,
        span: [0, 1],
    };
    let program = compile("main".into(), "".into(), source, &raw).unwrap();
    let library = Library::new(vec![program]).unwrap();
    let mut vm = Vm::new(
        library,
        Config {
            run_until_yield: native,
            ..Default::default()
        },
        1,
        1,
    )
    .unwrap();
    for (slot, value) in [
        (52, 144),
        (53, 145),
        (54, 146),
        (55, 301),
        (56, 302),
        (57, 303),
    ] {
        vm.set_system(slot, value).unwrap();
    }
    vm.spawn(Target::Id("main".into()), 1, vec![]).unwrap();
    vm.tick(vec![]).unwrap();
    vm
}

#[test]
fn native_keyboard_and_pad_binding_collisions_exchange_existing_assignments() {
    // All six collision directions, independently in both device banks.
    for base in [52, 55] {
        let initial = if base == 52 {
            [144, 145, 146]
        } else {
            [301, 302, 303]
        };
        for target in 0..3 {
            for old in 0..3 {
                let vm = run(&[(base + target as i32, initial[old])], true);
                assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
                let mut expected = initial;
                expected.swap(target, old);
                let actual = if base == 52 {
                    vm.input_bindings().0
                } else {
                    vm.input_bindings().1
                };
                assert_eq!(actual, expected, "base={base}, target={target}, old={old}");
            }
        }
    }
}

#[test]
fn native_binding_ranges_accept_boundaries_and_ignore_invalid_values() {
    let vm = run(
        &[
            (52, 100),
            (53, 355),
            (54, 99),
            (54, 356),
            (55, 301),
            (56, 320),
            (57, 300),
            (57, 321),
        ],
        true,
    );
    assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
    assert_eq!(vm.input_bindings(), ([100, 355, 146], [301, 320, 303]));
    let vm = run(&[(52, 145), (52, 146), (55, 302), (55, 303)], true);
    assert_eq!(vm.input_bindings(), ([146, 144, 145], [303, 301, 302]));
}

#[test]
fn binding_setters_remain_native_profile_only() {
    let vm = run(&[(52, 145)], false);
    assert!(vm.state().fibers[&1]
        .fault
        .as_ref()
        .unwrap()
        .message
        .contains("platform/profile adapter"));
    assert_eq!(vm.input_bindings(), ([144, 145, 146], [301, 302, 303]));
}

#[test]
fn native_settings_system_writes_clamp_frame_skip_and_ignore_invalid_window_mode() {
    for (input, expected) in [(-7, 0), (0, 0), (1, 1), (2, 2), (99, 2)] {
        let vm = run(&[(33, input)], true);
        assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
        assert_eq!(vm.native_number(9_000_033).unwrap(), expected);
    }
    for mode in 0..=2 {
        let vm = run(&[(138, mode), (138, -1), (138, 3)], true);
        assert_eq!(vm.state().fibers[&1].wait, Wait::Done);
        assert_eq!(vm.native_number(9_000_138).unwrap(), mode);
    }
}
