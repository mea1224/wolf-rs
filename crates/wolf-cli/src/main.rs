use serde::Serialize;
use std::{
    collections::BTreeMap,
    error::Error,
    io::{self, Write},
    path::PathBuf,
};
use wolf_assets::{AssetId, LooseMount};
use wolf_format::{CommonEvents, Limits, Map, RawCommand, PROFILE};
mod startup;
mod vm;

#[derive(Default, Serialize)]
struct Counts {
    maps: usize,
    map_versions: BTreeMap<u32, usize>,
    map_events: usize,
    map_pages: usize,
    map_commands: usize,
    common_events: usize,
    common_commands: usize,
    no_tile_maps: usize,
    opcodes: BTreeMap<u32, usize>,
}
impl Counts {
    fn commands(&mut self, commands: &[RawCommand]) {
        for c in commands {
            *self.opcodes.entry(c.opcode).or_default() += 1;
        }
    }
    fn map(&mut self, map: &Map) {
        self.maps += 1;
        *self.map_versions.entry(map.version).or_default() += 1;
        self.no_tile_maps += usize::from(map.tiles.is_none());
        self.map_events += map.events.len();
        for event in &map.events {
            self.map_pages += event.pages.len();
            for page in &event.pages {
                self.map_commands += page.commands.len();
                self.commands(&page.commands);
            }
        }
    }
    fn common(&mut self, common: &CommonEvents) {
        self.common_events += common.events.len();
        for event in &common.events {
            self.common_commands += event.commands.len();
            self.commands(&event.commands);
        }
    }
}
#[derive(Serialize)]
struct FileReport<T> {
    identity: AssetId,
    stored_bytes: usize,
    reconstructed_bytes: usize,
    reconstructed_blake3: String,
    data: Option<T>,
}
fn report<T>(
    id: AssetId,
    stored_bytes: usize,
    parsed: wolf_format::Parsed<T>,
    detail: bool,
) -> FileReport<T> {
    FileReport {
        identity: id,
        stored_bytes,
        reconstructed_bytes: parsed.reconstructed.len(),
        reconstructed_blake3: wolf_assets::digest(&parsed.reconstructed),
        data: detail.then_some(parsed.value),
    }
}
#[derive(Serialize)]
struct Corpus {
    schema: &'static str,
    profile: &'static str,
    offset_domain: &'static str,
    counts: Counts,
    maps: Vec<FileReport<Map>>,
    common: FileReport<CommonEvents>,
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let command = args
        .next()
        .ok_or("usage: wolf-cli inspect-corpus --assets <extracted-directory> [--details]")?;
    if command == "verify-archive" {
        return verify_archive(args.collect());
    }
    if command == "inspect-startup" {
        return startup::run(args.collect());
    }
    if command == "inspect-vm" {
        return vm::run(args.collect());
    }
    if command == "--help" || command == "-h" {
        println!("wolf-cli inspect-startup --archives <Data-directory>");
        println!("wolf-cli inspect-corpus --assets <directory containing MapData and BasicData> [--details]\nwolf-cli verify-archive <archive.wolf> [--compare <extracted-root>]\nwolf-cli inspect-vm (--assets <loose-root> | --archives <Data-directory>) [--run <program-id> --ticks <count> [--host [--host-actions <json>]]]\nJSON to stdout; diagnostics to stderr. No original game execution.");
        return Ok(());
    }
    if command != "inspect-corpus" {
        return Err("unknown command (use --help)".into());
    }
    let mut asset_root = None;
    let mut detail = false;
    while let Some(arg) = args.next() {
        if arg == "--assets" && asset_root.is_none() {
            asset_root = Some(PathBuf::from(
                args.next().ok_or("missing --assets directory")?,
            ));
        } else if arg == "--details" && !detail {
            detail = true;
        } else {
            return Err(format!("unknown or repeated argument {arg:?}").into());
        }
    }
    let limits = Limits::default();
    let mount = LooseMount::new(
        asset_root.ok_or("--assets is required; paths never default to the process directory")?,
        "project-loose-v1",
        limits.max_stored_bytes,
    )?;
    let mut counts = Counts::default();
    let mut maps = Vec::new();
    // Validate both directory inventories for case/NFC ambiguity before any read.
    let map_keys = mount.list("MapData")?;
    let basic_keys = mount.list("BasicData")?;
    if !basic_keys.iter().any(|k| k == "BasicData/CommonEvent.dat") {
        return Err("missing BasicData/CommonEvent.dat".into());
    }
    for key in map_keys {
        if !key.ends_with(".mps") {
            continue;
        }
        let asset = mount.read(&key, PROFILE)?;
        let parsed =
            wolf_format::parse_map(&asset.bytes, &limits).map_err(|e| format!("{key}: {e}"))?;
        counts.map(&parsed.value);
        maps.push(report(asset.id, asset.bytes.len(), parsed, detail));
    }
    if maps.is_empty() {
        return Err("no .mps files in MapData".into());
    }
    let key = "BasicData/CommonEvent.dat";
    let asset = mount.read(key, PROFILE)?;
    let parsed =
        wolf_format::parse_common(&asset.bytes, &limits).map_err(|e| format!("{key}: {e}"))?;
    counts.common(&parsed.value);
    let corpus = Corpus {
        schema: "wolf-rs-corpus-v1",
        profile: PROFILE,
        offset_domain: "reconstructed-prefix-retained",
        counts,
        maps,
        common: report(asset.id, asset.bytes.len(), parsed, detail),
    };
    let mut output = io::BufWriter::new(io::stdout().lock());
    serde_json::to_writer(&mut output, &corpus)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
fn verify_archive(args: Vec<std::ffi::OsString>) -> Result<(), Box<dyn Error>> {
    let mut args = args.into_iter();
    let path = PathBuf::from(
        args.next()
            .ok_or("verify-archive <archive.wolf> [--compare <extracted-root>]")?,
    );
    let limits = wolf_assets::wolf350::Limits::default();
    let compare = match args.next() {
        None => None,
        Some(a) if a == "--compare" => Some(LooseMount::new(
            PathBuf::from(args.next().ok_or("missing compare root")?),
            "reference",
            limits.member_bytes,
        )?),
        _ => return Err("unexpected verify-archive argument".into()),
    };
    if args.next().is_some() {
        return Err("unexpected trailing argument".into());
    }
    let file = std::fs::File::open(&path)?;
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(limits.archive_bytes as u64 + 1)
        .read_to_end(&mut bytes)?;
    let archive = wolf_assets::wolf350::Archive::open(bytes, limits)?;
    let mut entries = Vec::new();
    for e in archive.entries() {
        let decoded = archive.read(&e.path)?;
        let matched = if let Some(ref mount) = compare {
            let reference = mount.read(&e.path, "reference")?;
            const PREFIX: &[u8] =
                b"Extracting data from encrypted files violates the guidelines.\0";
            if reference.bytes == decoded.bytes {
                Some("exact")
            } else if decoded
                .bytes
                .strip_prefix(PREFIX)
                .is_some_and(|d| d == reference.bytes.as_ref())
            {
                Some("reference_removed_62_byte_prefix")
            } else {
                return Err(format!(
                    "member mismatch: {} (archive {} B, reference {} B)",
                    e.path,
                    decoded.bytes.len(),
                    reference.bytes.len()
                )
                .into());
            }
        } else {
            None
        };
        entries.push(serde_json::json!({"entry": e, "blake3": decoded.id.content_blake3, "reference_matches": matched}));
    }
    println!(
        "{}",
        serde_json::to_string(
            &serde_json::json!({"profile": wolf_assets::wolf350::PROFILE,
        "source": path, "source_blake3": archive.source_blake3, "members": entries})
        )?
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("wolf-cli: {e}");
        std::process::exit(1);
    }
}
