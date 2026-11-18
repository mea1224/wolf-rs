//! Scene/VM boundary for A03150. All operands remain script-derived.
use super::{Scene, route::Environment};
use wolf_vm::runtime::{NativeContext, World};

pub(super) fn bind(
    data: &wolf_format::MoveData,
    target: i32,
    context: &NativeContext<'_>,
) -> Result<wolf_format::MoveData, String> {
    let mut data = data.clone();
    // A05B08..A05C27, current >=325 profile. Globals and absolute map-self
    // operands remain live, while common-self/sys/database inputs are captured.
    for command in &mut data.routes {
        for (index, arg) in command.args.iter_mut().enumerate() {
            if index == 0 && matches!(command.opcode, 28 | 55) {
                continue;
            }
            if (1_100_000..1_200_000).contains(arg) {
                if target >= 0 {
                    *arg = arg
                        .wrapping_sub(100_000)
                        .wrapping_add(target.wrapping_mul(10));
                }
            } else if *arg >= 1_000_000
                && !(1_000_000..1_100_000).contains(arg)
                && !(2_000_000..3_000_000).contains(arg)
            {
                *arg = context.number(*arg)?;
            }
        }
    }
    Ok(data)
}

struct Host<'a> {
    scene: &'a Scene,
    world: &'a mut World,
    actor: i32,
    rng: &'a mut u32,
}
impl Host<'_> {
    fn actors_clear(&self, target: [i32; 2], half: bool, ignore_goal: Option<[i32; 2]>) -> bool {
        let overlaps = |target: [i32; 2], p: [i32; 2], other_half: bool| {
            let dx = (target[0] - p[0]).abs();
            let dx = if self.scene.loop_mode & 1 != 0 {
                dx.min((dx - self.scene.width * 2).abs())
            } else {
                dx
            };
            let dy = target[1] - p[1];
            let ys = if self.scene.loop_mode & 2 != 0 {
                [dy, dy - self.scene.height * 2, dy + self.scene.height * 2]
            } else {
                [dy; 3]
            };
            dx <= 1
                && ys
                    .into_iter()
                    .any(|d| d == 0 || other_half && d == -1 || half && d == 1)
        };
        let player = (self.actor != -2 && !self.scene.player_route.through).then(|| {
            (
                -2,
                self.player_position(),
                self.scene.player_route.half_height(),
            )
        });
        let obstacles = || {
            player
                .into_iter()
                .chain(self.scene.events.iter().filter_map(|(&id, actor)| {
                    if id == self.actor
                        || !actor.active_page
                        || actor.route.through
                        || actor.route.jump_active()
                    {
                        return None;
                    }
                    let graphic = actor.route.graphic.as_deref().unwrap_or(&actor.graphic);
                    if actor.tile_id == -1
                        && graphic.strip_suffix(&[0]).unwrap_or(graphic).is_empty()
                    {
                        return None;
                    }
                    Some((id, [actor.x2, actor.y2], actor.route.half_height()))
                }))
        };
        // 9F46A0..9F4B31 removes the destination occupant from the temporary
        // obstacle matrix. Actual walking still collides with that character.
        let ignored = ignore_goal.and_then(|goal| {
            obstacles()
                .rev()
                .find(|(_, p, h)| overlaps(goal, *p, *h))
                .map(|(id, _, _)| id)
        });
        !obstacles().any(|(id, p, h)| Some(id) != ignored && overlaps(target, p, h))
    }
    fn address(&self, n: i32) -> Result<wolf_ir::Var, String> {
        let n = if (1_100_000..1_200_000).contains(&n) {
            if self.actor < 0 {
                return Err("player route has no event-self destination".into());
            }
            1_000_000 + self.actor * 10 + n % 10
        } else {
            n
        };
        wolf_ir::address(n)
    }
}
impl Environment for Host<'_> {
    fn number(&self, n: i32) -> Result<i32, String> {
        // A03320..A033A5: no common call frame exists during route execution.
        if n < 1_000_000 {
            return Ok(n);
        }
        if (1_600_000..1_700_000).contains(&n)
            || self.actor < 0 && (1_100_000..1_200_000).contains(&n)
        {
            return Ok(0);
        }
        if wolf_vm::native_values::DatabaseAddress::decode(n).is_some() {
            return self.world.databases.number(n);
        }
        let (slots, key) = match self.address(n)? {
            wolf_ir::Var::Global(key) => (Some(&self.world.globals), key),
            wolf_ir::Var::System(key) => (Some(&self.world.systems), key),
            wolf_ir::Var::Common(key) => (Some(&self.world.commons), key),
            wolf_ir::Var::MapEvent { event, slot } => {
                (self.world.events.get(&u64::from(event)), slot)
            }
            other => return Err(format!("unsupported route numeric address {other:?}")),
        };
        Ok(slots
            .and_then(|s| s.numbers.get(&key))
            .copied()
            .unwrap_or(0))
    }
    fn assign(&mut self, destination: i32, value: i32) -> Result<(), String> {
        let (slots, key) = match self.address(destination)? {
            wolf_ir::Var::Global(key) => (&mut self.world.globals, key),
            wolf_ir::Var::Common(key) => (&mut self.world.commons, key),
            wolf_ir::Var::MapEvent { event, slot } => {
                (self.world.events.entry(u64::from(event)).or_default(), slot)
            }
            other => {
                return Err(format!(
                    "route assignment requires a verified setter for {other:?}"
                ));
            }
        };
        slots.numbers.insert(key, value);
        Ok(())
    }
    fn player_position(&self) -> [i32; 2] {
        [self.scene.player_x2, self.scene.player_y2]
    }
    fn actor_position(&self, target: i32) -> Result<Option<[i32; 2]>, String> {
        match target {
            -1 => Ok(None),
            -2 => Ok(Some(self.player_position())),
            n if n < -2 => Err(format!("route follower target {n} requires follower state")),
            n => Ok(self.scene.events.get(&n).map(|actor| [actor.x2, actor.y2])),
        }
    }
    fn random(&mut self) -> Result<u32, String> {
        wolf_vm::runtime::native_random(self.rng)
    }
    fn directions(&self) -> u8 {
        self.scene.player_directions
    }
    fn path_clear_goal(
        &self,
        from: [i32; 2],
        to: [i32; 2],
        direction: i16,
        avoid_events: bool,
        goal: [i32; 2],
    ) -> Result<bool, String> {
        if !self.path_clear(from, to, direction, false)? {
            return Ok(false);
        }
        if !avoid_events {
            return Ok(true);
        }
        let from = from.map(|v| v * 2);
        let to = to.map(|v| v * 2);
        let goal = Some(goal.map(|v| v * 2));
        if !self.actors_clear(to, false, goal) {
            return Ok(false);
        }
        let d = super::route::path::direction_delta(direction);
        Ok(d[0] == 0
            || d[1] == 0
            || self.actors_clear([to[0], from[1]], false, goal)
                && self.actors_clear([from[0], to[1]], false, goal))
    }
    fn path_clear(
        &self,
        from: [i32; 2],
        to: [i32; 2],
        direction: i16,
        avoid_events: bool,
    ) -> Result<bool, String> {
        let from = from.map(|v| v * 2);
        let to = to.map(|v| v * 2);
        if !self.scene.passable_query(to[0], to[1], false)? {
            return Ok(false);
        }
        if direction == 0 {
            return Ok(true);
        }
        let delta = super::route::path::direction_delta(direction);
        let cardinal = i32::from(direction);
        let axes: &[i32] = match direction {
            1 => &[2, 4],
            3 => &[2, 6],
            7 => &[8, 4],
            9 => &[8, 6],
            _ => std::slice::from_ref(&cardinal),
        };
        for &axis in axes {
            if !self
                .scene
                .passable_direction(from[0], from[1], false, axis, 2)?
                || !self
                    .scene
                    .passable_direction(to[0], to[1], false, 10 - axis, 2)?
            {
                return Ok(false);
            }
        }
        if avoid_events && !self.clear(from, to, direction, 2, false, false)? {
            return Ok(false);
        }
        if delta[0] != 0 && delta[1] != 0 {
            for (point, dir) in [
                ([to[0], from[1]], if delta[0] < 0 { 4 } else { 6 }),
                ([from[0], to[1]], if delta[1] < 0 { 8 } else { 2 }),
            ] {
                if !self.scene.passable_query(point[0], point[1], false)?
                    || avoid_events && !self.clear(from, point, dir, 2, false, false)?
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    fn clear(
        &self,
        position: [i32; 2],
        target: [i32; 2],
        direction: i16,
        step: i32,
        half: bool,
        through: bool,
    ) -> Result<bool, String> {
        if through {
            return Ok(true);
        }
        if !self.scene.passable_query(target[0], target[1], half)? {
            return Ok(false);
        }
        if half && step == 1 && !self.scene.passable_query(target[0], target[1] - 1, half)? {
            return Ok(false);
        }
        let dir = i32::from(direction);
        let axes: &[i32] = match dir {
            0 => &[],
            1 => &[2, 4],
            3 => &[2, 6],
            7 => &[8, 4],
            9 => &[8, 6],
            _ => std::slice::from_ref(&dir),
        };
        for &axis in axes {
            if !self
                .scene
                .passable_direction(position[0], position[1], half, axis, step)?
                || !self
                    .scene
                    .passable_direction(target[0], target[1], half, 10 - axis, step)?
            {
                return Ok(false);
            }
        }
        Ok(self.actors_clear(target, half, None))
    }
}

impl Scene {
    pub(super) fn advance_route(
        &mut self,
        target: i32,
        world: &mut World,
        rng: &mut u32,
        tick: bool,
    ) -> Result<(), String> {
        let geometry = self.route_geometry()?;
        let player = target == -2;
        let (mut route, mut direction, mut frame, mut position) = if player {
            (
                std::mem::take(&mut self.player_route),
                self.direction,
                self.player_frame,
                [self.player_x2, self.player_y2],
            )
        } else {
            let actor = self.events.get_mut(&target).ok_or("route actor missing")?;
            (
                std::mem::take(&mut actor.route),
                actor.direction,
                actor.frame,
                [actor.x2, actor.y2],
            )
        };
        let mut host = Host {
            scene: self,
            world,
            actor: target,
            rng,
        };
        let result = if tick {
            route.tick_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                player,
                &mut host,
            )
        } else {
            route.run_with_host(
                &mut direction,
                &mut frame,
                &mut position,
                geometry,
                &mut host,
            )
        };
        if player {
            self.player_route = route;
            self.direction = direction;
            self.player_frame = frame;
            [self.player_x2, self.player_y2] = position;
        } else {
            let actor = self.events.get_mut(&target).unwrap();
            actor.route = route;
            actor.direction = direction;
            actor.frame = frame;
            [actor.x2, actor.y2] = position;
        }
        result.map_err(|e| format!("map{} actor{target} route: {e}", self.map_id))
    }
}
