//! Versioned semantic IR. `compile` is a conservative, explicitly provisional WOLF adapter.
mod compile;
pub mod native_input;
pub mod native_startup;
pub mod startup;
pub use compile::{compile, Coverage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wolf_format::RawCommand;
pub const PROFILE: &str = "wolf350-evidence-subset-v12";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub asset: String,
    pub content_blake3: String,
    pub event: u32,
    pub page: Option<u32>,
    pub command: usize,
    pub span: [usize; 2],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Var {
    Global(i32),
    Event(i32),
    /// Absolute event-self slot within the current map (native 1000000 bank).
    MapEvent { event: u32, slot: i32 },
    Local(i32),
    /// Persistent common-event self slot, indexed as common_id * 100 + slot.
    Common(i32),
    System(i32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Value {
    Int(i32),
    /// Decimal database selector; resolved against the runtime's loaded databases.
    Database(i32),
    Var(Var),
    /// Raw 121's string-classified operand: A058D0 followed by BA6B62 (atoi).
    StringNumber(Var),
    Indirect(Box<Value>),
    /// Raw121 bit0x20/0x40: resolve an operand again with native context and atoi.
    NativeIndirect { pointer: Box<Value>, common_event: Option<u32> },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binary {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    RandomInclusive,
    And,
    Or,
    Xor,
    ShiftLeft,
    ShiftRight,
    SignedShift,
    /// Raw121 update9 uses both original operands as atan2(y, x), in tenths of a degree.
    NativeAngle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arithmetic {
    /// Explicit Rust runtime policy for synthetic IR; not an engine compatibility claim.
    Wrapping,
    /// GamePro opcode 121 ordinary integer path; preprocessing fast paths are not proven equivalent.
    WolfInteger { clamp_999999: bool },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Update {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Min,
    Max,
    Abs,
    NativeSine,
    NativeCosine,
    NativeSqrt,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compare {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Text {
    Literal(String),
    Var(Var),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StringCompare {
    Eq,
    Ne,
    Contains,
    StartsWith,
    NotContains,
    EndsWith,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    /// Raw 112: RHS passes through 94FC50 before the byte-string comparison.
    StringCompare {
        lhs: Text,
        cmp: StringCompare,
        rhs: Text,
        common_event: Option<u32>,
    },
    BitsContain {
        lhs: Value,
        rhs: Value,
    },
    Number {
        lhs: Value,
        cmp: Compare,
        rhs: Value,
    },
    Text {
        lhs: Text,
        cmp: Compare,
        rhs: Text,
    },
    All(Vec<Condition>),
    Any(Vec<Condition>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Id(String),
    Name(String),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    Message {
        text: String,
    },
    Choice {
        options: Vec<String>,
        targets: Vec<usize>,
        cancel: Option<usize>,
    },
    Movement {
        actor: u64,
        route: Vec<(u8, Vec<i32>)>,
    },
    Asset {
        path: String,
    },
    Transition {
        style: u32,
        ticks: u32,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Op {
    Nop,
    /// Synchronous native host command. The host must validate its raw parameter form.
    NativeHost,
    Compute {
        dst: Var,
        lhs: Value,
        binary: Binary,
        rhs: Value,
        update: Update,
        arithmetic: Arithmetic,
    },
    /// Raw121 bit0x10 dereferences the destination before evaluating operands.
    ComputeIndirect {
        pointer: Value,
        lhs: Value,
        binary: Binary,
        rhs: Value,
        update: Update,
        arithmetic: Arithmetic,
    },
    /// Native 121 inclusive destination range; operands are re-read after each write.
    ComputeRange {
        destinations: Vec<Var>,
        lhs: Value,
        binary: Binary,
        rhs: Value,
        update: Update,
        arithmetic: Arithmetic,
    },
    SetText {
        dst: Var,
        value: Text,
        append: bool,
    },
    Branch {
        cases: Vec<(Condition, usize)>,
        fallback: usize,
    },
    Jump(usize),
    /// A1E230 rebuilds loop ancestry without re-evaluating entered bounds.
    NativeLabelJump { target: usize, loops: Vec<usize> },
    Loop {
        count: Option<Value>,
        end: usize,
    },
    Next {
        start: usize,
    },
    Break {
        start: usize,
        end: usize,
    },
    Call {
        target: Target,
        args: Vec<Value>,
    },
    /// Native 210/300: sequential writes to the callee's persistent self slots.
    /// String expressions still require the verified plain-text runtime subset.
    NativeCommonCall {
        target: Target,
        numbers: Vec<Value>,
        strings: Vec<Text>,
        output: Option<i32>,
    },
    Reserve {
        target: Target,
        args: Vec<Value>,
    },
    /// GamePro 211: one pending main common, or a current-map event flag.
    NativeReserve { target: Value },
    Return,
    Wait(Value),
    /// Native 180: nonpositive waits do not force a yield. Synthetic Wait keeps its policy.
    NativeWait(Value),
    /// Native 123 keyboard subset, backed by a tick-boundary device snapshot.
    Keyboard {
        dst: Var,
        key: Value,
        mode: u8,
        wait: bool,
    },
    /// Native pad and engine-logical sampling; host publishes native device state.
    NativeInput {
        dst: Var,
        key: Value,
        flags: i32,
    },
    /// Native126 logical controls: bit9 disables event input; bits8/9 disable player controls.
    InputControl {
        flags: i32,
    },
    /// A18190: latch engine logical buttons for the next device update.
    SimulateInput { flags: i32 },
    Request(Service),
    Halt,
    Unsupported {
        opcode: u32,
        reason: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instruction {
    pub source: Source,
    pub raw: Option<RawCommand>,
    pub op: Op,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub code: Vec<Instruction>,
}
impl Program {
    pub fn validate(&self) -> Result<(), String> {
        if self.profile != PROFILE {
            return Err("unsupported IR profile".into());
        }
        if self.code.len() > 1_000_000 {
            return Err("program instruction limit".into());
        }
        let valid = |pc| {
            if pc <= self.code.len() {
                Ok(())
            } else {
                Err(format!("invalid jump target {pc}"))
            }
        };
        let mut budget = 1_000_000usize;
        let mut loop_stack: Vec<usize> = Vec::new();
        for (pc, ins) in self.code.iter().enumerate() {
            while loop_stack.last().is_some_and(|end| *end <= pc) {
                loop_stack.pop();
            }
            if ins.source.span[0] > ins.source.span[1] {
                return Err("invalid source span".into());
            }
            match &ins.op {
                Op::NativeHost => {
                    let raw = ins
                        .raw
                        .as_ref()
                        .ok_or("native host operation requires its raw record")?;
                    if !matches!(
                        raw.opcode,
                        106 | 122
                            | 124
                            | 130
                            | 140
                            | 150
                            | 151
                            | 160
                            | 161
                            | 162
                            | 175
                            | 201
                            | 202
                            | 220
                            | 221
                            | 222
                            | 250
                            | 270
                            | 281
                            | 290
                            | 1000
                    ) || raw.args.len() > 64
                        || raw.strings.len() > 16
                        || (raw.movement.is_some() && raw.opcode != 201)
                        || raw.extension_length != 0
                        || !raw.extension.is_empty()
                    {
                        return Err("invalid native host envelope".into());
                    }
                    for string in &raw.strings {
                        if string.raw.len() > 1 << 20 {
                            return Err("native host string limit".into());
                        }
                        string.text().map_err(|e| e.to_string())?;
                    }
                }
                Op::Compute { dst, lhs, rhs, .. } => {
                    valid_var(dst)?;
                    valid_value(lhs, 0, &mut budget)?;
                    valid_value(rhs, 0, &mut budget)?;
                }
                Op::ComputeIndirect {
                    pointer, lhs, rhs, ..
                } => {
                    valid_value(pointer, 0, &mut budget)?;
                    valid_value(lhs, 0, &mut budget)?;
                    valid_value(rhs, 0, &mut budget)?;
                }
                Op::ComputeRange {
                    destinations,
                    lhs,
                    rhs,
                    ..
                } => {
                    if destinations.is_empty() || destinations.len() > 10_000 {
                        return Err("numeric destination range limit".into());
                    }
                    for dst in destinations {
                        valid_var(dst)?;
                    }
                    valid_value(lhs, 0, &mut budget)?;
                    valid_value(rhs, 0, &mut budget)?;
                }
                Op::SetText { dst, value, .. } => {
                    valid_var(dst)?;
                    valid_text(value)?;
                }
                Op::Jump(p) => valid(*p)?,
                Op::NativeLabelJump { target, loops } => {
                    valid(*target)?;
                    let expected: Vec<_> = self.code.iter().enumerate().filter_map(|(start, ins)| {
                        match ins.op { Op::Loop { end, .. } if start < *target && *target < end => Some(start), _ => None }
                    }).collect();
                    if *loops != expected { return Err("label loop ancestry mismatch".into()); }
                }
                Op::Branch { cases, fallback } => {
                    valid(*fallback)?;
                    for (c, p) in cases {
                        valid_condition(c, 0, &mut budget)?;
                        valid(*p)?;
                    }
                }
                Op::Loop { count, end } => {
                    valid(*end)?;
                    if *end <= pc + 1
                        || !matches!(self.code.get(end - 1).map(|i| &i.op), Some(Op::Next { start }) if *start == pc)
                        || loop_stack.last().is_some_and(|outer_end| end >= outer_end)
                    {
                        return Err("invalid or crossing loop interval".into());
                    }
                    if let Some(v) = count {
                        valid_value(v, 0, &mut budget)?;
                    }
                    loop_stack.push(*end);
                }
                Op::Next { start } | Op::Break { start, .. } => {
                    if *start >= pc
                        || !matches!(self.code.get(*start).map(|i| &i.op), Some(Op::Loop { .. }))
                    {
                        return Err("invalid loop reference".into());
                    }
                    if let Op::Break { end, .. } = ins.op {
                        valid(end)?;
                        if !matches!(self.code[*start].op, Op::Loop { end: target, .. } if target == end && pc < end - 1)
                        {
                            return Err("break exit disagrees with its loop".into());
                        }
                    } else if !matches!(self.code[*start].op, Op::Loop { end, .. } if end == pc + 1)
                    {
                        return Err("Next is not the referenced loop end".into());
                    }
                }
                Op::Call { args, .. } | Op::Reserve { args, .. } => {
                    if args.len() > 100 {
                        return Err("call argument limit".into());
                    }
                    for v in args {
                        valid_value(v, 0, &mut budget)?;
                    }
                }
                Op::NativeCommonCall {
                    numbers, strings, ..
                } => {
                    if numbers.len() > 5 || strings.len() > 5 {
                        return Err("native common parameter limit".into());
                    }
                    for v in numbers {
                        valid_value(v, 0, &mut budget)?;
                    }
                    for t in strings {
                        valid_text(t)?;
                    }
                }
                Op::Wait(v) | Op::NativeWait(v) | Op::NativeReserve { target: v } => valid_value(v, 0, &mut budget)?,
                Op::Keyboard { dst, key, mode, .. } => {
                    valid_var(dst)?;
                    valid_value(key, 0, &mut budget)?;
                    if *mode > 3 || matches!(dst, Var::System(_)) {
                        return Err("unsupported keyboard mode/destination".into());
                    }
                }
                Op::NativeInput { dst, key, flags } => {
                    valid_var(dst)?;
                    valid_value(key, 0, &mut budget)?;
                    native_input::validate_flags(*flags)?;
                    if matches!(dst, Var::System(_)) {
                        return Err("native input system destination is unsupported".into());
                    }
                }
                Op::InputControl { flags } => {
                    if flags & !0x3f7 != 0 {
                        return Err("unsupported input control flags".into());
                    }
                }
                Op::SimulateInput { flags } => {
                    if flags & !0xf7 != 0 {
                        return Err("unsupported simulated input flags".into());
                    }
                }
                Op::Request(Service::Choice {
                    options,
                    targets,
                    cancel,
                }) => {
                    if options.is_empty() || options.len() > 256 || options.len() != targets.len() {
                        return Err("choice target count".into());
                    }
                    if options
                        .iter()
                        .map(String::len)
                        .try_fold(0usize, usize::checked_add)
                        .is_none_or(|n| n > 1 << 20)
                    {
                        return Err("choice text byte limit".into());
                    }
                    for p in targets {
                        valid(*p)?;
                    }
                    if let Some(p) = cancel {
                        valid(*p)?;
                    }
                }
                Op::Request(Service::Message { text })
                | Op::Request(Service::Asset { path: text }) => valid_literal(text)?,
                Op::Request(Service::Movement { route, .. }) => {
                    if route.len() > 100_000 || route.iter().any(|(_, args)| args.len() > 64) {
                        return Err("route command/argument limit".into());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
fn valid_var(v: &Var) -> Result<(), String> {
    if let Var::MapEvent { event, slot } = v {
        return if *event < 10_000 && (0..10).contains(slot) { Ok(()) } else { Err("absolute event address limit".into()) };
    }
    let n = match v { Var::Global(n) | Var::Event(n) | Var::Local(n) | Var::Common(n) | Var::System(n) => n, _ => unreachable!() };
    if !(0..1_000_000).contains(n) {
        return Err("variable index limit".into());
    }
    Ok(())
}
fn valid_literal(s: &str) -> Result<(), String> {
    if s.len() > 1 << 20 {
        return Err("IR string byte limit".into());
    }
    Ok(())
}
fn valid_text(t: &Text) -> Result<(), String> {
    match t {
        Text::Literal(s) => valid_literal(s),
        Text::Var(v) => valid_var(v),
    }
}
fn valid_value(v: &Value, depth: usize, budget: &mut usize) -> Result<(), String> {
    *budget = budget.checked_sub(1).ok_or("expression node limit")?;
    if depth > 16 {
        return Err("indirection depth limit".into());
    }
    match v {
        Value::Int(_) => Ok(()),
        Value::Database(1_000_000_000..=1_199_999_999 | 1_300_000_000..=1_399_999_999) => Ok(()),
        Value::Database(_) => Err("invalid database selector".into()),
        Value::Var(v) | Value::StringNumber(v) => valid_var(v),
        Value::Indirect(v) => valid_value(v, depth + 1, budget),
        Value::NativeIndirect { pointer, common_event } => {
            if common_event.is_some_and(|event| event >= 10_000) { return Err("indirect common context limit".into()); }
            valid_value(pointer, depth + 1, budget)
        }
    }
}
fn valid_condition(c: &Condition, depth: usize, budget: &mut usize) -> Result<(), String> {
    *budget = budget.checked_sub(1).ok_or("expression node limit")?;
    if depth > 32 {
        return Err("condition nesting limit".into());
    }
    match c {
        Condition::Number { lhs, rhs, .. } | Condition::BitsContain { lhs, rhs } => {
            valid_value(lhs, 0, budget)?;
            valid_value(rhs, 0, budget)
        }
        Condition::Text { lhs, rhs, .. } | Condition::StringCompare { lhs, rhs, .. } => {
            valid_text(lhs)?;
            valid_text(rhs)
        }
        Condition::All(cases) | Condition::Any(cases) => {
            for c in cases {
                valid_condition(c, depth + 1, budget)?;
            }
            Ok(())
        }
    }
}
#[derive(Debug, Clone)]
pub struct Library {
    programs: BTreeMap<String, Program>,
    names: BTreeMap<String, String>,
    identity: String,
}
impl Library {
    pub fn new(programs: Vec<Program>) -> Result<Self, String> {
        let mut by_id = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut ambiguous = std::collections::BTreeSet::new();
        for p in programs {
            p.validate()?;
            if !p.name.is_empty() && names.insert(p.name.clone(), p.id.clone()).is_some() {
                ambiguous.insert(p.name.clone());
            }
            if by_id.insert(p.id.clone(), p).is_some() {
                return Err("duplicate program ID".into());
            }
        }
        for name in ambiguous {
            names.remove(&name);
        }
        let identity = blake3::hash(&serde_json::to_vec(&by_id).map_err(|e| e.to_string())?)
            .to_hex()
            .to_string();
        Ok(Self {
            programs: by_id,
            names,
            identity,
        })
    }
    pub fn get(&self, id: &str) -> Option<&Program> {
        self.programs.get(id)
    }
    pub fn resolve(&self, target: &Target) -> Option<&Program> {
        match target {
            Target::Id(id) => self.get(id),
            Target::Name(n) => self.names.get(n).and_then(|id| self.get(id)),
        }
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn native_common_count(&self) -> u32 {
        self.programs.keys().filter_map(|id| id.strip_prefix("common/")?.parse::<u32>().ok())
            .max().map_or(0, |id| id.saturating_add(1))
    }
    /// Native named common calls do not search map event labels.
    pub fn resolve_common(&self, target: &Target) -> Option<&Program> {
        match target {
            Target::Id(id) => self.get(id).filter(|p| p.id.starts_with("common/")),
            Target::Name(name) => {
                let mut matches = self
                    .programs
                    .values()
                    .filter(|p| p.id.starts_with("common/") && p.name == *name);
                let first = matches.next()?;
                if matches.next().is_some() {
                    None
                } else {
                    Some(first)
                }
            }
        }
    }
}
pub fn address(n: i32) -> Result<Var, String> {
    // GamePro 0x9e2260 classifies these as string references. Numeric conversion
    // (including text-to-number) needs its own adapter; missing numbers must not become zero.
    if matches!(n, 1_600_005..=1_600_009 | 9_900_000..=9_999_999)
        || ((9_100_000..=9_199_999).contains(&n) && n % 10 == 9)
    {
        return Err(format!(
            "string-to-number address semantics are unverified: {n}"
        ));
    }
    match n {
        1_000_000..=1_099_999 => Ok(Var::MapEvent { event: ((n - 1_000_000) / 10) as u32, slot: n % 10 }),
        1_100_000..=1_199_999 => Ok(Var::Event(n % 10)),
        1_600_000..=1_699_999 => Err("common self address needs program context".into()),
        2_000_000..=2_999_999 => Ok(Var::Global(n - 2_000_000)),
        9_000_000..=9_099_999 => Ok(Var::System(n - 9_000_000)),
        15_000_000..=15_999_999 if !(5..=9).contains(&(n % 100)) => Ok(Var::Common(n - 15_000_000)),
        _ => Err(format!("unresolved variable address {n}")),
    }
}
pub fn contextual_address(n: i32, source: &Source) -> Result<Var, String> {
    if (1_600_000..=1_699_999).contains(&n) {
        // Numeric/string classification is based on the original address first.
        if (5..=9).contains(&(n % 100)) {
            return Err("common self string conversion is unverified".into());
        }
        if source.page.is_some() {
            return Err("common self address has no common-event context".into());
        }
        let index = source
            .event
            .checked_mul(100)
            .and_then(|x| x.checked_add(n as u32 % 100))
            .filter(|n| *n < 1_000_000)
            .ok_or("common self address limit")?;
        return Ok(Var::Common(index as i32));
    }
    address(n)
}
/// Native arithmetic operands classify strings before numeric resolution.
pub fn native_arithmetic_value(n: i32, common_event: Option<u32>) -> Result<Value, String> {
    if matches!(n, 1_600_005..=1_600_009 | 3_000_000..=3_999_999 | 9_900_000..=9_999_999)
        || ((15_000_000..=15_999_999).contains(&n) && (5..=9).contains(&(n % 100)))
    {
        return contextual_string_address(n, common_event).map(Value::StringNumber);
    }
    if (1_600_000..=1_699_999).contains(&n) {
        if (5..=9).contains(&(n % 100)) { return Err("common self string classification outside verified source range".into()); }
        let index = common_event.and_then(|event| event.checked_mul(100))
            .and_then(|base| base.checked_add(n as u32 % 100))
            .filter(|index| *index < 1_000_000).ok_or("numeric common operand needs valid context")?;
        return Ok(Value::Var(Var::Common(index as i32)));
    }
    value(n)
}
/// String banks proven by 9E53E0 and the raw-112 caller at A0D330.
pub fn contextual_string_address(n: i32, common_event: Option<u32>) -> Result<Var, String> {
    match n {
        1_600_000..=1_699_999 if (5..=9).contains(&(n % 100)) => {
            let index = common_event
                .and_then(|event| event.checked_mul(100))
                .and_then(|base| base.checked_add(n as u32 % 100))
                .filter(|index| *index < 1_000_000)
                .ok_or("common self string needs valid common-event context")?;
            Ok(Var::Common(index as i32))
        }
        15_000_000..=15_999_999 if (5..=9).contains(&(n % 100)) => Ok(Var::Common(n - 15_000_000)),
        3_000_000..=3_999_999 => Ok(Var::Global(n - 3_000_000)),
        9_900_000..=9_999_999 => Ok(Var::System(n - 9_900_000)),
        _ => Err(format!("unsupported string address {n}")),
    }
}
pub fn value(n: i32) -> Result<Value, String> {
    if n < 1_000_000 {
        Ok(Value::Int(n))
    } else {
        address(n).map(Value::Var)
    }
}
