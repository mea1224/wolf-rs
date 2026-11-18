use std::{collections::BTreeMap, io::Read, path::{Path, PathBuf}};
use wolf_assets::wolf350::{Archive, Limits as ArchiveLimits};
use wolf_format::{Limits, Map, RawCommand};
use wolf_ir::{Coverage, Library, Program, Source, Target};

#[cfg(test)]
#[path = "native_menu_tests.rs"]
mod menu_tests;

#[cfg(test)]
#[path = "native_settings_tests.rs"]
mod settings_tests;

#[path = "native_game_save.rs"]
mod native_save;

pub struct Game {
    root: PathBuf,
    exit_requested: bool,
    map_slots: BTreeMap<u32, MapSlot>,
    pub map: Map,
    pub map_name: String,
    pub map_programs: usize,
    pub coverage: Coverage,
    pub vm: wolf_vm::runtime::Vm,
    pub host: wolf_vm::host::Host,
    completions: Vec<wolf_vm::runtime::Completion>,
    services: crate::native_services::Services,
    control: crate::native_control::Control,
    titles: crate::native_system::Titles,
    common_slots: Vec<CommonSlot>,
    scene: Option<crate::native_scene::Scene>,
    scene_renderer: Option<crate::native_scene_render::SceneRenderer>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct MapSlot {
    selected: Option<usize>,
    fiber: Option<u64>,
    #[serde(default)]
    pending: bool,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CommonSlot {
    common: u32,
    condition: wolf_ir::native_startup::CommonCondition,
    fiber: u64,
}

fn read_archive(path: &Path) -> Result<Archive, String> {
    let cap = ArchiveLimits::default();
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(cap.archive_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Archive::open(bytes, cap).map_err(|e| e.to_string())
}

impl Game {
    pub(crate) fn text_input_active(&self) -> bool { self.control.text_input.is_some() }
    pub(crate) fn text_input_key(&mut self, key: &winit::keyboard::Key) {
        if let Some(input) = &mut self.control.text_input { input.key(key); }
    }
    pub(crate) fn text_input_commit(&mut self, text: &str) {
        if let Some(input) = &mut self.control.text_input { input.commit(text); }
    }
    pub(crate) fn text_input_preedit(&mut self, text: &str) {
        if let Some(input) = &mut self.control.text_input { input.preedit(text); }
    }
    pub fn set_save_root(&mut self, path: &Path) -> Result<(), String> {
        self.control.set_save_root(path)?;
        self.services.set_save_root(path);
        Ok(())
    }
    /// Isolated real-archive command probe. It deliberately does not enter the game's title flow.
    pub fn input_probe(root: &Path) -> Result<Self, String> {
        let basic = read_archive(&root.join("Data/BasicData.wolf"))?;
        let asset = basic.read("CommonEvent.dat").map_err(|e| e.to_string())?;
        let bytes = asset
            .bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&asset.bytes);
        let common =
            wolf_format::parse_common(bytes, &Limits::default()).map_err(|e| e.to_string())?;
        let event = common
            .value
            .events
            .iter()
            .find(|e| e.id == 26)
            .ok_or("probe common 26 missing")?;
        let raw = event
            .commands
            .get(201)
            .ok_or("probe common 26 PC201 missing")?;
        if raw.opcode != 123 || raw.args != [1600058, 384, 100] {
            return Err("input probe fixture no longer matches the analysed archive".into());
        }
        let program = wolf_ir::compile(
            "input-probe".into(),
            event.name.text().unwrap_or("").into(),
            Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: asset.id.content_blake3.clone(),
                event: 26,
                page: None,
                command: 201,
                span: [0, 0],
            },
            std::slice::from_ref(raw),
        )?;
        let mut coverage = Coverage::default();
        coverage.add(&program);
        let mut vm =
            wolf_vm::runtime::Vm::new(Library::new(vec![program])?, Default::default(), 1, 1)?;
        vm.spawn(Target::Id("input-probe".into()), 0, vec![])?;
        let database = basic.read("SysDatabase.dat").map_err(|e| e.to_string())?;
        let database = wolf_format::parse_database(&database.bytes, &Limits::default())
            .map_err(|e| e.to_string())?;
        let initial = wolf_ir::startup::initial_position(&database.value)?;
        let maps = read_archive(&root.join("Data/MapData.wolf"))?;
        let map_name = initial
            .map_path
            .strip_prefix("MapData/")
            .unwrap()
            .to_owned();
        let map_asset = maps.read(&map_name).map_err(|e| e.to_string())?;
        let map = wolf_format::parse_map(&map_asset.bytes, &Limits::default())
            .map_err(|e| e.to_string())?
            .value;
        println!(
            "INPUT PROBE: isolated Common26 PC201, opcode123 [1600058,384,100], source {}",
            asset.id.content_blake3
        );
        Ok(Self {
            root: root.to_owned(),
            exit_requested: false,
            map_slots: BTreeMap::new(),
            map,
            map_name,
            map_programs: 0,
            coverage,
            vm,
            host: wolf_vm::host::Host::new(4096, 16384)?,
            completions: vec![],
            services: Self::services(root)?,
            control: crate::native_control::Control::new(root),
            titles: crate::native_system::Titles::load(root)?,
            common_slots: Vec::new(),
            scene: None,
            scene_renderer: None,
        })
    }

    pub fn startup(root: &Path) -> Result<Self, String> {
        let archive = read_archive(&root.join("Data/BasicData.wolf"))?;
        let asset = archive.read("Game.dat").map_err(|e| e.to_string())?;
        let config = wolf_format::parse_game(&asset.bytes, &Limits::default())
            .map_err(|e| format!("BasicData/Game.dat: {e}"))?;
        let database = archive.read("SysDatabase.dat").map_err(|e| e.to_string())?;
        let database = wolf_format::parse_database(&database.bytes, &Limits::default())
            .map_err(|e| e.to_string())?;
        let initial = wolf_ir::startup::initial_position(&database.value)?;
        let maps = read_archive(&root.join("Data/MapData.wolf"))?;
        let asset = maps
            .read(initial.map_path.strip_prefix("MapData/").unwrap())
            .map_err(|e| e.to_string())?;
        let map =
            wolf_format::parse_map(&asset.bytes, &Limits::default()).map_err(|e| e.to_string())?;
        let mut active = Vec::new();
        let mut selected_pages = Vec::new();
        for event in &map.value.events {
            let selected = wolf_ir::startup::initial_page(&event.pages, event.id, |n| match n {
                ..=999_999 => Ok(n),
                1_000_000..=1_199_999 | 2_000_000..=2_999_999 => Ok(0),
                _ => Err(format!(
                    "initial page condition reads uninitialized native value {n}"
                )),
            })?;
            println!(
                "STARTUP map {} event {} selected page {:?}",
                initial.map_id, event.id, selected
            );
            selected_pages.push((event.id, selected));
            if let Some(page) = selected {
                if matches!(event.pages[page].conditions[0], 1 | 2) {
                    active.push((
                        format!("{}/{}/{}", initial.map_path, event.id, page),
                        event.id,
                    ));
                }
            }
        }
        let (entry, _) = active
            .first()
            .ok_or("initial map has no automatic/parallel page")?;
        let mut game = Self::load(root, Some(initial.map_id), entry)?;
        for (entry, owner) in active.iter().skip(1) {
            game.vm
                .spawn(Target::Id(entry.clone()), u64::from(*owner), vec![])?;
        }
        let tilesets = archive.read("TileSetData.dat").map_err(|e| e.to_string())?;
        let mut tilesets = wolf_format::parse_tilesets(&tilesets.bytes, &Limits::default())
            .map_err(|e| e.to_string())?;
        let tileset = tilesets
            .get_mut(game.map.tileset as usize)
            .ok_or("initial map tileset missing")?;
        let mut scene = crate::native_scene::Scene::initial(
            &game.map,
            &initial,
            std::mem::take(&mut tileset.tags),
        )?;
        scene.set_game_configuration(&config)?;
        scene.set_passability(std::mem::take(&mut tileset.flags),database.value.types[0].integer(initial.map_id as usize,3).ok_or("initial map loop setting missing")?)?;
        scene.center_camera([0, 0])?;
        for (event_id, selected) in selected_pages {
            let event = game
                .map
                .events
                .iter()
                .find(|event| event.id == event_id)
                .ok_or("selected startup event missing from loaded scene")?;
            scene.set_initial_page(event_id, selected.map(|page| &event.pages[page]))?;
            let fiber = game.vm.state().fibers.values().find(|f| f.owner == u64::from(event_id)).map(|f| f.id);
            game.map_slots.insert(event_id, MapSlot { selected, fiber, pending: false });
        }
        game.scene = Some(scene);
        for request in crate::native_map_audio::requests(&game.vm.state().world.databases, initial.map_id as i32)? {
            game.services.map_audio(request.kind, request.row, request.path, &game.vm.state().world)?;
        }
        // Reserve the compiled entry instances, then apply the same single-main
        // selection used on subsequent frames instead of starting every autorun.
        for slot in game.map_slots.values() {
            if let Some(fiber) = slot.fiber { game.vm.cancel(fiber)?; }
        }
        game.vm.set_native_main_fiber(None)?;
        game.schedule_map_slots()?;
        game.install_common_slots(root)?;
        Ok(game)
    }

    fn install_common_slots(&mut self, root: &Path) -> Result<(), String> {
        let bytes = crate::assets::read_member(root, "BasicData/CommonEvent.dat")?;
        let bytes = bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&bytes);
        let common =
            wolf_format::parse_common(bytes, &Limits::default()).map_err(|e| e.to_string())?;
        let mut events: Vec<_> = common.value.events.iter().collect();
        events.sort_by_key(|e| e.id);
        for event in events {
            let condition =
                wolf_ir::native_startup::CommonCondition::decode(event.unknown1, &event.unknown2)?;
            if !matches!(condition.flags & 15, 2 | 3) {
                continue;
            }
            // Reserve stable slots in common-index order even when currently ineligible.
            // 958939 passes native owner -1, preserved as the unsigned sentinel.
            let fiber =
                self.vm
                    .spawn(Target::Id(format!("common/{}", event.id)), u64::MAX, vec![])?;
            self.vm.cancel(fiber)?;
            self.common_slots.push(CommonSlot {
                common: event.id,
                condition,
                fiber,
            });
        }
        Ok(())
    }

