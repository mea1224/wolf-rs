//! Bounded Common89 rich text subset. Variable expansion precedes this parser.
//! 94D980 tokens 103/112/127/130/131; 94FC50 clamps font to 2..999 and
//! offsets to -5000..5000. Font rasterization and resource lookup are callbacks.
use std::sync::Arc;
use wolf_media::Image;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    FontSize(u32),
    FontFace(u8),
    Color(i32),
    OutlineColor(i32),
    OutlineWidth(u8),
    CharacterCompression(i32),
    LineSpacing(i32),
    Align(u8),
    AbsoluteX(i32),
    Image(String),
    OffsetX(i32),
    OffsetY(i32),
    NewLine,
    Instant,
}

fn integer(text: &str) -> Result<i32, String> {
    let s = text.trim_start_matches([' ', '\t', '\r', '\n']);
    let sign_len = usize::from(s.starts_with(['+', '-']));
    let digits = s[sign_len..].bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Ok(0);
    }
    s[..sign_len + digits]
        .parse()
        .map_err(|_| "rich text integer overflow".into())
}

pub fn parse(source: &str) -> Result<Vec<Inline>, String> {
    if source.len() > 1 << 20 || source.contains('\0') || source.contains('\u{1}') {
        return Err("rich text source byte/control limit".into());
    }
    let mut items = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if items.len() >= 16_384 {
            return Err("rich text operation limit".into());
        }
        let tail = &source[cursor..];
        if tail.starts_with('\n') {
            items.push(Inline::NewLine);
            cursor += 1;
            continue;
        }
        if tail.starts_with("\r\n") {
            items.push(Inline::NewLine);
            cursor += 2;
            continue;
        }
        if tail.starts_with("\\>") {
            items.push(Inline::Instant);
            cursor += 2;
            continue;
        }
        if let Some((tag, mode)) = [("<L>",0),("<C>",1),("<R>",2)].into_iter().find(|(tag,_)|tail.starts_with(tag)) {
            items.push(Inline::Align(mode));
            cursor += tag.len();
            continue;
        }
        let kind = [("\\f[", 0), ("\\mx[", 1), ("\\my[", 2), ("\\img[", 3), ("\\font[",4), ("\\c[",5), ("\\space[",6), ("\\ax[",7), ("\\cE[",8), ("\\wE[",9), ("\\-[",10)]
            .into_iter()
            .find(|(prefix, _)| tail.starts_with(prefix));
        if let Some((prefix, kind)) = kind {
            let end = tail[prefix.len()..]
                .find(']')
                .ok_or("unterminated rich text control")?
                + prefix.len();
            let argument = &tail[prefix.len()..end];
            let item = match kind {
                0 => Inline::FontSize(integer(argument)?.clamp(2, 999) as u32),
                1 => Inline::OffsetX(integer(argument)?.clamp(-5000, 5000)),
                2 => Inline::OffsetY(integer(argument)?.clamp(-5000, 5000)),
                4 => Inline::FontFace(integer(argument)?.clamp(0,3) as u8),
                5 => Inline::Color(integer(argument)?),
                6 => Inline::LineSpacing(integer(argument)?.clamp(-1000,1000)),
                7 => Inline::AbsoluteX(integer(argument)?.clamp(-5000,5000)),
                8 => Inline::OutlineColor(integer(argument)?.clamp(-1,9999)),
                9 => Inline::OutlineWidth(integer(argument)?.clamp(1,5) as u8),
                10 => Inline::CharacterCompression(integer(argument)?.clamp(-100,100)),
                _ => {
                    if argument.is_empty()
                        || argument.len() > 4096
                        || argument.contains(['[', '\n', '\r'])
                    {
                        return Err("unsupported inline image path".into());
                    }
                    Inline::Image(argument.to_owned())
                }
            };
            items.push(item);
            cursor += end + 1;
        } else if tail.starts_with(['\\', '\r']) {
            return Err(format!("unsupported rich text control near {tail:?}"));
        } else {
            let len = tail.find(['\\', '\r', '\n', '<']).unwrap_or(tail.len());
            let len = if len == 0 { tail.chars().next().unwrap().len_utf8() } else { len };
            items.push(Inline::Text(tail[..len].to_owned()));
            cursor += len;
        }
    }
    Ok(items)
}

#[derive(Clone)]
pub struct RasterText {
    pub image: Arc<Image>,
    /// Logical advance can differ from the glyph ink's bitmap width.
    pub advance: i32,
    pub line_height: u32,
}

