//! Physical-key settings exercise; saves are redirected before the first tick.
use super::Game;
use std::path::Path;
use winit::keyboard::KeyCode;
use wolf_vm::native_values::{DatabaseAddress, DatabaseKind, NativeValue};

fn advance(
    game: &mut Game,
    keyboard: &mut crate::keyboard::Keyboard,
    ticks: u32,
    keys: &[KeyCode],
) {
    advance_with_pad(game, keyboard, ticks, keys, 0);
}

fn advance_with_pad(
    game: &mut Game,
    keyboard: &mut crate::keyboard::Keyboard,
    ticks: u32,
    keys: &[KeyCode],
    pad: u32,
) {
    for _ in 0..ticks {
        for key in [
            KeyCode::Enter,
            KeyCode::ArrowDown,
            KeyCode::ArrowUp,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::KeyQ,
            KeyCode::KeyE,
            KeyCode::KeyX,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyR,
            KeyCode::KeyT,
            KeyCode::KeyF,
            KeyCode::KeyG,
            KeyCode::KeyH,
            KeyCode::KeyJ,
            KeyCode::KeyK,
            KeyCode::KeyL,
            KeyCode::KeyY,
            KeyCode::KeyU,
            KeyCode::KeyI,
            KeyCode::Escape,
            KeyCode::ShiftLeft,
        ] {
            keyboard.key(key, keys.contains(&key));
        }
        let input =
            keyboard.sample_with_pad_buttons(game.vm.input_policy(), game.vm.input_bindings(), pad);
        game.vm.set_keyboard(input);
        if let Err(error) = game.tick() {
            panic!(
                "settings tick {}: {error}; frames={:?}",
                game.vm.state().tick,
                game.vm
                    .state()
                    .fibers
                    .values()
                    .flat_map(|fiber| fiber.frames.iter().map(|frame| (&frame.program, frame.pc)))
                    .collect::<Vec<_>>()
            );
        }
        // Transition completion requires its final draw boundary, including
        // in an integration test with no native window.
        game.render().unwrap();
    }
}

fn pulse(game: &mut Game, keyboard: &mut crate::keyboard::Keyboard, key: KeyCode) {
    advance(game, keyboard, 3, &[key]);
    advance(game, keyboard, 65, &[]);
}

fn active(game: &Game, id: u32) -> bool {
    let name = format!("common/{id}");
    game.vm
        .state()
        .fibers
        .values()
        .any(|fiber| fiber.frames.iter().any(|frame| frame.program == name))
}

fn number(game: &Game, ty: usize, row: usize, field: usize) -> i32 {
    database_number(game, DatabaseKind::Mutable, ty, row, field)
}

fn database_number(game: &Game, kind: DatabaseKind, ty: usize, row: usize, field: usize) -> i32 {
    match game
        .vm
        .state()
        .world
        .databases
        .get(DatabaseAddress {
            kind,
            type_id: ty,
            row,
            field,
        })
        .unwrap()
    {
        NativeValue::Number(value) => *value,
        _ => panic!("expected numeric settings field"),
    }
}

fn assert_selected(game: &Game, common: u32, row: usize) {
    assert_eq!(
        game.vm
            .state()
            .world
            .commons
            .numbers
            .get(&((common * 100 + 62) as i32)),
        Some(&(row as i32)),
        "Common{common} selected the wrong row"
    );
}

