use wolf_format::{Database, DatabaseRow, DatabaseType, Span, WolfString};
use wolf_vm::native_values::*;

#[test]
fn composite_database_addresses_preserve_kind_row_and_field() {
    assert_eq!(
        DatabaseAddress::decode(1_109_000_700),
        Some(DatabaseAddress {
            kind: DatabaseKind::Mutable,
            type_id: 9,
            row: 7,
            field: 0,
        })
    );
    assert_eq!(
        DatabaseAddress::decode(1_311_000_200),
        Some(DatabaseAddress {
            kind: DatabaseKind::System,
            type_id: 11,
            row: 2,
            field: 0,
        })
    );
    assert_eq!(
        DatabaseAddress::decode(1_007_123_456).unwrap().kind,
        DatabaseKind::User
    );
    assert!(DatabaseAddress::decode(1_200_000_000).is_none());
}

#[test]
fn database_uses_field_mapping_and_rejects_missing_or_type_changing_writes() {
    let span = Span { start: 0, end: 0 };
    let source = Database {
        types: vec![DatabaseType {
            span,
            tag: 0,
            extra: None,
            fields: vec![2000, 1001, 1000],
            rows: vec![DatabaseRow {
                span,
                integers: vec![10, 20],
                strings: vec![WolfString {
                    span,
                    raw: b"Picture/logo.png\0".to_vec(),
                }],
            }],
        }],
    };
    let mut db = NativeDatabases::default();
    db.insert(DatabaseKind::Mutable, &source).unwrap();
    assert_eq!(db.number(1_100_000_001).unwrap(), 20);
    assert_eq!(db.number(1_100_000_002).unwrap(), 10);
    assert_eq!(db.number(1_100_000_000).unwrap(), 0);
    assert!(db.number(1_100_000_100).is_err());
    let at = DatabaseAddress::decode(1_100_000_001).unwrap();
    db.write_mutable(at, NativeValue::Number(55)).unwrap();
    assert_eq!(db.number(1_100_000_001).unwrap(), 55);
    assert!(db
        .write_mutable(at, NativeValue::Text("55".into()))
        .is_err());
}

#[test]
fn real_title_path_and_nested_common_strings_expand_in_native_order() {
    let mut reads = Vec::new();
    let path = expand_plain_text(
        r"Picture/title_m\cself[15].png",
        Some(23),
        1024,
        |address| {
            reads.push(address);
            Ok(NativeValue::Number(3))
        },
    )
    .unwrap();
    assert_eq!(path, "Picture/title_m3.png");
    assert_eq!(reads, [15_002_315]);
    let mut reads = Vec::new();
    let text = expand_plain_text(r"\cself[5]/\v[3]", Some(17), 1024, |address| {
        reads.push(address);
        Ok(match address {
            15_001_705 => NativeValue::Text(r"\s[2]".into()),
            3_000_002 => NativeValue::Text("日本語".into()),
            _ => NativeValue::Number(10),
        })
    })
    .unwrap();
    assert_eq!(text, "日本語/10");
    assert_eq!(reads, [2_000_003, 15_001_705, 3_000_002]);
    assert_eq!(
        expand_plain_text(r"\sysS[80]", Some(23), 1024, |address| {
            assert_eq!(address, 9_900_080);
            Ok(NativeValue::Text("host supplied value".into()))
        })
        .unwrap(),
        "host supplied value"
    );
}

#[test]
fn unsupported_escapes_and_unbounded_expansion_are_not_successful_empty_text() {
    assert_eq!(
        expand_plain_text("@", Some(19), 1024, |_| unreachable!()).unwrap(),
        "@"
    );
    assert_eq!(
        expand_plain_text("＠\r\n", Some(19), 1024, |_| unreachable!()).unwrap(),
        "＠\n"
    );
    let image_text = r"\img[Picture/save/time.png]\c[10]\v[3]";
    assert_eq!(
        expand_plain_text(image_text, Some(10), 1024, |address| {
            assert_eq!(address, 2_000_003);
            Ok(NativeValue::Number(7))
        })
        .unwrap(),
        r"\img[Picture/save/time.png]\c[10]7"
    );
    for input in [r"\Cself[5]", r"\v[", r"\v[-1]", r"\cself[100]"] {
        assert!(expand_plain_text(input, Some(0), 1024, |_| Ok(NativeValue::Number(0))).is_err());
    }
    assert!(expand_plain_text(r"\cself[5]", None, 1024, |_| Ok(NativeValue::Number(0))).is_err());
    assert!(
        expand_plain_text(r"\s[0]", None, 1024, |_| Ok(NativeValue::Text(
            r"\s[0]".into()
        )))
        .is_err()
    );
    assert!(
        expand_plain_text(r"\s[0]", None, 5, |_| Ok(NativeValue::Text(
            "123456".into()
        )))
        .is_err()
    );
}

