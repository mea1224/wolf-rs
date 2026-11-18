//! Reachable title audio: A18940 -> 96BD80 / 96A760 / 96D7A0.
//! Device gain remains a linear output policy, not 88D340 amplitude parity.
use crate::audio::Audio;
use serde::{Deserialize, Serialize};
use wolf_vm::{
    native_values::{DatabaseAddress, DatabaseKind, NativeValue},
    runtime::NativeContext,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Spec {
    kind: u8,
    row: i32,
    path: Option<String>,
    volume: i32,
    duration: i32,
    /// 96D2A3 queues this SE group; nonzero groups use independent handles.
    #[serde(default)]
    se_group: i32,
    #[serde(default)]
    action: u8,
    /// A18940 arg3 -> 88DC30 -> 8A2E20, in milliseconds.
    #[serde(default)]
    seek_ms: i32,
    /// 88CE90 / 88D870 signed stereo balance, clamped to +/-255.
    #[serde(default)]
    pan: i32,
    #[serde(default = "default_pitch")]
    pitch: u32,
    #[serde(default)]
    loop_start_ms: u32,
}
fn default_pitch() -> u32 { 100 }

fn specification(
    args: &[i32], path: Option<String>, zero_is_default: bool,
    mut database: impl FnMut(usize, usize, usize) -> Result<NativeValue, String>,
) -> Result<Spec, String> {
    // A189AB/CF/F8 and A18A23 read optional fields by envelope length.
    if !(3..=9).contains(&args.len()) { return Err("140 audio operand count outside 3..9".into()); }
    let flags = args[0] as u32;
    let source = flags & 0x0f00_0000;
    let kind = ((flags >> 4) & 15) as u8;
    let action = (flags & 15) as u8;
    if kind > 2 || action > 2 { return Err(format!("unsupported 140 audio flags {flags:#x}")); }
    let duration = args[1];
    let seek = args.get(3).copied().unwrap_or(0);
    let extra = args.get(7).copied().unwrap_or(0);
    let group = if kind == 2 { args.get(6) } else if kind == 1 { args.get(8) } else { None }.copied().unwrap_or(0);
    if !(0..=1_000_000).contains(&duration) || group < -1 {
        return Err("140 invalid duration/group".into());
    }
    let direct_row = if source == 0 { ((flags >> 8) & 65535) as i32 }
        else if source == 0x0100_0000 { args[2] }
        else { -1 };
    let stop = matches!(direct_row, -1 | 65535) && source != 0x0200_0000;
    let (row, path, mut volume, pitch, loop_start) = if stop {
        (-1, None, 0, 100, 0)
    } else if matches!(source, 0 | 0x0100_0000) {
        if direct_row < 0 { return Err("140 negative audio database row".into()); }
        let ty = usize::from(kind) + 1;
        let path = match database(ty, direct_row as usize, 0)? {
            NativeValue::Text(value) => value,
            _ => return Err("140 audio database path is not text".into()),
        };
        let mut number = |field| match database(ty, direct_row as usize, field)? {
            NativeValue::Number(value) => Ok(value),
            _ => Err("140 audio database metadata is not numeric".to_owned()),
        };
        (direct_row, Some(path), number(1)?, number(2)?, number(3)?)
    } else if source == 0x0200_0000 {
        if args.len() < 6 { return Err("140 file source requires volume and pitch".into()); }
        let path = path.ok_or("140 file source lacks a path operand")?;
        if path.is_empty() { (-1, None, 0, 100, 0) }
        else { (-2, Some(path), args[4], args[5], if kind < 2 { args.get(6).copied().unwrap_or(0) } else { 0 }) }
    } else { return Err(format!("unsupported 140 audio source {source:#x}")); };
    if zero_is_default && volume == 0 && path.is_some() { volume = 100; }
    if !(0..=100).contains(&volume) || pitch > 1_000_000 || loop_start < -1 {
        return Err(format!("140 unsupported volume/pitch/loop metadata {volume}/{pitch}/{loop_start}"));
    }
    Ok(Spec { kind, row, path, volume, duration, se_group: group, action, seek_ms: seek.max(0), pan: extra.clamp(-255, 255), pitch: if pitch <= 0 { 100 } else { pitch as u32 }, loop_start_ms: loop_start.max(0) as u32 })
}

/// Native 96C3A9 stores requested duration + 1. 96D7A0 applies the current
/// timer before subtracting FA2AE8. Values are integer engine volume units.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Fade {
    from: i32,
    to: i32,
    total: i32,
    remaining: i32,
}
impl Fade {
    fn new(from: i32, to: i32, duration: i32) -> Self {
        Self {
            from,
            to,
            total: duration + 1,
            remaining: duration + 1,
        }
    }
    fn tick(&mut self) -> i32 {
        let old = self.remaining;
        if old < 0 {
            return self.to;
        }
        // 96DB00/96DB18: integer start term; 96DB1A..96DB30: ratio then target.
        let value = (i64::from(self.from) * i64::from(old) / i64::from(self.total)) as i32
            + (f64::from(self.total - old) / f64::from(self.total) * f64::from(self.to)) as i32;
        self.remaining = if old == 0 { -1 } else { old - 1 };
        value
    }
    fn done(&self) -> bool {
        self.remaining < 0
    }
    fn retarget(&mut self, target: i32) {
        // 9E8F08..9E8F49 preserves +FC (actual interpolation start) and active
        // timer. It copies old target to +F0, which is a DIFFERENT saved field.
        if self.remaining < 0 {
            self.total = 1;
            self.remaining = 0;
        }
        self.to = target;
    }
}

fn output_gain(volume: i32, master: i32, kind: u8) -> Result<u16, String> {
    if master < 0 {
        return Err("negative native audio master volume".into());
    }
    let mut level = i64::from(volume) * i64::from(master) / 100;
    // 96D33F..96D365 preserves a minimum level 1 on the non-default SE branch.
    if kind == 2 && master > 0 {
        level = level.max(1);
    }
    let gain = level * 256 / 100;
    if !(0..=256).contains(&gain) {
        return Err("native audio exceeds linear output gain range".into());
    }
    Ok(gain as u16)
}

struct Background {
    spec: Spec,
    audio: Audio,
    fade: Fade,
    level: i32,
    master: i32,
    pan_from: i32,
}
enum Pending {
    Background(Spec, Option<wolf_media::Clip>, i32, Fade),
    Effect(Spec, wolf_media::Clip, u16),
}