// Exercise the actual script's type and range metadata. Expectations here are
// independent of the VM's arithmetic implementation (Common15 PC303..358).
fn change_setting(game: &mut Game, keyboard: &mut crate::keyboard::Keyboard, row: usize) {
    let kind = database_number(game, DatabaseKind::User, 4, row, 3);
    assert!(
        kind == 0 || kind == 1,
        "row {row} is not a scalar setting: {kind}"
    );
    let max = if kind == 0 {
        100
    } else {
        database_number(game, DatabaseKind::User, 4, row, 4)
    };
    let before = number(game, 9, row, 0);
    let expected = if kind == 0 {
        (before - 10).max(0)
    } else if before == 0 {
        max
    } else {
        before - 1
    };
    pulse(game, keyboard, KeyCode::ArrowLeft);
    assert_eq!(number(game, 9, row, 0), expected, "setting row {row} left");
    let mut expected = if kind == 0 {
        (expected + 10).min(100)
    } else if expected == max {
        0
    } else {
        expected + 1
    };
    pulse(game, keyboard, KeyCode::ArrowRight);
    assert_eq!(number(game, 9, row, 0), expected, "setting row {row} right");
    if kind == 0 {
        // Held input traverses Common22's repeat path as well as both clamps.
        advance(game, keyboard, 120, &[KeyCode::ArrowLeft]);
        advance(game, keyboard, 65, &[]);
        assert_eq!(number(game, 9, row, 0), 0, "setting row {row} lower bound");
        advance(game, keyboard, 120, &[KeyCode::ArrowRight]);
        advance(game, keyboard, 65, &[]);
        assert_eq!(
            number(game, 9, row, 0),
            100,
            "setting row {row} upper bound"
        );
        expected = 100;
    } else {
        // Visit every enum value and the wrap edge using archive range metadata.
        for _ in 0..=max {
            expected = if expected == max { 0 } else { expected + 1 };
            pulse(game, keyboard, KeyCode::ArrowRight);
            assert_eq!(
                number(game, 9, row, 0),
                expected,
                "setting row {row} enum cycle"
            );
        }
    }
    // Finish with a changed value, so reloading cannot pass by restoring defaults.
    let direction = if expected > 0 {
        KeyCode::ArrowLeft
    } else {
        KeyCode::ArrowRight
    };
    pulse(game, keyboard, direction);
    assert_ne!(
        number(game, 9, row, 0),
        expected,
        "setting row {row} did not change"
    );
    for (setting, system) in [(0, 100), (1, 102), (3, 101), (6, 138), (11, 33)] {
        if row == setting {
            assert_eq!(
                game.vm.state().world.systems.numbers.get(&system),
                Some(&number(game, 9, row, 0)),
                "setting row {row} did not reach system {system}"
            );
        }
    }
    eprintln!(
        "SETTINGS row={row} kind={kind} range=0..{max} value={}",
        number(game, 9, row, 0)
    );
}

fn settings_fixture() -> (
    &'static Path,
    std::path::PathBuf,
    Game,
    crate::keyboard::Keyboard,
) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let isolated = root
        .join("wolf-rs/validation/settings-extended")
        .join(nonce.to_string());
    std::fs::create_dir_all(&isolated).unwrap();
    for entry in std::fs::read_dir(root.join("Save")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file()
            && entry.path().extension().is_some_and(|ext| ext == "txt")
        {
            std::fs::copy(entry.path(), isolated.join(entry.file_name())).unwrap();
        }
    }
    // Common23 PC3..32 initializes defaults when no numbered save is valid.
    // Keep that native first-run branch separate from configuration persistence.
    std::fs::copy(root.join("Save/auto.sav"), isolated.join("SaveData01.sav")).unwrap();
    let mut game = Game::startup(root).unwrap();
    game.set_save_root(&isolated).unwrap();
    let mut keyboard = crate::keyboard::Keyboard::default();
    let mut down_until = 0;
    for tick in 0..900 {
        if (550..800).contains(&tick)
            && tick % 30 == 0
            && game.vm.state().world.commons.numbers.get(&2322) != Some(&2)
        {
            down_until = tick + 3;
        }
        let keys = if (300..303).contains(&tick) || (820..823).contains(&tick) {
            vec![KeyCode::Enter]
        } else if tick < down_until {
            vec![KeyCode::ArrowDown]
        } else {
            vec![]
        };
        advance(&mut game, &mut keyboard, 1, &keys);
    }
    assert!(active(&game, 15), "settings did not open");
    advance(&mut game, &mut keyboard, 140, &[]);
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    (root, isolated, game, keyboard)
}

