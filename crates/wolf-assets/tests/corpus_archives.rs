//! Opt-in check using local game data. No game assets are bundled in this crate.
use std::{fs, path::PathBuf};
use wolf_assets::wolf350::{Archive, Limits};

#[test]
#[ignore = "requires WOLF_ARCHIVE_ROOT and WOLF_ARCHIVE_REFERENCE_ROOT"]
fn all_local_archives_match_independent_extraction() {
    let source = PathBuf::from(std::env::var_os("WOLF_ARCHIVE_ROOT").expect("WOLF_ARCHIVE_ROOT"));
    let reference = PathBuf::from(
        std::env::var_os("WOLF_ARCHIVE_REFERENCE_ROOT").expect("WOLF_ARCHIVE_REFERENCE_ROOT"),
    );
    let mut paths: Vec<_> = fs::read_dir(source)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "wolf"))
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 12);
    let (mut members, mut exact, mut prefixed) = (0, 0, 0);
    const PREFIX: &[u8] = b"Extracting data from encrypted files violates the guidelines.\0";
    for path in paths {
        let archive = Archive::open(fs::read(&path).unwrap(), Limits::default()).unwrap();
        let directory = reference.join(path.file_stem().unwrap());
        for entry in archive.entries() {
            let decoded = archive.read(&entry.path).unwrap();
            let expected = fs::read(directory.join(&entry.path)).unwrap();
            members += 1;
            if decoded.bytes.as_ref() == expected {
                exact += 1;
            } else {
                assert_eq!(
                    decoded.bytes.strip_prefix(PREFIX),
                    Some(expected.as_slice()),
                    "{}: {}",
                    path.display(),
                    entry.path
                );
                prefixed += 1;
            }
        }
        eprintln!("{}: {} members", path.display(), archive.entries().count());
    }
    assert_eq!((members, exact, prefixed), (2023, 2019, 4));
}
