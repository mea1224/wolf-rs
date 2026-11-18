//! Non-media host subset. Existing original saves are never declared valid merely by existence.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use wolf_vm::runtime::NativeContext;
pub(crate) use crate::native_media::snapshot_bytes::BoundedBytes;

const HOST_SAVE_MAGIC: &[u8] = b"WOLF-RS-HOST\x01\n";
const HOST_SAVE_MAGIC_V2: &[u8] = b"WOLF-RS-HOST\x02\n";
const HOST_SAVE_LIMIT: usize = 512 * 1024 * 1024;

#[path = "native_text_input.rs"]
pub(crate) mod text_input;

pub struct Control {
    root: PathBuf,
    save_root: Option<PathBuf>,
    save_identity: Option<String>,
    snapshots: BTreeMap<String, CachedSnapshot>,
    restore_request: Option<RestoreRequest>,
    host_snapshots: BTreeMap<String, CachedHostSnapshot>,
    save_request: Option<SaveRequest>,
    host_restore_request: Option<HostRestoreRequest>,
    pub(crate) text_input: Option<text_input::TextInput>,
    /// One-based six-member graphic slots (270 / A2A570).
    pub characters: BTreeMap<i32, (String, String)>,
}

struct CachedSnapshot {
    snapshot: crate::native_save_payload::SaveSnapshot,
    envelope: crate::native_save_cipher::SaveEnvelope,
    disk_bytes: Vec<u8>,
    dirty: bool,
}