fn one_cell_database() -> Database {
    let span = Span { start: 0, end: 0 };
    Database {
        types: vec![DatabaseType {
            span,
            tag: 0,
            extra: None,
            fields: vec![1000],
            rows: vec![DatabaseRow {
                span,
                integers: vec![7],
                strings: vec![],
            }],
        }],
    }
}

fn one_cell_project() -> Vec<u8> {
    fn number(out: &mut Vec<u8>, n: u32) {
        out.extend(n.to_le_bytes());
    }
    fn string(out: &mut Vec<u8>, s: &str) {
        number(out, s.len() as u32 + 1);
        out.extend(s.as_bytes());
        out.push(0);
    }
    let mut bytes = Vec::new();
    number(&mut bytes, 1);
    string(&mut bytes, "Settings");
    number(&mut bytes, 1);
    string(&mut bytes, "Language");
    number(&mut bytes, 1);
    string(&mut bytes, "Current");
    string(&mut bytes, "");
    number(&mut bytes, 1);
    bytes.push(0);
    for _ in 0..3 {
        number(&mut bytes, 0);
    }
    number(&mut bytes, 1);
    number(&mut bytes, 0);
    bytes
}

#[test]
fn named_database_commands_preserve_indirection_and_assignment() {
    let mut db = NativeDatabases::default();
    db.insert(DatabaseKind::Mutable, &one_cell_database())
        .unwrap();
    db.insert_project(DatabaseKind::Mutable, &one_cell_project())
        .unwrap();
    let names = ["", "Settings", "Current", "Language"].map(str::to_owned);
    let result = db
        .prepare_command(
            &[99, 99, 99, 0x71001, 2_000_003],
            &names,
            |n| {
                assert_eq!(n, 2_000_003);
                Ok(NativeValue::Number(2_000_007))
            },
            |s| Ok(s.into()),
        )
        .unwrap();
    assert_eq!(
        result,
        DatabaseEffect::Read {
            destination: 2_000_007,
            value: NativeValue::Number(7),
            operator: 0
        }
    );
    let effect = db
        .prepare_command(
            &[0, 0, 0, 0x70011, 2_000_003],
            &names,
            |n| {
                Ok(NativeValue::Number(match n {
                    2_000_003 => 2_000_004,
                    2_000_004 => 5,
                    _ => n,
                }))
            },
            |s| Ok(s.into()),
        )
        .unwrap();
    db.apply_write(effect).unwrap();
    assert_eq!(db.number(1_100_000_000).unwrap(), 12);
    let result = db
        .prepare_command(
            &[0, -1, 0, 0x1000, 2_000_003],
            &[],
            |n| Ok(NativeValue::Number(n)),
            |s| Ok(s.into()),
        )
        .unwrap();
    assert_eq!(
        result,
        DatabaseEffect::Read {
            destination: 2_000_003,
            value: NativeValue::Number(1),
            operator: 0
        }
    );
}

