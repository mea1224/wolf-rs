//! Application save boundary. Native saves and the versioned Rust host format
//! are intentionally distinct; the latter retains executing fibers and waits.
use super::{CommonSlot, Game, MapSlot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wolf_vm::runtime::Vm;

const CAP: usize = 512 << 20;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    session: u64,
    vm: serde_json::Value,
    host: serde_json::Value,
    services: serde_json::Value,
    scene: Option<crate::native_scene::Scene>,
    map: wolf_format::Map,
    map_name: String,
    map_slots: BTreeMap<u32, MapSlot>,
    common_slots: Vec<CommonSlot>,
    characters: BTreeMap<i32, (String, String)>,
    #[serde(default)]
    text_input: Option<crate::native_control::text_input::TextInput>,
}

impl Game {
    /// Explicit diagnostic entry used to measure a state reached by the real
    /// input-driven integration test. Normal startup never calls this method.
    pub(crate) fn diagnostic_restore_checkpoint(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), String> {
        if std::fs::metadata(path)
            .map_err(|error| error.to_string())?
            .len()
            > CAP as u64
        {
            return Err("diagnostic checkpoint byte limit".into());
        }
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let (payload, world): (Vec<u8>, wolf_vm::runtime::World) =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        self.restore_host(crate::native_control::HostRestoreRequest {
            path: path.to_string_lossy().into_owned(),
            slot: 1,
            payload,
            world,
        })
    }

    pub(super) fn capture_host(&mut self) -> Result<Vec<u8>, String> {
        // Poll at this tick boundary so the paired Host snapshot owns all
        // responses. They will be reissued by poll after a restore.
        let poll = self.host.poll(&self.vm, &[], |_| {
            Err("save encountered an unbound asset request".into())
        })?;
        self.completions = poll.completions;
        let saved = Snapshot {
            version: 5,
            session: self.vm.session(),
            vm: serde_json::from_slice(&self.vm.save()?).map_err(|e| e.to_string())?,
            host: serde_json::from_slice(&self.host.save(&self.vm, 32 << 20)?)
                .map_err(|e| e.to_string())?,
            services: serde_json::from_slice(&self.services.save()?).map_err(|e| e.to_string())?,
            scene: self.scene.clone(),
            map: self.map.clone(),
            map_name: self.map_name.clone(),
            map_slots: self.map_slots.clone(),
            common_slots: self.common_slots.clone(),
            characters: self.control.characters.clone(),
            text_input: self.control.text_input.clone(),
        };
        let mut bytes = crate::native_control::BoundedBytes::new(CAP);
        serde_json::to_writer(&mut bytes, &saved)
            .map_err(|e| format!("application snapshot: {e}"))?;
        Ok(bytes.bytes)
    }

