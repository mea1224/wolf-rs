//! Read-only loose files and bounded WOLF350 archives.
pub mod wolf350;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
use unicode_normalization::UnicodeNormalization;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes directory junctions, not just symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub fn normalize_path(input: &str) -> io::Result<String> {
    let key = input.replace('\\', "/");
    if key.is_empty()
        || key.starts_with('/')
        || key
            .chars()
            .any(|c| c.is_control() || ":<>\"|?*".contains(c))
    {
        return Err(invalid("invalid virtual asset path"));
    }
    for part in key.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.ends_with([' ', '.']) {
            return Err(invalid("ambiguous path component"));
        }
        let stem = part.split('.').next().unwrap().to_uppercase();
        if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
            || ["COM", "LPT"].iter().any(|p| {
                stem.strip_prefix(p).is_some_and(|s| {
                    ["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"].contains(&s)
                })
            })
        {
            return Err(invalid("reserved Windows device component"));
        }
    }
    Ok(key)
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetId {
    pub mount: String,
    pub original_path: String,
    pub key: String,
    pub content_blake3: String,
    pub decoder_profile: String,
}
pub struct Asset {
    pub id: AssetId,
    pub bytes: Arc<[u8]>,
}
pub fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// A single explicit mount: exact UTF-8 lookup, rejecting NFC/lowercase collisions.
/// No cache, workers, priority overlay, or guessed Windows/WOLF lookup semantics yet.
pub struct LooseMount {
    root: PathBuf,
    id: String,
    max_bytes: usize,
}
impl LooseMount {
    pub fn new(
        root: impl AsRef<Path>,
        id: impl Into<String>,
        max_bytes: usize,
    ) -> io::Result<Self> {
        let root = root.as_ref().canonicalize()?;
        if !root.is_dir() {
            return Err(invalid("asset mount is not a directory"));
        }
        Ok(Self {
            root,
            id: id.into(),
            max_bytes,
        })
    }
    fn resolve(&self, key: &str) -> io::Result<PathBuf> {
        let normalized = normalize_path(key)?;
        let mut path = self.root.clone();
        // Enumerate each component so exact spelling has the same behavior on
        // case-insensitive Windows/macOS and case-sensitive Linux filesystems.
        for component in normalized.split('/') {
            let mut found = None;
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                if entry.file_name() == std::ffi::OsStr::new(component) {
                    found = Some(entry.path());
                    break;
                }
            }
            path = found.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "exact asset path not found")
            })?;
            if is_link(&fs::symlink_metadata(&path)?) {
                return Err(invalid("asset links/reparse points are unsupported"));
            }
        }
        let resolved = path.canonicalize()?;
        if !resolved.starts_with(&self.root) {
            return Err(invalid("asset escaped mount"));
        }
        Ok(resolved)
    }
    pub fn read(&self, key: &str, profile: &str) -> io::Result<Asset> {
        let normalized = normalize_path(key)?;
        let path = self.resolve(&normalized)?;
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > self.max_bytes as u64 {
            return Err(invalid("asset file size limit"));
        }
        let cap = self
            .max_bytes
            .checked_add(1)
            .ok_or_else(|| invalid("asset cap overflow"))?;
        let mut bytes = Vec::new();
        file.take(cap as u64).read_to_end(&mut bytes)?;
        if bytes.len() > self.max_bytes {
            return Err(invalid("asset grew past limit"));
        }
        let id = AssetId {
            mount: self.id.clone(),
            original_path: key.into(),
            key: normalized,
            content_blake3: digest(&bytes),
            decoder_profile: profile.into(),
        };
        Ok(Asset {
            id,
            bytes: bytes.into(),
        })
    }
    /// Sorted, nonrecursive inventory. Reparse/symlink entries are rejected.
    pub fn list(&self, directory: &str) -> io::Result<Vec<String>> {
        let directory = normalize_path(directory)?;
        let mut keys = Vec::new();
        let mut collisions = BTreeMap::new();
        for entry in fs::read_dir(self.resolve(&directory)?)? {
            let entry = entry?;
            if is_link(&fs::symlink_metadata(entry.path())?) {
                return Err(invalid(
                    "links/reparse points are unsupported in loose inventory",
                ));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("non-UTF-8 loose filename"))?;
            let key = normalize_path(&format!("{directory}/{name}"))?;
            let collision_key: String = key.nfc().flat_map(char::to_lowercase).collect();
            if let Some(old) = collisions.insert(collision_key, key.clone()) {
                return Err(invalid(format!("path collision: {old} / {key}")));
            }
            if entry.file_type()?.is_file() {
                self.resolve(&key)?;
                keys.push(key);
            }
        }
        keys.sort();
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_paths_on_every_host() {
        for p in [
            "../x",
            "a/../x",
            "/x",
            "\\\\host\\x",
            "C:\\x",
            "a//b",
            "a/./b",
            "a\0b",
            "NUL.txt",
            "com1/x",
            "x. ",
            "a/b:",
            "LPT¹",
        ] {
            assert!(normalize_path(p).is_err(), "accepted {p:?}");
        }
        assert_eq!(
            normalize_path("MapData\\Map001.mps").unwrap(),
            "MapData/Map001.mps"
        );
    }
    #[test]
    fn content_identity_changes_with_bytes() {
        assert_eq!(digest(b"a"), digest(b"a"));
        assert_ne!(digest(b"a"), digest(b"b"));
    }
    #[test]
    fn loose_mount_enforces_exact_names_and_size_cap() {
        let dir = std::env::temp_dir().join(format!(
            "wolf-assets-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        fs::create_dir(dir.join("MapData")).unwrap();
        fs::write(dir.join("MapData/Case.mps"), b"abc").unwrap();
        let mount = LooseMount::new(&dir, "test", 3).unwrap();
        assert_eq!(
            &*mount.read("MapData/Case.mps", "test").unwrap().bytes,
            b"abc"
        );
        assert!(mount.read("mapdata/Case.mps", "test").is_err());
        assert!(mount.read("MapData/case.mps", "test").is_err());
        assert_eq!(mount.list("MapData").unwrap(), ["MapData/Case.mps"]);
        assert!(LooseMount::new(&dir, "test", 2)
            .unwrap()
            .read("MapData/Case.mps", "test")
            .is_err());
    }
}
