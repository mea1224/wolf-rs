//! Bounded structural reader for version-158/159 native save snapshots.
//! Reader order is 9AF230's query path; this does not restore native execution.
use std::collections::BTreeMap;

const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 1_000_000;

// Each case is one mov/movsx/movzx from system core followed immediately by
// return; verified jump-table bytes in native-save-restore/manifest.json.
const SYSTEM_SCALARS: &[(i32, usize, usize)] = &[
    (0, 2288, 4),
    (1, 2292, 4),
    (2, 2296, 4),
    (3, 2300, 4),
    (4, 2304, 4),
    (5, 2418, 2),
    (6, 2420, 2),
    (7, 2320, 4),
    (8, 2364, 4),
    (9, 2440, 4),
    (10, 2448, 4),
    (11, 2444, 4),
    (15, 2404, 4),
    (16, 2408, 4),
    (17, 2308, 4),
    (18, 2312, 4),
    (19, 2316, 4),
    (26, 3112, 4),
    (27, 3116, 4),
    (28, 2560, 4),
    (29, 2256, 4),
    (31, 2280, 4),
    (32, 2284, 4),
    (34, 2816, 4),
    (36, 2400, 2),
    (37, 2424, 2),
    (38, 2436, 4),
    (40, 2432, 2),
    (41, 2434, 2),
    (42, 2820, 4),
    (59, 2528, 4),
    (60, 2532, 4),
    (61, 2380, 4),
    (62, 2384, 4),
    (63, 2388, 4),
    (64, 2392, 4),
    (65, 2396, 4),
    (66, 2368, 4),
    (67, 2372, 4),
    (68, 2376, 4),
    (73, 2464, 4),
    (100, 2612, 4),
    (101, 2616, 4),
    (102, 2620, 4),
    (122, 2888, 4),
    (123, 2884, 4),
    (125, 2892, 4),
    (127, 2896, 4),
    (129, 2900, 4),
    (130, 2904, 4),
    (131, 2908, 4),
    (132, 2912, 4),
    (134, 2916, 1),
    (135, 2917, 1),
];