pub(crate) struct RestoreRequest {
    pub path: String,
    pub slot: i32,
    pub snapshot: crate::native_save_payload::SaveSnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct SaveRequest {
    pub path: String,
    #[allow(dead_code)] // Retained request identity; the path selects the persisted slot.
    pub slot: i32,
}

pub(crate) struct HostRestoreRequest {
    pub path: String,
    pub slot: i32,
    pub world: wolf_vm::runtime::World,
    pub payload: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
struct HostSnapshot {
    version: u32,
    identity: String,
    world: wolf_vm::runtime::World,
    /// Complete application-owned VM, scene and media state; never a native template.
    payload: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostHeader {
    identity: String,
    world: wolf_vm::runtime::World,
}
struct CachedHostSnapshot {
    snapshot: HostSnapshot,
    disk_bytes: Vec<u8>,
    dirty: bool,
}

impl Control {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            save_root: None,
            save_identity: None,
            snapshots: BTreeMap::new(),
            restore_request: None,
            host_snapshots: BTreeMap::new(),
            save_request: None,
            host_restore_request: None,
            text_input: None,
            characters: BTreeMap::new(),
        }
    }
    #[cfg(test)]
    pub fn with_save_root(root: &Path, save_root: &Path) -> Self {
        let mut value = Self::new(root);
        value.save_root = Some(save_root.into());
        value
    }
    pub fn set_save_root(&mut self, save_root: &Path) -> Result<(), String> {
        if self.restore_request.is_some()
            || self.save_request.is_some()
            || self.host_restore_request.is_some()
            || self.snapshots.values().any(|entry| entry.dirty)
            || self.host_snapshots.values().any(|entry| entry.dirty)
        {
            return Err("cannot change save root while a save operation is pending".into());
        }
        self.save_root = Some(save_root.into());
        self.snapshots.clear();
        self.host_snapshots.clear();
        Ok(())
    }
    pub fn save_root(&self) -> Option<&Path> {
        self.save_root.as_deref()
    }
    pub fn take_restore_request(&mut self) -> Option<RestoreRequest> {
        self.restore_request.take()
    }
    pub fn take_save_request(&mut self) -> Option<SaveRequest> {
        self.save_request.take()
    }
    pub fn take_host_restore_request(&mut self) -> Option<HostRestoreRequest> {
        self.host_restore_request.take()
    }
    fn host_storage_path(&self, path: &str) -> PathBuf {
        let full = self.storage_path(path);
        let mut name = full.as_os_str().to_os_string();
        name.push(".wolf-rs");
        PathBuf::from(name)
    }
    /// Mode 0 saves the application state at its completed tick boundary. It
    /// uses a distinct format/path and does not overwrite an original .sav.
    pub fn complete_save(
        &mut self,
        request: SaveRequest,
        world: &wolf_vm::runtime::World,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        if payload.is_empty() || payload.len() > HOST_SAVE_LIMIT {
            return Err("host save requires a complete bounded application payload".into());
        }
        self.ensure_identity()?;
        let snapshot = HostSnapshot {
            version: 1,
            identity: self.save_identity.clone().unwrap(),
            world: world.clone(),
            payload,
        };
        let bytes = encode_host_snapshot(&snapshot)?;
        let full = self.host_storage_path(&request.path);
        let previous = match std::fs::read(&full) {
            Ok(value) => Some(value),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("host save existing {}: {e}", full.display())),
        };
        replace_host_file(&full, previous.as_deref(), &bytes)?;
        self.host_snapshots.insert(
            request.path,
            CachedHostSnapshot {
                snapshot,
                disk_bytes: bytes,
                dirty: false,
            },
        );
        Ok(())
    }
    fn storage_path(&self, path: &str) -> PathBuf {
        if let Some(root) = &self.save_root {
            // Common10's last-slot sidecar is at the game root natively. Keep
            // it inside the isolated store alongside Save files when redirected.
            if path.eq_ignore_ascii_case("currentsav.txt") {
                return root.join("currentsav.txt");
            }
            if path
                .get(..5)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Save/"))
            {
                return root.join(&path[5..]);
            }
        }
        self.root.join(path)
    }
    pub fn execute(&mut self, ctx: &mut NativeContext<'_>) -> Result<bool, String> {
        let raw = ctx
            .instruction
            .raw
            .as_ref()
            .ok_or("native host instruction has no raw record")?;
        let a = &raw.args;
        if raw.opcode == 122 && a.get(1).is_some_and(|flags| flags & 15 == 3) {
            return text_input::execute(&mut self.text_input, ctx);
        }
        match raw.opcode {
            106 if a.is_empty() && raw.strings.len() == 1 => {
                // A0B3E0 checks special prefixes before A0CB49's debug gate.
                // Ordinary text returns immediately in the shipped release mode;
                // it does not perform variable expansion or pause execution.
                release_debug_message(raw.strings[0].text().map_err(|e| e.to_string())?)?;
                Ok(true)
            }
            // A13617/A13655 -> A2F7C0; A30668..A30785 recognizes
            // <<GET_FILE_EXIST>>, probes via 7496A0 and returns "1"/"0".
            122 if a.len() == 3 && a[1] == 0x40200 && a[2] == 0 && raw.strings.len() == 1 => {
                let query = ctx.expand(raw.strings[0].text().map_err(|e| e.to_string())?)?;
                let path = query
                    .strip_prefix("<<GET_FILE_EXIST>>")
                    .ok_or_else(|| format!("unsupported native special text query {query}"))?;
                // The native backend also accepts search patterns and special
                // path expansion. This subset handles ordinary local paths.
                if path.starts_with("<<") || path.contains(['*', '?', '\u{1}']) {
                    return Err("native file-existence query requires an ordinary path".into());
                }
                let path = wolf_assets::normalize_path(path).map_err(|e| e.to_string())?;
                let exists = match std::fs::metadata(self.storage_path(&path)) {
                    Ok(_) => true,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                    Err(e) => return Err(format!("native file-existence query {path}: {e}")),
                };
                ctx.set_text(a[0], if exists { "1" } else { "0" }.into())?;
                Ok(true)
            }
            // A124BF..A12535: current UTF-8 engine mode writes encoding 0
            // unchanged; encoding 1 adds BOM. Encoding 2 is locale conversion.
            122 if a.len() == 4
                && matches!(a[1], 0x500 | 0x600)
                && a[2] == 0
                && (a[3] == 1 || (a[1] == 0x600 && a[3] == 0))
                && raw.strings.len() == 1 =>
            {
                let path = ctx.expand(raw.strings[0].text().map_err(|e| e.to_string())?)?;
                if path.starts_with("<<") {
                    return Err("special text-file command is not an ordinary path".into());
                }
                let path = wolf_assets::normalize_path(&path).map_err(|e| e.to_string())?;
                let full = self.storage_path(&path);
                if a[1] == 0x500 {
                    let text = match read_bounded(&full) {
                        Ok(bytes) => decode_utf8_file(&bytes)?,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "<NoFile>".into(),
                        Err(e) => return Err(format!("native text read {path}: {e}")),
                    };
                    // This game's version is >=340: A12054..A120BA calls 94FC50.
                    // 94FE82..94FEC3 removes all CR through AC1D80/AC8410.
                    let text = ctx.expand(&text.replace('\r', ""))?;
                    ctx.set_text(a[0], text)?;
                } else {
                    // Native handler rejects writes of BasicData .dat/.project files.
                    let upper = path.to_ascii_uppercase();
                    if upper.contains("BASICDATA/")
                        && (upper.contains(".DAT") || upper.contains(".PROJECT"))
                    {
                        return Err(format!(
                            "native text write rejects protected data path {path}"
                        ));
                    }
                    // 955A50 reverses internal 0x01 display controls; it does not
                    // evaluate variable escapes again. Plain text is unchanged.
                    let text = ctx.text(a[0])?;
                    if text.contains('\u{1}') {
                        return Err(
                            "native text write needs internal display-control conversion".into(),
                        );
                    }
                    let bytes = encode_text_file(&text, a[3] == 1);
                    if let Some(parent) = full.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("native text directory {path}: {e}"))?;
                    }
                    std::fs::write(&full, bytes)
                        .map_err(|e| format!("native text write {path}: {e}"))?;
                }
                Ok(true)
            }
            270 if a.len() == 2
                && matches!(a[0], 0..=2)
                && raw.strings.len() == usize::from(a[0] != 0) =>
            {
                let slot = ctx.number(a[1])?;
                if !(1..=6).contains(&slot) {
                    return Err("party character slot outside 1..6".into());
                }
                let graphic = if a[0] == 0 {
                    String::new()
                } else {
                    // A2A896 -> 56C150 is the literal string-vector accessor,
                    // not variable expansion. Variable-string forms use bit100.
                    let mut graphic = raw.strings[0].text().map_err(|e| e.to_string())?.to_owned();
                    while graphic.contains("\\\\") {
                        graphic = graphic.replace("\\\\", "\\");
                    }
                    graphic
                };
                if a[0] == 1 {
                    // A2A9A3..A2AAF5 copies slots 5->6 down to slot->slot+1,
                    // preserving the associated graphic layers. Slot 6 drops.
                    for destination in ((slot + 1)..=6).rev() {
                        let previous = self
                            .characters
                            .get(&(destination - 1))
                            .cloned()
                            .unwrap_or_default();
                        self.characters.insert(destination, previous);
                    }
                }
                // Modes 0/1/2 clear supplementary layers at the selected slot.
                // Mode2 replaces in place; mode1 inserts before it.
                self.characters.insert(slot, (graphic, String::new()));
                println!(
                    "NATIVE tick {} party character {slot} mode {}",
                    ctx.tick, a[0]
                );
                Ok(true)
            }
            221 if a.len() == 4 && matches!(a[3], 0 | 1) && raw.strings.is_empty() => {
                let (path, _) = save_path(ctx, a[1])?;
                let selector = if a[3] == 1 { ctx.number(a[2])? } else { a[2] };
                if self.load_host_snapshot(&path)? {
                    let saved = &self.host_snapshots[&path].snapshot.world;
                    if is_text_address(a[0]) {
                        ctx.set_text(a[0], host_text(saved, selector)?)?;
                    } else {
                        ctx.set_number(a[0], host_number(saved, selector)?)?;
                    }
                    return Ok(true);
                }
                let valid = self.load_snapshot(&path)?;
                if is_text_address(a[0]) {
                    let value = if valid {
                        self.snapshots[&path].snapshot.text(selector)?
                    } else {
                        String::new()
                    };
                    ctx.set_text(a[0], value)?;
                } else {
                    let value = if valid {
                        self.snapshots[&path].snapshot.number(selector)?
                    } else {
                        0
                    };
                    ctx.set_number(a[0], value)?;
                }
                Ok(true)
            }
            222 if a.len() == 4 && matches!(a[3], 0 | 1) && raw.strings.is_empty() => {
                let (path, _) = save_path(ctx, a[1])?;
                let selector = if a[3] == 1 { ctx.number(a[2])? } else { a[2] };
                if self.load_host_snapshot(&path)? {
                    let saved = self.host_snapshots.get_mut(&path).unwrap();
                    let value = if is_text_address(a[0]) {
                        wolf_vm::native_values::NativeValue::Text(ctx.text(a[0])?)
                    } else {
                        wolf_vm::native_values::NativeValue::Number(ctx.number(a[0])?)
                    };
                    patch_host_world(&mut saved.snapshot.world, selector, value)?;
                    saved.dirty = true;
                    return Ok(true);
                }
                if !self.load_snapshot(&path)? {
                    return Err(format!(
                        "222 cannot patch missing or foreign snapshot {path}"
                    ));
                }
                let cached = self.snapshots.get_mut(&path).unwrap();
                if is_text_address(a[0]) {
                    cached.snapshot.set_text(selector, &ctx.text(a[0])?)?;
                } else {
                    cached.snapshot.set_number(selector, ctx.number(a[0])?)?;
                }
                cached.dirty = true;
                Ok(true)
            }
            220 if a.len() == 2 && raw.strings.is_empty() => {
                // A1EA83 flushes the entire dirty path table before mode/path.
                self.flush_snapshots()?;
                let (path, slot) = save_path(ctx, a[1])?;
                if a[0] == 0 {
                    if self.save_request.is_some() {
                        return Err("host save request already pending".into());
                    }
                    self.save_request = Some(SaveRequest { path, slot });
                    ctx.yield_once();
                    return Ok(true);
                }
                if a[0] != 1 {
                    return Err(format!(
                        "220 mode {} requires a complete host save adapter",
                        a[0]
                    ));
                }
                if self.restore_request.is_some() || self.host_restore_request.is_some() {
                    return Err("native restore request already pending".into());
                }
                if self.load_host_snapshot(&path)? {
                    let saved = &self.host_snapshots[&path].snapshot;
                    self.host_restore_request = Some(HostRestoreRequest {
                        path,
                        slot,
                        world: saved.world.clone(),
                        payload: saved.payload.clone(),
                    });
                    ctx.invalidate_execution();
                    return Ok(true);
                }
                if !self.load_snapshot(&path)? {
                    return Err(format!(
                        "220 snapshot is missing or belongs to another game: {path}"
                    ));
                }
                let snapshot = self.snapshots[&path].snapshot.clone();
                self.restore_request = Some(RestoreRequest {
                    path,
                    slot,
                    snapshot,
                });
                ctx.invalidate_execution();
                Ok(true)
            }
            _ => Err(format!(
                "unimplemented native control {} {:?}",
                raw.opcode, a
            )),
        }
    }

    fn load_snapshot(&mut self, path: &str) -> Result<bool, String> {
        // 9B3640 keeps independent snapshots keyed by normalized path.
        if self.snapshots.contains_key(path) {
            return Ok(true);
        }
        let bytes = match read_bounded(&self.storage_path(path)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("native save read {path}: {e}")),
        };
        let envelope = crate::native_save_cipher::decode(&bytes)
            .map_err(|e| format!("native save {path}: {e}"))?;
        self.ensure_identity()?;
        if self.save_identity.as_deref() != Some(envelope.identity.as_str()) {
            // Native code returns failure and renames .anotherGame. Querying
            // from this runtime leaves the user's file intact.
            return Ok(false);
        }
        let snapshot = crate::native_save_payload::parse(&envelope.snapshot, envelope.version)
            .map_err(|e| format!("native save {path}: {e}"))?;
        self.snapshots.insert(
            path.to_owned(),
            CachedSnapshot {
                snapshot,
                envelope,
                disk_bytes: bytes,
                dirty: false,
            },
        );
        Ok(true)
    }

    pub fn flush_snapshots(&mut self) -> Result<(), String> {
        let host_paths: Vec<_> = self
            .host_snapshots
            .iter()
            .filter(|(_, entry)| entry.dirty)
            .map(|(path, _)| path.clone())
            .collect();
        for path in host_paths {
            let full = self.host_storage_path(&path);
            let cached = self.host_snapshots.get_mut(&path).unwrap();
            let bytes = encode_host_snapshot(&cached.snapshot)?;
            replace_host_file(&full, Some(&cached.disk_bytes), &bytes)?;
            cached.disk_bytes = bytes;
            cached.dirty = false;
        }
        let paths: Vec<_> = self
            .snapshots
            .iter()
            .filter(|(_, entry)| entry.dirty)
            .map(|(path, _)| path.clone())
            .collect();
        for path in paths {
            let full = self.storage_path(&path);
            let cached = self.snapshots.get_mut(&path).unwrap();
            if read_bounded(&full).map_err(|e| format!("native save before write {path}: {e}"))?
                != cached.disk_bytes
            {
                return Err(format!("native save changed since it was loaded: {path}"));
            }
            cached.envelope.snapshot = cached.snapshot.bytes().to_vec();
            let bytes = crate::native_save_cipher::encode(&cached.envelope)?;
            // Parse the exact encoded result before making it persistent.
            let verified = crate::native_save_cipher::decode(&bytes)?;
            crate::native_save_payload::parse(&verified.snapshot, verified.version)?;
            let backup = full.with_extension("sav.wolf-rs-backup");
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(&cached.disk_bytes)
                        .map_err(|e| format!("native save backup: {e}"))?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(format!("native save backup: {e}")),
            }
            let pending = full.with_extension("sav.wolf-rs-pending");
            std::fs::write(&pending, &bytes)
                .map_err(|e| format!("native save temporary write: {e}"))?;
            std::fs::rename(&pending, &full)
                .map_err(|e| format!("native save replace {path}: {e}"))?;
            cached.disk_bytes = bytes;
            cached.dirty = false;
        }
        Ok(())
    }

    fn ensure_identity(&mut self) -> Result<(), String> {
        if self.save_identity.is_none() {
            let bytes = crate::assets::read_member(&self.root, "BasicData/Game.dat")?;
            let config = wolf_format::parse_game(&bytes, &Default::default())
                .map_err(|e| format!("native save game identity: {e}"))?;
            self.save_identity = Some(config.title.text().map_err(|e| e.to_string())?.to_owned());
        }
        Ok(())
    }

    fn load_host_snapshot(&mut self, path: &str) -> Result<bool, String> {
        if self.host_snapshots.contains_key(path) {
            return Ok(true);
        }
        let full = self.host_storage_path(path);
        let file = match std::fs::File::open(&full) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("host save read {}: {e}", full.display())),
        };
        if file.metadata().map_err(|e| e.to_string())?.len() > HOST_SAVE_LIMIT as u64 {
            return Err("host save exceeds byte bound".into());
        }
        use std::io::Read;
        let mut bytes = vec![];
        file.take(HOST_SAVE_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let snapshot = decode_host_snapshot(&bytes)?;
        self.ensure_identity()?;
        if self.save_identity.as_deref() != Some(&snapshot.identity) {
            return Ok(false);
        }
        self.host_snapshots.insert(
            path.into(),
            CachedHostSnapshot {
                snapshot,
                disk_bytes: bytes,
                dirty: false,
            },
        );
        Ok(true)
    }
}