    pub(super) fn restore_host(
        &mut self,
        request: crate::native_control::HostRestoreRequest,
    ) -> Result<(), String> {
        if request.payload.len() > CAP {
            return Err("application save byte limit".into());
        }
        // Check the envelope before decoding Scene and VM map contexts.
        let mut value: serde_json::Value =
            serde_json::from_slice(&request.payload).map_err(|e| e.to_string())?;
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if !matches!(version, Some(4 | 5)) {
            return Err(format!("unsupported application snapshot version {}; expected 4 or 5 (older movement and map execution states are incompatible)", value.get("version").unwrap_or(&serde_json::Value::Null)));
        }
        if version == Some(4) {
            // V4 omitted the configured initial route step. Recover only this
            // constant from Game.dat; active actor routes retain saved overrides.
            let bytes = crate::assets::read_member(&self.root, "BasicData/Game.dat")?;
            let config =
                wolf_format::parse_game(&bytes, &Default::default()).map_err(|e| e.to_string())?;
            let mode = *config
                .configuration
                .get(8)
                .ok_or("saved scene step configuration missing")?;
            if let Some(scene) = value
                .get_mut("scene")
                .and_then(serde_json::Value::as_object_mut)
            {
                scene.insert("configured_step_mode".into(), serde_json::Value::from(mode));
            }
            value["version"] = serde_json::Value::from(5);
        }
        let saved: Snapshot = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if let Some(input) = &saved.text_input {
            input.validate()?;
        }
        let session = self
            .vm
            .session()
            .max(saved.session)
            .checked_add(1)
            .ok_or("restore generation exhausted")?;
        let mut vm = Vm::restore(
            self.vm.library().clone(),
            &serde_json::to_vec(&saved.vm).map_err(|e| e.to_string())?,
            session,
            256 << 20,
        )?;
        let host = wolf_vm::host::Host::restore(
            &serde_json::to_vec(&saved.host).map_err(|e| e.to_string())?,
            &vm,
            32 << 20,
        )?;
        vm.replace_saved_world(request.world)?;
        vm.set_system(25, request.slot)?;
        for slot in &saved.common_slots {
            if !vm.state().fibers.contains_key(&slot.fiber) {
                return Err("saved common slot has no fiber".into());
            }
        }
        for (&id, slot) in &saved.map_slots {
            let event = saved
                .map
                .events
                .iter()
                .find(|event| event.id == id)
                .ok_or("saved map slot has no event")?;
            if slot.selected.is_some_and(|page| page >= event.pages.len())
                || slot
                    .fiber
                    .is_some_and(|id| !vm.state().fibers.contains_key(&id))
            {
                return Err("invalid saved map slot".into());
            }
        }
        let mut services = crate::native_services::Services::restore_with_save_root(
            &self.root,
            self.control.save_root(),
            &serde_json::to_vec(&saved.services).map_err(|e| e.to_string())?,
        )?;
        // Decode/compose candidate assets before publishing any replacement
        // state: a missing image or invalid atlas must leave the live game intact.
        let scene_renderer = if let Some(scene) = &saved.scene {
            let renderer = crate::native_scene_render::SceneRenderer::new(&mut services)?;
            let player = saved
                .characters
                .get(&1)
                .map(|value| value.0.as_bytes())
                .unwrap_or_default();
            let image = renderer.render(
                scene,
                saved.map.tileset,
                player,
                vm.state().tick,
                &mut services,
            )?;
            services.set_scene_image(image)?;
            Some(renderer)
        } else {
            None
        };
        self.vm = vm;
        self.host = host;
        self.services = services;
        self.scene = saved.scene;
        self.scene_renderer = scene_renderer;
        self.map = saved.map;
        self.map_name = saved.map_name;
        self.map_programs = self.map.events.iter().map(|event| event.pages.len()).sum();
        self.map_slots = saved.map_slots;
        self.common_slots = saved.common_slots;
        self.control.characters = saved.characters;
        self.control.text_input = saved.text_input;
        self.completions.clear();
        self.exit_requested = false;
        println!(
            "HOST RESTORE path={} slot={} map={}",
            request.path, request.slot, self.map_name
        );
        Ok(())
    }

