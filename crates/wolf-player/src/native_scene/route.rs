//! Verified A03150 route controls and 9F2BC0 through-enabled relative jumps.
use wolf_format::{MoveData, RouteCommand};
pub(super) mod path;

pub(super) trait Environment {
    fn number(&self, encoded: i32) -> Result<i32, String>;
    fn assign(&mut self, destination: i32, value: i32) -> Result<(), String>;
    fn clear(
        &self,
        position: [i32; 2],
        target: [i32; 2],
        direction: i16,
        step: i32,
        half: bool,
        through: bool,
    ) -> Result<bool, String>;
    fn player_position(&self) -> [i32; 2];
    fn actor_position(&self, target: i32) -> Result<Option<[i32; 2]>, String>;
    fn random(&mut self) -> Result<u32, String>;
    fn directions(&self) -> u8;
    fn path_clear_goal(
        &self,
        from: [i32; 2],
        to: [i32; 2],
        direction: i16,
        avoid_events: bool,
        _goal: [i32; 2],
    ) -> Result<bool, String> {
        self.path_clear(from, to, direction, avoid_events)
    }
    fn path_clear(
        &self,
        from: [i32; 2],
        to: [i32; 2],
        direction: i16,
        avoid_events: bool,
    ) -> Result<bool, String> {
        let _ = avoid_events;
        self.clear(
            from.map(|v| v * 2),
            to.map(|v| v * 2),
            direction,
            2,
            false,
            false,
        )
    }
}

#[cfg(test)]
struct NoEnvironment;
#[cfg(test)]
impl Environment for NoEnvironment {
    fn number(&self, n: i32) -> Result<i32, String> {
        if n < 1_000_000 {
            Ok(n)
        } else {
            Err("route requires variable host".into())
        }
    }
    fn assign(&mut self, _: i32, _: i32) -> Result<(), String> {
        Err("route requires variable host".into())
    }
    fn clear(
        &self,
        _: [i32; 2],
        _: [i32; 2],
        _: i16,
        _: i32,
        _: bool,
        through: bool,
    ) -> Result<bool, String> {
        Ok(through)
    }
    fn player_position(&self) -> [i32; 2] {
        [0, 0]
    }
    fn actor_position(&self, _: i32) -> Result<Option<[i32; 2]>, String> {
        Ok(None)
    }
    fn random(&mut self) -> Result<u32, String> {
        Err("route requires RNG host".into())
    }
    fn directions(&self) -> u8 {
        4
    }
}