pub struct PositionedImage {
    pub image: Arc<Image>,
    pub x: i32,
    pub y: i32,
    pub reveal_at: u32,
}
pub struct TextLayout {
    pub width: u32,
    pub height: u32,
    pub images: Vec<PositionedImage>,
    pub reveal_units: u32,
    pub instant_ranges: Vec<[u32; 2]>,
}

/// Common image/delta cursor layout. 975D10:97A861..97A962 aligns images to
/// the current line's bottom; 973B30:9746A9 does the same for text font height.
/// Scaling, alternate fonts and character spacing remain separate profiles.
#[cfg(test)]
#[allow(dead_code)] // Used by the separate native_text integration harness.
pub fn layout(
    source: &str,
    default_size: u32,
    load_image: impl FnMut(&str) -> Result<Arc<Image>, String>,
    mut raster_text: impl FnMut(&str, u32) -> Result<RasterText, String>,
) -> Result<TextLayout, String> {
    layout_with_style(source, default_size, load_image, |text,size,font,color| {
        if font != 0 || color != 0 { return Err("styled text requires a font/palette callback".into()); }
        raster_text(text,size)
    })
}

#[cfg(test)]
#[allow(dead_code)] // Used by the separate native_text integration harness.
pub fn layout_with_style(
    source: &str,
    default_size: u32,
    load_image: impl FnMut(&str) -> Result<Arc<Image>, String>,
    raster_text: impl FnMut(&str, u32, u8, i32) -> Result<RasterText, String>,
) -> Result<TextLayout, String> {
    layout_with_spacing(source, default_size, 0, 32, load_image, raster_text)
}

#[cfg(test)]
#[allow(dead_code)] // Used by the separate native_text integration harness.
pub fn layout_with_spacing(
    source: &str,
    default_size: u32,
    default_spacing: i32,
    spacing_scale: i32,
    load_image: impl FnMut(&str) -> Result<Arc<Image>, String>,
    mut raster_text: impl FnMut(&str, u32, u8, i32) -> Result<RasterText, String>,
) -> Result<TextLayout, String> {
    layout_with_outline(source,default_size,default_spacing,spacing_scale,load_image,|text,size,font,color,width,_edge| {
        if width != 0 { return Err("outlined text requires an outline raster callback".into()); }
        raster_text(text,size,font,color)
    })
}

pub fn layout_with_outline(
    source: &str,
    default_size: u32,
    default_spacing: i32,
    spacing_scale: i32,
    load_image: impl FnMut(&str) -> Result<Arc<Image>, String>,
    raster_text: impl FnMut(&str, u32, u8, i32, u8, i32) -> Result<RasterText, String>,
) -> Result<TextLayout, String> {
    layout_impl(source, default_size, default_spacing, spacing_scale, false, load_image, raster_text)
}

pub fn layout_progressive(
    source: &str,
    default_size: u32,
    default_spacing: i32,
    spacing_scale: i32,
    load_image: impl FnMut(&str) -> Result<Arc<Image>, String>,
    raster_text: impl FnMut(&str, u32, u8, i32, u8, i32) -> Result<RasterText, String>,
) -> Result<TextLayout, String> {
    layout_impl(source, default_size, default_spacing, spacing_scale, true, load_image, raster_text)
}