    fn maintain_common_slots(&mut self) -> Result<(), String> {
        for slot in &self.common_slots {
            let eligible = slot
                .condition
                .eligible(slot.common, |n| self.vm.native_number(n))?;
            let wait = &self.vm.state().fibers[&slot.fiber].wait;
            if eligible && *wait == wolf_vm::runtime::Wait::Done {
                self.vm
                    .replace_page(slot.fiber, Target::Id(format!("common/{}", slot.common)))?;
            } else if !eligible && *wait != wolf_vm::runtime::Wait::Done {
                self.vm.cancel(slot.fiber)?;
            }
        }
        Ok(())
    }

    fn maintain_map_slots(&mut self) -> Result<(), String> {
        if self.scene.is_none() { return Ok(()); }
        // Reservations refer to the cached page at execution time. Apply them
        // before reselection, so replacing that page clears its pending flag.
        for id in self.vm.take_native_map_reservations() {
            if let Some(slot) = self.map_slots.get_mut(&id) {
                if slot.selected.is_some() { slot.pending = true; }
            }
        }
        let mut pages = Vec::new();
        for event in &self.map.events {
            let selected = wolf_ir::startup::initial_page(&event.pages, event.id, |n| self.vm.native_number(n))?;
            pages.push((event.id, selected));
        }
        pages.sort_by_key(|&(id, _)| id);
        for &(id, selected) in &pages {
            let slot = self.map_slots.entry(id).or_insert(MapSlot { selected: None, fiber: None, pending: false });
            if selected != slot.selected {
                if let Some(fiber) = slot.fiber.take() { self.vm.cancel(fiber)?; }
                let event = self.map.events.iter().find(|event|event.id==id).unwrap();
                self.scene.as_mut().unwrap().set_page(id, selected.map(|page| &event.pages[page]))?;
                slot.selected = selected;
                slot.pending = false;
            }
        }
        self.schedule_map_slots()
    }

