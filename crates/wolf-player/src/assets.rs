use std::{io::Read, path::Path};
use wolf_assets::wolf350::{Archive, Limits};

pub fn read_file(path: &Path, cap: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > cap as u64 {
        return Err("file byte limit".into());
    }
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > cap {
        return Err("file grew past byte limit".into());
    }
    Ok(bytes)
}
/// Explicit archive identity. No guessed WOLF search priorities or loose fallback.
pub fn read_member(root: &Path, path: &str) -> Result<Vec<u8>, String> {
    let path = wolf_assets::normalize_path(path).map_err(|e| e.to_string())?;
    let (mount, member) = path
        .split_once('/')
        .ok_or("asset path must be Archive/member")?;
    let limits = Limits {
        member_bytes: 64 << 20,
        ..Limits::default()
    };
    let bytes = read_file(
        &root.join("Data").join(format!("{mount}.wolf")),
        limits.archive_bytes,
    )?;
    let archive = Archive::open(bytes, limits).map_err(|e| e.to_string())?;
    Ok(archive
        .read(member)
        .map_err(|e| e.to_string())?
        .bytes
        .to_vec())
}
