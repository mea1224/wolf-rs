//! Explicit Rust scene policy. Raw WOLF tile/page/route flags require an adapter.
use crate::runtime::{Vm, Wait};
use std::collections::BTreeMap;
use wolf_ir::Target;

/// Bridges movement requests to an explicitly decoded route. Owns the grid so
/// external route replacement cannot silently complete the wrong VM request.
pub struct Movement {
    grid: Grid,
    pending: BTreeMap<crate::runtime::Token, (u64, bool)>,
    last_tick: Option<u64>,
}
impl Movement {
    pub fn new(grid: Grid) -> Self {
        Self {
            grid,
            pending: BTreeMap::new(),
            last_tick: None,
        }
    }
    pub fn position(&self, actor: u64) -> Option<Position> {
        self.grid.position(actor)
    }
    /// Once after each VM tick, before Host::poll. Feed returned actions into
    /// that poll. Resolver errors fail the request; they never imply success.
    /// Scene state is not yet part of VM saves: do not restore this adapter
    /// against a restored VM or claim replay hashes cover movement positions.
    pub fn update(
        &mut self,
        vm: &Vm,
        mut decode: impl FnMut(&crate::runtime::Request) -> Result<(Vec<Step>, BlockedPolicy), String>,
    ) -> Result<Vec<crate::host::Action>, String> {
        use crate::host::Action;
        use wolf_ir::Service;
        let tick = vm.state().tick;
        if self.last_tick.is_some_and(|last| tick < last) {
            return Err("movement clock moved backwards".into());
        }
        if self.last_tick == Some(tick) {
            return Ok(Vec::new());
        }
        let live: std::collections::BTreeSet<_> = vm.requests().map(|r| r.token).collect();
        let stale: Vec<_> = self
            .pending
            .keys()
            .filter(|t| !live.contains(t))
            .copied()
            .collect();
        for token in stale {
            let (actor, active) = self.pending.remove(&token).unwrap();
            if active {
                self.grid.cancel(actor)?;
            }
        }
        let mut actions = Vec::new();
        for request in vm.requests() {
            let Service::Movement { actor, .. } = &request.service else {
                continue;
            };
            if self.pending.contains_key(&request.token) {
                continue;
            }
            let result = if self
                .pending
                .values()
                .any(|(id, active)| id == actor && *active)
            {
                Err("actor already has an outstanding movement".into())
            } else {
                decode(request).and_then(|(steps, policy)| self.grid.route(*actor, steps, policy))
            };
            let active = result.is_ok();
            if let Err(error) = result {
                actions.push(Action::Failed(
                    request.token,
                    error.chars().take(1024).collect(),
                ));
            }
            self.pending.insert(request.token, (*actor, active));
        }
        for result in self.grid.tick()? {
            let (actor, blocked) = match result {
                RouteResult::Finished(id) => (id, false),
                RouteResult::Blocked(id) => (id, true),
            };
            if let Some((token, (_, active))) = self
                .pending
                .iter_mut()
                .find(|(_, (id, active))| *id == actor && *active)
            {
                *active = false;
                actions.push(if blocked {
                    Action::Failed(*token, "movement blocked".into())
                } else {
                    Action::MovementFinished(*token)
                });
            }
        }
        self.last_tick = Some(tick);
        Ok(actions)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}
#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}
impl Direction {
    fn delta(self) -> (i32, i32) {
        match self {
            Self::Up => (0, -1),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
        }
    }
}
#[derive(Clone, Debug)]
pub enum Step {
    Move(Direction),
    Wait(u32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedPolicy {
    Retry,
    Skip,
    Fail,
}
#[derive(Clone, Debug)]
struct Route {
    steps: Vec<Step>,
    pc: usize,
    remaining: u32,
    blocked: BlockedPolicy,
}
#[derive(Clone, Debug)]
struct Actor {
    position: Position,
    solid: bool,
    route: Option<Route>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteResult {
    Finished(u64),
    Blocked(u64),
}
pub struct Grid {
    width: i32,
    height: i32,
    blocked: Vec<bool>,
    actors: BTreeMap<u64, Actor>,
    tick: u64,
}
impl Grid {
    pub fn new(width: u32, height: u32, blocked: Vec<bool>) -> Result<Self, String> {
        let size = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || size > 4_000_000 || size != blocked.len() as u64 {
            return Err("invalid grid dimensions/mask limit".into());
        }
        Ok(Self {
            width: width as i32,
            height: height as i32,
            blocked,
            actors: BTreeMap::new(),
            tick: 0,
        })
    }
    fn index(&self, p: Position) -> Option<usize> {
        (p.x >= 0 && p.y >= 0 && p.x < self.width && p.y < self.height)
            .then(|| p.y as usize * self.width as usize + p.x as usize)
    }
    fn passable(&self, p: Position, solid: bool) -> bool {
        self.index(p).is_some_and(|i| {
            !solid
                || (!self.blocked[i] && !self.actors.values().any(|a| a.solid && a.position == p))
        })
    }
    pub fn insert(&mut self, id: u64, position: Position, solid: bool) -> Result<(), String> {
        if self.actors.len() >= 65_536
            || self.actors.contains_key(&id)
            || !self.passable(position, solid)
        {
            return Err("actor limit, duplicate ID or occupied position".into());
        }
        self.actors.insert(
            id,
            Actor {
                position,
                solid,
                route: None,
            },
        );
        Ok(())
    }
    pub fn position(&self, id: u64) -> Option<Position> {
        self.actors.get(&id).map(|a| a.position)
    }
    pub fn route(
        &mut self,
        id: u64,
        steps: Vec<Step>,
        blocked: BlockedPolicy,
    ) -> Result<(), String> {
        if steps.len() > 100_000 {
            return Err("route step limit".into());
        }
        let a = self.actors.get_mut(&id).ok_or("unknown actor")?;
        a.route = Some(Route {
            steps,
            pc: 0,
            remaining: 0,
            blocked,
        });
        Ok(())
    }
    pub fn cancel(&mut self, id: u64) -> Result<(), String> {
        self.actors.get_mut(&id).ok_or("unknown actor")?.route = None;
        Ok(())
    }
    /// One step per tick, actors in ascending ID order. Solid actors cannot swap
    /// occupied cells or enter the same destination. A wait of zero takes one tick.
    pub fn tick(&mut self) -> Result<Vec<RouteResult>, String> {
        self.tick = self.tick.checked_add(1).ok_or("scene tick exhausted")?;
        let mut results = Vec::new();
        for id in self.actors.keys().copied().collect::<Vec<_>>() {
            let mut actor = self.actors.remove(&id).unwrap();
            if let Some(mut route) = actor.route.take() {
                let mut failed = false;
                if let Some(step) = route.steps.get(route.pc) {
                    match *step {
                        Step::Wait(ticks) => {
                            if route.remaining == 0 {
                                route.remaining = ticks.max(1);
                            }
                            route.remaining -= 1;
                            if route.remaining == 0 {
                                route.pc += 1;
                            }
                        }
                        Step::Move(direction) => {
                            let (dx, dy) = direction.delta();
                            let next = Position {
                                x: actor.position.x + dx,
                                y: actor.position.y + dy,
                            };
                            if self.passable(next, actor.solid) {
                                actor.position = next;
                                route.pc += 1;
                            } else {
                                match route.blocked {
                                    BlockedPolicy::Retry => {}
                                    BlockedPolicy::Skip => route.pc += 1,
                                    BlockedPolicy::Fail => {
                                        results.push(RouteResult::Blocked(id));
                                        failed = true;
                                    }
                                }
                            }
                        }
                    }
                }
                if !failed {
                    if route.pc == route.steps.len() {
                        results.push(RouteResult::Finished(id));
                    } else {
                        actor.route = Some(route);
                    }
                }
            }
            self.actors.insert(id, actor);
        }
        Ok(results)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Action,
    Once,
    Parallel,
}
#[derive(Clone, Debug)]
pub struct PageRule {
    pub target: Target,
    /// Explicit global equality guards; no interpretation of raw page bytes.
    pub equals: Vec<(i32, i32)>,
    pub trigger: Trigger,
}
struct Event {
    pages: Vec<PageRule>,
    selected: Option<usize>,
    fiber: Option<u64>,
}
pub struct Pages {
    events: BTreeMap<u64, Event>,
}
impl Default for Pages {
    fn default() -> Self {
        Self::new()
    }
}
impl Pages {
    pub fn new() -> Self {
        Self {
            events: BTreeMap::new(),
        }
    }
    pub fn insert(&mut self, owner: u64, pages: Vec<PageRule>) -> Result<(), String> {
        if self.events.len() >= 65_536
            || self.events.contains_key(&owner)
            || pages.len() > 256
            || pages.iter().any(|p| {
                p.equals.len() > 64
                    || p.equals
                        .iter()
                        .any(|(key, _)| !(0..1_000_000).contains(key))
            })
        {
            return Err("invalid page rules/event limit".into());
        }
        self.events.insert(
            owner,
            Event {
                pages,
                selected: None,
                fiber: None,
            },
        );
        Ok(())
    }
    /// Highest matching page wins. Call once before each VM tick. Page changes
    /// cancel pending services; parallel pages restart only after normal completion.
    /// Errors are per owner so one malformed target does not starve other events.
    pub fn update(&mut self, vm: &mut Vm, actions: &[u64]) -> Vec<(u64, String)> {
        let mut errors = Vec::new();
        for (&owner, event) in &mut self.events {
            let selected = event.pages.iter().rposition(|p| {
                p.equals.iter().all(|(key, value)| {
                    vm.state()
                        .world
                        .globals
                        .numbers
                        .get(key)
                        .copied()
                        .unwrap_or(0)
                        == *value
                })
            });
            let changed = selected != event.selected;
            if changed {
                if let Some(id) = event.fiber {
                    if vm.state().fibers.contains_key(&id) {
                        if let Err(e) = vm.cancel(id) {
                            errors.push((owner, e));
                            continue;
                        }
                    }
                }
                event.selected = selected;
            }
            let Some(index) = selected else {
                continue;
            };
            let page = &event.pages[index];
            let wait = event
                .fiber
                .and_then(|id| vm.state().fibers.get(&id))
                .map(|f| &f.wait);
            let should_start = match page.trigger {
                Trigger::Action => {
                    actions.contains(&owner)
                        && (changed || wait.is_none() || wait == Some(&Wait::Done))
                }
                Trigger::Once => changed,
                Trigger::Parallel => changed || wait.is_none() || wait == Some(&Wait::Done),
            };
            if !should_start {
                continue;
            }
            let result =
                if let Some(id) = event.fiber.filter(|id| vm.state().fibers.contains_key(id)) {
                    vm.replace_page(id, page.target.clone()).map(|_| id)
                } else {
                    vm.spawn(page.target.clone(), owner, vec![])
                };
            match result {
                Ok(id) => event.fiber = Some(id),
                Err(e) => {
                    errors.push((owner, e));
                    if changed {
                        event.selected = None;
                    }
                }
            }
        }
        errors
    }
}
