//! Headless deterministic executor for wolf-ir. Arithmetic/scheduling are versioned runtime policy,
//! not a claim that every WOLF instruction has been reverse engineered.
use crate::input::{KeyboardSample, KeyboardState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::ops::Bound::{Excluded, Unbounded};
use wolf_ir::{
    Arithmetic, Binary, Compare, Condition, Library, Op, Service, Source, Target, Text, Update,
    Value, Var,
};
type Result<T> = std::result::Result<T, String>;
// v7 includes common return metadata and prioritizes the configured native main fiber.
const SAVE_VERSION: u32 = 11;
const MAX_ARGUMENTS: usize = 100;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Native A06570 runs each execution context until it yields before the next slot.
    pub run_until_yield: bool,
    /// Main native context whose +808 program pointer backs system variable 13.
    pub native_main_fiber: Option<u64>,
    /// Common record +13D8: -1 disables return, otherwise persistent self slot.
    pub native_common_returns: BTreeMap<u32, i32>,
    pub instruction_budget: u32,
    pub max_stack: usize,
    pub max_fibers: usize,
    pub hot_loop_ticks: u32,
    pub max_save_bytes: usize,
    pub max_string_bytes: usize,
    pub max_slots: usize,
    pub max_event_owners: usize,
    pub max_completions_per_tick: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            run_until_yield: false,
            native_main_fiber: None,
            native_common_returns: BTreeMap::new(),
            instruction_budget: 10_000,
            max_stack: 128,
            max_fibers: 4096,
            hot_loop_ticks: 120,
            max_save_bytes: 16 << 20,
            max_string_bytes: 1 << 20,
            max_slots: 100_000,
            max_event_owners: 65_536,
            max_completions_per_tick: 16_384,
        }
    }
}
impl Config {
    fn validate(&self) -> Result<()> {
        if !(1..=1_000_000).contains(&self.instruction_budget)
            || !(1..=1024).contains(&self.max_stack)
            || !(1..=65_536).contains(&self.max_fibers)
            || self.hot_loop_ticks == 0
            || !(1..=256 << 20).contains(&self.max_save_bytes)
            || !(1..=16 << 20).contains(&self.max_string_bytes)
            || !(1..=1_000_000).contains(&self.max_slots)
            || !(1..=65_536).contains(&self.max_event_owners)
            || !(1..=1_000_000).contains(&self.max_completions_per_tick)
        {
            return Err("VM limit outside supported range".into());
        }
        if self
            .native_common_returns
            .iter()
            .any(|(&id, &slot)| id >= 10_000 || !(-1..100).contains(&slot))
        {
            return Err("invalid common return metadata".into());
        }
        Ok(())
    }
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Slots {
    pub numbers: BTreeMap<i32, i32>,
    pub strings: BTreeMap<i32, String>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct World {
    #[serde(default)]
    pub databases: crate::native_values::NativeDatabases,
    pub globals: Slots,
    pub commons: Slots,
    pub systems: Slots,
    pub events: BTreeMap<u64, Slots>,
    /// Loaded map and banks belonging to unloaded maps. Execution contexts
    /// retain their originating map across a transfer (A053D0/A07386).
    pub current_map: Option<i32>,
    pub map_events: BTreeMap<i32, BTreeMap<u64, Slots>>,
    #[serde(skip)]
    event_context: Option<i32>,
}
impl World {
    fn event_bank(&self) -> Option<&BTreeMap<u64, Slots>> {
        match self.event_context {
            Some(map) if Some(map) != self.current_map => self.map_events.get(&map),
            _ => Some(&self.events),
        }
    }
    fn event_bank_mut(&mut self) -> &mut BTreeMap<u64, Slots> {
        match self.event_context {
            Some(map) if Some(map) != self.current_map => self.map_events.entry(map).or_default(),
            _ => &mut self.events,
        }
    }
    fn event_owner_count(&self) -> usize {
        self.map_events.values().fold(self.events.len(), |n, bank| n.saturating_add(bank.len()))
    }
    pub fn switch_map(&mut self, map: i32) -> Result<()> {
        if map < 0 || self.event_context.is_some() {
            return Err("invalid map switch outside a host tick boundary".into());
        }
        if self.current_map == Some(map) { return Ok(()); }
        if let Some(old) = self.current_map {
            self.map_events.insert(old, std::mem::take(&mut self.events));
            self.events = self.map_events.remove(&map).unwrap_or_default();
        } else if let Some(bank) = self.map_events.remove(&map) {
            self.events = bank;
        }
        self.current_map = Some(map);
        Ok(())
    }
    fn validate_maps(&self) -> Result<()> {
        if self.current_map.is_some_and(|map| map < 0)
            || self.map_events.keys().any(|map| *map < 0 || Some(*map) == self.current_map)
            || self.map_events.len() > 65_536
        {
            return Err("invalid saved map variable banks".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Token {
    pub session: u64,
    pub sequence: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wait {
    Ready,
    Ticks(u64),
    Service(Token),
    Keyboard { key: i32, memory: i32 },
    NativeInput { key: i32, memory: i32 },
    Done,
    Fault,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopState {
    pub entered_by_label: bool,
    pub end: usize,
    pub remaining: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub program: String,
    pub pc: usize,
    pub locals: Slots,
    pub loops: BTreeMap<usize, LoopState>,
    /// Native201 selected actor, or i32::MIN for Native202 all forced routes.
    pub native_route_wait: Option<i32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub tick: u64,
    pub fiber: u64,
    pub source: Option<Source>,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fiber {
    pub id: u64,
    pub owner: u64,
    pub map_id: Option<i32>,
    pub page_generation: u64,
    pub frames: Vec<Frame>,
    pub wait: Wait,
    pub fault: Option<Diagnostic>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub token: Token,
    pub fiber: u64,
    pub page_generation: u64,
    pub source: Source,
    pub service: Service,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Done,
    Choice(usize),
    Cancel,
    Failed(String),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    pub token: Token,
    pub response: Response,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Reserved {
    target: String,
    owner: u64,
    map_id: Option<i32>,
    args: Vec<i32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    version: u32,
    library: String,
    config: Config,
    pub tick: u64,
    pub rng: u32,
    /// Exact 942820 state; the supplied VM seed replaces native GetTickCount initialization.
    /// Zero and multiples of 0x7fffffff require the unbound native clock reseed path.
    pub native_rng: u32,
    pub world: World,
    pub keyboard: KeyboardState,
    pub fibers: BTreeMap<u64, Fiber>,
    session: u64,
    next_fiber: u64,
    next_token: u64,
    cursor: Option<u64>,
    requests: BTreeMap<u64, Request>,
    reserved: Vec<Reserved>,
    native_pending_common: Option<u32>,
    native_common_fiber: Option<u64>,
    native_pending_maps: BTreeSet<(Option<i32>, u32)>,
    exhausted_ticks: u32,
}
#[derive(Debug, Default, Serialize)]
pub struct TickReport {
    pub tick: u64,
    pub instructions: u32,
    pub stale_completions: usize,
    pub rejected_completions: usize,
    pub budget_exhausted: bool,
    /// A host restore/exit invalidated the old execution tree during this tick.
    pub execution_invalidated: bool,
    pub diagnostics: Vec<Diagnostic>,
}
pub struct Vm {
    library: Library,
    state: State,
    // Pending external transaction; cannot be serialized without its host payload.
    execution_invalidated: bool,
}
/// Borrowed host boundary. Script state remains in the VM; media handles remain in the player.
pub struct NativeContext<'a> {
    pub instruction: &'a wolf_ir::Instruction,
    pub world: &'a mut World,
    pub owner: u64,
    pub frame: &'a mut Frame,
    pub tick: u64,
    native_rng: &'a mut u32,
    config: &'a Config,
    yield_requested: bool,
    invalidate_requested: bool,
    retry_requested: bool,
    return_requested: bool,
}
impl NativeContext<'_> {
    pub fn native_random(&mut self) -> Result<u32> {
        native_random(self.native_rng)
    }
    pub fn world_and_native_rng_mut(&mut self) -> (&mut World, &mut u32) {
        (self.world, self.native_rng)
    }
    /// The host recognized this operation but its completion gate remains closed.
    pub fn retry(&mut self) {
        self.retry_requested = true;
    }
    /// Accept the instruction, then return this native execution context for this tick.
    pub fn yield_once(&mut self) {
        self.yield_requested = true;
    }
    /// A0AC90 ends only this native call context, like opcode172.
    pub fn return_from_call(&mut self) {
        self.return_requested = true;
        self.yield_requested = true;
    }
    /// A successful full restore or exit makes every remaining old context invalid.
    /// The host must commit its transaction and clear/rebuild execution before ticking again.
    pub fn invalidate_execution(&mut self) {
        self.invalidate_requested = true;
    }
    /// The verified sysS60 setter; other system writes still need their own adapters.
    pub fn set_window_title(&mut self, value: String) -> Result<()> {
        if value.len() > self.config.max_string_bytes {
            return Err("window title byte limit".into());
        }
        check_slot_write(Some(&self.world.systems), 60, true, self.config)?;
        self.world.systems.strings.insert(60, value);
        Ok(())
    }
    pub fn number(&self, encoded: i32) -> Result<i32> {
        if encoded < 1_000_000 {
            return Ok(encoded);
        }
        if crate::native_values::DatabaseAddress::decode(encoded).is_some() {
            return self.world.databases.number(encoded);
        }
        let var = wolf_ir::contextual_address(encoded, &self.instruction.source)?;
        number(self.world, self.owner, &self.frame.locals, &var)
    }
    pub fn set_number(&mut self, encoded: i32, value: i32) -> Result<()> {
        let var = wolf_ir::contextual_address(encoded, &self.instruction.source)?;
        check_write(
            self.world,
            self.owner,
            &self.frame.locals,
            &var,
            false,
            self.config,
        )?;
        write_number(self.world, self.owner, &mut self.frame.locals, &var, value)?;
        Ok(())
    }
    pub fn text(&self, encoded: i32) -> Result<String> {
        let var = self.string_address(encoded)?;
        read_text(self.world, self.owner, &self.frame.locals, &Text::Var(var))
    }
    pub fn set_text(&mut self, encoded: i32, value: String) -> Result<()> {
        if value.len() > self.config.max_string_bytes {
            return Err("string byte limit".into());
        }
        let var = self.string_address(encoded)?;
        check_write(
            self.world,
            self.owner,
            &self.frame.locals,
            &var,
            true,
            self.config,
        )?;
        slots_mut(self.world, self.owner, &mut self.frame.locals, &var)?
            .strings
            .insert(var_id(&var), value);
        Ok(())
    }
    fn string_address(&self, n: i32) -> Result<Var> {
        match n {
            1_600_005..=1_600_009 if self.instruction.source.page.is_none() => Ok(Var::Common(
                self.instruction.source.event as i32 * 100 + n % 100,
            )),
            15_000_000..=15_999_999 if (5..=9).contains(&(n % 100)) => {
                Ok(Var::Common(n - 15_000_000))
            }
            3_000_000..=3_999_999 => Ok(Var::Global(n - 3_000_000)),
            9_900_000..=9_999_999 => Ok(Var::System(n - 9_900_000)),
            _ => Err(format!("unsupported string address {n}")),
        }
    }
    pub fn expand(&self, text: &str) -> Result<String> {
        crate::native_values::expand_native_text(
            text,
            self.instruction
                .source
                .page
                .is_none()
                .then_some(self.instruction.source.event),
            self.config.max_string_bytes,
            |n| {
                if (3_000_000..4_000_000).contains(&n)
                    || (9_900_000..10_000_000).contains(&n)
                    || (15_000_000..16_000_000).contains(&n) && (5..=9).contains(&(n % 100))
                {
                    self.text(n).map(crate::native_values::NativeValue::Text)
                } else {
                    self.number(n)
                        .map(crate::native_values::NativeValue::Number)
                }
            },
        )
    }
}
impl Vm {
    pub fn initialize_databases(
        &mut self,
        databases: crate::native_values::NativeDatabases,
    ) -> Result<()> {
        if self.state.tick != 0 {
            return Err("databases must be initialized before the first tick".into());
        }
        self.state.world.databases = databases;
        Ok(())
    }
    pub fn new(library: Library, config: Config, session: u64, seed: u32) -> Result<Self> {
        config.validate()?;
        let state = State {
            version: SAVE_VERSION,
            library: library.identity().into(),
            config,
            tick: 0,
            rng: seed.max(1),
            native_rng: seed,
            world: World::default(),
            keyboard: KeyboardState::default(),
            fibers: BTreeMap::new(),
            session,
            next_fiber: 1,
            next_token: 1,
            cursor: None,
            requests: BTreeMap::new(),
            reserved: Vec::new(),
            native_pending_common: None,
            native_common_fiber: None,
            native_pending_maps: BTreeSet::new(),
            exhausted_ticks: 0,
        };
        Ok(Self {
            library,
            state,
            execution_invalidated: false,
        })
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    /// Host-owned simulation updates (such as map movement routes) share the
    /// VM's variable storage at logical tick boundaries.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.state.world
    }
    pub fn world_and_native_rng_mut(&mut self) -> (&mut World, &mut u32) {
        (&mut self.state.world, &mut self.state.native_rng)
    }
    pub fn library(&self) -> &Library {
        &self.library
    }
    pub fn session(&self) -> u64 {
        self.state.session
    }
    pub fn config(&self) -> &Config {
        &self.state.config
    }
    pub fn set_native_main_fiber(&mut self, id: Option<u64>) -> Result<()> {
        if id.is_some_and(|id| !self.state.fibers.contains_key(&id)) {
            return Err("native main fiber does not exist".into());
        }
        self.state.config.native_main_fiber = id;
        let active = id.is_some_and(|id| self.state.fibers.get(&id)
            .is_some_and(|f| !matches!(f.wait, Wait::Done | Wait::Fault)));
        self.state.world.systems.numbers.insert(13, i32::from(active));
        Ok(())
    }
    /// A05DB0 consumes the last 211 common reservation before map triggers.
    /// This slot is distinct from the generic asynchronous Reserve operation.
    pub fn start_native_reserved_main(&mut self) -> Result<bool> {
        if self.state.config.native_main_fiber.is_some_and(|id| self.state.fibers.get(&id)
            .is_some_and(|f| !matches!(f.wait, Wait::Done | Wait::Fault))) {
            return Ok(false);
        }
        let Some(common) = self.state.native_pending_common else { return Ok(false); };
        let target = Target::Id(format!("common/{common}"));
        let id = if let Some(id) = self.state.native_common_fiber.filter(|id| self.state.fibers.contains_key(id)) {
            self.replace_page(id, target)?;
            id
        } else {
            let id = self.spawn(target, u64::MAX, vec![])?;
            self.state.native_common_fiber = Some(id);
            id
        };
        self.state.native_pending_common = None;
        self.state.config.native_main_fiber = Some(id);
        self.state.world.systems.numbers.insert(13, 1);
        Ok(true)
    }
    /// The map host applies these flags to the currently selected command page.
    pub fn take_native_map_reservations(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.state.native_pending_maps).into_iter()
            .filter_map(|(map,id)| (map == self.state.world.current_map).then_some(id)).collect()
    }
    /// Commit an execution replacement without resetting monotonic fiber/token identities.
    /// World values and input state are replaced separately by the owning host transaction.
    pub fn clear_execution(&mut self) {
        self.state.fibers.clear();
        self.state.requests.clear();
        self.state.reserved.clear();
        self.state.native_pending_common = None;
        self.state.native_common_fiber = None;
        self.state.native_pending_maps.clear();
        self.state.cursor = None;
        self.state.exhausted_ticks = 0;
        self.state.config.native_main_fiber = None;
        self.execution_invalidated = false;
    }
    /// Replace validated snapshot state only after old execution has been cleared.
    pub fn replace_world_for_restore(&mut self, world: World, native_rng: u32) -> Result<()> {
        if !self.state.fibers.is_empty()
            || !self.state.requests.is_empty()
            || !self.state.reserved.is_empty()
            || self.state.native_pending_common.is_some()
            || !self.state.native_pending_maps.is_empty()
            || self.execution_invalidated
        {
            return Err("clear execution before restoring the world".into());
        }
        self.replace_saved_world(world)?;
        self.state.native_rng = native_rng;
        Ok(())
    }
    /// Apply a separately validated world to an already restored VM snapshot.
    /// Keeps that snapshot's execution tree, waits and outstanding service tokens.
    pub fn replace_saved_world(&mut self, mut world: World) -> Result<()> {
        if self.execution_invalidated {
            return Err("cannot patch an uncommitted execution replacement".into());
        }
        world.databases.validate()?;
        world.validate_maps()?;
        check_slots(&world.globals, &self.state.config)?;
        check_slots(&world.commons, &self.state.config)?;
        check_slots(&world.systems, &self.state.config)?;
        if world.event_owner_count() > self.state.config.max_event_owners {
            return Err("restored event owner limit".into());
        }
        for slots in world.events.values().chain(world.map_events.values().flat_map(|bank| bank.values())) {
            check_slots(slots, &self.state.config)?;
        }
        world.event_context = None;
        self.state.world = world;
        Ok(())
    }
    /// Publish once before tick. History advances once per tick, never per fiber or window event.
    pub fn set_keyboard(&mut self, sample: KeyboardSample) {
        // 9EC4AE / 9EC678: no pad returns empty; an unclassified connected
        // controller returns UNKNOWN. WinMM has no native DirectInput type ID.
        self.state.world.systems.strings.insert(71,
            if sample.pad_connected { "UNKNOWN" } else { "" }.into());
        self.state.keyboard.sample = sample;
        self.state.keyboard.policy.simulated = 0;
    }
    pub fn input_policy(&self) -> crate::input::InputPolicy {
        self.state.keyboard.policy
    }
    /// Public native key codes; zero means this host has not bound the slot.
    pub fn input_bindings(&self) -> ([i32; 3], [i32; 3]) {
        let keys = |base| {
            std::array::from_fn(|i| {
                self.state
                    .world
                    .systems
                    .numbers
                    .get(&(base + i as i32))
                    .copied()
                    .unwrap_or(0)
            })
        };
        (keys(52), keys(55))
    }
    /// Explicit tick-boundary inputs; callers record these with replay inputs.
    pub fn set_system(&mut self, key: i32, value: i32) -> Result<()> {
        check_slot_write(
            Some(&self.state.world.systems),
            key,
            false,
            &self.state.config,
        )?;
        self.state.world.systems.numbers.insert(key, value);
        Ok(())
    }
    /// Platform-provided system strings, separate from script setter permissions.
    pub fn set_system_text(&mut self, key: i32, value: String) -> Result<()> {
        if value.len() > self.state.config.max_string_bytes {
            return Err("system string byte limit".into());
        }
        check_slot_write(
            Some(&self.state.world.systems),
            key,
            true,
            &self.state.config,
        )?;
        self.state.world.systems.strings.insert(key, value);
        Ok(())
    }
    /// Absolute numeric getter for scheduler conditions (common-self is relocated by caller).
    pub fn native_number(&self, encoded: i32) -> Result<i32> {
        if encoded < 1_000_000 {
            return Ok(encoded);
        }
        // 9440C0 relocates page-self operands before calling 9E49E0.
        // This is an absolute event/slot address, unlike contextual 1_100_000.
        if (1_000_000..1_100_000).contains(&encoded) {
            let relative = encoded - 1_000_000;
            return number(
                &self.state.world,
                (relative / 10) as u64,
                &Slots::default(),
                &Var::Event(relative % 10),
            );
        }
        if crate::native_values::DatabaseAddress::decode(encoded).is_some() {
            return self.state.world.databases.number(encoded);
        }
        number(
            &self.state.world,
            0,
            &Slots::default(),
            &wolf_ir::address(encoded)?,
        )
    }
    pub fn set_global(&mut self, key: i32, value: i32) -> Result<()> {
        check_slot_write(
            Some(&self.state.world.globals),
            key,
            false,
            &self.state.config,
        )?;
        self.state.world.globals.numbers.insert(key, value);
        Ok(())
    }
    pub fn requests(&self) -> impl Iterator<Item = &Request> {
        self.state.requests.values()
    }
    pub fn spawn(&mut self, target: Target, owner: u64, args: Vec<i32>) -> Result<u64> {
        let id = self
            .library
            .resolve(&target)
            .ok_or("unknown/ambiguous event target")?
            .id
            .clone();
        self.spawn_id(id, owner, args)
    }
    fn spawn_id(&mut self, program: String, owner: u64, args: Vec<i32>) -> Result<u64> {
        if self.state.fibers.len() + self.state.reserved.len() >= self.state.config.max_fibers {
            return Err("fiber limit (reap completed fibers explicitly)".into());
        }
        if args.len() > MAX_ARGUMENTS || args.len() > self.state.config.max_slots {
            return Err("call argument limit".into());
        }
        self.state
            .next_fiber
            .checked_add(self.state.reserved.len() as u64 + 1)
            .ok_or("fiber ID exhausted")?;
        Ok(self.insert_fiber(program, owner, args))
    }
    /// Callers reserve capacity and IDs before changing any state.
    fn insert_fiber(&mut self, program: String, owner: u64, args: Vec<i32>) -> u64 {
        let id = self.state.next_fiber;
        self.state.next_fiber += 1;
        self.state.fibers.insert(
            id,
            Fiber {
                id,
                owner,
                map_id: self.state.world.current_map,
                page_generation: 0,
                frames: vec![frame(program, args)],
                wait: Wait::Ready,
                fault: None,
            },
        );
        id
    }
    pub fn reap(&mut self) {
        self.state.fibers.retain(|_, f| f.wait != Wait::Done);
        if self.state.native_common_fiber.is_some_and(|id| !self.state.fibers.contains_key(&id)) {
            self.state.native_common_fiber = None;
        }
    }
    pub fn cancel(&mut self, id: u64) -> Result<()> {
        let f = self.state.fibers.get_mut(&id).ok_or("unknown fiber")?;
        f.page_generation = f
            .page_generation
            .checked_add(1)
            .ok_or("page generation exhausted")?;
        f.wait = Wait::Done;
        f.frames.clear();
        f.fault = None;
        self.state.requests.retain(|_, r| r.fiber != id);
        Ok(())
    }
    pub fn replace_page(&mut self, id: u64, target: Target) -> Result<()> {
        let program = self
            .library
            .resolve(&target)
            .ok_or("unknown page program")?
            .id
            .clone();
        self.cancel(id)?;
        let f = self.state.fibers.get_mut(&id).unwrap();
        f.frames = vec![frame(program, vec![])];
        f.map_id = self.state.world.current_map;
        f.wait = Wait::Ready;
        f.fault = None;
        Ok(())
    }
    pub fn tick(&mut self, completions: Vec<Completion>) -> Result<TickReport> {
        self.tick_with_host(completions, &mut |_| {
            Err("native host service is not connected".into())
        })
    }
    pub fn tick_with_host(
        &mut self,
        mut completions: Vec<Completion>,
        host: &mut dyn FnMut(&mut NativeContext<'_>) -> Result<bool>,
    ) -> Result<TickReport> {
        if self.execution_invalidated {
            return Err("host execution replacement has not been committed".into());
        }
        // Every tick-level rejection is checked before advancing time or applying responses.
        // Instruction-level failures are diagnostics on that fiber and do not abort the tick.
        let next_tick = self.state.tick.checked_add(1).ok_or("tick exhausted")?;
        if completions.len() > self.state.config.max_completions_per_tick {
            return Err("completion batch limit".into());
        }
        if self.state.fibers.len() + self.state.reserved.len() > self.state.config.max_fibers {
            return Err("reserved fiber limit".into());
        }
        self.state
            .next_fiber
            .checked_add(self.state.reserved.len() as u64)
            .ok_or("fiber ID exhausted")?;
        self.state.tick = next_tick;
        let mut report = TickReport {
            tick: self.state.tick,
            ..Default::default()
        };
        completions.sort_by_key(|c| c.token);
        let mut duplicates = BTreeSet::new();
        let mut prev = None;
        for c in &completions {
            if prev == Some(c.token) {
                duplicates.insert(c.token);
            }
            prev = Some(c.token);
        }
        for c in completions {
            if matches!(&c.response, Response::Failed(message) if message.len() > self.state.config.max_string_bytes)
            {
                report.rejected_completions += 1;
                continue;
            }
            if duplicates.contains(&c.token) {
                report.rejected_completions += 1;
                continue;
            }
            let Some(r) = self
                .state
                .requests
                .get(&c.token.sequence)
                .cloned()
                .filter(|r| r.token == c.token)
            else {
                report.stale_completions += 1;
                continue;
            };
            let Some(f) = self.state.fibers.get_mut(&r.fiber).filter(|f| {
                f.wait == Wait::Service(r.token) && f.page_generation == r.page_generation
            }) else {
                report.stale_completions += 1;
                continue;
            };
            let target = match (&r.service, &c.response) {
                (_, Response::Failed(message)) => {
                    let d = Diagnostic {
                        tick: self.state.tick,
                        fiber: f.id,
                        source: Some(r.source.clone()),
                        message: format!("service failed: {message}"),
                    };
                    f.fault = Some(d.clone());
                    f.wait = Wait::Fault;
                    report.diagnostics.push(d);
                    self.state.requests.remove(&c.token.sequence);
                    continue;
                }
                (Service::Choice { targets, .. }, Response::Choice(n)) => match targets.get(*n) {
                    Some(p) => Some(*p),
                    None => {
                        report.rejected_completions += 1;
                        continue;
                    }
                },
                (
                    Service::Choice {
                        cancel: Some(p), ..
                    },
                    Response::Cancel,
                ) => Some(*p),
                (Service::Choice { .. }, _) => {
                    report.rejected_completions += 1;
                    continue;
                }
                (_, Response::Done) => None,
                _ => {
                    report.rejected_completions += 1;
                    continue;
                }
            };
            if let Some(p) = target {
                // Valid states always have a frame here; reject rather than partially
                // aborting a tick if a future state transition violates that invariant.
                let Some(frame) = f.frames.last_mut() else {
                    report.rejected_completions += 1;
                    continue;
                };
                jump(frame, p);
            }
            f.wait = Wait::Ready;
            self.state.requests.remove(&c.token.sequence);
        }
        let reserved = std::mem::take(&mut self.state.reserved);
        for r in reserved {
            let id = self.insert_fiber(r.target, r.owner, r.args);
            self.state.fibers.get_mut(&id).unwrap().map_id = r.map_id;
        }
        self.start_native_reserved_main()?;
        for f in self.state.fibers.values_mut() {
            if matches!(f.wait, Wait::Ticks(t) if t <= self.state.tick) {
                f.wait = Wait::Ready;
            }
        }
        // A06570 returns after entering the input-poll path, even when it clears the wait.
        // Such fibers must not join this pass's runnable set.
        let polled: BTreeSet<_> = self
            .state
            .fibers
            .iter()
            .filter_map(|(id, f)| {
                matches!(f.wait, Wait::Keyboard { .. } | Wait::NativeInput { .. }).then_some(*id)
            })
            .collect();
        for id in &polled {
            let mut f = self.state.fibers.remove(id).unwrap();
            self.state.world.event_context = self.frame_map(&f);
            let result = self.poll_keyboard(&mut f);
            self.state.world.event_context = None;
            if let Err(message) = result {
                let source = f.frames.last().and_then(|frame| {
                    self.library
                        .get(&frame.program)?
                        .code
                        .get(frame.pc.checked_sub(1)?)
                        .map(|ins| ins.source.clone())
                });
                let d = Diagnostic {
                    tick: self.state.tick,
                    fiber: *id,
                    source,
                    message,
                };
                f.wait = Wait::Fault;
                f.fault = Some(d.clone());
                report.diagnostics.push(d);
            }
            self.state.fibers.insert(*id, f);
        }
        // Build the runnable set once per tick rather than allocate/scan all fibers
        // for every instruction. A step only changes its own runnable status.
        let mut ready: BTreeSet<_> = self
            .state
            .fibers
            .iter()
            .filter_map(|(id, f)| (f.wait == Wait::Ready && !polled.contains(id)).then_some(*id))
            .collect();
        while report.instructions < self.state.config.instruction_budget
            && !ready.is_empty()
            && !self.execution_invalidated
        {
            if let Some(main) = self.state.config.native_main_fiber {
                // 15593AC is main context 1558BA4 +808; 97C590 publishes message activity.
                let active = self
                    .state
                    .fibers
                    .get(&main)
                    .is_some_and(|f| !f.frames.is_empty() && f.wait != Wait::Done);
                self.state
                    .world
                    .systems
                    .numbers
                    .insert(13, i32::from(active));
                let message = self
                    .state
                    .requests
                    .values()
                    .any(|r| matches!(r.service, Service::Message { .. }));
                self.state
                    .world
                    .systems
                    .numbers
                    .insert(12, i32::from(message));
            }
            let id = if self.state.config.run_until_yield {
                self.state
                    .config
                    .native_main_fiber
                    .filter(|id| ready.contains(id))
                    .unwrap_or_else(|| *ready.first().unwrap())
            } else {
                self.state
                    .cursor
                    .and_then(|c| ready.range((Excluded(c), Unbounded)).next().copied())
                    .unwrap_or_else(|| *ready.first().unwrap())
            };
            self.state.cursor = Some(id);
            let mut f = self.state.fibers.remove(&id).unwrap();
            let source = self.source(&f);
            self.state.world.event_context = self.frame_map(&f);
            let result = self.step(&mut f, host);
            self.state.world.event_context = None;
            if let Err(message) = result {
                let d = Diagnostic {
                    tick: self.state.tick,
                    fiber: id,
                    source,
                    message,
                };
                f.wait = Wait::Fault;
                f.fault = Some(d.clone());
                report.diagnostics.push(d);
            }
            if f.wait != Wait::Ready {
                ready.remove(&id);
            }
            self.state.fibers.insert(id, f);
            report.instructions += 1;
        }
        report.execution_invalidated = self.execution_invalidated;
        report.budget_exhausted = !self.execution_invalidated
            && report.instructions == self.state.config.instruction_budget
            && !ready.is_empty();
        self.state.exhausted_ticks = if report.budget_exhausted {
            self.state.exhausted_ticks.saturating_add(1)
        } else {
            0
        };
        if self.state.exhausted_ticks == self.state.config.hot_loop_ticks {
            report.diagnostics.push(Diagnostic {
                tick: self.state.tick,
                fiber: self.state.cursor.unwrap_or(0),
                source: None,
                message: "global instruction budget repeatedly exhausted; possible hot loop".into(),
            });
        }
        self.state.keyboard.finish_tick();
        Ok(report)
    }
    fn poll_keyboard(&mut self, f: &mut Fiber) -> Result<()> {
        let (key, mut memory, native) = match f.wait {
            Wait::Keyboard { key, memory } => (key, memory, false),
            Wait::NativeInput { key, memory } => (key, memory, true),
            _ => return Ok(()),
        };
        let frame = f.frames.last_mut().ok_or("input wait without frame")?;
        let ins = self
            .library
            .get(&frame.program)
            .and_then(|p| frame.pc.checked_sub(1).and_then(|pc| p.code.get(pc)))
            .ok_or("input wait without preceding command")?;
        let blocked = self.state.keyboard.sample.blocked;
        let (dst, result) = match &ins.op {
            Op::Keyboard {
                dst,
                mode,
                wait: true,
                ..
            } if !native => (
                dst,
                if blocked {
                    0
                } else {
                    self.state.keyboard.sample(key, *mode, true, &mut memory)
                },
            ),
            Op::NativeInput { dst, flags, .. } if native && flags & 0x80 != 0 => (
                dst,
                if blocked {
                    0
                } else {
                    self.state
                        .keyboard
                        .sample_command(*flags, key, true, &mut memory)
                },
            ),
            _ => return Err("input wait does not match preceding command".into()),
        };
        if blocked || result != 0 {
            check_write(
                &self.state.world,
                f.owner,
                &frame.locals,
                dst,
                false,
                &self.state.config,
            )?;
            slots_mut(&mut self.state.world, f.owner, &mut frame.locals, dst)?
                .numbers
                .insert(var_id(dst), result);
        }
        f.wait = if !blocked && result != 0 {
            Wait::Ready
        } else if native {
            Wait::NativeInput { key, memory }
        } else {
            Wait::Keyboard { key, memory }
        };
        Ok(())
    }
    fn source(&self, f: &Fiber) -> Option<Source> {
        let frame = f.frames.last()?;
        Some(
            self.library
                .get(&frame.program)?
                .code
                .get(frame.pc)?
                .source
                .clone(),
        )
    }
    fn frame_map(&self, f: &Fiber) -> Option<i32> {
        let frame = f.frames.last()?;
        let program = self.library.get(&frame.program)?;
        // A058AB/A0560F: this game's >=317 profile binds a map frame to
        // its creation map; a common frame uses the currently loaded map.
        program.code.first().filter(|ins| ins.source.page.is_some()).and(f.map_id)
    }
    fn return_frame(&mut self, f: &mut Fiber) -> Result<()> {
        // A06B81..A06C01 reads the callee's persistent return slot only after
        // normal completion. Cancellation/restore simply discard the frames.
        if f.frames.len() >= 2 {
            let child = f.frames.last().unwrap();
            let callee_id = child
                .program
                .strip_prefix("common/")
                .and_then(|id| id.parse::<u32>().ok());
            let parent_index = f.frames.len() - 2;
            let parent = &mut f.frames[parent_index];
            let call = self
                .library
                .get(&parent.program)
                .and_then(|p| parent.pc.checked_sub(1).and_then(|pc| p.code.get(pc)));
            if let Some(ins) = call {
                self.state.world.event_context = ins.source.page.and(f.map_id);
                if let Op::NativeCommonCall {
                    output: Some(encoded),
                    ..
                } = &ins.op
                {
                    let callee = callee_id.ok_or("returning frame lacks common identity")?;
                    let slot = *self
                        .state
                        .config
                        .native_common_returns
                        .get(&callee)
                        .ok_or("native common return metadata is unbound")?;
                    if slot >= 0 && *encoded != 0 {
                        let src = Var::Common(callee as i32 * 100 + slot);
                        let is_string = (5..=9).contains(&slot);
                        let dst = if is_string {
                            wolf_ir::native_startup::stored_text_address(*encoded, &ins.source)?
                        } else {
                            wolf_ir::contextual_address(*encoded, &ins.source)?
                        };
                        check_write(
                            &self.state.world,
                            f.owner,
                            &parent.locals,
                            &dst,
                            is_string,
                            &self.state.config,
                        )?;
                        if is_string {
                            let text = read_text(
                                &self.state.world,
                                f.owner,
                                &parent.locals,
                                &Text::Var(src),
                            )?;
                            slots_mut(&mut self.state.world, f.owner, &mut parent.locals, &dst)?
                                .strings
                                .insert(var_id(&dst), text);
                        } else {
                            let value = number(&self.state.world, f.owner, &parent.locals, &src)?;
                            write_number(
                                &mut self.state.world,
                                f.owner,
                                &mut parent.locals,
                                &dst,
                                value,
                            )?;
                        }
                    }
                }
            }
        }
        f.frames.pop();
        if f.frames.is_empty() {
            f.wait = Wait::Done;
        }
        Ok(())
    }
    fn step(
        &mut self,
        f: &mut Fiber,
        host: &mut dyn FnMut(&mut NativeContext<'_>) -> Result<bool>,
    ) -> Result<()> {
        let Some(current) = f.frames.last() else {
            f.wait = Wait::Done;
            return Ok(());
        };
        let program = self
            .library
            .get(&current.program)
            .ok_or("missing program")?;
        if current.pc == program.code.len() {
            self.return_frame(f)?;
            return Ok(());
        }
        let mut ins = program
            .code
            .get(current.pc)
            .ok_or("PC outside program")?
            .clone();
        let pc = current.pc;
        let frame = f.frames.last_mut().unwrap();
        let read = |v| read_value(&self.state.world, f.owner, &frame.locals, v, 0);
        if let Op::ComputeIndirect {
            pointer,
            lhs,
            binary,
            rhs,
            update,
            arithmetic,
        } = &ins.op
        {
            // A0EE79/A0EF5A and A0F2EE..A0F308 resolve the destination
            // through the current event's address context before committing.
            let encoded = read_value(&self.state.world, f.owner, &frame.locals, pointer, 0)?;
            let dst = wolf_ir::contextual_address(encoded, &ins.source)?;
            ins.op = Op::Compute {
                dst,
                lhs: lhs.clone(),
                binary: *binary,
                rhs: rhs.clone(),
                update: *update,
                arithmetic: *arithmetic,
            };
        }
        match ins.op {
            Op::ComputeIndirect { .. } => {
                return Err("unresolved indirect numeric destination".into())
            }
            Op::Nop => frame.pc += 1,
            Op::NativeHost => {
                let mut context = NativeContext {
                    instruction: &ins,
                    world: &mut self.state.world,
                    owner: f.owner,
                    frame,
                    tick: self.state.tick,
                    native_rng: &mut self.state.native_rng,
                    config: &self.state.config,
                    yield_requested: false,
                    invalidate_requested: false,
                    retry_requested: false,
                    return_requested: false,
                };
                let handled = host(&mut context)?;
                let applied = handled && !context.retry_requested;
                let yield_requested = context.yield_requested;
                let invalidate_requested = context.invalidate_requested;
                let return_requested = context.return_requested;
                if !applied && invalidate_requested {
                    return Err("a retried native instruction cannot invalidate execution".into());
                }
                self.execution_invalidated |= applied && invalidate_requested;
                if applied {
                    frame.pc += 1;
                    if return_requested {
                        self.return_frame(f)?;
                    }
                    if (yield_requested || invalidate_requested) && f.wait != Wait::Done {
                        f.wait = Wait::Ticks(self.state.tick + 1);
                    }
                } else {
                    f.wait = Wait::Ticks(self.state.tick + 1);
                }
            }
            Op::Compute {
                dst,
                lhs,
                binary,
                rhs,
                update,
                arithmetic,
            } => {
                let (lhs, rhs) = (read(&lhs)?, read(&rhs)?);
                let old = if update == Update::Set {
                    0
                } else {
                    number(&self.state.world, f.owner, &frame.locals, &dst)?
                };
                let mut rng = self.state.rng;
                let mut native_rng = self.state.native_rng;
                let result = match arithmetic {
                    Arithmetic::Wrapping => {
                        assign(old, calculate(lhs, rhs, binary, &mut rng)?, update)?
                    }
                    Arithmetic::WolfInteger { clamp_999999 } => {
                        let value = wolf_operand(lhs, rhs, binary, &mut native_rng)?;
                        let result = wolf_assign(old, value, update)?;
                        if clamp_999999 {
                            result.clamp(-999_999, 999_999)
                        } else {
                            result
                        }
                    }
                };
                check_write(
                    &self.state.world,
                    f.owner,
                    &frame.locals,
                    &dst,
                    false,
                    &self.state.config,
                )?;
                write_number(
                    &mut self.state.world,
                    f.owner,
                    &mut frame.locals,
                    &dst,
                    result,
                )?;
                self.state.rng = rng;
                self.state.native_rng = native_rng;
                frame.pc += 1;
            }
            Op::ComputeRange {
                destinations,
                lhs,
                binary,
                rhs,
                update,
                arithmetic,
            } => {
                // A101C4 returns to A0EB50 for each destination: reads must stay
                // inside this loop because the previous destination can alias lhs.
                for dst in destinations {
                    let lhs = read_value(&self.state.world, f.owner, &frame.locals, &lhs, 0)?;
                    let rhs = read_value(&self.state.world, f.owner, &frame.locals, &rhs, 0)?;
                    let old = if update == Update::Set {
                        0
                    } else {
                        number(&self.state.world, f.owner, &frame.locals, &dst)?
                    };
                    let mut rng = self.state.rng;
                    let mut native_rng = self.state.native_rng;
                    let result = match arithmetic {
                        Arithmetic::Wrapping => {
                            assign(old, calculate(lhs, rhs, binary, &mut rng)?, update)?
                        }
                        Arithmetic::WolfInteger { clamp_999999 } => {
                            let value = wolf_operand(lhs, rhs, binary, &mut native_rng)?;
                            let result = wolf_assign(old, value, update)?;
                            if clamp_999999 {
                                result.clamp(-999_999, 999_999)
                            } else {
                                result
                            }
                        }
                    };
                    check_write(
                        &self.state.world,
                        f.owner,
                        &frame.locals,
                        &dst,
                        false,
                        &self.state.config,
                    )?;
                    write_number(
                        &mut self.state.world,
                        f.owner,
                        &mut frame.locals,
                        &dst,
                        result,
                    )?;
                    self.state.rng = rng;
                    self.state.native_rng = native_rng;
                }
                frame.pc += 1;
            }
            Op::SetText { dst, value, append } => {
                let text = read_text(&self.state.world, f.owner, &frame.locals, &value)?;
                let old_len = slots(&self.state.world, f.owner, &frame.locals, &dst)
                    .and_then(|s| s.strings.get(&var_id(&dst)))
                    .map_or(0, String::len);
                if text.len().saturating_add(if append { old_len } else { 0 })
                    > self.state.config.max_string_bytes
                {
                    return Err("string byte limit".into());
                }
                check_write(
                    &self.state.world,
                    f.owner,
                    &frame.locals,
                    &dst,
                    true,
                    &self.state.config,
                )?;
                let slots = slots_mut(&mut self.state.world, f.owner, &mut frame.locals, &dst)?;
                let old = slots.strings.entry(var_id(&dst)).or_default();
                if !append {
                    old.clear();
                }
                old.push_str(&text);
                frame.pc += 1;
            }
            Op::Branch { cases, fallback } => {
                let mut target = fallback;
                for (condition, pc) in cases {
                    if condition_value(
                        &self.state.world,
                        f.owner,
                        &frame.locals,
                        &condition,
                        0,
                        self.state.config.max_string_bytes,
                    )? {
                        target = pc;
                        break;
                    }
                }
                jump(frame, target);
            }
            Op::Jump(pc) => jump(frame, pc),
            Op::NativeLabelJump { target, loops } => {
                frame.loops.retain(|start, _| loops.contains(start));
                for start in loops {
                    let Op::Loop { count, end } = &program.code[start].op else { return Err("invalid label loop".into()); };
                    frame.loops.entry(start).or_insert(LoopState {
                        entered_by_label: true,
                        end: *end, remaining: Some(if count.is_none() { 2_000_000_000 } else { 0 }),
                    });
                }
                frame.pc = target;
            }
            Op::Loop { count, end } => {
                let remaining = count
                    .as_ref()
                    .map(read)
                    .transpose()?
                    .map(|v| v.max(0) as u32);
                if remaining == Some(0) {
                    jump(frame, end);
                } else {
                    frame.loops.insert(pc, LoopState { end, remaining, entered_by_label: false });
                    frame.pc += 1;
                }
            }
            Op::Next { start } => {
                let l = frame
                    .loops
                    .get_mut(&start)
                    .ok_or("loop end reached without active loop")?;
                let repeat = match &mut l.remaining {
                    None => true,
                    Some(n) => {
                        *n = n.saturating_sub(1);
                        *n > 0
                    }
                };
                if repeat {
                    frame.pc = start + 1;
                } else {
                    frame.loops.remove(&start);
                    frame.pc += 1;
                }
            }
            Op::Break { start, end } => {
                if frame.loops.remove(&start).is_none() {
                    return Err("break reached without active loop".into());
                }
                jump(frame, end);
            }
            Op::NativeReserve { target } => {
                let target = read(&target)?;
                if target >= 500_000 {
                    let common = if target >= 600_000 {
                        let count = self.library.native_common_count();
                        if count == 0 { return Err("native reservation has no common events".into()); }
                        let current = if program.code[pc].source.page.is_none() {
                            program.code[pc].source.event as i64
                        } else { -1 };
                        (current + i64::from(target) - 600_100).clamp(0, i64::from(count - 1)) as u32
                    } else { (target % 500_000) as u32 };
                    if self.library.get(&format!("common/{common}")).is_none() {
                        return Err("native reservation references an absent common event".into());
                    }
                    self.state.native_pending_common = Some(common);
                } else if target >= 0 {
                    if self.state.native_pending_maps.len() >= self.state.config.max_event_owners
                        && !self.state.native_pending_maps.contains(&(self.state.world.current_map, target as u32)) {
                        return Err("native map reservation limit".into());
                    }
                    self.state.native_pending_maps.insert((self.state.world.current_map, target as u32));
                }
                frame.pc += 1;
            }
            Op::NativeCommonCall {
                target,
                numbers,
                strings,
                output,
            } => {
                let callee = self
                    .library
                    .resolve_common(&target)
                    .ok_or("unknown/ambiguous common event")?;
                let callee_id = callee
                    .id
                    .strip_prefix("common/")
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|&id| {
                        callee
                            .code
                            .iter()
                            .all(|i| i.source.event == id && i.source.page.is_none())
                    })
                    .ok_or("native call target lacks a common-event context")?;
                let callee_program = callee.id.clone();
                if output.is_some()
                    && !self
                        .state
                        .config
                        .native_common_returns
                        .contains_key(&callee_id)
                {
                    return Err("native common return metadata is unbound".into());
                }
                let numeric_slots = wolf_ir::native_startup::numeric_common_parameter_slots(
                    callee_id,
                    numbers.len(),
                )?;
                let string_slots = wolf_ir::native_startup::string_common_parameter_slots(
                    callee_id,
                    strings.len(),
                )?;
                // Validate the stack limit before mutating arguments. The native
                // parameter loop itself is sequential, including same-common aliases.
                if f.frames.len() >= self.state.config.max_stack {
                    return Err("call stack limit".into());
                }
                let frame = f.frames.last_mut().unwrap();
                for (value, dst) in numbers.iter().zip(numeric_slots.iter()) {
                    let value = read_value(&self.state.world, f.owner, &frame.locals, value, 0)?;
                    check_write(
                        &self.state.world,
                        f.owner,
                        &frame.locals,
                        dst,
                        false,
                        &self.state.config,
                    )?;
                    self.state.world.commons.numbers.insert(var_id(dst), value);
                }
                for (expression, dst) in strings.iter().zip(string_slots.iter()) {
                    let value = read_text(&self.state.world, f.owner, &frame.locals, expression)?;
                    let context = NativeContext {
                        instruction: &program.code[pc],
                        world: &mut self.state.world,
                        owner: f.owner,
                        frame,
                        tick: self.state.tick,
                        native_rng: &mut self.state.native_rng,
                        config: &self.state.config,
                        yield_requested: false,
                        invalidate_requested: false,
                        retry_requested: false,
                        return_requested: false,
                    };
                    let value = context
                        .expand(&value)
                        .map_err(|e| format!("native common string expansion: {e}"))?;
                    if value.len() > self.state.config.max_string_bytes {
                        return Err("string byte limit".into());
                    }
                    check_write(
                        &self.state.world,
                        f.owner,
                        &frame.locals,
                        dst,
                        true,
                        &self.state.config,
                    )?;
                    self.state.world.commons.strings.insert(var_id(dst), value);
                }
                frame.pc += 1;
                f.frames.push(crate_frame(callee_program, vec![]));
            }
            Op::Call { target, args } | Op::Reserve { target, args } => {
                let reserve = matches!(program.code[pc].op, Op::Reserve { .. });
                let program = self
                    .library
                    .resolve(&target)
                    .ok_or("unknown/ambiguous common event")?
                    .id
                    .clone();
                if args.len() > MAX_ARGUMENTS || args.len() > self.state.config.max_slots {
                    return Err("call argument limit".into());
                }
                let args = args.iter().map(read).collect::<Result<Vec<_>>>()?;
                if reserve {
                    if self.state.reserved.len() + self.state.fibers.len() + 1
                        >= self.state.config.max_fibers
                    {
                        return Err("reserved fiber limit".into());
                    }
                    self.state
                        .next_fiber
                        .checked_add(self.state.reserved.len() as u64 + 1)
                        .ok_or("fiber ID exhausted")?;
                    self.state.reserved.push(Reserved {
                        target: program,
                        owner: f.owner,
                        map_id: f.map_id,
                        args,
                    });
                    frame.pc += 1;
                } else {
                    if f.frames.len() >= self.state.config.max_stack {
                        return Err("call stack limit".into());
                    }
                    f.frames.last_mut().unwrap().pc += 1;
                    f.frames.push(crate_frame(program, args));
                }
            }
            Op::Return => {
                self.return_frame(f)?;
            }
            Op::Wait(value) => {
                let ticks = read(&value)?.max(1) as u64;
                let at = self
                    .state
                    .tick
                    .checked_add(ticks)
                    .ok_or("wait tick overflow")?;
                frame.pc += 1;
                f.wait = Wait::Ticks(at);
            }
            Op::NativeWait(value) => {
                let ticks = read(&value)?.max(0) as u64;
                let at = self
                    .state
                    .tick
                    .checked_add(ticks)
                    .ok_or("wait tick overflow")?;
                frame.pc += 1;
                if ticks > 0 {
                    f.wait = Wait::Ticks(at);
                }
            }
            Op::Keyboard {
                dst,
                key,
                mode,
                wait,
            } => {
                let key = read(&key)?;
                check_write(
                    &self.state.world,
                    f.owner,
                    &frame.locals,
                    &dst,
                    false,
                    &self.state.config,
                )?;
                if wait {
                    f.wait = Wait::Keyboard {
                        key,
                        memory: self.state.keyboard.initialize_wait(),
                    };
                } else {
                    let value = if self.state.keyboard.sample.blocked {
                        0
                    } else {
                        self.state.keyboard.sample(key, mode, false, &mut 0)
                    };
                    slots_mut(&mut self.state.world, f.owner, &mut frame.locals, &dst)?
                        .numbers
                        .insert(var_id(&dst), value);
                }
                frame.pc += 1;
            }
            Op::NativeInput { dst, key, flags } => {
                let key = read(&key)?;
                check_write(
                    &self.state.world,
                    f.owner,
                    &frame.locals,
                    &dst,
                    false,
                    &self.state.config,
                )?;
                if flags & 0x80 != 0 {
                    f.wait = Wait::NativeInput {
                        key,
                        memory: self.state.keyboard.initialize_command(flags),
                    };
                } else {
                    let result = if self.state.keyboard.sample.blocked {
                        0
                    } else {
                        self.state
                            .keyboard
                            .sample_command(flags, key, false, &mut 0)
                    };
                    slots_mut(&mut self.state.world, f.owner, &mut frame.locals, &dst)?
                        .numbers
                        .insert(var_id(&dst), result);
                }
                frame.pc += 1;
            }
            Op::InputControl { flags } => {
                self.state.keyboard.policy.update(flags);
                frame.pc += 1;
            }
            Op::SimulateInput { flags } => {
                self.state.keyboard.policy.simulated |= flags as u8;
                frame.pc += 1;
            }
            Op::Request(service) => {
                check_service(&service, &self.state.config)?;
                let sequence = self.state.next_token;
                self.state.next_token = sequence.checked_add(1).ok_or("service token exhausted")?;
                let token = Token {
                    session: self.state.session,
                    sequence,
                };
                self.state.requests.insert(
                    sequence,
                    Request {
                        token,
                        fiber: f.id,
                        page_generation: f.page_generation,
                        source: ins.source,
                        service,
                    },
                );
                frame.pc += 1;
                f.wait = Wait::Service(token);
            }
            Op::Halt => {
                f.frames.clear();
                f.wait = Wait::Done;
            }
            Op::Unsupported { opcode, reason } => {
                return Err(format!("unsupported WOLF opcode {opcode}: {reason}"))
            }
        }
        Ok(())
    }
    pub fn save(&self) -> Result<Vec<u8>> {
        if self.execution_invalidated {
            return Err("cannot save an uncommitted host execution replacement".into());
        }
        let mut writer = LimitedWriter {
            bytes: Vec::new(),
            cap: self.state.config.max_save_bytes,
        };
        serde_json::to_writer(&mut writer, &self.state).map_err(|e| e.to_string())?;
        Ok(writer.bytes)
    }
    /// A new monotonic session generation invalidates every completion from before loading.
    pub fn restore(library: Library, bytes: &[u8], new_session: u64, cap: usize) -> Result<Self> {
        if cap == 0 || bytes.len() > cap || bytes.len() > 256 << 20 {
            return Err("save byte limit".into());
        }
        let mut s: State = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if s.version != SAVE_VERSION || s.library != library.identity() || new_session <= s.session
        {
            return Err("save version/library/session mismatch".into());
        }
        s.config.validate()?;
        s.world.databases.validate()?;
        s.world.validate_maps()?;
        if bytes.len() > s.config.max_save_bytes
            || s.fibers.len() > s.config.max_fibers
            || s.rng == 0
            || s.next_fiber == 0
            || s.next_token == 0
            || s.cursor.is_some_and(|id| id == 0 || id >= s.next_fiber)
        {
            return Err("invalid saved limits/state".into());
        }
        check_slots(&s.world.globals, &s.config)?;
        check_slots(&s.world.commons, &s.config)?;
        check_slots(&s.world.systems, &s.config)?;
        if s.world.event_owner_count() > s.config.max_event_owners {
            return Err("saved event owner limit".into());
        }
        for slots in s.world.events.values().chain(s.world.map_events.values().flat_map(|bank| bank.values())) {
            check_slots(slots, &s.config)?;
        }
        if s.reserved.len() + s.fibers.len() > s.config.max_fibers {
            return Err("saved reservation limit".into());
        }
        if s.native_pending_common.is_some_and(|id| library.get(&format!("common/{id}")).is_none())
            || s.native_common_fiber.is_some_and(|id| !s.fibers.contains_key(&id))
            || s.native_pending_maps.len() > s.config.max_event_owners
            || s.native_pending_maps.iter().any(|(map,id)| map.is_some_and(|map| map < 0) || *id >= 500_000)
        { return Err("invalid native reservation state".into()); }
        s.next_fiber
            .checked_add(s.reserved.len() as u64)
            .ok_or("saved fiber ID exhausted")?;
        for r in &s.reserved {
            if library.get(&r.target).is_none()
                || r.map_id.is_some_and(|map| map < 0)
                || r.args.len() > MAX_ARGUMENTS
                || r.args.len() > s.config.max_slots
            {
                return Err("invalid saved reservation".into());
            }
        }
        for (id, f) in &mut s.fibers {
            if *id == 0
                || f.map_id.is_some_and(|map| map < 0)
                || *id != f.id
                || *id >= s.next_fiber
                || f.frames.len() > s.config.max_stack
                || (f.frames.is_empty() && f.wait != Wait::Done)
                || (f.wait == Wait::Done && !f.frames.is_empty())
                || (f.wait == Wait::Fault) != f.fault.is_some()
                || matches!(f.wait, Wait::Ticks(at) if at <= s.tick)
            {
                return Err("invalid saved fiber".into());
            }
            if let Some(d) = &f.fault {
                if d.fiber != *id
                    || d.tick > s.tick
                    || d.message.len() > s.config.max_string_bytes + 1024
                {
                    return Err("invalid saved diagnostic".into());
                }
            }
            for frame in &f.frames {
                check_slots(&frame.locals, &s.config)?;
                let p = library.get(&frame.program).ok_or("saved program missing")?;
                if frame.pc > p.code.len() {
                    return Err("saved PC out of bounds".into());
                }
                if let Some(target) = frame.native_route_wait {
                    let opcode = p.code.get(frame.pc).and_then(|i| i.raw.as_ref()).map(|r| r.opcode);
                    if !matches!(opcode, Some(201 | 202))
                        || (opcode == Some(202)) != (target == i32::MIN)
                        || target < -2 && target != i32::MIN
                    {
                        return Err("invalid saved native route wait".into());
                    }
                }
                let mut parent_end = p.code.len() + 1;
                for (start, l) in &frame.loops {
                    if !matches!(p.code.get(*start).map(|i| &i.op), Some(Op::Loop { end, count })
                        if *end == l.end && (l.entered_by_label || count.is_some() == l.remaining.is_some()))
                        || l.remaining.is_some_and(|n| (!l.entered_by_label && n == 0) || n > i32::MAX as u32)
                        || frame.pc <= *start
                        || frame.pc >= l.end
                        || l.end >= parent_end
                    {
                        return Err("invalid saved loop".into());
                    }
                    parent_end = l.end;
                }
            }
            for pair in f.frames.windows(2) {
                let caller = &pair[0];
                let call = library
                    .get(&caller.program)
                    .and_then(|p| caller.pc.checked_sub(1).and_then(|pc| p.code.get(pc)));
                let callee = match call.map(|i| &i.op) {
                    Some(Op::Call { target, .. }) => library.resolve(target),
                    // Use exactly the resolver used when pushing this frame.
                    // Map event labels can duplicate a named common event.
                    Some(Op::NativeCommonCall { target, .. }) => library.resolve_common(target),
                    _ => None,
                };
                if callee.is_none_or(|p| p.id != pair[1].program) {
                    return Err(format!("saved call stack does not match program: {} pc {} -> {}", caller.program, caller.pc, pair[1].program));
                }
            }
            if let Wait::Keyboard { key, memory } = f.wait {
                let frame = f.frames.last().ok_or("saved input wait without frame")?;
                let op = library
                    .get(&frame.program)
                    .and_then(|p| frame.pc.checked_sub(1).and_then(|pc| p.code.get(pc)))
                    .map(|i| &i.op);
                if !matches!(op, Some(Op::Keyboard { key: original, wait: true, .. })
                    if !matches!(original, Value::Int(expected) if *expected != key))
                    || !(memory == 0
                        || ((2..=146).contains(&memory)
                            && ![28, 42, 54, 14, 57, 58, 112].contains(&memory)))
                {
                    return Err("invalid saved keyboard wait".into());
                }
            }
            if let Wait::NativeInput { key, memory } = f.wait {
                let frame = f
                    .frames
                    .last()
                    .ok_or("saved native input wait without frame")?;
                let op = library
                    .get(&frame.program)
                    .and_then(|p| frame.pc.checked_sub(1).and_then(|pc| p.code.get(pc)))
                    .map(|i| &i.op);
                if !matches!(op, Some(Op::NativeInput { key: original, flags, .. })
                    if flags & 0x80 != 0 && !matches!(original, Value::Int(expected) if *expected != key)
                    && (if flags & 0xf00 == 0 { memory == 0 } else { (memory as u32 & !0x7fff0 == 0) || (301..=315).contains(&memory) }))
                {
                    return Err("invalid saved native input wait".into());
                }
            }
            if let Wait::Service(t) = &mut f.wait {
                let r = s.requests.get(&t.sequence).ok_or("missing saved request")?;
                if r.token != *t
                    || t.session != s.session
                    || r.fiber != *id
                    || r.page_generation != f.page_generation
                {
                    return Err("saved service ownership mismatch".into());
                }
                t.session = new_session;
            }
        }
        for (seq, r) in &mut s.requests {
            if *seq == 0
                || *seq != r.token.sequence
                || *seq >= s.next_token
                || r.token.session != s.session
            {
                return Err("invalid saved request token".into());
            }
            check_service(&r.service, &s.config)?;
            let f = s.fibers.get(&r.fiber).ok_or("orphan saved request")?;
            if f.wait
                != Wait::Service(Token {
                    session: new_session,
                    sequence: *seq,
                })
            {
                return Err("orphan saved wait".into());
            }
            let frame = f.frames.last().ok_or("empty saved wait stack")?;
            let ins = library
                .get(&frame.program)
                .and_then(|p| frame.pc.checked_sub(1).and_then(|pc| p.code.get(pc)))
                .ok_or("invalid saved service PC")?;
            if !matches!(&ins.op, Op::Request(service) if *service == r.service)
                || ins.source != r.source
            {
                return Err("saved request does not match program".into());
            }
            r.token.session = new_session;
        }
        s.session = new_session;
        Ok(Self {
            library,
            state: s,
            execution_invalidated: false,
        })
    }
    /// Transport generations are excluded; logical request sequence numbers remain hashed.
    pub fn state_hash(&self) -> Result<String> {
        let mut state = self.state.clone();
        state.session = 0;
        for f in state.fibers.values_mut() {
            if let Wait::Service(t) = &mut f.wait {
                t.session = 0;
            }
        }
        for r in state.requests.values_mut() {
            r.token.session = 0;
        }
        Ok(
            blake3::hash(&serde_json::to_vec(&state).map_err(|e| e.to_string())?)
                .to_hex()
                .to_string(),
        )
    }
}
pub(crate) struct LimitedWriter {
    bytes: Vec<u8>,
    cap: usize,
}
impl LimitedWriter {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::new(),
            cap,
        }
    }
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
impl Write for LimitedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.cap.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("save byte limit"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn check_slot_write(slots: Option<&Slots>, key: i32, string: bool, config: &Config) -> Result<()> {
    if !(0..1_000_000).contains(&key) {
        return Err("variable index limit".into());
    }
    let count = slots.map_or(0, |s| s.numbers.len() + s.strings.len());
    let exists = slots.is_some_and(|s| {
        if string {
            s.strings.contains_key(&key)
        } else {
            s.numbers.contains_key(&key)
        }
    });
    if !exists && count >= config.max_slots {
        return Err("variable slot limit".into());
    }
    Ok(())
}
fn check_write(
    w: &World,
    owner: u64,
    local: &Slots,
    var: &Var,
    string: bool,
    config: &Config,
) -> Result<()> {
    if matches!(var, Var::System(_))
        && !(config.run_until_yield
            && ((string && matches!(var, Var::System(67))) || (!string && matches!(
                var,
                Var::System(1 | 2 | 5 | 6 | 9 | 11 | 33 | 36 | 40..=42 | 52..=57 | 100..=102 | 138)
            ))))
    {
        return Err("system variable write requires a platform/profile adapter".into());
    }
    if matches!(var, Var::Event(_))
        && !w.event_bank().is_some_and(|bank| bank.contains_key(&owner))
        && w.event_owner_count() >= config.max_event_owners
    {
        return Err("event owner limit".into());
    }
    if let Var::MapEvent { event, .. } = var {
        if !w.event_bank().is_some_and(|bank| bank.contains_key(&u64::from(*event))) && w.event_owner_count() >= config.max_event_owners {
            return Err("event owner limit".into());
        }
    }
    check_slot_write(slots(w, owner, local, var), var_id(var), string, config)
}
fn check_slots(slots: &Slots, config: &Config) -> Result<()> {
    if slots.numbers.len().saturating_add(slots.strings.len()) > config.max_slots
        || slots
            .numbers
            .keys()
            .chain(slots.strings.keys())
            .any(|key| !(0..1_000_000).contains(key))
        || slots
            .strings
            .values()
            .any(|s| s.len() > config.max_string_bytes)
    {
        return Err("saved variable/string limit".into());
    }
    Ok(())
}
fn check_service(service: &Service, config: &Config) -> Result<()> {
    let text_ok = |s: &str| s.len() <= config.max_string_bytes;
    let valid = match service {
        Service::Message { text } => text_ok(text),
        Service::Choice {
            options, targets, ..
        } => {
            options.len() <= config.max_slots
                && options.len() == targets.len()
                && options.iter().all(|s| text_ok(s))
        }
        Service::Movement { route, .. } => {
            route.len() <= config.max_slots
                && route.iter().all(|(_, args)| args.len() <= MAX_ARGUMENTS)
        }
        Service::Asset { path } => text_ok(path),
        Service::Transition { .. } => true,
    };
    if valid {
        Ok(())
    } else {
        Err("service payload limit".into())
    }
}
fn frame(program: String, args: Vec<i32>) -> Frame {
    Frame {
        program,
        pc: 0,
        loops: BTreeMap::new(),
        native_route_wait: None,
        locals: Slots {
            numbers: args
                .into_iter()
                .enumerate()
                .map(|(i, n)| (i as i32, n))
                .collect(),
            strings: BTreeMap::new(),
        },
    }
}
fn crate_frame(program: String, args: Vec<i32>) -> Frame {
    frame(program, args)
}
fn jump(f: &mut Frame, target: usize) {
    f.loops.retain(|start, l| target > *start && target < l.end);
    f.pc = target;
}
fn var_id(v: &Var) -> i32 {
    match v {
        Var::MapEvent { slot, .. } => *slot,
        Var::Global(n) | Var::Event(n) | Var::Local(n) | Var::Common(n) | Var::System(n) => *n,
    }
}
fn slots<'a>(w: &'a World, owner: u64, locals: &'a Slots, v: &Var) -> Option<&'a Slots> {
    match v {
        Var::Global(_) => Some(&w.globals),
        Var::Event(_) => w.event_bank()?.get(&owner),
        Var::MapEvent { event, .. } => w.event_bank()?.get(&u64::from(*event)),
        Var::Local(_) => Some(locals),
        Var::Common(_) => Some(&w.commons),
        Var::System(_) => Some(&w.systems),
    }
}
fn slots_mut<'a>(
    w: &'a mut World,
    owner: u64,
    locals: &'a mut Slots,
    v: &Var,
) -> Result<&'a mut Slots> {
    if !(0..1_000_000).contains(&var_id(v)) {
        return Err("variable index limit".into());
    }
    match v {
        Var::Global(_) => Ok(&mut w.globals),
        Var::Event(_) => Ok(w.event_bank_mut().entry(owner).or_default()),
        Var::MapEvent { event, .. } => Ok(w.event_bank_mut().entry(u64::from(*event)).or_default()),
        Var::Local(_) => Ok(locals),
        Var::Common(_) => Ok(&mut w.commons),
        Var::System(1 | 2 | 5 | 6 | 9 | 11 | 33 | 36 | 40..=42 | 52..=57 | 67 | 100..=102 | 138) => Ok(&mut w.systems),
        Var::System(_) => Err("system variable write requires a platform/profile adapter".into()),
    }
}
fn number(w: &World, owner: u64, locals: &Slots, v: &Var) -> Result<i32> {
    if !(0..1_000_000).contains(&var_id(v)) {
        return Err("variable index limit".into());
    }
    let result = slots(w, owner, locals, v)
        .and_then(|s| s.numbers.get(&var_id(v)))
        .copied();
    // Native system storage is a zero-initialized array. Only the selectors
    // with a platform side effect need an adapter on writes; reads of a slot
    // that has not been published yet still return the native zero value.
    Ok(result.unwrap_or(0))
}
fn write_number(
    w: &mut World,
    owner: u64,
    locals: &mut Slots,
    var: &Var,
    value: i32,
) -> Result<()> {
    // 9E96BE: only 0/1/2 request a display-mode change; other values return.
    if matches!(var, Var::System(138)) && !(0..=2).contains(&value) {
        return Ok(());
    }
    if let Var::System(slot @ 52..=57) = var {
        let base = if *slot < 55 { 52 } else { 55 };
        // 9BD760: invalid values do not modify bindings in the release engine.
        let valid = if base == 52 {
            (100..=355).contains(&value)
        } else {
            (301..=320).contains(&value)
        };
        if !valid {
            return Ok(());
        }
        let mut keys = [0; 3];
        for (i, key) in keys.iter_mut().enumerate() {
            *key = *w
                .systems
                .numbers
                .get(&(base + i as i32))
                .ok_or("unbound native input binding")?;
        }
        let target = (*slot - base) as usize;
        // Preserve the native ordered collision exchange, including duplicate inputs.
        // Keyboard 9BDA93..9BDB1E and pad 9BD8F8..9BD983 share this algorithm.
        if value == keys[0] && target != 0 {
            keys[0] = keys[target];
        }
        if value == keys[1] && target != 1 {
            keys[1] = keys[if target == 0 { 0 } else { 2 }];
        }
        if value == keys[2] && target != 2 {
            keys[2] = keys[if target == 0 { 0 } else { 1 }];
        }
        keys[target] = value;
        for (i, key) in keys.into_iter().enumerate() {
            w.systems.numbers.insert(base + i as i32, key);
        }
        return Ok(());
    }
    let value = native_numeric_write(var, value)?;
    slots_mut(w, owner, locals, var)?
        .numbers
        .insert(var_id(var), value);
    Ok(())
}
/// Exact direct setters from 9E8230; coordinate -1 needs the display adapter.
fn native_numeric_write(var: &Var, value: i32) -> Result<i32> {
    match var {
        Var::System(1 | 2) if value == -1 => {
            Err("system automatic coordinate needs native display conversion".into())
        }
        Var::System(5 | 6) => Ok(i32::from(value as i16)),
        // 9E85E0 -> 9BE390 writes the clamped mode back to core+0x938.
        Var::System(33) => Ok(value.clamp(0, 2)),
        Var::System(100..=102) => Ok(value.max(0)),
        // 9E863A stores a signed word, read by 9E9E4D.
        Var::System(36 | 40 | 41) => Ok(i32::from(value as i16)),
        // 9E86A4: native text-input font slot, clamped to 0..3.
        Var::System(42) => Ok(value.clamp(0, 3)),
        _ => Ok(value),
    }
}
fn read_value(w: &World, owner: u64, locals: &Slots, v: &Value, depth: usize) -> Result<i32> {
    if depth > 16 {
        return Err("indirection depth limit".into());
    }
    match v {
        Value::Int(n) => Ok(*n),
        Value::Database(n) => w.databases.number(*n),
        Value::Var(v) => number(w, owner, locals, v),
        Value::StringNumber(v) => crate::native_commands::parse_native_integer(&read_text(
            w,
            owner,
            locals,
            &Text::Var(v.clone()),
        )?),
        Value::Indirect(p) => number(
            w,
            owner,
            locals,
            &wolf_ir::address(read_value(w, owner, locals, p, depth + 1)?)?,
        ),
        Value::NativeIndirect { pointer, common_event } => {
            let encoded = read_value(w, owner, locals, pointer, depth + 1)?;
            if encoded < 1_000_000 { return Err("native indirect operand is not a variable address".into()); }
            let resolved = wolf_ir::native_arithmetic_value(encoded, *common_event)?;
            read_value(w, owner, locals, &resolved, depth + 1)
        }
    }
}
fn read_text(w: &World, owner: u64, locals: &Slots, t: &Text) -> Result<String> {
    match t {
        Text::Literal(s) => Ok(s.clone()),
        Text::Var(v) => {
            if !(0..1_000_000).contains(&var_id(v)) {
                return Err("variable index limit".into());
            }
            let text = slots(w, owner, locals, v)
                .and_then(|s| s.strings.get(&var_id(v)))
                .cloned();
            // System string slots are likewise zero/empty initialized until a
            // script or a host setter publishes a value.
            Ok(text.unwrap_or_default())
        }
    }
}
fn compare<T: Ord>(a: T, b: T, c: Compare) -> bool {
    match c {
        Compare::Eq => a == b,
        Compare::Ne => a != b,
        Compare::Lt => a < b,
        Compare::Le => a <= b,
        Compare::Gt => a > b,
        Compare::Ge => a >= b,
    }
}
fn condition_value(
    w: &World,
    owner: u64,
    locals: &Slots,
    c: &Condition,
    depth: usize,
    max_string_bytes: usize,
) -> Result<bool> {
    if depth > 32 {
        return Err("condition nesting limit".into());
    }
    match c {
        Condition::StringCompare {
            lhs,
            cmp,
            rhs,
            common_event,
        } => {
            let lhs = read_text(w, owner, locals, lhs)?;
            let rhs = crate::native_values::expand_plain_text(
                &read_text(w, owner, locals, rhs)?,
                *common_event,
                max_string_bytes,
                |n| {
                    if let Ok(var) = wolf_ir::contextual_string_address(n, *common_event) {
                        read_text(w, owner, locals, &Text::Var(var))
                            .map(crate::native_values::NativeValue::Text)
                    } else {
                        read_value(w, owner, locals, &wolf_ir::value(n)?, 0)
                            .map(crate::native_values::NativeValue::Number)
                    }
                },
            )?;
            Ok(match cmp {
                wolf_ir::StringCompare::Eq => lhs == rhs,
                wolf_ir::StringCompare::Ne => lhs != rhs,
                wolf_ir::StringCompare::Contains => lhs.contains(&rhs),
                wolf_ir::StringCompare::StartsWith => lhs.starts_with(&rhs),
                wolf_ir::StringCompare::NotContains => !lhs.contains(&rhs),
                wolf_ir::StringCompare::EndsWith => lhs.ends_with(&rhs),
            })
        }
        Condition::BitsContain { lhs, rhs } => {
            let lhs = read_value(w, owner, locals, lhs, 0)?;
            let rhs = read_value(w, owner, locals, rhs, 0)?;
            Ok(lhs & rhs == rhs)
        }
        Condition::Number { lhs, cmp, rhs } => Ok(compare(
            read_value(w, owner, locals, lhs, 0)?,
            read_value(w, owner, locals, rhs, 0)?,
            *cmp,
        )),
        Condition::Text { lhs, cmp, rhs } => Ok(compare(
            read_text(w, owner, locals, lhs)?,
            read_text(w, owner, locals, rhs)?,
            *cmp,
        )),
        Condition::All(cases) => {
            for c in cases {
                if !condition_value(w, owner, locals, c, depth + 1, max_string_bytes)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Condition::Any(cases) => {
            for c in cases {
                if condition_value(w, owner, locals, c, depth + 1, max_string_bytes)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}
fn calculate(a: i32, b: i32, op: Binary, rng: &mut u32) -> Result<i32> {
    Ok(match op {
        Binary::Add => a.wrapping_add(b),
        Binary::Sub => a.wrapping_sub(b),
        Binary::Mul => a.wrapping_mul(b),
        Binary::Div => {
            if b == 0 {
                return Err("division by zero".into());
            }
            a.wrapping_div(b)
        }
        Binary::Rem => {
            if b == 0 {
                return Err("remainder by zero".into());
            }
            a.wrapping_rem(b)
        }
        Binary::And => a & b,
        Binary::Or => a | b,
        Binary::Xor => a ^ b,
        Binary::SignedShift => signed_shift(a, b),
        Binary::NativeAngle => native_angle(a, b),
        Binary::ShiftLeft | Binary::ShiftRight => {
            if !(0..32).contains(&b) {
                return Err("shift count outside 0..31".into());
            }
            if op == Binary::ShiftLeft {
                a.wrapping_shl(b as u32)
            } else {
                a >> b
            }
        }
        Binary::RandomInclusive => {
            if a > b {
                return Err("inverted random range".into());
            }
            *rng ^= *rng << 13;
            *rng ^= *rng >> 17;
            *rng ^= *rng << 5;
            (i64::from(a) + (u64::from(*rng) % (i64::from(b) - i64::from(a) + 1) as u64) as i64)
                as i32
        }
    })
}
fn signed_shift(a: i32, b: i32) -> i32 {
    let bits = b.unsigned_abs() & 31;
    if b >= 0 {
        a.wrapping_shl(bits)
    } else {
        a >> bits
    }
}
// Binary64 constants read directly from the target's CA7F28/CA85A0/CA83D8.
#[allow(clippy::approx_constant)]
const NATIVE_PI: f64 = 3.1415926535;
fn native_angle(x: i32, y: i32) -> i32 {
    (3600 - (f64::from(y).atan2(f64::from(x)) / -NATIVE_PI * 1800.0) as i32) % 3600
}
fn native_function(n: i32, op: Update) -> i32 {
    let value = match op {
        Update::NativeSine => (f64::from(n) * NATIVE_PI / 1800.0).sin() * 1000.0,
        Update::NativeCosine => (f64::from(n) * NATIVE_PI / 1800.0).cos() * 1000.0,
        Update::NativeSqrt => f64::from(n.max(0)).sqrt() * 1000.0,
        _ => unreachable!(),
    };
    value as i32
}
// Ordinary integer handler 0xa0e590, tables at 0xa10f60/0xa10f88.
// Floating-point modes remain outside this evidence profile.
fn wolf_operand(a: i32, b: i32, op: Binary, rng: &mut u32) -> Result<i32> {
    if op != Binary::RandomInclusive {
        return wolf_calculate(a, b, op);
    }
    let next = native_random(rng)?;
    // A0F554..A0F588 uses 32-bit SUB/ABS/INC, unsigned DIV, signed MIN and ADD.
    let width = a.wrapping_sub(b).wrapping_abs().wrapping_add(1) as u32;
    Ok(a.min(b).wrapping_add((next % width) as i32))
}
/// Original 942820 generator shared by VM arithmetic and actor routes.
pub fn native_random(rng: &mut u32) -> Result<u32> {
    // 942857..9428B4: unsigned multiply, positive signed-i64 division, reject >2e9.
    // Native zero falls through to GetTickCount. A deterministic VM has no implicit
    // wall clock: report that boundary instead of inventing a replacement seed.
    let mut next = *rng;
    loop {
        next = ((u64::from(next) * 16_807) % 2_147_483_647) as u32;
        if next == 0 {
            return Err("native RNG requires GetTickCount reseed for zero state".into());
        }
        if next <= 2_000_000_000 {
            break;
        }
    }
    *rng = next;
    Ok(next)
}
fn wolf_calculate(a: i32, b: i32, op: Binary) -> Result<i32> {
    let cap = |n: i64| n.clamp(-2_000_000_000, 2_000_000_000) as i32;
    Ok(match op {
        Binary::Add => cap(i64::from(a) + i64::from(b)),
        Binary::Sub => cap(i64::from(a) - i64::from(b)),
        Binary::Mul => cap(i64::from(a) * i64::from(b)),
        Binary::Div => a
            .checked_div(if b == 0 { 1 } else { b })
            .ok_or("native signed division overflow")?,
        Binary::Rem => a
            .checked_rem(if b == 0 { 1 } else { b })
            .ok_or("native signed remainder overflow")?,
        Binary::And => a & b,
        Binary::Or => a | b,
        Binary::Xor => a ^ b,
        Binary::SignedShift => signed_shift(a, b),
        Binary::NativeAngle => native_angle(a, b),
        _ => return Err("arithmetic operation outside WOLF integer evidence profile".into()),
    })
}
fn wolf_assign(old: i32, n: i32, op: Update) -> Result<i32> {
    match op {
        Update::Set => Ok(n),
        Update::Max => Ok(old.max(n)),
        Update::Min => Ok(old.min(n)),
        Update::Abs => Ok(n.wrapping_abs()),
        Update::NativeSine | Update::NativeCosine | Update::NativeSqrt => Ok(native_function(n, op)),
        _ => wolf_calculate(
            old,
            n,
            match op {
                Update::Add => Binary::Add,
                Update::Sub => Binary::Sub,
                Update::Mul => Binary::Mul,
                Update::Div => Binary::Div,
                Update::Rem => Binary::Rem,
                _ => unreachable!(),
            },
        ),
    }
}
fn assign(old: i32, n: i32, op: Update) -> Result<i32> {
    match op {
        Update::Set => Ok(n),
        Update::Min => Ok(old.min(n)),
        Update::Max => Ok(old.max(n)),
        Update::Abs => Ok(n.wrapping_abs()),
        Update::NativeSine | Update::NativeCosine | Update::NativeSqrt => Ok(native_function(n, op)),
        _ => calculate(
            old,
            n,
            match op {
                Update::Add => Binary::Add,
                Update::Sub => Binary::Sub,
                Update::Mul => Binary::Mul,
                Update::Div => Binary::Div,
                Update::Rem => Binary::Rem,
                _ => unreachable!(),
            },
            &mut 1,
        ),
    }
}

/// Integer fixed clock; preserves backlog instead of dropping simulation ticks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixedClock {
    hz: u32,
    accumulator: u128,
}
impl FixedClock {
    pub fn new(hz: u32) -> Result<Self> {
        if hz == 0 {
            Err("zero tick rate".into())
        } else {
            Ok(Self { hz, accumulator: 0 })
        }
    }
    pub fn advance(&mut self, elapsed_ns: u64, max_ticks: u32) -> Result<(u32, u128)> {
        if self.hz == 0 {
            return Err("zero tick rate".into());
        }
        self.accumulator = self
            .accumulator
            .checked_add(u128::from(elapsed_ns) * u128::from(self.hz))
            .ok_or("clock overflow")?;
        let ticks = (self.accumulator / 1_000_000_000).min(u128::from(max_ticks)) as u32;
        self.accumulator -= u128::from(ticks) * 1_000_000_000;
        Ok((ticks, self.accumulator / 1_000_000_000))
    }
}
