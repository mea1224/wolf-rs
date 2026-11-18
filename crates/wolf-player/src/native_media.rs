//! Script-driven native picture/transition subset. No title sequence is embedded here.
//!
//! Evidence: docs-append/media-execution-static-report.md, A18DF0, 98A2F0,
//! 996470, 996B80, 964370, 9652B0. Unsupported modes return an error before mutation.
use std::{collections::BTreeMap, sync::Arc};
use wolf_media::{Image, Picture, Pictures};
#[path = "native_media/filter.rs"]
mod filter;
#[path = "native_media/blend.rs"]
mod blend;
#[path = "native_media/screen.rs"]
mod screen;
#[path = "native_media/text.rs"]
pub mod text;

/// 459460 binds Game.dat configuration[4] through FA5228. A5514A..A55172
/// derives FA2AE8 = 60 / max(1, signed byte). This host currently advances by1.
pub fn validate_timing_configuration(configuration: &[u8]) -> Result<(), String> {
    let byte = *configuration
        .get(4)
        .ok_or("Game.dat timing configuration is missing")?;
    let rate = i32::from(byte as i8).max(1);
    let step = 60 / rate;
    if step != 1 {
        return Err(format!(
            "Game.dat timing step {step} is unsupported by the step1 media host"
        ));
    }
    Ok(())
}

/// 98AFEB..98B062 parses the six decimal digits; 9A0B49..9A0E47
/// draws one row per logical pixel, using digit*29 (clamped), not digit*255/9.
fn gradient_image(text: &str, width: i32, height: i32) -> Result<Arc<Image>, String> {
    let b = text.as_bytes();
    if b.len() != 15 || !matches!(&b[..7], b"<GRADY-" | b"<GRADX-") || b[10] != b'-' || b[14] != b'>' {
        return Err(format!("unsupported native generated picture {text}"));
    }
    let mut colors = [0_i32; 6];
    for (i, index) in [7, 8, 9, 11, 12, 13].into_iter().enumerate() {
        if !b[index].is_ascii_digit() {
            return Err("native gradient requires six decimal digits".into());
        }
        colors[i] = (i32::from(b[index] - b'0') * 29).min(255);
    }
    if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > 16_000_000 {
        return Err("native gradient dimensions exceed limits".into());
    }
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        let color = |position: i32, length: i32| -> [u8; 3] { std::array::from_fn(|i| {
            (i64::from(colors[i])
                - i64::from(colors[i] - colors[i + 3]) * i64::from(position) / i64::from(length))
                as u8
        }) };
        let rgb = color(y, height);
        for p in pixels[y as usize * width as usize * 4..][..width as usize * 4].chunks_exact_mut(4)
        .enumerate() {
            let (x, p) = p;
            p[..3].copy_from_slice(&if b[5] == b'X' { color(x as i32, width) } else { rgb });
            p[3] = 255;
        }
    }
    Ok(Arc::new(Image::new(
        width as usize,
        height as usize,
        pixels,
    )?))
}

/// 98B139 loads a 3x3 skin; 9A0E54..9A1803 preserves corners and
/// stretches edges/center when bit27 (native+2FF) is clear.
fn window_image(source: &Image, width: i32, height: i32) -> Result<Arc<Image>, String> {
    let (sw, sh) = source.dimensions();
    if sw % 3 != 0 || sh % 3 != 0 || sw < 3 || sh < 3 {
        return Err("native window skin requires a divisible 3x3 image".into());
    }
    let (cw, ch) = (sw / 3, sh / 3);
    if width < (cw * 2) as i32
        || height < (ch * 2) as i32
        || i64::from(width) * i64::from(height) > 16_000_000
    {
        return Err(
            "native window dimensions are smaller than corners or exceed pixel budget".into(),
        );
    }
    let (width, height) = (width as u32, height as u32);
    let axis = |position: u32, dest: u32, cell: u32| {
        if position < cell {
            position
        } else if position >= dest - cell {
            2 * cell + position - (dest - cell)
        } else {
            cell + (u64::from(position - cell) * u64::from(cell) / u64::from(dest - 2 * cell))
                as u32
        }
    };
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let sx = axis(x, width, cw);
            let sy = axis(y, height, ch);
            let from = (sy as usize * sw as usize + sx as usize) * 4;
            let to = (y as usize * width as usize + x as usize) * 4;
            pixels[to..to + 4].copy_from_slice(&source.bytes()[from..from + 4]);
        }
    }
    Image::new(width as usize, height as usize, pixels).map(Arc::new)
}

fn generated_image(
    text: &str,
    width: i32,
    height: i32,
    load_image: &mut impl FnMut(&str) -> Result<Arc<Image>, String>,
) -> Result<Arc<Image>, String> {
    if text.starts_with('<') {
        gradient_image(text, width, height)
    } else {
        window_image(load_image(text)?.as_ref(), width, height)
    }
}

fn tinted_channel(destination: u8, source: u8, alpha: u32, mode: u32, tint: i32) -> u8 {
    // 99FA9D chooses the first pass's blend; 99FDD1 applies its negative
    // modulation. 9A1A1E/9A1A69 selects positive modulation and add(2)
    // for the second pass regardless of that original blend mode.
    let base = u32::from(source) * (255 + tint.min(0)).clamp(0, 255) as u32 / 255;
    let first = match mode {
        2 => u32::from(destination).saturating_sub((base * alpha + 127) / 255),
        1 => (u32::from(destination) + (base * alpha + 127) / 255).min(255),
        3 => (u32::from(destination) * (255 * (255 - alpha) + base * alpha) + 32512) / 65025,
        _ => (base * alpha + u32::from(destination) * (255 - alpha) + 127) / 255,
    };
    let positive = (u64::from(source) * u64::from(alpha) * tint.max(0) as u64 + 32512) / 65025;
    (u64::from(first) + positive).min(255) as u8
}

/// Native mode1 maps to DX add(2/18), mode3 to alpha-aware multiply(11).
/// FA3690 blend table and 99FA9D..99FAB9 establish the mapping. Rendering
/// remains byte-space; final GPU UNORM rounding may differ by one unit.
#[allow(clippy::too_many_arguments)] // Direct rectangle/opacity/blend rendering contract.
fn blend_picture(
    target: &mut Image,
    source: &Image,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    opacity: u32,
    mode: u32,
    tint: [i32; 3],
    screen_mask: Option<&[u8]>,
) {
    if opacity == 0 || width == 0 || height == 0 { return; }
    let (tw, th) = target.dimensions();
    let (sw, sh) = source.dimensions();
    let left = i64::from(x).clamp(0, i64::from(tw));
    let right = (i64::from(x) + i64::from(width)).clamp(0, i64::from(tw));
    let top = i64::from(y).clamp(0, i64::from(th));
    let bottom = (i64::from(y) + i64::from(height)).clamp(0, i64::from(th));
    // Exact 1:1 source-over fast path. Preserve destination alpha, as the
    // general native compositor does; do not change byte-space rounding.
    if width == sw && screen_mask.is_none() {
        for dy in top..bottom {
            let count = right.saturating_sub(left) as usize * 4;
            if count == 0 { continue; }
            let sy = (dy - i64::from(y)) * i64::from(sh) / i64::from(height);
            let start = (sy * i64::from(sw) + left - i64::from(x)) as usize * 4;
            let offset = (dy * i64::from(tw) + left) as usize * 4;
            let src = &source.bytes()[start..start + count];
            let dst = &mut target.bytes_mut()[offset..offset + count];
            if mode == 0 && tint == [0; 3] && opacity == 255 && src.chunks_exact(4).all(|p| p[3] == 255) && dst.chunks_exact(4).all(|p| p[3] == 255) {
                dst.copy_from_slice(src);
                continue;
            }
            blend::row(dst, src, opacity, mode, tint);
        }
        return;
    }
    // Nearest source X is identical across rows. Division inside the pixel
    // loop previously cost two full-frame integer divisions for every layer.
    let source_x: Vec<usize> = (left..right).map(|dx|
        ((dx - i64::from(x)) as u64 * u64::from(sw) / u64::from(width)) as usize * 4
    ).collect();
    let mut sampled = vec![0u8; source_x.len() * 4];
    for dy in top..bottom {
        let sy = (dy - i64::from(y)) as u64 * u64::from(sh) / u64::from(height);
        let src_row = &source.bytes()[sy as usize * sw as usize * 4..][..sw as usize * 4];
        let offset = (dy * i64::from(tw) + left) as usize;
        let dst_row = &mut target.bytes_mut()[offset * 4..][..source_x.len() * 4];
        if screen_mask.is_none() {
            for (&sx, pixel) in source_x.iter().zip(sampled.chunks_exact_mut(4)) {
                pixel.copy_from_slice(&src_row[sx..sx+4]);
            }
            blend::row(dst_row, &sampled, opacity, mode, tint);
            continue;
        }
        for (column, (&sx, dst)) in source_x.iter().zip(dst_row.chunks_exact_mut(4)).enumerate() {
            if screen_mask.is_some_and(|mask| mask[offset + column] != 0) { continue; }
            let src = &src_row[sx..sx+4];
            let alpha = (u32::from(src[3]) * opacity + 127) / 255;
            if alpha == 0 { continue; }
            for c in 0..3 { dst[c] = tinted_channel(dst[c], src[c], alpha, mode, tint[c]); }
        }
    }
}