fn encode_host_snapshot(snapshot: &HostSnapshot) -> Result<Vec<u8>, String> {
    if snapshot.version != 1 || snapshot.payload.is_empty() {
        return Err("unsupported or incomplete host save".into());
    }
    // The application payload is already serialized. Embedding it as Vec<u8>
    // JSON expands each byte into decimal text and made real saves exceed the
    // cap. V2 stores the payload verbatim with checked lengths and exact EOF.
    let overhead = HOST_SAVE_MAGIC_V2.len() + 16;
    let metadata_cap = HOST_SAVE_LIMIT
        .checked_sub(snapshot.payload.len())
        .and_then(|n| n.checked_sub(overhead))
        .ok_or("host save exceeds byte bound")?;
    #[derive(Serialize)]
    struct Header<'a> {
        identity: &'a str,
        world: &'a wolf_vm::runtime::World,
    }
    let mut metadata = BoundedBytes::new(metadata_cap);
    serde_json::to_writer(
        &mut metadata,
        &Header {
            identity: &snapshot.identity,
            world: &snapshot.world,
        },
    )
    .map_err(|e| format!("host save metadata: {e}"))?;
    let mut bytes = Vec::with_capacity(overhead + metadata.bytes.len() + snapshot.payload.len());
    bytes.extend_from_slice(HOST_SAVE_MAGIC_V2);
    bytes.extend_from_slice(&(metadata.bytes.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(snapshot.payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&metadata.bytes);
    bytes.extend_from_slice(&snapshot.payload);
    Ok(bytes)
}

fn decode_host_snapshot(bytes: &[u8]) -> Result<HostSnapshot, String> {
    if bytes.len() > HOST_SAVE_LIMIT {
        return Err("host save exceeds byte bound".into());
    }
    if let Some(body) = bytes.strip_prefix(HOST_SAVE_MAGIC_V2) {
        let lengths = body.get(..16).ok_or("truncated host save lengths")?;
        let metadata_len = usize::try_from(u64::from_le_bytes(lengths[..8].try_into().unwrap()))
            .map_err(|_| "host save length overflow")?;
        let payload_len = usize::try_from(u64::from_le_bytes(lengths[8..].try_into().unwrap()))
            .map_err(|_| "host save length overflow")?;
        let split = 16usize
            .checked_add(metadata_len)
            .ok_or("host save length overflow")?;
        let end = split
            .checked_add(payload_len)
            .ok_or("host save length overflow")?;
        if payload_len == 0 || end != body.len() {
            return Err("invalid host save lengths/EOF".into());
        }
        let metadata = body.get(16..split).ok_or("truncated host save metadata")?;
        let header: HostHeader =
            serde_json::from_slice(metadata).map_err(|e| format!("host save metadata: {e}"))?;
        return Ok(HostSnapshot {
            version: 1,
            identity: header.identity,
            world: header.world,
            payload: body[split..].to_vec(),
        });
    }
    let body = bytes
        .strip_prefix(HOST_SAVE_MAGIC)
        .ok_or("invalid host save signature")?;
    let snapshot: HostSnapshot =
        serde_json::from_slice(body).map_err(|e| format!("host save decode: {e}"))?;
    if snapshot.version != 1 || snapshot.payload.is_empty() {
        return Err("unsupported or incomplete host save".into());
    }
    Ok(snapshot)
}

fn replace_host_file(full: &Path, previous: Option<&[u8]>, bytes: &[u8]) -> Result<(), String> {
    let current = match std::fs::read(full) {
        Ok(value) => Some(value),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("host save reread: {e}")),
    };
    if current.as_deref() != previous {
        return Err("host save changed since loaded".into());
    }
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("host save directory: {e}"))?;
    }
    let mut pending_name = full.as_os_str().to_os_string();
    pending_name.push(".pending");
    let pending = PathBuf::from(pending_name);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|e| format!("host save temporary: {e}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("host save write: {e}"))?;
    drop(file);
    std::fs::rename(&pending, full).map_err(|e| format!("host save replace: {e}"))
}