    fn main_active(&self) -> bool {
        self.vm.config().native_main_fiber.is_some_and(|id| self.vm.state().fibers.get(&id)
            .is_some_and(|fiber| !matches!(fiber.wait, wolf_vm::runtime::Wait::Done | wolf_vm::runtime::Wait::Fault)))
    }

    /// A05DB0 consumes pending flags in map entity order. All nonparallel map
    /// pages share one main context, including confirmation and both touch kinds.
    fn schedule_map_slots(&mut self) -> Result<(), String> {
        let reserved_common = self.vm.start_native_reserved_main()?;
        // A05E76 clears input/touch flags when the reserved common takes main.
        if reserved_common {
            for event in &self.map.events {
                if let Some(slot) = self.map_slots.get_mut(&event.id) {
                    if slot.selected.is_some_and(|page| matches!(event.pages[page].conditions[0], 0 | 3 | 4)) {
                        slot.pending = false;
                    }
                }
            }
        }
        let mut main = self.vm.config().native_main_fiber.filter(|_| self.main_active());
        if main.is_none() { main = self.map.events.iter().find_map(|event| {
            let slot = self.map_slots.get(&event.id)?;
            let page = slot.selected?;
            let id = slot.fiber?;
            (event.pages[page].conditions[0] != 2 && self.vm.state().fibers.get(&id).is_some_and(|f| !matches!(f.wait,wolf_vm::runtime::Wait::Done | wolf_vm::runtime::Wait::Fault))).then_some(id)
        }); }
        let pages: Vec<_> = self.map_slots.iter().map(|(&id, slot)| (id, slot.selected)).collect();
        for (id, selected) in pages {
            let Some(page) = selected else { continue; };
            let event = self.map.events.iter().find(|event|event.id==id).unwrap();
            let trigger = event.pages[page].conditions[0];
            let slot = self.map_slots.get_mut(&id).unwrap();
            // 211 may also reserve a parallel page into the shared main context;
            // its independently scheduled parallel instance remains separate.
            if trigger == 2 && slot.pending && main.is_none() && !event.pages[page].commands.is_empty() {
                main = Some(self.vm.spawn(Target::Id(format!("MapData/{}/{id}/{page}", self.map_name)), u64::from(id), vec![])?);
                slot.pending = false;
            }
            let running = slot.fiber.is_some_and(|id| self.vm.state().fibers.get(&id).is_some_and(|f| f.wait != wolf_vm::runtime::Wait::Done));
            if running || event.pages[page].commands.is_empty() { continue; }
            if trigger != 2 && (main.is_some() || !(trigger == 1 || slot.pending)) { continue; }
            let target = Target::Id(format!("MapData/{}/{id}/{page}", self.map_name));
            let fiber = match slot.fiber {
                Some(fiber) => { self.vm.replace_page(fiber,target)?; fiber },
                None => self.vm.spawn(target,u64::from(id),vec![])?,
            };
            slot.fiber = Some(fiber);
            slot.pending = false;
            if trigger != 2 { main = Some(fiber); }
        }
        self.vm.set_native_main_fiber(main)?;
        Ok(())
    }

    fn collect_map_triggers(&mut self, confirm: bool, landed: bool, blocked_direction: Option<i32>) -> Result<(), String> {
        if self.main_active() { return Ok(()); }
        let Some(scene) = &self.scene else { return Ok(()); };
        let pages: Vec<_> = self.map.events.iter().filter_map(|event| {
            let slot = self.map_slots.get(&event.id)?;
            let page = slot.selected?;
            let running = slot.fiber.is_some_and(|id| self.vm.state().fibers.get(&id)
                .is_some_and(|f| !matches!(f.wait, wolf_vm::runtime::Wait::Done)));
            (!running && !event.pages[page].commands.is_empty()).then_some((event.id, &event.pages[page]))
        }).collect();
        for id in scene.map_trigger_candidates(&pages, confirm, landed, blocked_direction)? {
            if let Some(slot) = self.map_slots.get_mut(&id) { slot.pending = true; }
        }
        Ok(())
    }

    fn services(root: &Path) -> Result<crate::native_services::Services, String> {
        let bytes = crate::assets::read_member(root, "BasicData/Game.dat")?;
        let config =
            wolf_format::parse_game(&bytes, &Limits::default()).map_err(|e| e.to_string())?;
        crate::native_media::validate_timing_configuration(&config.configuration)?;
        let ini = crate::window_config::read_ini(&root.join("Game.ini"))?;
        let config =
            crate::window_config::WindowConfig::from_words_and_ini(&config.unknown_words, &ini)?;
        crate::native_services::Services::new(root, config.logical_width, config.logical_height)
    }

