//! Current GamePro tile atlas profile. Evidence: validation/native-scene-render/tile-audit.json.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileDraw {
    pub path: String,
    /// Atlas x, y, width, height in pixels.
    pub source: [u32; 4],
    /// Tile-local x, y, width, height; the scene supplies the map position.
    pub destination: [i32; 4],
    /// 9D0B20: bit0x10 wins (2), otherwise bit0x100 selects 1/0.
    pub priority: u8,
}

pub(crate) fn animation_period(path: &str) -> u64 {
    let Some((_, suffix)) = path.split_once("_FRAME=") else {
        return 20;
    };
    // 9D17A9 calls atoi; 9D17B2 clamps its result to at least one.
    let suffix = suffix.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (negative, digits) = if let Some(s) = suffix.strip_prefix('-') {
        (true, s)
    } else {
        (false, suffix.strip_prefix('+').unwrap_or(suffix))
    };
    let value = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0_u64, |value, digit| {
            value
                .saturating_mul(10)
                .saturating_add(u64::from(digit - b'0'))
        });
    if negative {
        1
    } else {
        value.clamp(1, i32::MAX as u64)
    }
}

/// Resolve one map word. `images` is TileSet.images (base atlas then 31 auto
/// atlases); `tick` is the native map animation clock corresponding to 14E0F6C.
/// Atlas dimensions come from the scene's image cache. No image pixels are copied.
pub fn resolve(
    word: i32,
    images: &[String],
    flags: &[i32],
    tile_px: u32,
    tick: u64,
    mut dimensions: impl FnMut(&str) -> Result<(u32, u32), String>,
) -> Result<Vec<TileDraw>, String> {
    // 9D2BB8 skips negative words, and 9D238B skips auto group1.
    if word < 0 || (100_000..200_000).contains(&word) {
        return Ok(Vec::new());
    }
    if tile_px == 0 || tile_px > 8192 || !tile_px.is_multiple_of(2) {
        return Err(format!("unsupported tile pixel size {tile_px}"));
    }
    let auto = word >= 100_000;
    let group = word as usize / 100_000;
    if auto && !(2..=32).contains(&group) {
        return Err(format!(
            "unsupported auto tile group {group} for word {word}"
        ));
    }
    let image_index = if auto { group - 1 } else { 0 };
    let path = images
        .get(image_index)
        .ok_or("tile image index exceeds tileset")?;
    // Empty tileset entries are disabled by 9D1337/9D17FE.
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let flag_index = if auto { group - 1 } else { word as usize + 32 };
    let flag = flags.get(flag_index).copied().unwrap_or(0);
    let priority = if flag & 0x10 != 0 {
        2
    } else {
        ((flag >> 8) & 1) as u8
    };
    let (width, height) = dimensions(path)?;
    if !auto {
        // 9D1132 validates eight columns; 94CD70 appends x inside the y loop.
        if width != tile_px * 8 || height % tile_px != 0 {
            return Err(format!(
                "base tile atlas {path} must have eight whole-tile columns"
            ));
        }
        let index = word as u32;
        let y = (index / 8)
            .checked_mul(tile_px)
            .ok_or("tile source overflow")?;
        if y.checked_add(tile_px).is_none_or(|bottom| bottom > height) {
            return Err(format!("tile word {word} exceeds atlas {path}"));
        }
        return Ok(vec![TileDraw {
            path: path.clone(),
            source: [(index % 8) * tile_px, y, tile_px, tile_px],
            destination: [0, 0, tile_px as i32, tile_px as i32],
            priority,
        }]);
    }
    // 9D14C4: frame count=width/tile; 9D14E8 requires >=1 frame and 5 rows.
    if width < tile_px || width % tile_px != 0 || height < tile_px * 5 || height % tile_px != 0 {
        return Err(format!(
            "auto tile atlas {path} needs whole frames and at least five rows"
        ));
    }
    let frames = width / tile_px;
    let frame = ((tick / animation_period(path)) % u64::from(frames)) as u32;
    let code = word as u32 % 100_000;
    let rows = [code / 1000, (code / 100) % 10, (code / 10) % 10, code % 10];
    let half = tile_px / 2;
    let mut draws = Vec::with_capacity(4);
    for (quadrant, row) in rows.into_iter().enumerate() {
        let x = (quadrant as u32 % 2) * half;
        let y = (quadrant as u32 / 2) * half;
        let source_y = row
            .checked_mul(tile_px)
            .and_then(|n| n.checked_add(y))
            .ok_or("auto tile source overflow")?;
        if source_y
            .checked_add(half)
            .is_none_or(|bottom| bottom > height)
        {
            return Err(format!(
                "auto tile word {word} quadrant {quadrant} exceeds atlas {path}"
            ));
        }
        // 9D244F..9D2500 selects TL/TR/BL/BR halves from each encoded row.
        draws.push(TileDraw {
            path: path.clone(),
            source: [frame * tile_px + x, source_y, half, half],
            destination: [x as i32, y as i32, half as i32, half as i32],
            priority,
        });
    }
    Ok(draws)
}