fn argument_count(opcode: u8) -> Option<usize> {
    match opcode {
        0..=20 | 22..=27 | 32..=41 | 48..=52 | 56..=57 => Some(0),
        29..=31 | 44..=47 | 53 => Some(1),
        21 | 28 | 54..=55 | 60 | 160 => Some(2),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub tile: i32,
    pub size: [i32; 2],
    pub loops: [bool; 2],
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Program {
    commands: Vec<RouteCommand>,
    flags: u8,
    pc: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct RouteState {
    pub graphic: Option<Vec<u8>>,
    pub graphic_row: Option<i32>,
    pub opacity: Option<u16>,
    current: Program,
    backup: Program,
    pub forced: bool,
    wait: i32,
    pub direction_fixed: bool,
    pub through: bool,
    walking_animation: bool,
    stationary_animation: bool,
    above: bool,
    speed: i16,
    frequency: i16,
    animation_speed: i16,
    standing_frame: i16,
    step_width: i32,
    half_height: bool,
    render_quarters: Option<[i32; 2]>,
    jump_remaining: i32,
    jump_peak: i32,
    sounds: Vec<i32>,
    animation_mode: i32,
    pixel_offset: [i32; 2],
    height_offset: i32,
    moving_history: [bool; 2],
    manual_active: bool,
    walking_speeds: Option<[i16; 7]>,
    diagonal_percent: [i32; 2],
    patterns: i16,
    cyclic_animation: bool,
    animation_base: i16,
    animation_phase: i16,
    animation_quarters: i32,
    idle_ticks: i32,
    recent_movement: i32,
    #[serde(default)]
    movement_type: u8,
    #[serde(default)]
    path: Option<path::Search>,
}

impl Default for RouteState {
    fn default() -> Self {
        Self {
            graphic: None,
            graphic_row: None,
            opacity: None,
            current: Program::default(),
            backup: Program::default(),
            forced: false,
            wait: 0,
            direction_fixed: false,
            through: false,
            walking_animation: true,
            stationary_animation: false,
            above: false,
            speed: 3,
            frequency: 6,
            animation_speed: 2,
            standing_frame: 1,
            step_width: 1,
            half_height: false,
            render_quarters: None,
            jump_remaining: 0,
            jump_peak: 0,
            sounds: Vec::new(),
            animation_mode: 0,
            pixel_offset: [0, 0],
            height_offset: 0,
            moving_history: [false; 2],
            manual_active: false,
            walking_speeds: None,
            diagonal_percent: [100; 2],
            patterns: 3,
            cyclic_animation: false,
            animation_base: 1,
            animation_phase: 0,
            animation_quarters: 0,
            idle_ticks: 0,
            recent_movement: 0,
            movement_type: 0,
            path: None,
        }
    }
}

impl RouteState {
    pub(super) fn jump_active(&self) -> bool {
        self.jump_remaining > 0
    }
    pub(super) fn step_width(&self) -> i32 {
        self.step_width
    }
    pub(super) fn half_height(&self) -> bool {
        self.half_height
    }
    pub(super) fn pixel_offsets(&self) -> [i32; 2] {
        self.pixel_offset
    }
    pub(super) fn player_control_ready(&self) -> bool {
        !self.forced && !self.manual_active && self.jump_remaining == 0
    }
    pub fn configure(
        &mut self,
        config: &wolf_format::GameConfig,
        player: bool,
    ) -> Result<(), String> {
        // 9ECDE0..9ECDED initializes the player's collision height from
        // FA523C (configuration[9]); map actors instead use page flag 0x20.
        if player {
            self.half_height = *config.configuration.get(9)
                .ok_or("Game.dat player collision height missing")? == 1;
        }
        // 4596E9..459764 binds alternating Game.dat words to FA52C4/FA52E0.
        let mut speeds = [0; 7];
        for (index, speed) in speeds.iter_mut().enumerate() {
            let word = index * 2 + if player { 1 } else { 2 };
            let bytes = config
                .unknown_words
                .get(word * 2..word * 2 + 2)
                .ok_or("Game.dat movement speed table missing")?;
            *speed = i16::from_le_bytes(bytes.try_into().unwrap());
        }
        self.walking_speeds = Some(speeds);
        self.patterns = i16::from(
            *config
                .configuration
                .get(7)
                .ok_or("Game.dat animation patterns missing")?,
        );
        if self.patterns <= 0 {
            return Err("Game.dat animation pattern count must be positive".into());
        }
        // 4594F6 binds configuration[34] at FA5180+120 = FA52A0.
        self.cyclic_animation = *config
            .configuration
            .get(34)
            .ok_or("Game.dat animation cycle missing")?
            != 0;
        self.animation_base = if self.patterns == 5 { 2 } else { 1 };
        Ok(())
    }
    pub fn set_diagonal_percent(&mut self, percent: [i32; 2]) {
        self.diagonal_percent = percent;
    }
    pub fn page(
        &mut self,
        flags: u8,
        movement: &[u8],
        standing_frame: i16,
        route_flags: u8,
        commands: &[RouteCommand],
    ) {
        self.graphic = None;
        self.graphic_row = None;
        self.opacity = None;
        self.walking_animation = flags & 2 != 0;
        self.stationary_animation = flags & 1 != 0;
        self.direction_fixed = flags & 4 != 0;
        self.through = flags & 8 != 0;
        self.above = flags & 16 != 0;
        self.half_height = flags & 32 != 0;
        if movement.len() >= 4 {
            // 56ECE0 deserializes file bytes into native offsets +1,+2,+3,+0.
            // 9F8E14 then copies native +0/+1/+2/+3 to type/animation/speed/frequency.
            self.speed = i16::from(movement[1]);
            self.frequency = i16::from(movement[2]);
            self.animation_speed = i16::from(movement[0]);
            self.movement_type = movement[3];
        }
        self.standing_frame = standing_frame;
        // 9F8F65 replaces the autonomous route even while a forced route is active.
        let page = Program {
            commands: commands.to_vec(),
            flags: route_flags,
            pc: 0,
        };
        if self.forced {
            self.backup = page;
        } else {
            self.current = page;
            self.path = None;
        }
    }

    pub fn set_step_width(&mut self, width: i32) {
        self.step_width = width;
    }
    pub fn set_standing_frame(&mut self, frame: i16) {
        self.standing_frame = frame;
    }
    pub fn relocate(&mut self) {
        self.path = None;
        self.render_quarters = None;
        self.jump_remaining = 0;
        self.jump_peak = 0;
        self.manual_active = false;
        self.moving_history = [false; 2];
    }

    /// Start one native player walking step. The caller performs map/entity
    /// collision checks; this method owns only the route timing and render
    /// interpolation state.
    pub fn start_manual_move(
        &mut self,
        direction: i16,
        position: &mut [i32; 2],
        geometry: Geometry,
    ) -> Result<bool, String> {
        if !self.player_control_ready() {
            return Ok(false);
        }
        self.start_walk(direction, position, geometry)
    }

    fn start_walk(
        &mut self,
        direction: i16,
        position: &mut [i32; 2],
        geometry: Geometry,
    ) -> Result<bool, String> {
        let delta = match direction {
            1 => [-1, 1],
            2 => [0, 1],
            3 => [1, 1],
            4 => [-1, 0],
            6 => [1, 0],
            7 => [-1, -1],
            8 => [0, -1],
            9 => [1, -1],
            _ => return Ok(false),
        };
        let delta = delta.map(|d: i32| d.wrapping_mul(self.step_width));
        let mut target = [
            position[0].wrapping_add(delta[0]),
            position[1].wrapping_add(delta[1]),
        ];
        if geometry.loops[0] {
            target[0] = target[0].rem_euclid(geometry.size[0] * 2);
        }
        if geometry.loops[1] {
            target[1] = target[1].rem_euclid(geometry.size[1] * 2);
        }
        let min_y = if self.half_height { -1 } else { 0 };
        let in_bounds = (geometry.loops[0] || (0..geometry.size[0] * 2 - 1).contains(&target[0]))
            && (geometry.loops[1] || (min_y..geometry.size[1] * 2 - 1).contains(&target[1]));
        if !in_bounds {
            return Ok(false);
        }
        // Shift both endpoints across loop seams; never tween across the map.
        self.render_quarters =
            Some(self.target_quarters([target[0] - delta[0], target[1] - delta[1]], geometry.tile));
        *position = target;
        self.manual_active = true;
        self.idle_ticks = 0;
        Ok(true)
    }

    fn target_quarters(&self, position: [i32; 2], tile: i32) -> [i32; 2] {
        std::array::from_fn(|axis| {
            position[axis]
                .wrapping_mul(tile)
                .wrapping_mul(2)
                .wrapping_add(self.pixel_offset[axis].wrapping_mul(4))
        })
    }

    pub fn render_quarters(&self, position: [i32; 2], tile: i32) -> [i32; 2] {
        self.render_quarters
            .unwrap_or(self.target_quarters(position, tile))
    }

    pub fn jump_offset_pixels(&self, tile: i32) -> i32 {
        // 9F0590..9F05CD; positive value means height above the ground foot.
        let distance = self.jump_remaining.wrapping_sub(self.jump_peak);
        self.jump_peak
            .wrapping_mul(self.jump_peak)
            .wrapping_sub(distance.wrapping_mul(distance))
            .wrapping_mul(3)
            / (160 / tile)
            + self.height_offset
    }

    pub(super) fn height_offset(&self) -> i32 {
        self.height_offset
    }

    pub fn moving(&self, position: [i32; 2], tile: i32) -> bool {
        self.render_quarters(position, tile) != self.target_quarters(position, tile)
    }

    pub fn animated(&self, position: [i32; 2], tile: i32) -> bool {
        self.moving(position, tile)
            || self.moving_history.iter().any(|moving| *moving)
            || self.recent_movement > 0
            || self.animation_mode == 1
    }

    pub fn restore(
        &mut self,
        saved: &crate::native_save_payload::SavedCharacter,
    ) -> Result<(), String> {
        let read = |offset, default| saved.fields.get(&offset).copied().unwrap_or(default);
        self.forced = read(0x56, 0) != 0;
        self.through = read(0x57, i32::from(self.through)) != 0;
        self.direction_fixed = read(0xAA, i32::from(self.direction_fixed)) != 0;
        self.above = read(0xAB, i32::from(self.above)) != 0;
        self.walking_animation = read(0xA8, i32::from(self.walking_animation)) != 0;
        self.stationary_animation = read(0xA9, i32::from(self.stationary_animation)) != 0;
        self.speed = read(0x5E, i32::from(self.speed)) as i16;
        self.frequency = read(0x62, i32::from(self.frequency)) as i16;
        self.animation_speed = read(0x60, i32::from(self.animation_speed)) as i16;
        self.movement_type = read(0x5C, i32::from(self.movement_type)) as u8;
        self.standing_frame = read(0x50, i32::from(saved.frame)) as i16;
        self.step_width = read(0xB0, self.step_width);
        self.animation_mode = read(0x148, 0);
        self.pixel_offset = [read(0x158, 0), read(0x15C, 0)];
        self.height_offset = read(0xC0, 0);
        self.relocate(); // 9EE38E and 9EF0BC rebuild rendered position from the saved target.
        self.wait = 0; // Not serialized by 9EE140.
        for (index, pc_offset) in [0xA4, 0xA6].into_iter().enumerate() {
            if let Some(route) = saved.routes.get(index) {
                // 56E580 stores a one-byte NUL for empty string operands. Treat
                // that framing byte as the native empty value; only non-zero
                // payload bytes represent an actual string operand.
                if saved.route_strings.get(index).is_some_and(|strings| {
                    strings
                        .iter()
                        .any(|string| string.iter().any(|byte| *byte != 0))
                }) {
                    return Err("native saved route contains unsupported string operands".into());
                }
                let saved_pc =
                    usize::try_from(read(pc_offset, 0)).map_err(|_| "negative saved route PC")?;
                // Native keeps the last PC fields even after a route vector has
                // been drained. An empty route has no executable cursor, so its
                // stale PC is restored as the initial cursor rather than
                // rejecting an otherwise valid save.
                let pc = if route.routes.is_empty() { 0 } else { saved_pc };
                if pc > route.routes.len() {
                    return Err("saved route PC exceeds route length".into());
                }
                let program = Program {
                    commands: route.routes.clone(),
                    flags: route.flags,
                    pc,
                };
                if index == 0 {
                    self.current = program;
                } else {
                    self.backup = program;
                }
            }
        }
        Ok(())
    }

    pub fn take_sounds(&mut self) -> Vec<i32> {
        std::mem::take(&mut self.sounds)
    }

    pub fn install(&mut self, data: &MoveData) -> Result<(), String> {
        if data.unknown.len() != 5 || data.flags & !7 != 0 {
            return Err("unverified movement route header/flags".into());
        }
        for (pc, command) in data.routes.iter().enumerate() {
            let count = argument_count(command.opcode).ok_or_else(|| {
                format!(
                    "route PC{pc} opcode{} needs native movement consumer",
                    command.opcode
                )
            })?;
            if command.args.len() != count {
                return Err(format!(
                    "route PC{pc} opcode{} needs verified argument resolution",
                    command.opcode
                ));
            }
        }
        // 9EFAC1: preserve the previous route only while the backup vector is empty.
        if self.backup.commands.is_empty() {
            self.backup = self.current.clone();
        }
        self.current = Program {
            commands: data.routes.clone(),
            flags: data.flags,
            pc: 0,
        };
        self.path = None;
        self.forced = true;
        self.wait = 0;
        Ok(())
    }

    #[cfg(test)]
    pub fn tick(
        &mut self,
        direction: &mut i16,
        frame: &mut i16,
        position: &mut [i32; 2],
        geometry: Geometry,
        player: bool,
    ) -> Result<(), String> {
        self.tick_with_host(
            direction,
            frame,
            position,
            geometry,
            player,
            &mut NoEnvironment,
        )
    }

    pub fn tick_with_host(
        &mut self,
        direction: &mut i16,
        frame: &mut i16,
        position: &mut [i32; 2],
        geometry: Geometry,
        player: bool,
        host: &mut impl Environment,
    ) -> Result<(), String> {
        if !player && !self.forced && self.movement_type > 1 {
            return Err(format!(
                "autonomous movement type {} needs its native consumer",
                self.movement_type
            ));
        }
        self.advance_animation(frame)?;
        self.moving_history = [
            self.moving(*position, geometry.tile),
            self.moving_history[0],
        ];
        self.recent_movement = (self.recent_movement - 1).max(0);
        if self.moving_history[0] {
            self.recent_movement = 4;
        }
        if self.manual_active {
            let speeds = self
                .walking_speeds
                .as_ref()
                .ok_or("walking requires Game.dat speed table")?;
            let speed = i32::from(
                *speeds
                    .get(usize::try_from(self.speed).map_err(|_| "negative walking speed index")?)
                    .ok_or("walking speed index outside Game.dat table")?,
            );
            if speed <= 0 {
                return Err("walking speed must be positive".into());
            }
            let target = self.target_quarters(*position, geometry.tile);
            let mut rendered = self.render_quarters(*position, geometry.tile);
            // 9F0D60 updates down, left, right, up in this order. The second
            // axis sees the first axis's clamped result on the last tick.
            for (axis, positive) in [(1, true), (0, false), (0, true), (1, false)] {
                let difference = target[axis] - rendered[axis];
                if difference == 0 || (difference > 0) != positive {
                    continue;
                }
                let step = if rendered[1 - axis] == target[1 - axis] {
                    speed
                } else {
                    (speed.wrapping_mul(self.diagonal_percent[axis]) / 100).max(1)
                };
                rendered[axis] += difference.signum() * step.min(difference.abs());
            }
            self.manual_active = rendered != target;
            self.render_quarters = if self.manual_active {
                Some(rendered)
            } else {
                None
            };
            self.idle_ticks = if self.manual_active {
                0
            } else {
                self.idle_ticks.saturating_add(1)
            };
            self.animation_quarters += if self.walking_animation {
                4
            } else if self.stationary_animation {
                3
            } else {
                0
            };
            return Ok(());
        }
        if self.jump_remaining > 0 {
            // 9F088D..9F08E2 interpolates integer quarter-pixels after decrement.
            self.jump_remaining -= 1;
            let remaining = self.jump_remaining;
            let old = self.render_quarters(*position, geometry.tile);
            let target = self.target_quarters(*position, geometry.tile);
            self.render_quarters = Some(std::array::from_fn(|axis| {
                old[axis].wrapping_mul(remaining).wrapping_add(target[axis]) / (remaining + 1)
            }));
        }
        // 9F1010: stationary animation runs at 0.75 of the host step.
        self.animation_quarters += if self.animation_mode == 1 && self.walking_animation {
            4
        } else if self.stationary_animation {
            3
        } else if *frame != self.standing_frame {
            4
        } else {
            0
        };
        self.idle_ticks = self.idle_ticks.saturating_add(1);
        // 9F0BDD: events skip route dispatch when a positive wait is decremented.
        // Players (+58) dispatch separately at 9F7E76 after the decrement.
        if self.wait > 0 {
            self.wait -= 1;
            if !player || self.wait > 0 {
                return Ok(());
            }
        }
        if self.forced
            || !player
                && self.movement_type == 1
                && self.idle_ticks > i32::from(self.frequency).pow(2) * 20
        {
            self.run_with_host(direction, frame, position, geometry, host)?;
        }
        Ok(())
    }

    fn advance_animation(&mut self, frame: &mut i16) -> Result<(), String> {
        // FA4E44, with the five-pattern adjustments at 9F0821..9F0877.
        let thresholds = if self.patterns == 5 {
            [0, 3, 5, 8, 12, 40, 63]
        } else {
            [0, 6, 10, 16, 24, 60, 84]
        };
        let threshold = *thresholds
            .get(usize::try_from(self.animation_speed).map_err(|_| "negative animation speed")?)
            .ok_or("animation speed outside native table")?;
        if self.animation_quarters <= threshold * 4 {
            return Ok(());
        }
        self.animation_quarters = 0;
        if !self.stationary_animation
            && self.idle_ticks > self.recent_movement + 1
            && self.animation_mode == 0
        {
            *frame = self.standing_frame;
            self.animation_phase = 0;
        } else if self.cyclic_animation {
            self.animation_phase = (self.animation_phase + 1) % self.patterns;
            *frame = (self.animation_base + self.animation_phase) % self.patterns;
        } else if self.patterns == 3 {
            self.animation_phase = (self.animation_phase + 1) % 4;
            *frame = self.animation_base
                + if self.animation_phase < 2 {
                    self.animation_phase
                } else {
                    2 - self.animation_phase
                };
        } else if self.patterns == 5 {
            self.animation_phase = (self.animation_phase + 1) % 8;
            *frame = self.animation_base
                + if self.animation_phase < 5 {
                    2 - (2 - self.animation_phase).abs()
                } else {
                    -2 + (6 - self.animation_phase).abs()
                };
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn run(
        &mut self,
        direction: &mut i16,
        frame: &mut i16,
        position: &mut [i32; 2],
        geometry: Geometry,
    ) -> Result<(), String> {
        self.run_with_host(direction, frame, position, geometry, &mut NoEnvironment)
    }

    pub fn run_with_host(
        &mut self,
        direction: &mut i16,
        frame: &mut i16,
        position: &mut [i32; 2],
        geometry: Geometry,
        host: &mut impl Environment,
    ) -> Result<(), String> {
        if self.jump_remaining > 0 || self.manual_active {
            return Ok(());
        }
        // A03262 continues opcode 60 without rereading its destination operands.
        // Opcode 160 calls 9F5930 immediately after its first movement attempt.
        if let Some(mut path) = self.path.take() {
            if *position == path.goal.map(|v| v * 2) {
                self.current.pc += 1;
            } else {
                let next = path.next(*position, self.half_height, geometry, host)?;
                let moved = self.approach(direction, position, next, geometry, host)?;
                self.idle_ticks = 0;
                if moved || self.current.flags & 2 == 0 {
                    self.path = Some(path);
                } else {
                    self.current.pc += 1;
                }
                return Ok(());
            }
        }
        // Host work bound, not a native route limit. Detect a control-only loop.
        for _ in 0..4096 {
            let Some(command) = self.current.commands.get(self.current.pc) else {
                // A04534 restores the route displaced by a forced route.
                if self.current.flags & 1 != 0 && !self.current.commands.is_empty() {
                    self.current.pc = 0;
                } else {
                    if self.forced {
                        self.current = std::mem::take(&mut self.backup);
                    }
                    self.forced = false;
                }
                self.idle_ticks = 0;
                return Ok(());
            };
            let count = argument_count(command.opcode).ok_or_else(|| {
                format!(
                    "route PC{} opcode{} needs native movement consumer",
                    self.current.pc, command.opcode
                )
            })?;
            if command.args.len() != count {
                return Err(format!(
                    "route PC{} opcode{} has unsupported operands",
                    self.current.pc, command.opcode
                ));
            }
            let mut command = command.clone();
            for (index, arg) in command.args.iter_mut().enumerate() {
                if index != 0 || !matches!(command.opcode, 28 | 55) {
                    *arg = host.number(*arg)?;
                }
            }
            self.current.pc += 1;
            match command.opcode {
                60 | 160 => {
                    let goal = [command.args[0], command.args[1]];
                    if *position == goal.map(|v| v.wrapping_mul(2)) {
                        return Ok(());
                    }
                    let Some(mut path) =
                        path::Search::new(*position, goal, self.half_height, geometry, host)?
                    else {
                        if self.current.flags & 2 == 0 {
                            self.current.pc -= 1;
                        }
                        return Ok(());
                    };
                    let next = path.next(*position, self.half_height, geometry, host)?;
                    let moved = self.approach(direction, position, next, geometry, host)?;
                    if command.opcode == 60 {
                        self.current.pc -= 1;
                        self.path = Some(path);
                    } else if !moved && self.current.flags & 2 == 0 {
                        self.current.pc -= 1;
                    }
                    return Ok(());
                }
                0..=7 | 16..=20 | 53..=54 => {
                    let destination = match command.opcode {
                        17..=18 => Some(host.player_position()),
                        53 => host.actor_position(command.args[0])?,
                        54 => Some([
                            command.args[0].wrapping_mul(2),
                            command.args[1].wrapping_mul(2),
                        ]),
                        _ => None,
                    };
                    if matches!(command.opcode, 53..=54) && destination.is_none() {
                        return Ok(());
                    }
                    let mut alternate = None;
                    let walking_direction = match command.opcode {
                        0..=7 => [2, 4, 6, 8, 1, 3, 7, 9][usize::from(command.opcode)],
                        16 => [2, 4, 6, 8, 1, 3, 7, 9]
                            [host.random()? as usize % if host.directions() == 8 { 8 } else { 4 }],
                        17..=18 | 53..=54 => {
                            let mut d =
                                self.target_delta(*position, geometry, destination.unwrap());
                            if command.opcode == 18 {
                                d = d.map(|v| -v);
                            }
                            if d == [0, 0] {
                                if matches!(command.opcode, 17..=18) && self.current.flags & 2 == 0
                                {
                                    self.current.pc -= 1;
                                }
                                return Ok(());
                            }
                            if host.directions() == 8 && d[0] != 0 && d[1] != 0 {
                                match (d[0] > 0, d[1] > 0) {
                                    (false, true) => 1,
                                    (true, true) => 3,
                                    (false, false) => 7,
                                    (true, false) => 9,
                                }
                            } else {
                                let x = if d[0] < 0 { 4 } else { 6 };
                                let y = if d[1] < 0 { 8 } else { 2 };
                                let horizontal = if d[0].abs() == d[1].abs() {
                                    host.random()? & 1 == 0
                                } else {
                                    d[0].abs() > d[1].abs()
                                };
                                if horizontal {
                                    if d[1] != 0 {
                                        alternate = Some(y);
                                    }
                                    x
                                } else {
                                    if d[0] != 0 {
                                        alternate = Some(x);
                                    }
                                    y
                                }
                            }
                        }
                        19 => *direction,
                        _ => 10 - *direction,
                    };
                    let keep_direction = command.opcode == 20;
                    let mut moved = false;
                    for walking_direction in std::iter::once(walking_direction).chain(alternate) {
                        if !self.direction_fixed && !keep_direction {
                            *direction = self.facing(walking_direction, *direction);
                        }
                        let d = match walking_direction {
                            1 => [-1, 1],
                            2 => [0, 1],
                            3 => [1, 1],
                            4 => [-1, 0],
                            6 => [1, 0],
                            7 => [-1, -1],
                            8 => [0, -1],
                            9 => [1, -1],
                            _ => [0, 0],
                        };
                        let mut target =
                            std::array::from_fn(|axis| position[axis] + d[axis] * self.step_width);
                        for (axis, coordinate) in target.iter_mut().enumerate() {
                            if geometry.loops[axis] {
                                *coordinate = coordinate.rem_euclid(geometry.size[axis] * 2);
                            }
                        }
                        moved = host.clear(
                            *position,
                            target,
                            walking_direction,
                            self.step_width,
                            self.half_height,
                            self.through,
                        )? && self.start_walk(walking_direction, position, geometry)?;
                        if moved {
                            break;
                        }
                    }
                    if !moved && self.current.flags & 2 == 0 {
                        self.current.pc -= 1;
                    }
                    return Ok(());
                }
                21 => {
                    let delta = [
                        command.args[0].wrapping_mul(self.step_width),
                        command.args[1].wrapping_mul(self.step_width),
                    ];
                    if delta.iter().any(|value| value.unsigned_abs() > 32767) {
                        return Err(
                            "route jump exceeds host half-coordinate displacement limit".into()
                        );
                    }
                    let target = [
                        position[0].wrapping_add(delta[0]),
                        position[1].wrapping_add(delta[1]),
                    ];
                    if !self.direction_fixed && delta != [0, 0] {
                        *direction = if delta[0].unsigned_abs() > delta[1].unsigned_abs() {
                            if delta[0] < 0 { 4 } else { 6 }
                        } else if delta[1] < 0 {
                            8
                        } else {
                            2
                        };
                    }
                    // 9EFC83..9EFCB4 repeats the bounds check at y-1 for a
                    // half-height actor using half steps, even when through.
                    let min_y = if self.half_height && self.step_width != 1 {
                        -1
                    } else {
                        0
                    };
                    let in_bounds = (geometry.loops[0]
                        || (0..geometry.size[0] * 2 - 1).contains(&target[0]))
                        && (geometry.loops[1]
                            || (min_y..geometry.size[1] * 2 - 1).contains(&target[1]));
                    if !in_bounds
                        || delta != [0, 0]
                            && !host.clear(
                                *position,
                                target,
                                0,
                                self.step_width,
                                self.half_height,
                                self.through,
                            )?
                    {
                        if self.current.flags & 2 == 0 {
                            self.current.pc -= 1;
                        }
                        return Ok(());
                    }
                    self.render_quarters = Some(self.render_quarters(*position, geometry.tile));
                    *position = target;
                    if self.walking_animation || self.stationary_animation {
                        *frame = self.standing_frame;
                    }
                    // 9F2C8D..9F2CE9: sqrt of half-coordinate squared distance,
                    // truncated before subtracting speed and dividing by two.
                    let distance =
                        (f64::from(delta[0]).powi(2) + f64::from(delta[1]).powi(2)).sqrt() as i32;
                    self.jump_peak = (distance - i32::from(self.speed) + 15) / 2;
                    self.jump_remaining = self.jump_peak * 2;
                    self.idle_ticks = 0;
                    return Ok(());
                }
                8..=15 | 22..=27 => {
                    let random = if matches!(command.opcode, 24..=25) {
                        Some(host.random()?)
                    } else {
                        None
                    };
                    if !self.direction_fixed {
                        *direction = match command.opcode {
                            8..=11 => [2, 4, 6, 8][usize::from(command.opcode - 8)],
                            12..=15 => self
                                .facing([1, 3, 7, 9][usize::from(command.opcode - 12)], *direction),
                            22 => match *direction {
                                2 => 4,
                                4 => 8,
                                8 => 6,
                                6 => 2,
                                other => other,
                            },
                            23 => match *direction {
                                2 => 6,
                                6 => 8,
                                8 => 4,
                                4 => 2,
                                other => other,
                            },
                            24 => {
                                if random.unwrap() & 1 == 0 {
                                    match *direction {
                                        2 => 4,
                                        4 => 8,
                                        8 => 6,
                                        6 => 2,
                                        other => other,
                                    }
                                } else {
                                    match *direction {
                                        2 => 6,
                                        6 => 8,
                                        8 => 4,
                                        4 => 2,
                                        other => other,
                                    }
                                }
                            }
                            25 => self.facing(
                                [8, 6, 4, 2, 1, 3, 7, 9][random.unwrap() as usize
                                    % if host.directions() == 8 { 8 } else { 4 }],
                                *direction,
                            ),
                            26 => self.toward_player(*position, geometry, host.player_position()),
                            _ => {
                                10 - self.toward_player(*position, geometry, host.player_position())
                            }
                        };
                    }
                    self.idle_ticks = 0;
                    return Ok(());
                }
                28 => host.assign(command.args[0], command.args[1])?,
                55 => host.assign(
                    command.args[0],
                    host.number(command.args[0])?.wrapping_add(command.args[1]),
                )?,
                29 => self.speed = command.args[0] as i16,
                30 => self.frequency = command.args[0] as i16,
                31 => self.animation_speed = command.args[0] as i16,
                32 => self.stationary_animation = true,
                33 => self.stationary_animation = false,
                34 => self.walking_animation = true,
                35 => self.walking_animation = false,
                36 => self.direction_fixed = true,
                37 => self.direction_fixed = false,
                38 => self.through = true,
                39 => self.through = false,
                40 => self.above = true,
                41 => self.above = false,
                // A035B2..A03647 resolves SysDB8 row to a character graphic.
                44 => self.graphic_row = Some(command.args[0]),
                // A03651..A03719 clamps actor opacity, independently of route timing.
                45 => self.opacity = Some(command.args[0].clamp(0, 255) as u16),
                46 => self.sounds.push(command.args[0]),
                47 => self.wait = command.args[0],
                // Movement step width is stored even though translating forms are gated.
                48 => self.step_width = 1,
                49 => self.step_width = 2,
                50..=52 | 56..=57 => {
                    self.standing_frame = match command.opcode {
                        50..=52 => i16::from(command.opcode - 50),
                        56 => 3,
                        _ => 4,
                    };
                    *frame = self.standing_frame;
                }
                opcode => return Err(format!("unimplemented saved route opcode{opcode}")),
            }
            if self.wait > 0 {
                return Ok(());
            }
        }
        Err("native route exceeded host control-instruction budget".into())
    }

    fn approach(
        &mut self,
        direction: &mut i16,
        position: &mut [i32; 2],
        target: [i32; 2],
        geometry: Geometry,
        host: &mut impl Environment,
    ) -> Result<bool, String> {
        let d = self.target_delta(*position, geometry, target);
        if d == [0, 0] {
            return Ok(false);
        }
        let (first, second) = if host.directions() == 8 && d[0] != 0 && d[1] != 0 {
            (
                match (d[0] > 0, d[1] > 0) {
                    (false, true) => 1,
                    (true, true) => 3,
                    (false, false) => 7,
                    (true, false) => 9,
                },
                None,
            )
        } else {
            let x = if d[0] < 0 { 4 } else { 6 };
            let y = if d[1] < 0 { 8 } else { 2 };
            let horizontal = if d[0].abs() == d[1].abs() {
                host.random()? & 1 == 0
            } else {
                d[0].abs() > d[1].abs()
            };
            if horizontal {
                (x, (d[1] != 0).then_some(y))
            } else {
                (y, (d[0] != 0).then_some(x))
            }
        };
        for walking in std::iter::once(first).chain(second) {
            if !self.direction_fixed {
                *direction = self.facing(walking, *direction);
            }
            let d = path::direction_delta(walking);
            let mut target = std::array::from_fn(|a| position[a] + d[a] * self.step_width);
            for (a, coordinate) in target.iter_mut().enumerate() {
                if geometry.loops[a] {
                    *coordinate = coordinate.rem_euclid(geometry.size[a] * 2);
                }
            }
            if host.clear(
                *position,
                target,
                walking,
                self.step_width,
                self.half_height,
                self.through,
            )? && self.start_walk(walking, position, geometry)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn facing(&self, direction: i16, old: i16) -> i16 {
        // Four-direction graphic fallback in 9F2E30/70/B0/F0.
        match direction {
            1 => {
                if old == 6 {
                    4
                } else {
                    2
                }
            }
            3 => {
                if old == 4 {
                    6
                } else {
                    2
                }
            }
            7 => {
                if old == 6 {
                    4
                } else {
                    8
                }
            }
            9 => {
                if old == 4 {
                    6
                } else {
                    8
                }
            }
            other => other,
        }
    }

    fn target_delta(&self, position: [i32; 2], geometry: Geometry, player: [i32; 2]) -> [i32; 2] {
        std::array::from_fn(|axis| {
            let d = player[axis] - position[axis];
            if geometry.loops[axis] {
                [d, d - geometry.size[axis] * 2, d + geometry.size[axis] * 2]
                    .into_iter()
                    .min_by_key(|d| d.abs())
                    .unwrap()
            } else {
                d
            }
        })
    }
    fn toward_player(&self, position: [i32; 2], geometry: Geometry, player: [i32; 2]) -> i16 {
        let d = self.target_delta(position, geometry, player);
        if d[0].abs() > d[1].abs() {
            if d[0] < 0 { 4 } else { 6 }
        } else if d[1] < 0 {
            8
        } else {
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct FixtureEnvironment {
        clear: bool,
        values: std::collections::BTreeMap<i32, i32>,
    }
    impl Environment for FixtureEnvironment {
        fn number(&self, n: i32) -> Result<i32, String> {
            Ok(if n < 1_000_000 {
                n
            } else {
                *self.values.get(&n).unwrap_or(&0)
            })
        }
        fn assign(&mut self, n: i32, v: i32) -> Result<(), String> {
            self.values.insert(n, v);
            Ok(())
        }
        fn clear(
            &self,
            _: [i32; 2],
            _: [i32; 2],
            _: i16,
            _: i32,
            _: bool,
            through: bool,
        ) -> Result<bool, String> {
            Ok(self.clear || through)
        }
        fn player_position(&self) -> [i32; 2] {
            [10, 2]
        }
        fn actor_position(&self, _: i32) -> Result<Option<[i32; 2]>, String> {
            Ok(Some([10, 2]))
        }
        fn random(&mut self) -> Result<u32, String> {
            Ok(2)
        }
        fn directions(&self) -> u8 {
            4
        }
    }
    fn instruction(opcode: u8, args: Vec<i32>) -> RouteCommand {
        RouteCommand {
            opcode,
            args,
            span: wolf_format::Span { start: 0, end: 0 },
        }
    }

    #[derive(Default)]
    struct GridEnvironment {
        walls: std::collections::BTreeSet<[i32; 2]>,
        blocked: bool,
        assigned: i32,
        values: std::collections::BTreeMap<i32, i32>,
    }
    impl Environment for GridEnvironment {
        fn number(&self, n: i32) -> Result<i32, String> {
            Ok(self.values.get(&n).copied().unwrap_or(n))
        }
        fn assign(&mut self, _: i32, n: i32) -> Result<(), String> {
            self.assigned = n;
            Ok(())
        }
        fn clear(
            &self,
            _: [i32; 2],
            to: [i32; 2],
            _: i16,
            _: i32,
            _: bool,
            through: bool,
        ) -> Result<bool, String> {
            Ok(through || !self.blocked && !self.walls.contains(&to.map(|v| v / 2)))
        }
        fn player_position(&self) -> [i32; 2] {
            [0, 0]
        }
        fn actor_position(&self, _: i32) -> Result<Option<[i32; 2]>, String> {
            Ok(None)
        }
        fn random(&mut self) -> Result<u32, String> {
            Ok(2)
        }
        fn directions(&self) -> u8 {
            4
        }
        fn path_clear(&self, _: [i32; 2], to: [i32; 2], _: i16, _: bool) -> Result<bool, String> {
            Ok(!self.walls.contains(&to))
        }
    }
    fn path_state(opcode: u8, flags: u8) -> RouteState {
        let mut state = RouteState {
            step_width: 2,
            walking_speeds: Some([128; 7]),
            ..RouteState::default()
        };
        state
            .install(&MoveData {
                unknown: vec![0; 5],
                flags,
                routes: vec![
                    instruction(opcode, vec![6, 2]),
                    instruction(28, vec![2_000_001, 77]),
                ],
            })
            .unwrap();
        state
    }

    #[test]
    fn path_60_detours_and_restores_its_queue_before_following_command() {
        let mut state = path_state(60, 0);
        let geometry = Geometry {
            tile: 32,
            size: [10, 8],
            loops: [false; 2],
        };
        let mut host = GridEnvironment {
            walls: std::collections::BTreeSet::from([[3, 2], [3, 3]]),
            ..Default::default()
        };
        let (mut direction, mut frame, mut position) = (2, 1, [2, 4]);
        state
            .run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
            .unwrap();
        assert!(state.path.is_some());
        assert_eq!(state.current.pc, 0);
        assert_eq!(host.assigned, 0);
        let mut other: RouteState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        let (mut other_direction, mut other_frame, mut other_position) =
            (direction, frame, position);
        let mut other_host = GridEnvironment {
            walls: host.walls.clone(),
            ..Default::default()
        };
        for _ in 0..80 {
            state
                .tick_with_host(
                    &mut direction,
                    &mut frame,
                    &mut position,
                    geometry,
                    false,
                    &mut host,
                )
                .unwrap();
            other
                .tick_with_host(
                    &mut other_direction,
                    &mut other_frame,
                    &mut other_position,
                    geometry,
                    false,
                    &mut other_host,
                )
                .unwrap();
            assert_eq!(position, other_position);
            assert_eq!(host.assigned, other_host.assigned);
            assert!(!host.walls.contains(&position.map(|v| v / 2)));
            if host.assigned == 77 {
                break;
            }
        }
        assert_eq!(position, [12, 4]);
        assert_eq!(host.assigned, 77);
        assert!(state.path.is_none());
    }

    #[test]
    fn path_160_only_moves_once_then_advances_the_program() {
        let mut state = path_state(160, 0);
        let geometry = Geometry {
            tile: 32,
            size: [10, 8],
            loops: [false; 2],
        };
        let mut host = GridEnvironment::default();
        let (mut direction, mut frame, mut position) = (2, 1, [2, 4]);
        state
            .run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
            .unwrap();
        assert_eq!(position, [4, 4]);
        assert!(state.path.is_none());
        assert_eq!(state.current.pc, 1);
        for _ in 0..3 {
            state
                .tick_with_host(
                    &mut direction,
                    &mut frame,
                    &mut position,
                    geometry,
                    false,
                    &mut host,
                )
                .unwrap();
        }
        assert_eq!(position, [4, 4]);
        assert_eq!(host.assigned, 77);
    }

    #[test]
    fn path_failure_and_dynamic_blocking_retry_or_skip_from_route_flags() {
        let geometry = Geometry {
            tile: 32,
            size: [10, 8],
            loops: [false; 2],
        };
        let mut state = path_state(60, 0);
        let mut host = GridEnvironment {
            walls: std::collections::BTreeSet::from([[6, 2]]),
            ..Default::default()
        };
        let (mut direction, mut frame, mut position) = (2, 1, [2, 4]);
        state
            .run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
            .unwrap();
        assert_eq!(position, [2, 4]);
        assert_eq!(state.current.pc, 0);
        assert!(state.path.is_none());
        host.walls.clear();
        host.blocked = true;
        state
            .run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
            .unwrap();
        assert!(state.path.is_some());
        assert_eq!(state.current.pc, 0);
        state
            .run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
            .unwrap();
        assert_eq!(position, [2, 4]);
        assert_eq!(state.current.pc, 0);
        host.blocked = false;
        state
            .run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
            .unwrap();
        assert_eq!(position, [4, 4]);
        let mut skip = path_state(60, 2);
        host.walls.insert([6, 2]);
        skip.run_with_host(
            &mut direction,
            &mut frame,
            &mut position,
            geometry,
            &mut host,
        )
        .unwrap();
        assert_eq!(skip.current.pc, 1);
        assert!(skip.path.is_none());
        let mut skip = path_state(60, 2);
        host.walls.clear();
        host.blocked = true;
        skip.run_with_host(
            &mut direction,
            &mut frame,
            &mut position,
            geometry,
            &mut host,
        )
        .unwrap();
        assert!(skip.path.is_some());
        skip.run_with_host(
            &mut direction,
            &mut frame,
            &mut position,
            geometry,
            &mut host,
        )
        .unwrap();
        assert_eq!(skip.current.pc, 1);
        assert!(skip.path.is_none());
    }

    #[test]
    fn path_60_captures_destination_until_arrival() {
        let mut state = path_state(60, 0);
        state.current.commands[0].args = vec![2_000_010, 2_000_011];
        let mut host = GridEnvironment {
            values: std::collections::BTreeMap::from([(2_000_010, 6), (2_000_011, 2)]),
            ..Default::default()
        };
        let geometry = Geometry {
            tile: 32,
            size: [10, 8],
            loops: [false; 2],
        };
        let (mut direction, mut frame, mut position) = (2, 1, [2, 4]);
        state
            .run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
            .unwrap();
        host.values.insert(2_000_010, 8);
        for _ in 0..20 {
            state
                .tick_with_host(
                    &mut direction,
                    &mut frame,
                    &mut position,
                    geometry,
                    false,
                    &mut host,
                )
                .unwrap();
        }
        assert_eq!(position, [12, 4]);
        assert_eq!(host.assigned, 77);
    }

    #[test]
    fn page_header_deserialization_order_preserves_speed_frequency_and_route_type() {
        let mut state = RouteState::default();
        state.page(2, &[2, 3, 0, 1], 1, 1, &[instruction(10, vec![])]);
        assert_eq!(
            (
                state.animation_speed,
                state.speed,
                state.frequency,
                state.movement_type
            ),
            (2, 3, 0, 1)
        );
        let (mut direction, mut frame, mut position) = (2, 1, [2, 2]);
        state
            .tick(
                &mut direction,
                &mut frame,
                &mut position,
                Geometry {
                    tile: 32,
                    size: [10, 10],
                    loops: [false; 2],
                },
                false,
            )
            .unwrap();
        assert_eq!(direction, 6);
        assert!(!state.forced);
    }

    #[test]
    fn blocked_route_retries_then_walks_and_reads_current_variables() {
        let mut host = FixtureEnvironment::default();
        host.values.insert(2_000_002, 7);
        let mut state = RouteState {
            walking_speeds: Some([16; 7]),
            ..RouteState::default()
        };
        state
            .install(&MoveData {
                unknown: vec![0; 5],
                flags: 0,
                routes: vec![
                    instruction(2, vec![]),
                    instruction(28, vec![2_000_001, 2_000_002]),
                    instruction(55, vec![2_000_001, 5]),
                ],
            })
            .unwrap();
        let (mut direction, mut frame, mut position) = (2, 1, [2, 2]);
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false; 2],
        };
        state
            .run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
            .unwrap();
        assert_eq!(position, [2, 2]);
        assert_eq!(direction, 6);
        assert_eq!(state.current.pc, 0);
        host.clear = true;
        state
            .tick_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                false,
                &mut host,
            )
            .unwrap();
        assert_eq!(position, [3, 2]);
        assert_eq!(state.current.pc, 1);
        host.values.insert(2_000_002, 11);
        for _ in 0..5 {
            state
                .tick_with_host(
                    &mut direction,
                    &mut frame,
                    &mut position,
                    geometry,
                    false,
                    &mut host,
                )
                .unwrap();
        }
        assert_eq!(host.values.get(&2_000_001), Some(&16));
        assert!(!state.forced);
    }

    #[test]
    fn page_route_resumes_after_forced_route_and_page_change_updates_backup() {
        let mut state = RouteState::default();
        state.page(2, &[2, 3, 0, 1], 1, 1, &[instruction(10, vec![])]);
        state
            .install(&MoveData {
                unknown: vec![0; 5],
                flags: 0,
                routes: vec![instruction(8, vec![])],
            })
            .unwrap();
        state.page(2, &[2, 3, 0, 1], 1, 1, &[instruction(11, vec![])]);
        let (mut direction, mut frame, mut position) = (4, 1, [2, 2]);
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false; 2],
        };
        state
            .run(&mut direction, &mut frame, &mut position, geometry)
            .unwrap();
        assert_eq!(direction, 2);
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, false)
            .unwrap();
        assert!(!state.forced);
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, false)
            .unwrap();
        assert_eq!(direction, 8);
    }
    #[test]
    fn controls_continue_but_wait_and_facing_end_the_route_update() {
        let mut state = RouteState::default();
        let mut direction = 2;
        let mut frame = 0;
        let mut position = [0, 0];
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false, false],
        };
        let command = |opcode, args| RouteCommand {
            opcode,
            args,
            span: wolf_format::Span { start: 0, end: 0 },
        };
        state
            .install(&MoveData {
                unknown: vec![3, 3, 3, 0, 3],
                flags: 2,
                routes: vec![
                    command(33, vec![]),
                    command(51, vec![]),
                    command(47, vec![2]),
                    command(9, vec![]),
                    command(11, vec![]),
                ],
            })
            .unwrap();
        state
            .run(&mut direction, &mut frame, &mut position, geometry)
            .unwrap();
        assert_eq!((direction, frame, state.wait), (2, 1, 2));
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, false)
            .unwrap();
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, false)
            .unwrap();
        assert_eq!(direction, 2);
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, false)
            .unwrap();
        assert_eq!(direction, 4);
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, false)
            .unwrap();
        assert_eq!(direction, 8);
        assert!(state.forced);
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, false)
            .unwrap();
        assert!(!state.forced);
    }

    #[test]
    fn through_jump_interpolates_then_resumes_facing() {
        let mut state = RouteState::default();
        let mut direction = 2;
        let mut frame = 1;
        let mut position = [2, 2];
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false, false],
        };
        let command = |opcode, args| RouteCommand {
            opcode,
            args,
            span: wolf_format::Span { start: 0, end: 0 },
        };
        state
            .install(&MoveData {
                unknown: vec![3, 3, 3, 0, 3],
                flags: 2,
                routes: vec![
                    command(38, vec![]),
                    command(21, vec![3, 2]),
                    command(10, vec![]),
                    command(39, vec![]),
                ],
            })
            .unwrap();
        state
            .run(&mut direction, &mut frame, &mut position, geometry)
            .unwrap();
        assert_eq!(position, [5, 4]);
        assert_eq!(state.render_quarters(position, 32), [128, 128]);
        assert_eq!(state.jump_remaining, 14);
        for _ in 0..7 {
            state
                .tick(&mut direction, &mut frame, &mut position, geometry, false)
                .unwrap();
        }
        assert_eq!(state.jump_offset_pixels(32), 29);
        assert!(state.moving(position, 32));
        for _ in 0..7 {
            state
                .tick(&mut direction, &mut frame, &mut position, geometry, false)
                .unwrap();
        }
        assert_eq!(state.render_quarters(position, 32), [320, 256]);
        assert_eq!(state.jump_offset_pixels(32), 0);
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, false)
            .unwrap();
        assert!(!state.forced);
        assert!(!state.through);
    }

    #[test]
    fn player_wait_can_dispatch_on_the_tick_its_counter_reaches_zero() {
        let command = |opcode, args| RouteCommand {
            opcode,
            args,
            span: wolf_format::Span { start: 0, end: 0 },
        };
        let mut state = RouteState::default();
        state
            .install(&MoveData {
                unknown: vec![0; 5],
                flags: 0,
                routes: vec![command(47, vec![1]), command(11, vec![])],
            })
            .unwrap();
        let (mut direction, mut frame, mut position) = (2, 1, [2, 2]);
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false, false],
        };
        state
            .run(&mut direction, &mut frame, &mut position, geometry)
            .unwrap();
        state
            .tick(&mut direction, &mut frame, &mut position, geometry, true)
            .unwrap();
        assert_eq!(direction, 8);
    }

    #[test]
    fn manual_player_step_commits_target_and_interpolates_render_position() {
        let mut state = RouteState {
            walking_speeds: Some([8; 7]),
            step_width: 2,
            ..RouteState::default()
        };
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false, false],
        };
        let mut position = [2, 2];
        assert!(state.start_manual_move(6, &mut position, geometry).unwrap());
        assert_eq!(position, [4, 2]);
        assert!(state.moving(position, 32));
        let mut direction = 2;
        let mut frame = 1;
        for _ in 0..32 {
            state
                .tick(&mut direction, &mut frame, &mut position, geometry, true)
                .unwrap();
        }
        assert!(!state.moving(position, 32));
        assert_eq!(state.render_quarters(position, 32), [256, 128]);
    }

    #[test]
    fn walking_uses_configured_speed_and_loop_seam_is_one_step() {
        let mut state = RouteState {
            walking_speeds: Some([16; 7]),
            step_width: 1,
            ..RouteState::default()
        };
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [true, false],
        };
        let (mut position, mut direction, mut frame) = ([19, 2], 6, 1);
        state.start_manual_move(6, &mut position, geometry).unwrap();
        assert_eq!(position, [0, 2]);
        assert_eq!(state.render_quarters(position, 32), [-64, 128]);
        for expected in [-48, -32, -16, 0] {
            state
                .tick(&mut direction, &mut frame, &mut position, geometry, true)
                .unwrap();
            assert_eq!(state.render_quarters(position, 32)[0], expected);
        }
        assert!(state.player_control_ready());
    }

    #[test]
    fn walking_animation_cycles_and_returns_to_standing() {
        let mut state = RouteState {
            walking_speeds: Some([1; 7]),
            ..RouteState::default()
        };
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false; 2],
        };
        let (mut position, mut direction, mut frame) = ([2, 2], 6, 1);
        state.start_manual_move(6, &mut position, geometry).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            state
                .tick(&mut direction, &mut frame, &mut position, geometry, true)
                .unwrap();
            seen.insert(frame);
        }
        assert_eq!(seen, [0, 1, 2].into_iter().collect());
        for _ in 0..24 {
            state
                .tick(&mut direction, &mut frame, &mut position, geometry, true)
                .unwrap();
        }
        assert_eq!(frame, 1);
        assert!(!state.animated(position, 32));
    }

    #[test]
    fn stationary_five_pattern_animation_survives_snapshot_mid_cycle() {
        let mut state = RouteState {
            stationary_animation: true,
            patterns: 5,
            animation_base: 2,
            standing_frame: 2,
            ..RouteState::default()
        };
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false; 2],
        };
        let (mut position, mut direction, mut frame) = ([2, 2], 2, 2);
        for _ in 0..13 {
            state
                .tick(&mut direction, &mut frame, &mut position, geometry, false)
                .unwrap();
        }
        let mut restored: RouteState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        let mut other_frame = frame;
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..100 {
            state
                .tick(&mut direction, &mut frame, &mut position, geometry, false)
                .unwrap();
            restored
                .tick(
                    &mut direction,
                    &mut other_frame,
                    &mut position,
                    geometry,
                    false,
                )
                .unwrap();
            assert_eq!(frame, other_frame);
            seen.insert(frame);
        }
        assert_eq!(seen, [0, 1, 2, 3, 4].into_iter().collect());
    }

    #[test]
    fn half_height_through_jump_still_checks_the_second_footprint() {
        let command = |opcode, args| RouteCommand {
            opcode,
            args,
            span: wolf_format::Span { start: 0, end: 0 },
        };
        let route = MoveData {
            unknown: vec![0; 5],
            flags: 2,
            routes: vec![command(21, vec![0, -1])],
        };
        let geometry = Geometry {
            tile: 32,
            size: [10, 10],
            loops: [false, false],
        };
        let (mut direction, mut frame) = (2, 1);
        let mut state = RouteState {
            through: true,
            half_height: true,
            ..RouteState::default()
        };
        let mut position = [2, 0];
        state.install(&route).unwrap();
        state
            .run(&mut direction, &mut frame, &mut position, geometry)
            .unwrap();
        assert_eq!(position, [2, 0]);
        assert_eq!(state.jump_remaining, 0);
        state.set_step_width(2);
        position[1] = 1;
        state.install(&route).unwrap();
        state
            .run(&mut direction, &mut frame, &mut position, geometry)
            .unwrap();
        assert_eq!(position, [2, -1]);
    }
}
