//! Bounded tick input logs for deterministic verification of the new runtime policy.
use crate::runtime::{Completion, Response, Slots, TickReport, Vm};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::BTreeSet, fmt, io, marker::PhantomData};
use wolf_ir::Library;

const VERSION: u32 = 3;
const MAX_FRAMES: usize = 1_000_000;
const MAX_INPUTS: usize = 1_000_000;
const MAX_SAVE_BYTES: usize = 256 << 20;
const MAX_TAPE_BYTES: usize = 512 << 20;
const MAX_RESPONSE_BYTES: usize = 16 << 20;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default)]
    pub keyboard: Option<crate::input::KeyboardSample>,
    #[serde(deserialize_with = "input_vec")]
    pub globals: Vec<(i32, i32)>,
    #[serde(deserialize_with = "input_vec")]
    pub systems: Vec<(i32, i32)>,
    #[serde(deserialize_with = "completion_vec")]
    pub completions: Vec<Completion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub tick: u64,
    pub input: Input,
    #[serde(deserialize_with = "state_hash")]
    pub state_hash: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Tape {
    version: u32,
    initial_save: Vec<u8>,
    initial_state_hash: String,
    pub frames: Vec<Frame>,
}

fn input_limits(input: &Input) -> Result<(), String> {
    if input.globals.len() > MAX_INPUTS
        || input.systems.len() > MAX_INPUTS
        || input.completions.len() > MAX_INPUTS
    {
        return Err("replay input limit".into());
    }
    if input.completions.iter().any(
        |c| matches!(&c.response, Response::Failed(message) if message.len() > MAX_RESPONSE_BYTES),
    ) {
        return Err("replay response byte limit".into());
    }
    Ok(())
}
fn preflight_slots(slots: &Slots, writes: &[(i32, i32)], cap: usize) -> Result<(), String> {
    let mut new_keys = BTreeSet::new();
    let occupied = slots.numbers.len().saturating_add(slots.strings.len());
    for &(id, _) in writes {
        if !(0..1_000_000).contains(&id) {
            return Err("variable index limit".into());
        }
        if !slots.numbers.contains_key(&id) {
            new_keys.insert(id);
            if occupied.saturating_add(new_keys.len()) > cap {
                return Err("variable slot limit".into());
            }
        }
    }
    Ok(())
}
fn preflight(vm: &Vm, input: &Input) -> Result<(), String> {
    input_limits(input)?;
    vm.state().tick.checked_add(1).ok_or("tick exhausted")?;
    if input.completions.len() > vm.config().max_completions_per_tick {
        return Err("completion batch limit".into());
    }
    preflight_slots(
        &vm.state().world.globals,
        &input.globals,
        vm.config().max_slots,
    )?;
    preflight_slots(
        &vm.state().world.systems,
        &input.systems,
        vm.config().max_slots,
    )
}
fn apply(vm: &mut Vm, input: &Input) -> Result<TickReport, String> {
    if let Some(keyboard) = input.keyboard {
        vm.set_keyboard(keyboard);
    }
    for &(id, value) in &input.globals {
        vm.set_global(id, value)?;
    }
    for &(id, value) in &input.systems {
        vm.set_system(id, value)?;
    }
    vm.tick(input.completions.clone())
}