#[derive(Serialize, Deserialize)]
struct SavedVoice {
    path: String,
    elapsed_frames: u64,
    sample_rate: u32,
    gain: u16,
    #[serde(default)]
    se_group: i32,
    #[serde(default)]
    pan: i16,
    #[serde(default = "default_pitch")]
    pitch: u32,
    #[serde(default)]
    loop_start_ms: u32,
}
impl SavedVoice {
    fn capture(path: String, audio: &Audio) -> Self {
        Self {
            path,
            elapsed_frames: audio.rendered_frames(),
            sample_rate: audio.sample_rate(),
            gain: audio.gain(),
            se_group: 0,
            pan: audio.pan(),
            pitch: audio.pitch(),
            loop_start_ms: audio.loop_start_ms(),
        }
    }
    fn restore(
        self,
        looping: bool,
        read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<Audio, String> {
        let audio = Audio::with_playback(
            read_clip(&self.path, read)?,
            self.gain,
            looping,
            self.elapsed_frames,
            self.sample_rate,
            self.pitch,
            self.loop_start_ms,
        )?;
        audio.set_pan(i32::from(self.pan))?;
        Ok(audio)
    }
}
#[derive(Serialize, Deserialize)]
struct SavedBackground {
    spec: Spec,
    voice: SavedVoice,
    fade: Fade,
    level: i32,
    master: i32,
    #[serde(default)]
    pan_from: i32,
}
#[derive(Serialize, Deserialize)]
enum SavedPending {
    Background(Spec, i32, Fade),
    Effect(Spec, u16),
}
#[derive(Serialize, Deserialize)]
struct SavedAudio {
    version: u32,
    zero_is_default: bool,
    background_row: i32,
    background: Option<SavedBackground>,
    retiring: Vec<SavedBackground>,
    effects: Vec<SavedVoice>,
    pending: Vec<SavedPending>,
    #[serde(default)]
    ambience: std::collections::BTreeMap<i32, Vec<u8>>,
}

fn read_clip(
    path: &str,
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<wolf_media::Clip, String> {
    let bytes = read(path)?;
    if path.to_ascii_lowercase().ends_with(".ogg") {
        wolf_media::Clip::from_ogg(&bytes)
    } else if path.to_ascii_lowercase().ends_with(".wav") {
        wolf_media::Clip::from_wav(&bytes)
    } else {
        Err("140 audio decoder supports Ogg/WAV".into())
    }
}

pub struct NativeAudio {
    zero_is_default: bool,
    pending: Vec<Pending>,
    background: Option<Background>,
    retiring: Vec<Background>,
    effects: Vec<(String, i32, Audio)>,
    background_row: i32,
    ambience: std::collections::BTreeMap<i32, NativeAudio>,
    cache: std::collections::BTreeMap<String, wolf_media::Clip>,
}
impl NativeAudio {
    /// A40519..A4067C rebuilds the saved BGM row/path and starts a 20-step
    /// fade from zero. Native snapshots do not resume the device sample clock.
    pub fn restore_native(
        &mut self,
        saved: &crate::native_save_payload::SaveSnapshot,
        mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        let row = saved.system_i32(2268).ok_or("saved BGM row missing")?;
        let path = std::str::from_utf8(
            saved
                .system_strings
                .get(&2624)
                .ok_or("saved BGM path missing")?,
        )
        .map_err(|e| format!("saved BGM path: {e}"))?
        .to_owned();
        let volume = saved.system_i32(2648).ok_or("saved BGM volume missing")?;
        let pitch = saved.system_i32(2652).ok_or("saved BGM pitch missing")?;
        let loop_start = saved
            .system_i32(2656)
            .ok_or("saved BGM loop metadata missing")?;
        let extra = saved
            .system_i32(3472)
            .ok_or("saved BGM extra metadata missing")?;
        let master = saved.system_i32(2612).ok_or("saved BGM master missing")?;
        let mut pending = Vec::new();
        if row >= 0 || row == -3 {
            if !(0..=100).contains(&volume)
                || pitch > 1_000_000
                || loop_start < -1
            {
                return Err(format!(
                    "saved BGM metadata unsupported: {volume}/{pitch}/{loop_start}/{extra}"
                ));
            }
            if !path.is_empty() {
                let clip = read_clip(&path, &mut read)?;
                output_gain(volume, master, 0)?;
                let target = (i64::from(volume) * i64::from(master) / 100) as i32;
                let spec = Spec {
                    kind: 0,
                    row,
                    path: Some(path),
                    volume,
                    duration: 20,
                    se_group: 0, action: 0, seek_ms: 0, pan: extra,
                    pitch: if pitch <= 0 { 100 } else { pitch as u32 }, loop_start_ms: loop_start.max(0) as u32,
                };
                pending.push(Pending::Background(
                    spec,
                    Some(clip),
                    master,
                    Fade::new(0, target, 20),
                ));
            }
        }
        // A40681..A4092F walks active channel IDs; metadata records can also
        // contain previously stopped channels and must not revive them.
        let mut ambience = std::collections::BTreeMap::new();
        let bgs_master = saved.system_i32(2616).ok_or("saved BGS master missing")?;
        for (&channel, &active_row) in &saved.background_sound_ids {
            if active_row < 0 && active_row != -3 { continue; }
            if channel < 0 || ambience.len() >= 32 { return Err("saved BGS channel limit".into()); }
            let (_, path, fields) = saved.background_sound_rows.iter().find(|(id, _, _)| *id == channel).ok_or("saved active BGS channel lacks metadata")?;
            if path.is_empty() { continue; }
            if fields.len() != 4 { return Err("saved BGS metadata field count".into()); }
            let path = std::str::from_utf8(path).map_err(|e| format!("saved BGS path: {e}"))?.to_owned();
            let mut spec = specification(&[0x0200_0010, 20, 0, 0, fields[0], fields[1], fields[2], fields[3], channel], Some(path), self.zero_is_default, |_, _, _| unreachable!())?;
            spec.kind = 0;
            spec.row = active_row;
            let mut audio = Self::empty(self.zero_is_default);
            audio.queue(spec, bgs_master, &mut read as &mut dyn FnMut(&str) -> Result<Vec<u8>, String>)?;
            ambience.insert(channel, audio);
        }
        self.ambience = ambience;
        self.pending = pending;
        self.background = None;
        self.retiring.clear();
        self.effects.clear();
        self.background_row = if self.pending.is_empty() { -1 } else { row };
        Ok(())
    }

    pub fn capture_state(&self) -> Result<Vec<u8>, String> {
        let capture = |bg: &Background| -> Result<SavedBackground, String> {
            Ok(SavedBackground {
                spec: bg.spec.clone(),
                voice: SavedVoice::capture(
                    bg.spec.path.clone().ok_or("active BGM missing source")?,
                    &bg.audio,
                ),
                fade: bg.fade.clone(),
                level: bg.level,
                master: bg.master,
                pan_from: bg.pan_from,
            })
        };
        let state = SavedAudio {
            version: 3,
            ambience: self.ambience.iter().map(|(&channel, audio)| Ok((channel, audio.capture_state()?))).collect::<Result<_, String>>()?,
            zero_is_default: self.zero_is_default,
            background_row: self.background_row,
            background: self.background.as_ref().map(capture).transpose()?,
            retiring: self
                .retiring
                .iter()
                .map(capture)
                .collect::<Result<_, _>>()?,
            effects: self
                .effects
                .iter()
                .map(|(path, group, audio)| {
                    let mut voice = SavedVoice::capture(path.clone(), audio);
                    voice.se_group = *group;
                    voice
                })
                .collect(),
            pending: self
                .pending
                .iter()
                .map(|pending| match pending {
                    Pending::Background(spec, _, master, fade) => {
                        SavedPending::Background(spec.clone(), *master, fade.clone())
                    }
                    Pending::Effect(spec, _, gain) => SavedPending::Effect(spec.clone(), *gain),
                })
                .collect(),
        };
        serde_json::to_vec(&state).map_err(|e| format!("audio capture: {e}"))
    }

    pub fn restore_state(
        &mut self,
        bytes: &[u8],
        mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        if bytes.len() > 1024 * 1024 {
            return Err("saved audio byte limit".into());
        }
        let state: SavedAudio =
            serde_json::from_slice(bytes).map_err(|e| format!("audio restore: {e}"))?;
        if state.version != 3
            || state.effects.len() + state.retiring.len() + state.pending.len() > 32
        {
            return Err("saved audio version/voice limit".into());
        }
        for bg in state.background.iter().chain(state.retiring.iter()) {
            if bg.spec.kind != 0
                || bg.fade.total < 1
                || bg.fade.remaining < -1
                || bg.fade.remaining > bg.fade.total
                || bg.spec.pitch == 0 || bg.spec.pitch > 1_000_000 || bg.spec.seek_ms < 0
            {
                return Err("saved BGM fade state invalid".into());
            }
            output_gain(bg.level, 100, 0)?;
        }
        for pending in &state.pending {
            match pending {
                SavedPending::Background(spec, master, fade) => {
                    if spec.kind != 0
                        || fade.total < 1
                        || fade.remaining < -1
                        || fade.remaining > fade.total
                        || spec.pitch == 0 || spec.pitch > 1_000_000 || spec.seek_ms < 0
                    {
                        return Err("saved pending BGM fade state invalid".into());
                    }
                    output_gain(spec.volume, *master, 0)?;
                    output_gain(fade.from, 100, 0)?;
                    output_gain(fade.to, 100, 0)?;
                }
                SavedPending::Effect(spec, gain)
                    if spec.kind != 2 || spec.se_group < -1 || spec.se_group == -1 && spec.row != -1 || *gain > 256
                        || spec.pitch == 0 || spec.pitch > 1_000_000 || spec.duration < 0 =>
                {
                    return Err("saved pending SE state invalid".into());
                }
                SavedPending::Effect(_, _) => {}
            }
        }
        let mut restore_background = |saved: SavedBackground| -> Result<Background, String> {
            let audio = saved.voice.restore(true, &mut read)?;
            Ok(Background {
                spec: saved.spec,
                audio,
                fade: saved.fade,
                level: saved.level,
                master: saved.master,
                pan_from: saved.pan_from,
            })
        };
        let background = state.background.map(&mut restore_background).transpose()?;
        let retiring = state
            .retiring
            .into_iter()
            .map(&mut restore_background)
            .collect::<Result<Vec<_>, _>>()?;
        let mut effects = Vec::new();
        for voice in state.effects {
            let path = voice.path.clone();
            let group = voice.se_group;
            if group < 0 {
                return Err("saved SE group invalid".into());
            }
            let audio = voice.restore(false, &mut read)?;
            if !audio.finished() {
                effects.push((path, group, audio));
            }
        }
        let mut pending = Vec::new();
        for saved in state.pending {
            match saved {
                SavedPending::Background(spec, master, fade) => {
                    let clip = spec
                        .path
                        .as_deref()
                        .map(|path| read_clip(path, &mut read))
                        .transpose()?;
                    pending.push(Pending::Background(spec, clip, master, fade));
                }
                SavedPending::Effect(spec, gain) => {
                    let clip = if spec.row == -1 { wolf_media::Clip::new(8000, vec![[0, 0]])? }
                        else { read_clip(spec.path.as_deref().ok_or("saved SE missing source")?, &mut read)? };
                    pending.push(Pending::Effect(spec, clip, gain));
                }
            }
        }
        if state.ambience.len() > 32 { return Err("saved BGS channel limit".into()); }
        let mut ambience = std::collections::BTreeMap::new();
        for (channel, bytes) in state.ambience {
            if channel < 0 { return Err("negative saved BGS channel".into()); }
            let nested: SavedAudio = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if !nested.ambience.is_empty() { return Err("nested BGS channel state".into()); }
            let mut audio = Self::empty(state.zero_is_default);
            audio.restore_state(&bytes, &mut read as &mut dyn FnMut(&str) -> Result<Vec<u8>, String>)?;
            ambience.insert(channel, audio);
        }
        *self = Self {
            ambience, cache: Default::default(),
            zero_is_default: state.zero_is_default,
            pending,
            background,
            retiring,
            effects,
            background_row: state.background_row,
        };
        Ok(())
    }

    pub fn new(configuration: &[u8]) -> Result<Self, String> {
        // 459523..459526 binds config+39 to object+134; global object FA5180
        // therefore publishes it through FA52B4. 96C355..96C36D consumes it.
        let zero_is_default = *configuration
            .get(39)
            .ok_or("Game.dat audio configuration missing")?
            == 1;
        Ok(Self::empty(zero_is_default))
    }
    fn empty(zero_is_default: bool) -> Self {
        Self {
            ambience: Default::default(), cache: Default::default(),
            zero_is_default,
            pending: Vec::new(),
            background: None,
            retiring: Vec::new(),
            effects: Vec::new(),
            background_row: -1,
        }
    }
    pub fn execute(
        &mut self,
        context: &mut NativeContext<'_>,
        read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        let raw = context
            .instruction
            .raw
            .as_ref()
            .ok_or("140 lacks raw command")?;
        let mut args = raw.args.clone();
        for value in args.iter_mut().skip(1) {
            *value = context.number(*value)?;
        }
        let path = raw
            .strings
            .first()
            .map(|s| s.text().map(str::to_owned).map_err(|e| e.to_string()))
            .transpose()?
            .map(|s| context.expand(&s))
            .transpose()?;
        let spec = specification(&args, path, self.zero_is_default, |type_id, row, field| {
            context
                .world
                .databases
                .get(DatabaseAddress {
                    kind: DatabaseKind::System,
                    type_id,
                    row,
                    field,
                })
                .cloned()
        })?;
        if spec.action == 0 && spec.kind == 2 && spec.row == -1 && spec.duration == 0 {
            self.stop_effect_group(spec.se_group);
            return Ok(());
        }
        let master = context.number(9_000_100 + i32::from(spec.kind))?;
        self.queue(spec, master, read)
    }
    /// Route46 A0377E..A0379D calls 96BD80 with kind2, resolved database
    /// row, duration0, seek0 and group1. The group creates independent voices.
    pub fn route_effect(
        &mut self,
        row: i32,
        master: i32,
        database: impl FnMut(usize, usize, usize) -> Result<NativeValue, String>,
        read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        if matches!(row, -1 | 65535) {
            self.stop_effect_group(1);
            return Ok(());
        }
        let mut spec = specification(
            &[0x0100_0020, 0, row, 0, 0, 0, 0, 0],
            None,
            self.zero_is_default,
            database,
        )?;
        spec.se_group = 1;
        self.queue(spec, master, read)
    }
    /// 9FE550 map metadata: -2 keeps playback, -1 stops, -3 uses the
    /// configured file. Outgoing voices fade for five ticks; the new map voice
    /// receives timer=0,total=1 at 9FE638 / 9FE7D9.
    pub(crate) fn map_change(
        &mut self, kind: u8, row: i32, path: Option<String>, master: i32,
        database: impl FnMut(usize, usize, usize) -> Result<NativeValue, String>,
        mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        if kind > 1 || row < -3 { return Err("invalid map audio kind/row".into()); }
        if row == -2 { return Ok(()); }
        let flags = (i32::from(kind) << 4) | if row == -3 { 0x0200_0000 } else { 0x0100_0000 };
        let spec = specification(&[flags, 5, row, 0, 100, 100, 0, 0, 0], path, self.zero_is_default, database)?;
        if kind == 1 {
            for (&channel, audio) in &mut self.ambience {
                if channel == 0 { continue; }
                let stop = specification(&[0x0100_0000, 5, -1], None, self.zero_is_default, |_, _, _| unreachable!())?;
                audio.queue(stop, master, &mut read as &mut dyn FnMut(&str) -> Result<Vec<u8>, String>)?;
            }
        }
        self.queue(spec, master, read)?;
        let channel = if kind == 1 { self.ambience.get_mut(&0).ok_or("map BGS channel missing")? } else { self };
        if let Some(Pending::Background(spec, _, _, fade)) = channel.pending.iter_mut().rev().find(|pending| matches!(pending, Pending::Background(..))) {
            fade.total = 1;
            fade.remaining = 0;
            fade.to = (i64::from(spec.volume) * i64::from(master) / 100) as i32;
        } else if let Some(background) = &mut channel.background {
            background.fade.total = 1;
            background.fade.remaining = 0;
            background.fade.to = (i64::from(background.spec.volume) * i64::from(master) / 100) as i32;
        }
        Ok(())
    }
    fn queue(
        &mut self,
        mut spec: Spec,
        master: i32,
        mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        if spec.kind == 1 {
            if spec.se_group == -1 {
                if spec.row != -1 { return Err("BGS all-channel operation needs stop sentinel".into()); }
                for audio in self.ambience.values_mut() {
                    let mut stop = spec.clone(); stop.kind = 0; stop.se_group = 0;
                    audio.queue(stop, master, &mut read as &mut dyn FnMut(&str) -> Result<Vec<u8>, String>)?;
                }
                return Ok(());
            }
            if !self.ambience.contains_key(&spec.se_group) && self.ambience.len() >= 32 { return Err("BGS channel limit".into()); }
            let channel = self.ambience.entry(spec.se_group).or_insert_with(|| Self::empty(self.zero_is_default));
            spec.kind = 0;
            return channel.queue(spec, master, read);
        }
        if spec.action != 0 {
            if let Some(path) = &spec.path {
                if spec.action == 1 {
                    self.cached_clip(path, &mut read)?;
                } else {
                    self.cache.remove(path);
                    self.effects.retain(|(old, _, _)| old != path);
                    self.pending.retain(|pending| match pending { Pending::Background(old, ..) | Pending::Effect(old, ..) => old.path.as_ref() != Some(path) });
                    if self.background.as_ref().is_some_and(|bg| bg.spec.path.as_ref() == Some(path)) { self.background = None; self.background_row = -1; }
                    self.retiring.retain(|bg| bg.spec.path.as_ref() != Some(path));
                }
            }
            return Ok(());
        }
        let gain = output_gain(spec.volume, master, spec.kind)?;
        let same = |old: &Spec| old.row == spec.row && (spec.row != -2 || old.path == spec.path);
        if spec.kind == 0 {
            if let Some(Pending::Background(old, _, _, _)) = self.pending.iter_mut().rev().find(|pending| matches!(pending, Pending::Background(..))) {
                if same(old) {
                    if spec.seek_ms > 0 { old.seek_ms = spec.seek_ms; }
                    old.pan = spec.pan;
                    old.volume = spec.volume;
                    return Ok(());
                }
            } else if self.background.as_ref().is_some_and(|bg| same(&bg.spec)) {
                // Same-row playback keeps its fade and handle clock unless the
                // script explicitly supplies a new position (96BE76..96BF20).
                let replacement = if spec.seek_ms > 0 {
                    let path = spec.path.as_deref().ok_or("audio seek without source")?;
                    let clip = self.cached_clip(path, &mut read)?;
                    let old = &self.background.as_ref().unwrap().spec;
                    Some(Audio::with_playback(clip, self.background.as_ref().unwrap().audio.gain(), true, spec.seek_ms as u64 * 100, 1000 * old.pitch, old.pitch, old.loop_start_ms)?)
                } else { None };
                let bg = self.background.as_mut().unwrap();
                if let Some(audio) = replacement { bg.audio = audio; }
                bg.pan_from = if spec.duration == 0 { spec.pan } else { bg.spec.pan };
                bg.spec.pan = spec.pan;
                bg.spec.volume = spec.volume;
                if spec.duration == 0 { bg.audio.set_pan(spec.pan)?; }
                return Ok(());
            }
        }
        if spec.kind == 2 && spec.row != -1 && master == 0 {
            return Ok(());
        }
        if self.pending.len() + self.effects.len() + self.retiring.len() >= 32 {
            return Err("native audio voice limit".into());
        }
        let clip = spec.path.as_ref().map(|path| self.cached_clip(path, &mut read)).transpose()?;
        if spec.kind == 0 {
            self.background_row = spec.row;
            // 96C377..96C390 applies the master before fade interpolation.
            let target = (i64::from(spec.volume) * i64::from(master) / 100) as i32;
            let fade = Fade::new(0, target, spec.duration);
            self.pending
                .push(Pending::Background(spec, clip, master, fade));
        } else {
            self.pending.push(Pending::Effect(
                spec,
                clip.unwrap_or(wolf_media::Clip::new(8000, vec![[0,0]])?),
                gain,
            ));
        }
        Ok(())
    }
    fn cached_clip(&mut self, path: &str, read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>) -> Result<wolf_media::Clip, String> {
        if let Some(clip) = self.cache.get(path) { return Ok(clip.clone()); }
        let clip = read_clip(path, read)?;
        // Bounded decoded PCM cache; Clip clones share immutable samples.
        // Cache eviction never stops voices already using those samples.
        let bytes = clip.frames().len() * 4;
        const BUDGET: usize = 128 << 20;
        let mut total: usize = self.cache.values().map(|old| old.frames().len() * 4).sum();
        while total + bytes > BUDGET {
            let Some((_, old)) = self.cache.pop_first() else { break; };
            total -= old.frames().len() * 4;
        }
        if bytes <= BUDGET { self.cache.insert(path.to_owned(), clip.clone()); }
        Ok(clip)
    }
    fn stop_effect_group(&mut self, group: i32) {
        // Dropping the Audio releases its stream. Also remove commands queued
        // earlier in this logical tick so a subsequent tick cannot revive them.
        self.effects
            .retain(|(_, active_group, _)| group != -1 && *active_group != group);
        self.pending.retain(|pending| {
            !matches!(pending,
            Pending::Effect(spec, _, _) if group == -1 || spec.se_group == group)
        });
    }
    pub fn sync_bgs_master(&mut self, master: i32) -> Result<(), String> {
        for channel in self.ambience.values_mut() { channel.sync_bgm_master(master)?; }
        Ok(())
    }
    pub fn sync_bgm_master(&mut self, master: i32) -> Result<(), String> {
        if master < 0 {
            return Err("negative native BGM master".into());
        }
        if let Some(bg) = &mut self.background {
            if bg.master != master {
                // Validate the output limit before changing the active envelope.
                output_gain(bg.spec.volume, master, 0)?;
                let target = (i64::from(bg.spec.volume) * i64::from(master) / 100) as i32;
                bg.fade.retarget(target);
                bg.master = master;
            }
        }
        for pending in &mut self.pending {
            if let Pending::Background(spec, _, old_master, fade) = pending {
                output_gain(spec.volume, master, 0)?;
                if *old_master != master {
                    let target = (i64::from(spec.volume) * i64::from(master) / 100) as i32;
                    fade.retarget(target);
                    *old_master = master;
                }
            }
        }
        Ok(())
    }
    /// Advance once per logical step1 tick. Never call per redraw/device callback.
    pub fn tick(&mut self) -> Result<(), String> {
        for channel in self.ambience.values_mut() { channel.tick()?; }
        self.effects.retain(|(_, _, audio)| !audio.finished());
        let mut pending_commands = std::collections::VecDeque::from(std::mem::take(&mut self.pending));
        while let Some(pending) = pending_commands.pop_front() {
            match pending {
                Pending::Background(spec, clip, master, fade) => {
                    let pan_from = if spec.duration == 0 { spec.pan } else { self.background.as_ref().map_or(0, |old| old.spec.pan) };
                    let reuse = self.retiring.iter().position(|old| old.spec.row == spec.row && (spec.row != -2 || old.spec.path == spec.path));
                    let next = if let Some(index) = reuse.filter(|_| spec.seek_ms == 0) { Some(self.retiring.remove(index).audio) }
                        else { clip.map(|clip| Audio::with_playback(clip, 0, true, spec.seek_ms as u64 * 100, 1000 * spec.pitch, spec.pitch, spec.loop_start_ms)).transpose()? };
                    // 96BF25..96BF54 keeps a reused handle, releases other outgoing voices.
                    self.retiring.clear();
                    if let Some(mut old) = self.background.take() {
                        // 96BFBA/96C344 snapshots target (+E4) into outgoing
                        // volume (+F4), even if the previous fade was unfinished.
                        old.fade = Fade::new(old.fade.to, 0, spec.duration);
                        old.pan_from = i32::from(old.audio.pan());
                        old.spec.pan = old.pan_from;
                        if !(spec.row == -1 && spec.duration == 0) { self.retiring.push(old); }
                    }
                    if let Some(audio) = next {
                        audio.set_pan(pan_from)?;
                        println!(
                            "NATIVE BGM start path={} duration={} looping=true",
                            spec.path.as_deref().unwrap_or(""),
                            spec.duration
                        );
                        self.background = Some(Background {
                            fade,
                            spec,
                            audio,
                            level: 0,
                            master,
                            pan_from,
                        });
                    }
                }
                Pending::Effect(mut spec, clip, gain) => {
                    // 96EAE9 tests zero before 96EC76 decrements the delay.
                    if spec.duration > 0 {
                        spec.duration -= 1;
                        self.pending.push(Pending::Effect(spec, clip, gain));
                        continue;
                    }
                    if spec.row == -1 {
                        self.stop_effect_group(spec.se_group);
                        // 96D410 removes every matching queued handle, including
                        // entries later in the queue. Those entries are still
                        // in this tick's work list, not in self.pending yet.
                        pending_commands.retain(|pending| !matches!(pending,
                            Pending::Effect(other, _, _) if spec.se_group == -1 || other.se_group == spec.se_group));
                        continue;
                    }
                    let path = spec.path.ok_or("140 pending SE lacks path")?;
                    let audio = Audio::with_playback(clip, gain, false, 0, 1, spec.pitch, 0)?;
                    audio.set_pan(spec.pan)?;
                    // 96D250..96D270 duplicates handles only for nonzero
                    // groups. Group zero reuses/restarts the resource handle.
                    if spec.se_group == 0 {
                        self.effects
                            .retain(|(old, group, _)| old != &path || *group != 0);
                    }
                    println!("NATIVE SE start path={path} gain={gain}/256 looping=false");
                    self.effects.push((path, spec.se_group, audio));
                }
            }
        }
        for bg in self.background.iter_mut().chain(self.retiring.iter_mut()) {
            // 96DB94 interpolates pan with the same timer as the BGM fade.
            let left = bg.fade.remaining.max(0);
            let pan = (i64::from(bg.pan_from) * i64::from(left) / i64::from(bg.fade.total)) as i32
                + (f64::from(bg.fade.total - left) / f64::from(bg.fade.total) * f64::from(bg.spec.pan)) as i32;
            bg.audio.set_pan(pan)?;
            bg.level = bg.fade.tick();
            bg.audio
                .set_gain(output_gain(bg.level, 100, bg.spec.kind)?)?;
        }
        self.retiring.retain(|old| !old.fade.done());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_map_route_sound_uses_database_group_one_and_group_stop() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = crate::assets::read_member(&root, "BasicData/Game.dat").unwrap();
        let config = wolf_format::parse_game(&bytes, &Default::default()).unwrap();
        let bytes = crate::assets::read_member(&root, "BasicData/SysDatabase.dat").unwrap();
        let database = wolf_format::parse_database(&bytes, &Default::default()).unwrap();
        let lookup = |ty: usize, row: usize, field: usize| {
            assert_eq!((ty, row), (3, 16));
            let ty = &database.value.types[ty];
            Ok(if field == 0 {
                NativeValue::Text(ty.string(row, field).unwrap().text().unwrap().into())
            } else {
                NativeValue::Number(ty.integer(row, field).unwrap())
            })
        };
        let mut audio = NativeAudio::new(&config.configuration).unwrap();
        for _ in 0..2 {
            audio
                .route_effect(16, 100, lookup, |path| {
                    crate::assets::read_member(&root, path)
                })
                .unwrap();
        }
        assert_eq!(audio.pending.len(), 2);
        for pending in &audio.pending {
            let Pending::Effect(spec, _, _) = pending else {
                panic!("expected route SE");
            };
            assert_eq!((spec.row, spec.se_group, spec.duration), (16, 1, 0));
        }
        audio
            .route_effect(-1, 100, |_, _, _| unreachable!(), |_| unreachable!())
            .unwrap();
        assert!(audio.pending.is_empty());
        audio
            .route_effect(16, 0, lookup, |_| panic!("muted SE must not load"))
            .unwrap();
        assert!(audio.pending.is_empty());
    }
    #[test]
    fn real_continue_stop_targets_group_four_and_cancels_queued_voices() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = crate::assets::read_member(&root, "BasicData/CommonEvent.dat").unwrap();
        let bytes = bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&bytes);
        let common = wolf_format::parse_common(bytes, &Default::default()).unwrap();
        let command = &common
            .value
            .events
            .iter()
            .find(|event| event.id == 10)
            .unwrap()
            .commands[687];
        assert_eq!(command.args, [-224, 0, 0, 0, 0, 0, 4, 0]);
        let program = wolf_ir::compile(
            "se-stop".into(),
            "Common10 SE group stop".into(),
            wolf_ir::Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: "private-fixture".into(),
                event: 10,
                page: None,
                command: 687,
                span: [0, 0],
            },
            std::slice::from_ref(command),
        )
        .unwrap();
        let mut vm = wolf_vm::runtime::Vm::new(
            wolf_ir::Library::new(vec![program]).unwrap(),
            Default::default(),
            1,
            1000,
        )
        .unwrap();
        vm.spawn(wolf_ir::Target::Id("se-stop".into()), 0, vec![])
            .unwrap();
        let mut audio = NativeAudio::new(&[0; 40]).unwrap();
        for group in [0, 3, 4, 4] {
            let spec = Spec {
                kind: 2,
                row: -2,
                path: Some("same.wav".into()),
                volume: 100,
                duration: 0,
                se_group: group, action: 0, seek_ms: 0, pan: 0, pitch: 100, loop_start_ms: 0,
            };
            audio.pending.push(Pending::Effect(
                spec,
                wolf_media::Clip::new(8000, vec![[0, 0]; 8]).unwrap(),
                256,
            ));
        }
        let report = vm
            .tick_with_host(vec![], &mut |ctx| {
                audio.execute(ctx, |_| Err("stop must not read an audio resource".into()))?;
                Ok(true)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        let groups = audio
            .pending
            .iter()
            .filter_map(|pending| match pending {
                Pending::Effect(spec, _, _) => Some(spec.se_group),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(groups, [0, 3]);
        let stop = specification(&[-224, 0, 0, 0, 0, 0, -1, 0], None, false, fixture).unwrap();
        audio.stop_effect_group(stop.se_group);
        assert!(audio.pending.is_empty());
    }

    #[test]
    fn database_unload_minus_one_does_not_turn_into_a_group_stop() {
        // Common69 PC10462 uses this variable-row unload envelope. A18A38..74
        // skips unload when the resolved row is -1; it is not a play/stop command.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = crate::assets::read_member(&root, "BasicData/CommonEvent.dat").unwrap();
        let bytes = bytes.strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0").unwrap_or(&bytes);
        let common = wolf_format::parse_common(bytes, &Default::default()).unwrap();
        let mut command = common.value.events.iter().find(|event| event.id == 69).unwrap().commands[10462].clone();
        assert_eq!(command.args, [0x0100_0022, 0, 1600010]);
        command.args[2] = -1;
        let program = wolf_ir::compile("unload".into(), "resolved unload sentinel".into(), wolf_ir::Source {
            asset: "BasicData/CommonEvent.dat".into(), content_blake3: "private-fixture".into(),
            event: 69, page: None, command: 10462, span: [0, 0],
        }, &[command]).unwrap();
        let mut vm = wolf_vm::runtime::Vm::new(wolf_ir::Library::new(vec![program]).unwrap(), Default::default(), 1, 1000).unwrap();
        vm.spawn(wolf_ir::Target::Id("unload".into()), 0, vec![]).unwrap();
        let mut audio = NativeAudio::empty(false);
        let spec = specification(&[0x0200_0020, 5, 0, 0, 100, 100, 0, 0], Some("queued.wav".into()), false, fixture).unwrap();
        audio.queue(spec, 100, |_| Ok(wave_fixture())).unwrap();
        let report = vm.tick_with_host(vec![], &mut |ctx| {
            audio.execute(ctx, |_| panic!("unload sentinel read a resource"))?;
            Ok(true)
        }).unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert_eq!(audio.pending.len(), 1, "unload -1 cancelled queued group-zero audio");
    }

    #[test]
    fn real_newgame_se_group_is_preserved_in_pending_snapshot() {
        let spec = specification(
            &[0x0200_0020, 0, 0, 0, 100, 100, 3, 0],
            Some("SE/dialogue.wav".into()),
            false,
            |_, _, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(spec.se_group, 3);
        let mut audio = NativeAudio::new(&[0; 40]).unwrap();
        audio.pending.push(Pending::Effect(
            spec,
            wolf_media::Clip::new(8000, vec![[0, 0]; 8]).unwrap(),
            256,
        ));
        let saved: SavedAudio = serde_json::from_slice(&audio.capture_state().unwrap()).unwrap();
        assert_eq!(saved.version, 3);
        let SavedPending::Effect(spec, _) = &saved.pending[0] else {
            panic!("missing SE");
        };
        assert_eq!(spec.se_group, 3);
    }
    #[test]
    fn pending_audio_snapshot_reloads_resource_and_preserves_fade_progress() {
        let mut audio = NativeAudio::new(&[0; 40]).unwrap();
        let mut fade = Fade::new(0, 80, 20);
        for _ in 0..7 {
            fade.tick();
        }
        let expected = fade.clone();
        let spec = Spec {
            kind: 0,
            row: 7,
            path: Some("test.wav".into()),
            volume: 80,
            duration: 20,
            se_group: 0, action: 0, seek_ms: 0, pan: 0, pitch: 100, loop_start_ms: 0,
        };
        audio.background_row = 7;
        audio.pending.push(Pending::Background(
            spec,
            Some(wolf_media::Clip::new(8000, vec![[1, 2]; 8]).unwrap()),
            100,
            fade,
        ));
        let saved = audio.capture_state().unwrap();
        let mut restored = NativeAudio::new(&[0; 40]).unwrap();
        let mut reads = 0;
        restored
            .restore_state(&saved, |path| {
                assert_eq!(path, "test.wav");
                reads += 1;
                let mut bytes = b"RIFF".to_vec();
                bytes.extend_from_slice(&68u32.to_le_bytes());
                bytes.extend_from_slice(b"WAVEfmt ");
                bytes.extend_from_slice(&16u32.to_le_bytes());
                bytes.extend_from_slice(&1u16.to_le_bytes());
                bytes.extend_from_slice(&2u16.to_le_bytes());
                bytes.extend_from_slice(&8000u32.to_le_bytes());
                bytes.extend_from_slice(&32000u32.to_le_bytes());
                bytes.extend_from_slice(&4u16.to_le_bytes());
                bytes.extend_from_slice(&16u16.to_le_bytes());
                bytes.extend_from_slice(b"data");
                bytes.extend_from_slice(&32u32.to_le_bytes());
                bytes.extend_from_slice(&[0; 32]);
                Ok(bytes)
            })
            .unwrap();
        assert_eq!(reads, 1);
        assert_eq!(restored.background_row, 7);
        let Pending::Background(_, Some(clip), _, actual) = &mut restored.pending[0] else {
            panic!("missing reloaded pending BGM")
        };
        assert_eq!(clip.frames().len(), 8);
        let mut expected = expected;
        for _ in 0..30 {
            assert_eq!(actual.tick(), expected.tick());
        }
    }
    fn fixture(ty: usize, row: usize, field: usize) -> Result<NativeValue, String> {
        let path = match (ty, row) {
            (1, 0) => "BGM/title_1520344.ogg",
            (3, 0) => "SE/cursor_107925.ogg",
            (3, 1) => "SE/button_66418.ogg",
            _ => return Err("missing audio row".into()),
        };
        Ok(if field == 0 {
            NativeValue::Text(path.into())
        } else {
            NativeValue::Number(0)
        })
    }
    #[test]
    fn actual_title_forms_select_database_audio_and_preserve_duration() {
        let bgm = specification(&[0, 60, 0, 0], None, true, fixture).unwrap();
        assert_eq!(bgm.path.as_deref(), Some("BGM/title_1520344.ogg"));
        assert_eq!(
            (bgm.kind, bgm.row, bgm.volume, bgm.duration),
            (0, 0, 100, 60)
        );
        for (flags, expected) in [
            (0x20, "SE/cursor_107925.ogg"),
            (0x120, "SE/button_66418.ogg"),
        ] {
            let se = specification(&[flags, 0, 0, 0, 0, 0, 0, 0], None, true, fixture).unwrap();
            assert_eq!(se.path.as_deref(), Some(expected));
            assert_eq!(se.volume, 100);
        }
    }
    #[test]
    fn dialog_variable_database_row_uses_resolved_operand() {
        let spec = specification(&[0x0100_0020, 0, 1, 0, 0, 0, 0], None, true, fixture).unwrap();
        assert_eq!(spec.path.as_deref(), Some("SE/button_66418.ogg"));
        assert_eq!(spec.row, 1);
        assert_eq!(specification(&[0x0100_0020, 0, 1, 0, 0, 0, 1], None, true, fixture).unwrap().se_group, 1);
    }
    #[test]
    fn script_seek_and_cv_pan_survive_pending_roundtrip_without_redecoding() {
        let spec = specification(&[0x0200_0020, 0, 0, 0, 75, 100, 3, 89], Some("cv.wav".into()), false, fixture).unwrap();
        assert_eq!((spec.pan, spec.se_group), (89, 3));
        let bytes = wave_fixture();
        let mut reads = 0;
        let mut audio = NativeAudio::empty(false);
        for _ in 0..2 {
            audio.queue(spec.clone(), 100, |_| { reads += 1; Ok(bytes.clone()) }).unwrap();
        }
        assert_eq!(reads, 1, "ordinary play must populate the decoded PCM cache");
        let saved = audio.capture_state().unwrap();
        let mut restored = NativeAudio::empty(false);
        restored.restore_state(&saved, |_| Ok(bytes.clone())).unwrap();
        let Pending::Effect(spec, _, _) = &restored.pending[0] else { panic!("missing CV") };
        assert_eq!((spec.pan, spec.se_group), (89, 3));
        let bgm = specification(&[0x0200_0000, 30, 0, 48246, 100, 100, 0, 0], Some("bgm.wav".into()), true, fixture).unwrap();
        audio.queue(bgm.clone(), 100, |_| Ok(bytes.clone())).unwrap();
        let mut seek = bgm;
        seek.seek_ms = 1000;
        audio.queue(seek, 100, |_| panic!("same row redecoded")).unwrap();
        let Pending::Background(spec, _, _, fade) = audio.pending.last().unwrap() else { panic!("missing BGM") };
        assert_eq!((spec.seek_ms, fade.total), (1000, 31));
    }
    fn wave_fixture() -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&68u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&8000u32.to_le_bytes());
        bytes.extend_from_slice(&32000u32.to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&32u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 32]);
        bytes
    }
    #[test]
    fn delayed_group_stop_is_not_discarded_when_master_is_muted() {
        let spec = specification(&[-224, 1, 0, 0, 0, 0, 3, 0], None, false, fixture).unwrap();
        let mut audio = NativeAudio::empty(false);
        audio.queue(spec, 0, |_| panic!("stop decoded audio")).unwrap();
        assert_eq!(audio.pending.len(), 1);
        let snapshot = audio.capture_state().unwrap();
        audio.restore_state(&snapshot, |_| panic!("delayed stop must restore without a resource path")).unwrap();
        audio.tick().unwrap();
        assert_eq!(audio.pending.len(), 1);
        audio.tick().unwrap();
        assert!(audio.pending.is_empty());
    }
    #[test]
    fn delayed_stop_cancels_matching_sounds_on_both_sides_of_the_work_queue() {
        for stop_first in [false, true] {
            for group in [3, -1] {
                let mut audio = NativeAudio::empty(false);
                let sound = specification(&[0x0200_0020, 5, 0, 0, 100, 100, 3, 0], Some("queued.wav".into()), false, fixture).unwrap();
                let stop = specification(&[-224, 1, 0, 0, 0, 0, group, 0], None, false, fixture).unwrap();
                if stop_first { audio.queue(stop.clone(), 0, |_| unreachable!()).unwrap(); }
                audio.queue(sound.clone(), 100, |_| Ok(wave_fixture())).unwrap();
                let mut other = sound;
                other.se_group = 4;
                audio.queue(other, 100, |_| unreachable!()).unwrap();
                if !stop_first { audio.queue(stop, 0, |_| unreachable!()).unwrap(); }
                audio.tick().unwrap();
                audio.tick().unwrap();
                let groups: Vec<_> = audio.pending.iter().map(|pending| match pending {
                    Pending::Effect(spec, _, _) => spec.se_group,
                    _ => panic!("unexpected background request"),
                }).collect();
                assert_eq!(groups, if group == -1 { vec![] } else { vec![4] }, "stop_first={stop_first} group={group}");
            }
        }
    }
    #[test]
    fn native_restore_uses_active_bgs_channels_and_their_saved_pan() {
        let mut saved = crate::native_save_payload::SaveSnapshot::default();
        for (offset, value) in [(2268, -1i32), (2648, 100), (2652, 100), (2656, 0), (3472, 0), (2612, 100), (2616, 60)] {
            saved.system_fields.insert(offset, value.to_le_bytes().to_vec());
        }
        saved.system_strings.insert(2624, Vec::new());
        saved.background_sound_ids.extend([(0, -1), (3, -3)]);
        saved.background_sound_rows.extend([
            (0, b"stopped.wav".to_vec(), vec![100, 100, 0, 0]),
            (3, b"ambience.wav".to_vec(), vec![80, 100, 0, -89]),
        ]);
        let mut audio = NativeAudio::empty(false);
        let mut reads = Vec::new();
        audio.restore_native(&saved, |path| { reads.push(path.to_owned()); Ok(wave_fixture()) }).unwrap();
        assert_eq!(reads, ["ambience.wav"]);
        assert_eq!(audio.ambience.keys().copied().collect::<Vec<_>>(), [3]);
        let Pending::Background(spec, _, master, fade) = &audio.ambience[&3].pending[0] else { panic!("missing BGS restore") };
        assert_eq!((spec.row, spec.pan, *master, fade.to, fade.total), (-3, -89, 60, 48, 21));
    }
    #[test]
    fn map_audio_metadata_keeps_or_stops_and_resets_only_new_channel_fade() {
        let mut audio = NativeAudio::empty(false);
        let spec = specification(&[0x0200_0010, 10, 0, 0, 70, 100, 0, 0, 3], Some("rain.wav".into()), false, fixture).unwrap();
        audio.queue(spec, 100, |_| Ok(wave_fixture())).unwrap();
        audio.map_change(1, -2, None, 100, fixture, |_| panic!("keep loaded audio")).unwrap();
        assert_eq!(audio.ambience[&3].pending.len(), 1);
        audio.map_change(1, -3, Some("map.wav".into()), 80, fixture, |_| Ok(wave_fixture())).unwrap();
        let Pending::Background(spec, _, _, fade) = audio.ambience[&3].pending.last().unwrap() else { panic!("missing nonzero channel stop") };
        assert_eq!((spec.row, spec.duration, fade.total), (-1, 5, 6));
        let Pending::Background(spec, _, _, fade) = &audio.ambience[&0].pending[0] else { panic!("missing map BGS") };
        assert_eq!((spec.duration, fade.to, fade.total, fade.remaining), (5, 80, 1, 0));
        audio.map_change(0, -1, None, 100, fixture, |_| panic!("map stop loaded audio")).unwrap();
        let Pending::Background(spec, clip, _, _) = &audio.pending[0] else { panic!("missing map BGM stop") };
        assert_eq!(spec.row, -1);
        assert!(clip.is_none());
    }
    #[test]
    fn zero_volume_compatibility_is_configured_and_file_volume_is_preserved() {
        assert_eq!(
            specification(&[0, 60, 0, 0], None, false, fixture)
                .unwrap()
                .volume,
            0
        );
        assert_eq!(
            specification(
                &[0x0200_0020, 0, 0, 0, 70, 100, 0, 0],
                Some("SE/logo_862730.ogg".into()),
                true,
                fixture
            )
            .unwrap()
            .volume,
            70
        );
        assert_eq!(output_gain(70, 50, 2).unwrap(), 89);
    }
    #[test]
    fn stop_is_the_native_sentinel_not_row_zero() {
        let stop = specification(&[0x00ff_ff00, 60, 0, 0], None, true, fixture).unwrap();
        assert_eq!(stop.row, -1);
        assert!(stop.path.is_none());
    }
    #[test]
    fn native_fade_applies_before_decrement_and_keeps_endpoint_tick() {
        let mut fade = Fade::new(0, 100, 60);
        assert_eq!(fade.tick(), 0);
        for elapsed in 1..=61 {
            assert_eq!(fade.tick(), 100 * elapsed / 61);
        }
        assert!(fade.done());
        assert_eq!(fade.tick(), 100);
        let mut stop = Fade::new(100, 0, 1);
        assert_eq!([stop.tick(), stop.tick(), stop.tick()], [100, 50, 0]);
        assert!(stop.done());
    }
    #[test]
    fn unsupported_audio_modes_fail_before_loading() {
        for args in [
            [3, 0, 0, 0],
            [0x10, 0, 0, 0],
            [0, -1, 0, 0],
        ] {
            assert!(specification(&args, None, true, fixture).is_err());
        }
    }
    #[test]
    fn master_retarget_keeps_active_timer_and_applies_finished_fade_immediately() {
        let mut fade = Fade::new(0, 100, 3);
        assert_eq!([fade.tick(), fade.tick()], [0, 25]);
        fade.retarget(50);
        assert_eq!((fade.total, fade.remaining), (4, 2));
        assert_eq!([fade.tick(), fade.tick(), fade.tick()], [25, 37, 50]);
        fade.retarget(20);
        assert_eq!((fade.total, fade.remaining), (1, 0));
        assert_eq!(fade.tick(), 20);
    }
    #[test]
    fn current_archive_audio_rows_and_configuration_match_static_evidence() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        if !root.join("Data/BasicData.wolf").exists() {
            eprintln!("private game fixture absent");
            return;
        }
        let archive = wolf_assets::wolf350::Archive::open(
            std::fs::read(root.join("Data/BasicData.wolf")).unwrap(),
            Default::default(),
        )
        .unwrap();
        println!("current BasicData blake3={}", archive.source_blake3);
        for member in ["Game.dat", "SysDatabase.dat"] {
            let current = archive.read(member).unwrap();
            println!("current {member} blake3={}", current.id.content_blake3);
        }
        let bytes = crate::assets::read_member(&root, "BasicData/Game.dat").unwrap();
        let config = wolf_format::parse_game(&bytes, &Default::default()).unwrap();
        assert!(
            NativeAudio::new(&config.configuration)
                .unwrap()
                .zero_is_default
        );
        let old =
            std::fs::read(root.join("wolf-rs/validation/archive-reference/BasicData/Game.dat"))
                .unwrap();
        let old_config = wolf_format::parse_game(&old, &Default::default()).unwrap();
        let changed: Vec<_> = config
            .configuration
            .iter()
            .zip(&old_config.configuration)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i, *b, *a))
            .collect();
        println!("Game.dat configuration differences (index,reference,current)={changed:?}; current audio config39={}", config.configuration[39]);
        let bytes = crate::assets::read_member(&root, "BasicData/SysDatabase.dat").unwrap();
        let database = wolf_format::parse_database(&bytes, &Default::default()).unwrap();
        for (ty, row, path) in [
            (1, 0, "BGM/title_1520344.ogg"),
            (3, 0, "SE/cursor_107925.ogg"),
            (3, 1, "SE/button_66418.ogg"),
        ] {
            assert_eq!(
                database.value.types[ty]
                    .string(row, 0)
                    .unwrap()
                    .text()
                    .unwrap(),
                path
            );
            assert_eq!(
                (1..=3)
                    .map(|field| database.value.types[ty].integer(row, field).unwrap())
                    .collect::<Vec<_>>(),
                [0, 0, 0]
            );
        }
    }
}