#[allow(clippy::too_many_arguments)]
fn layout_impl(
    source: &str, default_size: u32, default_spacing: i32, spacing_scale: i32,
    progressive: bool,
    mut load_image: impl FnMut(&str) -> Result<Arc<Image>, String>,
    mut raster_text: impl FnMut(&str, u32, u8, i32, u8, i32) -> Result<RasterText, String>,
) -> Result<TextLayout, String> {
    let mut font_size = default_size.clamp(2, 999);
    let (mut outline_width, mut outline_color) = (0u8,-1i32);
    let (mut font_face, mut color, mut line_spacing, mut alignment) = (0u8,0i32,default_spacing,0u8);
    let (mut x, mut y) = (0i32, 0i32);
    let (mut width, mut height) = (0i32, 0i32);
    let mut row_extent = 0i32;
    // 973B30 initializes the first row to zero; an initial f22 must not
    // inherit the default font's 28-pixel height before any glyph is emitted.
    let mut line_height = 0;
    let mut images = Vec::new();
    let mut image_rows = Vec::new();
    let mut row_widths = Vec::new();
    let mut current_row_width = 0;
    let mut compression = 0;
    let mut reveal_units = 0u32;
    let mut instant_ranges = Vec::new();
    let mut instant_start = None;
    let mut pending = std::collections::VecDeque::from(parse(source)?);
    while let Some(item) = pending.pop_front() {
        // 94F103 token107 -> 952B1B signed compression -> packet7.
        // 975AC3 advances each glyph by its measured width minus compression.
        let text_item = matches!(item, Inline::Text(_));
        let mut raster = match item {
            Inline::CharacterCompression(value) => { compression = value; continue; }
            Inline::FontSize(size) => {
                font_size = size;
                line_height = line_height.max(size);
                continue;
            }
            Inline::FontFace(face) => { font_face = face; continue; }
            Inline::Color(value) => { color = value; continue; }
            Inline::OutlineColor(value) => { outline_color = value; continue; }
            Inline::OutlineWidth(value) => { outline_width = value; continue; }
            Inline::LineSpacing(value) => { line_spacing = value; continue; }
            Inline::Align(value) => { alignment = value; continue; }
            Inline::AbsoluteX(value) => { x = value; width = width.max(x); current_row_width = current_row_width.max(x); continue; }
            Inline::OffsetX(delta) => {
                x = x.checked_add(delta).ok_or("text x overflow")?;
                width = width.max(x);
                current_row_width = current_row_width.max(x);
                row_extent = y + line_height as i32;
                continue;
            }
            Inline::OffsetY(delta) => {
                y = y.checked_add(delta).ok_or("text y overflow")?;
                continue;
            }
            Inline::NewLine => {
                reveal_units += 1;
                height = height.max(row_extent);
                row_widths.push(current_row_width);
                current_row_width = 0;
                x = 0;
                y = y
                    .checked_add(line_height as i32 + line_spacing * spacing_scale / 32)
                    .ok_or("text newline overflow")?;
                line_height = font_size;
                row_extent = y + line_height as i32;
                continue;
            }
            Inline::Instant => { instant_start.get_or_insert(reveal_units + 1); continue; }
            Inline::Image(path) => {
                let image = load_image(&path)?;
                let (w, h) = image.dimensions();
                RasterText {
                    image,
                    advance: w as i32,
                    line_height: h,
                }
            }
            Inline::Text(text) => {
                if (progressive || compression != 0) && text.chars().count() > 1 {
                    let mut chars = text.chars();
                    let first = chars.next().unwrap();
                    for ch in chars.rev() { pending.push_front(Inline::Text(ch.to_string())); }
                    reveal_units += first.len_utf8().min(2) as u32;
                    raster_text(&first.to_string(), font_size, font_face, color, outline_width, outline_color)?
                } else {
                    reveal_units += text.chars().map(|c| c.len_utf8().min(2) as u32).sum::<u32>();
                    raster_text(&text, font_size, font_face, color, outline_width, outline_color)?
                }
            }
        };
        raster.advance -= compression;
        if text_item { raster.advance = raster.advance.max(0); }
        line_height = line_height.max(raster.line_height);
        let placed_y = y
            .checked_add((line_height - raster.line_height) as i32)
            .ok_or("text baseline overflow")?;
        images.push(PositionedImage {
            image: raster.image,
            x,
            y: placed_y,
            reveal_at: reveal_units,
        });
        image_rows.push((row_widths.len(), alignment));
        x = x
            .checked_add(raster.advance)
            .ok_or("text advance overflow")?;
        // 9739C0 keeps maximum logical x, but replaces the row y and height.
        // 973530 measures the resulting row data, not all glyph/image ink bounds.
        width = width.max(x);
        current_row_width = current_row_width.max(x);
        row_extent = y
            .checked_add(line_height as i32)
            .ok_or("text height overflow")?;
        if x.abs() > 32768 || y.abs() > 32768 || width > 32768 || height > 32768 {
            return Err("rich text extent limit".into());
        }
    }
    row_widths.push(current_row_width);
    if let Some(start) = instant_start { instant_ranges.push([start, reveal_units]); }
    for (placed, (row, alignment)) in images.iter_mut().zip(image_rows) {
        placed.x += (width - row_widths[row]) * i32::from(alignment) / 2;
    }
    let width = width.max(1) as u32;
    let height = height.max(row_extent).max(1) as u32;
    if u64::from(width) * u64::from(height) > 16_000_000 {
        return Err("rich text pixel budget".into());
    }
    Ok(TextLayout {
        width,
        height,
        images,
        reveal_units,
        instant_ranges,
    })
}