#[test]
#[ignore = "private archives; all settings rows, keyboard/pad bindings and isolated persistence"]
fn local_settings_tabs_keys_and_persistence() {
    let (root, isolated, mut game, mut keyboard) = settings_fixture();
    // Both navigation edges wrap, including the language row (edited last,
    // because the real script deliberately closes settings after that change).
    pulse(&mut game, &mut keyboard, KeyCode::ArrowUp);
    assert_selected(&game, 15, 7);
    pulse(&mut game, &mut keyboard, KeyCode::ArrowDown);
    assert_selected(&game, 15, 0);
    for row in 0..7 {
        assert_selected(&game, 15, row);
        change_setting(&mut game, &mut keyboard, row);
        if row != 6 {
            pulse(&mut game, &mut keyboard, KeyCode::ArrowDown);
        }
    }
    let volume = number(&game, 9, 0, 0);
    pulse(&mut game, &mut keyboard, KeyCode::KeyE);
    assert_eq!(
        game.vm.state().world.commons.numbers.get(&1532),
        Some(&1),
        "second tab did not open"
    );
    pulse(&mut game, &mut keyboard, KeyCode::ArrowUp);
    assert_selected(&game, 15, 4);
    pulse(&mut game, &mut keyboard, KeyCode::ArrowDown);
    assert_selected(&game, 15, 0);
    let key_index = (0..5)
        .find(|&index| {
            game.vm
                .state()
                .world
                .databases
                .get(DatabaseAddress {
                    kind: DatabaseKind::User,
                    type_id: 4,
                    row: 8 + index,
                    field: 3,
                })
                .is_ok_and(|value| *value == NativeValue::Number(2))
        })
        .expect("key binding row in settings database");
    for index in 0..5 {
        assert_selected(&game, 15, index);
        if index != key_index {
            change_setting(&mut game, &mut keyboard, 8 + index);
        }
        if index != 4 {
            pulse(&mut game, &mut keyboard, KeyCode::ArrowDown);
        }
    }
    let skip = game.vm.state().world.systems.numbers.get(&33).copied();
    for _ in key_index..4 {
        pulse(&mut game, &mut keyboard, KeyCode::ArrowUp);
    }
    let old_key = number(&game, 8, 0, 0);
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    assert!(active(&game, 16), "key configuration did not open");
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    // Reserved physical codes must keep the editor waiting and preserve data.
    pulse(&mut game, &mut keyboard, KeyCode::KeyA);
    assert_eq!(
        number(&game, 8, 0, 0),
        old_key,
        "reserved A key was assigned"
    );
    pulse(
        &mut game,
        &mut keyboard,
        // Common16 PC330/340 explicitly excludes A/S/D and fixed engine keys.
        if old_key == 119 {
            KeyCode::KeyT
        } else {
            KeyCode::KeyR
        },
    );
    let key = number(&game, 8, 0, 0);
    assert_ne!(key, old_key, "physical key was not assigned");
    let assignments = [
        (KeyCode::KeyF, 133),
        (KeyCode::KeyG, 134),
        (KeyCode::KeyH, 135),
        (KeyCode::KeyJ, 136),
        (KeyCode::KeyK, 137),
        (KeyCode::KeyL, 138),
        (KeyCode::KeyY, 121),
        (KeyCode::KeyU, 122),
        (KeyCode::KeyI, 123),
    ];
    for (row, (physical, native)) in assignments.into_iter().enumerate() {
        assert_selected(&game, 16, row);
        pulse(&mut game, &mut keyboard, KeyCode::Enter);
        pulse(&mut game, &mut keyboard, physical);
        assert_eq!(number(&game, 8, row, 0), native, "key row {row} assignment");
        if row < 8 {
            pulse(&mut game, &mut keyboard, KeyCode::ArrowDown);
        }
    }
    // A duplicate key must exchange the previous binding with its former owner.
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    pulse(&mut game, &mut keyboard, KeyCode::KeyF);
    assert_eq!(number(&game, 8, 8, 0), 133);
    assert_eq!(number(&game, 8, 0, 0), 123);
    for _ in 0..8 {
        pulse(&mut game, &mut keyboard, KeyCode::ArrowUp);
    }
    assert_selected(&game, 16, 0);
    // Select an existing pad binding so both the pad assignment and duplicate
    // exchange use the native Common16 path and the real device adapter.
    let old_pad = number(&game, 8, 0, 1);
    let other_pad = number(&game, 8, 7, 1);
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    advance_with_pad(&mut game, &mut keyboard, 3, &[], 1 << (other_pad - 297));
    advance(&mut game, &mut keyboard, 65, &[]);
    assert_eq!(number(&game, 8, 0, 1), other_pad);
    assert_eq!(number(&game, 8, 7, 1), old_pad);
    assert_eq!(game.vm.input_bindings().1[0], other_pad);
    let expected_keys: Vec<_> = (0..9)
        .map(|row| (number(&game, 8, row, 0), number(&game, 8, row, 1)))
        .collect();
    let key = expected_keys[0].0;
    pulse(&mut game, &mut keyboard, KeyCode::KeyG);
    assert!(!active(&game, 16), "key configuration did not close");
    // The newly bound tab-left key must switch back after configuration returns.
    pulse(&mut game, &mut keyboard, KeyCode::KeyL);
    assert_eq!(game.vm.state().world.commons.numbers.get(&1532), Some(&0));
    pulse(&mut game, &mut keyboard, KeyCode::ArrowUp);
    assert_selected(&game, 15, 7);
    let previous_language = number(&game, 9, 7, 0);
    pulse(&mut game, &mut keyboard, KeyCode::ArrowRight);
    assert_ne!(number(&game, 9, 7, 0), previous_language);
    assert!(
        active(&game, 18),
        "language change confirmation did not open"
    );
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    advance(&mut game, &mut keyboard, 120, &[]);
    assert!(
        !active(&game, 15) && !active(&game, 16),
        "settings did not close"
    );
    let expected_settings: Vec<_> = (0..13).map(|row| number(&game, 9, row, 0)).collect();
    assert!(
        isolated.join("game_config.txt").exists(),
        "configuration was not written"
    );
    game.render()
        .unwrap()
        .write_png(std::fs::File::create(isolated.join("closed.png")).unwrap())
        .unwrap();
    for name in ["game_config.txt", "key_config.txt"] {
        std::fs::copy(
            isolated.join(name),
            isolated.join(format!("before-reload-{name}")),
        )
        .unwrap();
    }
    let mut reloaded = Game::startup(root).unwrap();
    reloaded.set_save_root(&isolated).unwrap();
    advance(
        &mut reloaded,
        &mut crate::keyboard::Keyboard::default(),
        550,
        &[],
    );
    assert_eq!(
        number(&reloaded, 9, 0, 0),
        volume,
        "volume did not survive reload"
    );
    for (row, expected) in expected_settings.into_iter().enumerate() {
        assert_eq!(
            number(&reloaded, 9, row, 0),
            expected,
            "setting row {row} did not survive reload"
        );
    }
    assert_eq!(
        number(&reloaded, 8, 0, 0),
        expected_keys[0].0,
        "key assignment did not survive reload"
    );
    for (row, (key, pad)) in expected_keys.into_iter().enumerate() {
        assert_eq!(
            number(&reloaded, 8, row, 0),
            key,
            "key row {row} persistence"
        );
        assert_eq!(
            number(&reloaded, 8, row, 1),
            pad,
            "pad row {row} persistence"
        );
    }
    assert_eq!(
        Some(number(&reloaded, 9, 11, 0)),
        skip,
        "quality database setting did not survive reload"
    );
    eprintln!(
        "SETTINGS verified volume={volume} key={key} skip={skip:?}; {}",
        isolated.display()
    );
}