/// Owns the VM to prevent mutations that bypass the input log.
pub struct Recorder {
    vm: Vm,
    tape: Tape,
    max_frames: usize,
    failed: bool,
}
impl Recorder {
    /// Initialize the world and spawn events before transferring ownership.
    pub fn new(vm: Vm, max_frames: usize) -> Result<Self, String> {
        if max_frames > MAX_FRAMES {
            return Err("replay frame limit".into());
        }
        let initial_save = vm.save()?;
        let initial_state_hash = vm.state_hash()?;
        Ok(Self {
            vm,
            tape: Tape {
                version: VERSION,
                initial_save,
                initial_state_hash,
                frames: Vec::new(),
            },
            max_frames,
            failed: false,
        })
    }
    pub fn vm(&self) -> &Vm {
        &self.vm
    }
    pub fn tick(&mut self, input: Input) -> Result<TickReport, String> {
        if self.failed {
            return Err("recorder stopped after internal tick/hash failure".into());
        }
        if self.tape.frames.len() >= self.max_frames {
            return Err("replay frame limit".into());
        }
        // Input failures are checked before mutating either variable space. Vm owns
        // valid reservations and ID counters; execution faults are TickReport data.
        preflight(&self.vm, &input)?;
        self.failed = true;
        let report = apply(&mut self.vm, &input)?;
        self.tape.frames.push(Frame {
            tick: report.tick,
            input,
            state_hash: self.vm.state_hash()?,
        });
        self.failed = false;
        Ok(report)
    }
    pub fn finish(self) -> (Vm, Tape) {
        (self.vm, self.tape)
    }
}
impl Tape {
    fn shape(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err("unsupported replay version".into());
        }
        if self.initial_save.len() > MAX_SAVE_BYTES || self.frames.len() > MAX_FRAMES {
            return Err("replay save/frame limit".into());
        }
        if !valid_hash(&self.initial_state_hash) {
            return Err("invalid initial replay state hash".into());
        }
        for (index, frame) in self.frames.iter().enumerate() {
            input_limits(&frame.input)?;
            if !valid_hash(&frame.state_hash) {
                return Err(format!("invalid replay state hash at frame {index}"));
            }
            if index > 0 && self.frames[index - 1].tick.checked_add(1) != Some(frame.tick) {
                return Err(format!(
                    "replay frame {index}: nonconsecutive tick {}",
                    frame.tick
                ));
            }
        }
        Ok(())
    }
    pub fn to_bytes(&self, cap: usize) -> Result<Vec<u8>, String> {
        self.shape()?;
        let mut writer = LimitedWriter {
            bytes: Vec::new(),
            cap: cap.min(MAX_TAPE_BYTES),
        };
        serde_json::to_writer(&mut writer, self).map_err(|e| e.to_string())?;
        Ok(writer.bytes)
    }
    pub fn from_bytes(bytes: &[u8], cap: usize, max_frames: usize) -> Result<Self, String> {
        if bytes.len() > cap.min(MAX_TAPE_BYTES) {
            return Err("replay byte limit".into());
        }
        let mut decoder = serde_json::Deserializer::from_slice(bytes);
        let tape = TapeSeed {
            save_cap: bytes.len().min(MAX_SAVE_BYTES),
            max_frames: max_frames.min(MAX_FRAMES),
        }
        .deserialize(&mut decoder)
        .map_err(|e| e.to_string())?;
        decoder.end().map_err(|e| e.to_string())?;
        tape.shape()?;
        Ok(tape)
    }
    /// Rebind current-session completions, preserving intentionally stale tokens.
    /// Hashes detect divergence and accidental corruption, not authenticated provenance.
    pub fn verify(
        &self,
        library: Library,
        new_session: u64,
        save_cap: usize,
    ) -> Result<Vm, String> {
        self.shape()?;
        if self.initial_save.len() > save_cap {
            return Err("replay initial save limit".into());
        }
        #[derive(Deserialize)]
        struct Session {
            session: u64,
        }
        let old_session = serde_json::from_slice::<Session>(&self.initial_save)
            .map_err(|e| e.to_string())?
            .session;
        if new_session == old_session
            || self.frames.iter().any(|f| {
                f.input
                    .completions
                    .iter()
                    .any(|c| c.token.session != old_session && c.token.session == new_session)
            })
        {
            return Err("replay session collides with recorded token generation".into());
        }
        let mut vm = Vm::restore(library, &self.initial_save, new_session, save_cap)?;
        if vm.state_hash()? != self.initial_state_hash {
            return Err("replay initial state hash mismatch".into());
        }
        for (index, frame) in self.frames.iter().enumerate() {
            if vm.state().tick.checked_add(1) != Some(frame.tick) {
                return Err(format!(
                    "replay frame {index}: nonconsecutive tick {}",
                    frame.tick
                ));
            }
            let mut input = frame.input.clone();
            for completion in &mut input.completions {
                if completion.token.session == old_session {
                    completion.token.session = new_session;
                }
            }
            preflight(&vm, &input).map_err(|e| format!("replay tick {}: {e}", frame.tick))?;
            apply(&mut vm, &input).map_err(|e| format!("replay tick {}: {e}", frame.tick))?;
            let actual = vm.state_hash()?;
            if actual != frame.state_hash {
                return Err(format!(
                    "replay divergence at tick {}: expected {}, got {actual}",
                    frame.tick, frame.state_hash
                ));
            }
        }
        Ok(vm)
    }
}