#[test]
fn project_metadata_and_assignment_fail_safely() {
    let bytes = one_cell_project();
    for length in 0..bytes.len() {
        assert!(NativeDatabases::default()
            .insert_project(DatabaseKind::Mutable, &bytes[..length])
            .is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(NativeDatabases::default()
        .insert_project(DatabaseKind::Mutable, &trailing)
        .is_err());
    assert_eq!(
        assign_native_value(NativeValue::Number(i32::MAX), NativeValue::Number(1), 1).unwrap(),
        NativeValue::Number(i32::MIN)
    );
    assert_eq!(
        assign_native_value(NativeValue::Number(6), NativeValue::Number(0), 4).unwrap(),
        NativeValue::Number(6)
    );
    assert_eq!(
        assign_native_value(NativeValue::Number(6), NativeValue::Number(0), 5).unwrap(),
        NativeValue::Number(0)
    );
    assert!(
        assign_native_value(NativeValue::Number(i32::MIN), NativeValue::Number(-1), 4).is_err()
    );
}

fn database_with_signed_defaults(kind: DatabaseKind) -> NativeDatabases {
    let mut database = one_cell_database();
    database.types[0].fields = vec![1001, 2000, 1000];
    database.types[0].rows[0].integers = vec![7, 8];
    database.types[0].rows[0].strings = vec![WolfString {
        span: Span { start: 0, end: 0 },
        raw: b"text\0".to_vec(),
    }];
    let mut bytes = Vec::new();
    fn n(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }
    fn s(bytes: &mut Vec<u8>, value: &str) {
        n(bytes, value.len() as u32 + 1);
        bytes.extend(value.as_bytes());
        bytes.push(0);
    }
    n(&mut bytes, 1);
    s(&mut bytes, "Defaults");
    n(&mut bytes, 3);
    for name in ["Positive", "Text", "Negative"] {
        s(&mut bytes, name);
    }
    n(&mut bytes, 1);
    s(&mut bytes, "Row");
    s(&mut bytes, "");
    n(&mut bytes, 3);
    bytes.extend([0, 0, 0]);
    for _ in 0..3 {
        n(&mut bytes, 0);
    }
    n(&mut bytes, 3);
    for value in [41i32, 999, -73] {
        bytes.extend(value.to_le_bytes());
    }
    let mut db = NativeDatabases::default();
    db.insert(kind, &database).unwrap();
    db.insert_project(kind, &bytes).unwrap();
    db.validate().unwrap();
    db
}

#[test]
fn mutable_missing_row_reads_signed_project_defaults_by_field_not_storage_slot() {
    let db = database_with_signed_defaults(DatabaseKind::Mutable);
    assert_eq!(db.number(1_100_000_000).unwrap(), 8);
    assert_eq!(db.number(1_100_000_002).unwrap(), 7);
    assert_eq!(db.number(1_100_000_100).unwrap(), 41);
    assert_eq!(db.number(1_100_999_902).unwrap(), -73);
    let result = db
        .prepare_command(
            &[0, 10, 2, 0x1000, 2_000_003],
            &[],
            |n| Ok(NativeValue::Number(n)),
            |s| Ok(s.into()),
        )
        .unwrap();
    assert!(matches!(
        result,
        DatabaseEffect::Read {
            value: NativeValue::Number(-73),
            ..
        }
    ));
    for address in [1_101_000_100, 1_100_000_103, 1_100_000_101] {
        assert!(db.number(address).is_err());
    }
    assert!(db
        .prepare_command(
            &[0, 10, 1, 0x1000, 2_000_003],
            &[],
            |n| Ok(NativeValue::Number(n)),
            |s| Ok(s.into())
        )
        .is_err());
    for (kind, address) in [
        (DatabaseKind::User, 1_000_000_100),
        (DatabaseKind::System, 1_300_000_100),
    ] {
        assert!(database_with_signed_defaults(kind).number(address).is_err());
    }
    let mut without_project = NativeDatabases::default();
    without_project
        .insert(DatabaseKind::Mutable, &one_cell_database())
        .unwrap();
    assert!(without_project
        .number(1_100_000_100)
        .unwrap_err()
        .contains("project default"));
}

#[test]
fn database_reset_restores_project_defaults_and_whole_type_row_count() {
    let mut db = database_with_signed_defaults(DatabaseKind::Mutable);
    db.apply_write(DatabaseEffect::Write {
        address: DatabaseAddress { kind: DatabaseKind::Mutable, type_id: 0, row: 2, field: 0 },
        value: NativeValue::Number(123), operator: 0,
    }).unwrap();
    db.apply_write(DatabaseEffect::Reset { type_id: 0, row: Some(0) }).unwrap();
    assert_eq!(db.number(1_100_000_000).unwrap(), 41);
    assert_eq!(db.number(1_100_000_002).unwrap(), -73);
    assert_eq!(db.number(1_100_000_200).unwrap(), 123);
    assert_eq!(db.get(DatabaseAddress { kind: DatabaseKind::Mutable, type_id: 0, row: 0, field: 1 }).unwrap(), &NativeValue::Text(String::new()));
    let reset = db.prepare_command(&[0,-2,0,0,0], &[], |n| Ok(NativeValue::Number(n)), |s| Ok(s.into())).unwrap();
    db.apply_write(reset).unwrap();
    assert_eq!(db.number(1_100_000_200).unwrap(), 41);
    assert!(db.get(DatabaseAddress { kind: DatabaseKind::Mutable, type_id: 0, row: 2, field: 0 }).is_err());
    db.validate().unwrap();
}

#[test]
fn mutable_writes_grow_rows_with_defaults_and_preserve_existing_cells() {
    let mut db = database_with_signed_defaults(DatabaseKind::Mutable);
    let at = DatabaseAddress::decode(1_100_000_100).unwrap();
    assert_eq!(db.number(1_100_000_100).unwrap(), 41);
    assert!(db.get(at).is_err());
    let effect = db
        .prepare_command(
            &[0, 3, 1, 2, 0],
            &["---".into()],
            |n| Ok(NativeValue::Number(n)),
            |s| Ok(s.into()),
        )
        .unwrap();
    db.apply_write(effect).unwrap();
    assert_eq!(
        db.get(DatabaseAddress {
            row: 3,
            field: 1,
            ..at
        })
        .unwrap(),
        &NativeValue::Text("---".into())
    );
    assert_eq!(db.number(1_100_000_000).unwrap(), 8);
    assert_eq!(db.number(1_100_000_100).unwrap(), 41);
    assert_eq!(db.number(1_100_000_302).unwrap(), -73);
    assert_eq!(
        db.get(DatabaseAddress {
            row: 2,
            field: 1,
            ..at
        })
        .unwrap(),
        &NativeValue::Text(String::new())
    );
    let effect = db
        .prepare_command(
            &[0, 4, 0, 0x10, 5],
            &[],
            |n| Ok(NativeValue::Number(n)),
            |s| Ok(s.into()),
        )
        .unwrap();
    db.apply_write(effect).unwrap();
    assert_eq!(db.number(1_100_000_400).unwrap(), 46);
    db.validate().unwrap();
    let before = serde_json::to_vec(&db).unwrap();
    assert!(db
        .write_mutable(
            DatabaseAddress { row: 5, ..at },
            NativeValue::Text("wrong type".into())
        )
        .is_err());
    assert!(db
        .write_mutable(
            DatabaseAddress { row: 10_000, ..at },
            NativeValue::Number(0)
        )
        .is_err());
    assert_eq!(serde_json::to_vec(&db).unwrap(), before);
}

#[test]
fn defaults_and_field_schema_survive_serialization_and_reject_malformed_state() {
    let db = database_with_signed_defaults(DatabaseKind::Mutable);
    let serialized = serde_json::to_value(&db).unwrap();
    let restored: NativeDatabases = serde_json::from_value(serialized.clone()).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored.number(1_100_000_102).unwrap(), -73);
    let mutations: &[(&[&str], serde_json::Value)] = &[
        (&["numeric_fields", "Mutable"], serde_json::json!([])),
        (&["field_counts", "Mutable"], serde_json::json!([2])),
        (&["names", "Mutable"], serde_json::json!([])),
    ];
    for (path, value) in mutations {
        let mut bad = serialized.clone();
        bad[path[0]][path[1]] = value.clone();
        assert!(serde_json::from_value::<NativeDatabases>(bad)
            .unwrap()
            .validate()
            .is_err());
    }
    for (field, value) in [
        ("numeric_defaults", serde_json::json!([41, -73])),
        ("fields", serde_json::json!(["Positive", "Negative"])),
    ] {
        let mut bad = serialized.clone();
        bad["names"]["Mutable"][0][field] = value;
        assert!(serde_json::from_value::<NativeDatabases>(bad)
            .unwrap()
            .validate()
            .is_err());
    }
    let mut bad = serialized;
    bad["numeric_fields"]["Mutable"][0][1] = serde_json::json!(true);
    assert!(serde_json::from_value::<NativeDatabases>(bad)
        .unwrap()
        .validate()
        .is_err());
}

#[test]
fn vm_save_restore_preserves_signed_defaults() {
    use wolf_ir::Library;
    use wolf_vm::runtime::{Config, Vm};
    let library = Library::new(vec![]).unwrap();
    let mut vm = Vm::new(library.clone(), Config::default(), 7, 1).unwrap();
    vm.initialize_databases(database_with_signed_defaults(DatabaseKind::Mutable))
        .unwrap();
    let saved = vm.save().unwrap();
    let restored = Vm::restore(library.clone(), &saved, 8, 1 << 20).unwrap();
    assert_eq!(restored.native_number(1_100_000_102).unwrap(), -73);
    let mut corrupt: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    corrupt["world"]["databases"]["names"]["Mutable"][0]["numeric_defaults"] =
        serde_json::json!([41, -73]);
    assert!(Vm::restore(library, &serde_json::to_vec(&corrupt).unwrap(), 8, 1 << 20).is_err());
}

#[test]
fn database_field_minus_three_reads_expanded_row_name_or_named_row_index() {
    let mut db = NativeDatabases::default();
    db.insert(DatabaseKind::Mutable, &one_cell_database())
        .unwrap();
    db.insert_project(DatabaseKind::Mutable, &one_cell_project())
        .unwrap();
    let effect = db
        .prepare_command(
            &[0, 1_600_026, -3, 0x1000, 1_600_008],
            &[],
            |n| Ok(NativeValue::Number(if n == 1_600_026 { 0 } else { n })),
            |s| Ok(format!("expanded:{s}")),
        )
        .unwrap();
    assert_eq!(
        effect,
        DatabaseEffect::Read {
            destination: 1_600_008,
            value: NativeValue::Text("expanded:Current".into()),
            operator: 0,
        }
    );
    let effect = db
        .prepare_command(
            &[0, 0, -3, 0x21001, 2_000_003],
            &["".into(), "".into(), "Current".into()],
            |n| {
                Ok(NativeValue::Number(if n == 2_000_003 {
                    2_000_004
                } else {
                    n
                }))
            },
            |s| Ok(s.into()),
        )
        .unwrap();
    assert_eq!(
        effect,
        DatabaseEffect::Read {
            destination: 2_000_004,
            value: NativeValue::Number(0),
            operator: 0,
        }
    );
    // The sentinel is raw field -3, not a variable that merely evaluates to -3.
    assert!(db
        .prepare_command(
            &[0, 0, 2_000_005, 0x1000, 3_000_000],
            &[],
            |n| Ok(NativeValue::Number(if n == 2_000_005 { -3 } else { n })),
            |s| Ok(s.into())
        )
        .is_err());
}

#[test]
#[ignore = "requires local proprietary game archive"]
fn shipped_projects_and_startup_database_selectors() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let archive = wolf_assets::wolf350::Archive::open(
        std::fs::read(root.join("Data/BasicData.wolf")).unwrap(),
        wolf_assets::wolf350::Limits::default(),
    )
    .unwrap();
    let mut db = NativeDatabases::default();
    for (kind, name) in [
        (DatabaseKind::Mutable, "CDataBase"),
        (DatabaseKind::User, "DataBase"),
        (DatabaseKind::System, "SysDatabase"),
    ] {
        let source = archive.read(&format!("{name}.dat")).unwrap();
        let data = wolf_format::parse_database(&source.bytes, &wolf_format::Limits::default())
            .unwrap()
            .value;
        println!(
            "{kind:?} type row-name modes: {:?}",
            data.types.iter().map(|t| t.tag).collect::<Vec<_>>()
        );
        db.insert(kind, &data).unwrap();
        db.insert_project(
            kind,
            &archive.read(&format!("{name}.project")).unwrap().bytes,
        )
        .unwrap();
    }
    db.validate().unwrap();
    let path = root.join("wolf-rs/validation/native-values/startup-database-commands.json");
    let commands: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for (event, pc) in [(8, 115), (23, 325)] {
        let command = commands
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["event"] == event && c["pc"] == pc)
            .unwrap();
        let args = command["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as i32)
            .collect::<Vec<_>>();
        let strings = command["strings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let effect = db
            .prepare_command(
                &args,
                &strings,
                |n| Ok(NativeValue::Number(n)),
                |s| Ok(s.into()),
            )
            .unwrap();
        assert!(matches!(
            effect,
            DatabaseEffect::Read {
                value: NativeValue::Number(_),
                ..
            }
        ));
        println!("Common{event} pc{pc}: {effect:?}");
    }
    // 965070 -> 947C80 binds mutable type9 rows through User type4's first strings.
    assert_eq!(
        db.row_name(DatabaseKind::Mutable, 9, 7).unwrap(),
        "言語設定"
    );
    // Actual Common89 PC32: dynamic row names drive the [決定] inline key icon.
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("wolf-rs/validation/current-startup-corpus.json")).unwrap(),
    )
    .unwrap();
    let common22 = corpus["common"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["id"] == 22)
        .unwrap();
    let raw = &common22["commands"][259];
    assert_eq!(raw["opcode"], 250);
    let args: Vec<i32> = raw["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_i64().unwrap() as i32)
        .collect();
    let strings: Vec<String> = raw["strings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let mut bytes: Vec<u8> = s["raw"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap() as u8)
                .collect();
            assert_eq!(bytes.pop(), Some(0));
            String::from_utf8(bytes).unwrap()
        })
        .collect();
    let effect = db
        .prepare_command(
            &args,
            &strings,
            |n| {
                Ok(NativeValue::Number(match n {
                    1_600_038 => 10,
                    1_600_012 => 0,
                    _ => n,
                }))
            },
            |s| Ok(s.into()),
        )
        .unwrap();
    assert!(matches!(
        effect,
        DatabaseEffect::Read {
            value: NativeValue::Number(0),
            ..
        }
    ));
    assert!(db
        .get(DatabaseAddress {
            kind: DatabaseKind::Mutable,
            type_id: 8,
            row: 10,
            field: 0
        })
        .is_err());
    let common89 = corpus["common"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["id"] == 89)
        .unwrap();
    let raw = &common89["commands"][32];
    let args: Vec<i32> = raw["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_i64().unwrap() as i32)
        .collect();
    let strings: Vec<String> = raw["strings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            let mut bytes: Vec<u8> = s["raw"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap() as u8)
                .collect();
            assert_eq!(bytes.pop(), Some(0));
            String::from_utf8(bytes).unwrap()
        })
        .collect();
    let mut names = Vec::new();
    for row in 0..9 {
        let effect = db
            .prepare_command(
                &args,
                &strings,
                |n| Ok(NativeValue::Number(if n == 1_600_026 { row } else { n })),
                |s| Ok(s.into()),
            )
            .unwrap();
        let DatabaseEffect::Read {
            destination,
            value: NativeValue::Text(name),
            ..
        } = effect
        else {
            panic!("Common89 row-name command must return text");
        };
        assert_eq!(destination, 1_600_008);
        assert_eq!(
            name,
            db.row_name(DatabaseKind::Mutable, 8, row as usize).unwrap()
        );
        names.push(name);
    }
    assert!(names.iter().any(|name| name == "決定"));
    let command = commands
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["event"] == 23 && c["pc"] == 385)
        .unwrap();
    let args = command["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i32)
        .collect::<Vec<_>>();
    let strings = command["strings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let effect = db
        .prepare_command(
            &args,
            &strings,
            |n| Ok(NativeValue::Number(n)),
            |s| Ok(s.into()),
        )
        .unwrap();
    db.apply_write(effect).unwrap();
    assert_eq!(db.number(1_109_000_700).unwrap(), 1);
    // pc389 carries raw row=0 but the same explicit name must still resolve to row7.
    let mut args = args;
    args[1] = 0;
    args[4] = 0;
    let effect = db
        .prepare_command(
            &args,
            &strings,
            |n| Ok(NativeValue::Number(n)),
            |s| Ok(s.into()),
        )
        .unwrap();
    db.apply_write(effect).unwrap();
    assert_eq!(db.number(1_109_000_700).unwrap(), 0);
}
#[test]
fn native_window_title_substitutions_are_ordered_and_keep_spacing() {
    use wolf_vm::native_values::expand_window_title;
    assert_eq!(
        expand_window_title(
            "Don't Return to Wonderland  <TITLE_PLUS>　 <SOFT>",
            "Unused title",
            "Ver.0.1.9",
            ""
        )
        .unwrap(),
        "Don't Return to Wonderland  Ver.0.1.9　 "
    );
    assert_eq!(
        expand_window_title("<TITLE>/<TITLE>", "X<TITLE_PLUS>", "V<SOFT>", "S").unwrap(),
        "XVS/XVS"
    );
    assert_eq!(
        expand_window_title("<TITLE>", "<TITLE>", "", "").unwrap(),
        "<TITLE>"
    );
    assert!(expand_window_title("x\0y", "", "", "").is_err());
}

#[test]
fn native_rich_text_expands_variables_inside_controls_and_paths() {
    use wolf_vm::native_values::expand_native_text;
    let expand = |source: &str| {
        expand_native_text(source, Some(89), 4096, |address| {
            Ok(match address {
                15_008_929 => NativeValue::Number(90),
                15_008_928 => NativeValue::Number(22),
                15_008_909 => NativeValue::Text(r"BasicData\icon\cself[29].png".into()),
                15_008_907 => NativeValue::Text(r"\my[4]\img[\cself[9]]\my[-4]\mx[10]OK".into()),
                _ => return Err(format!("unexpected read {address}")),
            })
        })
    };
    assert_eq!(
        expand(r"BasicData\icon\cself[29].png").unwrap(),
        r"BasicData\icon90.png"
    );
    assert_eq!(
        expand(r"\>\f[\cself[28]]\cself[7]").unwrap(),
        r"\>\f[22]\my[4]\img[BasicData\icon90.png]\my[-4]\mx[10]OK"
    );
    assert_eq!(expand(r"\-[\cself[28]]").unwrap(), r"\-[22]");
    assert!(expand(r"\cself[x]").is_err());
    assert!(
        expand_native_text(r"\s[0]", None, 4096, |_| Ok(NativeValue::Text(
            r"\s[0]".into()
        )))
        .is_err()
    );
}

#[test]
fn native_rich_text_replacement_preserves_inline_asset_controls() {
    use wolf_vm::native_commands::transform_native_string;
    let tag = "[決定]";
    let icon = r"\my[4]\img[BasicData\icon90.png]\my[-4]\mx[10]";
    let rich = transform_native_string(0x900, "[決定]OK", tag, Some(icon)).unwrap();
    assert_eq!(rich, format!("{icon}OK"));
    // Subsequent key rows scan the already formatted string without corrupting it.
    assert_eq!(
        transform_native_string(0x900, &rich, "[キャンセル]", Some("other")).unwrap(),
        rich
    );
    assert_eq!(
        transform_native_string(0, "", r"BasicData\icon90.png", None).unwrap(),
        r"BasicData\icon90.png"
    );
    assert!(transform_native_string(0xA00, &rich, "OK", None).is_err());
}
#[test]
fn dialogue_arguments_keep_speaker_and_controls_but_normalize_native_cr_bytes() {
    use wolf_vm::native_values::{expand_native_text, NativeValue};
    let source = "@Chie\r\n\\font[2]\\space[1]\\c[3]\\ax[4]<C><R><L>\\wE[2]\\cE[7] \\cself[1]";
    let expanded = expand_native_text(source, Some(19), 4096, |address| {
        assert_eq!(address, 15_001_901);
        Ok(NativeValue::Number(23))
    })
    .unwrap();
    assert_eq!(
        expanded,
        "@Chie\n\\font[2]\\space[1]\\c[3]\\ax[4]<C><R><L>\\wE[2]\\cE[7] 23"
    );
    assert!(expand_native_text("\\unknown[1]", Some(19), 4096, |_| unreachable!()).is_err());
}