#[test]
#[ignore = "private archives; settings and key reset accept/cancel, isolated configuration"]
fn local_settings_reset_branches() {
    let (_root, isolated, mut game, mut keyboard) = settings_fixture();
    let original = number(&game, 9, 0, 0);
    pulse(&mut game, &mut keyboard, KeyCode::ArrowLeft);
    let changed = number(&game, 9, 0, 0);
    assert_ne!(changed, original);
    pulse(&mut game, &mut keyboard, KeyCode::ShiftLeft);
    assert!(active(&game, 18), "settings reset dialog missing");
    pulse(&mut game, &mut keyboard, KeyCode::KeyX);
    assert!(!active(&game, 18));
    assert!(active(&game, 15));
    assert_eq!(
        number(&game, 9, 0, 0),
        changed,
        "declined reset changed volume"
    );
    pulse(&mut game, &mut keyboard, KeyCode::ShiftLeft);
    assert!(active(&game, 18));
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    advance(&mut game, &mut keyboard, 120, &[]);
    assert!(
        !active(&game, 15),
        "accepted settings reset did not exit through label jump"
    );
    for row in 0..13 {
        let default = if row == 7 {
            game.vm.state().world.systems.numbers[&141]
        } else {
            database_number(&game, DatabaseKind::User, 4, row, 12)
        };
        assert_eq!(number(&game, 9, row, 0), default, "reset setting {row}");
    }
    // The title retains selection 2 when Common15 returns.
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    advance(&mut game, &mut keyboard, 140, &[]);
    assert!(active(&game, 15));
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    pulse(&mut game, &mut keyboard, KeyCode::KeyE);
    let key_index = (0..5)
        .find(|&i| database_number(&game, DatabaseKind::User, 4, 8 + i, 3) == 2)
        .unwrap();
    for _ in 0..key_index {
        pulse(&mut game, &mut keyboard, KeyCode::ArrowDown);
    }
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    assert!(active(&game, 16));
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    pulse(&mut game, &mut keyboard, KeyCode::KeyF);
    assert_eq!(number(&game, 8, 0, 0), 133);
    // Ten selectable rows: nine bindings, then the reset command.
    pulse(&mut game, &mut keyboard, KeyCode::ArrowUp);
    assert_selected(&game, 16, 9);
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    assert!(active(&game, 18));
    pulse(&mut game, &mut keyboard, KeyCode::KeyX);
    assert_eq!(
        number(&game, 8, 0, 0),
        133,
        "declined key reset changed binding"
    );
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    assert!(active(&game, 18));
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    advance(&mut game, &mut keyboard, 120, &[]);
    assert!(
        active(&game, 16),
        "key reset did not restart menu through backward label"
    );
    assert_selected(&game, 16, 0);
    for row in 0..9 {
        assert_eq!(
            number(&game, 8, row, 0),
            database_number(&game, DatabaseKind::User, 3, row, 1),
            "reset keyboard {row}"
        );
        assert_eq!(
            number(&game, 8, row, 1),
            database_number(&game, DatabaseKind::User, 3, row, 2),
            "reset pad {row}"
        );
    }
    for _ in 0..3 {
        pulse(&mut game, &mut keyboard, KeyCode::KeyX);
    }
    assert!(!active(&game, 15) && !active(&game, 16));
    eprintln!(
        "SETTINGS reset accept/cancel, forward exit and backward restart passed; {}",
        isolated.display()
    );
}