fn host_slots(
    world: &wolf_vm::runtime::World,
    selector: i32,
    text: bool,
) -> Result<(&wolf_vm::runtime::Slots, i32), String> {
    match selector {
        2_000_000..=2_999_999 if !text => Ok((&world.globals, selector - 2_000_000)),
        3_000_000..=3_999_999 if text => Ok((&world.globals, selector - 3_000_000)),
        9_000_000..=9_099_999 if !text => Ok((&world.systems, selector - 9_000_000)),
        9_900_000..=9_999_999 if text => Ok((&world.systems, selector - 9_900_000)),
        15_000_000..=15_999_999 if text == (5..=9).contains(&(selector % 100)) => {
            Ok((&world.commons, selector - 15_000_000))
        }
        _ => Err(format!("unsupported host saved selector {selector}")),
    }
}

fn host_number(world: &wolf_vm::runtime::World, selector: i32) -> Result<i32, String> {
    if selector == 9_000_024 {
        return Ok(1);
    }
    if wolf_vm::native_values::DatabaseAddress::decode(selector).is_some() {
        return world.databases.number(selector);
    }
    let (slots, key) = host_slots(world, selector, false)?;
    Ok(slots.numbers.get(&key).copied().unwrap_or(0))
}
fn host_text(world: &wolf_vm::runtime::World, selector: i32) -> Result<String, String> {
    if let Some(at) = wolf_vm::native_values::DatabaseAddress::decode(selector) {
        return world.databases.get(at).cloned().map(|value| value.text());
    }
    let (slots, key) = host_slots(world, selector, true)?;
    Ok(slots.strings.get(&key).cloned().unwrap_or_default())
}
fn patch_host_world(
    world: &mut wolf_vm::runtime::World,
    selector: i32,
    value: wolf_vm::native_values::NativeValue,
) -> Result<(), String> {
    use wolf_vm::native_values::{DatabaseAddress, NativeValue};
    if let Some(address) = DatabaseAddress::decode(selector) {
        return world.databases.write_mutable(address, value);
    }
    let is_text = matches!(value, NativeValue::Text(_));
    let (_, key) = host_slots(world, selector, is_text)?;
    let slots = match selector {
        2_000_000..=3_999_999 => &mut world.globals,
        9_000_000..=9_999_999 => &mut world.systems,
        _ => &mut world.commons,
    };
    match value {
        NativeValue::Number(value) => {
            slots.numbers.insert(key, value);
        }
        NativeValue::Text(value) => {
            if value.len() > 16 * 1024 * 1024 || value.contains('\0') {
                return Err("host saved text bound or embedded NUL".into());
            }
            slots.strings.insert(key, value);
        }
    }
    Ok(())
}

