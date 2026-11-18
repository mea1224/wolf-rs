use wolf_format::{parse_database, Limits};

fn stored(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0, 0x57, 0, 0, 0x4f, 0x4c, 0x55, 0x46, 0x4d, 0, 0xc4];
    let mut block = vec![0xf0];
    let mut remaining = payload.len() - 15;
    while remaining >= 255 {
        block.push(255);
        remaining -= 255;
    }
    block.push(remaining as u8);
    block.extend(payload);
    bytes.extend((payload.len() as u32).to_le_bytes());
    bytes.extend((block.len() as u32).to_le_bytes());
    bytes.extend(block);
    bytes
}

#[test]
fn reordered_fields_bounds_and_exact_eof() {
    let mut payload = Vec::new();
    for n in [
        1u32,
        0xffff_fffe,
        0,
        3,
        1001,
        2000,
        1000,
        1,
        42,
        u32::MAX,
        2,
    ] {
        payload.extend(n.to_le_bytes());
    }
    payload.extend([b'x', 0, 0xc4]);
    let bytes = stored(&payload);
    let db = parse_database(&bytes, &Limits::default()).unwrap().value;
    assert_eq!(db.types[0].integer(0, 0), Some(-1));
    assert_eq!(db.types[0].integer(0, 2), Some(42));
    assert_eq!(db.types[0].string(0, 1).unwrap().text().unwrap(), "x");
    assert_eq!(db.types[0].integer(0, 1), None);
    for end in 0..bytes.len() {
        assert!(parse_database(&bytes[..end], &Limits::default()).is_err());
    }
    let limits = Limits {
        max_ast_bytes: 1,
        ..Limits::default()
    };
    assert!(parse_database(&bytes, &limits).is_err());
    payload.push(0);
    assert!(parse_database(&stored(&payload), &Limits::default()).is_err());
}

#[test]
fn real_database_matches_independent_reconstruction() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/evidence");
    let bytes = std::fs::read(root.join("data_copies/BasicData/SysDatabase.dat")).unwrap();
    let parsed = parse_database(&bytes, &Limits::default()).unwrap();
    assert_eq!(
        parsed.reconstructed,
        std::fs::read(root.join("data_decoded/SysDatabase.dat")).unwrap()
    );
    assert_eq!(parsed.value.types.len(), 28);
    assert_eq!(parsed.value.types[0].rows.len(), 54);
    assert_eq!(
        (0..3)
            .map(|f| parsed.value.types[7].integer(0, f).unwrap())
            .collect::<Vec<_>>(),
        [2, 6, 6]
    );
}
