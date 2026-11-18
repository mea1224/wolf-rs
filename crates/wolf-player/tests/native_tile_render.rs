#[path = "../src/native_tile_render.rs"]
mod native_tile_render;
use native_tile_render::resolve;

#[test]
fn base_atlas_is_eight_columns_and_priority_uses_native_precedence() {
    let images = vec!["base.png".into()];
    let mut flags = vec![0; 64];
    flags[41] = 0x100;
    let draws = resolve(9, &images, &flags, 48, 0, |_| Ok((384, 96))).unwrap();
    assert_eq!(draws[0].source, [48, 48, 48, 48]);
    assert_eq!(draws[0].destination, [0, 0, 48, 48]);
    assert_eq!(draws[0].priority, 1);
    flags[41] |= 0x10;
    assert_eq!(
        resolve(9, &images, &flags, 48, 0, |_| Ok((384, 96))).unwrap()[0].priority,
        2
    );
    assert!(resolve(16, &images, &flags, 48, 0, |_| Ok((384, 96))).is_err());
    assert!(resolve(1, &images, &flags, 48, 0, |_| Ok((385, 96))).is_err());
}

#[test]
fn auto_quadrants_select_decimal_rows_and_horizontal_animation_phase() {
    let images = vec!["base.png".into(), "water_FRAME=3.png".into()];
    let draws = resolve(201234, &images, &[0, 0x100], 48, 5, |_| Ok((144, 240))).unwrap();
    assert_eq!(
        draws.iter().map(|d| d.source).collect::<Vec<_>>(),
        vec![
            [48, 48, 24, 24],
            [72, 96, 24, 24],
            [48, 168, 24, 24],
            [72, 216, 24, 24]
        ]
    );
    assert_eq!(
        draws.iter().map(|d| d.destination).collect::<Vec<_>>(),
        vec![
            [0, 0, 24, 24],
            [24, 0, 24, 24],
            [0, 24, 24, 24],
            [24, 24, 24, 24]
        ]
    );
    assert!(draws.iter().all(|d| d.priority == 1 && d.path == images[1]));
    let looped = resolve(201234, &images, &[], 48, 9, |_| Ok((144, 240))).unwrap();
    assert_eq!(looped[0].source[0], 0);
    assert!(resolve(205000, &images, &[], 48, 0, |_| Ok((144, 240))).is_err());
    assert!(resolve(-1, &[], &[], 48, 0, |_| unreachable!())
        .unwrap()
        .is_empty());
    assert!(resolve(100000, &[], &[], 48, 0, |_| unreachable!())
        .unwrap()
        .is_empty());
}

#[test]
fn animation_default_marker_case_and_clamping_follow_constructor() {
    for (path, tick, expected_x) in [
        ("water.png", 19, 0),
        ("water.png", 20, 48),
        ("water_frame=1.png", 1, 0),
        ("water_FRAME=0.png", 1, 48),
        ("water_FRAME=-2.png", 1, 48),
        ("water_FRAME=+2.png", 2, 48),
    ] {
        let images = vec![String::new(), path.into()];
        let draws = resolve(200000, &images, &[], 48, tick, |_| Ok((144, 240))).unwrap();
        assert_eq!(draws[0].source[0], expected_x, "{path}");
    }
}

#[test]
fn current_new_game_map_words_resolve_against_real_atlas_dimensions() {
    use std::collections::{BTreeMap, BTreeSet};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../validation/archive-reference");
    let sets = wolf_format::parse_tilesets(
        &std::fs::read(root.join("BasicData/TileSetData.dat")).unwrap(),
        &Default::default(),
    )
    .unwrap();
    let game = wolf_format::parse_game(
        &std::fs::read(root.join("BasicData/Game.dat")).unwrap(),
        &Default::default(),
    )
    .unwrap();
    let map = wolf_format::parse_map(
        &std::fs::read(root.join("MapData/Map001.mps")).unwrap(),
        &Default::default(),
    )
    .unwrap()
    .value;
    let set = &sets[map.tileset as usize];
    let images: Vec<String> = set
        .images
        .iter()
        .map(|s| s.text().unwrap().into())
        .collect();
    let mut cache = BTreeMap::new();
    let words: BTreeSet<i32> = map.tiles.unwrap().into_iter().collect();
    assert!(words.len() > 1);
    for word in words {
        let draws = resolve(
            word,
            &images,
            &set.flags,
            game.configuration[0] as u32,
            37,
            |path| {
                if let Some(&size) = cache.get(path) {
                    return Ok(size);
                }
                let bytes = std::fs::read(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
                let size = wolf_media::Image::from_png(&bytes)?.dimensions();
                cache.insert(path.to_owned(), size);
                Ok(size)
            },
        )
        .unwrap_or_else(|e| panic!("tile {word}: {e}"));
        assert!(draws.len() <= 4);
    }
    assert!(!cache.is_empty());
}