fn is_text_address(value: i32) -> bool {
    matches!(value, 1_600_005..=1_600_009 | 3_000_000..=3_999_999 | 9_900_000..=9_999_999)
        || (15_000_000..16_000_000).contains(&value) && (5..=9).contains(&(value % 100))
}

fn save_path(ctx: &NativeContext<'_>, operand: i32) -> Result<(String, i32), String> {
    let (path, slot) = if is_text_address(operand) {
        (ctx.text(operand)?, 0)
    } else {
        let slot = ctx.number(operand)?;
        if slot < 0 {
            return Err("negative original save selector".into());
        }
        (
            if slot == 0 {
                "Save/SaveData".into()
            } else {
                format!("Save/SaveData{slot:02}.sav")
            },
            slot,
        )
    };
    Ok((
        wolf_assets::normalize_path(&path).map_err(|e| e.to_string())?,
        slot,
    ))
}

fn release_debug_message(text: &str) -> Result<(), String> {
    for prefix in [
        "MessageBox:",
        "PictureWatch:",
        "ValWatch:",
        "MoveWatch:",
        "PerformanceMonitor:",
    ] {
        if text.starts_with(prefix) {
            return Err(format!("unsupported native debug command {prefix}"));
        }
    }
    Ok(())
}

// The native binary reads raw file bytes, then constructs a C string (A11F2E),
// removes EF BB BF (A12016), and leaves CR/LF bytes untouched. Invalid UTF-8
// and locale conversion are explicit unsupported boundaries of this subset.
fn decode_utf8_file(bytes: &[u8]) -> Result<String, String> {
    const LIMIT: usize = 16 * 1024 * 1024;
    if bytes.len() > LIMIT {
        return Err("native text file exceeds 16 MiB host bound".into());
    }
    let bytes = &bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())];
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec())
        .map_err(|e| format!("native text file is not valid UTF-8: {e}"))
}

fn read_bounded(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    const LIMIT: u64 = 16 * 1024 * 1024;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "native text file exceeds 16 MiB host bound",
        ));
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "native text file grew beyond 16 MiB host bound",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
fn encode_utf8_text_file(text: &str) -> Vec<u8> {
    encode_text_file(text, true)
}

