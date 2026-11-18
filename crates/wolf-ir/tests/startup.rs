use wolf_format::{parse_database, Limits};
use wolf_ir::startup::{initial_page, initial_position, map_path};

#[test]
fn initial_position_and_non_numeric_map_paths() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../docs/evidence/data_copies/BasicData/SysDatabase.dat");
    let mut db = parse_database(&std::fs::read(path).unwrap(), &Limits::default())
        .unwrap()
        .value;
    let initial = initial_position(&db).unwrap();
    assert_eq!((initial.map_id, initial.x, initial.y), (2, 6, 6));
    for (id, name) in [
        (3, "Map005"),
        (5, "Map005_A"),
        (25, "Map008_1"),
        (43, "Map007_1"),
    ] {
        assert_eq!(map_path(&db, id).unwrap(), format!("MapData/{name}.mps"));
    }
    assert!(map_path(&db, 54).is_err());
    db.types[0].rows[2].strings[0].raw = b"MapData/../escape.mps\0".to_vec();
    assert!(initial_position(&db).is_err());
    db.types[7].rows[0].integers[0] = -1;
    assert!(initial_position(&db).is_err());
}

fn page_with_conditions(conditions: &[(u8, i32, i32)]) -> wolf_format::Page {
    let span = wolf_format::Span { start: 0, end: 0 };
    let mut raw = vec![0; 37];
    for (n, &(flags, left, right)) in conditions.iter().enumerate() {
        raw[1 + n] = flags;
        raw[5 + n * 4..9 + n * 4].copy_from_slice(&left.to_le_bytes());
        raw[21 + n * 4..25 + n * 4].copy_from_slice(&right.to_le_bytes());
    }
    wolf_format::Page {
        span,
        unknown: 0,
        graphic: wolf_format::WolfString { span, raw: vec![0] },
        graphic_fields: vec![],
        conditions: raw,
        movement: vec![],
        flags: 0,
        route_flags: 0,
        routes: vec![],
        commands: vec![],
        features: 0,
        shadow_collision: vec![],
        page_transfer: None,
    }
}

#[test]
fn initial_page_prefers_highest_matching_page_and_short_circuits_conditions() {
    let pages = [
        page_with_conditions(&[]),
        page_with_conditions(&[(0x21, 2_000_000, 1)]),
    ];
    let read = |value| Ok(if value == 2_000_000 { 0 } else { value });
    assert_eq!(initial_page(&pages, 0, read).unwrap(), Some(0));
    assert_eq!(initial_page(&pages, 0, |_| Ok(1)).unwrap(), Some(1));

    // A disabled slot terminates the enabled prefix; later slots must not be evaluated.
    let page = page_with_conditions(&[(0, 0, 0), (0x21, 2_000_000, 1)]);
    assert_eq!(
        initial_page(&[page], 0, |_| Err("must not read disabled suffix".into())).unwrap(),
        Some(0)
    );
    let page = page_with_conditions(&[(0x21, 1, 2), (0x21, 2_000_000, 1)]);
    assert_eq!(
        initial_page(&[page], 0, |value| {
            if value >= 1_000_000 {
                return Err("must stop at first failed condition".into());
            }
            Ok(value)
        })
        .unwrap(),
        None
    );
}

#[test]
fn initial_page_relocates_owner_with_the_native_left_right_asymmetry() {
    let page = page_with_conditions(&[(0x21, 1_000_004, 1_000_004), (0x21, 9_190_002, 1_100_003)]);
    let mut reads = Vec::new();
    assert_eq!(
        initial_page(&[page], 7, |encoded| {
            reads.push(encoded);
            Ok(0)
        })
        .unwrap(),
        Some(0)
    );
    assert_eq!(reads, [1_000_074, 1_000_004, 9_100_072, 1_000_073]);
}

#[test]
fn initial_page_bit_test_compares_with_raw_rhs_and_checks_invalid_inputs() {
    let page = page_with_conditions(&[(0x61, 2_000_002, 2_000_001)]);
    assert_eq!(
        initial_page(std::slice::from_ref(&page), 0, |encoded| Ok(
            match encoded {
                2_000_002 => 2_000_001,
                2_000_001 => 2_000_003,
                _ => unreachable!(),
            }
        ))
        .unwrap(),
        Some(0)
    );
    assert_eq!(
        initial_page(&[page], 0, |encoded| Ok(if encoded == 2_000_002 {
            7
        } else {
            3
        }))
        .unwrap(),
        None
    );
    assert_eq!(initial_page(&[], 0, Ok).unwrap(), None);
    let mut malformed = page_with_conditions(&[]);
    malformed.conditions.pop();
    assert!(initial_page(&[malformed], 0, Ok).is_err());
    assert!(initial_page(
        &[page_with_conditions(&[(0x21, 1_000_000, 0)])],
        u32::MAX,
        Ok
    )
    .is_err());
    assert!(initial_page(
        &[page_with_conditions(&[(0x21, 2_000_000, 0)])],
        0,
        |_| Err("unavailable getter".into())
    )
    .is_err());
}
