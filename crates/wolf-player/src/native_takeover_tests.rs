//! Archive fixture regression through physical input, including real save UI.
use super::Game;
use crate::keyboard::Keyboard;
use std::path::Path;
use winit::keyboard::KeyCode;

fn active(game: &Game, name: &str) -> bool {
    game.vm
        .state()
        .fibers
        .values()
        .any(|fiber| fiber.frames.iter().any(|frame| frame.program == name))
}

fn common(game: &Game, id: i32) -> i32 {
    game.vm
        .state()
        .world
        .commons
        .numbers
        .get(&id)
        .copied()
        .unwrap_or_default()
}

fn step(game: &mut Game, keyboard: &mut Keyboard, keys: &[KeyCode]) {
    for key in [
        KeyCode::Enter,
        KeyCode::ArrowDown,
        KeyCode::ArrowUp,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::KeyC,
        KeyCode::KeyX,
        KeyCode::KeyQ,
        KeyCode::KeyE,
        KeyCode::ShiftLeft,
    ] {
        keyboard.key(key, keys.contains(&key));
    }
    game.vm.set_keyboard(keyboard.sample_with_pad_buttons(
        game.vm.input_policy(),
        game.vm.input_bindings(),
        0,
    ));
    if let Err(error) = game.tick() {
        panic!(
            "takeover tick {}: {error}; frames={:?}",
            game.vm.state().tick,
            game.vm
                .state()
                .fibers
                .values()
                .map(|fiber| (
                    fiber.id,
                    &fiber.fault,
                    fiber
                        .frames
                        .iter()
                        .map(|frame| (&frame.program, frame.pc))
                        .collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>()
        );
    }
    game.render().unwrap();
}

fn idle(game: &mut Game, keyboard: &mut Keyboard, ticks: usize) {
    for _ in 0..ticks {
        step(game, keyboard, &[]);
    }
}

fn pulse(game: &mut Game, keyboard: &mut Keyboard, key: KeyCode) {
    for _ in 0..3 {
        step(game, keyboard, &[key]);
    }
    idle(game, keyboard, 65);
}

fn wait_for(game: &mut Game, keyboard: &mut Keyboard, label: &str, ready: impl Fn(&Game) -> bool) {
    for _ in 0..600 {
        if ready(game) {
            return;
        }
        step(game, keyboard, &[]);
    }
    panic!(
        "timeout waiting for {label}; tick={} position={:?} policy={:?} frames={:?}",
        game.vm.state().tick,
        game.scene.as_ref().map(|scene| scene.position()),
        game.vm.input_policy(),
        game.vm
            .state()
            .fibers
            .values()
            .flat_map(|fiber| fiber.frames.iter().map(|frame| (&frame.program, frame.pc)))
            .collect::<Vec<_>>()
    );
}

fn move_once(game: &mut Game, keyboard: &mut Keyboard, key: KeyCode) {
    let before = game.scene.as_ref().unwrap().position();
    step(game, keyboard, &[key]);
    idle(game, keyboard, 60);
    let after = game.scene.as_ref().unwrap().position();
    assert_ne!(
        (before.0, before.1),
        (after.0, after.1),
        "player movement did not resume"
    );
    assert!(
        !game.scene.as_ref().unwrap().player_view(&[]).moving,
        "physical movement did not finish its interpolation"
    );
}

fn direction_key(direction: i32) -> KeyCode {
    match direction { 2 => KeyCode::ArrowDown, 4 => KeyCode::ArrowLeft, 6 => KeyCode::ArrowRight, 8 => KeyCode::ArrowUp, _ => panic!("invalid navigation direction") }
}

fn walk_out_and_back(game: &mut Game, keyboard: &mut Keyboard) {
    let scene = game.scene.as_ref().unwrap();
    let initial = scene.position();
    let direction = [4, 6, 2, 8].into_iter().find(|&direction| {
        let mut probe = scene.clone();
        probe.apply_player_input(direction).unwrap();
        let moved = probe.position();
        if (moved.0, moved.1) == (initial.0, initial.1) { return false; }
        probe.relocate(moved.0, moved.1, None).unwrap();
        probe.apply_player_input(10 - direction).unwrap();
        let returned = probe.position();
        (returned.0, returned.1) == (initial.0, initial.1)
    }).expect("no reversible walking step at actual takeover location");
    move_once(game, keyboard, direction_key(direction));
    move_once(game, keyboard, direction_key(10 - direction));
    let returned = game.scene.as_ref().unwrap().position();
    assert_eq!((returned.0, returned.1), (initial.0, initial.1));
}

fn move_available(game: &mut Game, keyboard: &mut Keyboard) {
    let initial = game.scene.as_ref().unwrap().position();
    let direction = [4, 6, 2, 8].into_iter().find(|&direction| {
        let mut scene = game.scene.as_ref().unwrap().clone();
        scene.apply_player_input(direction).unwrap();
        let next = scene.position();
        (initial.0, initial.1) != (next.0, next.1)
    }).expect("no walkable neighboring position");
    move_once(game, keyboard, direction_key(direction));
}

/// Search the actual selected scene's walkable half-grid. Event IDs identify
/// private fixtures only; no production navigation or story logic is supplied.
fn navigate_to_action(game: &mut Game, keyboard: &mut Keyboard, event_id: u32) {
    use std::collections::{BTreeSet, VecDeque};
    let scene = game.scene.as_ref().unwrap();
    let page_index = game.map_slots[&event_id].selected.expect("interaction page is inactive");
    let page = &game.map.events.iter().find(|event| event.id == event_id).unwrap().pages[page_index];
    let initial = scene.position();
    let mut queue = VecDeque::from([(initial, Vec::<i32>::new())]);
    let mut seen = BTreeSet::from([initial]);
    let mut route = None;
    while let Some((position, path)) = queue.pop_front() {
        let mut probe = scene.clone();
        probe.relocate(position.0, position.1, Some(i32::from(position.2))).unwrap();
        if probe.map_trigger_candidates(&[(event_id, page)], true, true, None).unwrap().contains(&event_id) {
            route = Some(path.clone());
            break;
        }
        for direction in [2, 4, 6, 8] {
            let mut probe = scene.clone();
            probe.relocate(position.0, position.1, Some(i32::from(position.2))).unwrap();
            probe.apply_player_input(direction).unwrap();
            let p = probe.position();
            if seen.insert(p) {
                let mut next = path.clone();
                next.push(direction);
                queue.push_back((p, next));
            }
        }
    }
    let route = route.unwrap_or_else(|| {
        let event = game.map.events.iter().find(|event| event.id == event_id).unwrap();
        let near: Vec<_> = seen.iter().filter(|&&(x,y,_)| (x-event.x as i32*2).abs() <= 4 && (y-event.y as i32*2).abs() <= 4).collect();
        let player = scene.player_view(&[]);
        let event_view = scene.visible_events().find(|actor| actor.id == event_id as i32);
        let probes: Vec<_> = [
            (36, 16, 8), (36, 16, 2), (34, 16, 6), (36, 18, 8), (36, 18, 6),
        ].into_iter().map(|(x,y,d)| {
            let mut probe = scene.clone();
            probe.relocate(x,y,Some(d)).unwrap();
            ((x,y,d), probe.map_trigger_candidates(&[(event_id,page)],true,true,None).unwrap())
        }).collect();
        panic!("selected event has no reachable interaction footprint: event={event_id}, page={page_index}, flags={}, route_flags={}, trigger={}, graphic={:?}, graphic_fields={:?}, shadow={:?}, actor={event_view:?}, player={:?}, probes={probes:?}, near={near:?}, seen={}", page.flags, page.route_flags, page.conditions[0], page.graphic.raw, page.graphic_fields, page.shadow_collision, (player.position,player.direction), seen.len());
    });
    eprintln!("NAVIGATION map={} event={event_id} start={initial:?} route={route:?}", game.map_name);
    for direction in route {
        step(game, keyboard, &[direction_key(direction)]);
        idle(game, keyboard, 60);
    }
    let page_index = game.map_slots[&event_id].selected.unwrap();
    let page = &game.map.events.iter().find(|event| event.id == event_id).unwrap().pages[page_index];
    assert!(game.scene.as_ref().unwrap().map_trigger_candidates(&[(event_id, page)], true, true, None).unwrap().contains(&event_id));
}

fn interact_with_bed(game: &mut Game, keyboard: &mut Keyboard) {
    navigate_to_action(game, keyboard, 7);
    let mut saw_dialogue = false;
    for tick in 0..600 {
        step(
            game,
            keyboard,
            if tick % 90 < 3 {
                &[KeyCode::Enter]
            } else {
                &[]
            },
        );
        saw_dialogue |= active(game, "common/37");
        if saw_dialogue && !active(game, "common/37") && game.vm.input_policy().player == 0xf7 {
            idle(game, keyboard, 30);
            return;
        }
    }
    panic!(
        "bed dialogue failed to start or return; started={saw_dialogue}, position={:?}",
        game.scene.as_ref().unwrap().position()
    );
}

fn enter_mirror(game: &mut Game, keyboard: &mut Keyboard) {
    navigate_to_action(game, keyboard, 3);
    let source_map = game.scene.as_ref().unwrap().map_id();
    let mut source_fiber = None;
    let mut transferred = false;
    let mut completed_tail = false;
    for tick in 0..18_000 {
        step(game, keyboard, if tick % 90 < 3 { &[KeyCode::Enter] } else { &[] });
        if source_fiber.is_none() {
            source_fiber = game.vm.state().fibers.values().find(|fiber| {
                fiber.frames.iter().any(|frame| frame.program == "MapData/Map001.mps/3/0")
            }).map(|fiber| (fiber.id, fiber.page_generation));
        }
        transferred |= game.scene.as_ref().unwrap().map_id() != source_map;
        if transferred {
            if let Some((id, generation)) = source_fiber {
                let fiber = &game.vm.state().fibers[&id];
                assert_eq!(fiber.page_generation, generation, "transfer cancelled its initiating main context");
                completed_tail |= matches!(fiber.wait, wolf_vm::runtime::Wait::Done);
            }
            if completed_tail && !game.main_active() && game.vm.input_policy().player == 0xf7
                && game.scene.as_ref().unwrap().player_control_ready() {
                eprintln!("MIRROR transfer complete: map={} tick={} source_fiber={source_fiber:?}", game.scene.as_ref().unwrap().map_id(), game.vm.state().tick);
                move_available(game, keyboard);
                return;
            }
        }
    }
    panic!("mirror continuation did not finish: transferred={transferred}, completed_tail={completed_tail}, source_fiber={source_fiber:?}, map={}", game.scene.as_ref().unwrap().map_id());
}

#[test]
#[ignore = "private archive sprite animation fixtures; no story reachability claim"]
fn local_stationary_effect_sprites_advance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap();
    let mut services = Game::services(root).unwrap();
    let limits = wolf_format::Limits::default();
    let config = wolf_format::parse_game(&crate::assets::read_member(root, "BasicData/Game.dat").unwrap(), &limits).unwrap();
    let output = root.join("wolf-rs/validation/native-scene-movement/effect-animation");
    std::fs::create_dir_all(&output).unwrap();
    for (map_name, event_id) in [("Map001.mps",10), ("Map031.mps",3), ("Map000.mps",48)] {
        let map = wolf_format::parse_map(&crate::assets::read_member(root, &format!("MapData/{map_name}")).unwrap(), &limits).unwrap().value;
        let fixture = map.events.iter().find(|event| event.id == event_id).unwrap();
        let mut scene = crate::native_scene::Scene::initial(&map, &wolf_ir::startup::InitialPosition {
            map_id: 0, x: fixture.x as i32, y: fixture.y as i32, map_path: format!("MapData/{map_name}")
        }, vec![]).unwrap();
        scene.set_game_configuration(&config).unwrap();
        for event in &map.events {
            scene.set_initial_page(event.id, (event.id == event_id).then_some(&event.pages[0])).unwrap();
        }
        let mut world = wolf_vm::runtime::World::default();
        let mut rng = 1;
        let mut phases = std::collections::BTreeMap::new();
        let mut raster_hashes = std::collections::BTreeSet::new();
        for tick in 0..180 {
            scene.tick_routes([0,0], &mut world, &mut rng).unwrap();
            let actor = scene.visible_events().find(|actor| actor.id == event_id as i32).unwrap();
            if phases.contains_key(&actor.frame) { continue; }
            phases.insert(actor.frame, tick);
            let path = std::str::from_utf8(actor.graphic.strip_suffix(&[0]).unwrap_or(actor.graphic)).unwrap();
            let sprite = services.image_resource(path).unwrap();
            let layout = crate::native_actor_render::layout(crate::native_actor_render::ActorSpec {
                graphic: actor.graphic, tile_id: actor.tile_id, direction: actor.direction, frame: actor.frame, animated: actor.animated
            }, sprite.dimensions().into(), scene.tile_pixels().unwrap() as u32, config.configuration[1], config.configuration[7]).unwrap();
            let mut image = wolf_media::Image::new(layout.size[0] as usize,layout.size[1] as usize,vec![24;layout.size[0] as usize*layout.size[1] as usize*4]).unwrap();
            for pixel in image.bytes_mut().chunks_exact_mut(4) { pixel[3]=255; }
            crate::native_scene_render::blit(&mut image, &sprite, &crate::native_scene_render::Blit {
                source:layout.source,destination:[0,0,layout.size[0] as i32,layout.size[1] as i32],opacity:actor.opacity as u8,blend:actor.blend as u8
            }).unwrap();
            raster_hashes.insert(image.bytes().to_vec());
            image.write_png(std::fs::File::create(output.join(format!("{map_name}-event{event_id}-frame{}.png",actor.frame))).unwrap()).unwrap();
        }
        eprintln!("EFFECT {map_name} event{event_id} phase_first_ticks={phases:?}, distinct_rasters={}",raster_hashes.len());
        assert!(phases.len() >= 3, "stationary effect did not advance through configured frames");
        assert!(raster_hashes.len() >= 2, "animated effect still renders a static image");
    }
}

fn open_menu(game: &mut Game, keyboard: &mut Keyboard) {
    pulse(game, keyboard, KeyCode::KeyC);
    wait_for(game, keyboard, "game menu", |game| active(game, "common/9"));
    idle(game, keyboard, 100);
    let menu_contexts = game.vm.state().fibers.values().filter(|fiber| {
        fiber.frames.iter().any(|frame| frame.program == "common/9")
    }).count();
    assert_eq!(menu_contexts, 1, "held menu key started overlapping main contexts");
    assert!(game.main_active(), "reserved menu must own the main context");
}

fn open_slot_menu(game: &mut Game, keyboard: &mut Keyboard, choice: i32) {
    select_menu_item(game, keyboard, choice);
    wait_for(game, keyboard, "save/load slots", |game| {
        active(game, "common/10")
    });
    idle(game, keyboard, 100);
    assert_eq!(common(game, 1000), choice - 2, "wrong save/load mode");
}

fn select_menu_item(game: &mut Game, keyboard: &mut Keyboard, choice: i32) {
    for _ in 0..5 {
        if common(game, 938) == choice {
            break;
        }
        pulse(game, keyboard, KeyCode::ArrowRight);
    }
    assert_eq!(
        common(game, 938),
        choice,
        "menu cursor did not move to requested item"
    );
    pulse(game, keyboard, KeyCode::Enter);
}

fn close_menu(game: &mut Game, keyboard: &mut Keyboard) {
    for _ in 0..4 {
        if !active(game, "common/9") && !active(game, "common/10") {
            break;
        }
        pulse(game, keyboard, KeyCode::KeyX);
    }
    wait_for(
        game,
        keyboard,
        "menu cancellation and player control",
        |game| {
            !active(game, "common/9")
                && !active(game, "common/10")
                && game.vm.input_policy().player == 0xf7
        },
    );
}

#[test]
#[ignore = "private archives; full opening, physical map interaction, isolated save/load UI"]
fn local_takeover_interaction_and_save_roundtrip() {
    let selected_case = std::env::var("WOLF_TAKEOVER_CASE").ok();
    assert!(
        selected_case.as_deref().is_none_or(|case| [
            "interaction",
            "settings",
            "empty-shift",
            "save",
            "transfer"
        ]
        .contains(&case)),
        "unknown WOLF_TAKEOVER_CASE"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let isolated = root
        .join("wolf-rs/validation/takeover-save")
        .join(format!("{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&isolated).unwrap();
    // Only independent settings are copied. Every numbered slot is new and all
    // writes from the game itself stay in this test's unique Save root.
    for entry in std::fs::read_dir(root.join("Save")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file()
            && entry.path().extension().is_some_and(|ext| ext == "txt")
        {
            std::fs::copy(entry.path(), isolated.join(entry.file_name())).unwrap();
        }
    }
    eprintln!("TAKEOVER save root={}", isolated.display());
    let mut game = Game::startup(root).unwrap();
    game.set_save_root(&isolated).unwrap();
    let mut keyboard = Keyboard::default();
    if let Some(checkpoint) = std::env::var_os("WOLF_TAKEOVER_RESTORE") {
        let (payload, world): (Vec<u8>, wolf_vm::runtime::World) =
            serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
        game.restore_host(crate::native_control::HostRestoreRequest {
            path: Path::new(&checkpoint).to_string_lossy().into_owned(),
            slot: 1,
            payload,
            world,
        })
        .unwrap();
    } else {
        for tick in 0..6000 {
            let help_open = active(&game, "common/94");
            let help_last = common(&game, 9430) == common(&game, 9431);
            let keys: &[KeyCode] = if help_open && !help_last && tick % 90 < 3 {
                &[KeyCode::ArrowRight]
            } else if (!help_open || help_last)
                && ((300..303).contains(&tick)
                    || (820..823).contains(&tick)
                    || (tick >= 1000 && tick % 90 < 3))
            {
                &[KeyCode::Enter]
            } else {
                &[]
            };
            step(&mut game, &mut keyboard, keys);
            if game.scene.as_ref().is_some_and(|scene| scene.map_id() == 1)
                && game.vm.state().world.globals.numbers.get(&0) == Some(&1)
                && !help_open
                && game.vm.input_policy().player == 0xf7
            {
                break;
            }
        }
    }
    assert_eq!(game.scene.as_ref().unwrap().map_id(), 1);
    assert_eq!(
        game.vm.state().world.globals.numbers.get(&0),
        Some(&1),
        "opening did not complete"
    );
    idle(&mut game, &mut keyboard, 90);
    // An explicitly supplied checkpoint avoids replaying the opening while
    // iterating on a UI fault. It is captured from this exact physical run.
    let checkpoint = (game.capture_host().unwrap(), game.vm.state().world.clone());
    std::fs::write(
        isolated.join("takeover-checkpoint.json"),
        serde_json::to_vec(&checkpoint).unwrap(),
    )
    .unwrap();
    let initial = game.scene.as_ref().unwrap().position();
    eprintln!("TAKEOVER control position={initial:?}");
    if selected_case.as_deref() == Some("transfer") {
        enter_mirror(&mut game, &mut keyboard);
        return;
    }
    if selected_case
        .as_deref()
        .is_none_or(|case| case == "interaction")
    {
        walk_out_and_back(&mut game, &mut keyboard);
        interact_with_bed(&mut game, &mut keyboard);
        if selected_case.as_deref() == Some("interaction") {
            return;
        }
    }

    if selected_case
        .as_deref()
        .is_none_or(|case| case == "settings")
    {
        open_menu(&mut game, &mut keyboard);
        select_menu_item(&mut game, &mut keyboard, 4);
        wait_for(&mut game, &mut keyboard, "in-game settings", |game| {
            active(game, "common/15")
        });
        idle(&mut game, &mut keyboard, 150);
        pulse(&mut game, &mut keyboard, KeyCode::Enter);
        for _ in 0..2 {
            let tab = common(&game, 1532);
            pulse(&mut game, &mut keyboard, KeyCode::KeyE);
            assert_ne!(
                common(&game, 1532),
                tab,
                "in-game settings tab did not advance"
            );
            pulse(&mut game, &mut keyboard, KeyCode::KeyQ);
            assert_eq!(
                common(&game, 1532),
                tab,
                "in-game settings tab did not return"
            );
        }
        for _ in 0..4 {
            if !active(&game, "common/15") {
                break;
            }
            pulse(&mut game, &mut keyboard, KeyCode::KeyX);
        }
        wait_for(
            &mut game,
            &mut keyboard,
            "settings return to menu",
            |game| !active(game, "common/15"),
        );
        close_menu(&mut game, &mut keyboard);
        walk_out_and_back(&mut game, &mut keyboard);
        eprintln!("TAKEOVER in-game settings passed");
    }
    if selected_case
        .as_deref()
        .is_none_or(|case| case == "empty-shift")
    {
        open_menu(&mut game, &mut keyboard);
        open_slot_menu(&mut game, &mut keyboard, 3);
        // The isolated Save root contains no numbered saves. Reproduce rapid
        // Shift taps and longer held repeats before continuing normal input.
        for _ in 0..12 {
            step(&mut game, &mut keyboard, &[KeyCode::ShiftLeft]);
            idle(&mut game, &mut keyboard, 2);
        }
        for _ in 0..60 {
            step(&mut game, &mut keyboard, &[KeyCode::ShiftLeft]);
        }
        idle(&mut game, &mut keyboard, 100);
        assert!(
            active(&game, "common/10"),
            "empty-slot Shift unexpectedly closed the load UI"
        );
        let row = common(&game, 1034);
        pulse(&mut game, &mut keyboard, KeyCode::ArrowDown);
        assert_ne!(common(&game, 1034), row, "load cursor stalled after Shift");
        pulse(&mut game, &mut keyboard, KeyCode::Enter);
        assert!(
            active(&game, "common/10"),
            "confirming empty slot lost load UI"
        );
        close_menu(&mut game, &mut keyboard);
        walk_out_and_back(&mut game, &mut keyboard);
        eprintln!("TAKEOVER empty-slot repeated Shift passed");
    }
    if selected_case.as_deref().is_some_and(|case| case != "save") {
        return;
    }

    // Re-entering and paging before cancellation used to strand the menu fiber.
    for _ in 0..2 {
        open_menu(&mut game, &mut keyboard);
        open_slot_menu(&mut game, &mut keyboard, 2);
        let page = common(&game, 1032);
        pulse(&mut game, &mut keyboard, KeyCode::ArrowRight);
        assert_ne!(common(&game, 1032), page, "save page did not change");
        pulse(&mut game, &mut keyboard, KeyCode::ArrowLeft);
        assert_eq!(common(&game, 1032), page, "save page did not return");
        close_menu(&mut game, &mut keyboard);
    }
    open_menu(&mut game, &mut keyboard);
    open_slot_menu(&mut game, &mut keyboard, 2);
    let saved_position = game.scene.as_ref().unwrap().position();
    let slot = common(&game, 1034) + common(&game, 1032) * 8;
    assert!(
        slot > 0,
        "UI selected the autosave instead of a numbered slot"
    );
    let sidecar = isolated.join(format!("SaveData{slot:02}.sav.wolf-rs"));
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    wait_for(
        &mut game,
        &mut keyboard,
        "saved application sidecar",
        |_| sidecar.exists(),
    );
    idle(&mut game, &mut keyboard, 100);
    let screenshot = std::fs::read(isolated.join(format!("ss{slot}.jpg")))
        .expect("script did not create the selected slot screenshot");
    assert!(screenshot.starts_with(&[0xff, 0xd8]), "slot screenshot is not JPEG");
    wolf_media::Image::from_encoded(&screenshot).expect("slot screenshot is not decodable");
    close_menu(&mut game, &mut keyboard);
    move_available(&mut game, &mut keyboard);
    open_menu(&mut game, &mut keyboard);
    open_slot_menu(&mut game, &mut keyboard, 3);
    assert_eq!(
        common(&game, 1034) + common(&game, 1032) * 8,
        slot,
        "load selected another slot"
    );
    // A populated slot enters Common10's source-3 text editor; empty Shift
    // cannot cover this branch. Confirm and cancel must both return to slots.
    pulse(&mut game, &mut keyboard, KeyCode::ShiftLeft);
    wait_for(&mut game, &mut keyboard, "save memo input", Game::text_input_active);
    game.text_input_commit("保存メモ test");
    game.text_input_key(&winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter));
    idle(&mut game, &mut keyboard, 100);
    assert!(!game.text_input_active());
    assert_eq!(game.vm.state().world.systems.numbers.get(&39), Some(&0));
    assert!(active(&game, "common/10"), "memo confirmation lost slot UI");
    pulse(&mut game, &mut keyboard, KeyCode::ShiftLeft);
    wait_for(&mut game, &mut keyboard, "save memo cancel", Game::text_input_active);
    game.text_input_commit("discard");
    game.text_input_key(&winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape));
    idle(&mut game, &mut keyboard, 100);
    assert!(!game.text_input_active());
    assert_eq!(game.vm.state().world.systems.numbers.get(&39), Some(&-1));
    assert!(active(&game, "common/10"), "memo cancellation lost slot UI");
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    wait_for(&mut game, &mut keyboard, "load confirmation", |game| {
        active(game, "common/18")
    });
    pulse(&mut game, &mut keyboard, KeyCode::Enter);
    wait_for(&mut game, &mut keyboard, "restored gameplay", |game| {
        !active(game, "common/9")
            && !active(game, "common/10")
            && !active(game, "common/18")
            && game.vm.input_policy().player == 0xf7
            && game
                .vm
                .state()
                .world
                .globals
                .numbers
                .get(&100002)
                .copied()
                .unwrap_or_default()
                == 0
    });
    assert_eq!(
        game.scene.as_ref().unwrap().position(),
        saved_position,
        "save did not restore player position"
    );
    walk_out_and_back(&mut game, &mut keyboard);
    interact_with_bed(&mut game, &mut keyboard);
    game.render()
        .unwrap()
        .write_png(std::fs::File::create(isolated.join("restored-and-interacted.png")).unwrap())
        .unwrap();
    eprintln!(
        "TAKEOVER complete slot={slot} tick={} position={:?}",
        game.vm.state().tick,
        game.scene.as_ref().unwrap().position()
    );
}
