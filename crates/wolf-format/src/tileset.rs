//! D3 tilesets: native 69FD23/69FF44 and 572B70. Older versions are separate profiles.
use crate::{reader::Reader, Domain, Limits, Result, WolfString};

#[derive(Debug)]
pub struct TileSet {
    pub name: WolfString,
    pub images: Vec<WolfString>,
    pub tags: Vec<u8>,
    pub flags_marker: u8,
    pub flags: Vec<i32>,
}

pub fn parse_tilesets(input: &[u8], limits: &Limits) -> Result<Vec<TileSet>> {
    let mut r = Reader::new(input, 0, Domain::Stored, limits)?;
    if input.len() > limits.max_stored_bytes {
        return Err(r.error("tileset stored size limit"));
    }
    r.expect(b"\0W\0\0OLUFM\0\xd3")?;
    let sets = r.vector(175, |r| {
        let name = r.string()?;
        let mut images = r.vec_capacity(32, 5)?;
        for _ in 0..32 {
            images.push(r.string()?);
        }
        r.expect(&[255])?;
        let count = r.count()?;
        let tags = r.blob(count)?;
        // 57337B consumes this byte without checking its value.
        let flags_marker = r.u8()?;
        let flags = r.vector(4, |r| r.i32())?;
        Ok(TileSet {
            name,
            images,
            tags,
            flags_marker,
            flags,
        })
    })?;
    r.expect(&[0xcf])?;
    r.eof()?;
    Ok(sets)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires locally extracted game fixture"]
    fn real_tilesets_and_truncation() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../validation/archive-reference/BasicData/TileSetData.dat");
        let bytes = std::fs::read(path).expect("local TileSetData.dat fixture");
        let sets = parse_tilesets(&bytes, &Limits::default()).unwrap();
        assert_eq!(sets.len(), 9);
        assert_eq!(sets[2].tags.len(), 80);
        assert_eq!(sets[2].flags.len(), 80);
        assert_eq!(sets[2].images.len(), 32);
        for end in [10, 14, 200, bytes.len() - 1] {
            assert!(parse_tilesets(&bytes[..end], &Limits::default()).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(parse_tilesets(&trailing, &Limits::default()).is_err());
        let cap = Limits {
            max_records: 8,
            ..Limits::default()
        };
        assert!(parse_tilesets(&bytes, &cap).is_err());
    }
}