impl TextLayout {
    pub fn image(&self) -> Result<Arc<Image>, String> {
        let mut pixels = vec![0u8; self.width as usize * self.height as usize * 4];
        for placed in &self.images {
            let (width, height) = placed.image.dimensions();
            for sy in 0..height {
                let dy = placed.y + sy as i32;
                if !(0..self.height as i32).contains(&dy) {
                    continue;
                }
                for sx in 0..width {
                    let dx = placed.x + sx as i32;
                    if !(0..self.width as i32).contains(&dx) {
                        continue;
                    }
                    let at = (sy as usize * width as usize + sx as usize) * 4;
                    let source = &placed.image.bytes()[at..at + 4];
                    let at = (dy as usize * self.width as usize + dx as usize) * 4;
                    let dest = &mut pixels[at..at + 4];
                    let sa = u32::from(source[3]);
                    let da = u32::from(dest[3]);
                    let alpha = sa * 255 + da * (255 - sa);
                    if alpha != 0 {
                        for channel in 0..3 {
                            dest[channel] = ((u32::from(source[channel]) * sa * 255
                                + u32::from(dest[channel]) * da * (255 - sa)
                                + alpha / 2)
                                / alpha) as u8;
                        }
                    }
                    dest[3] = ((alpha + 127) / 255) as u8;
                }
            }
        }
        Image::new(self.width as usize, self.height as usize, pixels).map(Arc::new)
    }
}

/// Translate native saved-picture control packets to the supported text markup.
/// Packet length is byte1 + 1 (A11ADC); some numeric forms include a final
/// 0x01 byte counted by that length. It is not the start of the next packet.
pub fn decode_saved_text(source: &[u8]) -> Result<String, String> {
    if source.len() > 1 << 20 || source.contains(&0) {
        return Err("saved text source limit or embedded NUL".into());
    }
    let mut result = String::new();
    let mut cursor = 0;
    let mut visible = false;
    while cursor < source.len() {
        if source[cursor] != 1 {
            let length = source[cursor..].iter().position(|&b| b == 1)
                .unwrap_or(source.len() - cursor);
            let text = std::str::from_utf8(&source[cursor..cursor + length])
                .map_err(|_| "saved text contains invalid UTF-8")?;
            visible |= text.chars().any(|c| !c.is_whitespace());
            result.push_str(text);
            cursor += length;
            continue;
        }
        let length = usize::from(*source.get(cursor + 1).ok_or("truncated saved text packet")?) + 1;
        if length < 3 { return Err("invalid saved text packet length".into()); }
        let packet = source.get(cursor..cursor + length).ok_or("truncated saved text packet")?;
        let code = packet[2];
        let data = &packet[3..];
        let numeric = |digits: usize, terminator: bool| -> Result<i32, String> {
            if data.len() != digits + usize::from(terminator)
                || terminator && data.last() != Some(&1)
                || data.first().is_none_or(|&b| b < 14)
                || data[1..digits].iter().any(|&b| !(14..78).contains(&b)) {
                return Err(format!("invalid saved text packet {code:#x} numeric payload"));
            }
            Ok(data[..digits].iter().fold(0, |value, &b| value * 64 + i32::from(b - 14)))
        };
        match code {
            2 => result.push_str(&format!("\\c[{}]", numeric(3, true)?)),
            3 => {
                let digits = match length { 5 => 1, 6 => 2, _ => return Err("invalid saved font-size packet".into()) };
                result.push_str(&format!("\\f[{}]", numeric(digits, true)?));
            }
            0x11 => result.push_str(&format!("\\font[{}]", numeric(1, true)?.clamp(0, 3))),
            0x12 => result.push_str(&format!("\\space[{}]", numeric(2, false)? - 1000)),
            0x13..=0x15 if data.is_empty() => result.push_str(["<L>", "<C>", "<R>"][usize::from(code - 0x13)]),
            0x18 | 0x19 | 0x1d => {
                let tag = match code { 0x18 => "mx", 0x19 => "my", _ => "ax" };
                result.push_str(&format!("\\{tag}[{}]", numeric(2, true)? - 5000));
            }
            7 if data.len() == 1 => result.push_str(&format!("\\-[{}]", i32::from(data[0])-120)),
            // Instant reveal does not change the final saved picture.
            0x20 if data.is_empty() => {}
            0x21 => { let value = numeric(3, false)? - 1; result.push_str(&format!("\\cE[{value}]")); }
            0x23 => { let value = numeric(2, true)?; result.push_str(&format!("\\wE[{value}]")); }
            _ => return Err(format!("unsupported saved text packet {code:#x}")),
        }
        cursor += length;
    }
    // Native saves can retain only formatting packets after a message clears.
    // No glyph can consume the unsupported style in this case.
    if !visible { return Ok(String::new()); }
    Ok(result)
}