/// 9A0114..9A02C9 passes independent signed scales and tenth-degree
/// rotation to 7652E0. This CPU path uses nearest samples; GPU edge coverage
/// remains a separate validation boundary.
#[allow(clippy::too_many_arguments)]
fn transformed_picture(
    target: &mut Image,
    source: &Image,
    position: [f64; 2],
    size: [f64; 2],
    anchor: u32,
    rotation: i32,
    opacity: u32,
    mode: u32,
    tint: [i32; 3],
    screen_mask: Option<&[u8]>,
) {
    if size[0].abs() < 0.0001 || size[1].abs() < 0.0001 || opacity == 0 {
        return;
    }
    let (sw, sh) = source.dimensions();
    let (tw, th) = target.dimensions();
    let (ax, ay) = anchor_offset(anchor, sw, sh);
    let center = [
        position[0] + (0.5 - f64::from(ax) / f64::from(sw)) * size[0],
        position[1] + (0.5 - f64::from(ay) / f64::from(sh)) * size[1],
    ];
    let radians = f64::from(rotation) * 0.0017453292519444446;
    let (sin, cos) = radians.sin_cos();
    let half = [
        (size[0] * cos).abs() / 2.0 + (size[1] * sin).abs() / 2.0,
        (size[0] * sin).abs() / 2.0 + (size[1] * cos).abs() / 2.0,
    ];
    let left = (center[0] - half[0]).floor().max(0.0) as u32;
    let top = (center[1] - half[1]).floor().max(0.0) as u32;
    let right = (center[0] + half[0]).ceil().clamp(0.0, f64::from(tw)) as u32;
    let bottom = (center[1] + half[1]).ceil().clamp(0.0, f64::from(th)) as u32;
    for y in top..bottom {
        for x in left..right {
            if screen_mask.is_some_and(|mask| mask[y as usize * tw as usize + x as usize] != 0) {
                continue;
            }
            let dx = f64::from(x) + 0.5 - center[0];
            let dy = f64::from(y) + 0.5 - center[1];
            let u = (dx * cos + dy * sin) / size[0] + 0.5;
            let v = (-dx * sin + dy * cos) / size[1] + 0.5;
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let sx = (u * f64::from(sw)) as usize;
            let sy = (v * f64::from(sh)) as usize;
            let src = &source.bytes()[(sy * sw as usize + sx) * 4..][..4];
            let dst = &mut target.bytes_mut()[(y as usize * tw as usize + x as usize) * 4..][..4];
            let alpha = (u32::from(src[3]) * opacity + 127) / 255;
            for c in 0..3 {
                dst[c] = tinted_channel(dst[c], src[c], alpha, mode, tint[c]);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOutcome {
    Applied,
    /// Native AL=0: retain the command PC and retry on a later host tick.
    Retry,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Geometry {
    x: i32,
    y: i32,
    scale: i32,
    scale_y: i32,
    rotation: i32,
    opacity: i32,
    color: [i32; 3],
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Entry {
    #[serde(with = "saved_image")]
    image: Arc<Image>,
    frames: Option<FrameAnimation>,
    #[serde(default)]
    text_reveal: Option<text::TextReveal>,
    path: String,
    anchor: u32,
    blend: u32,
    from: Geometry,
    to: Geometry,
    duration: i32,
    remaining: i32,
    deleting: bool,
    swing: Option<Swing>,
    scale_effect: ScaleEffect,
    zoom: Zoom,
    scale_pulse: Option<ScalePulse>,
    offset_effect: ScaleEffect,
    opacity_effect: ScaleEffect,
    color_effect: Option<ColorEffect>,
    color_flash: Option<ColorFlash>,
    #[serde(default)]
    color_overlay: Option<ColorOverlay>,
    color_transition: ColorTransition,
    pending: bool,
    masked: bool,
    map_relative: bool,
}

/// 997162..997391 updates one animation count per host tick. This subset
/// admits horizontal strips only; other atlas orders require separate evidence.
#[derive(serde::Serialize, serde::Deserialize)]
struct FrameAnimation {
    #[serde(with = "saved_image")]
    sheet: Arc<Image>,
    columns: i32,
    base: i32,
    first: i32,
    last: i32,
    current: i32,
    remaining: i32,
    duration: i32,
    mode: i32,
}
impl FrameAnimation {
    fn validate(&self) -> Result<(), String> {
        validate_duration(self.duration)?;
        let (w, _) = self.sheet.dimensions();
        if self.columns <= 0
            || self.columns > 4096
            || w % self.columns as u32 != 0
            || !(1..=self.columns).contains(&self.base)
            || self.duration > 0
                && (!(1..=self.columns).contains(&self.first)
                    || !(1..=self.columns).contains(&self.last)
                    || !(self.first.min(self.last)..=self.first.max(self.last))
                        .contains(&self.current)
                    || self.remaining <= 0
                    || self.remaining > self.duration
                    || !matches!(self.mode, -2 | 0 | 1 | 2))
        {
            return Err("unsupported/invalid saved horizontal frame animation".into());
        }
        Ok(())
    }
    fn image(&self) -> Result<Arc<Image>, String> {
        let (sw, h) = self.sheet.dimensions();
        let w = sw / self.columns as u32;
        let index = if self.duration > 0 {
            self.current
        } else {
            self.base
        } - 1;
        let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            let start = (y as usize * sw as usize + index as usize * w as usize) * 4;
            pixels.extend_from_slice(&self.sheet.bytes()[start..start + w as usize * 4]);
        }
        Image::new(w as usize, h as usize, pixels).map(Arc::new)
    }
    fn tick(&mut self) -> Result<Option<Arc<Image>>, String> {
        if self.duration <= 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        if self.remaining != 0 {
            return Ok(None);
        }
        let step = (self.last - self.first).signum();
        match self.mode {
            0 if self.current == self.last => self.duration = 0,
            0 => {
                self.current += step;
                self.base = self.current;
            }
            1 if self.current == self.last => self.current = self.first,
            1 => self.current += step,
            2 if self.current == self.last => {
                self.mode = -2;
                self.current -= step;
            }
            2 => self.current += step,
            -2 if self.current == self.first => {
                self.mode = 2;
                self.current += step;
            }
            -2 => self.current -= step,
            _ => return Err("unsupported saved frame animation mode".into()),
        }
        self.remaining = self.duration;
        self.image().map(Some)
    }
}

/// 9A3D7B / 997136 / 998599: a triangular additive scale pulse, then a
/// cooldown while remaining is negative. Zero duration disables the consumer.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ScalePulse {
    amount: [i32; 2],
    duration: i32,
    remaining: i32,
    pause: i32,
}
impl ScalePulse {
    fn current(self) -> [f32; 2] {
        if self.duration <= 0 || self.remaining < 0 {
            return [0.0; 2];
        }
        self.amount.map(|v| {
            v as f32 * (self.duration - (self.remaining - self.duration).abs()) as f32
                / self.duration as f32
        })
    }
    fn tick(&mut self) {
        if self.duration > 0 {
            self.remaining -= 1;
            if self.remaining <= -self.pause {
                self.remaining = 2 * self.duration;
            }
        }
    }
}

/// 9A3E6C/9973E0/998495: independent X/Y multipliers over the base scale.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ScaleEffect {
    from: [i32; 2],
    to: [i32; 2],
    duration: i32,
    remaining: i32,
}
impl Default for ScaleEffect {
    fn default() -> Self {
        Self {
            from: [100; 2],
            to: [100; 2],
            duration: 0,
            remaining: 0,
        }
    }
}
impl ScaleEffect {
    fn set(&mut self, to: [i32; 2], duration: i32) {
        self.from = std::array::from_fn(|i| {
            self.to[i].wrapping_sub(
                self.to[i]
                    .wrapping_sub(self.from[i])
                    .wrapping_mul(self.remaining)
                    / self.duration.max(1),
            )
        });
        self.to = to;
        self.duration = duration;
        self.remaining = duration;
    }
    fn current(self) -> [f32; 2] {
        std::array::from_fn(|i| {
            self.to[i] as f32
                - (self.to[i] - self.from[i]) as f32 * self.remaining as f32
                    / self.duration.max(1) as f32
        })
    }
    fn tick(&mut self) {
        if self.remaining > 0 {
            self.remaining -= 1;
            if self.remaining == 0 {
                self.from = self.to;
            }
        }
    }
}

/// Native type4 zoom keeps a moving center and percentage independently of geometry.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Zoom { from: [i32;3], to: [i32;3], duration: i32, remaining: i32 }
impl Default for Zoom {
    fn default() -> Self { Self { from: [0,0,100], to: [0,0,100], duration: 0, remaining: 0 } }
}
impl Zoom {
    fn current(self) -> [f32;3] {
        std::array::from_fn(|i| self.to[i] as f32 - (self.to[i]-self.from[i]) as f32 * self.remaining as f32 / self.duration.max(1) as f32)
    }
    fn set(&mut self, target: [i32;3], duration: i32) {
        let mut from = std::array::from_fn(|i| self.to[i] - ((i64::from(self.to[i])-i64::from(self.from[i])) * i64::from(self.remaining) / i64::from(self.duration.max(1))) as i32);
        // 9A3BAB: at neutral scale the old center has no visible meaning.
        if from[2] == 100 { from[..2].copy_from_slice(&target[..2]); }
        *self = Self { from, to: target, duration, remaining: duration };
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ColorEffect {
    channels: [i32; 3],
    duration: i32,
    phase: i32,
}

/// 9A3F96 setter and 997D89 draw: a finite triangular flash, independent
/// of the recurring color effect and the permanent color transition.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ColorFlash {
    channels: [i32; 3],
    duration: i32,
    remaining: i32,
}

/// Native picture-effect selector 0 stores packed RGB at +37C and a
/// host-tick countdown at +384. This one-way fade is independent of the
/// interpolated transition and triangular flash effects.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ColorOverlay {
    channels: [i32; 3],
    duration: i32,
    remaining: i32,
}
impl ColorOverlay {
    fn current(self) -> [i32; 3] {
        if self.duration <= 0 || self.remaining <= 0 {
            return [0; 3];
        }
        self.channels.map(|channel| {
            (f64::from(channel - 100) * 2.55 * 2.0 * f64::from(self.remaining)
                / f64::from(self.duration)) as i32
        })
    }
}
impl ColorFlash {
    fn apply(self, color: [i32; 3]) -> [i32; 3] {
        if self.remaining <= 0 { return color; }
        let half = self.duration / 2;
        if half == 0 { return color; }
        let factor = f64::from(half - (self.remaining - half).abs()) / f64::from(half);
        std::array::from_fn(|i| (f64::from(color[i]) + f64::from(self.channels[i] - 100) * 2.55 * factor) as i32)
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct ColorTransition {
    from: [i32; 3],
    to: [i32; 3],
    duration: i32,
    remaining: i32,
}
impl Default for ColorTransition {
    fn default() -> Self {
        Self {
            from: [100; 3],
            to: [100; 3],
            duration: 0,
            remaining: 0,
        }
    }
}
impl ColorTransition {
    fn set(&mut self, channels: [i32; 3], duration: i32) {
        // 9A3802..9A3AA8 snapshots each packed channel with integer division.
        self.from = std::array::from_fn(|i| {
            self.to[i] - (self.to[i] - self.from[i]) * self.remaining / self.duration.max(1)
        });
        self.to = channels.map(|v| (v / 2 + 100).clamp(0, 200));
        self.duration = duration;
        self.remaining = duration;
    }
    fn current(self) -> [i32; 3] {
        // 997E70..997FE2 performs f32 interpolation before f64 channel scaling.
        std::array::from_fn(|i| {
            let channel = (self.to[i] - 100) as f32
                - (self.to[i] - self.from[i]) as f32 * self.remaining as f32
                    / self.duration.max(1) as f32;
            (f64::from(channel) * 2.55 * 2.0) as i32
        })
    }
}
impl ColorEffect {
    fn current(self) -> [i32; 3] {
        if self.duration <= 0 {
            return [0; 3];
        }
        self.channels.map(|c| {
            (f64::from(2 * (c - 100)) * 2.55 * f64::from((self.phase - self.duration).abs())
                / f64::from(self.duration)) as i32
        })
    }
    fn tick(&mut self) {
        if self.duration > 0 {
            self.phase = (self.phase + 1) % (self.duration * 2);
        }
    }
}

/// Native picture +3BC/+3C0/+3CC/+3D0/+3D4. This is a triangle wave,
/// with f32 products and truncation at the two branches in 9920F0..9921DF.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Swing {
    x: i32,
    y: i32,
    count: i32,
    leg: i32,
    remaining: i32,
}

impl Swing {
    fn offset(self) -> (i32, i32) {
        if self.count <= 0 {
            return (0, 0);
        }
        let sign = (self.count & 1) * 2 - 1;
        let axis = |amplitude: i32| {
            let value = if self.remaining >= self.leg {
                amplitude
                    - (amplitude as f32 * (self.remaining - self.leg) as f32 / self.leg as f32)
                        as i32
            } else {
                (amplitude as f32 * self.remaining as f32 / self.leg as f32) as i32
            };
            value * sign
        };
        (axis(self.x), axis(self.y))
    }

    fn tick(&mut self) {
        if self.count <= 0 {
            return;
        }
        // 9970A3 decrements once per host update, independently of global step.
        self.remaining -= 1;
        if self.remaining <= 0 {
            if self.count >= 100_000 {
                self.count = self.count.wrapping_add((!self.count & 1) * 2);
            }
            self.count = self.count.wrapping_sub(1);
            self.remaining = self.leg * 2;
        }
    }
}

impl Entry {
    fn validate_text(&self) -> Result<(), String> {
        if self.text_reveal.as_ref().is_some_and(|text| !text.valid()) {
            return Err("invalid saved text reveal state".into());
        }
        Ok(())
    }
    /// Returns whether the entry's next host tick can change pixels that are
    /// visible in the composed scene. Static entries stay cacheable between
    /// ticks; active effects conservatively invalidate the cache while they
    /// are running.
    fn render_dynamic(&self) -> bool {
        // FrameAnimation::tick reports an actual frame replacement separately.
        // Its countdown alone does not change the visible image.
        (self.duration > 0 && self.remaining > 0)
            || self.swing.as_ref().is_some_and(|swing| swing.count > 0)
            || self.scale_effect.remaining > 0
            || self.offset_effect.remaining > 0
            || self.opacity_effect.remaining > 0
            || self.zoom.remaining > 0
            || self
                .scale_pulse
                .as_ref()
                .is_some_and(|pulse| pulse.duration > 0 && pulse.remaining >= 0)
            || self.color_effect.as_ref().is_some_and(|effect| effect.duration > 0)
            || self
                .color_flash
                .as_ref()
                .is_some_and(|flash| flash.remaining > 0)
            || self
                .color_overlay
                .is_some_and(|overlay| overlay.remaining > 0)
            || self.color_transition.remaining > 0
    }

    fn validate(&self) -> Result<(), String> {
        validate_duration(self.duration)?;
        if self.remaining < 0
            || self.remaining > self.duration
            || self.anchor > 6
            || !matches!(self.blend, 0..=3)
        {
            return Err("invalid saved picture geometry".into());
        }
        for effect in [self.scale_effect, self.offset_effect, self.opacity_effect] {
            validate_duration(effect.duration)?;
            if effect.remaining < 0
                || effect.remaining > effect.duration
                || effect
                    .from
                    .iter()
                    .chain(&effect.to)
                    .any(|v| v.unsigned_abs() > 32768)
            {
                return Err("invalid saved picture scale/offset effect".into());
            }
        }
        if let Some(swing) = self.swing {
            if !(1..=1_000_000).contains(&swing.leg)
                || !(0..=2 * swing.leg).contains(&swing.remaining)
                || swing.x.unsigned_abs() > 32768
                || swing.y.unsigned_abs() > 32768
            {
                return Err("invalid saved picture swing".into());
            }
        }
        if let Some(pulse) = self.scale_pulse {
            validate_duration(pulse.duration)?;
            validate_duration(pulse.pause)?;
            if !(-pulse.pause..=2 * pulse.duration).contains(&pulse.remaining)
                || pulse.amount.iter().any(|v| v.unsigned_abs() > 1600)
            {
                return Err("invalid saved picture scale pulse".into());
            }
        }
        let color = self.color_transition;
        validate_duration(color.duration)?;
        if color.remaining < 0
            || color.remaining > color.duration
            || color
                .from
                .iter()
                .chain(&color.to)
                .any(|v| !(0..=200).contains(v))
        {
            return Err("invalid saved picture color transition".into());
        }
        if let Some(color) = self.color_effect {
            validate_duration(color.duration)?;
            if color.phase < 0
                || color.phase >= (2 * color.duration).max(1)
                || color.channels.iter().any(|v| !(0..=200).contains(v))
            {
                return Err("invalid saved picture color pulse".into());
            }
        }
        if let Some(frames) = &self.frames {
            frames.validate()?;
        }
        if let Some(flash) = self.color_flash {
            validate_duration(flash.duration)?;
            if !(0..=flash.duration).contains(&flash.remaining) || flash.channels.iter().any(|v| !(0..=200).contains(v)) {
                return Err("invalid saved picture color flash".into());
            }
        }
        if let Some(overlay) = self.color_overlay {
            validate_duration(overlay.duration)?;
            if !(0..=overlay.duration).contains(&overlay.remaining)
                || overlay.channels.iter().any(|v| !(0..=200).contains(v))
            {
                return Err("invalid saved picture color overlay".into());
            }
        }
        Ok(())
    }
    fn deleted(&self) -> bool {
        self.deleting && self.remaining <= 0
    }

    fn current(&self) -> Geometry {
        let interpolate = |a: i32, b: i32| {
            if self.duration <= 0 || self.remaining <= 0 {
                b
            } else {
                // Native integer interpolation truncates toward zero.
                (i64::from(b)
                    + (i64::from(a) - i64::from(b)) * i64::from(self.remaining)
                        / i64::from(self.duration)) as i32
            }
        };
        Geometry {
            x: interpolate(self.from.x, self.to.x),
            y: interpolate(self.from.y, self.to.y),
            scale: interpolate(self.from.scale, self.to.scale),
            scale_y: interpolate(self.from.scale_y, self.to.scale_y),
            rotation: interpolate(self.from.rotation, self.to.rotation),
            opacity: interpolate(self.from.opacity, self.to.opacity),
            color: std::array::from_fn(|i| interpolate(self.from.color[i], self.to.color[i])),
        }
    }
}

/// A saved screen is independent of the mutable picture list.
#[derive(serde::Serialize, serde::Deserialize)]
struct Transition {
    kind: i32,
    duration: i32,
    gate: bool,
    #[serde(default)]
    black_backdrop: bool,
    #[serde(with = "saved_optional_image")]
    prepared: Option<Arc<Image>>,
    active: bool,
    timer: i32,
    draw_timer: i32,
    active_duration: i32,
    active_kind: i32,
    #[serde(with = "saved_optional_image")]
    mask: Option<Arc<Image>>,
    #[serde(with = "saved_optional_image")]
    active_mask: Option<Arc<Image>>,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            kind: -1,
            duration: 0,
            gate: false,
            black_backdrop: false,
            prepared: None,
            active: false,
            timer: 0,
            draw_timer: 0,
            active_duration: 0,
            active_kind: -1,
            mask: None,
            active_mask: None,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct NativeMedia {
    width: u32,
    height: u32,
    pictures: BTreeMap<i32, Entry>,
    transition: Transition,
    queued: Vec<QueuedPicture>,
    screen_mask: Option<Vec<u8>>,
    camera_offset: [i32; 2],
    scene_zoom: Zoom,
    #[serde(default)]
    screen: screen::Screen,
    /// Rebuilt from Scene by the composition root after loading a snapshot.
    #[serde(skip)]
    scene_image: Option<Arc<Image>>,
    #[serde(skip)]
    render_cache: Option<Arc<Image>>,
    #[serde(skip)]
    render_dirty: bool,
    #[serde(skip)]
    text_creation: Option<text::TextReveal>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct QueuedPicture {
    opcode: u32,
    raw: Vec<i32>,
    args: Vec<i32>,
    text: String,
    #[serde(with = "saved_optional_image")]
    image: Option<Arc<Image>>,
    delay: i32,
    #[serde(default)]
    text_reveal: Option<text::TextReveal>,
}
impl QueuedPicture {
    fn picture_id(&self) -> i32 { self.args[if self.opcode == 290 { 2 } else { 1 }] }
}

impl NativeMedia {
    pub fn restore_native_screen(&mut self, fields: &BTreeMap<usize, Vec<u8>>) -> Result<(), String> {
        self.screen = screen::Screen::restore_native(fields)?;
        self.render_dirty = true;
        self.render_cache = None;
        Ok(())
    }
    pub fn set_scene_image(&mut self, image: Arc<Image>) -> Result<(), String> {
        if image.dimensions() != (self.width, self.height) {
            return Err("scene image does not match media viewport".into());
        }
        if self.scene_image.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &image)) {
            self.scene_image = Some(image);
            self.render_dirty = true;
            self.render_cache = None;
        }
        Ok(())
    }
    pub fn set_camera_offset(&mut self, offset: [i32; 2]) {
        if self.camera_offset != offset {
            self.camera_offset = offset;
            self.render_dirty = true;
            self.render_cache = None;
        }
    }
    pub fn camera_offset(&self) -> [i32; 2] {
        self.camera_offset
    }
    pub fn picture_position(&self, id: i32) -> Result<Option<[i32; 2]>, String> {
        let Some(entry) = self.pictures.get(&id).filter(|p| !p.deleted()) else {
            return Ok(None);
        };
        let current = entry.current();
        let offset = if entry.map_relative {
            self.camera_offset
        } else {
            [0; 2]
        };
        Ok(Some([
            current
                .x
                .checked_sub(offset[0])
                .ok_or("picture query X overflow")?,
            current
                .y
                .checked_sub(offset[1])
                .ok_or("picture query Y overflow")?,
        ]))
    }
    pub fn picture_effect_position(&self, id: i32) -> Result<Option<[i32; 2]>, String> {
        let Some(mut position) = self.picture_position(id)? else { return Ok(None); };
        let entry = &self.pictures[&id];
        let swing = entry.swing.map(Swing::offset).unwrap_or((0, 0));
        let offset = entry.offset_effect.current();
        for (axis, swing) in [swing.0, swing.1].into_iter().enumerate() {
            position[axis] = (position[axis] as f32 + swing as f32 + offset[axis]) as i32;
        }
        Ok(Some(position))
    }
    pub fn viewport(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn restore_native_picture(
        &mut self,
        fields: &BTreeMap<usize, i32>,
        source: &[u8],
        arrays: &[Vec<i32>],
        mut load_image: impl FnMut(&str) -> Result<Arc<Image>, String>,
    ) -> Result<(), String> {
        let get = |offset| fields.get(&offset).copied().unwrap_or(0);
        let id = get(772);
        validate_picture_layer(id)?;
        if self.pictures.contains_key(&id) || self.pictures.len() >= 4096 {
            return Err("saved picture duplicate/count".into());
        }
        if !arrays.is_empty() || get(1360) != 0 {
            return Err(format!(
                "saved picture {id} needs unimplemented custom effect/mask restoration"
            ));
        }
        // Preserve an explicit boundary for state not consumed by this host;
        // these geometry/color/corner/effect fields are neutral in the audited save.
        for offset in [
            576, 580, 584, 588, 592, 596, 608, 616, 624, 632, 640, 648, 656, 664, 672, 680, 688,
            696, 704, 712, 720, 728, 736, 767, 892, 896, 900, 904, 908, 912, 984, 988, 992, 996,
            1008, 1012, 1028, 1076, 1080, 1084, 1088, 1092, 1096, 1124, 1128, 1132, 1136,
        ] {
            if get(offset) != 0 {
                return Err(format!(
                    "saved picture {id} unsupported nonneutral field +{offset:#x}"
                ));
            }
        }
        if get(1000) != 100
            || get(1004) != 100
            || !matches!(get(764), 0 | 1)
            || !matches!(get(765), 0 | 1)
        {
            return Err(format!(
                "saved picture {id} unsupported opacity effect/kind"
            ));
        }
        let path = if get(764) == 1 {
            format!("<saved-text-{id}>")
        } else {
            String::from_utf8(source.to_vec()).map_err(|_| "saved picture path is not UTF-8")?
        };
        let mut image = if get(765) != 0 || path.starts_with('<') && get(764) != 1 {
            generated_image(&path, get(776), get(780), &mut load_image)?
        } else {
            load_image(&path)?
        };
        let frames =
            if get(765) == 0 && get(764) == 0 && (get(776) > 1 || get(780) > 1 || get(1064) > 0) {
                if get(780) != 1 {
                    return Err("saved multirow picture atlas is not implemented".into());
                }
                let frames = FrameAnimation {
                    sheet: image.clone(),
                    columns: get(776),
                    base: get(488),
                    first: get(1052),
                    last: get(1056),
                    current: get(1072),
                    remaining: get(1060),
                    duration: get(1064),
                    mode: get(1068),
                };
                frames.validate()?;
                image = frames.image()?;
                Some(frames)
            } else {
                None
            };
        let packed = |value: i32| [(value >> 16) & 255, (value >> 8) & 255, value & 255];
        let duration = get(516).max(0);
        let remaining = get(512).max(0);
        validate_duration(duration)?;
        if remaining > duration || !matches!(get(504), 0..=3) || !(0..=6).contains(&get(768)) {
            return Err("invalid native saved geometry".into());
        }
        let entry = Entry {
            image,
            frames,
            text_reveal: None,
            path,
            anchor: get(768) as u32,
            blend: get(504) as u32,
            from: Geometry {
                x: get(464),
                y: get(480),
                scale: get(552),
                scale_y: get(568),
                rotation: get(524),
                opacity: get(500),
                color: [get(588), get(592), get(596)],
            },
            to: Geometry {
                x: get(456),
                y: get(472),
                scale: get(544),
                scale_y: get(560),
                rotation: get(520),
                opacity: get(496),
                color: [get(576), get(580), get(584)],
            },
            duration,
            remaining,
            deleting: get(784) != 0,
            swing: (get(972) != 0).then_some(Swing {
                x: get(956),
                y: get(960),
                count: get(972),
                leg: get(976).max(1),
                remaining: get(980),
            }),
            zoom: Zoom { from: [get(988),get(996),get(1004)], to: [get(984),get(992),get(1000)], duration: get(1008), remaining: get(1012) },
            scale_effect: ScaleEffect {
                from: [get(1104), get(1112)],
                to: [get(1100), get(1108)],
                duration: get(1116),
                remaining: get(1120).max(0),
            },
            scale_pulse: (get(1044) > 0).then_some(ScalePulse {
                amount: [get(1032), get(1036)],
                duration: get(1044),
                remaining: get(1040),
                pause: get(1048),
            }),
            offset_effect: ScaleEffect {
                from: [get(936), get(944)],
                to: [get(932), get(940)],
                duration: get(948),
                remaining: get(952).max(0),
            },
            // 998796..998854 adds interpolated +474/+478 to opacity;
            // 99745B decrements +480 and commits on completion.
            opacity_effect: ScaleEffect {
                from: [get(1144); 2],
                to: [get(1140); 2],
                duration: get(1148),
                remaining: get(1152).max(0),
            },
            color_effect: (get(1020) > 0).then_some(ColorEffect {
                channels: packed(get(1016)),
                duration: get(1020),
                phase: get(1024),
            }),
            color_flash: (get(912) > 0).then_some(ColorFlash {
                channels: packed(get(904)), duration: get(908), remaining: get(912),
            }),
            color_overlay: None,
            color_transition: ColorTransition {
                from: packed(get(920)),
                to: packed(get(916)),
                duration: get(924),
                remaining: get(928).max(0),
            },
            pending: false,
            masked: false,
            map_relative: get(766) != 0,
        };
        entry.validate()?;
        self.pictures.insert(id, entry);
        Ok(())
    }
    pub fn capture_state(&self) -> Result<Vec<u8>, String> {
        let _images = saved_image::Budget::new(256 << 20);
        let mut bytes = snapshot_bytes::BoundedBytes::new(512 << 20);
        serde_json::to_writer(&mut bytes, &(4u32, self)).map_err(|e| format!("media snapshot: {e}"))?;
        Ok(bytes.bytes)
    }
    pub fn restore_state(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 512 << 20 {
            return Err("media snapshot exceeds host budget".into());
        }
        let _images = saved_image::Budget::new(256 << 20);
        let (version, state): (u32, Self) =
            serde_json::from_slice(bytes).map_err(|e| format!("media snapshot: {e}"))?;
        if !matches!(version, 3 | 4) {
            return Err("unsupported media snapshot version".into());
        }
        if !state.screen.valid() { return Err("invalid screen effect snapshot".into()); }
        Self::new(state.width, state.height)?;
        if state.pictures.len() > 4096
            || state.queued.len() > 4096
            || state.screen_mask.as_ref().is_some_and(|mask| {
                mask.len() != state.width as usize * state.height as usize
                    || mask.iter().any(|v| !matches!(v, 0 | 255))
            })
        {
            return Err("invalid media snapshot size/mask".into());
        }
        for (&id, entry) in &state.pictures {
            validate_picture_layer(id)?;
            entry.validate()?;
            entry.validate_text()?;
        }
        validate_duration(state.transition.duration)?;
        validate_duration(state.transition.active_duration)?;
        if !matches!(state.transition.kind, -1..=2)
            || !matches!(state.transition.active_kind, -1..=2)
            || state.transition.active
                && state
                    .transition
                    .prepared
                    .as_ref()
                    .is_none_or(|image| image.dimensions() != (state.width, state.height))
            || state.transition.timer < -10
            || state.transition.timer > state.transition.active_duration
            || state.transition.draw_timer < -10
            || state.transition.draw_timer > state.transition.active_duration
        {
            return Err("invalid saved media transition".into());
        }
        if state
            .queued
            .iter()
            .any(|q| !matches!(q.opcode, 150 | 290) || q.args.len() < if q.opcode == 290 { 8 } else { 3 } || q.raw.len() != q.args.len() || q.delay < 0)
        {
            return Err("invalid saved picture queue".into());
        }
        let mut state = state;
        state.scene_image = None;
        state.render_cache = None;
        state.render_dirty = true;
        Ok(state)
    }
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_000_000 {
            return Err("native media viewport dimensions out of bounds".into());
        }
        Ok(Self {
            width,
            height,
            pictures: BTreeMap::new(),
            transition: Transition::default(),
            queued: Vec::new(),
            screen_mask: None,
            camera_offset: [0; 2],
            scene_zoom: Zoom::default(),
            screen: screen::Screen::default(),
            scene_image: None,
            render_cache: None,
            render_dirty: true,
            text_creation: None,
        })
    }

    #[cfg(test)]
    #[allow(dead_code)] // Used by integration tests importing this module, not bin unit tests.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn picture_count(&self) -> usize {
        self.pictures.len()
    }
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn transition_active(&self) -> bool {
        self.transition.active
    }
    pub fn vm_blocked(&self) -> bool {
        self.transition.active && (self.transition.gate || self.transition.black_backdrop)
    }

    /// A18560 high nibble 1 starts the configured capture transition; 2 also
    /// sets 14DEC74. A40430 then draws 964370 against the cleared frame instead
    /// of rendering the destination scene until the transition completes.
    pub(crate) fn map_transition(&mut self, flags: i32) -> Result<bool, String> {
        let mode = (flags >> 4) & 15;
        if !matches!(mode, 1 | 2) { return Ok(true); }
        if self.transition.active { return Ok(false); }
        self.transition.prepared = Some(Arc::new(self.render_scene()?));
        self.transition.active = true;
        self.transition.timer = self.transition.duration;
        self.transition.draw_timer = self.transition.duration;
        self.transition.active_duration = self.transition.duration;
        self.transition.active_kind = self.transition.kind;
        self.transition.active_mask = self.transition.mask.clone();
        self.transition.black_backdrop = mode == 2;
        Ok(true)
    }
    pub fn picture_path(&self, id: i32) -> Option<&str> {
        self.pictures
            .get(&id)
            .filter(|p| !p.pending && !p.deleted())
            .map(|p| p.path.as_str())
    }
    pub fn picture_exists(&self, id: i32) -> bool {
        self.pictures.get(&id).is_some_and(|p| !p.deleted())
    }
    pub fn picture_text_finished(&self, id: i32) -> i32 {
        self.pictures.get(&id).filter(|p| !p.pending && !p.deleted())
            .and_then(|p| p.text_reveal.as_ref()).map_or(-1, |text| i32::from(text.finished()))
    }

    pub fn execute_text(&mut self, raw: &[i32], args: &[i32], source: &str, layout: text::TextReveal) -> Result<MediaOutcome, String> {
        if !layout.valid() { return Err("invalid prepared text reveal layout".into()); }
        let image = layout.image()?;
        self.text_creation = Some(layout);
        let result = self.execute(150, raw, args, source, |_| Ok(image.clone()));
        self.text_creation = None;
        result
    }
    pub fn picture_dimensions(&self, id: i32) -> Option<(u32, u32)> {
        self.pictures.get(&id).filter(|p| !p.deleted()).map(|p| {
            if p.pending {
                (0, 0)
            } else {
                p.image.dimensions()
            }
        })
    }
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn active_paths(&self) -> Vec<(i32, String)> {
        self.pictures
            .iter()
            .filter(|(_, entry)| !entry.pending && !entry.deleted())
            .map(|(&id, entry)| (id, entry.path.clone()))
            .collect()
    }

    /// The caller resolves VM values using the instruction-specific schema and expands
    /// strings in the current Common/map context. Both slices exclude the opcode.
    /// A frame loader may supply a decoded movie frame; decoding remains outside the VM.
    pub fn execute(
        &mut self,
        opcode: u32,
        raw_args: &[i32],
        resolved_args: &[i32],
        text: &str,
        mut load_image: impl FnMut(&str) -> Result<Arc<Image>, String>,
    ) -> Result<MediaOutcome, String> {
        if raw_args.len() != resolved_args.len() {
            return Err("native media raw/resolved argument length mismatch".into());
        }
        self.render_dirty = true;
        self.render_cache = None;
        match opcode {
            151 => {
                self.screen.execute(raw_args, resolved_args)?;
                Ok(MediaOutcome::Applied)
            }
            150 => self.picture(raw_args, resolved_args, text, &mut load_image),
            290 if raw_args.first().is_some_and(|flags| flags & 15 == 3) => {
                self.picture_mask(raw_args, resolved_args, text, &mut load_image)
            }
            290 if raw_args.first() == Some(&2) && (7..=8).contains(&resolved_args.len()) => {
                validate_duration(resolved_args[1])?;
                self.scene_zoom.set([resolved_args[4],resolved_args[5],resolved_args[6].max(1)],resolved_args[1]);
                Ok(MediaOutcome::Applied)
            }
            290 => self.picture_effect(raw_args, resolved_args, text),
            160 => {
                if !(2..=3).contains(&raw_args.len()) {
                    return Err(
                        "160 requires kind, packed duration/gate, and optional duration".into(),
                    );
                }
                let kind = raw_args[0];
                if kind == -2 {
                    return Err("160 kind -2 resource reuse is not supported".into());
                }
                let duration = if raw_args.len() == 3 {
                    resolved_args[2]
                } else {
                    raw_args[1] & 0xffff
                };
                validate_duration(duration)?;
                let mask = if kind >= 0 {
                    if text.is_empty() {
                        return Err("160 mask requires resolved SystemDB type11 path".into());
                    }
                    let mask = load_image(text)?;
                    if mask.dimensions() != (self.width, self.height) {
                        return Err("160 mask dimensions must match the configured viewport in this profile".into());
                    }
                    Some(mask)
                } else {
                    None
                };
                self.transition.kind = kind;
                self.transition.duration = duration;
                self.transition.gate = raw_args[1] & 0x10000 != 0;
                self.transition.mask = mask;
                Ok(MediaOutcome::Applied)
            }
            161 => {
                if !raw_args.is_empty() {
                    return Err("161 unexpected numeric arguments".into());
                }
                if self.transition.active {
                    return Ok(MediaOutcome::Retry);
                }
                self.transition.prepared = Some(Arc::new(self.render_scene()?));
                Ok(MediaOutcome::Applied)
            }
            162 => {
                if !raw_args.is_empty() {
                    return Err("162 unexpected numeric arguments".into());
                }
                if self.transition.active {
                    return Ok(MediaOutcome::Retry);
                }
                if self.transition.prepared.is_none() {
                    self.transition.prepared = Some(Arc::new(self.render_scene()?));
                }
                self.transition.active = true;
                self.transition.black_backdrop = false;
                self.transition.timer = self.transition.duration;
                self.transition.draw_timer = self.transition.duration;
                self.transition.active_duration = self.transition.duration;
                self.transition.active_kind = self.transition.kind;
                self.transition.active_mask = self.transition.mask.clone();
                Ok(MediaOutcome::Applied)
            }
            _ => Err(format!("unsupported native media opcode {opcode}")),
        }
    }

    fn picture_mask(
        &mut self,
        raw: &[i32],
        args: &[i32],
        text: &str,
        load_image: &mut impl FnMut(&str) -> Result<Arc<Image>, String>,
    ) -> Result<MediaOutcome, String> {
        if args.len() != 7 || raw[0] & !0xff3 != 0 {
            return Err("290 unsupported screen-mask form".into());
        }
        let mode = (raw[0] >> 4) & 255;
        match mode {
            0 => {
                for picture in self.pictures.values_mut() {
                    picture.masked = false;
                }
                if let Some(mask) = &mut self.screen_mask {
                    mask.fill(0);
                }
            }
            1 | 2 => {
                let first = args[2].min(args[3]);
                let last = args[2].max(args[3]);
                if first <= last {
                    for (_, picture) in self.pictures.range_mut(first..=last) {
                        picture.masked = mode == 1;
                    }
                }
            }
            3 | 4 => {
                // 97EC3B initializes the manager before a fill request.
                self.screen_mask
                    .get_or_insert_with(|| vec![0; self.width as usize * self.height as usize])
                    .fill(if mode == 3 { 0 } else { 255 });
            }
            5 | 6 => {
                if !matches!(args[6], 0 | 1) || text.is_empty() {
                    return Err(
                        "290 screen mask requires a resource path and supported origin".into(),
                    );
                }
                let image = load_image(text)?;
                let (width, height) = image.dimensions();
                // 78AE8D verifies BGR masks, then 78AF20/40 copies the LOW byte: BLUE.
                // The verified screen-mask subset is binary; soft-mask GPU shader parity remains separate.
                if image
                    .bytes()
                    .chunks_exact(4)
                    .any(|pixel| !matches!(pixel[2], 0 | 255))
                {
                    return Err(
                        "290 nonbinary screen-mask GPU semantics are not yet verified".into(),
                    );
                }
                let x = i64::from(args[4])
                    - if args[6] == 1 {
                        i64::from(width / 2)
                    } else {
                        0
                    };
                let y = i64::from(args[5])
                    - if args[6] == 1 {
                        i64::from(height / 2)
                    } else {
                        0
                    };
                let mask = self
                    .screen_mask
                    .get_or_insert_with(|| vec![0; self.width as usize * self.height as usize]);
                for sy in 0..height {
                    for sx in 0..width {
                        let dx = x + i64::from(sx);
                        let dy = y + i64::from(sy);
                        if dx < 0
                            || dy < 0
                            || dx >= i64::from(self.width)
                            || dy >= i64::from(self.height)
                        {
                            continue;
                        }
                        let value =
                            image.bytes()[(sy as usize * width as usize + sx as usize) * 4 + 2];
                        let destination =
                            &mut mask[dy as usize * self.width as usize + dx as usize];
                        if mode == 5 {
                            *destination &= value;
                        } else {
                            *destination |= value;
                        }
                    }
                }
            }
            _ => return Err(format!("290 unsupported screen-mask mode {mode}")),
        }
        Ok(MediaOutcome::Applied)
    }

    fn picture_effect(
        &mut self,
        raw: &[i32],
        args: &[i32],
        text: &str,
    ) -> Result<MediaOutcome, String> {
        // A2BEE0 -> 9A8400 -> 9A3360. Validate the supported effect family
        // before changing entries or installing their delayed operations.
        if !(7..=8).contains(&args.len())
            || !matches!(
                raw[0],
                0 | 0x10 | 0x20 | 0x30 | 0x40 | 0x50 | 0x70 | 0x80 | 0x90 | 0xa0 | 0xc0 | 0xe0 | 0x110 | 0x170
            )
            || !text.is_empty()
        {
            return Err(
                "290 requires a verified immediate effect type and seven/eight operands".into(),
            );
        }
        let delay = args.get(7).copied().unwrap_or(0).max(0);
        validate_duration(delay)?;
        validate_duration(args[1])?;
        if delay > 0 {
            let ids: Vec<_> = self.pictures.range(args[2].min(args[3])..=args[2].max(args[3])).map(|(&id, _)| id).collect();
            if self.queued.len() + ids.len() > 4096 { return Err("native picture queue limit".into()); }
            for id in ids {
                let mut resolved = args.to_vec();
                resolved[2] = id; resolved[3] = id; resolved[7] = 0;
                self.queued.push(QueuedPicture { opcode: 290, raw: resolved.clone(), args: resolved, text: String::new(), image: None, text_reveal: None, delay });
            }
            return Ok(MediaOutcome::Applied);
        }
        if raw[0] == 0 {
            for (_, entry) in self.pictures.range_mut(args[2].min(args[3])..=args[2].max(args[3])) {
                entry.color_overlay = Some(ColorOverlay {
                    channels: [args[4], args[5], args[6]].map(|v| (v / 2 + 100).clamp(0, 200)),
                    duration: args[1],
                    remaining: args[1],
                });
            }
            return Ok(MediaOutcome::Applied);
        }
        if raw[0] == 0x40 {
            for (_, entry) in self.pictures.range_mut(args[2].min(args[3])..=args[2].max(args[3])) {
                entry.zoom.set([args[4], args[5], args[6]], args[1]);
            }
            return Ok(MediaOutcome::Applied);
        }
        if raw[0] == 0x170 {
            // The filter mutates the current texture; later filters accumulate.
            // Build all outputs before replacing any selected picture.
            let filtered: Result<Vec<_>, String> = self.pictures
                .range(args[2].min(args[3])..=args[2].max(args[3]))
                .map(|(&id, entry)| Ok((id, Arc::new(filter::blur(&entry.image, args[4])?))))
                .collect();
            for (id, image) in filtered? { self.pictures.get_mut(&id).unwrap().image = image; }
            return Ok(MediaOutcome::Applied);
        }
        if matches!(raw[0], 0xe0 | 0x110) {
            for (_, entry) in self.pictures.range_mut(args[2].min(args[3])..=args[2].max(args[3])) {
                if raw[0] == 0xe0 {
                    entry.opacity_effect.set([args[4]; 2], args[1]);
                } else {
                    entry.color_flash = Some(ColorFlash { channels: [args[4], args[5], args[6]].map(|v| (v / 2 + 100).clamp(0, 200)), duration: args[1], remaining: args[1] });
                }
            }
            return Ok(MediaOutcome::Applied);
        }
        if raw[0] == 0x70 {
            validate_duration(args[1])?;
            validate_duration(args[6])?;
            if args[4..6].iter().any(|v| v.unsigned_abs() > 1600) {
                return Err("290 scale pulse exceeds host scale limit".into());
            }
            let first = args[2].min(args[3]);
            let last = args[2].max(args[3]);
            if first <= last {
                for (_, entry) in self.pictures.range_mut(first..=last) {
                    entry.scale_pulse = Some(ScalePulse {
                        amount: [args[4], args[5]],
                        duration: args[1],
                        remaining: 2 * args[1],
                        pause: args[6],
                    });
                }
            }
            return Ok(MediaOutcome::Applied);
        }
        if matches!(raw[0], 0x10 | 0x20 | 0x50 | 0xc0) {
            validate_duration(args[1])?;
            if raw[0] == 0xc0
                && (args[6] != 0 || args[4..6].iter().any(|v| !(1..=1600).contains(v)))
            {
                return Err("290 scale effect requires bounded positive percentages and neutral third parameter".into());
            }
            if raw[0] == 0x20
                && args[4..6].iter().any(|v| v.unsigned_abs() > 32768)
            {
                return Err("290 offset effect exceeds supported displacement".into());
            }
            if matches!(raw[0], 0x10 | 0x50) && args[4..7].iter().any(|v| !(-100..=100).contains(v))
            {
                return Err("290 color pulse exceeds verified channel range".into());
            }
            let first = args[2].min(args[3]);
            let last = args[2].max(args[3]);
            if last >= first {
                for (_, entry) in self.pictures.range_mut(first..=last) {
                    match raw[0] {
                        0x10 => entry
                            .color_transition
                            .set([args[4], args[5], args[6]], args[1]),
                        0xc0 => entry.scale_effect.set([args[4], args[5]], args[1]),
                        // 9A3AAD..9A3B38 reads only args4/5 and duration.
                        // The generic third effect operand is not consumed.
                        0x20 => entry.offset_effect.set([args[4], args[5]], args[1]),
                        _ => {
                            entry.color_effect = Some(ColorEffect {
                                channels: [args[4], args[5], args[6]]
                                    .map(|v| (v / 2 + 100).clamp(0, 200)),
                                duration: args[1],
                                phase: 0,
                            })
                        }
                    }
                }
            }
            return Ok(MediaOutcome::Applied);
        }
        if matches!(raw[0], 0x80 | 0x90 | 0xa0) {
            // 9A3DAA..9A3DD1: first/current=args4, last=args5,
            // duration/remaining=args1 and mode=type-8 (once/loop/pingpong).
            validate_duration(args[1])?;
            if args[6] != 0 {
                return Err("290 frame animation requires neutral third parameter".into());
            }
            let first = args[2].min(args[3]);
            let last = args[2].max(args[3]);
            if first <= last {
                for (_, entry) in self.pictures.range_mut(first..=last) {
                    if let Some(frames) = &mut entry.frames {
                        if args[1] > 0
                            && (!(1..=frames.columns).contains(&args[4])
                                || !(1..=frames.columns).contains(&args[5]))
                        {
                            return Err("290 frame animation range exceeds horizontal atlas".into());
                        }
                        frames.duration = args[1];
                        frames.remaining = args[1];
                        frames.first = args[4];
                        frames.last = args[5];
                        frames.current = args[4];
                        frames.mode = (raw[0] >> 4) - 8;
                        entry.image = frames.image()?;
                    } else if args[1] > 0 {
                        return Err("290 nonzero frame animation requires divided picture".into());
                    }
                }
            }
            return Ok(MediaOutcome::Applied);
        }
        // Resource bounds are host policy, not native operand limits. This player
        // uses global timing divisor 1; native 9A36F0 divides durations >=2 by it.
        let leg = args[1].max(1);
        if leg > 1_000_000
            || !(-32768..=32768).contains(&args[4])
            || !(-32768..=32768).contains(&args[5])
        {
            return Err("290 swing exceeds host duration/displacement limits".into());
        }
        let first = args[2].min(args[3]);
        let last = args[2].max(args[3]);
        if last >= first {
            for (_, entry) in self.pictures.range_mut(first..=last) {
                entry.swing = Some(Swing {
                    x: args[4],
                    y: args[5],
                    count: args[6],
                    leg,
                    remaining: leg * 2,
                });
            }
        }
        Ok(MediaOutcome::Applied)
    }

    fn picture(
        &mut self,
        raw: &[i32],
        args: &[i32],
        text: &str,
        load_image: &mut impl FnMut(&str) -> Result<Arc<Image>, String>,
    ) -> Result<MediaOutcome, String> {
        let flags = *raw.first().ok_or("150 missing flags")? as u32;
        let operation = flags & 15;
        if operation == 3 {
            // A1AD7A..A1AECA clears both command/string queue vectors of
            // existing pictures in the inclusive normalized ID range.
            let range = flags & 0x0100_0000 != 0;
            if args.len() != if range { 3 } else { 2 }
                || !text.is_empty()
            {
                return Err(format!("150 unsupported queue-clear form {flags:#x}"));
            }
            let first = args[1];
            let last = if range { args[2] } else { first };
            self.queued
                .retain(|command| !(first.min(last)..=first.max(last)).contains(&command.picture_id()));
            return Ok(MediaOutcome::Applied);
        }
        if args.len() < 3 {
            return Err("150 requires flags, picture ID and duration".into());
        }
        let duration = args[2];
        validate_duration(duration)?;
        if operation == 2 {
            return self.delete_picture(flags, args, text);
        }
        let id = args[1];
        validate_picture_layer(id)?;
        let allowed = if operation == 1 {
            0x0171_ff31
        } else {
            0x0271_ff30
        };
        if operation > 1 || flags & !allowed != 0 {
            return Err(format!(
                "150 unsupported operation/content/blend/range flags {flags:#x}"
            ));
        }
        let variable_path = flags & 0x10 != 0;
        if operation == 0 && !matches!((flags >> 4) & 15, 0..=3) {
            return Err("150 text pictures require the rich-text renderer".into());
        }
        let generated = operation == 0 && (flags >> 4) & 15 == 3;
        let text_picture = operation == 0 && (flags >> 4) & 15 == 2;
        let screenshot = operation == 0 && !generated && !text_picture && text == "<SCREENSHOT>";
        let blend = (flags >> 8) & 15;
        if !(matches!(blend, 0..=3 | 15)) {
            return Err(format!("150 unsupported blend {blend}"));
        }
        if args.len() != 11
            && args.len() != 15
            && args.len() != 18
            && args.len() != 19
            && !(variable_path && args.len() == 12)
        {
            return Err(format!(
                "150 ordinary picture requires 11/18 args or variable-path 12 args, got {}",
                args.len()
            ));
        }
        let range = flags & 0x0100_0000 != 0;
        let delay = if args.len() >= 15 {
            let neutral = (matches!(args.len(), 18 | 19)
                && raw[12] == 0x0200_0000
                )
                || (args.len() == 15 && raw[12] == 0x0100_0000);
            if !neutral || (!variable_path && args[11] != 0) || (!range && args[14] != 0) {
                return Err(
                    "150 unsupported extended picture layout".into(),
                );
            }
            validate_duration(args[13])?;
            args[13]
        } else {
            0
        };
        let anchor = (flags >> 12) & 15;
        if anchor > 6 {
            return Err(format!("150 unsupported anchor {anchor}"));
        }
        if operation == 0
            && !generated
            && !text_picture
            && (!(1..=4096).contains(&args[3]) || args[4] != 1 || !(1..=args[3]).contains(&args[5]))
        {
            return Err("150 divided picture requires a valid horizontal strip and cell".into());
        }
        // A19027..A190B7: mode0 uniform, mode1 Y, mode2 X,
        // mode3 X=args9/Y=args18, mode4 preserves both axes.
        let scale_mode = (flags >> 20) & 15;
        let (scale_x, scale_y) = match scale_mode {
            0 => (args[9], args[9]),
            1 => (100, args[9]),
            2 => (args[9], 100),
            3 if args.len() == 19 => (args[9], args[18]),
            4 => (-1_000_000, -1_000_000),
            _ => return Err("150 unsupported scale selector/operand count".into()),
        };
        // 98C192/98C1AC skip each sentinel store on replacement creates too.
        let scale_x = if operation == 0 && scale_x == -1_000_000 {
            self.pictures.get(&id).map_or(100, |p| p.to.scale)
        } else { scale_x };
        let scale_y = if operation == 0 && scale_y == -1_000_000 {
            self.pictures.get(&id).map_or(100, |p| p.to.scale_y)
        } else { scale_y };
        let valid_scale = |s: i32| (-1600..=1600).contains(&s) || operation == 1 && s == -1_000_000;
        if !((0..=255).contains(&args[6]) || args[6] == -1_000_000)
            || !valid_scale(scale_x)
            || !valid_scale(scale_y)
            || !(args[10].unsigned_abs() <= 36_000 || args[10] == -1_000_000)
        {
            return Err("150 opacity, scale or rotation exceeds supported bounds".into());
        }
        if args[7].unsigned_abs() > 1_000_000 || args[8].unsigned_abs() > 1_000_000 {
            return Err("150 coordinates exceed supported bounds".into());
        }
        if range {
            if operation != 1 || args.len() < 15 {
                return Err("native picture range requires extended move".into());
            }
            let end = args[14];
            let ids: Vec<_> = self
                .pictures
                .range(id.min(end)..=id.max(end))
                .map(|(&id, _)| id)
                .collect();
            let mut one_raw = raw.to_vec();
            let mut one_args = args.to_vec();
            one_raw[0] &= !0x0100_0000;
            one_args[0] &= !0x0100_0000;
            one_raw[14] = 0;
            one_args[14] = 0;
            for id in ids {
                one_args[1] = id;
                self.picture(&one_raw, &one_args, text, load_image)?;
            }
            return Ok(MediaOutcome::Applied);
        }
        // A1AA79..A1AC84 searches the existing picture vector even for a
        // single-ID move. An absent ID has no receiver, including no queue to
        // receive a delayed move; a later create of the same ID is independent.
        if operation == 1 && !self.pictures.contains_key(&id) {
            return Ok(MediaOutcome::Applied);
        }
        if delay > 0 {
            if self.queued.len() >= 4096 {
                return Err("native queued picture limit".into());
            }
            if operation == 0 && self.pictures.len() >= 4096 && !self.pictures.contains_key(&id) {
                return Err("native picture count limit".into());
            }
            let image = if operation == 0 {
                if !text_picture
                    && (text.is_empty()
                                || (!generated && !screenshot && text.starts_with('<')))
                {
                    return Err("150 requires a resolved ordinary resource path".into());
                }
                let image = if screenshot {
                    Arc::new(self.render_scene()?)
                } else if generated {
                    generated_image(text, args[3], args[4], load_image)?
                } else {
                    load_image(text)?
                };
                let (w, h) = image.dimensions();
                if u64::from(w) * u64::from(scale_x.unsigned_abs()) / 100 > 32768
                    || u64::from(h) * u64::from(scale_y.unsigned_abs()) / 100 > 32768
                {
                    return Err("150 scaled picture exceeds renderer limits".into());
                }
                Some(image)
            } else {
                None
            };
            if let Some(image) = image.as_ref().filter(|_| !self.pictures.contains_key(&id)) {
                // 9A6CC2 inserts the object before 98A34A queues creation.
                // Its dimensions are zero, query9 is true, and effects already tick.
                let empty = Geometry {
                    x: 0,
                    y: 0,
                    scale: 100,
                    scale_y: 100,
                    rotation: 0,
                    opacity: 0,
                    color: [0; 3],
                };
                let frames = if !generated && !text_picture && args[3] > 1 {
                    let frames = FrameAnimation {
                        sheet: image.clone(),
                        columns: args[3],
                        base: args[5],
                        first: 1,
                        last: 1,
                        current: args[5],
                        remaining: 0,
                        duration: 0,
                        mode: 0,
                    };
                    frames.validate()?;
                    Some(frames)
                } else {
                    None
                };
                self.pictures.insert(
                    id,
                    Entry {
                        image: image.clone(),
                        frames,
                        text_reveal: None,
                        path: text.to_owned(),
                        anchor: 0,
                        blend: 0,
                        from: empty,
                        to: empty,
                        duration: 0,
                        remaining: 0,
                        deleting: false,
                        swing: None,
                        scale_effect: ScaleEffect::default(),
                        zoom: Zoom::default(),
                        scale_pulse: None,
                        offset_effect: ScaleEffect {
                            from: [0; 2],
                            to: [0; 2],
                            duration: 0,
                            remaining: 0,
                        },
                        color_effect: None,
                        color_flash: None,
                        color_overlay: None,
                        opacity_effect: ScaleEffect {
                            from: [0; 2],
                            to: [0; 2],
                            duration: 0,
                            remaining: 0,
                        },
                        color_transition: ColorTransition::default(),
                        pending: true,
                        masked: false,
                        map_relative: false,
                    },
                );
            }
            let mut queued_raw = raw.to_vec();
            let mut queued_args = args.to_vec();
            queued_raw[13] = 0;
            queued_args[13] = 0;
            self.queued.push(QueuedPicture {
                opcode: 150,
                raw: queued_raw,
                args: queued_args,
                text: text.to_owned(),
                image,
                delay,
                text_reveal: self.text_creation.clone(),
            });
            return Ok(MediaOutcome::Applied);
        }
        let mut to = Geometry {
            x: args[7],
            y: args[8],
            scale: scale_x,
            scale_y,
            rotation: args[10],
            opacity: args[6],
            // 98C227..98C28B / 98FFFC..99005A: base RGB offsets.
            color: if args.len() >= 18 { std::array::from_fn(|i| (f64::from(args[15+i].clamp(0,200) - 100) * 2.55) as i32) } else { [0; 3] },
        };
        if operation == 0 {
            let previous = self.pictures.get(&id);
            if to.opacity == -1_000_000 { to.opacity = previous.map_or(255, |p| p.to.opacity); }
            if to.rotation == -1_000_000 { to.rotation = previous.map_or(0, |p| p.to.rotation); }
        }
        if operation == 1 {
            let entry = self
                .pictures
                .get_mut(&id)
                .ok_or_else(|| format!("150 move of absent picture {id}"))?;
            if entry.deleted() {
                return Err(format!(
                    "150 move of deleted picture {id} requires native revival evidence"
                ));
            }
            entry.from = entry.current();
            // 98FF65..98FF6F: the opacity sentinel preserves the target,
            // including when an earlier interpolation is still in progress.
            if to.opacity == -1_000_000 {
                to.opacity = entry.to.opacity;
            }
            // 98FF7F..98FFBE: either scale sentinel preserves the pair;
            // the rotation sentinel independently preserves its target.
            if to.scale == -1_000_000 || to.scale_y == -1_000_000 {
                to.scale = entry.to.scale;
                to.scale_y = entry.to.scale_y;
            }
            if to.rotation == -1_000_000 {
                to.rotation = entry.to.rotation;
            }
            // Move preserves the picture's origin; bits12..15 only select create origin.
            if flags & 0x10000 != 0 {
                to.x = entry
                    .from
                    .x
                    .checked_add(to.x)
                    .ok_or("relative picture X overflow")?;
                to.y = entry
                    .from
                    .y
                    .checked_add(to.y)
                    .ok_or("relative picture Y overflow")?;
            }
            entry.to = to;
            if blend != 15 {
                entry.blend = blend;
            }
            entry.duration = duration;
            entry.remaining = duration;
            entry.deleting = false;
            return Ok(MediaOutcome::Applied);
        }
        if !text_picture
            && (text.is_empty()
                || (!generated && !screenshot && text.starts_with('<')))
        {
            return Err("150 requires a resolved ordinary resource path".into());
        }
        if self.pictures.len() >= 4096 && !self.pictures.contains_key(&id) {
            return Err("native picture count limit".into());
        }
        // 98B410..98B47A captures the current framebuffer through 9625F0(2),
        // then 98550C resolves <SCREENSHOT> from core+0xC10 instead of a file.
        let mut image = if screenshot {
            Arc::new(self.render_scene()?)
        } else if generated {
            generated_image(text, args[3], args[4], load_image)?
        } else {
            load_image(text)?
        };
        // 986F0B..986FE9 builds row-major cells; 99F515..99F548
        // selects the one-based base frame when no animation is active.
        let frames = if !generated && !text_picture && args[3] > 1 {
            let mut frames = FrameAnimation {
                sheet: image.clone(),
                columns: args[3],
                base: args[5],
                first: 1,
                last: 1,
                current: args[5],
                remaining: 0,
                duration: 0,
                mode: 0,
            };
            frames.validate()?;
            // A1AD7A/98A2F0 attach later picture operations to the existing
            // object. Keep an animation applied to its delayed-create
            // placeholder when the queued create supplies the real atlas.
            if let Some(previous) = self.pictures.get(&id).filter(|p| p.pending) {
                if let Some(pending) = previous.frames.as_ref().filter(|f| f.duration > 0) {
                    frames.first = pending.first;
                    frames.last = pending.last;
                    frames.current = pending.current;
                    frames.remaining = pending.remaining;
                    frames.duration = pending.duration;
                    frames.mode = pending.mode;
                }
            }
            frames.validate()?;
            image = frames.image()?;
            Some(frames)
        } else {
            None
        };
        let (w, h) = image.dimensions();
        if u64::from(w) * u64::from(scale_x.unsigned_abs()) / 100 > 32768
            || u64::from(h) * u64::from(scale_y.unsigned_abs()) / 100 > 32768
        {
            return Err("150 scaled picture exceeds renderer limits".into());
        }
        // 98C2F2 initializes new ordinary-image opacity to zero; native render
        // interpolates it toward +1F0 using the command duration at +200/+204.
        let previous = self.pictures.get(&id);
        // 98C184 skips blend replacement for sentinel15, including creates.
        let blend = if blend == 15 { previous.map_or(0, |p| p.blend) } else { blend };
        let map_relative = flags & 0x0200_0000 != 0;
        let relative = flags & 0x10000 != 0;
        let was_map_relative = previous.is_some_and(|p| p.map_relative);
        // A19AA7 encodes bit25 into blend bit8. 98AC20..98ACE5 then
        // converts screen input to map coordinates, except for relative
        // replacements already expressed in the same coordinate space.
        let camera_sign = if map_relative && !(relative && was_map_relative) {
            1_i64
        } else if !map_relative && was_map_relative && relative {
            -1
        } else {
            0
        };
        to.x = i32::try_from(i64::from(to.x) + camera_sign * i64::from(self.camera_offset[0]))
            .map_err(|_| "map-relative create X overflow")?;
        to.y = i32::try_from(i64::from(to.y) + camera_sign * i64::from(self.camera_offset[1]))
            .map_err(|_| "map-relative create Y overflow")?;
        let from = if let Some(previous) = previous.filter(|p| !p.pending) {
            previous.current()
        } else {
            Geometry { opacity: 0, ..to }
        };
        // 98BAA5..98BB06 adds the snapshotted current position for relative creates.
        if flags & 0x10000 != 0 {
            if let Some(previous) = previous {
                let current = previous.current();
                to.x = current
                    .x
                    .checked_add(to.x)
                    .ok_or("relative picture X overflow")?;
                to.y = current
                    .y
                    .checked_add(to.y)
                    .ok_or("relative picture Y overflow")?;
            }
        }
        // 98A962 -> 9A4AD0 only releases/restores texture backup fields;
        // creation preserves the owning object's effects and command queues.
        let swing = previous.and_then(|p| p.swing);
        let scale_effect = previous.map_or_else(ScaleEffect::default, |p| p.scale_effect);
        let scale_pulse = previous.and_then(|p| p.scale_pulse);
        let offset_effect = previous.map_or(
            ScaleEffect {
                from: [0; 2],
                to: [0; 2],
                duration: 0,
                remaining: 0,
            },
            |p| p.offset_effect,
        );
        let color_effect = previous.and_then(|p| p.color_effect);
        let color_flash = previous.and_then(|p| p.color_flash);
        let color_overlay = previous.and_then(|p| p.color_overlay);
        let opacity_effect = previous.map_or(
            ScaleEffect {
                from: [0; 2],
                to: [0; 2],
                duration: 0,
                remaining: 0,
            },
            |p| p.opacity_effect,
        );
        let color_transition = previous.map(|p| p.color_transition).unwrap_or_default();
        let masked = previous.is_some_and(|p| p.masked);
        self.pictures.insert(
            id,
            Entry {
                image,
                frames,
                text_reveal: if text_picture { self.text_creation.clone() } else { None },
                path: text.to_owned(),
                anchor,
                blend,
                from,
                to,
                duration,
                remaining: duration,
                deleting: false,
                swing,
                scale_effect,
                zoom: previous.map_or_else(Zoom::default, |p| p.zoom),                scale_pulse,
                offset_effect,
                opacity_effect,
                color_effect,
                color_flash,
                color_overlay,
                color_transition,
                pending: false,
                masked,
                map_relative,
            },
        );
        Ok(MediaOutcome::Applied)
    }

    /// A1AC84 -> 996470: duration and delay are independent; an inclusive
    /// range selects existing objects now, then appends to each object's queue.
    fn delete_picture(
        &mut self,
        flags: u32,
        args: &[i32],
        text: &str,
    ) -> Result<MediaOutcome, String> {
        let range = flags & 0x0100_0000 != 0;
        // A1AC84 checks only the operation nibble: retained creation variant,
        // origin and blend bits do not change the delete consumer.
        if flags & !0x0301_ff32 != 0
            || (!range && !matches!(args.len(), 3 | 4))
            || (range && (args.len() != 6 || args[4] != 0))
            || !text.is_empty()
        {
            return Err(format!("150 unsupported delete form {flags:#x}: {args:?}"));
        }
        let delay = args.get(3).copied().unwrap_or(0).max(0);
        validate_duration(delay)?;
        let end = if range { args[5] } else { args[1] };
        let first = args[1].min(end);
        let last = args[1].max(end);
        if last < first {
            return Ok(MediaOutcome::Applied);
        }
        let ids: Vec<_> = self
            .pictures
            .range(first..=last)
            .map(|(&id, _)| id)
            .collect();
        if delay > 0 && self.queued.len() + ids.len() > 4096 {
            return Err("native queued picture limit".into());
        }
        for id in ids {
            if delay > 0 {
                let command = vec![2, id, args[2]];
                self.queued.push(QueuedPicture {
                    opcode: 150,
                    raw: command.clone(),
                    args: command,
                    text: String::new(),
                    image: None,
                    text_reveal: None,
                    delay,
                });
            } else {
                let entry = self
                    .pictures
                    .get_mut(&id)
                    .expect("selected existing picture");
                // 996790: an already deleting object does not restart its timer.
                if !entry.deleting {
                    entry.from = entry.current();
                    entry.to = Geometry {
                        opacity: 0,
                    color: [0; 3],
                        ..entry.from
                    };
                    entry.duration = args[2];
                    entry.remaining = args[2];
                    entry.deleting = true;
                    if entry.deleted() {
                        entry.swing = None;
                    }
                }
            }
        }
        // 9A8E33: completed objects survive while their own queue is nonempty.
        self.collect_deleted();
        Ok(MediaOutcome::Applied)
    }

    fn collect_deleted(&mut self) {
        self.pictures
            .retain(|id, p| !p.deleted() || self.queued.iter().any(|q| q.picture_id() == *id));
    }

    /// Supply a decoded frame without reissuing the script command or resetting its tween.
    pub fn replace_frame(&mut self, id: i32, image: Arc<Image>) -> Result<(), String> {
        let entry = self
            .pictures
            .get_mut(&id)
            .ok_or("frame for absent picture")?;
        if image.dimensions() != entry.image.dimensions() {
            return Err("movie frame dimensions changed".into());
        }
        if Arc::ptr_eq(&entry.image, &image) {
            return Ok(());
        }
        entry.image = image;
        self.render_dirty = true;
        self.render_cache = None;
        Ok(())
    }

    /// Host logical step, not wall-clock milliseconds. Continue ticking during VM waits.
    pub fn tick(&mut self, step: i32) -> Result<(), String> {
        if !(0..=1_000_000).contains(&step) {
            return Err("invalid media host step".into());
        }
        let mut render_changed = self.screen.tick(step);
        // 996B80: test delay before subtracting. A delay reaching zero fires next tick.
        // Retain every not-yet-executed command in the owning queue, including
        // commands ready in this batch. Native collection must still see them.
        for command in &mut self.queued {
            command.delay = if command.delay > 0 {
                (command.delay - step).max(0)
            } else {
                -1
            };
        }
        while let Some(index) = self.queued.iter().position(|command| command.delay < 0) {
            let command = self.queued.remove(index);
            // A queued operation may capture the scene or change a static
            // effect. Invalidate before execution, just as execute() does.
            self.render_dirty = true;
            self.render_cache = None;
            if command.opcode == 290 {
                self.picture_effect(&command.raw, &command.args, &command.text)?;
                continue;
            }
            let image = command.image;
            self.text_creation = command.text_reveal;
            let result = self.picture(&command.raw, &command.args, &command.text, &mut |_| {
                image
                    .clone()
                    .ok_or("queued move unexpectedly loaded an image".into())
            });
            self.text_creation = None;
            result?;
            render_changed = true;
        }
        for picture in self.pictures.values_mut() {
            let was_dynamic = picture.render_dynamic();
            if !picture.pending {
                if let Some(text) = &mut picture.text_reveal {
                    if let Some(image) = text.tick(step)? {
                        picture.image = image;
                        render_changed = true;
                    }
                }
            }
            if let Some(frames) = &mut picture.frames {
                if let Some(image) = frames.tick()? {
                    picture.image = image;
                    render_changed = true;
                }
            }
            picture.remaining = picture.remaining.saturating_sub(step).max(0);
            if let Some(swing) = &mut picture.swing {
                swing.tick();
            }
            picture.scale_effect.tick();
            if let Some(pulse) = &mut picture.scale_pulse {
                pulse.tick();
            }
            picture.offset_effect.tick();
            picture.opacity_effect.tick();
            picture.zoom.remaining = (picture.zoom.remaining - 1).max(0);
            picture.color_transition.remaining = (picture.color_transition.remaining - 1).max(0);
            if let Some(color) = &mut picture.color_effect {
                color.tick();
            }
            if let Some(flash) = &mut picture.color_flash { flash.remaining = (flash.remaining - 1).max(0); }
            if let Some(overlay) = &mut picture.color_overlay { overlay.remaining = (overlay.remaining - 1).max(0); }
            render_changed |= was_dynamic || picture.render_dynamic();
        }
        let scene_zoom_was_dynamic = self.scene_zoom.remaining > 0;
        self.scene_zoom.remaining = (self.scene_zoom.remaining - 1).max(0);
        render_changed |= scene_zoom_was_dynamic || self.scene_zoom.remaining > 0;
        self.collect_deleted();
        if self.transition.active {
            // The captured overlay is applied by render(), after the cached
            // scene. Advancing it does not invalidate the underlying pixels.
            self.transition.timer = self.transition.timer.saturating_sub(step).max(-10);
            // 9652B0 uses the newly updated main timer, not old draw_timer.
            self.transition.draw_timer = self.transition.timer.saturating_sub(step).max(-10);
        }
        if render_changed {
            self.render_dirty = true;
            self.render_cache = None;
        }
        Ok(())
    }

    fn draw_map(&self, target: &mut Image, scene: &Image) {
        let zoom = self.scene_zoom.current();
        let factor = f64::from(zoom[2]) / 100.0;
        blend_picture(target, scene,
            (f64::from(zoom[0]) * (1.0-factor)) as i32,
            (f64::from(zoom[1]) * (1.0-factor)) as i32,
            (f64::from(self.width)*factor).max(1.0) as u32,
            (f64::from(self.height)*factor).max(1.0) as u32,
            255,0,[0;3],None);
    }

    pub fn render_scene(&self) -> Result<Image, String> {
        if !self.render_dirty {
            if let Some(image) = &self.render_cache {
                return Image::new(
                    image.dimensions().0 as usize,
                    image.dimensions().1 as usize,
                    image.bytes().to_vec(),
                );
            }
        }
        let mut rgba = vec![0; self.width as usize * self.height as usize * 4];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        let mut target = Image::new(self.width as usize, self.height as usize, rgba)?;
        let mut scene_pending = self.scene_image.as_ref();
        let mut tone_pending = true;
        for (&id, entry) in &self.pictures {
            // A3E570 draws buckets6/5 before map layers; bucket0 follows them
            // at A412C5. Interleaved map buckets remain explicitly unsupported.
            if id >= 0 {
                if let Some(scene) = scene_pending.take() {
                    self.draw_map(&mut target, scene);
                }
            }
            if id >= 100_000 && tone_pending {
                self.screen.tone(&mut target);
                tone_pending = false;
            }
            if entry.pending || entry.deleted() {
                continue;
            }
            let mut current = entry.current();
            current.opacity = (current.opacity as f32 + entry.opacity_effect.current()[0]) as i32;
            // Native picture output is an opaque framebuffer. A fully invisible
            // layer contributes no color, including the additive tint pass.
            if current.opacity <= 0 {
                continue;
            }
            if entry.map_relative {
                current.x = current
                    .x
                    .checked_sub(self.camera_offset[0])
                    .ok_or("map-relative picture X overflow")?;
                current.y = current
                    .y
                    .checked_sub(self.camera_offset[1])
                    .ok_or("map-relative picture Y overflow")?;
            }
            let screen_mask = if entry.masked {
                self.screen_mask.as_deref()
            } else {
                None
            };
            let (w, h) = entry.image.dimensions();
            let effect = entry.scale_effect.current();
            let interpolate_scale = |from, to| {
                if entry.duration > 0 && entry.remaining > 0 {
                    f64::from(to)
                        + (f64::from(from) - f64::from(to)) * f64::from(entry.remaining)
                            / f64::from(entry.duration)
                } else {
                    f64::from(to)
                }
            };
            let scale_x = interpolate_scale(entry.from.scale, entry.to.scale);
            let scale_y = interpolate_scale(entry.from.scale_y, entry.to.scale_y);
            let pulse = entry
                .scale_pulse
                .map(ScalePulse::current)
                .unwrap_or([0.0; 2]);
            let mut signed_size = [
                f64::from(w) * (scale_x * f64::from(effect[0]) / 100.0 + f64::from(pulse[0]))
                    / 100.0,
                f64::from(h) * (scale_y * f64::from(effect[1]) / 100.0 + f64::from(pulse[1]))
                    / 100.0,
            ];
            let zoom = entry.zoom.current();
            let factor = f64::from(zoom[2]) / 100.0;
            current.x = (f64::from(zoom[0]) + (f64::from(current.x) - f64::from(zoom[0])) * factor) as i32;
            current.y = (f64::from(zoom[1]) + (f64::from(current.y) - f64::from(zoom[1])) * factor) as i32;
            signed_size = signed_size.map(|s| s * factor);
            if signed_size.iter().any(|v| v.abs() > 32768.0) {
                return Err("effect-scaled picture exceeds renderer limits".into());
            }
            let (sx, sy) = entry.swing.map(Swing::offset).unwrap_or((0, 0));
            let offset = entry.offset_effect.current();
            let tint = entry
                .color_effect
                .map(ColorEffect::current)
                .unwrap_or([0; 3]);
            let overlay = entry.color_overlay.map(ColorOverlay::current).unwrap_or([0; 3]);
            let tint = std::array::from_fn(|i| current.color[i] + tint[i] + overlay[i]);
            let tint = entry.color_flash.map_or(tint, |flash| flash.apply(tint));
            let transition_tint = entry.color_transition.current();
            let tint = std::array::from_fn(|i| tint[i] + transition_tint[i]);
            if signed_size[0] <= 0.0 || signed_size[1] <= 0.0 || current.rotation != 0 {
                transformed_picture(
                    &mut target,
                    &entry.image,
                    [
                        f64::from(current.x) + f64::from(sx) + f64::from(offset[0]),
                        f64::from(current.y) + f64::from(sy) + f64::from(offset[1]),
                    ],
                    signed_size,
                    entry.anchor,
                    current.rotation,
                    current.opacity.clamp(0, 255) as u32,
                    entry.blend,
                    tint,
                    screen_mask,
                );
                continue;
            }
            let width = signed_size[0].max(1.0) as u32;
            let height = signed_size[1].max(1.0) as u32;
            if width > 32768 || height > 32768 {
                return Err("effect-scaled picture exceeds renderer limits".into());
            }
            let (ax, ay) = anchor_offset(entry.anchor, width, height);
            let (sx, sy) = entry.swing.map(Swing::offset).unwrap_or((0, 0));
            let offset = entry.offset_effect.current();
            let x = current
                .x
                .checked_sub(ax)
                .and_then(|x| x.checked_add(sx))
                .ok_or("picture effect X exceeds host coordinate range")?;
            let y = current
                .y
                .checked_sub(ay)
                .and_then(|y| y.checked_add(sy))
                .ok_or("picture effect Y exceeds host coordinate range")?;
            let x = (f64::from(x) + f64::from(offset[0])) as i32;
            let y = (f64::from(y) + f64::from(offset[1])) as i32;
            // This framebuffer starts opaque and every native pass preserves
            // that alpha. Its ordinary source-over is exactly the mode-zero
            // byte-space kernel, including opacity rounding. Use the same
            // vectorized path for plain pictures as for lighting layers.
            blend_picture(
                &mut target, &entry.image, x, y, width, height,
                current.opacity.clamp(0, 255) as u32,
                entry.blend, tint, screen_mask,
            );
        }
        if let Some(scene) = scene_pending {
            self.draw_map(&mut target, scene);
        }
        if tone_pending { self.screen.tone(&mut target); }
        self.screen.flash(&mut target);
        Ok(target)
    }

    /// Draw-stage completion clears the transition gate, as in 964370. Captured
    /// pixels remain immutable while ordinary pictures continue being updated.
    pub fn render(&mut self) -> Result<Image, String> {
        let mut target = self.render_scene()?;
        if self.render_cache.is_none() || self.render_dirty {
            self.render_cache = Some(Arc::new(Image::new(
                target.dimensions().0 as usize,
                target.dimensions().1 as usize,
                target.bytes().to_vec(),
            )?));
            self.render_dirty = false;
        }
        if self.transition.active {
            if self.transition.draw_timer < 0 || self.transition.active_duration <= 0 {
                self.transition.active = false;
                self.transition.black_backdrop = false;
                self.transition.prepared = None;
                self.transition.active_mask = None;
            } else {
                if self.transition.black_backdrop {
                    for pixel in target.bytes_mut().chunks_exact_mut(4) { pixel.copy_from_slice(&[0, 0, 0, 255]); }
                }
                let opacity = (255_i64 * i64::from(self.transition.draw_timer)
                    / i64::from(self.transition.active_duration))
                .clamp(0, 255) as u8;
                let captured = self
                    .transition
                    .prepared
                    .as_ref()
                    .ok_or("active transition without capture")?;
                if let Some(mask) = &self.transition.active_mask {
                    // 964370 progress, 8C4680.. parameters, and the observed
                    // D3D11 mask shader: saturate((mask.alpha + offset) * 4).
                    // 760720/73A587 copy the RED channel into mask texture alpha.
                    // The shipped tr_ink.png contains unequal RGB channels.
                    let progress = 255 - i32::from(opacity);
                    let q = 1280 - progress * 1280 / 255;
                    let offset = if q >= 1024 {
                        ((q - 1024) / 4) as f32 / 255.0
                    } else {
                        -((255 - q * 255 / 1024) as f32) / 255.0
                    };
                    let (mw, mh) = mask.dimensions();
                    for y in 0..self.height {
                        for x in 0..self.width {
                            let mi = ((u64::from(y) * u64::from(mh) / u64::from(self.height))
                                * u64::from(mw)
                                + u64::from(x) * u64::from(mw) / u64::from(self.width))
                                as usize
                                * 4;
                            let i = (y as usize * self.width as usize + x as usize) * 4;
                            let alpha = ((f32::from(mask.bytes()[mi]) / 255.0 + offset) * 4.0)
                                .clamp(0.0, 1.0);
                            for c in 0..3 {
                                let old = f32::from(captured.bytes()[i + c]);
                                let new = f32::from(target.bytes()[i + c]);
                                target.bytes_mut()[i + c] =
                                    (old * alpha + new * (1.0 - alpha)).round() as u8;
                            }
                        }
                    }
                    return Ok(target);
                }
                let mut overlay = Pictures::default();
                overlay.show(
                    0,
                    Picture {
                        image: captured.clone(),
                        x: 0,
                        y: 0,
                        width: self.width,
                        height: self.height,
                        opacity,
                        z: 0,
                    },
                )?;
                overlay.render(&mut target);
            }
        }
        Ok(target)
    }
}

#[path = "snapshot_bytes.rs"]
pub(crate) mod snapshot_bytes;
mod saved_image {
    use super::*;
    use serde::{Deserialize, Serialize};
    std::thread_local! { static IMAGE_BUDGET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }
    // Compressed input must not amplify into thousands of 64 MiB images.
    // This scoped budget also applies to capture, preserving read/write symmetry.
    pub(super) struct Budget(Option<usize>);
    impl Budget { pub(super) fn new(cap: usize) -> Self { Self(IMAGE_BUDGET.with(|budget| budget.replace(Some(cap)))) } }
    impl Drop for Budget { fn drop(&mut self) { IMAGE_BUDGET.with(|budget| budget.set(self.0)); } }
    fn reserve(bytes: usize) -> Result<(), String> {
        if bytes > 64 << 20 { return Err("saved image exceeds byte budget".into()); }
        IMAGE_BUDGET.with(|budget| {
            if let Some(left) = budget.get() {
                let remaining = left.checked_sub(bytes).ok_or("saved images exceed total byte budget")?;
                budget.set(Some(remaining));
            }
            Ok(())
        })
    }
    pub(super) fn serialize<S: serde::Serializer>(
        image: &Arc<Image>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        reserve(image.bytes().len()).map_err(serde::ser::Error::custom)?;
        let (w,h) = image.dimensions();
        if u64::from(w) * u64::from(h) > 16_000_000 {
            // Image::new allows 64 MiB while the PNG backend's pixel limit is
            // slightly smaller. Preserve the supported raw-image boundary.
            return (w,h,image.bytes()).serialize(serializer);
        }
        let mut png = snapshot_bytes::BoundedBytes::new(64 << 20);
        image.write_png(&mut png).map_err(serde::ser::Error::custom)?;
        let mut text = String::with_capacity(png.bytes.len() * 2);
        const HEX: &[u8] = b"0123456789abcdef";
        for byte in png.bytes {
            text.push(HEX[usize::from(byte >> 4)] as char);
            text.push(HEX[usize::from(byte & 15)] as char);
        }
        Encoded { png_hex_v1: text }.serialize(serializer)
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Image>, D::Error> {
        let image = match Saved::deserialize(deserializer)? {
            Saved::Png(encoded) => {
                let text = encoded.png_hex_v1.as_bytes();
                if text.len() > 128 << 20 || text.len() % 2 != 0 {
                    return Err(serde::de::Error::custom("saved PNG hex length limit"));
                }
                let digit = |byte: u8| -> Result<u8, D::Error> {
                    match byte { b'0'..=b'9' => Ok(byte-b'0'), b'a'..=b'f' => Ok(byte-b'a'+10), _ => Err(serde::de::Error::custom("invalid saved PNG hex")) }
                };
                let mut bytes = Vec::with_capacity(text.len()/2);
                for pair in text.chunks_exact(2) { bytes.push(digit(pair[0])? * 16 + digit(pair[1])?); }
                if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 24 {
                    return Err(serde::de::Error::custom("invalid saved PNG header"));
                }
                let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                let size = u64::from(width).checked_mul(u64::from(height)).and_then(|n|n.checked_mul(4))
                    .and_then(|n|usize::try_from(n).ok()).ok_or_else(||serde::de::Error::custom("saved image size overflow"))?;
                reserve(size).map_err(serde::de::Error::custom)?;
                Image::from_png(&bytes)
            }
            Saved::Legacy((w,h,bytes)) => { reserve(bytes.len()).map_err(serde::de::Error::custom)?; Image::new(w as usize,h as usize,bytes) },
        };
        image
            .map(Arc::new)
            .map_err(serde::de::Error::custom)
    }
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Encoded { png_hex_v1: String }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved { Png(Encoded), Legacy((u32,u32,Vec<u8>)) }

    #[test]
    fn compressed_images_are_exact_and_legacy_images_remain_readable() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper(#[serde(with="super::saved_image")] Arc<Image>);
        let mut rgba = vec![0; 1280 * 720 * 4];
        for (i, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[(i % 251) as u8, 42, (i / 1280 % 256) as u8, if i%7 == 0 { 0 } else { 255 }]);
        }
        let image = Wrapper(Arc::new(Image::new(1280,720,rgba.clone()).unwrap()));
        let bytes = serde_json::to_vec(&image).unwrap();
        assert!(bytes.len() < rgba.len()/2, "image must not expand RGBA to decimal JSON");
        let restored: Wrapper = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.0.bytes(), rgba);
        let old = serde_json::to_vec(&(1280,720,rgba)).unwrap();
        let legacy: Wrapper = serde_json::from_slice(&old).unwrap();
        assert_eq!(legacy.0.bytes(), restored.0.bytes());
        {
            let _budget = Budget::new(1 << 20);
            assert!(serde_json::from_slice::<Wrapper>(&bytes).is_err());
        }
        let again: Wrapper = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(again.0.bytes(), restored.0.bytes(), "failure must release scoped budget");
        for invalid in [r#"{"png_hex_v1":"0"}"#, r#"{"png_hex_v1":"gg"}"#, r#"{"png_hex_v1":"00"}"#] {
            assert!(serde_json::from_str::<Wrapper>(invalid).is_err());
        }
    }
}
mod saved_optional_image {
    use super::*;
    use serde::{Deserialize, Serialize};
    #[derive(Serialize, Deserialize)]
    struct Wrapped(#[serde(with = "saved_image")] Arc<Image>);
    pub(super) fn serialize<S: serde::Serializer>(
        image: &Option<Arc<Image>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        image
            .as_ref()
            .map(|image| Wrapped(image.clone()))
            .serialize(serializer)
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Arc<Image>>, D::Error> {
        Ok(Option::<Wrapped>::deserialize(deserializer)?.map(|image| image.0))
    }
}

fn validate_picture_layer(id: i32) -> Result<(), String> {
    if (-299_999..=-1).contains(&id) {
        return Err(format!(
            "picture {id} requires interleaved native map layers"
        ));
    }
    Ok(())
}

fn validate_duration(duration: i32) -> Result<(), String> {
    if !(0..=1_000_000).contains(&duration) {
        Err("native media duration outside supported range".into())
    } else {
        Ok(())
    }
}

fn anchor_offset(anchor: u32, width: u32, height: u32) -> (i32, i32) {
    // 98C5DC jump table, 98BBE4..98BC63; supported axis-aligned images.
    let (w, h) = (width as i32, height as i32);
    match anchor {
        0 => (0, 0),
        1 => (w / 2, h / 2),
        2 => (0, h),
        3 => (w, 0),
        4 => (w, h),
        5 => (w / 2, 0),
        6 => (w / 2, h),
        _ => unreachable!("validated anchor"),
    }
}

/// 9851F0 is deliberately case-specific: arbitrary mixed-case names do not match.
pub fn native_loop_name(path: &str) -> bool {
    path.contains("_loop.") || path.contains("_LOOP.")
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn opaque_framebuffer_kernel_matches_general_picture_compositor() {
        let source = Arc::new(Image::new(3, 2, (0..24).map(|i| (i * 53 % 256) as u8).collect()).unwrap());
        for opacity in [0, 1, 45, 128, 254, 255] {
            for (x, y, width, height) in [(-1, 0, 3, 2), (1, -1, 5, 4), (0, 0, 2, 1)] {
                let pixels: Vec<u8> = (0..20).flat_map(|i| [i * 7, i * 3, i * 11, 255]).collect();
                let mut expected = Image::new(5, 4, pixels.clone()).unwrap();
                let mut actual = Image::new(5, 4, pixels).unwrap();
                let mut pictures = Pictures::default();
                pictures.show(0, Picture { image: source.clone(), x, y, width, height, opacity, z: 0 }).unwrap();
                pictures.render(&mut expected);
                blend_picture(&mut actual, &source, x, y, width, height, u32::from(opacity), 0, [0; 3], None);
                assert_eq!(actual.bytes(), expected.bytes());
            }
        }
    }

    #[test]
    fn map_transition_flags_use_configured_duration_and_mode_two_black_frame() {
        let mut media = NativeMedia::new(1, 1).unwrap();
        media.set_scene_image(Arc::new(Image::new(1, 1, vec![255, 0, 0, 255]).unwrap())).unwrap();
        media.transition.duration = 4;
        assert!(media.map_transition(32).unwrap());
        assert!(media.vm_blocked());
        assert!(!media.map_transition(16).unwrap());
        media.set_scene_image(Arc::new(Image::new(1, 1, vec![0, 0, 255, 255]).unwrap())).unwrap();
        media.tick(1).unwrap();
        let during = media.render().unwrap();
        assert_eq!(during.bytes()[2], 0, "destination must remain hidden during mode two");
        assert!(during.bytes()[0] > 0 && during.bytes()[0] < 255);
        for _ in 0..4 { media.tick(1).unwrap(); media.render().unwrap(); }
        assert!(!media.vm_blocked());
        assert_eq!(media.render().unwrap().bytes(), &[0, 0, 255, 255]);
        assert!(media.map_transition(16).unwrap());
        assert!(!media.vm_blocked(), "mode one follows the configured event gate");
    }

    #[test]
    fn static_ticks_preserve_scene_cache() {
        let mut media = NativeMedia::new(2, 2).unwrap();
        media.render().unwrap();
        let cached = media.render_cache.as_ref().unwrap().clone();
        for _ in 0..60 {
            media.tick(1).unwrap();
            media.render().unwrap();
            assert!(Arc::ptr_eq(&cached, media.render_cache.as_ref().unwrap()));
        }
    }

    #[test]
    fn animation_countdown_reuses_cache_until_the_visible_frame_changes() {
        let mut media = NativeMedia::new(1, 1).unwrap();
        let first = Arc::new(Image::new(1, 1, vec![255, 0, 0, 255]).unwrap());
        let create = [0, 17, 0, 1, 1, 1, 255, 0, 0, 100, 0];
        media.picture(&create, &create, "fixture", &mut |_| Ok(first.clone())).unwrap();
        media.pictures.get_mut(&17).unwrap().frames = Some(FrameAnimation {
            sheet: Arc::new(Image::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap()),
            columns: 2, base: 1, first: 1, last: 2, current: 1,
            remaining: 4, duration: 4, mode: 1,
        });
        assert_eq!(media.render().unwrap().bytes(), &[255, 0, 0, 255]);
        let cached = media.render_cache.as_ref().unwrap().clone();
        media.replace_frame(17, first).unwrap();
        for _ in 0..3 {
            media.tick(1).unwrap();
            media.render().unwrap();
            assert!(Arc::ptr_eq(&cached, media.render_cache.as_ref().unwrap()));
        }
        media.tick(1).unwrap();
        assert_eq!(media.render().unwrap().bytes(), &[0, 0, 255, 255]);
        assert!(!Arc::ptr_eq(&cached, media.render_cache.as_ref().unwrap()));
    }

    #[test]
    fn missing_picture_moves_do_not_fault_or_target_a_future_create() {
        let mut media = NativeMedia::new(2, 2).unwrap();
        let immediate = [1, 17, 5, 0, 0, 1, 150, 10, 0, 100, 0];
        let delayed = [1, 17, 5, 0, 0, 1, 0, 30, 0, 100, 0, 0, 0x0200_0000, 10, 0, 100, 100, 100];
        for _ in 0..12 {
            for args in [immediate.as_slice(), delayed.as_slice()] {
                assert!(matches!(media.picture(args, args, "", &mut |_| Err("move must not load".into())).unwrap(), MediaOutcome::Applied));
            }
            for id in [17, 16] {
                let delete = [2, id, 20];
                media.picture(&delete, &delete, "", &mut |_| Err("delete must not load".into())).unwrap();
            }
        }
        assert!(media.queued.is_empty());
        let create = [0, 17, 0, 1, 1, 1, 255, 0, 0, 100, 0];
        media.picture(&create, &create, "fixture.png", &mut |_| {
            Ok(Arc::new(Image::new(1, 1, vec![255; 4])?))
        }).unwrap();
        for _ in 0..30 { media.tick(1).unwrap(); }
        assert_eq!(media.picture_position(17).unwrap(), Some([0, 0]));
        assert_eq!(media.pictures[&17].current().opacity, 255);
    }

    #[test]
    fn offset_effect_does_not_consume_the_third_generic_operand() {
        let mut media = NativeMedia::new(2, 2).unwrap();
        let create = [0, 17, 0, 1, 1, 1, 255, 0, 0, 100, 0];
        media.picture(&create, &create, "fixture", &mut |_| Ok(Arc::new(Image::new(1, 1, vec![255; 4])?))).unwrap();
        let effect = [0x20, 10, 17, 17, 8, -4, 40];
        media.picture_effect(&effect, &effect, "").unwrap();
        for _ in 0..10 { media.tick(1).unwrap(); }
        assert_eq!(media.pictures[&17].offset_effect.current(), [8.0, -4.0]);
    }
}