    /// Isolated original Common23 picture block. This is a media test, not full startup.
    pub fn media_probe(root: &Path) -> Result<Self, String> {
        let archive = read_archive(&root.join("Data/BasicData.wolf"))?;
        let asset = archive.read("CommonEvent.dat").map_err(|e| e.to_string())?;
        let bytes = asset
            .bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&asset.bytes);
        let commons =
            wolf_format::parse_common(bytes, &Limits::default()).map_err(|e| e.to_string())?;
        let event = commons
            .value
            .events
            .iter()
            .find(|e| e.id == 23)
            .ok_or("media probe common missing")?;
        let commands = event
            .commands
            .get(45..56)
            .ok_or("media probe block missing")?;
        if commands.iter().map(|c| c.opcode).collect::<Vec<_>>()
            != [121, 121, 150, 121, 140, 150, 180, 150, 180, 150, 180]
        {
            return Err("media probe source sequence changed".into());
        }
        let source = Source {
            asset: "BasicData/CommonEvent.dat".into(),
            content_blake3: asset.id.content_blake3.clone(),
            event: 23,
            page: None,
            command: 45,
            span: [0, 0],
        };
        let program = wolf_ir::compile(
            "media-probe".into(),
            "original Common23 PC45..55".into(),
            source,
            commands,
        )?;
        let mut vm =
            wolf_vm::runtime::Vm::new(Library::new(vec![program])?, Default::default(), 1, 1)?;
        vm.spawn(Target::Id("media-probe".into()), 1, vec![])?;
        let mut game = Self::load(root, None, "common/0")?;
        game.vm = vm;
        println!("MEDIA PROBE: original Common23 PC45..55; config, save queries and key guide are outside this isolated test");
        Ok(game)
    }
    pub fn render(&mut self) -> Result<wolf_media::Image, String> {
        self.refresh_scene_image()?;
        let mut result = self.services.render()?;
        if let Some(input) = &mut self.control.text_input {
            input.render(&mut result, &self.services.input_font()?)?;
        }
        Ok(result)
    }

    fn refresh_scene_image(&mut self) -> Result<(), String> {
        let Some(scene) = &self.scene else { return Ok(()); };
        if self.scene_renderer.is_none() {
            self.scene_renderer = Some(crate::native_scene_render::SceneRenderer::new(&mut self.services)?);
        }
        let player = self.control.characters.get(&1).map(|v|v.0.as_bytes()).unwrap_or_default();
        let image = self.scene_renderer.as_ref().unwrap().render(scene,self.map.tileset,player,self.vm.state().tick,&mut self.services)?;
        self.services.set_scene_image(image)
    }

    pub fn load(root: &Path, requested_map: Option<u32>, program: &str) -> Result<Self, String> {
        let map_archive = read_archive(&root.join("Data/MapData.wolf"))?;
        let basic_archive = read_archive(&root.join("Data/BasicData.wolf"))?;
        let limits = Limits::default();
        let mut programs = Vec::<Program>::new();
        let mut coverage = Coverage::default();
        let mut maps = Vec::<(String, Map)>::new();

        for entry in map_archive
            .entries()
            .filter(|e| e.path.to_ascii_lowercase().ends_with(".mps"))
        {
            let name = entry.path.clone();
            let asset = map_archive.read(&name).map_err(|e| e.to_string())?;
            let parsed = wolf_format::parse_map(&asset.bytes, &limits)
                .map_err(|e| format!("{name}: {e}"))?;
            if !parsed.value.events.is_empty() {
                for event in &parsed.value.events {
                    for (page, data) in event.pages.iter().enumerate() {
                        let program_id = format!("MapData/{name}/{}/{}", event.id, page);
                        let source = Source {
                            asset: format!("MapData/{name}"),
                            content_blake3: asset.id.content_blake3.clone(),
                            event: event.id,
                            page: Some(page as u32),
                            command: 0,
                            span: [0, 0],
                        };
                        let program = match wolf_ir::compile(
                            program_id,
                            event.name.text().unwrap_or("").to_owned(),
                            source.clone(),
                            &data.commands,
                        ) {
                            Ok(p) => p,
                            Err(error) => { eprintln!("IR compilation failed for {name} event {} page {page}: {error}",event.id); fallback_program(
                                &name,
                                event.id,
                                Some(page as u32),
                                event.name.text().unwrap_or(""),
                                &asset.id.content_blake3,
                                &data.commands,
                            )? },
                        };
                        coverage.add(&program);
                        programs.push(program);
                    }
                }
            }
            maps.push((name, parsed.value));
        }

        let common_asset = basic_archive
            .read("CommonEvent.dat")
            .map_err(|e| e.to_string())?;
        let common_bytes = common_asset
            .bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&common_asset.bytes);
        let common = wolf_format::parse_common(common_bytes, &limits).map_err(|e| e.to_string())?;
        for event in &common.value.events {
            let id = format!("common/{}", event.id);
            let source = Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: common_asset.id.content_blake3.clone(),
                event: event.id,
                page: None,
                command: 0,
                span: [0, 0],
            };
            let program = match wolf_ir::compile(
                id,
                event.name.text().unwrap_or("").to_owned(),
                source,
                &event.commands,
            ) {
                Ok(p) => p,
                Err(error) => { eprintln!("IR compilation failed for common {}: {error}",event.id); fallback_program(
                    "CommonEvent.dat",
                    event.id,
                    None,
                    event.name.text().unwrap_or(""),
                    &common_asset.id.content_blake3,
                    &event.commands,
                )? },
            };
            coverage.add(&program);
            programs.push(program);
        }

        let database = basic_archive
            .read("SysDatabase.dat")
            .map_err(|e| e.to_string())?;
        let database =
            wolf_format::parse_database(&database.bytes, &limits).map_err(|e| e.to_string())?;
        let map_id = match requested_map {
            Some(id) => id,
            None => wolf_ir::startup::initial_position(&database.value)?.map_id,
        };
        let path = wolf_ir::startup::map_path(&database.value, map_id)?;
        let member = path.strip_prefix("MapData/").unwrap();
        let (map_name, map) = maps
            .into_iter()
            .find(|(name, _)| name.replace('\\', "/") == member)
            .ok_or_else(|| format!("requested map {map_id} is not present"))?;
        let library = Library::new(programs).map_err(|e| e.to_string())?;
        let map_programs = map
            .events
            .iter()
            .flat_map(|e| e.pages.iter())
            .filter(|p| !p.commands.is_empty())
            .count();
        let owner = library
            .get(program)
            .and_then(|p| p.code.first())
            .filter(|i| i.source.page.is_some())
            .map_or(1, |i| u64::from(i.source.event));
        let mut vm = wolf_vm::runtime::Vm::new(
            library,
            wolf_vm::runtime::Config {
                run_until_yield: true,
                native_main_fiber: Some(1),
                native_common_returns: common.value.events.iter().map(|event| (event.id,event.metadata.tail.as_ref().map_or(-1,|(_,slot)|*slot as i32))).collect(),
                ..Default::default()
            },
            1,
            1,
        )
        .map_err(|e| e.to_string())?;
        let mut databases = wolf_vm::native_values::NativeDatabases::default();
        use wolf_vm::native_values::DatabaseKind;
        for (kind, name) in [
            (DatabaseKind::Mutable, "CDataBase"),
            (DatabaseKind::User, "DataBase"),
            (DatabaseKind::System, "SysDatabase"),
        ] {
            let asset = basic_archive
                .read(&format!("{name}.dat"))
                .map_err(|e| e.to_string())?;
            let parsed =
                wolf_format::parse_database(&asset.bytes, &limits).map_err(|e| e.to_string())?;
            databases.insert(kind, &parsed.value)?;
            let project = basic_archive
                .read(&format!("{name}.project"))
                .map_err(|e| e.to_string())?;
            databases.insert_project(kind, &project.bytes)?;
        }
        vm.initialize_databases(databases)?;
        let titles = crate::native_system::Titles::load(root)?;
        titles.initialize(&mut vm)?;
        vm.world_mut().switch_map(map_id as i32)?;
        // Diagnostic entry is supplied explicitly. Never infer event-page
        // eligibility or original startup order from page order.
        vm.spawn(Target::Id(program.into()), owner, vec![])?;
        let _ = basic_archive.read("Game.dat").map_err(|e| e.to_string())?;
        Ok(Self {
            root: root.to_owned(),
            exit_requested: false,
            map_slots: BTreeMap::new(),
            map,
            map_name,
            map_programs,
            coverage,
            vm,
            host: wolf_vm::host::Host::new(4096, 16384).map_err(|e| e.to_string())?,
            completions: vec![],
            services: Self::services(root)?,
            control: crate::native_control::Control::new(root),
            titles,
            common_slots: Vec::new(),
            scene: None,
            scene_renderer: None,
        })
    }

    pub fn tick(&mut self) -> Result<(), String> {
        if self.text_input_active() {
            self.vm.set_keyboard(wolf_vm::input::KeyboardSample { blocked: true, ..Default::default() });
        }
        if self.exit_requested { return Ok(()); }
        let poll = self
            .host
            .poll(&self.vm, &[], |_| {
                Err("player asset service is not connected to VM asset requests".into())
            })
            .map_err(|e| e.to_string())?;
        self.completions.extend(poll.completions);
        if self.services.vm_blocked() {
            self.services.tick()?;
            return Ok(());
        }
        // A00DC0 precedes main A06570 and the ascending 9588D0 slot pass.
        self.maintain_map_slots()?;
        self.maintain_common_slots()?;
        let main_active = self.main_active();
        let mut landed = false;
        let mut confirm = false;
        let mut blocked_direction = None;
        if let Some(scene) = &mut self.scene {
            let policy = self.vm.input_policy();
            let logical = self.vm.state().keyboard.sample.logical;
            let direction = scene.player_input_direction(logical.directions);
            let ready = scene.player_control_ready();
            let before = scene.position();
            confirm = !main_active && ready && policy.player & 1 != 0
                && logical.buttons & !self.vm.state().keyboard.previous_logical.buttons & 1 != 0;
            let direction_mask = match direction {
                1 => 0x20 | 0x10,
                2 => 0x10,
                3 => 0x40 | 0x10,
                4 => 0x20,
                6 => 0x40,
                7 => 0x20 | 0x80,
                8 => 0x80,
                9 => 0x40 | 0x80,
                _ => 0,
            };
            if !main_active && ready && direction_mask != 0 && policy.player & direction_mask == direction_mask {
                scene.apply_player_input(direction)?;
                if scene.position().0 == before.0 && scene.position().1 == before.1 {
                    blocked_direction = Some(direction);
                }
            }
            let offsets = [131, 132].map(|key| self.vm.state().world.systems.numbers.get(&key).copied().unwrap_or(0));
            scene.set_diagonal_percent([129, 130].map(|key| self.vm.state().world.systems.numbers.get(&key).copied().unwrap_or(100)));
            let (world, rng) = self.vm.world_and_native_rng_mut();
            scene.tick_routes(offsets, world, rng)?;
            landed = !ready && scene.player_control_ready();
            confirm &= scene.player_control_ready();
            scene.apply_route_properties(&self.vm.state().world.databases)?;
            for row in scene.take_route_sounds() {
                self.services.route_sound(row, &self.vm.state().world)?;
            }
        }
        self.collect_map_triggers(confirm, landed, blocked_direction)?;
        self.schedule_map_slots()?;
        self.refresh_scene_image()?;
        if let Some(scene) = &self.scene {
            self.services.set_camera_offset(scene.camera_offset());
            let offsets = scene.camera_system_offsets()?;
            self.vm.set_system(104, offsets[0])?;
            self.vm.set_system(105, offsets[1])?;
        }
        let services = &mut self.services;
        let control = &mut self.control;
        let titles = &self.titles;
        let scene = &mut self.scene;
        let mut lifecycle = None;
        self.vm
            .tick_with_host(std::mem::take(&mut self.completions), &mut |ctx| {
                let op = ctx
                    .instruction
                    .raw
                    .as_ref()
                    .ok_or("raw instruction missing")?
                    .opcode;
                if let Some(request) = wolf_vm::native_commands::parse_lifecycle_request(ctx)? {
                    if lifecycle.is_some() { return Ok(false); }
                    match request {
                        wolf_vm::native_commands::LifecycleRequest::MapTransfer(request) => {
                            if !services.map_transition(request.flags)? { return Ok(false); }
                            ctx.yield_once();
                        }
                        wolf_vm::native_commands::LifecycleRequest::Exit => ctx.invalidate_execution(),
                    }
                    lifecycle = Some(request);
                    return Ok(true);
                }
                if titles.execute(ctx)? {
                    return Ok(true);
                }
                if let Some(scene) = scene.as_mut() {
                    if scene.execute(ctx)? {
                        return Ok(true);
                    }
                }
                if matches!(op, 122 | 250) && wolf_vm::native_commands::execute_values(ctx)? {
                    return Ok(true);
                }
                if matches!(op, 106 | 270 | 220 | 221 | 222 | 122) {
                    control.execute(ctx)
                } else {
                    services.execute(ctx)
                }
            })
            .map_err(|e| e.to_string())?;
        // Surface VM faults at the scheduler boundary before lifecycle or
        // restore requests can consume the same tick.  Native map transfer
        // and exit requests are host side effects; they must not hide an
        // already recorded interpreter failure.
        self.check_vm_faults()?;
        if let Some(request) = lifecycle {
            match request {
                wolf_vm::native_commands::LifecycleRequest::Exit => {
                    self.control.flush_snapshots()?;
                    self.exit_requested = true;
                    return Ok(());
                }
                wolf_vm::native_commands::LifecycleRequest::MapTransfer(request) => self.transfer(request)?,
            }
        }
        if let Some(request) = self.control.take_host_restore_request() {
            self.restore_host(request)?;
            return Ok(());
        }
        if let Some(request) = self.control.take_restore_request() {
            self.restore_native(request)?;
            return Ok(());
        }
        self.services.sync_bgm_master(
            *self
                .vm
                .state()
                .world
                .systems
                .numbers
                .get(&100)
                .unwrap_or(&100),
            *self.vm.state().world.systems.numbers.get(&101).unwrap_or(&100),
        )?;
        self.services.tick()?;
        self.check_vm_faults()?;
        if let Some(request) = self.control.take_save_request() {
            let payload = self.capture_host()?;
            self.control.complete_save(request, &self.vm.state().world, payload)?;
        }
        Ok(())
    }

    pub fn exit_requested(&self) -> bool { self.exit_requested }

    fn check_vm_faults(&self) -> Result<(), String> {
        if let Some(f) = self.vm.state().fibers.values().find(|f| f.fault.is_some()) {
            let d = f.fault.as_ref().unwrap();
            let where_at = d
                .source
                .as_ref()
                .map(|s| {
                    format!(
                        "{} event {} page {:?} command {} offset 0x{:x}",
                        s.asset, s.event, s.page, s.command, s.span[0]
                    )
                })
                .unwrap_or_default();
            return Err(format!(
                "VM halted fiber {} tick {}: {} ({where_at})",
                f.id, d.tick, d.message
            ));
        }
        Ok(())
    }

    fn transfer(&mut self, request: wolf_vm::native_commands::MapTransferRequest) -> Result<(), String> {
        let old_scene = self.scene.as_ref().ok_or("transfer requires an active scene")?;
        let old_id = old_scene.map_id();
        let target = request.map_id.unwrap_or(old_id);
        let center = [131, 132].map(|key| self.vm.state().world.systems.numbers.get(&key).copied().unwrap_or(0));
        if target == old_id {
            let scene = self.scene.as_mut().unwrap();
            scene.relocate(request.x_half, request.y_half, request.direction)?;
            return scene.center_camera(center);
        }
        let direction = request.direction.unwrap_or(i32::from(old_scene.position().2));
        let map_id = u32::try_from(target).map_err(|_| "negative target map")?;
        let basic = read_archive(&self.root.join("Data/BasicData.wolf"))?;
        let database = basic.read("SysDatabase.dat").map_err(|e|e.to_string())?;
        let database = wolf_format::parse_database(&database.bytes, &Limits::default()).map_err(|e|e.to_string())?;
        let path = wolf_ir::startup::map_path(&database.value, map_id)?;
        let maps = read_archive(&self.root.join("Data/MapData.wolf"))?;
        let member = path.strip_prefix("MapData/").ok_or("invalid mapped path")?;
        let asset = maps.read(member).map_err(|e|e.to_string())?;
        let map = wolf_format::parse_map(&asset.bytes,&Limits::default()).map_err(|e|e.to_string())?.value;
        let tilesets = basic.read("TileSetData.dat").map_err(|e|e.to_string())?;
        let mut tilesets = wolf_format::parse_tilesets(&tilesets.bytes,&Limits::default()).map_err(|e|e.to_string())?;
        let tags = std::mem::take(&mut tilesets.get_mut(map.tileset as usize).ok_or("target tileset missing")?.tags);
        let flags = std::mem::take(&mut tilesets.get_mut(map.tileset as usize).ok_or("target tileset missing")?.flags);
        let mut scene = crate::native_scene::Scene::initial(&map, &wolf_ir::startup::InitialPosition { map_id,x:0,y:0,map_path:path.clone() },tags)?;
        let config = basic.read("Game.dat").map_err(|e|e.to_string())?;
        scene.set_game_configuration(&wolf_format::parse_game(&config.bytes,&Limits::default()).map_err(|e|e.to_string())?)?;
        scene.set_passability(flags,database.value.types[0].integer(map_id as usize,3).ok_or("target map loop setting missing")?)?;
        scene.relocate(request.x_half,request.y_half,Some(direction))?;
        scene.center_camera(center)?;
        self.vm.world_mut().switch_map(target)?;
        let mut selected = Vec::new();
        for event in &map.events {
            let page = wolf_ir::startup::initial_page(&event.pages,event.id,|n|self.vm.native_number(n))?;
            scene.set_initial_page(event.id,page.map(|i|&event.pages[i]))?;
            selected.push((event.id,page));
        }
        // 9FD1D0 destroys old actor-owned parallel instances. The independent
        // main context survives A416D0 and must execute the transfer's tail.
        let main = self.vm.config().native_main_fiber.filter(|_| self.main_active());
        for slot in self.map_slots.values() {
            if let Some(id) = slot.fiber.filter(|id| Some(*id) != main) { self.vm.cancel(id)?; }
        }
        self.map_slots.clear();
        self.vm.set_native_main_fiber(main)?;
        for (event_id,page) in selected {
            self.map_slots.insert(event_id,MapSlot { selected:page,fiber:None,pending:false });
        }
        self.map_programs=map.events.iter().map(|e|e.pages.len()).sum();
        self.map_name=member.to_owned();
        self.map=map;
        self.scene=Some(scene);
        for request in crate::native_map_audio::requests(&self.vm.state().world.databases, target)? {
            self.services.map_audio(request.kind, request.row, request.path, &self.vm.state().world)?;
        }
        self.collect_map_triggers(false, true, None)?;
        self.maintain_map_slots()?;
        println!("NATIVE MAP {old_id} -> {target}: {path}, position=({},{}), direction={direction}",request.x_half,request.y_half);
        Ok(())
    }

    pub fn acknowledge_messages(&mut self) -> Result<(), String> {
        let actions: Vec<_> = self
            .host
            .requests()
            .filter(|r| matches!(r.service, wolf_ir::Service::Message { .. }))
            .map(|r| wolf_vm::host::Action::Acknowledge(r.token))
            .collect();
        let poll = self
            .host
            .poll(&self.vm, &actions, |_| {
                Err("player asset service is unavailable".into())
            })
            .map_err(|e| e.to_string())?;
        self.completions.extend(poll.completions);
        Ok(())
    }

    pub fn pending_messages(&self) -> impl Iterator<Item = (wolf_vm::runtime::Token, &str)> {
        self.host
            .requests()
            .filter_map(|request| match &request.service {
                wolf_ir::Service::Message { text } => Some((request.token, text.as_str())),
                _ => None,
            })
    }
}

