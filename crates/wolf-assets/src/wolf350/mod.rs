//! DXA v8 / WOLF350 only. Read-only, memory-backed archive; no extraction side effects.
mod compression;
mod crypto;
use crate::{digest, invalid, normalize_path, Asset, AssetId};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use unicode_normalization::UnicodeNormalization;
type Result<T> = io::Result<T>;
fn bad(s: &str) -> io::Error {
    invalid(format!("WOLF350: {s}"))
}
fn range(data: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    data.get(at..at.checked_add(len).ok_or_else(|| bad("range overflow"))?)
        .ok_or_else(|| bad("range outside input"))
}
fn u64_at(data: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(range(data, at, 8)?.try_into().unwrap()))
}
fn index(n: u64) -> Result<usize> {
    usize::try_from(n).map_err(|_| bad("offset exceeds address space"))
}
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b).ok_or_else(|| bad("offset overflow"))
}
pub const PROFILE: &str = "dxa8-wolf350-v1";
#[derive(Debug, Clone)]
pub struct Limits {
    pub archive_bytes: usize,
    pub metadata_bytes: usize,
    pub member_bytes: usize,
    pub total_decoded_bytes: u64,
    pub entries: usize,
    pub depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            archive_bytes: 512 << 20,
            metadata_bytes: 32 << 20,
            member_bytes: 256 << 20,
            total_decoded_bytes: 2 << 30,
            entries: 100_000,
            depth: 64,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub path: String,
    pub raw_name: Vec<u8>,
    pub attributes: u64,
    pub times: [u64; 3],
    pub data_offset: usize,
    pub size: usize,
    pub lz_size: Option<usize>,
    pub huffman_size: Option<usize>,
}
pub struct Archive {
    data: Vec<u8>,
    entries: BTreeMap<String, Entry>,
    inner_key: [u8; 256],
    huffman_kb: u8,
    data_end: usize,
    pub source_blake3: String,
}
impl Archive {
    pub fn open(mut bytes: Vec<u8>, limits: Limits) -> Result<Self> {
        if bytes.len() < 1088 || bytes.len() > limits.archive_bytes {
            return Err(bad(
                "archive size/profile limit (small WOLF350 archives not verified)",
            ));
        }
        if range(&bytes, 0, 4)? != b"DX\x08\0" {
            return Err(bad("expected DXA v8"));
        }
        let flags = u32::from_le_bytes(bytes[44..48].try_into().unwrap());
        if flags != 350 << 16 {
            return Err(bad(
                "unsupported crypt profile/flags (only corpus flags verified)",
            ));
        }
        let cp = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        if cp != 932 {
            return Err(bad("expected CP932 names"));
        }
        let source_blake3 = digest(&bytes);
        let p: [u8; 15] = bytes[49..64].try_into().unwrap();
        crypto::addresses(&mut bytes[..64], &p);
        let data_start = index(u64_at(&bytes, 8)?)?;
        let name_start = index(u64_at(&bytes, 16)?)?;
        let file_start = index(u64_at(&bytes, 24)?)?;
        let dir_start = index(u64_at(&bytes, 32)?)?;
        let meta_len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        if data_start < 64
            || data_start > name_start
            || name_start >= bytes.len()
            || bytes.len() - name_start > limits.metadata_bytes
            || meta_len > limits.metadata_bytes
            || file_start > dir_start
            || dir_start > meta_len
        {
            return Err(bad("invalid metadata bounds"));
        }
        let huffman_kb = bytes[48];
        let outer = crypto::key(&p, true);
        let end = bytes.len() - 64;
        crypto::xor(&mut bytes[64..end], &outer, 64);
        let body = crypto::body_len(&p, bytes.len());
        if body > bytes.len() - 64 {
            return Err(bad("AES body outside archive"));
        }
        let mut aes = crypto::WolfAes::new(&p);
        aes.crypt(&mut bytes[64..64 + body]);
        aes.crypt(&mut bytes[name_start..]);
        let inner_key = crypto::key(&p, false);
        let mut meta = bytes[name_start..].to_vec();
        crypto::xor(&mut meta, &inner_key, 0);
        let meta = compression::lz(
            &compression::huffman(&meta, limits.metadata_bytes)?,
            limits.metadata_bytes,
        )?;
        if meta.len() != meta_len {
            return Err(bad("metadata length mismatch"));
        }
        let names = range(&meta, 0, file_start)?;
        let files = range(&meta, file_start, dir_start - file_start)?;
        let dirs = &meta[dir_start..];
        let mut entries = BTreeMap::new();
        let mut canonical = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let mut seen_files = BTreeSet::new();
        let mut stack = vec![(0usize, String::new(), 0usize)];
        let mut total = 0u64;
        let mut records = 0usize;
        while let Some((dir, parent, depth)) = stack.pop() {
            if depth > limits.depth || !seen.insert(dir) {
                return Err(bad("directory cycle/depth limit"));
            }
            if dir % 32 != 0 {
                return Err(bad("unaligned directory record"));
            }
            let d = range(dirs, dir, 32)?;
            let count = index(u64_at(d, 16)?)?;
            let first = index(u64_at(d, 24)?)?;
            if first % 72 != 0 {
                return Err(bad("unaligned file record"));
            }
            records = add(records, count)?;
            if records > limits.entries {
                return Err(bad("entry count limit"));
            }
            range(
                files,
                first,
                count
                    .checked_mul(72)
                    .ok_or_else(|| bad("file table overflow"))?,
            )?;
            for i in 0..count {
                let at = first + i * 72;
                if !seen_files.insert(at) {
                    return Err(bad("file record appears in multiple directories"));
                }
                let f = range(files, at, 72)?;
                let name_at = index(u64_at(f, 0)?)?;
                let n = range(names, name_at, 4)?;
                let padded = usize::from(u16::from_le_bytes([n[0], n[1]])) * 4;
                let raw = range(names, add(name_at, add(4, padded)?)?, padded)?;
                let zero = raw
                    .iter()
                    .position(|b| *b == 0)
                    .ok_or_else(|| bad("unterminated archive name"))?;
                let raw = &raw[..zero];
                let name = encoding_rs::SHIFT_JIS
                    .decode_without_bom_handling_and_without_replacement(raw)
                    .ok_or_else(|| bad("invalid CP932 name"))?;
                if name.contains(['/', '\\']) {
                    return Err(bad("separator in archive name component"));
                }
                let path = normalize_path(&if parent.is_empty() {
                    name.to_string()
                } else {
                    format!("{parent}/{name}")
                })?;
                let key = path.nfc().collect::<String>().to_lowercase();
                if canonical.insert(key, path.clone()).is_some() {
                    return Err(bad("case/Unicode duplicate archive path"));
                }
                let attrs = u64_at(f, 8)?;
                let address = index(u64_at(f, 40)?)?;
                if attrs & 0x400 != 0 {
                    return Err(bad("archive reparse entry"));
                }
                if attrs & 16 != 0 {
                    stack.push((address, path, depth + 1));
                    continue;
                }
                let size = index(u64_at(f, 48)?)?;
                total = total
                    .checked_add(size as u64)
                    .ok_or_else(|| bad("decoded byte count overflow"))?;
                if size > limits.member_bytes || total > limits.total_decoded_bytes {
                    return Err(bad("decoded member budget"));
                }
                let optional = |at| -> Result<Option<usize>> {
                    let n = u64_at(f, at)?;
                    if n == u64::MAX {
                        Ok(None)
                    } else {
                        Ok(Some(index(n)?))
                    }
                };
                let e = Entry {
                    path: path.clone(),
                    raw_name: raw.to_vec(),
                    attributes: attrs,
                    times: [u64_at(f, 16)?, u64_at(f, 24)?, u64_at(f, 32)?],
                    data_offset: add(data_start, address)?,
                    size,
                    lz_size: optional(56)?,
                    huffman_size: optional(64)?,
                };
                if e.lz_size.is_some_and(|s| s > limits.member_bytes)
                    || e.huffman_size.is_some_and(|s| s > limits.member_bytes)
                {
                    return Err(bad("compressed member budget"));
                }
                entries.insert(path, e);
            }
        }
        let result = Self {
            data: bytes,
            entries,
            inner_key,
            huffman_kb,
            data_end: name_start,
            source_blake3,
        };
        for e in result.entries.values() {
            if e.data_offset < data_start {
                return Err(bad("member begins before data region"));
            }
            result.stored(e)?;
        }
        Ok(result)
    }
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }
    fn stored(&self, e: &Entry) -> Result<&[u8]> {
        let intermediate = e.lz_size.unwrap_or(e.size);
        let edge = usize::from(self.huffman_kb) * 1024;
        let middle = if self.huffman_kb != 255 {
            intermediate.saturating_sub(edge * 2)
        } else {
            0
        };
        let size = if let Some(h) = e.huffman_size {
            add(h, middle)?
        } else {
            intermediate
        };
        range(&self.data[..self.data_end], e.data_offset, size)
    }
    /// Returns exact archive member bytes, including any inner-format prefix.
    pub fn read(&self, path: &str) -> Result<Asset> {
        let key = normalize_path(path)?;
        let e = self
            .entries
            .get(&key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "archive member not found"))?;
        let mut data = self.stored(e)?.to_vec();
        crypto::xor(&mut data, &self.inner_key, e.size as u64);
        if let Some(h) = e.huffman_size {
            let size = e.lz_size.unwrap_or(e.size);
            let edge = usize::from(self.huffman_kb) * 1024;
            let expected = if self.huffman_kb != 255 {
                size.min(edge * 2)
            } else {
                size
            };
            // The index already bounds each stage. Reject forged larger sizes before allocation.
            let expanded = compression::huffman(range(&data, 0, h)?, expected)?;
            if self.huffman_kb != 255 && size > edge * 2 {
                if expanded.len() != edge * 2 || data.len() - h != size - edge * 2 {
                    return Err(bad("partial Huffman size mismatch"));
                }
                let mut joined = Vec::with_capacity(size);
                joined.extend_from_slice(&expanded[..edge]);
                joined.extend_from_slice(&data[h..]);
                joined.extend_from_slice(&expanded[edge..]);
                data = joined;
            } else {
                if expanded.len() != size {
                    return Err(bad("Huffman member size mismatch"));
                }
                data = expanded;
            }
        }
        if e.lz_size.is_some() {
            data = compression::lz(&data, e.size)?;
        }
        if data.len() != e.size {
            return Err(bad("member size mismatch"));
        }
        Ok(Asset {
            id: AssetId {
                mount: self.source_blake3.clone(),
                original_path: e.path.clone(),
                key,
                content_blake3: digest(&data),
                decoder_profile: PROFILE.into(),
            },
            bytes: Arc::from(data),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_profiles_and_untrusted_lengths() {
        for n in [0, 63, 64, 1000, 1088, 4096] {
            assert!(Archive::open(vec![0; n], Limits::default()).is_err());
        }
        let mut data = vec![0; 1088];
        data[..4].copy_from_slice(b"DX\x08\0");
        data[44..48].copy_from_slice(&(350u32 << 16).to_le_bytes());
        data[40..44].copy_from_slice(&932u32.to_le_bytes());
        assert!(Archive::open(data, Limits::default()).is_err());
    }
}
