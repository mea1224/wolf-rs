//! Initial actor properties from A0AE50/A14C10, 9EC820 and SysDatabase startup.
//! Movement/teleport commands must update this state before they can be enabled.
use std::collections::BTreeMap;
use wolf_vm::runtime::NativeContext;
mod route;
mod route_host;
mod trigger;
mod lifecycle;
mod tile_commands;
use route::{Geometry, RouteState};

#[derive(Debug, Clone, Copy)]
pub struct SceneActorView<'a> {
    pub id: i32,
    pub position: [i32; 2],
    pub direction: i16,
    pub frame: i16,
    pub opacity: u16,
    pub blend: u16,
    pub tile_id: i32,
    pub moving: bool,
    pub animated: bool,
    pub render_quarters: [i32; 2],
    pub jump_offset_pixels: i32,
    pub graphic: &'a [u8],
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Actor {
    #[serde(default)]
    erase: Option<lifecycle::Erasure>,
    route: RouteState,
    x2: i32,
    y2: i32,
    direction: i16,
    frame: i16,
    opacity: u16,
    blend: u16,
    tile_id: i32,
    initial_page_ready: bool,
    active_page: bool,
    graphic: Vec<u8>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Scene {
    player_route: RouteState,
    map_id: i32,
    native_version: Option<i16>,
    width: i32,
    height: i32,
    player_x2: i32,
    player_y2: i32,
    direction: i16,
    player_frame: i16,
    player_opacity: u16,
    player_blend: u16,
    player_tile_id: i32,
    layers: u32,
    camera_lock_mask: u8,
    camera_quarters: [i32; 2],
    tile_pixels: Option<i32>,
    viewport: [i32; 2],
    passability: Option<Vec<i32>>,
    loop_mode: u8,
    collision_mode: u8,
    configured_step_mode: u8,
    player_directions: u8,
    collision_layer_limit: i8,
    tiles: Option<Vec<i32>>,
    tags: Vec<u8>,
    events: BTreeMap<i32, Actor>,
}

impl Scene {
    pub fn map_id(&self) -> i32 {
        self.map_id
    }

    pub fn loop_axes(&self) -> [bool; 2] {
        [self.loop_mode & 1 != 0, self.loop_mode & 2 != 0]
    }

    pub fn position(&self) -> (i32, i32, i16) {
        (self.player_x2, self.player_y2, self.direction)
    }

    pub fn player_view<'a>(&'a self, graphic: &'a [u8]) -> SceneActorView<'a> {
        SceneActorView {
            id: -2,
            position: [self.player_x2, self.player_y2],
            direction: self.direction,
            frame: self.player_frame,
            opacity: self.player_route.opacity.unwrap_or(self.player_opacity),
            blend: self.player_blend,
            tile_id: self.player_tile_id,
            moving: self.player_route.moving([self.player_x2, self.player_y2], self.tile_pixels.unwrap_or(0)),
            animated: self.player_route.animated([self.player_x2, self.player_y2], self.tile_pixels.unwrap_or(0)),
            render_quarters: self.player_route.render_quarters([self.player_x2, self.player_y2], self.tile_pixels.unwrap_or(0)),
            jump_offset_pixels: self.tile_pixels.map_or(0, |tile| self.player_route.jump_offset_pixels(tile)),
            graphic: self.player_route.graphic.as_deref().unwrap_or(graphic),
        }
    }

    pub fn relocate(&mut self, x2: i32, y2: i32, direction: Option<i32>) -> Result<(), String> {
        self.player_x2 = x2.rem_euclid(self.width * 2);
        self.player_y2 = y2.rem_euclid(self.height * 2);
        self.player_route.relocate();
        if let Some(direction) = direction {
            self.direction = i16::try_from(direction).map_err(|_| "invalid player direction")?;
        }
        Ok(())
    }

    pub fn initial(
        map: &wolf_format::Map,
        position: &wolf_ir::startup::InitialPosition,
        tags: Vec<u8>,
    ) -> Result<Self, String> {
        if map.width == 0
            || map.height == 0
            || map.width > i32::MAX as u32 / 2
            || map.height > i32::MAX as u32 / 2
        {
            return Err("map dimensions exceed native half-coordinate range".into());
        }
        Ok(Self {
            player_route: RouteState::default(),
            map_id: i32::try_from(position.map_id)
                .map_err(|_| "map ID exceeds native signed range")?,
            native_version: None,
            width: i32::try_from(map.width).map_err(|_| "map width exceeds native signed range")?,
            height: i32::try_from(map.height)
                .map_err(|_| "map height exceeds native signed range")?,
            player_x2: position
                .x
                .checked_mul(2)
                .ok_or("initial player X overflow")?,
            player_y2: position
                .y
                .checked_mul(2)
                .ok_or("initial player Y overflow")?,
            // 9ECA9D loads 2; 9ECAF3 stores it to actor+4C.
            direction: 2,
            player_frame: 1,
            player_opacity: 255,
            player_blend: 0,
            player_tile_id: -1,
            layers: map.layers,
            camera_lock_mask: 0,
            camera_quarters: [0; 2],
            tile_pixels: None,
            viewport: [0; 2],
            passability: None,
            loop_mode: 0,
            collision_mode: 0,
            configured_step_mode: 0,
            player_directions: 4,
            collision_layer_limit: 0,
            tiles: map.tiles.clone(),
            tags,
            events: map
                .events
                .iter()
                .map(|event| {
                    let id = i32::try_from(event.id)
                        .map_err(|_| "event ID exceeds native signed range")?;
                    let x = i32::try_from(event.x)
                        .map_err(|_| "event X exceeds native signed range")?;
                    let y = i32::try_from(event.y)
                        .map_err(|_| "event Y exceeds native signed range")?;
                    // 9F88C1..9F8916 calls 9F0170 with coordinate mode zero.
                    // 9F0446..9F04A8 wraps before multiplying by two.
                    Ok((
                        id,
                        Actor {
                            erase: None,
                            route: RouteState::default(),
                            // 9F8932..896B's preliminary constructor selection
                            // uses page0 for every nonempty page vector. The later
                            // reverse eligibility pass is separate.
                            x2: if event.pages.first().is_some_and(|p| p.flags & 0x80 != 0) {
                                x.wrapping_mul(2).wrapping_sub(1)
                            } else {
                                (x % (map.width as i32 * 2)).wrapping_mul(2)
                            },
                            y2: if event.pages.first().is_some_and(|p| p.flags & 0x40 != 0) {
                                y.wrapping_mul(2).wrapping_sub(1)
                            } else {
                                (y % (map.height as i32 * 2)).wrapping_mul(2)
                            },
                            direction: 2,
                            frame: 1,
                            opacity: 255,
                            blend: 0,
                            tile_id: -1,
                            initial_page_ready: false,
                            active_page: false,
                            graphic: Vec::new(),
                        },
                    ))
                })
                .collect::<Result<_, String>>()?,
        })
    }

    pub fn set_game_configuration(
        &mut self,
        config: &wolf_format::GameConfig,
    ) -> Result<(), String> {
        // 45976A/45976D bind Game.dat word15 to FA52FC.
        let bytes = config
            .unknown_words
            .get(30..32)
            .ok_or("Game.dat version word missing")?;
        self.native_version = Some(i16::from_le_bytes(bytes.try_into().unwrap()));
        let tile = i32::from(
            *config
                .configuration
                .first()
                .ok_or("Game.dat tile size missing")?,
        );
        if !matches!(tile, 16 | 32 | 40 | 48) {
            return Err(format!("unsupported native tile size {tile}"));
        }
        self.tile_pixels = Some(tile);
        self.player_directions = *config.configuration.get(1).ok_or("Game.dat player directions missing")?;
        self.configured_step_mode = *config.configuration.get(8).ok_or("Game.dat step mode missing")?;
        let step = 1 + i32::from(self.configured_step_mode == 1);
        self.player_route.set_step_width(step);
        self.player_route.configure(config, true)?;
        for actor in self.events.values_mut() {
            actor.route.set_step_width(step);
            actor.route.configure(config, false)?;
        }
        // 9ECAF7..9ECB08: five-frame sheets start at frame 2, others at 1.
        self.player_frame = if config.configuration.get(7) == Some(&5) {
            2
        } else {
            1
        };
        self.player_route.set_standing_frame(self.player_frame);
        for (axis, word) in [16, 17].into_iter().enumerate() {
            let bytes = config
                .unknown_words
                .get(word * 2..word * 2 + 2)
                .ok_or("Game.dat logical viewport missing")?;
            self.viewport[axis] = i32::from(u16::from_le_bytes(bytes.try_into().unwrap()));
        }
        self.collision_mode = *config
            .configuration
            .get(9)
            .ok_or("Game.dat collision mode missing")?;
        self.collision_layer_limit = *config
            .configuration
            .get(36)
            .ok_or("Game.dat collision layers missing")? as i8;
        Ok(())
    }

    pub fn set_passability(&mut self, flags: Vec<i32>, loop_mode: i32) -> Result<(), String> {
        if !(0..=3).contains(&loop_mode) {
            return Err("invalid native map loop mode".into());
        }
        self.passability = Some(flags);
        self.loop_mode = loop_mode as u8;
        Ok(())
    }

    /// Initialize query-visible page properties. Execution cancellation is owned by Game.
    pub fn set_initial_page(
        &mut self,
        event_id: u32,
        page: Option<&wolf_format::Page>,
    ) -> Result<(), String> {
        self.set_page(event_id, page)
    }

    /// 9F8EB3..9F8F61 preserves direction only for an already selected page
    /// when the new page's fourth feature byte requests retention.
    pub fn set_page(
        &mut self,
        event_id: u32,
        page: Option<&wolf_format::Page>,
    ) -> Result<(), String> {
        let id = i32::try_from(event_id).map_err(|_| "event ID exceeds native signed range")?;
        let actor = self
            .events
            .get_mut(&id)
            .ok_or("initial page references absent event")?;
        if let Some(page) = page {
            let direction = *page
                .graphic_fields
                .first()
                .ok_or("page direction missing")?;
            let retain = actor.active_page && page.page_transfer.is_some_and(|flag| flag != 0);
            if !retain {
                actor.direction = i16::from(direction);
                // 9F8F13..9F8F22; retained page properties preserve both fields.
                actor.frame = i16::from(*page.graphic_fields.get(1).ok_or("page frame missing")?);
            }
            actor.graphic = page.graphic.raw.clone();
            // Page's first signed word is copied to actor+44 at 9F8DB9.
            actor.tile_id = page.unknown as i32;
            // 9F8E08/9F8E10: unlike direction/frame, page retention does not gate these.
            actor.opacity = u16::from(*page.graphic_fields.get(2).ok_or("page opacity missing")?);
            actor.blend = u16::from(*page.graphic_fields.get(3).ok_or("page blend missing")?);
            actor.route.page(page.flags, &page.movement, actor.frame, page.route_flags, &page.routes);
            // 9F8EA7: page activation resets route-modified step width.
            actor.route.set_step_width(1 + i32::from(self.configured_step_mode == 1));
        } else {
            // Negative-page branch clears graphics but never writes actor+4C.
            actor.graphic.clear();
            actor.tile_id = -1;
        }
        actor.active_page = page.is_some();
        actor.erase = None;
        actor.initial_page_ready = true;
        Ok(())
    }

    pub fn restore_snapshot(
        &mut self,
        snapshot: &crate::native_save_payload::SaveSnapshot,
    ) -> Result<(), String> {
        if snapshot.map_id != self.map_id
            || snapshot.width != self.width as usize
            || snapshot.height != self.height as usize
            || snapshot.layers != self.layers as usize
        {
            return Err("native scene snapshot map identity/dimensions mismatch".into());
        }
        let expected = snapshot
            .width
            .checked_mul(snapshot.height)
            .and_then(|n| n.checked_mul(snapshot.layers))
            .ok_or("native scene tile count overflow")?;
        if snapshot
            .tiles
            .as_ref()
            .is_some_and(|tiles| tiles.len() != expected)
        {
            return Err("native scene snapshot tile count mismatch".into());
        }
        let mut events = self.events.clone();
        let mut seen = std::collections::BTreeSet::new();
        for saved in &snapshot.map_characters {
            if !seen.insert(saved.id) {
                return Err("duplicate native scene saved event".into());
            }
            let event = events
                .get_mut(&saved.id)
                .ok_or("native scene saved event is absent from map")?;
            event.x2 = saved.x2;
            event.y2 = saved.y2;
            event.direction = saved.direction;
            event.frame = saved.frame;
            event.opacity = saved.opacity;
            event.blend = saved.blend;
            event.tile_id = saved.tile_id;
            event.graphic = saved.graphic.clone();
            event.route.restore(saved)?;
            if let Some(inactive) = saved.fields.get(&0xBA) { event.active_page = *inactive == 0; }
            event.initial_page_ready = true;
        }
        let mut player_route = self.player_route.clone();
        player_route.restore(&snapshot.player)?;
        self.events = events;
        self.player_x2 = snapshot.player.x2;
        self.player_y2 = snapshot.player.y2;
        self.direction = snapshot.player.direction;
        self.player_frame = snapshot.player.frame;
        self.player_opacity = snapshot.player.opacity;
        self.player_blend = snapshot.player.blend;
        self.player_tile_id = snapshot.player.tile_id;
        self.player_route = player_route;
        // 9FBBDB/9FBC1F deserialize map+1AE4/+1AE8, in quarter pixels.
        self.camera_quarters = [snapshot.map_x, snapshot.map_y];
        self.tiles = snapshot.tiles.clone();
        Ok(())
    }
    #[cfg(test)]
    pub fn camera_lock_mask(&self) -> u8 {
        self.camera_lock_mask
    }

    pub fn camera_offset(&self) -> [i32; 2] {
        self.camera_quarters.map(|n| n / 4)
    }

    #[cfg(test)]
    pub fn camera_quarters(&self) -> [i32; 2] {
        self.camera_quarters
    }

    /// 9F7A55/9F7B2E follows rendered movement, with 9FEC00..9FED79 bounds.
    fn follow_player_motion(&mut self, before: [i32; 2], after: [i32; 2], system_offsets: [i32; 2]) -> Result<(), String> {
        let tile = i64::from(self.tile_pixels()?);
        for axis in 0..2 {
            if self.camera_lock_mask & (1 << axis) != 0 { continue; }
            let delta = i64::from(after[axis]) - i64::from(before[axis]);
            if delta == 0 { continue; }
            let camera = i64::from(self.camera_quarters[axis]);
            let dimension = i64::from([self.width, self.height][axis]);
            let period = dimension * tile * 4;
            let loops = self.loop_mode & (1 << axis) != 0;
            let mut relative = i64::from(after[axis]) - camera;
            if loops {
                // The native loop tests separately truncated pixel positions.
                let pixels = i64::from(after[axis] / 4) - camera / 4;
                if pixels < 0 {
                    relative += ((-pixels + period / 4 - 1) / (period / 4)) * period;
                }
            }
            let threshold = 2 * (i64::from(self.viewport[axis]) + 2 * i64::from(system_offsets[axis]) - tile);
            if (delta < 0 && relative < threshold) || (delta > 0 && relative > threshold) {
                let next = if loops {
                    if delta < 0 { (camera + period + delta) % period } else { camera + delta }
                } else if delta < 0 {
                    (camera + delta).max(0)
                } else {
                    (camera + delta).clamp(0, (period - i64::from(self.viewport[axis]) * 4).max(0))
                };
                self.camera_quarters[axis] = i32::try_from(next).map_err(|_| "native following camera coordinate overflow")?;
            }
        }
        Ok(())
    }

    pub fn tile_pixels(&self) -> Result<i32, String> {
        self.tile_pixels
            .ok_or("native scene tile size is not configured".into())
    }

    /// Native system getters104/105 use map-wrapped pixel coordinates.
    pub fn camera_system_offsets(&self) -> Result<[i32; 2], String> {
        let tile = self.tile_pixels()?;
        let version = self
            .native_version
            .ok_or("native scene version is not configured")?;
        let mut result = [0; 2];
        for (axis, dimension) in [self.width, self.height].into_iter().enumerate() {
            let period = dimension
                .checked_mul(tile)
                .ok_or("native map pixel size overflow")?;
            let value = self.camera_quarters[axis] / 4;
            result[axis] = if version >= 366 {
                value.rem_euclid(period)
            } else {
                let mut value = value % period;
                let maximum = (dimension - 10) * tile;
                while value > maximum {
                    value -= period;
                }
                value
            };
        }
        Ok(result)
    }

    /// A4174C / 9F62B0 centers every committed player transfer, including same-map.
    pub fn center_camera(&mut self, system_offsets: [i32; 2]) -> Result<(), String> {
        let tile = self.tile_pixels()?;
        let mut camera = [0; 2];
        for (axis, (position, dimension)) in
            [(self.player_x2, self.width), (self.player_y2, self.height)]
                .into_iter()
                .enumerate()
        {
            let total = i64::from(dimension) * i64::from(tile) * 4;
            let target = 2
                * ((i64::from(position) + 1) * i64::from(tile)
                    - i64::from(self.viewport[axis])
                    - 2 * i64::from(system_offsets[axis]));
            let target = if self.loop_mode & (1 << axis) == 0 {
                target.clamp(0, (total - i64::from(self.viewport[axis]) * 4).max(0))
            } else {
                (target + total) % total
            };
            camera[axis] =
                i32::try_from(target).map_err(|_| "native camera coordinate overflow")?;
        }
        self.camera_quarters = camera;
        Ok(())
    }

    pub fn tiles(&self) -> Option<&[i32]> {
        self.tiles.as_deref()
    }

    pub fn dimensions(&self) -> [u32; 3] {
        [self.width as u32, self.height as u32, self.layers]
    }

    /// Active event graphics and authoritative half-tile position/direction.
    pub fn visible_events(&self) -> impl Iterator<Item = SceneActorView<'_>> {
        self.events
            .iter()
            .filter(|(_, actor)| {
                actor.active_page && (actor.tile_id >= 0 || !actor.route.graphic.as_deref().unwrap_or(&actor.graphic).is_empty())
            })
            .map(|(&id, actor)| SceneActorView {
                id,
                position: [actor.x2, actor.y2],
                direction: actor.direction,
                frame: actor.frame,
                opacity: actor.route.opacity.unwrap_or(actor.opacity),
                blend: actor.blend,
                tile_id: actor.tile_id,
                moving: actor.route.moving([actor.x2, actor.y2], self.tile_pixels.unwrap_or(0)),
                animated: actor.route.animated([actor.x2, actor.y2], self.tile_pixels.unwrap_or(0)),
                render_quarters: actor.route.render_quarters([actor.x2, actor.y2], self.tile_pixels.unwrap_or(0)),
                jump_offset_pixels: self.tile_pixels.map_or(0, |tile| actor.route.jump_offset_pixels(tile)),
                graphic: actor.route.graphic.as_deref().unwrap_or(actor.graphic.as_slice()),
            })
    }

    pub fn execute(&mut self, context: &mut NativeContext<'_>) -> Result<bool, String> {
        let raw = context
            .instruction
            .raw
            .as_ref()
            .ok_or("missing scene raw command")?;
        if raw.opcode == 242 {
            return self.replace_tiles_command(context);
        }
        if matches!(raw.opcode, 201 | 202) {
            if let Some(target) = context.frame.native_route_wait {
                let busy = if target == i32::MIN {
                    self.player_route.forced || self.events.values().any(|a| a.active_page && a.route.forced)
                } else if target == -2 {
                    self.player_route.forced
                } else {
                    self.events.get(&target).is_some_and(|a| a.route.forced)
                };
                if busy { context.retry(); } else { context.frame.native_route_wait = None; }
                return Ok(true);
            }
            if raw.opcode == 202 {
                if !raw.args.is_empty() || raw.movement.is_some() || !raw.strings.is_empty() {
                    return Err("202 route wait has an invalid envelope".into());
                }
                if self.player_route.forced || self.events.values().any(|a| a.active_page && a.route.forced) {
                    context.frame.native_route_wait = Some(i32::MIN);
                    context.retry();
                }
                return Ok(true);
            }
            if raw.args.len() != 1 || !raw.strings.is_empty() {
                return Err("201 route assignment requires one actor selector".into());
            }
            let movement = raw.movement.as_ref().ok_or("201 missing movement route")?;
            let selector = context.number(context.number(raw.args[0])?)?;
            let target = if selector == -1 { i32::try_from(context.owner).map_err(|_| "route owner exceeds native range")? } else { selector };
            let movement = route_host::bind(movement, target, context)?;
            if target == -2 {
                self.player_route.install(&movement)?;
                let (world,rng)=context.world_and_native_rng_mut();
                self.advance_route(target, world, rng, false)?;
            } else if let Some(actor) = self.events.get_mut(&target) {
                actor.route.install(&movement)?;
                if actor.active_page {
                    let (world,rng)=context.world_and_native_rng_mut();
                    self.advance_route(target, world, rng, false)?;
                }
            } else if target < -2 {
                return Err(format!("201 follower selector {target} needs follower scene state"));
            } else {
                return Ok(true); // A1BA0C invalid event selector is an accepted no-op.
            }
            self.apply_route_properties(&context.world.databases)?;
            let busy = if target == -2 { self.player_route.forced } else { self.events.get(&target).is_some_and(|a| a.route.forced) };
            if movement.flags & 4 != 0 && busy {
                context.frame.native_route_wait = Some(target);
                context.retry();
            }
            return Ok(true);
        }
        if raw.opcode == 281 {
            if raw.args.len() != 3 || !raw.strings.is_empty() || !(2..=5).contains(&raw.args[0]) {
                return Err("281 camera movement requires a scroll interpolation adapter".into());
            }
            // A2BE71/85/99/AA update the X/Y follow-disable bits. These modes
            // never read args1/2. 9F7A55 and 9F7B2E consume the two bits.
            self.camera_lock_mask = match raw.args[0] {
                2 => 3,
                3 => 0,
                4 => self.camera_lock_mask | 1,
                5 => self.camera_lock_mask | 2,
                _ => unreachable!(),
            };
            return Ok(true);
        }
        if raw.opcode != 124
            || !matches!(
                raw.args.get(1),
                Some(flags) if matches!(*flags & !0x40, 0x1000 | 0x3000) || *flags & !0xff0020 == 0x2000
            )
        {
            return Ok(false);
        }
        // A14CD0..A14CE3 selects the category with (flags >> 12) & 15.
        // Bit6 is consumed only by picture queries, not map properties.
        if raw.args[1] & !0x40 == 0x3000 {
            if raw.args.len() != 3 || !raw.strings.is_empty() {
                return Err("124 map query requires three operands and no strings".into());
            }
            let value = match raw.args[2] {
                // A170B4..A170C8 uses the pending map during a transfer.
                0 => self.map_id,
                // A1769B/A176A6 read the current map dimensions, not pixels.
                15 => self.width,
                16 => self.height,
                other => {
                    return Err(format!(
                        "124 scene query selector {other} needs a host adapter"
                    ))
                }
            };
            context.set_number(raw.args[0], value)?;
            return Ok(true);
        }
        if raw.args.len() != 4 || !raw.strings.is_empty() {
            return Err("124 actor query needs four operands and no strings".into());
        }
        if raw.args[1] & 0xf000 == 0x2000 {
            let selector = (raw.args[1] >> 16) & 255;
            let half = raw.args[1] & 0x20 != 0;
            let x = context.number(raw.args[2])?;
            let y = context.number(raw.args[3])?;
            if (5..=10).contains(&selector) {
                let layer = if selector < 8 { selector - 5 } else { selector - 8 } as usize;
                let value = if selector < 8 {
                    let scale = if half { 1 } else { 2 };
                    self.external_tile_number(self.tile_index(layer, x.wrapping_mul(scale), y.wrapping_mul(scale))?)?
                } else if half {
                    self.layer_tag(layer, x, y)? as i32
                } else {
                    // A15CDC..A15EC0 samples both half-grid foot points.
                    let x = x.wrapping_mul(2); let y = y.wrapping_mul(2).wrapping_add(1);
                    self.layer_tag(layer, x, y)?.max(self.layer_tag(layer, x.wrapping_add(1), y)?) as i32
                };
                context.set_number(raw.args[0], value)?;
                return Ok(true);
            }
            if half || !matches!(selector, 0 | 2 | 4) {
                return Err(format!("124 tile selector {selector} half={half} requires a native adapter"));
            }
            let x2 = context.number(raw.args[2])?.wrapping_mul(2);
            let y2 = context.number(raw.args[3])?.wrapping_mul(2);
            let value = if raw.args[1] == 0x2000 {
                i32::from(self.tile_tag(x2, y2)?)
            } else if raw.args[1] == 0x22000 {
                // A159D0..A15A42 queries tiles only, direction0, check mode1.
                let pass = match self.collision_mode {
                    0 => self.passable_query(x2, y2, false)?,
                    1 => {
                        self.passable_query(x2, y2, true)?
                            && self.passable_query(x2, y2.wrapping_sub(1), true)?
                    }
                    mode => return Err(format!("unsupported native collision mode {mode}")),
                };
                i32::from(!pass)
            } else {
                self.top_tile_number(x2, y2)?
            };
            context.set_number(raw.args[0], value)?;
            return Ok(true);
        }
        // Both A14CEF and the A0AE50 helper resolve the selector numerically.
        let actor = context.number(context.number(raw.args[2])?)?;
        let value = self.actor_property(actor, context.owner, raw.args[3])?;
        context.set_number(raw.args[0], value)?;
        Ok(true)
    }

    /// Called at the native actor update boundary, before event contexts.
    pub fn set_diagonal_percent(&mut self, percent: [i32; 2]) {
        self.player_route.set_diagonal_percent(percent);
        for actor in self.events.values_mut() { actor.route.set_diagonal_percent(percent); }
    }

    pub fn tick_routes(&mut self, system_offsets: [i32; 2], world: &mut wolf_vm::runtime::World, rng: &mut u32) -> Result<(), String> {
        let geometry = self.route_geometry()?;
        let before = self.player_route.render_quarters([self.player_x2,self.player_y2], geometry.tile);
        self.advance_route(-2,world,rng,true)?;
        let after = self.player_route.render_quarters([self.player_x2,self.player_y2], geometry.tile);
        self.follow_player_motion(before, after, system_offsets)?;
        let targets: Vec<_> = self.events.iter().filter(|(_,actor)| actor.active_page).map(|(&id,_)|id).collect();
        for target in targets {
            self.advance_route(target,world,rng,true)?;
        }
        Ok(())
    }

    pub fn apply_route_properties(&mut self, database: &wolf_vm::native_values::NativeDatabases) -> Result<(), String> {
        use wolf_vm::native_values::{DatabaseAddress, DatabaseKind, NativeValue};
        for route in std::iter::once(&mut self.player_route).chain(self.events.values_mut().map(|actor| &mut actor.route)) {
            if let Some(row) = route.graphic_row.take() {
                let row = usize::try_from(row).map_err(|_| "negative route character graphic row")?;
                let NativeValue::Text(path) = database.get(DatabaseAddress { kind: DatabaseKind::System, type_id: 8, row, field: 0 })? else { return Err("route character graphic must be text".into()); };
                route.graphic = Some(path.as_bytes().to_vec());
            }
        }
        Ok(())
    }

    fn route_geometry(&self) -> Result<Geometry, String> {
        Ok(Geometry { tile: self.tile_pixels.ok_or("route geometry requires Game.dat tile size")?, size: [self.width, self.height], loops: self.loop_axes() })
    }

    pub fn take_route_sounds(&mut self) -> Vec<i32> {
        let mut sounds = self.player_route.take_sounds();
        for actor in self.events.values_mut() { sounds.extend(actor.route.take_sounds()); }
        sounds
    }

    pub fn player_input_direction(&self, directions: [i32; 8]) -> i32 {
        // 9F7812/9F7923 select the native eight/four-direction input slots.
        match self.player_directions { 8 => directions[1], 4 => directions[0], _ => 0 }
    }

    /// Apply the engine's default player-control direction for this logical
    /// tick. The input value is the normalized WOLF direction (1,2,3,4,6,7,8,9)
    /// produced by the VM input adapter; no key is interpreted here.
    pub fn apply_player_input(&mut self, direction: i32) -> Result<(), String> {
        if !self.player_route.player_control_ready() || !matches!(direction, 1 | 2 | 3 | 4 | 6 | 7 | 8 | 9) {
            return Ok(());
        }
        // 9F9F40 and its directional siblings attempt movement after updating
        // facing. A wall must not prevent turning to face an action event.
        if !self.player_route.direction_fixed {
            self.direction = i16::try_from(direction).map_err(|_| "player direction overflow")?;
        }
        let geometry = self.route_geometry()?;
        let step = self.player_route.step_width();
        let delta = match direction {
            1 => [-step, step], 2 => [0, step], 3 => [step, step], 4 => [-step, 0],
            6 => [step, 0], 7 => [-step, -step], 8 => [0, -step], 9 => [step, -step], _ => unreachable!(),
        };
        let mut target = [self.player_x2.wrapping_add(delta[0]), self.player_y2.wrapping_add(delta[1])];
        if geometry.loops[0] { target[0] = target[0].rem_euclid(self.width * 2); }
        if geometry.loops[1] { target[1] = target[1].rem_euclid(self.height * 2); }
        if !self.player_destination_clear(target)? || !self.player_direction_clear(target, direction)? {
            return Ok(());
        }
        let mut position = [self.player_x2, self.player_y2];
        self.player_route.start_manual_move(i16::try_from(direction).map_err(|_| "player direction overflow")?, &mut position, geometry)?;
        [self.player_x2, self.player_y2] = position;
        Ok(())
    }

    /// Restricted 9FEDE0 path used by opcode124 type2, direction0, check mode1.
    /// This does not implement directional movement or event collision.
    fn passable_query(&self, x2: i32, y2: i32, half: bool) -> Result<bool, String> {
        self.passable_direction(x2, y2, half, 0, 1)
    }

    /// 9FEDE0 receives source/target facing and the actor's step width.
    fn passable_direction(&self, x2: i32, y2: i32, half: bool, direction: i32, step: i32) -> Result<bool, String> {
        let flags = self
            .passability
            .as_ref()
            .ok_or("tile passability table is not bound")?;
        let tiles = self
            .tiles
            .as_ref()
            .ok_or("passability on sentinel map is not implemented")?;
        // 9FED80 checks the full two-point footprint before wrapping samples.
        if self.loop_mode & 1 == 0 && !(0..self.width * 2 - 1).contains(&x2)
            || self.loop_mode & 2 == 0 && !(-i32::from(half)..self.height * 2 - 1).contains(&y2)
        {
            return Ok(false);
        }
        // cmp map_layers,configured; cmovge configured is a minimum, not maximum.
        let layers = (self.layers as i32).min(i32::from(self.collision_layer_limit));
        for dx in 0..2 {
            let sample_x = x2.wrapping_add(dx);
            let sample_y = y2.wrapping_add(1);
            let x = (sample_x.rem_euclid(self.width * 2) / 2) as usize;
            let y = (sample_y.rem_euclid(self.height * 2) / 2) as usize;
            let mut layer = layers - 1;
            while layer >= 0 {
                let property = loop {
                    let word = *tiles
                        .get((layer as usize * self.width as usize + x) * self.height as usize + y)
                        .ok_or("native passability tile buffer mismatch")?;
                    let index = if word < 100_000 {
                        word.wrapping_add(32)
                    } else {
                        word / 100_000 - 1
                    };
                    if index == 0 && layer > 0 {
                        layer -= 1;
                        continue;
                    }
                    if index < 0 {
                        return Ok(false);
                    }
                    let Some(&property) = flags.get(index as usize) else {
                        return Ok(false);
                    };
                    break property;
                };
                if property & 0x20 != 0 {
                    if step == 2 && property & 15 != 0 { return Ok(false); }
                    let blocked = match (sample_x % 2, y2 % 2) {
                        (0, 0) => 8,
                        (0, 1) => 2,
                        (1, 0) => 4,
                        (1, 1) => 1,
                        _ => 0,
                    };
                    if property & blocked != 0 {
                        return Ok(false);
                    }
                } else {
                    let mask = match direction { 2 => 1, 4 => 2, 6 => 4, 8 => 8, _ => 0 };
                    // 9FF0C5..9FF19A distinguishes the two sampled feet and
                    // half-grid boundaries. Source and opposite target checks
                    // are both required by 9EFCB9..9EFD0D.
                    let side_overlap = x2 % 2 == 1 && (direction == 6 && dx == 0 && property & 4 != 0
                        || direction == 4 && dx == 1 && property & 2 != 0);
                    let edge = match direction {
                        4 | 6 => dx == 0 && x2 % 2 == 0,
                        2 => y2 % 2 == 0,
                        8 => y2 % 2 == 1,
                        _ => false,
                    };
                    if property & 15 == 15 || side_overlap || property & mask != 0 && (step == 2 || step == 1 && edge) {
                        return Ok(false);
                    }
                }
                // Star/upper-layer pass-through continues downward; otherwise
                // this sample is accepted. Both samples must avoid a blocker.
                if property & 0x200 == 0 {
                    break;
                }
                layer -= 1;
            }
        }
        Ok(true)
    }

    fn tile_index(&self, layer: usize, x2: i32, y2: i32) -> Result<i32, String> {
        let tiles = self.tiles.as_ref().ok_or("tile query on sentinel map")?;
        if layer >= self.layers as usize { return Err("tile query layer exceeds map layers".into()); }
        let x = (x2.rem_euclid(self.width * 2) / 2) as usize;
        let y = (y2.rem_euclid(self.height * 2) / 2) as usize;
        let word = *tiles.get((layer * self.width as usize + x) * self.height as usize + y).ok_or("tile buffer mismatch")?;
        Ok(if word < 100_000 { word.wrapping_add(32) } else { word / 100_000 - 1 })
    }
    fn layer_tag(&self, layer: usize, x2: i32, y2: i32) -> Result<u8, String> {
        let index = usize::try_from(self.tile_index(layer, x2, y2)?).map_err(|_| "negative tile tag index")?;
        self.tags.get(index).copied().ok_or("tile tag index exceeds tileset".into())
    }
    fn external_tile_number(&self, number: i32) -> Result<i32, String> {
        let version = self.native_version.ok_or("tile numbering requires Game.dat version")?;
        Ok(if version >= 230 { number } else if (16..32).contains(&number) { -1 } else if number >= 32 { number - 16 } else { number })
    }

    fn top_tile_number(&self, x2: i32, y2: i32) -> Result<i32, String> {
        let version = self
            .native_version
            .ok_or("tile numbering requires Game.dat version")?;
        let tiles = self
            .tiles
            .as_ref()
            .ok_or("tile number query on sentinel map is not implemented")?;
        let width = self.width as usize;
        let height = self.height as usize;
        let x = (x2.rem_euclid(self.width * 2) / 2) as usize;
        let y = (y2.rem_euclid(self.height * 2) / 2) as usize;
        let mut number = 0;
        // A15B4B..A15BA2 queries 9FE460 from the highest layer, proceeding
        // downward only while the converted number is zero (not the raw word).
        for layer in (0..self.layers as usize).rev() {
            let word = *tiles
                .get((layer * width + x) * height + y)
                .ok_or("tile coordinate exceeds map buffer")?;
            number = if word < 100_000 {
                word.wrapping_add(32)
            } else {
                word / 100_000 - 1
            };
            if number != 0 {
                break;
            }
        }
        // A2F760(-1): current startup's FA5318 is initialized to -1 at 459AD0.
        // Before version230 only, remove the legacy sixteen-slot gap.
        Ok(if version >= 230 {
            number
        } else if (16..32).contains(&number) {
            -1
        } else if number >= 32 {
            number - 16
        } else {
            number
        })
    }

    fn actor_property(&self, selector: i32, owner: u64, field: i32) -> Result<i32, String> {
        let (x2, y2, direction, route) = match selector {
            -2 => (self.player_x2, self.player_y2, self.direction, &self.player_route),
            -7..=-3 => {
                return Err(format!(
                    "124 party actor selector {selector} is not implemented"
                ))
            }
            _ => {
                // A0AE94 uses the map owner for -1, and A0AEDC resolves other
                // selectors by event ID, not vector position or active page.
                let id = if selector == -1 {
                    i32::try_from(owner).ok()
                } else {
                    Some(selector)
                };
                let Some(actor) = id.and_then(|id| self.events.get(&id)) else {
                    return Ok(-1); // A14D00..A14D05: null actor.
                };
                if !actor.initial_page_ready {
                    return Err("event initial page was not synchronized with Scene".into());
                }
                (actor.x2, actor.y2, actor.direction, &actor.route)
            }
        };
        // A14D8B..A14DD5 use signed division/remainder, not Euclidean modulo.
        Ok(match field {
            0 => self.width.wrapping_add(x2 / 2) % self.width,
            1 => self.height.wrapping_add(y2 / 2) % self.height,
            2 => (2 * self.width).wrapping_add(x2) % (2 * self.width),
            3 => (2 * self.height).wrapping_add(y2) % (2 * self.height),
            4 => route.height_offset(),
            5 => i32::from(direction),
            6 | 7 => self.actor_screen_position([x2, y2], route)?[(field - 6) as usize],
            9 => i32::from(self.tile_tag(x2, y2)?),
            field => return Err(format!("124 actor property {field} is not implemented")),
        })
    }

    fn actor_screen_position(&self, position: [i32; 2], route: &route::RouteState) -> Result<[i32; 2], String> {
        let tile = self.tile_pixels.ok_or("actor screen position requires Game.dat tile size")?;
        let rendered = route.render_quarters(position, tile);
        // 9F04E0 / 9F0590 round positive quarter coordinates upward and
        // negative coordinates toward zero, before adding the actor's foot.
        let mut pixels: [i32; 2] = std::array::from_fn(|axis| {
            let delta = rendered[axis].wrapping_sub(self.camera_quarters[axis]);
            if delta >= 0 { delta.wrapping_add(3) / 4 } else { delta / 4 }
        });
        pixels[0] += tile / 2;
        pixels[1] += tile - route.jump_offset_pixels(tile);
        if self.loop_mode & 1 != 0 {
            let period = self.width * tile;
            if pixels[0] < 0 { pixels[0] = pixels[0].rem_euclid(period); }
            let maximum = self.viewport[0] + ((period - self.viewport[0]) / 2).max(0);
            if pixels[0] > maximum { pixels[0] -= period; }
        }
        if self.loop_mode & 2 != 0 {
            let period = self.height * tile;
            if pixels[1] < 0 { pixels[1] = pixels[1].rem_euclid(period); }
            if pixels[1] > period { pixels[1] -= period; }
            if pixels[1] <= 0 { pixels[1] += period; }
        }
        let offsets = route.pixel_offsets();
        Ok([pixels[0] + offsets[0], pixels[1] + offsets[1]])
    }

    fn tile_tag(&self, x2: i32, y2: i32) -> Result<u8, String> {
        let Some(tiles) = &self.tiles else {
            return Err("tile tag query on sentinel map has no verified adapter".into());
        };
        let width = self.width as usize;
        let height = self.height as usize;
        let y = (y2.wrapping_add(1).rem_euclid(self.height * 2) / 2) as usize;
        let xs = [x2, x2.wrapping_add(1)].map(|x| (x.rem_euclid(self.width * 2) / 2) as usize);
        for layer in (0..self.layers as usize).rev() {
            let mut result = 0;
            for x in xs {
                let word = *tiles
                    .get((layer * width + x) * height + y)
                    .ok_or("tile coordinate exceeds map buffer")?;
                let index = if word < 100_000 {
                    word.checked_add(32)
                } else {
                    Some(word / 100_000 - 1)
                }
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("invalid tile tag index")?;
                result = result.max(
                    *self
                        .tags
                        .get(index)
                        .ok_or("tile tag index exceeds tileset")?,
                );
            }
            if result != 0 {
                return Ok(result);
            }
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passability_checks_tile_layers_quadrants_bounds_and_loops() {
        let mut scene = Scene {
            player_route: RouteState::default(),
            map_id: 2,
            native_version: Some(371),
            width: 2,
            height: 2,
            player_x2: 0,
            player_y2: 0,
            direction: 2,
            layers: 2,
            player_frame: 1,
            player_opacity: 255,
            player_blend: 0,
            player_tile_id: -1,
            camera_lock_mask: 0,
            camera_quarters: [0; 2],
            tile_pixels: None,
            viewport: [0; 2],
            passability: None,
            loop_mode: 0,
            collision_mode: 0,
            configured_step_mode: 0,
            collision_layer_limit: 2,
            player_directions: 4,
            tiles: Some(vec![300_000; 8]),
            tags: vec![],
            events: BTreeMap::new(),
        };
        assert!(scene.passable_query(0, 0, false).is_err());
        scene.set_passability(vec![0, 15, 0], 0).unwrap();
        assert!(scene.passable_query(0, 0, false).unwrap());
        scene.tiles.as_mut().unwrap()[4] = 200_000;
        assert!(!scene.passable_query(0, 0, false).unwrap());
        scene.tiles.as_mut().unwrap()[4] = 100_000;
        assert!(scene.passable_query(0, 0, false).unwrap());
        scene.tiles.as_mut().unwrap()[0] = 200_000;
        assert!(!scene.passable_query(0, 0, false).unwrap());
        scene.tiles.as_mut().unwrap()[4] = 300_000;
        assert!(scene.passable_query(0, 0, false).unwrap());
        scene.passability.as_mut().unwrap()[2] = 0x200;
        assert!(!scene.passable_query(0, 0, false).unwrap());
        scene.tiles.as_mut().unwrap().fill(300_000);
        scene.passability.as_mut().unwrap()[2] = 0x20 | 8;
        assert!(!scene.passable_query(0, 0, false).unwrap());
        assert!(scene.passable_query(0, 1, false).unwrap());
        scene.passability.as_mut().unwrap()[2] = 0x20 | 1;
        assert!(scene.passable_query(0, 0, false).unwrap());
        assert!(!scene.passable_query(0, 1, false).unwrap());
        scene.passability.as_mut().unwrap()[2] = 0;
        assert!(!scene.passable_query(-1, 0, false).unwrap());
        assert!(!scene.passable_query(3, 0, false).unwrap());
        assert!(!scene.passable_query(0, -1, false).unwrap());
        assert!(scene.passable_query(0, -1, true).unwrap());
        scene.loop_mode = 3;
        assert!(scene.passable_query(-1, -2, false).unwrap());
        assert!(scene.passable_query(4, 4, false).unwrap());
        scene.collision_layer_limit = 3;
        assert!(scene.passable_query(0, 0, false).unwrap());
        scene.collision_layer_limit = 0;
        scene.passability.as_mut().unwrap()[2] = 15;
        assert!(scene.passable_query(0, 0, false).unwrap());

        scene.tile_pixels = Some(48);
        scene.viewport = [48, 48];
        scene.loop_mode = 0;
        scene.player_x2 = 2;
        scene.player_y2 = 0;
        scene.center_camera([0, 0]).unwrap();
        assert_eq!(scene.camera_offset(), [48, 0]);
        assert_eq!(scene.camera_system_offsets().unwrap(), [48, 0]);
        scene.center_camera([24, 0]).unwrap();
        assert_eq!(scene.camera_offset(), [24, 0]);
        scene.loop_mode = 3;
        scene.player_x2 = 0;
        scene.viewport = [240, 48];
        scene.center_camera([0, 0]).unwrap();
        assert_eq!(scene.camera_offset(), [0, 0]);
        scene.camera_quarters = [-1, -5];
        assert_eq!(scene.camera_offset(), [0, -1]);
        assert_eq!(scene.camera_system_offsets().unwrap(), [0, 95]);
        scene.loop_mode = 0;
        scene.viewport = [48, 48];
        scene.camera_quarters = [50, 50];
        scene.follow_player_motion([50, 50], [54, 46], [0, 0]).unwrap();
        assert_eq!(scene.camera_quarters(), [54, 46]);
        scene.camera_lock_mask = 1;
        scene.follow_player_motion([54, 46], [58, 42], [0, 0]).unwrap();
        assert_eq!(scene.camera_quarters(), [54, 42]);
        scene.camera_lock_mask = 0;
        scene.follow_player_motion([58, 42], [200, -10], [0, 0]).unwrap();
        assert_eq!(scene.camera_quarters(), [192, 0]);
        scene.player_x2 = 0;
        scene.player_y2 = 0;
        scene.camera_quarters = [-1, 5];
        assert_eq!(scene.actor_property(-2, 0, 6).unwrap(), 25);
        assert_eq!(scene.actor_property(-2, 0, 7).unwrap(), 47);
        assert_eq!(scene.actor_property(-2, 0, 4).unwrap(), 0);
        scene.loop_mode = 3;
        scene.camera_quarters = [120, 192];
        assert_eq!(scene.actor_property(-2, 0, 6).unwrap(), -6);
        assert_eq!(scene.actor_property(-2, 0, 7).unwrap(), 96);
    }
    #[test]
    fn tile_number_uses_single_cell_and_converted_zero_for_layer_fallback() {
        let mut scene = Scene {
            player_route: RouteState::default(),
            map_id: 2,
            native_version: Some(371),
            width: 2,
            height: 2,
            player_x2: 0,
            player_y2: 0,
            direction: 2,
            layers: 2,
            player_frame: 1,
            player_opacity: 255,
            player_blend: 0,
            player_tile_id: -1,
            camera_lock_mask: 0,
            camera_quarters: [0; 2],
            tile_pixels: None,
            viewport: [0; 2],
            passability: None,
            loop_mode: 0,
            collision_mode: 0,
            configured_step_mode: 0,
            collision_layer_limit: 0,
            player_directions: 4,
            tiles: Some(vec![
                300_000, 300_000, 300_000, 300_000, 100_000, 100_000, 0, 100_000,
            ]),
            tags: vec![],
            events: BTreeMap::new(),
        };
        assert_eq!(scene.top_tile_number(0, 0).unwrap(), 2);
        // Ordinary raw tile0 converts to32; it does not fall through to layer0.
        assert_eq!(scene.top_tile_number(2, 0).unwrap(), 32);
        assert_eq!(scene.top_tile_number(0, 2).unwrap(), 2);
        assert_eq!(scene.top_tile_number(-1, -1).unwrap(), 2);
        // Querying an odd half-coordinate never samples an adjacent cell.
        assert_eq!(scene.top_tile_number(1, 0).unwrap(), 2);
        scene.native_version = Some(229);
        assert_eq!(scene.top_tile_number(2, 0).unwrap(), 16);
        scene.tiles.as_mut().unwrap()[4] = 1_700_000;
        assert_eq!(scene.top_tile_number(0, 0).unwrap(), -1);
        scene.native_version = None;
        assert!(scene.top_tile_number(0, 0).is_err());
    }

    #[test]
    fn real_parallel_scene_queries_resolve_map_and_inactive_event() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        if !root.join("Data/BasicData.wolf").exists() {
            eprintln!("private game fixture absent");
            return;
        }
        let bytes = crate::assets::read_member(&root, "BasicData/SysDatabase.dat").unwrap();
        let db = wolf_format::parse_database(&bytes, &Default::default()).unwrap();
        let initial = wolf_ir::startup::initial_position(&db.value).unwrap();
        let bytes = crate::assets::read_member(&root, &initial.map_path).unwrap();
        let map = wolf_format::parse_map(&bytes, &Default::default())
            .unwrap()
            .value;
        let mut scene = Scene::initial(&map, &initial, vec![]).unwrap();
        let bytes = crate::assets::read_member(&root, "BasicData/Game.dat").unwrap();
        scene
            .set_game_configuration(&wolf_format::parse_game(&bytes, &Default::default()).unwrap())
            .unwrap();
        for event in &map.events {
            let selected = wolf_ir::startup::initial_page(&event.pages, event.id, |n| match n {
                ..=999_999 => Ok(n),
                1_000_000..=1_199_999 | 2_000_000..=2_999_999 => Ok(0),
                _ => Err(format!("uninitialized native value {n}")),
            })
            .unwrap();
            if event.id == 0 {
                assert_eq!(selected, None);
            }
            scene
                .set_initial_page(event.id, selected.map(|p| &event.pages[p]))
                .unwrap();
        }
        let bytes = crate::assets::read_member(&root, "BasicData/CommonEvent.dat").unwrap();
        let bytes = bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&bytes);
        let commons = wolf_format::parse_common(bytes, &Default::default()).unwrap();
        let common = |id| {
            commons
                .value
                .events
                .iter()
                .find(|event| event.id == id)
                .unwrap()
        };
        let mut commands = vec![common(71).commands[0].clone()];
        commands.extend_from_slice(&common(91).commands[7..10]);
        commands.extend_from_slice(&common(38).commands[29..31]);
        assert_eq!(commands[0].args, [1_600_037, 0x3000, 0]);
        assert_eq!(commands[1].args, [1_600_016, 0x1000, 1_600_000, 0]);
        let mut camera = commands[0].clone();
        camera.opcode = 281;
        camera.args = vec![2, 0, 0];
        commands.push(camera);
        let program = wolf_ir::compile(
            "scene-queries".into(),
            "Common71/91 scene queries".into(),
            wolf_ir::Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: "private-fixture".into(),
                event: 91,
                page: None,
                command: 0,
                span: [0, 0],
            },
            &commands,
        )
        .unwrap();
        let mut vm = wolf_vm::runtime::Vm::new(
            wolf_ir::Library::new(vec![program]).unwrap(),
            Default::default(),
            1,
            1,
        )
        .unwrap();
        vm.spawn(
            wolf_ir::Target::Id("scene-queries".into()),
            u64::MAX,
            vec![],
        )
        .unwrap();
        let mut values = Vec::new();
        let report = vm
            .tick_with_host(vec![], &mut |ctx| {
                assert!(scene.execute(ctx)?);
                if ctx.instruction.raw.as_ref().unwrap().opcode == 124 {
                    values.push(ctx.number(ctx.instruction.raw.as_ref().unwrap().args[0])?);
                }
                Ok(true)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert_eq!(values, [2, 1, 1, 2, map.width as i32, map.height as i32]);
        assert_eq!(scene.camera_lock_mask(), 3);
        // No active page does not remove the entity. Owner references resolve IDs.
        assert_eq!(scene.actor_property(-1, 0, 0).unwrap(), 1);
        assert_eq!(scene.actor_property(-1, u64::MAX, 0).unwrap(), -1);
        assert_eq!(scene.actor_property(999_999, 0, 5).unwrap(), -1);
        assert!(scene.actor_property(-3, 0, 0).is_err());
        let mut selected_page = map.events[0].pages[0].clone();
        selected_page.graphic_fields[0] = 6;
        selected_page.graphic_fields[1] = 2;
        scene.set_initial_page(0, Some(&selected_page)).unwrap();
        assert_eq!(scene.actor_property(0, 0, 5).unwrap(), 6);
        assert_eq!(scene.events[&0].frame, 2);
        selected_page.graphic_fields[0] = 8;
        selected_page.graphic_fields[1] = 0;
        selected_page.page_transfer = Some(1);
        scene.set_page(0, Some(&selected_page)).unwrap();
        assert_eq!(scene.actor_property(0, 0, 5).unwrap(), 6);
        assert_eq!(scene.events[&0].frame, 2);
        scene.set_initial_page(0, None).unwrap();
        assert_eq!(scene.actor_property(0, 0, 5).unwrap(), 6);
        scene.set_page(0, Some(&selected_page)).unwrap();
        assert_eq!(scene.actor_property(0, 0, 5).unwrap(), 8);
        assert_eq!(scene.events[&0].frame, 0);
        // Sparse IDs must not be interpreted as vector positions.
        let actor = scene.events.remove(&0).unwrap();
        scene.events.insert(42, actor);
        assert_eq!(scene.actor_property(0, 0, 0).unwrap(), -1);
        assert_eq!(scene.actor_property(42, 0, 0).unwrap(), 1);
        assert_eq!(scene.actor_property(-1, 42, 5).unwrap(), 8);

        // Constructor half placement comes from the first page, even when a later
        // page is selected. X/Y flags are independent and are applied only once.
        let mut half_map = map.clone();
        half_map.events[0].pages[0].flags |= 0xC0;
        let mut half = Scene::initial(&half_map, &initial, vec![]).unwrap();
        half.set_page(0, None).unwrap();
        assert_eq!(half.events[&0].x2, half_map.events[0].x as i32 * 2 - 1);
        assert_eq!(half.events[&0].y2, half_map.events[0].y as i32 * 2 - 1);
        half.set_page(0, Some(&selected_page)).unwrap();
        assert_eq!(half.events[&0].x2, half_map.events[0].x as i32 * 2 - 1);
        let encoded = serde_json::to_vec(&half).unwrap();
        let decoded: Scene = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.events[&0].x2, half.events[&0].x2);

        // Execute the actual next Common71 query with player coordinates in self14/15.
        let command = common(71).commands[18].clone();
        assert_eq!(command.args, [1_600_027, 0x42000, 1_600_014, 1_600_015]);
        let program = wolf_ir::compile(
            "tile-number".into(),
            "Common71 PC18".into(),
            wolf_ir::Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: "private-fixture".into(),
                event: 71,
                page: None,
                command: 18,
                span: [0, 0],
            },
            &[command],
        )
        .unwrap();
        let mut vm = wolf_vm::runtime::Vm::new(
            wolf_ir::Library::new(vec![program]).unwrap(),
            Default::default(),
            1,
            1,
        )
        .unwrap();
        vm.spawn(wolf_ir::Target::Id("tile-number".into()), u64::MAX, vec![])
            .unwrap();
        let report = vm
            .tick_with_host(vec![], &mut |ctx| {
                ctx.set_number(1_600_014, initial.x)?;
                ctx.set_number(1_600_015, initial.y)?;
                assert!(scene.execute(ctx)?);
                // Current Map002 top layer at (6,6) is ordinary tile0 => number32.
                assert_eq!(ctx.number(1_600_027)?, 32);
                Ok(true)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    }

    #[test]
    fn tag_query_uses_top_layer_feet_pair_and_wrapped_coordinates() {
        let mut tags = vec![0; 35];
        tags[0] = 7;
        tags[33] = 10;
        tags[34] = 20;
        let mut scene = Scene {
            player_route: RouteState::default(),
            map_id: 2,
            native_version: Some(371),
            width: 2,
            height: 2,
            player_x2: 0,
            player_y2: 0,
            direction: 2,
            layers: 2,
            player_frame: 1,
            player_opacity: 255,
            player_blend: 0,
            player_tile_id: -1,
            tiles: Some(vec![100_000; 8]),
            camera_lock_mask: 0,
            camera_quarters: [0; 2],
            tile_pixels: None,
            viewport: [0; 2],
            passability: None,
            loop_mode: 0,
            collision_mode: 0,
            configured_step_mode: 0,
            collision_layer_limit: 0,
            tags,
            player_directions: 4,
            events: BTreeMap::new(),
        };
        // Transparent top layer; x-major cells (0,0),(0,1),(1,0),(1,1).
        scene.tiles.as_mut().unwrap()[4..].copy_from_slice(&[0, 1, 2, 0]);
        assert_eq!(scene.tile_tag(0, 0).unwrap(), 7);
        assert_eq!(scene.tile_tag(1, 0).unwrap(), 20);
        assert_eq!(scene.tile_tag(0, 1).unwrap(), 10);
        assert_eq!(scene.tile_tag(-1, -1).unwrap(), 20);
        assert_eq!(scene.tile_tag(4, 4).unwrap(), 7);
        assert_eq!(scene.tile_tag(0, 0).unwrap(), 7);
        scene.tiles.as_mut().unwrap()[4] = 99;
        assert!(scene.tile_tag(0, 0).is_err());
    }
}