fn fallback_program(
    map: &str,
    event: u32,
    page: Option<u32>,
    name: &str,
    hash: &str,
    commands: &[RawCommand],
) -> Result<Program, String> {
    let id = match page {
        Some(p) => format!("MapData/{map}/{event}/{p}"),
        None => format!("common/{event}"),
    };
    let source = Source {
        asset: if page.is_some() {
            format!("MapData/{map}")
        } else {
            "BasicData/CommonEvent.dat".into()
        },
        content_blake3: hash.into(),
        event,
        page,
        command: 0,
        span: [0, 0],
    };
    let code = commands
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut s = source.clone();
            s.command = i;
            s.span = [c.span.start, c.span.end];
            wolf_ir::Instruction {
                source: s,
                raw: Some(c.clone()),
                op: wolf_ir::Op::Unsupported {
                    opcode: c.opcode,
                    reason: "program control flow failed structural validation".into(),
                },
            }
        })
        .collect();
    Ok(Program {
        id,
        name: name.into(),
        profile: wolf_ir::PROFILE.into(),
        code,
    })
}

#[cfg(test)]
mod local_startup_tests {
    use super::Game;
    use std::path::Path;

    #[test]
    #[ignore = "private game diagnostic: continues after other fibers fault; not full startup acceptance"]
    fn local_main_startup_diagnostic() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap();
        let mut game = Game::startup(root).unwrap();
        for requested_tick in 1..=600 {
            let result = game.tick();
            game.render().expect("headless presentation");
            let main = &game.vm.state().fibers[&1];
            if let Some(fault) = &main.fault {
                if let Some(source) = &fault.source {
                    if source.page.is_none() {
                        let bytes =
                            crate::assets::read_member(root, "BasicData/CommonEvent.dat").unwrap();
                        let bytes = bytes
                            .strip_prefix(
                                b"Extracting data from encrypted files violates the guidelines.\0",
                            )
                            .unwrap_or(&bytes);
                        let parsed = wolf_format::parse_common(bytes, &Default::default()).unwrap();
                        if let Some(event) = parsed
                            .value
                            .events
                            .iter()
                            .find(|event| event.id == source.event)
                        {
                            eprintln!("MAIN RAW {:?}", event.commands.get(source.command));
                        }
                    }
                }
                panic!(
                    "MAIN STARTUP tick={requested_tick} source={:?}: {}",
                    fault.source, fault.message
                );
            }
            if result.is_err() && requested_tick == 1 {
                eprintln!("MAIN-ONLY DIAGNOSTIC continues with other fiber failures: {result:?}");
            }
            if game.vm.state().tick >= 300 {
                break;
            }
        }
        eprintln!(
            "MAIN-ONLY DIAGNOSTIC final main fiber: {:?}",
            game.vm.state().fibers[&1]
        );
    }

    #[test]
    #[ignore = "requires local proprietary game archives; reports the first real startup fault"]
    fn local_default_startup_survives_300_ticks_without_vm_fault() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("game workspace root");
        let mut game = Game::startup(root).unwrap_or_else(|error| {
            eprintln!("DEFAULT STARTUP LOAD FAILED: {error}");
            panic!("default startup load failed: {error}");
        });
        // Transition presentation can temporarily gate the VM. Pump the same CPU
        // render boundary as the GUI and still require all 300 VM ticks to execute.
        for requested_tick in 1..=600 {
            if let Err(error) = game.tick() {
                for fiber in game.vm.state().fibers.values() {
                    if let Some(fault) = &fiber.fault {
                        eprintln!(
                            "STARTUP DIAGNOSTIC fiber={} source={:?}: {}",
                            fiber.id, fault.source, fault.message
                        );
                    }
                }
                eprintln!(
                    "DEFAULT STARTUP FIRST ERROR at requested tick {requested_tick}: {error}"
                );
                panic!("default startup execution failed: {error}");
            }
            game.render().expect("headless presentation");
            if game.vm.state().tick >= 300 {
                break;
            }
        }
        assert_eq!(game.vm.state().tick, 300);
        eprintln!(
            "DEFAULT STARTUP main frames at tick300: {:?}",
            game.vm.state().fibers[&1].frames
        );
        eprintln!("DEFAULT STARTUP survived 300 ticks without a VM fault; pending messages: {}. This is a bounded execution check, not full game completion.",game.pending_messages().count());
    }

    #[test]
    #[ignore = "private game followthrough acceptance; fails on any actual fiber fault"]
    fn local_default_confirm_reaches_title_and_survives_900_more_ticks_without_vm_fault() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap();
        let mut game = Game::startup(root).unwrap();
        let mut keyboard = crate::keyboard::Keyboard::default();
        let mut saw_confirm_pulse = false;
        let mut left_guide_wait = false;
        let mut selected = Vec::new();
        for requested_tick in 1..=1500 {
            // Inject a physical Enter through the ordinary platform translation;
            // Common22 must still produce V3/V5 and Common23 must observe them.
            let before = game.vm.state().tick;
            keyboard.key(
                winit::keyboard::KeyCode::Enter,
                (300..303).contains(&before),
            );
            keyboard.key(
                winit::keyboard::KeyCode::ArrowDown,
                (600..603).contains(&before),
            );
            keyboard.key(
                winit::keyboard::KeyCode::ArrowUp,
                (700..703).contains(&before),
            );
            let sample = keyboard.sample_with_pad_buttons(
                game.vm.input_policy(),
                game.vm.input_bindings(),
                0,
            );
            game.vm.set_keyboard(sample);
            if let Err(error) = game.tick() {
                for fiber in game.vm.state().fibers.values() {
                    if let Some(fault) = &fiber.fault {
                        eprintln!(
                            "CONFIRM DIAGNOSTIC fiber={} source={:?}: {}",
                            fiber.id, fault.source, fault.message
                        );
                    }
                }
                panic!("default confirm failed at request {requested_tick}: {error}");
            }
            if before >= 300 {
                let globals = &game.vm.state().world.globals.numbers;
                if globals.get(&3) == Some(&10) && globals.get(&5) == Some(&1) && !saw_confirm_pulse
                {
                    saw_confirm_pulse = true;
                    eprintln!(
                        "CONFIRM script produced V3=10,V5=1 at VM tick {}",
                        game.vm.state().tick
                    );
                }
                if game.vm.state().fibers[&1]
                    .frames
                    .iter()
                    .any(|frame| frame.program == "common/23" && frame.pc > 75)
                    && !left_guide_wait
                {
                    left_guide_wait = true;
                    eprintln!(
                        "CONFIRM main advanced beyond guide wait at VM tick {}",
                        game.vm.state().tick
                    );
                }
            }
            game.render().expect("headless presentation");
            if [550, 650, 750].contains(&game.vm.state().tick) {
                selected.push(
                    *game
                        .vm
                        .state()
                        .world
                        .commons
                        .numbers
                        .get(&2322)
                        .expect("Common23 selected menu index"),
                );
            }
            if game.vm.state().tick >= 1200 {
                break;
            }
        }
        assert_eq!(game.vm.state().tick, 1200);
        assert!(
            saw_confirm_pulse && left_guide_wait,
            "physical Enter did not advance the script confirmation chain"
        );
        eprintln!(
            "DEFAULT CONFIRM main frames: {:?}",
            game.vm.state().fibers[&1].frames
        );
        let pictures = game.services.active_picture_paths();
        eprintln!("TITLE active pictures: {pictures:?}");
        assert!(pictures
            .iter()
            .any(|(_, path)| path == "Picture/title_loop.mp4"));
        assert_eq!(
            pictures
                .iter()
                .filter(|(_, path)| path.contains("Picture/title_m"))
                .count(),
            5
        );
        assert!(game.vm.state().fibers[&1]
            .frames
            .iter()
            .any(|frame| frame.program == "common/23" && (262..=269).contains(&frame.pc)));
        assert_eq!(game.pending_messages().count(), 0);
        eprintln!("TITLE navigation selection indices: {selected:?}");
        assert_eq!(selected.len(), 3);
        assert_ne!(selected[0], selected[1]);
        assert_eq!(selected[0], selected[2]);
    }
}
