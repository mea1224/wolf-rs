//! GamePro 9FE550 map settings: SystemDB type 0 owns automatic BGM/BGS.
//! The .mps tile/event payload does not contain these sound selections.
use wolf_vm::native_values::{DatabaseAddress, DatabaseKind, NativeDatabases, NativeValue};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MapAudio {
    pub kind: u8,
    pub row: i32,
    pub path: Option<String>,
}

fn resolve(
    mut read: impl FnMut(usize) -> Result<NativeValue, String>,
) -> Result<Vec<MapAudio>, String> {
    let mut result = Vec::with_capacity(2);
    for kind in 0..=1 {
        let row = match read(usize::from(kind) + 1)? {
            NativeValue::Number(row) => row,
            _ => return Err("map audio selector is not numeric".into()),
        };
        // 9FE633 and 9FE7D0 leave the active channels and fade state intact.
        if row == -2 {
            continue;
        }
        if row < -3 {
            return Err(format!("unsupported map audio selector {row}"));
        }
        let path = if row == -3 {
            // 9FE5EE/9FE739: explicit file paths use logical fields 5/6.
            Some(match read(usize::from(kind) + 5)? {
                NativeValue::Text(path) => path,
                _ => return Err("map audio file is not text".into()),
            })
        } else {
            None
        };
        result.push(MapAudio { kind, row, path });
    }
    Ok(result)
}

pub(crate) fn requests(databases: &NativeDatabases, map_id: i32) -> Result<Vec<MapAudio>, String> {
    let row = usize::try_from(map_id).map_err(|_| "negative map audio settings row")?;
    resolve(|field| {
        databases
            .get(DatabaseAddress {
                kind: DatabaseKind::System,
                type_id: 0,
                row,
                field,
            })
            .cloned()
    })
    .map_err(|error| format!("map {map_id} automatic audio: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_audio_preserves_channels_and_distinguishes_stop_from_database_zero() {
        let requests = resolve(|field| {
            Ok(NativeValue::Number(match field {
                1 => -2,
                2 => -1,
                _ => panic!("unexpected path read"),
            }))
        })
        .unwrap();
        assert_eq!(
            requests,
            vec![MapAudio {
                kind: 1,
                row: -1,
                path: None
            }]
        );
        let requests = resolve(|field| {
            Ok(NativeValue::Number(match field {
                1 => 0,
                2 => 12,
                _ => panic!("unexpected path read"),
            }))
        })
        .unwrap();
        assert_eq!(
            requests,
            vec![
                MapAudio {
                    kind: 0,
                    row: 0,
                    path: None
                },
                MapAudio {
                    kind: 1,
                    row: 12,
                    path: None
                }
            ]
        );
    }

    #[test]
    fn map_audio_resolves_both_explicit_file_fields() {
        let requests = resolve(|field| {
            Ok(match field {
                1 | 2 => NativeValue::Number(-3),
                5 => NativeValue::Text("BGM/map.ogg".into()),
                6 => NativeValue::Text("BGS/wind.ogg".into()),
                _ => panic!("unexpected field"),
            })
        })
        .unwrap();
        assert_eq!(
            requests,
            vec![
                MapAudio {
                    kind: 0,
                    row: -3,
                    path: Some("BGM/map.ogg".into())
                },
                MapAudio {
                    kind: 1,
                    row: -3,
                    path: Some("BGS/wind.ogg".into())
                }
            ]
        );
    }

    #[test]
    #[ignore = "private archive map settings audit"]
    fn local_map_audio_settings_cover_all_map_rows() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap();
        let bytes = crate::assets::read_member(root, "BasicData/SysDatabase.dat").unwrap();
        let parsed = wolf_format::parse_database(&bytes, &wolf_format::Limits::default()).unwrap();
        let mut databases = NativeDatabases::default();
        databases
            .insert(DatabaseKind::System, &parsed.value)
            .unwrap();
        let mut count = 0;
        for map in 0..10_000 {
            if databases
                .get(DatabaseAddress {
                    kind: DatabaseKind::System,
                    type_id: 0,
                    row: map,
                    field: 0,
                })
                .is_err()
            {
                break;
            }
            let requests = requests(&databases, map as i32).unwrap();
            eprintln!("MAP AUDIO {map}: {requests:?}");
            count += 1;
        }
        assert!(count > 0, "no map settings were parsed");
    }
}