fn encode_text_file(text: &str, bom: bool) -> Vec<u8> {
    // A12535 explicitly prepends BOM for encoding=1. L"w" at CA4E40
    // opens the CRT in text mode, which translates every LF to CR LF,
    // including an LF that was already preceded by CR.
    let mut bytes = Vec::with_capacity(text.len() + 3);
    if bom && !text.starts_with('\u{feff}') {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    for &b in text.as_bytes() {
        if b == b'\n' {
            bytes.push(b'\r');
        }
        bytes.push(b);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_map_party_insert_shifts_six_slots_and_replace_preserves_neighbors() {
        let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = crate::assets::read_member(&game, "MapData/Map001.mps").unwrap();
        let map = wolf_format::parse_map(&bytes, &Default::default()).unwrap();
        let command = map
            .value
            .events
            .iter()
            .flat_map(|event| event.pages.iter())
            .flat_map(|page| page.commands.iter())
            .find(|command| {
                command.opcode == 270
                    && command.args == [1, 1]
                    && command
                        .strings
                        .first()
                        .is_some_and(|s| s.text().ok() == Some("CharaChip/alice_x4_TX.png"))
            })
            .unwrap()
            .clone();
        let mut control = Control::new(&game);
        for slot in 1..=6 {
            control
                .characters
                .insert(slot, (format!("old-{slot}"), format!("layer-{slot}")));
        }
        let mut execute = |command: &wolf_format::RawCommand| {
            let program = wolf_ir::compile(
                "party".into(),
                "real Map001 party".into(),
                wolf_ir::Source {
                    asset: "MapData/Map001.mps".into(),
                    content_blake3: "private-fixture".into(),
                    event: 0,
                    page: Some(0),
                    command: 74,
                    span: [0, 0],
                },
                std::slice::from_ref(command),
            )
            .unwrap();
            let mut vm = wolf_vm::runtime::Vm::new(
                wolf_ir::Library::new(vec![program]).unwrap(),
                Default::default(),
                1,
                1_000,
            )
            .unwrap();
            vm.spawn(wolf_ir::Target::Id("party".into()), 0, vec![])
                .unwrap();
            let report = vm
                .tick_with_host(vec![], &mut |ctx| control.execute(ctx))
                .unwrap();
            assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
            control.characters.clone()
        };
        let inserted = execute(&command);
        assert_eq!(
            inserted[&1],
            ("CharaChip/alice_x4_TX.png".into(), String::new())
        );
        for slot in 2..=6 {
            assert_eq!(
                inserted[&slot],
                (format!("old-{}", slot - 1), format!("layer-{}", slot - 1))
            );
        }
        let mut replace = command.clone();
        replace.args = [2, 3].to_vec();
        let replaced = execute(&replace);
        assert_eq!(
            replaced[&3],
            ("CharaChip/alice_x4_TX.png".into(), String::new())
        );
        for slot in [1, 2, 4, 5, 6] {
            assert_eq!(replaced[&slot], inserted[&slot]);
        }
        let mut clear = command;
        clear.args = [0, 3].to_vec();
        clear.strings.clear();
        let cleared = execute(&clear);
        assert_eq!(cleared[&3], (String::new(), String::new()));
        assert_eq!(cleared[&4], inserted[&4]);
    }

    #[test]
    fn host_v2_is_linear_bounded_and_reads_v1() {
        let snapshot = HostSnapshot {
            version: 1,
            identity: "fixture".into(),
            world: Default::default(),
            payload: (0..=255).cycle().take(1 << 20).collect(),
        };
        let bytes = encode_host_snapshot(&snapshot).unwrap();
        assert!(bytes.starts_with(HOST_SAVE_MAGIC_V2));
        assert!(bytes.len() < snapshot.payload.len() + 16_384);
        let mut decoded = decode_host_snapshot(&bytes).unwrap();
        for _ in 0..8 {
            let next = encode_host_snapshot(&decoded).unwrap();
            assert_eq!(
                next, bytes,
                "repeated save must not nest or expand prior bytes"
            );
            decoded = decode_host_snapshot(&next).unwrap();
        }
        assert_eq!(decoded.payload, snapshot.payload);
        let mut old = HOST_SAVE_MAGIC.to_vec();
        serde_json::to_writer(&mut old, &snapshot).unwrap();
        assert_eq!(
            decode_host_snapshot(&old).unwrap().payload,
            snapshot.payload
        );
        for end in [
            0,
            HOST_SAVE_MAGIC_V2.len(),
            HOST_SAVE_MAGIC_V2.len() + 15,
            bytes.len() - 1,
        ] {
            assert!(decode_host_snapshot(&bytes[..end]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_host_snapshot(&trailing).is_err());
        let mut overflow = bytes;
        overflow[HOST_SAVE_MAGIC_V2.len()..HOST_SAVE_MAGIC_V2.len() + 8]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode_host_snapshot(&overflow).is_err());
        let mut bounded = BoundedBytes::new(3);
        assert!(serde_json::to_writer(&mut bounded, &"more than three").is_err());
        assert!(bounded.bytes.len() <= 3);
    }

    #[test]
    fn host_save_real_commands_restore_world_and_execution_independently() {
        let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = crate::assets::read_member(&game, "BasicData/CommonEvent.dat").unwrap();
        let bytes = bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&bytes);
        let parsed = wolf_format::parse_common(bytes, &Default::default()).unwrap();
        let common = parsed.value.events.iter().find(|e| e.id == 10).unwrap();
        assert_eq!(common.commands[549].args, [0, 1_600_015]);
        let program = wolf_ir::compile(
            "host-save".into(),
            "Common10 host save".into(),
            wolf_ir::Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: "private-fixture".into(),
                event: 10,
                page: None,
                command: 0,
                span: [0, 0],
            },
            &[
                common.commands[549].clone(),
                common.commands[688].clone(),
                common.commands[691].clone(),
            ],
        )
        .unwrap();
        let library = wolf_ir::Library::new(vec![program]).unwrap();
        let mut vm = wolf_vm::runtime::Vm::new(library.clone(), Default::default(), 1, 1).unwrap();
        vm.set_global(100_002, 77).unwrap();
        vm.spawn(wolf_ir::Target::Id("host-save".into()), 0, vec![])
            .unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let isolated =
            std::env::temp_dir().join(format!("wolf-host-save-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&isolated).unwrap();
        let mut control = Control::with_save_root(&game, &isolated);
        let report = vm
            .tick_with_host(vec![], &mut |ctx| {
                ctx.set_number(1_600_015, 1)?;
                control.execute(ctx)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        let request = control.take_save_request().unwrap();
        assert_eq!(request.slot, 1);
        control
            .complete_save(request, &vm.state().world, vm.save().unwrap())
            .unwrap();
        assert!(!isolated.join("SaveData01.sav").exists());
        assert!(isolated.join("SaveData01.sav.wolf-rs").exists());
        // Force disk decode through a fresh Control, then patch and restore.
        let mut control = Control::with_save_root(&game, &isolated);
        let report = vm
            .tick_with_host(vec![], &mut |ctx| control.execute(ctx))
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert!(report.execution_invalidated);
        assert_eq!(vm.state().world.globals.numbers[&100_002], 77);
        let request = control.take_host_restore_request().unwrap();
        assert_eq!(request.world.globals.numbers[&100_002], 1);
        let mut restored =
            wolf_vm::runtime::Vm::restore(library, &request.payload, 2, 1_000_000).unwrap();
        let fibers = serde_json::to_vec(&restored.state().fibers).unwrap();
        restored.replace_saved_world(request.world).unwrap();
        assert_eq!(restored.state().world.globals.numbers[&100_002], 1);
        assert_eq!(
            serde_json::to_vec(&restored.state().fibers).unwrap(),
            fibers
        );
        let mut control = Control::with_save_root(&game, &isolated);
        assert!(control.load_host_snapshot("Save/SaveData01.sav").unwrap());
        assert_eq!(
            host_number(
                &control.host_snapshots["Save/SaveData01.sav"].snapshot.world,
                2_100_002
            )
            .unwrap(),
            1
        );
        std::fs::remove_file(isolated.join("SaveData01.sav.wolf-rs")).unwrap();
        std::fs::remove_dir(isolated).unwrap();
    }

    #[test]
    fn native_restore_real_autosave_patch_flush_and_invalidation() {
        let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let original_path = game.join("Save/auto.sav");
        if !original_path.exists() {
            eprintln!("private autosave fixture absent");
            return;
        }
        let original = std::fs::read(&original_path).unwrap();
        let envelope = crate::native_save_cipher::decode(&original).unwrap();
        let snapshot =
            crate::native_save_payload::parse(&envelope.snapshot, envelope.version).unwrap();
        assert_eq!(snapshot.number(1_100_000_008).unwrap(), 0);
        eprintln!(
            "native autosave version={} map={} player={},{},{} banks={:?} events={} pictures={} rng={}",
            envelope.version,
            snapshot.map_id,
            snapshot.player.x2,
            snapshot.player.y2,
            snapshot.player.direction,
            snapshot
                .numeric_banks
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>(),
            snapshot.map_characters.len(),
            snapshot.picture_records.len(),
            snapshot.native_rng
        );
        let samples: Vec<_> = snapshot
            .pictures
            .iter()
            .map(|picture| {
                serde_json::json!({"source":String::from_utf8_lossy(&picture.source),
                "fields":picture.fields,"effect_arrays":picture.effect_arrays})
            })
            .collect();
        let sample_path = game.join("wolf-rs/validation/native-save-restore/auto-pictures.json");
        std::fs::write(sample_path, serde_json::to_vec_pretty(&samples).unwrap()).unwrap();
        let system = serde_json::json!({"fields":snapshot.system_fields,"strings":snapshot.system_strings.iter().map(|(k,v)|(k,String::from_utf8_lossy(v))).collect::<BTreeMap<_,_>>(),"sound_rows":snapshot.background_sound_rows});
        std::fs::write(
            game.join("wolf-rs/validation/native-save-restore/auto-system.json"),
            serde_json::to_vec_pretty(&system).unwrap(),
        )
        .unwrap();
        let mut base = wolf_vm::runtime::World::default();
        use wolf_vm::native_values::DatabaseKind;
        for (kind, name) in [
            (DatabaseKind::Mutable, "CDataBase"),
            (DatabaseKind::User, "DataBase"),
            (DatabaseKind::System, "SysDatabase"),
        ] {
            let data = crate::assets::read_member(&game, &format!("BasicData/{name}.dat")).unwrap();
            let db = wolf_format::parse_database(&data, &Default::default()).unwrap();
            base.databases.insert(kind, &db.value).unwrap();
            let project =
                crate::assets::read_member(&game, &format!("BasicData/{name}.project")).unwrap();
            base.databases.insert_project(kind, &project).unwrap();
        }
        let world = snapshot.restore_world(&base).unwrap();
        assert_eq!(
            world.globals.numbers.get(&100_002).copied().unwrap_or(0),
            snapshot.number(2_100_002).unwrap()
        );
        assert_eq!(
            world
                .databases
                .get(wolf_vm::native_values::DatabaseAddress::decode(1_100_000_013).unwrap())
                .unwrap()
                .clone()
                .text(),
            snapshot.text(1_100_000_013).unwrap()
        );
        let encoded = crate::native_save_cipher::encode(&envelope).unwrap();
        assert_eq!(
            crate::native_save_cipher::decode(&encoded)
                .unwrap()
                .snapshot,
            envelope.snapshot
        );

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let isolated =
            std::env::temp_dir().join(format!("wolf-restore-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&isolated).unwrap();
        let slot = isolated.join("SaveData01.sav");
        std::fs::write(&slot, &original).unwrap();
        let mut control = Control::with_save_root(&game, &isolated);

        let bytes = crate::assets::read_member(&game, "BasicData/CommonEvent.dat").unwrap();
        let bytes = bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&bytes);
        let parsed = wolf_format::parse_common(bytes, &Default::default()).unwrap();
        let common = parsed.value.events.iter().find(|e| e.id == 10).unwrap();
        assert_eq!(common.commands[688].args, [1, 1_600_015, 2_100_002, 0]);
        assert_eq!(common.commands[691].args, [1, 1_600_015]);
        let program = wolf_ir::compile(
            "restore-test".into(),
            "Common10 real restore".into(),
            wolf_ir::Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: "private-fixture".into(),
                event: 10,
                page: None,
                command: 0,
                span: [0, 0],
            },
            &[common.commands[688].clone(), common.commands[691].clone()],
        )
        .unwrap();
        let mut vm = wolf_vm::runtime::Vm::new(
            wolf_ir::Library::new(vec![program]).unwrap(),
            Default::default(),
            1,
            1,
        )
        .unwrap();
        vm.set_global(100_002, 987).unwrap();
        vm.spawn(wolf_ir::Target::Id("restore-test".into()), 0, vec![])
            .unwrap();
        let report = vm
            .tick_with_host(vec![], &mut |ctx| {
                ctx.set_number(1_600_015, 1)?;
                control.execute(ctx)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert_eq!(vm.state().world.globals.numbers.get(&100_002), Some(&987));
        let request = control
            .take_restore_request()
            .expect("restore must be published");
        assert_eq!(request.slot, 1);
        assert_eq!(request.path, "Save/SaveData01.sav");
        assert_eq!(request.snapshot.number(2_100_002).unwrap(), 1);
        let patched = crate::native_save_cipher::decode(&std::fs::read(&slot).unwrap()).unwrap();
        let patched =
            crate::native_save_payload::parse(&patched.snapshot, patched.version).unwrap();
        assert_eq!(patched.number(2_100_002).unwrap(), 1);
        assert_eq!(std::fs::read(&original_path).unwrap(), original);
        assert_eq!(
            std::fs::read(slot.with_extension("sav.wolf-rs-backup")).unwrap(),
            original
        );
        std::fs::remove_file(slot.with_extension("sav.wolf-rs-backup")).unwrap();
        std::fs::remove_file(slot).unwrap();
        std::fs::remove_dir(isolated).unwrap();
    }

    #[test]
    fn native_snapshot_text_patch_preserves_other_sections() {
        let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = game.join("Save/auto.sav");
        if !source.exists() {
            eprintln!("private autosave fixture absent");
            return;
        }
        let bytes = std::fs::read(source).unwrap();
        let envelope = crate::native_save_cipher::decode(&bytes).unwrap();
        let mut snapshot =
            crate::native_save_payload::parse(&envelope.snapshot, envelope.version).unwrap();
        let player = snapshot.player.raw.clone();
        let pictures = snapshot.picture_records.clone();
        let map = snapshot.map_id;
        snapshot
            .set_text(1_100_000_013, "隔离测试メモ\nsecond line")
            .unwrap();
        assert_eq!(
            snapshot.text(1_100_000_013).unwrap(),
            "隔离测试メモ\nsecond line"
        );
        assert_eq!(snapshot.player.raw, player);
        assert_eq!(snapshot.picture_records, pictures);
        assert_eq!(snapshot.map_id, map);
    }

    #[test]
    fn release_debug_text_does_not_expand_but_special_commands_are_not_swallowed() {
        assert!(release_debug_message(r"config missing \unsupported[0]").is_ok());
        assert!(release_debug_message("prefix MessageBox: ordinary text").is_ok());
        for prefix in [
            "MessageBox:",
            "PictureWatch:",
            "ValWatch:",
            "MoveWatch:",
            "PerformanceMonitor:",
        ] {
            assert!(release_debug_message(&format!("{prefix}ON=1")).is_err());
        }
    }
    #[test]
    fn original_utf8_config_read_keeps_crlf_and_c_string_boundary() {
        assert_eq!(
            decode_utf8_file(b"\xef\xbb\xbfBGM=80\r\nSE=80\r\n\0ignored").unwrap(),
            "BGM=80\r\nSE=80\r\n"
        );
        assert!(decode_utf8_file(&[0xff]).is_err());
    }
    #[test]
    fn original_utf8_config_write_has_bom_and_crt_newlines() {
        assert_eq!(encode_utf8_text_file("BGM=80\n"), b"\xef\xbb\xbfBGM=80\r\n");
        assert_eq!(
            encode_utf8_text_file("\u{feff}X\r\n"),
            b"\xef\xbb\xbfX\r\r\n"
        );
    }

    #[test]
    fn native_encoding_zero_writes_utf8_without_bom() {
        assert_eq!(
            encode_text_file("ACH_TEST\n保存", false),
            "ACH_TEST\r\n保存".as_bytes()
        );
        assert_eq!(
            encode_text_file("\u{feff}保存", false),
            "\u{feff}保存".as_bytes()
        );
    }
    #[test]
    fn real_common8_config_commands_round_trip_through_vm_host() {
        let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        if !game.join("Data/BasicData.wolf").exists() {
            eprintln!("private game fixture absent");
            return;
        }
        let bytes = crate::assets::read_member(&game, "BasicData/CommonEvent.dat").unwrap();
        let bytes = bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&bytes);
        let parsed = wolf_format::parse_common(bytes, &Default::default()).unwrap();
        let event = parsed.value.events.iter().find(|e| e.id == 8).unwrap();
        let commands = vec![
            event.commands[90].clone(),
            event.commands[144].clone(),
            event.commands[90].clone(),
            event.commands[168].clone(),
        ];
        assert_eq!(commands[0].args, vec![1_600_005, 0x40200, 0]);
        assert_eq!(commands[1].args, vec![1_600_006, 0x600, 0, 1]);
        assert_eq!(commands[3].args, vec![1_600_006, 0x500, 0, 1]);
        let program = wolf_ir::compile(
            "config-io".into(),
            "Common8 real config commands".into(),
            wolf_ir::Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: "private-fixture".into(),
                event: 8,
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
        vm.spawn(wolf_ir::Target::Id("config-io".into()), 0, vec![])
            .unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("wolf-native-config-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut control = Control::new(&root);
        let mut result = String::new();
        let mut existence = Vec::new();
        let report = vm
            .tick_with_host(vec![], &mut |ctx| {
                if ctx.instruction.raw.as_ref().unwrap().args[1] == 0x600 {
                    ctx.set_text(1_600_006, "BGM=80\nLanguage=0\n".into())?;
                }
                control.execute(ctx)?;
                if ctx.instruction.raw.as_ref().unwrap().args[1] == 0x40200 {
                    existence.push(ctx.text(1_600_005)?);
                } else {
                    result = ctx.text(1_600_006)?;
                }
                Ok(true)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert_eq!(
            std::fs::read(root.join("Save/game_config.txt")).unwrap(),
            b"\xef\xbb\xbfBGM=80\r\nLanguage=0\r\n"
        );
        assert_eq!(result, "BGM=80\nLanguage=0\n");
        assert_eq!(existence, ["0", "1"]);
        // Only these two files were created by this test; no recursive removal.
        std::fs::remove_file(root.join("Save/game_config.txt")).unwrap();
        std::fs::remove_dir(root.join("Save")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
