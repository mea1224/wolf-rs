//! Offline audit of the current archive. Does not execute any event or game code.
use std::{error::Error, path::Path};
use wolf_assets::wolf350::{Archive, Limits as ArchiveLimits};
use wolf_format::Limits;

fn open(path: &Path) -> Result<Archive, Box<dyn Error>> {
    Ok(Archive::open(
        std::fs::read(path)?,
        ArchiveLimits::default(),
    )?)
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("archive directory required")?;
    let output = std::env::args().nth(2).ok_or("output file required")?;
    let basic = open(&Path::new(&root).join("BasicData.wolf"))?;
    let maps = open(&Path::new(&root).join("MapData.wolf"))?;
    let member = basic.read("CommonEvent.dat")?;
    let prefix = b"Extracting data from encrypted files violates the guidelines.\0";
    let common = wolf_format::parse_common(
        member.bytes.strip_prefix(prefix).unwrap_or(&member.bytes),
        &Limits::default(),
    )?;
    let mut map_data = Vec::new();
    for entry in maps.entries().filter(|e| e.path.ends_with(".mps")) {
        let asset = maps.read(&entry.path)?;
        let parsed = wolf_format::parse_map(&asset.bytes, &Limits::default())?;
        map_data.push(
            serde_json::json!({"path": entry.path, "identity": asset.id, "data": parsed.value}),
        );
    }
    let report =
        serde_json::json!({"common_identity": member.id, "common": common.value, "maps": map_data});
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
