//! Real default-entry menu integration. Input follows the platform adapter and Common22.
use super::Game;
use std::path::Path;
use winit::keyboard::KeyCode;

#[path = "native_takeover_tests.rs"]
mod takeover;

#[test]
#[ignore = "private game archives, four real title branches; isolated save copies"]
fn local_title_menu_branches() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap();
    let selected_branch = std::env::var("WOLF_MENU_BRANCH").ok();
    if let Some(branch) = &selected_branch {
        assert!(["new-game","continue","settings","quit"].contains(&branch.as_str()),"unknown WOLF_MENU_BRANCH {branch:?}");
    }
    let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let run=root.join("wolf-rs/validation/menu-branches").join(format!("run-{}-{nonce}",std::process::id()));
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(root.join("wolf-rs/validation/menu-branches/latest-run.txt"),run.to_string_lossy().as_bytes()).unwrap();
    eprintln!("MENU output={}",run.display());
    let mut failures = Vec::new();
    for (name, choice) in [("new-game", 0), ("continue", 1), ("settings", 2), ("quit", 4)] {
        if selected_branch.as_deref().is_some_and(|filter| filter != name) { continue; }
        let sandbox = run.join(name);
        std::fs::create_dir_all(sandbox.join("Save")).unwrap();
        for entry in std::fs::read_dir(root.join("Save")).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() && !entry.file_name().to_string_lossy().ends_with(".wolf-rs") {
                std::fs::copy(entry.path(), sandbox.join("Save").join(entry.file_name())).unwrap();
            }
        }
        if name == "continue" {
            // A numbered-slot fixture uses a byte-identical copy of the real
            // native autosave. The user's original Save directory stays untouched.
            std::fs::copy(root.join("Save/auto.sav"), sandbox.join("Save/SaveData01.sav")).unwrap();
            std::fs::write(sandbox.join("Save/currentsav.txt"), b"sav:1").unwrap();
        }
        let mut game = Game::startup(root).unwrap();
        game.set_save_root(&sandbox.join("Save")).unwrap();
        let mut keyboard = crate::keyboard::Keyboard::default();
        let mut down_until = 0;
        let mut branch_entered = false;
        let mut failed = false;
        let mut cancelled = false;
        let mut previous_volume = None;
        let mut adjusted_volume = None;
        let mut restored_ticks = None;
        let mut new_game_ready = false;
        let mut new_game_ready_tick = None;
        for _ in 0..6000 {
            let tick = game.vm.state().tick;
            if (550..800).contains(&tick) && tick.is_multiple_of(30) && game.vm.state().world.commons.numbers.get(&2322) != Some(&choice) {
                down_until = tick + 3;
            }
            let branch_confirm = match name {
                "quit" => (1200..1203).contains(&tick) || (1400..1403).contains(&tick),
                "settings" => (1050..1053).contains(&tick),
                "continue" => (1000..1003).contains(&tick) || (1200..1203).contains(&tick),
                "new-game" => tick >= 1000 && tick % 90 < 3,
                _ => false,
            };
            let help_open = game.vm.state().fibers.values().any(|fiber| fiber.frames.iter().any(|frame| frame.program == "common/94" && frame.pc >= 213));
            // Common94 requires reading through its pages before confirming.
            let help_last = game.vm.state().world.commons.numbers.get(&9430) == game.vm.state().world.commons.numbers.get(&9431);
            keyboard.key(KeyCode::Enter, (!help_open || help_last) && restored_ticks.is_none() && ((300..303).contains(&tick) || (820..823).contains(&tick) || branch_confirm));
            keyboard.key(KeyCode::ArrowRight, name == "new-game" && help_open && !help_last && tick % 90 < 3);
            keyboard.key(KeyCode::Escape, name == "quit" && (1050..1053).contains(&tick));
            // Common15 uses the configured cancel action; Escape is the
            // independent action101. The fixture binds cancel to physical X.
            keyboard.key(KeyCode::KeyX, name == "settings" && ((1250..1253).contains(&tick) || (1400..1403).contains(&tick)));
            keyboard.key(KeyCode::ArrowLeft, name == "settings" && (1150..1153).contains(&tick));
            keyboard.key(KeyCode::ArrowDown, tick < down_until);
            let input = keyboard.sample_with_pad_buttons(game.vm.input_policy(), game.vm.input_bindings(), 0);
            game.vm.set_keyboard(input);
            if let Err(error) = game.tick() {
                eprintln!("MENU {name} FAILED tick {tick}: {error}");
                for f in game.vm.state().fibers.values().filter(|f| f.fault.is_some()) { eprintln!("MENU {name} fiber {} {:?}", f.id, f.fault); }
                failures.push(format!("{name}: {error}"));
                let _ = game.render().and_then(|image| image.write_png(std::fs::File::create(sandbox.join("first-fault.png")).map_err(|e|e.to_string())?));
                failed = true;
                break;
            }
            let image = game.render().unwrap();
            if name == "new-game" && new_game_ready_tick.is_none() && game.scene.as_ref().is_some_and(|scene|scene.map_id()==1) && game.vm.state().world.globals.numbers.get(&0)==Some(&1) {
                new_game_ready = true;
                new_game_ready_tick = Some(tick);
                image.write_png(std::fs::File::create(sandbox.join("room-ready.png")).unwrap()).unwrap();
                if std::env::var_os("WOLF_MENU_HOLD_NEW_GAME").is_none() { break; }
            }
            if name == "new-game" && std::env::var_os("WOLF_MENU_HOLD_NEW_GAME").is_some() && new_game_ready_tick.is_some_and(|start| tick >= start + 20) { break; }
            if name == "settings" && ((1249..1257).contains(&tick) || (1399..1407).contains(&tick)) {
                eprintln!("SETTINGS tick={tick} input={:?} pulse={:?} frames={:?}", game.vm.state().world.globals.numbers.get(&3), game.vm.state().world.globals.numbers.get(&5), game.vm.state().fibers.values().flat_map(|fiber|fiber.frames.iter().filter(|frame|frame.program=="common/15").map(|frame|frame.pc)).collect::<Vec<_>>());
            }
            if name == "continue" && game.scene.as_ref().is_some_and(|scene|scene.map_id() != 2) {
                let elapsed = restored_ticks.get_or_insert(0u32);
                *elapsed += 1;
                if *elapsed == 180 {
                    image.write_png(std::fs::File::create(sandbox.join("restored-180.png")).unwrap()).unwrap();
                    break;
                }
            }
            if name == "settings" && tick == 1049 { previous_volume = game.vm.state().world.systems.numbers.get(&100).copied(); }
            if name == "settings" && tick == 1249 { adjusted_volume = game.vm.state().world.systems.numbers.get(&100).copied(); }
            if name == "quit" && tick == 1150 {
                cancelled = !game.exit_requested() && !game.vm.state().fibers.values().any(|fiber|fiber.frames.iter().any(|frame|frame.program=="common/18"));
            }
            if game.vm.state().tick == 820 { eprintln!("MENU {name} selection {:?}", game.vm.state().world.commons.numbers.get(&2322)); }
            if [1000, 1500].contains(&game.vm.state().tick) { image.write_png(std::fs::File::create(sandbox.join(format!("tick-{}.png",game.vm.state().tick))).unwrap()).unwrap(); }
            for f in game.vm.state().fibers.values() {
                for frame in &f.frames {
                    if (name == "continue" && frame.program == "common/10") || (name == "settings" && frame.program == "common/15") || (name == "quit" && frame.program == "common/18") || (name == "new-game" && frame.program.starts_with("MapData/Map001.mps/")) { branch_entered = true; }
                }
            }
            if game.exit_requested() { break; }
            if name != "new-game" && game.vm.state().tick >= 1500 { break; }
        }
        eprintln!("MENU {name} entered={branch_entered} map={} tick={}", game.map_name, game.vm.state().tick);
        for fiber in game.vm.state().fibers.values().filter(|fiber| !fiber.frames.is_empty()) {
            eprintln!("MENU {name} final fiber {} frames {:?}", fiber.id, fiber.frames.iter().map(|frame|(&frame.program,frame.pc)).collect::<Vec<_>>());
        }
        if !failed && !branch_entered { failures.push(format!("{name}: branch was not entered")); }
        if !failed && name == "quit" && (!cancelled || !game.exit_requested()) { failures.push(format!("quit cancel={cancelled}, confirmed exit={}",game.exit_requested())); }
        if !failed && name == "settings" {
            if previous_volume == adjusted_volume || previous_volume.is_none() { failures.push(format!("settings volume did not change: {previous_volume:?}->{adjusted_volume:?}")); }
            if game.vm.state().fibers.values().any(|fiber|fiber.frames.iter().any(|frame|frame.program=="common/15")) { failures.push("settings did not return to title".into()); }
        }
        if !failed && name == "continue" && game.scene.as_ref().is_none_or(|scene|scene.map_id()==2) { failures.push("continue did not restore a scene".into()); }
        if !failed && name == "continue" && restored_ticks != Some(180) { failures.push(format!("continue post-restore ticks={restored_ticks:?}")); }
        if !failed && name == "new-game" && !new_game_ready { failures.push("new game did not finish the opening room transition".into()); }
    }
    assert!(failures.is_empty(), "menu failures: {failures:#?}");
}
