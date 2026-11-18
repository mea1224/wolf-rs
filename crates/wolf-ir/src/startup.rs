//! SysDatabase startup semantics supported by GamePro A65079 and 9FD26A.
use serde::Serialize;
use wolf_format::Database;

pub const PROFILE: &str = "wolf350-sysdatabase-startup-v1";

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct InitialPosition {
    pub map_id: u32,
    /// Map coordinates, not pixels or native doubled coordinates.
    pub x: i32,
    pub y: i32,
    pub map_path: String,
}

pub fn map_path(database: &Database, id: u32) -> Result<String, String> {
    let value = database
        .types
        .first()
        .and_then(|t| t.string(id as usize, 0))
        .ok_or_else(|| format!("SysDatabase type 0 row {id} field 0: missing map path"))?;
    let path = value
        .text()
        .map_err(|e| format!("map {id}: {e}"))?
        .replace('\\', "/");
    let member = path
        .strip_prefix("MapData/")
        .ok_or("map path must belong to MapData")?;
    if member.is_empty()
        || member
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p.contains(':') || p.contains('\0'))
    {
        return Err(format!("unsafe map path for ID {id}"));
    }
    Ok(path)
}

pub fn initial_position(database: &Database) -> Result<InitialPosition, String> {
    let t = database.types.get(7).ok_or("SysDatabase type 7 missing")?;
    let get = |f| {
        t.integer(0, f)
            .ok_or_else(|| format!("SysDatabase type 7 row 0 field {f}: missing integer"))
    };
    let map_id = u32::try_from(get(0)?).map_err(|_| "negative initial map ID")?;
    Ok(InitialPosition {
        map_id,
        x: get(1)?,
        y: get(2)?,
        map_path: map_path(database, map_id)?,
    })
}

/// Startup page selection, from 9440C0/9F8AA0 and the raw/runtime records captured
/// in startup-runtime-20261117.json. This does not implement later page-lock overrides.
pub fn initial_page(
    pages: &[wolf_format::Page],
    owner: u32,
    mut read: impl FnMut(i32) -> Result<i32, String>,
) -> Result<Option<usize>, String> {
    for (index, page) in pages.iter().enumerate().rev() {
        let c = &page.conditions;
        if c.len() != 37 {
            return Err("unsupported map page condition layout".into());
        }
        let mut matched = true;
        for n in 0..4 {
            let flags = c[1 + n];
            if flags & 1 == 0 {
                break;
            }
            let word = |at: usize| i32::from_le_bytes(c[at..at + 4].try_into().unwrap());
            let raw_l = word(5 + n * 4);
            let raw_r = word(21 + n * 4);
            let relocate = |v: i32, left: bool| -> Result<i32, String> {
                if (if left { 1_000_000 } else { 1_100_000 }..=1_199_999).contains(&v) {
                    i32::try_from(owner)
                        .ok()
                        .and_then(|o| o.checked_mul(10))
                        .and_then(|o| o.checked_add(1_000_000 + v % 10))
                        .ok_or_else(|| "page owner overflow".into())
                } else if (9_190_000..=9_199_999).contains(&v) {
                    i32::try_from(owner)
                        .ok()
                        .and_then(|o| o.checked_mul(10))
                        .and_then(|o| o.checked_add(9_100_000 + v % 10))
                        .ok_or_else(|| "page owner overflow".into())
                } else {
                    Ok(v)
                }
            };
            let lhs = read(relocate(raw_l, true)?)?;
            let rhs = read(relocate(raw_r, false)?)?;
            matched = match flags >> 4 {
                0 => lhs > rhs,
                1 => lhs >= rhs,
                2 => lhs == rhs,
                3 => lhs <= rhs,
                4 => lhs < rhs,
                5 => lhs != rhs,
                6 => (lhs & rhs) == raw_r,
                _ => false,
            };
            if !matched {
                break;
            }
        }
        if matched {
            return Ok(Some(index));
        }
    }
    Ok(None)
}