struct LimitedWriter {
    bytes: Vec<u8>,
    cap: usize,
}
impl io::Write for LimitedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.cap.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("replay byte limit"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn state_hash<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    struct HashVisitor;
    impl Visitor<'_> for HashVisitor {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("64 lowercase hexadecimal digits")
        }
        fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
            if !valid_hash(value) {
                return Err(E::custom("invalid replay state hash"));
            }
            Ok(value.to_owned())
        }
    }
    d.deserialize_str(HashVisitor)
}
struct BoundedVec<T> {
    cap: usize,
    marker: PhantomData<T>,
}
impl<T> BoundedVec<T> {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            marker: PhantomData,
        }
    }
}
impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for BoundedVec<T> {
    type Value = Vec<T>;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de, T: Deserialize<'de>> Visitor<'de> for BoundedVec<T> {
    type Value = Vec<T>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at most {} elements", self.cap)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(BoundedElement::<T> {
            allowed: values.len() < self.cap,
            marker: PhantomData,
        })? {
            values.push(value);
        }
        Ok(values)
    }
}
struct BoundedElement<T> {
    allowed: bool,
    marker: PhantomData<T>,
}
impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for BoundedElement<T> {
    type Value = T;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
        if !self.allowed {
            return Err(de::Error::custom("replay collection limit"));
        }
        T::deserialize(d)
    }
}
fn input_vec<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    BoundedVec::new(MAX_INPUTS).deserialize(d)
}
fn completion_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Completion>, D::Error> {
    // Decode the transport shape here, without changing the VM's public types.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireToken {
        session: u64,
        sequence: u64,
    }
    #[derive(Deserialize)]
    enum WireResponse {
        Done,
        Choice(usize),
        Cancel,
        Failed(#[serde(deserialize_with = "response_text")] String),
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireCompletion {
        token: WireToken,
        response: WireResponse,
    }
    let values = BoundedVec::<WireCompletion>::new(MAX_INPUTS).deserialize(d)?;
    Ok(values
        .into_iter()
        .map(|c| Completion {
            token: crate::runtime::Token {
                session: c.token.session,
                sequence: c.token.sequence,
            },
            response: match c.response {
                WireResponse::Done => Response::Done,
                WireResponse::Choice(i) => Response::Choice(i),
                WireResponse::Cancel => Response::Cancel,
                WireResponse::Failed(message) => Response::Failed(message),
            },
        })
        .collect())
}
fn response_text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    struct TextVisitor;
    impl Visitor<'_> for TextVisitor {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a bounded service error string")
        }
        fn visit_str<E: de::Error>(self, text: &str) -> Result<String, E> {
            if text.len() > MAX_RESPONSE_BYTES {
                return Err(E::custom("replay response byte limit"));
            }
            Ok(text.to_owned())
        }
    }
    d.deserialize_str(TextVisitor)
}
struct TapeSeed {
    save_cap: usize,
    max_frames: usize,
}
impl<'de> DeserializeSeed<'de> for TapeSeed {
    type Value = Tape;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Tape, D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for TapeSeed {
    type Value = Tape;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("version 2 replay tape")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Tape, A::Error> {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Version,
            InitialSave,
            InitialStateHash,
            Frames,
        }
        #[derive(Deserialize)]
        struct Hash(#[serde(deserialize_with = "state_hash")] String);
        let (mut version, mut initial_save, mut initial_state_hash, mut frames) =
            (None, None, None, None);
        while let Some(field) = map.next_key()? {
            match field {
                Field::Version => {
                    if version.is_some() {
                        return Err(de::Error::duplicate_field("version"));
                    }
                    let value = map.next_value::<u32>()?;
                    if value != VERSION {
                        return Err(de::Error::custom("unsupported replay version"));
                    }
                    version = Some(value);
                }
                Field::InitialSave => {
                    if initial_save.is_some() {
                        return Err(de::Error::duplicate_field("initial_save"));
                    }
                    initial_save = Some(map.next_value_seed(BoundedVec::new(self.save_cap))?);
                }
                Field::InitialStateHash => {
                    if initial_state_hash.is_some() {
                        return Err(de::Error::duplicate_field("initial_state_hash"));
                    }
                    initial_state_hash = Some(map.next_value::<Hash>()?.0);
                }
                Field::Frames => {
                    if frames.is_some() {
                        return Err(de::Error::duplicate_field("frames"));
                    }
                    frames = Some(map.next_value_seed(BoundedVec::new(self.max_frames))?);
                }
            }
        }
        Ok(Tape {
            version: version.ok_or_else(|| de::Error::missing_field("version"))?,
            initial_save: initial_save.ok_or_else(|| de::Error::missing_field("initial_save"))?,
            initial_state_hash: initial_state_hash
                .ok_or_else(|| de::Error::missing_field("initial_state_hash"))?,
            frames: frames.ok_or_else(|| de::Error::missing_field("frames"))?,
        })
    }
}
impl<'de> Deserialize<'de> for Tape {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let tape = TapeSeed {
            save_cap: MAX_SAVE_BYTES,
            max_frames: MAX_FRAMES,
        }
        .deserialize(d)?;
        tape.shape().map_err(de::Error::custom)?;
        Ok(tape)
    }
}