    pub(super) fn restore_native(
        &mut self,
        request: crate::native_control::RestoreRequest,
    ) -> Result<(), String> {
        let snapshot = &request.snapshot;
        // Construct and validate the entire replacement before committing it.
        let mut next = Game::startup(&self.root)?;
        if let Some(save_root) = self.control.save_root() {
            next.services.set_save_root(save_root);
        }
        next.vm.clear_execution();
        next.map_slots.clear();
        next.common_slots.clear();
        let world = snapshot.restore_world(&next.vm.state().world)?;
        next.vm
            .replace_world_for_restore(world, snapshot.native_rng)?;
        next.transfer(wolf_vm::native_commands::MapTransferRequest {
            map_id: Some(snapshot.map_id),
            x_half: snapshot.player.x2,
            y_half: snapshot.player.y2,
            direction: Some(i32::from(snapshot.player.direction)),
            flags: 0,
        })?;
        next.vm.clear_execution();
        for slot in next.map_slots.values_mut() {
            slot.fiber = None;
        }
        next.vm.replace_world_for_restore(
            snapshot.restore_world(&next.vm.state().world)?,
            snapshot.native_rng,
        )?;
        next.vm.set_system(25, request.slot)?;
        next.scene
            .as_mut()
            .ok_or("restore scene missing")?
            .restore_snapshot(snapshot)?;
        next.services
            .set_camera_offset(next.scene.as_ref().unwrap().camera_offset());
        next.services.restore_native(snapshot)?;
        let player_graphic = String::from_utf8(snapshot.player.graphic.clone())
            .map_err(|_| "saved player graphic is not UTF-8")?;
        let mut restored_characters = BTreeMap::new();
        restored_characters.insert(1, (player_graphic, String::new()));
        next.install_common_slots(&self.root)?;
        next.maintain_map_slots()?;
        next.control.characters = restored_characters;
        next.refresh_scene_image()?;
        self.vm = next.vm;
        self.host = next.host;
        self.services = next.services;
        self.scene = next.scene;
        self.scene_renderer = next.scene_renderer;
        self.map = next.map;
        self.map_name = next.map_name;
        self.map_programs = next.map_programs;
        self.map_slots = next.map_slots;
        self.common_slots = next.common_slots;
        self.control.characters = next.control.characters;
        self.completions.clear();
        self.exit_requested = false;
        println!(
            "NATIVE RESTORE path={} slot={} map={}",
            request.path, request.slot, self.map_name
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn host_sidecar_restores_complete_application_and_scene_graphics() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        // Only load the real startup model; this test never advances the title flow.
        let mut game = Game::startup(&root).unwrap();
        game.vm.set_global(100_002, 77).unwrap();
        let scene = game.scene.as_mut().unwrap();
        scene.relocate(3, 5, Some(8)).unwrap();
        let event = game
            .map
            .events
            .iter()
            .find(|event| !event.pages.is_empty())
            .unwrap();
        let mut page = event.pages[0].clone();
        page.graphic_fields[0] = 6;
        page.graphic_fields[1] = 2;
        page.graphic_fields[2] = 123;
        page.graphic_fields[3] = 1;
        page.page_transfer = None;
        page.unknown = 10;
        scene.set_page(event.id, Some(&page)).unwrap();
        scene.center_camera([13, -7]).unwrap();
        game.control
            .characters
            .insert(1, (String::new(), "face".into()));
        game.refresh_scene_image().unwrap();
        let scene_before = serde_json::to_value(&game.scene).unwrap();
        let services_before: serde_json::Value =
            serde_json::from_slice(&game.services.save().unwrap()).unwrap();
        let slots_before = serde_json::to_value(&game.map_slots).unwrap();
        let common_before = serde_json::to_value(&game.common_slots).unwrap();
        let fibers_before = serde_json::to_value(&game.vm.state().fibers).unwrap();
        let characters_before = game.control.characters.clone();
        let session_before = game.vm.session();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let isolated =
            std::env::temp_dir().join(format!("wolf-app-save-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&isolated).unwrap();
        let mut writer = crate::native_control::Control::with_save_root(&root, &isolated);
        let payload = game.capture_host().unwrap();
        writer
            .complete_save(
                crate::native_control::SaveRequest {
                    path: "Save/SaveData01.sav".into(),
                    slot: 1,
                },
                &game.vm.state().world,
                payload,
            )
            .unwrap();
        assert!(!isolated.join("SaveData01.sav").exists());

        // A fresh Control must decode the on-disk envelope. Execute the real
        // Common10 load command, not a manufactured HostRestoreRequest.
        let bytes = crate::assets::read_member(&root, "BasicData/CommonEvent.dat").unwrap();
        let bytes = bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&bytes);
        let parsed = wolf_format::parse_common(bytes, &Default::default()).unwrap();
        let common = parsed
            .value
            .events
            .iter()
            .find(|event| event.id == 10)
            .unwrap();
        let load = &common.commands[691];
        assert_eq!(load.opcode, 220);
        assert_eq!(load.args, [1, 1_600_015]);
        let program = wolf_ir::compile(
            "app-reload".into(),
            "Common10 load".into(),
            wolf_ir::Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: "private-fixture".into(),
                event: 10,
                page: None,
                command: 691,
                span: [0, 0],
            },
            std::slice::from_ref(load),
        )
        .unwrap();
        let mut loader = Vm::new(
            wolf_ir::Library::new(vec![program]).unwrap(),
            Default::default(),
            1,
            1_000,
        )
        .unwrap();
        loader
            .spawn(wolf_ir::Target::Id("app-reload".into()), 0, vec![])
            .unwrap();
        let mut reader = crate::native_control::Control::with_save_root(&root, &isolated);
        let report = loader
            .tick_with_host(vec![], &mut |ctx| {
                ctx.set_number(1_600_015, 1)?;
                reader.execute(ctx)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert!(report.execution_invalidated);
        let request = reader.take_host_restore_request().unwrap();
        game.scene
            .as_mut()
            .unwrap()
            .relocate(0, 0, Some(2))
            .unwrap();
        game.vm.set_global(100_002, -99).unwrap();
        game.control.characters.clear();
        game.exit_requested = true;
        game.restore_host(request).unwrap();
        assert_eq!(game.vm.session(), session_before + 1);
        assert_eq!(game.vm.state().world.globals.numbers[&100_002], 77);
        assert_eq!(serde_json::to_value(&game.scene).unwrap(), scene_before);
        assert_eq!(serde_json::to_value(&game.map_slots).unwrap(), slots_before);
        assert_eq!(
            serde_json::to_value(&game.common_slots).unwrap(),
            common_before
        );
        assert_eq!(
            serde_json::to_value(&game.vm.state().fibers).unwrap(),
            fibers_before
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&game.services.save().unwrap()).unwrap(),
            services_before
        );
        assert_eq!(game.control.characters, characters_before);
        assert!(!game.exit_requested);
        // Repeated capture/restore must serialize current state once, including
        // live scene pixels; it must never embed any earlier envelope recursively.
        let mut lengths = Vec::new();
        for _ in 0..3 {
            let payload = game.capture_host().unwrap();
            lengths.push(payload.len());
            game.restore_host(crate::native_control::HostRestoreRequest {
                path: "Save/repeated.sav".into(),
                slot: 1,
                world: game.vm.state().world.clone(),
                payload,
            })
            .unwrap();
        }
        assert!(
            lengths.iter().all(|&length| length <= lengths[0] + 1024),
            "snapshot growth: {lengths:?}"
        );
        assert!(
            lengths[0] < 16 << 20,
            "startup snapshot should remain compact: {lengths:?}"
        );
        let mut invalid: serde_json::Value =
            serde_json::from_slice(&game.capture_host().unwrap()).unwrap();
        assert_eq!(invalid["version"], 5);
        let mut v4 = invalid.clone();
        v4["version"] = serde_json::Value::from(4);
        v4["scene"]
            .as_object_mut()
            .unwrap()
            .remove("configured_step_mode");
        let scene_before_migration = serde_json::to_value(&game.scene).unwrap();
        game.restore_host(crate::native_control::HostRestoreRequest {
            path: "Save/migrate-v4.sav".into(),
            slot: 1,
            world: game.vm.state().world.clone(),
            payload: serde_json::to_vec(&v4).unwrap(),
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(&game.scene).unwrap(),
            scene_before_migration,
            "v4 migration must recover configuration without resetting live route overrides"
        );
        let mut old = invalid.clone();
        old["version"] = serde_json::Value::from(2);
        // An old Scene can lack newly required native movement fields. The
        // version diagnostic must precede deserialization of that old layout.
        old["scene"] = serde_json::json!({});
        assert!(game
            .restore_host(crate::native_control::HostRestoreRequest {
                path: "Save/old-v2.sav".into(),
                slot: 99,
                world: game.vm.state().world.clone(),
                payload: serde_json::to_vec(&old).unwrap(),
            })
            .unwrap_err()
            .contains("older movement and map execution states are incompatible"));
        invalid["map"]["tileset"] = serde_json::Value::from(u32::MAX);
        let vm_before_failure = game.vm.save().unwrap();
        let scene_before_failure = serde_json::to_value(&game.scene).unwrap();
        let services_before_failure = game.services.save().unwrap();
        let request = crate::native_control::HostRestoreRequest {
            path: "Save/invalid.sav".into(),
            slot: 99,
            world: game.vm.state().world.clone(),
            payload: serde_json::to_vec(&invalid).unwrap(),
        };
        assert!(game
            .restore_host(request)
            .unwrap_err()
            .contains("tileset missing"));
        assert_eq!(game.vm.save().unwrap(), vm_before_failure);
        assert_eq!(
            serde_json::to_value(&game.scene).unwrap(),
            scene_before_failure
        );
        assert_eq!(game.services.save().unwrap(), services_before_failure);
        std::fs::remove_file(isolated.join("SaveData01.sav.wolf-rs")).unwrap();
        std::fs::remove_dir(isolated).unwrap();
    }
}