#[derive(Debug, Default, Clone)]
#[allow(dead_code)] // Retained snapshot fields await native selector/full-restore integration.
pub(crate) struct SaveSnapshot {
    pub version: u16,
    pub map_id: i32,
    pub map_x: i32,
    pub map_y: i32,
    pub width: usize,
    pub height: usize,
    pub layers: usize,
    pub tiles: Option<Vec<i32>>,
    pub player: SavedCharacter,
    pub followers: Vec<SavedCharacter>,
    pub map_characters: Vec<SavedCharacter>,
    pub map_numbers: Vec<Vec<Vec<i32>>>,
    pub native_rng: u32,
    pub picture_records: Vec<Vec<u8>>,
    pub pictures: Vec<SavedPicture>,
    /// Native 95C020 destination offsets, not serialized offsets.
    pub system_fields: BTreeMap<usize, Vec<u8>>,
    pub system_strings: BTreeMap<usize, Vec<u8>>,
    pub system_string_array: Vec<Vec<u8>>,
    pub background_sound_rows: Vec<(i32, Vec<u8>, Vec<i32>)>,
    /// Native system+2272 map: BGS channel -> active database/file row.
    pub background_sound_ids: BTreeMap<i32, i32>,
    pub numeric_banks: Vec<Vec<i32>>,
    pub strings: Vec<Vec<u8>>,
    pub common_numbers: Vec<Vec<i32>>,
    pub common_strings: Vec<Vec<Vec<u8>>>,
    pub mutable_database: Vec<SavedDatabaseType>,
    pub serialized_sys140: i32,
    pub sections: Vec<(&'static str, usize, usize)>,
    raw: Vec<u8>,
    number_offsets: BTreeMap<i32, usize>,
    text_spans: BTreeMap<i32, (usize, usize)>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct SavedCharacter {
    pub fields: BTreeMap<usize, i32>,
    pub routes: Vec<wolf_format::MoveData>,
    pub route_strings: Vec<Vec<Vec<u8>>>,
    pub id: i32,
    pub x2: i32,
    pub y2: i32,
    pub direction: i16,
    pub frame: i16,
    pub opacity: u16,
    pub blend: u16,
    pub tile_id: i32,
    pub graphic: Vec<u8>,
    /// Complete version-specific record, retained for movement/page restoration.
    #[allow(dead_code)] // Preserve opaque native bytes until all character fields are interpreted.
    pub raw: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct SavedPicture {
    /// 981750 native destination byte offset -> serialized integer value.
    /// Double destinations retain their integer source exactly.
    pub fields: BTreeMap<usize, i32>,
    pub source: Vec<u8>,
    pub effect_arrays: Vec<Vec<i32>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Preserve parsed DB rows; snapshot validation must not discard their structure.
pub(crate) struct SavedDatabaseType {
    pub tag: i32,
    pub fields: Vec<i32>,
    pub rows: Vec<(Vec<i32>, Vec<Vec<u8>>)>,
}

impl SaveSnapshot {
    #[allow(dead_code)] // Destination-offset accessor for upcoming native save selectors.
    pub fn system_i32(&self, offset: usize) -> Option<i32> {
        self.system_fields
            .get(&offset)
            .and_then(|b| b.as_slice().try_into().ok())
            .map(i32::from_le_bytes)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }

    fn system_scalar(&self, selector: i32) -> Option<i32> {
        if (52..=54).contains(&selector) {
            return self.system_i32(2536 + (selector - 52) as usize * 4);
        }
        if (55..=57).contains(&selector) {
            let mask = self.system_i32(2548 + (selector - 55) as usize * 4)? as u32;
            return Some(
                (301..=320)
                    .find(|code| mask == 1 << (code - 297))
                    .unwrap_or(0),
            );
        }
        if selector == 140 {
            return Some(self.serialized_sys140);
        }
        let &(_, offset, width) = SYSTEM_SCALARS
            .iter()
            .find(|&&(key, _, _)| key == selector)?;
        let (&start, bytes) = self.system_fields.range(..=offset).next_back()?;
        let value = bytes.get(offset - start..offset - start + width)?;
        match width {
            1 => Some(i32::from(value[0])),
            2 => Some(i32::from(i16::from_le_bytes(value.try_into().ok()?))),
            4 => Some(i32::from_le_bytes(value.try_into().ok()?)),
            _ => None,
        }
    }

    /// 9E4C84..9E4CE8: banks contain 100,000 numbered variables each.
    pub fn number(&self, selector: i32) -> Result<i32, String> {
        if selector == 9_000_024 {
            return Ok(1);
        }
        if (9_000_000..9_100_000).contains(&selector) {
            return self
                .system_scalar(selector - 9_000_000)
                .ok_or_else(|| format!("unsupported saved system selector {selector}"));
        }
        if let Some(&offset) = self.number_offsets.get(&selector) {
            return Ok(i32::from_le_bytes(
                self.raw[offset..offset + 4].try_into().unwrap(),
            ));
        }
        // 9E52F1 -> 946830: an existing string DB field read through the
        // numeric accessor returns zero (946A62..946A76), not atoi.
        if let Some(at) = wolf_vm::native_values::DatabaseAddress::decode(selector) {
            if at.kind == wolf_vm::native_values::DatabaseKind::Mutable {
                if let Some(ty) = self.mutable_database.get(at.type_id) {
                    if at.row < ty.rows.len()
                        && ty
                            .fields
                            .get(at.field)
                            .is_some_and(|kind| (2000..3000).contains(kind))
                    {
                        return Ok(0);
                    }
                }
            }
        }
        if (2_000_000..3_000_000).contains(&selector)
            || (15_000_000..16_000_000).contains(&selector) && !(5..=9).contains(&(selector % 100))
        {
            return Ok(0);
        }
        Err(format!("unsupported saved numeric selector {selector}"))
    }

    pub fn text(&self, selector: i32) -> Result<String, String> {
        let Some(&(start, end)) = self.text_spans.get(&selector) else {
            if (3_000_000..4_000_000).contains(&selector)
                || (15_000_000..16_000_000).contains(&selector)
                    && (5..=9).contains(&(selector % 100))
            {
                return Ok(String::new());
            }
            return Err(format!("unsupported saved text selector {selector}"));
        };
        std::str::from_utf8(&self.raw[start + 4..end - 1])
            .map(str::to_owned)
            .map_err(|e| format!("saved text {selector}: {e}"))
    }

    pub fn set_number(&mut self, selector: i32, value: i32) -> Result<(), String> {
        let offset = *self
            .number_offsets
            .get(&selector)
            .ok_or_else(|| format!("saved numeric selector {selector} has no serialized cell"))?;
        let mut raw = self.raw.clone();
        raw[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        *self = parse(&raw, self.version)?;
        Ok(())
    }

    pub fn set_text(&mut self, selector: i32, value: &str) -> Result<(), String> {
        if value.contains('\0') || value.len() > 16 * 1024 * 1024 {
            return Err("saved text byte bound or embedded NUL".into());
        }
        let &(start, end) = self
            .text_spans
            .get(&selector)
            .ok_or_else(|| format!("saved text selector {selector} has no serialized cell"))?;
        let mut replacement = Vec::with_capacity(value.len() + 5);
        replacement.extend_from_slice(&((value.len() + 1) as i32).to_le_bytes());
        replacement.extend_from_slice(value.as_bytes());
        replacement.push(0);
        let mut raw = self.raw.clone();
        raw.splice(start..end, replacement);
        *self = parse(&raw, self.version)?;
        Ok(())
    }

    /// Apply saved variables and mutable cells to a preconfigured world. This
    /// does not replace scene/media/execution; the application commits those.
    pub fn restore_world(
        &self,
        base: &wolf_vm::runtime::World,
    ) -> Result<wolf_vm::runtime::World, String> {
        use wolf_vm::native_values::{DatabaseAddress, DatabaseKind, NativeValue};
        let mut world = base.clone();
        world.globals = Default::default();
        world.commons = Default::default();
        world.events.clear();
        world.current_map = Some(self.map_id);
        world.map_events.clear();
        for (map, events) in self.map_numbers.iter().enumerate() {
            if map as i32 == self.map_id { continue; }
            let bank = world.map_events.entry(map as i32).or_default();
            for (event, values) in events.iter().enumerate() {
                bank.entry(event as u64).or_default().numbers.extend(
                    values.iter().enumerate().map(|(slot, &value)| (slot as i32, value)));
            }
        }
        for &(selector, _, _) in SYSTEM_SCALARS {
            if let Some(value) = self.system_scalar(selector) {
                world.systems.numbers.insert(selector, value);
            }
        }
        for selector in [52, 53, 54, 55, 56, 57, 140] {
            if let Some(value) = self.system_scalar(selector) {
                world.systems.numbers.insert(selector, value);
            }
        }
        for (&selector, &offset) in &self.number_offsets {
            let value = i32::from_le_bytes(self.raw[offset..offset + 4].try_into().unwrap());
            if (2_000_000..3_000_000).contains(&selector) {
                world.globals.numbers.insert(selector - 2_000_000, value);
            } else if (15_000_000..16_000_000).contains(&selector) {
                world.commons.numbers.insert(selector - 15_000_000, value);
            } else if let Some(at) = DatabaseAddress::decode(selector) {
                if at.kind == DatabaseKind::Mutable {
                    world
                        .databases
                        .write_mutable(at, NativeValue::Number(value))?;
                }
            }
        }
        for &selector in self.text_spans.keys() {
            let value = self.text(selector)?;
            if (3_000_000..4_000_000).contains(&selector) {
                world.globals.strings.insert(selector - 3_000_000, value);
            } else if (15_000_000..16_000_000).contains(&selector) {
                world.commons.strings.insert(selector - 15_000_000, value);
            } else if let Some(at) = DatabaseAddress::decode(selector) {
                if at.kind == DatabaseKind::Mutable {
                    world
                        .databases
                        .write_mutable(at, NativeValue::Text(value))?;
                }
            }
        }
        // 9E4A41 uses the current map ID when the explicit map selector is -1;
        // 9E4B4E..9E4BE4 then indexes event and self slot within that map.
        if let Ok(map) = usize::try_from(self.map_id) {
            if let Some(events) = self.map_numbers.get(map) {
                for (event, values) in events.iter().enumerate() {
                    let slots = world.events.entry(event as u64).or_default();
                    slots.numbers.extend(
                        values
                            .iter()
                            .enumerate()
                            .map(|(slot, &value)| (slot as i32, value)),
                    );
                }
            }
        }
        Ok(world)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    section: &'static str,
    items_left: usize,
}
impl<'a> Cursor<'a> {
    fn error(&self, message: &str) -> String {
        format!(
            "native save {} offset {:#x}: {message}",
            self.section, self.pos
        )
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| self.error("size overflow"))?;
        let value = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| self.error("truncated payload"))?;
        self.pos = end;
        Ok(value)
    }
    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, String> {
        let value = self.i32()?;
        if value < 0 || value as usize > MAX_ITEMS {
            return Err(self.error("negative or excessive count"));
        }
        self.items_left = self
            .items_left
            .checked_sub(value as usize)
            .ok_or_else(|| self.error("aggregate item budget exceeded"))?;
        Ok(value as usize)
    }
    fn short_count(&mut self) -> Result<usize, String> {
        let value = self.u16()?;
        if value > i16::MAX as u16 {
            return Err(self.error("negative short count"));
        }
        Ok(value as usize)
    }
    fn words(&mut self, count: usize) -> Result<Vec<i32>, String> {
        let n = count
            .checked_mul(4)
            .ok_or_else(|| self.error("word count overflow"))?;
        Ok(self
            .take(n)?
            .chunks_exact(4)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
            .collect())
    }
    fn skip_words(&mut self, count: usize) -> Result<(), String> {
        self.skip(
            count
                .checked_mul(4)
                .ok_or_else(|| self.error("word count overflow"))?,
        )
    }
    fn string(&mut self, short: bool) -> Result<Vec<u8>, String> {
        let n = if short {
            self.short_count()?
        } else {
            self.count()?
        };
        let bytes = self.take(n)?;
        if bytes.last() != Some(&0) {
            return Err(self.error("string is not NUL terminated"));
        }
        Ok(bytes[..bytes.len() - 1].to_vec())
    }
    fn vector(&mut self) -> Result<Vec<i32>, String> {
        let n = self.count()?;
        self.words(n)
    }
}

/// `bytes` starts immediately after the outer version word and includes trailer.
/// Unsupported versions, malformed sections and any trailing data are errors.
pub(crate) fn parse(bytes: &[u8], version: u16) -> Result<SaveSnapshot, String> {
    if !matches!(version, 158 | 159) {
        return Err(format!("unsupported native snapshot version {version}"));
    }
    if bytes.len() > MAX_PAYLOAD {
        return Err("native snapshot exceeds 64 MiB".into());
    }
    let mut c = Cursor {
        bytes,
        pos: 0,
        section: "94BB60 dynamic arrays",
        items_left: MAX_ITEMS,
    };
    let mut out = SaveSnapshot {
        version,
        raw: bytes.to_vec(),
        ..Default::default()
    };
    let start = c.pos;
    dynamic_arrays(&mut c)?;
    out.sections.push((c.section, start, c.pos));
    c.section = "9FBB00 map";
    let start = c.pos;
    map_state(&mut c, &mut out)?;
    out.sections.push((c.section, start, c.pos));
    c.section = "95C020 system";
    let start = c.pos;
    system_state(&mut c, &mut out)?;
    out.sections.push((c.section, start, c.pos));
    c.section = "9E6240 variables";
    let start = c.pos;
    variables(&mut c, &mut out)?;
    out.sections.push((c.section, start, c.pos));
    c.section = "9F5EB0 party";
    let start = c.pos;
    out.player = character(&mut c)?;
    for _ in 0..c.count()? {
        out.followers.push(character(&mut c)?);
    }
    c.skip(2)?;
    c.vector()?;
    let n = c.count()?;
    c.skip_words(n.checked_mul(2).ok_or("party pair count overflow")?)?;
    out.sections.push((c.section, start, c.pos));
    c.section = "9A6710 pictures";
    let start = c.pos;
    for _ in 0..c.short_count()? {
        let start = c.pos;
        out.pictures.push(picture(&mut c)?);
        out.picture_records.push(c.bytes[start..c.pos].to_vec());
    }
    for _ in 0..c.count()? {
        c.string(false)?;
        c.skip(12)?;
    }
    out.sections.push((c.section, start, c.pos));
    c.section = "948A10 mutable database";
    let start = c.pos;
    mutable_database(&mut c, &mut out)?;
    out.sections.push((c.section, start, c.pos));
    c.section = "943DF0 tagged values";
    let start = c.pos;
    if c.u8()? == 1 {
        for _ in 0..c.count()? {
            let tag = c.u8()?;
            c.skip(if tag < 250 { 1 } else { 4 })?;
        }
    }
    out.sections.push((c.section, start, c.pos));
    c.section = "trailer";
    if c.u8()? != 0x19 {
        return Err(c.error("expected 0x19 trailer"));
    }
    if c.pos != c.bytes.len() {
        return Err(c.error("trailing bytes after snapshot"));
    }
    Ok(out)
}

fn dynamic_arrays(c: &mut Cursor<'_>) -> Result<(), String> {
    for kind in 0..2 {
        for _ in 0..c.count()? {
            if kind == 0 {
                c.i32()?;
            } else {
                c.string(false)?;
            }
            for _ in 0..c.u16()? {
                let n = c.u16()? as usize;
                c.skip_words(n)?;
            }
        }
    }
    Ok(())
}

fn map_state(c: &mut Cursor<'_>, out: &mut SaveSnapshot) -> Result<(), String> {
    out.map_id = c.i32()?;
    out.map_x = c.i32()?;
    out.map_y = c.i32()?;
    let width = c.count()?;
    let height = c.count()?;
    let layers = c.count()?;
    out.width = width;
    out.height = height;
    out.layers = layers;
    // 9FBD6D peeks the first tile; only the -1 branch consumes the sentinel.
    let saved = c.pos;
    if c.i32()? != -1 {
        c.pos = saved;
        let n = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(layers))
            .ok_or("map tile size overflow")?;
        out.tiles = Some(c.words(n)?);
    }
    c.string(false)?;
    for _ in 0..31 {
        c.string(false)?;
    }
    c.vector()?;
    for _ in 0..c.count()? {
        out.map_characters.push(character(c)?);
        if c.u8()? != 0 {
            c.string(true)?;
        }
    }
    c.skip(11 * 4)?;
    c.vector()?;
    Ok(())
}

fn character(c: &mut Cursor<'_>) -> Result<SavedCharacter, String> {
    // 9EE140, version 158: fixed fields interleaved with bounded collections.
    let start = c.pos;
    let id = c.i32()?;
    let x2 = c.i32()?;
    let y2 = c.i32()?;
    let mut fields = BTreeMap::new();
    fields.insert(0xC0, c.i32()?);
    fields.insert(0xBA, i32::from(c.u8()?));
    c.skip(2 * 4)?; // Version158 virtual property setters, not render X/Y.
    let tile_id = c.i32()?; // 9EE3CE writes actor+44.
    let graphic = c.string(true)?;
    let opacity = c.u16()?; // actor+48, 9EE57F.
    let blend = c.u16()?; // actor+4A, 9EE5C0.
    let direction = c.u16()? as i16;
    let frame = c.u16()? as i16; // 9EE642 writes actor+4E.
    fields.insert(0x56, i32::from(c.u8()?));
    fields.insert(0x57, i32::from(c.u8()?));
    // 9EE6E0/9EE6ED call 56ECE0, a stream reader, not a vector resize.
    let mut routes = Vec::new();
    let mut route_strings = Vec::new();
    for _ in 0..2 {
        let unknown = c.take(5)?.to_vec();
        let flags = c.u8()?;
        let n = c.count()?;
        if n > 65536 {
            return Err(c.error("too many movement commands"));
        }
        let mut commands = Vec::new();
        let mut strings = Vec::new();
        for _ in 0..n {
            let start = c.pos;
            let opcode = c.u8()?;
            let args = c.u8()? as usize;
            let args = c.words(args)?;
            let len = c.u8()? as usize;
            // 56E580 length is raw bytes and may be zero; it is not the
            // NUL-inclusive string framing used by other snapshot readers.
            strings.push(c.take(len)?.to_vec());
            commands.push(wolf_format::RouteCommand { opcode, args, span: wolf_format::Span { start, end: c.pos } });
        }
        routes.push(wolf_format::MoveData { unknown, flags, routes: commands });
        route_strings.push(strings);
    }
    for offset in [0x5C, 0x5E, 0x62, 0x60, 0xA4, 0xA6] { fields.insert(offset, i32::from(c.u16()? as i16)); }
    for offset in [0xAA, 0xAB, 0xAC] { fields.insert(offset, i32::from(c.u8()?)); }
    c.vector()?;
    for offset in [0xA8, 0xA9] { fields.insert(offset, i32::from(c.u8()?)); }
    c.skip(4 * 4)?;
    fields.insert(0xB0, c.i32()?);
    c.skip(3 * 4)?;
    for _ in 0..c.short_count()? {
        c.i32()?;
        c.string(true)?;
    }
    c.skip(4 + 1)?;
    for offset in [0x158, 0x15C, 0x148] { fields.insert(offset, c.i32()?); }
    fields.insert(0x50, i32::from(c.u16()? as i16));
    c.skip(2)?;
    for _ in 0..c.count()? {
        let n = c.count()?;
        c.skip_words(n.checked_mul(2).ok_or("character map overflow")?)?;
    }
    c.skip(4)?;
    if c.i32()? == 1 {
        c.skip(6 * 4)?;
    }
    c.skip(4)?;
    if c.u8()? != 0 {
        c.skip(8 * 4)?;
    }
    Ok(SavedCharacter {
        fields,
        routes,
        route_strings,
        id,
        x2,
        y2,
        direction,
        frame,
        opacity,
        blend,
        tile_id,
        graphic,
        raw: c.bytes[start..c.pos].to_vec(),
    })
}

fn variables(c: &mut Cursor<'_>, out: &mut SaveSnapshot) -> Result<(), String> {
    out.native_rng = c.i32()? as u32; // 9E62EE: 942820(mode2, saved_state).
    for _ in 0..c.count()? {
        let mut events = Vec::new();
        for _ in 0..c.count()? {
            let n = c.u8()? as usize;
            events.push(c.words(n)?);
        }
        out.map_numbers.push(events);
    }
    let n = c.count()?;
    if n > 9999 {
        return Err(c.error("too many variable banks"));
    }
    for bank in 0..n {
        let count = c.count()?;
        for slot in 0..count {
            let key = 2_000_000i64 + bank as i64 * 100_000 + slot as i64;
            if key < 3_000_000 {
                out.number_offsets.insert(key as i32, c.pos + slot * 4);
            }
        }
        out.numeric_banks.push(c.words(count)?);
    }
    for slot in 0..c.count()? {
        let start = c.pos;
        out.strings.push(c.string(false)?);
        out.text_spans
            .insert(3_000_000 + slot as i32, (start, c.pos));
    }
    let n = c.count()?;
    if n > 10000 {
        return Err(c.error("too many common numeric records"));
    }
    for common in 0..n {
        let n = c.u8()? as usize;
        for slot in 0..n {
            if !(5..=9).contains(&slot) {
                out.number_offsets.insert(
                    15_000_000 + common as i32 * 100 + slot as i32,
                    c.pos + slot * 4,
                );
            }
        }
        out.common_numbers.push(c.words(n)?);
    }
    let n = c.count()?;
    if n > 10000 {
        return Err(c.error("too many common string records"));
    }
    for common in 0..n {
        let mut values = Vec::new();
        for slot in 0..c.u8()? {
            let start = c.pos;
            values.push(c.string(false)?);
            out.text_spans.insert(
                15_000_000 + common as i32 * 100 + 5 + i32::from(slot),
                (start, c.pos),
            );
        }
        out.common_strings.push(values);
    }
    for _ in 0..c.count()? {
        c.i32()?;
        c.u8()?;
    }
    Ok(())
}

fn picture(c: &mut Cursor<'_>) -> Result<SavedPicture, String> {
    // 981750's version158/159 branch; native destinations, not guessed names.
    let mut out = SavedPicture::default();
    out.fields.insert(772, c.i32()?);
    out.fields.insert(784, i32::from(c.u8()?));
    out.fields.insert(785, i32::from(c.u8()?));
    out.fields.insert(768, i32::from(c.u16()? as i16));
    out.fields.insert(764, i32::from(c.u8()?));
    out.fields.insert(765, i32::from(c.u8()?));
    out.source = c.string(true)?;
    // 981C40..981DE5: position doubles and resource division dimensions.
    picture_words(c, &mut out, &[456, 472, 464, 480, 776, 780])?;
    picture_words(
        c,
        &mut out,
        &[488, 496, 500, 504, 512, 516, 544, 560, 552, 568, 520, 524],
    )?;
    picture_words(c, &mut out, &[576, 580, 584, 588, 592, 596])?;
    picture_words(
        c,
        &mut out,
        &[
            916, 920, 928, 924, 932, 940, 936, 944, 952, 948, 984, 988, 992, 996, 1012, 1008, 1000,
            1004,
        ],
    )?;
    for offset in [766, 767, 736] {
        out.fields.insert(offset, i32::from(c.u8()?));
    }
    // Four groups of four integer values converted to doubles natively.
    for base in [608, 672, 640, 704] {
        for item in 0..4 {
            out.fields.insert(base + item * 8, c.i32()?);
        }
    }
    picture_words(
        c,
        &mut out,
        &[
            1016, 1020, 1024, 1028, 1032, 1036, 1040, 1044, 1048, 1052, 1056, 1060, 1064, 1068,
            1072, 1160, 1164, 1168, 1176, 1180, 1184, 1240, 1244, 1248, 1076, 1080, 1084, 1088,
            1092, 1096, 1100, 1104, 1108, 1112, 1116, 1120, 1124, 1128, 1132, 1136, 1252, 1140,
            1144, 1148, 1152, 1256, 1260, 1264, 1268, 1272,
        ],
    )?;
    for _ in 0..c.count()? {
        out.effect_arrays.push(c.vector()?);
    }
    picture_words(
        c,
        &mut out,
        &[1360, 1364, 1372, 1376, 1380, 1384, 1388, 1412, 1416],
    )?;
    if c.u8()? != 0 {
        picture_words(
            c,
            &mut out,
            &[
                892, 896, 900, 904, 908, 912, 956, 960, 964, 968, 972, 976, 980,
            ],
        )?;
    }
    Ok(out)
}

fn picture_words(
    c: &mut Cursor<'_>,
    out: &mut SavedPicture,
    offsets: &[usize],
) -> Result<(), String> {
    for &offset in offsets {
        out.fields.insert(offset, c.i32()?);
    }
    Ok(())
}

fn system_fields(
    c: &mut Cursor<'_>,
    out: &mut SaveSnapshot,
    fields: &[(usize, usize)],
) -> Result<(), String> {
    for &(offset, width) in fields {
        out.system_fields.insert(offset, c.take(width)?.to_vec());
    }
    Ok(())
}

fn system_words(
    c: &mut Cursor<'_>,
    out: &mut SaveSnapshot,
    fields: &[usize],
) -> Result<(), String> {
    for &offset in fields {
        system_fields(c, out, &[(offset, 4)])?;
    }
    Ok(())
}

fn system_state(c: &mut Cursor<'_>, out: &mut SaveSnapshot) -> Result<(), String> {
    // 95C020 version 158. Keep destination offsets to avoid conflating file
    // positions with native fields (in particular +2356 is NOT serialized).
    system_fields(
        c,
        out,
        &[(2252, 1), (2253, 1), (2256, 4), (2260, 4), (2268, 4)],
    )?;
    let n = c.count()?;
    for _ in 0..n {
        let channel = c.i32()?;
        let row = c.i32()?;
        out.background_sound_ids.insert(channel, row);
    }
    c.skip(8)?;
    for offset in (2288..=2320).step_by(4) {
        system_words(c, out, &[offset])?;
    }
    system_fields(
        c,
        out,
        &[
            (2324, 18),
            (2342, 2),
            (2344, 2),
            (2346, 6),
            (2352, 2),
            (2354, 2),
        ],
    )?;
    for offset in (2364..=2396).step_by(4) {
        system_words(c, out, &[offset])?;
    }
    system_fields(
        c,
        out,
        &[
            (2416, 1),
            (2400, 2),
            (2404, 4),
            (2408, 4),
            (2418, 2),
            (2420, 2),
            (2432, 2),
            (2434, 2),
            (2440, 4),
            (2444, 4),
            (2448, 4),
            (2452, 1),
        ],
    )?;
    system_words(c, out, &[2456, 2460, 2464, 2468, 2472])?;
    system_fields(c, out, &[(2476, 1)])?;
    for _ in 0..c.count()? {
        out.system_string_array.push(c.string(false)?);
    }
    c.vector()?;
    system_words(
        c,
        out,
        &[
            2516, 2520, 2524, 2528, 2532, 2536, 2548, 2540, 2552, 2544, 2556,
        ],
    )?;
    system_fields(c, out, &[(2424, 2)])?;
    out.system_strings.insert(2564, c.string(true)?);
    out.system_strings.insert(2588, c.string(true)?);
    system_words(c, out, &[2560, 2612, 2616, 2620])?;
    out.system_strings.insert(2624, c.string(true)?);
    system_words(c, out, &[2648, 2652, 2656, 3472])?;
    for _ in 0..c.count()? {
        let id = c.i32()?;
        let path = c.string(true)?;
        let fields = c.words(4)?;
        out.background_sound_rows.push((id, path, fields));
    }
    out.system_strings.insert(2692, c.string(true)?);
    out.system_strings.insert(2716, c.string(true)?);
    system_fields(c, out, &[(2812, 1), (2813, 1)])?;
    for offset in [2740, 2788, 2764] {
        out.system_strings.insert(offset, c.string(true)?);
    }
    system_words(
        c,
        out,
        &[
            2816, 2820, 2824, 2828, 2832, 2884, 2892, 2888, 2896, 2900, 2904, 2908, 2912,
        ],
    )?;
    system_fields(c, out, &[(2916, 1), (2917, 1)])?;
    system_words(c, out, &[2944, 2952, 2956, 2960])?;
    for i in 0..2 {
        system_words(
            c,
            out,
            &[
                2964 + 12 * i,
                2968 + 12 * i,
                2972 + 12 * i,
                3004 + 4 * i,
                3012 + 4 * i,
                3020 + 4 * i,
                3028 + 12 * i,
                3032 + 12 * i,
                3036 + 12 * i,
                3052 + 4 * i,
                3060 + 4 * i,
                3068 + 4 * i,
            ],
        )?;
    }
    system_words(c, out, &[3108, 3112, 3116, 3192])?;
    out.system_strings.insert(3120, c.string(false)?);
    for _ in 0..c.count()? {
        c.string(false)?;
    }
    for offset in 3196..=3209 {
        system_fields(c, out, &[(offset, 1)])?;
    }
    system_fields(c, out, &[(3210, 256), (3466, 1), (3467, 1), (3468, 4)])?;
    for _ in 0..c.count()? {
        for _ in 0..c.count()? {
            c.i32()?;
            c.string(false)?;
        }
    }
    for _ in 0..c.count()? {
        c.string(false)?;
        c.string(false)?;
    }
    system_words(c, out, &[3484])?;
    out.serialized_sys140 = c.i32()?;
    system_words(c, out, &[2436])?;
    // 9604B3..9604F3 adds exactly one scalar in version 159.
    if out.version >= 159 {
        system_words(c, out, &[3504])?;
    }
    // 96053A/96056F: resize to count, but consume only ONE scalar when nonempty.
    if c.count()? != 0 {
        c.i32()?;
    }
    Ok(())
}

fn mutable_database(c: &mut Cursor<'_>, out: &mut SaveSnapshot) -> Result<(), String> {
    c.u8()?;
    for type_id in 0..c.count()? {
        // 6C4130 save mode bypasses external .dat metadata strings.
        let first = c.i32()?;
        let (tag, n) = if first <= -2 {
            (c.i32()?, c.count()?)
        } else if first >= 0 && first as usize <= MAX_ITEMS {
            (0, first as usize)
        } else {
            return Err(c.error("invalid database field count"));
        };
        let fields = c.words(n)?;
        let integers = fields
            .iter()
            .filter(|&&v| (1000..2000).contains(&v))
            .count();
        let strings = fields
            .iter()
            .filter(|&&v| (2000..3000).contains(&v))
            .count();
        let mut rows = Vec::new();
        for row in 0..c.count()? {
            let base = 1_100_000_000i64 + type_id as i64 * 1_000_000 + row as i64 * 100;
            let number_start = c.pos;
            let numbers = c.words(integers)?;
            for (field, &kind) in fields.iter().enumerate() {
                if (1000..2000).contains(&kind) && (kind - 1000) < integers as i32 {
                    let key = i32::try_from(base + field as i64)
                        .map_err(|_| "saved database selector overflow")?;
                    out.number_offsets
                        .insert(key, number_start + (kind - 1000) as usize * 4);
                }
            }
            let mut texts = Vec::new();
            for string_slot in 0..strings {
                let start = c.pos;
                texts.push(c.string(false)?);
                for (field, &kind) in fields.iter().enumerate() {
                    if kind == 2000 + string_slot as i32 {
                        let key = i32::try_from(base + field as i64)
                            .map_err(|_| "saved database selector overflow")?;
                        out.text_spans.insert(key, (start, c.pos));
                    }
                }
            }
            rows.push((numbers, texts));
        }
        out.mutable_database
            .push(SavedDatabaseType { tag, fields, rows });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires local decrypted native payload evidence"]
    fn local_snapshot_structure() {
        let crate_root = option_env!("CARGO_MANIFEST_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(file!())
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_path_buf()
            });
        let root = crate_root.join("../../validation/native-save/payload.bin");
        let bytes = std::fs::read(root).unwrap();
        assert_eq!(bytes[0], 0x19);
        let header = 3 + u16::from_le_bytes(bytes[1..3].try_into().unwrap()) as usize + 2;
        let version = u16::from_le_bytes(bytes[header - 2..header].try_into().unwrap());
        let snapshot = &bytes[header..];
        let mut c = Cursor {
            bytes: snapshot,
            pos: 0,
            section: "local map prefix",
            items_left: MAX_ITEMS,
        };
        let mut out = SaveSnapshot::default();
        dynamic_arrays(&mut c).unwrap();
        map_state(&mut c, &mut out).unwrap();
        assert_eq!(out.map_id, 2);
        assert_eq!(c.pos, 10031);
        let out = parse(snapshot, version).unwrap();
        assert_eq!(out.sections.len(), 8);
        assert_eq!(out.sections.last().unwrap().2 + 1, snapshot.len());
        assert!(!out.system_fields.contains_key(&2356)); // validity is not serialized
        assert!(!out.mutable_database.is_empty());
        eprintln!("validated native snapshot sections: {:?}", out.sections);

        // Reject truncations inside every section, at both sides of each
        // boundary and throughout the authentic byte stream.
        let mut cuts: Vec<_> = (0..snapshot.len()).step_by(257).collect();
        for &(_, start, end) in &out.sections {
            cuts.extend([start, end.saturating_sub(1), end]);
        }
        cuts.push(snapshot.len() - 1);
        cuts.sort_unstable();
        cuts.dedup();
        for cut in cuts {
            assert!(
                parse(&snapshot[..cut], version).is_err(),
                "accepted truncation at {cut}"
            );
        }
        let mut bad = snapshot.to_vec();
        bad[..4].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(parse(&bad, version).is_err());
        bad = snapshot.to_vec();
        // First map string's original terminator, independently located from
        // map header and its 50*15*3 tile cells.
        bad[9060] = b'X';
        assert!(parse(&bad, version).is_err());
        bad = snapshot.to_vec();
        *bad.last_mut().unwrap() = 0;
        assert!(parse(&bad, version).is_err());
        bad = snapshot.to_vec();
        bad.push(0);
        assert!(parse(&bad, version).is_err());
        assert!(parse(snapshot, 157).is_err());
    }

    #[test]
    fn rejects_hostile_lengths_and_exhausted_budget() {
        let bytes = i32::MAX.to_le_bytes();
        let mut c = Cursor {
            bytes: &bytes,
            pos: 0,
            section: "test",
            items_left: MAX_ITEMS,
        };
        assert!(c.count().is_err());
        let bytes = 2i32.to_le_bytes();
        let mut c = Cursor {
            bytes: &bytes,
            pos: 0,
            section: "test",
            items_left: 1,
        };
        assert!(c.count().is_err());
        let mut c = Cursor {
            bytes: &bytes,
            pos: 0,
            section: "test",
            items_left: MAX_ITEMS,
        };
        assert!(c.take(usize::MAX).is_err());
        assert!(parse(&[0x19, 0x19], 158).is_err());
    }
}
