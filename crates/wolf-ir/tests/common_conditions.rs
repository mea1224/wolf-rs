use wolf_ir::native_startup::CommonCondition;

fn condition(flags: u8, lhs: i32, rhs: i32, remaining: [u8; 2]) -> CommonCondition {
    let mut bytes = vec![flags];
    bytes.extend(lhs.to_le_bytes());
    bytes.extend(rhs.to_le_bytes());
    bytes.extend(remaining);
    CommonCondition::decode(
        u32::from_le_bytes(bytes[..4].try_into().unwrap()),
        &bytes[4..],
    )
    .unwrap()
}

#[test]
fn common_conditions_relocate_only_commonself_lhs_and_keep_rhs_literal() {
    // Raw cself values encode a slot modulo 100, independent of a caller page.
    let condition = condition(0x22, 1_600_107, 2_000_001, [0, 0]);
    let mut visited = Vec::new();
    assert!(condition
        .eligible(22, |n| {
            visited.push(n);
            Ok(2_000_001)
        })
        .unwrap());
    assert_eq!(visited, [15_002_207]);
    assert!(condition.eligible(u32::MAX, Ok).is_err());
    assert!(condition
        .eligible(22, |_| Err("getter unavailable".into()))
        .is_err());
    assert!(CommonCondition::decode(0, &[0; 6]).is_err());
    assert!(CommonCondition::decode(0, &[0; 8]).is_err());
}

#[test]
fn common_conditions_modes_operators_and_opaque_tail_follow_native_gates() {
    for tail in [[0, 0], [1, 0], [2, 0], [255, 255]] {
        // Flags low nibble 3 bypasses comparison entirely, even with high nibble 15.
        assert!(condition(0xF3, i32::MAX, i32::MIN, tail)
            .eligible(u32::MAX, |_| Err("unconditional must not read".into()))
            .unwrap());
        assert!(!condition(0x20, 2_000_000, 0, tail)
            .eligible(111, |_| Err("disabled must not read".into()))
            .unwrap());
    }
    for (op, lhs, rhs, expected) in [
        (0, -1, -2, true),
        (1, -1, -1, true),
        (2, -1, 1, false),
        (3, -1, -1, true),
        (4, -2, -1, true),
        (5, -1, 1, true),
        (6, 0b1011, 0b1001, true),
        (6, 0b0011, 0b1001, false),
        (7, 1, 1, false),
    ] {
        assert_eq!(
            condition((op << 4) | 2, 2_000_000, rhs, [0, 0])
                .eligible(0, |_| Ok(lhs))
                .unwrap(),
            expected
        );
    }
}

#[test]
fn current_startup_corpus_common_eligibility_matches_native_header_layout() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../validation/current-startup-corpus.json");
    let corpus: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut automatic = Vec::new();
    let mut conditional = Vec::new();
    for event in corpus["common"]["events"].as_array().unwrap() {
        let id = event["id"].as_u64().unwrap() as u32;
        let tail: Vec<_> = event["unknown2"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap() as u8)
            .collect();
        let c = CommonCondition::decode(event["unknown1"].as_u64().unwrap() as u32, &tail).unwrap();
        if c.flags & 15 == 3 {
            automatic.push(id);
            assert!(c
                .eligible(id, |_| Err("unconditional getter".into()))
                .unwrap());
        }
        if c.flags & 15 == 2 {
            conditional.push((id, c.encoded_lhs, c.raw_rhs));
        }
        if id == 14 {
            assert_eq!((c.flags, c.remaining), (0x23, [0, 0]));
        }
        if id == 22 {
            assert_eq!((c.flags, c.remaining), (0x23, [1, 0]));
        }
        if id == 111 {
            assert!(!c.eligible(id, Ok).unwrap());
        }
    }
    assert_eq!(automatic, [1, 2, 3, 14, 17, 22, 40, 49, 54, 57, 77]);
    assert_eq!(
        conditional,
        [
            (50, 2_900_001, 1),
            (65, 2_000_016, 0),
            (71, 2_000_013, 1),
            (86, 2_100_017, 1),
            (90, 2_000_013, 1),
            (91, 2_000_013, 1),
        ]
    );
}
