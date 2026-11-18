//! Actor atlas selection from this GamePro.exe's 9CE970 / 9CA770 / 9F3AC0.
//! Pixel dimensions come from the decoded image, including assets named `xN`.

#[derive(Debug, Clone, Copy)]
pub struct ActorSpec<'a> {
    pub graphic: &'a [u8],
    /// Native page +0 / actor +0x44. Nonnegative values select the tileset atlas.
    pub tile_id: i32,
    pub direction: i16,
    pub frame: i16,
    /// Native moving/animation state consumed by T/TX sheet selection.
    pub animated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorLayout {
    pub source: [u32; 4],
    pub size: [u32; 2],
    /// Offset from the image's top left to the actor's foot point.
    pub origin: [i32; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ordinary,
    Standing,
    Twin,
    Single,
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|part| part == needle)
}

fn mode(path: &[u8]) -> Mode {
    // 9F3AC0 tests substrings in this order; mixed-case extensions aren't folded.
    let mut result = Mode::Ordinary;
    if contains(path, b"T.png") || contains(path, b"T.PNG") {
        result = Mode::Standing;
    }
    if contains(path, b"TX.png") || contains(path, b"TX.PNG") {
        result = Mode::Twin;
    }
    if contains(path, b"$.png") || contains(path, b"$.PNG") {
        result = Mode::Single;
    }
    result
}

pub fn layout(
    actor: ActorSpec<'_>,
    image_size: [u32; 2],
    tile_px: u32,
    directions: u8,
    patterns: u8,
) -> Result<ActorLayout, String> {
    let [width, height] = image_size;
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err("actor image dimensions exceed drawable bounds".into());
    }
    if actor.tile_id >= 0 {
        // 9CED22: one-based resource handle = tile_id + 1; 8 columns.
        if tile_px == 0 || tile_px > i32::MAX as u32 {
            return Err("actor tile size exceeds drawable bounds".into());
        }
        let index = actor.tile_id as u32;
        let x = (index % 8)
            .checked_mul(tile_px)
            .ok_or("actor tile X overflow")?;
        let y = (index / 8)
            .checked_mul(tile_px)
            .ok_or("actor tile Y overflow")?;
        if x.checked_add(tile_px).is_none_or(|right| right > width)
            || y.checked_add(tile_px).is_none_or(|bottom| bottom > height)
        {
            return Err(format!(
                "actor tile {} lies outside tileset image",
                actor.tile_id
            ));
        }
        return Ok(ActorLayout {
            source: [x, y, tile_px, tile_px],
            size: [tile_px, tile_px],
            origin: [(tile_px / 2) as i32, tile_px as i32],
        });
    }
    if actor.tile_id != -1 {
        return Err(format!(
            "unsupported actor graphic selector {}",
            actor.tile_id
        ));
    }
    if actor.graphic.starts_with(b"<") {
        return Err("actor graphic prefix transformations are not implemented".into());
    }
    let sheet_mode = mode(actor.graphic);
    if sheet_mode == Mode::Single {
        return Ok(ActorLayout {
            source: [0, 0, width, height],
            size: [width, height],
            origin: [(width / 2) as i32, height as i32],
        });
    }
    if !matches!(directions, 4 | 8) || patterns == 0 || patterns > i8::MAX as u8 {
        return Err("unsupported actor direction/pattern configuration".into());
    }
    if actor.frame < 0 || actor.frame >= i16::from(patterns) {
        return Err(format!(
            "actor frame {} outside {} patterns",
            actor.frame, patterns
        ));
    }
    let diagonal = actor.direction & 1 != 0;
    let row = match actor.direction {
        2 => 0,
        4 => 1,
        6 => 2,
        8 => 3,
        1 if directions == 8 => 0,
        3 if directions == 8 => 1,
        7 if directions == 8 => 2,
        9 if directions == 8 => 3,
        other => return Err(format!("unsupported actor direction {other}")),
    };
    let patterns = u32::from(patterns);
    let half_columns = match sheet_mode {
        Mode::Ordinary => patterns,
        Mode::Standing => patterns + 1,
        Mode::Twin => patterns * 2,
        Mode::Single => unreachable!(),
    };
    let columns = half_columns * if directions == 8 { 2 } else { 1 };
    // Native integer division deliberately ignores any remainder at the edge.
    let cell_w = width / columns;
    let cell_h = height / 4;
    if cell_w == 0 || cell_h == 0 {
        return Err("actor image is smaller than its configured atlas".into());
    }
    let column = match sheet_mode {
        Mode::Ordinary => actor.frame as u32,
        Mode::Standing if actor.animated => actor.frame as u32 + 1,
        Mode::Standing => 0,
        Mode::Twin => actor.frame as u32 + if actor.animated { patterns } else { 0 },
        Mode::Single => unreachable!(),
    } + if diagonal { half_columns } else { 0 };
    Ok(ActorLayout {
        source: [column * cell_w, row * cell_h, cell_w, cell_h],
        size: [cell_w, cell_h],
        origin: [(cell_w / 2) as i32, cell_h as i32],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(graphic: &[u8], direction: i16, frame: i16, animated: bool) -> ActorSpec<'_> {
        ActorSpec {
            graphic,
            tile_id: -1,
            direction,
            frame,
            animated,
        }
    }

    #[test]
    fn cardinal_and_diagonal_frames_use_separate_horizontal_halves() {
        for (direction, row, diagonal) in [
            (2, 0, false),
            (4, 1, false),
            (6, 2, false),
            (8, 3, false),
            (1, 0, true),
            (3, 1, true),
            (7, 2, true),
            (9, 3, true),
        ] {
            let result = layout(
                actor(b"chara.png", direction, 2, false),
                [288, 256],
                32,
                8,
                3,
            )
            .unwrap();
            assert_eq!(
                result.source,
                [if diagonal { 240 } else { 96 }, row * 64, 48, 64]
            );
            assert_eq!(result.origin, [24, 64]);
        }
    }

    #[test]
    fn standing_and_twin_sheets_switch_only_with_native_animation_state() {
        let standing = actor(b"CharaChip/a_T.png", 6, 1, false);
        assert_eq!(
            layout(standing, [160, 200], 32, 4, 3).unwrap().source,
            [0, 100, 40, 50]
        );
        assert_eq!(
            layout(
                ActorSpec {
                    animated: true,
                    ..standing
                },
                [160, 200],
                32,
                4,
                3
            )
            .unwrap()
            .source,
            [80, 100, 40, 50]
        );
        let twin = actor(b"CharaChip/a_TX.png", 7, 1, false);
        assert_eq!(
            layout(twin, [480, 200], 32, 8, 3).unwrap().source,
            [280, 100, 40, 50]
        );
        assert_eq!(
            layout(
                ActorSpec {
                    animated: true,
                    ..twin
                },
                [480, 200],
                32,
                8,
                3
            )
            .unwrap()
            .source,
            [400, 100, 40, 50]
        );
    }

    #[test]
    fn full_image_and_tile_selectors_bypass_character_direction() {
        let single = layout(actor(b"a$.PNG", 0, 99, false), [71, 83], 32, 4, 3).unwrap();
        assert_eq!(single.source, [0, 0, 71, 83]);
        assert_eq!(single.origin, [35, 83]);
        let tile = ActorSpec {
            tile_id: 10,
            ..actor(b"ignored", 0, 99, false)
        };
        assert_eq!(
            layout(tile, [256, 256], 32, 4, 3).unwrap().source,
            [64, 32, 32, 32]
        );
        assert!(layout(tile, [32, 32], 32, 4, 3).is_err());
    }

    #[test]
    fn source_dimensions_and_pattern_configuration_define_cell_size() {
        let result = layout(actor(b"chara_x3.png", 8, 2, false), [450, 360], 32, 4, 5).unwrap();
        assert_eq!(result.source, [180, 270, 90, 90]);
        assert_eq!(result.size, [90, 90]);
        assert!(layout(actor(b"a.png", 5, 1, false), [96, 128], 32, 4, 3).is_err());
        assert!(layout(actor(b"a.png", 2, 3, false), [96, 128], 32, 4, 3).is_err());
        assert!(layout(actor(b"<<HALF>>a.png", 2, 1, false), [96, 128], 32, 4, 3).is_err());
    }
}
