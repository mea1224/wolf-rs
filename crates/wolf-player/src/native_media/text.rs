//! 98E2F0 text-picture state, 9737C0 tick, and 993B90 selector10.
use std::sync::Arc;
use wolf_media::Image;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Glyph {
    #[serde(with = "super::saved_image")]
    pub image: Arc<Image>,
    pub x: i32,
    pub y: i32,
    pub reveal_at: u32,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct TextReveal {
    pub width: u32,
    pub height: u32,
    pub glyphs: Vec<Glyph>,
    pub units: u32,
    pub instant_ranges: Vec<[u32; 2]>,
    pub rate: i32,
    pub accumulator: f64,
    pub last_step: i32,
    pub visible: usize,
}
impl TextReveal {
    pub fn valid(&self) -> bool {
        self.width > 0 && self.height > 0
            && u64::from(self.width) * u64::from(self.height) <= 16_000_000
            && self.glyphs.len() <= 1_048_576 && self.units <= 2_097_152
            && self.accumulator.is_finite() && self.accumulator.abs() <= 50_000_000.0
            && (0..=1_000_000).contains(&self.last_step) && self.visible <= self.glyphs.len()
            && self.glyphs.iter().all(|g| g.reveal_at <= self.units)
            && self.instant_ranges.iter().all(|r| r[0] <= self.units + 1 && r[1] <= self.units)
    }
    pub fn finished(&self) -> bool {
        // +E0 removes packet and UTF-8 surplus bytes from +18; the remaining
        // threshold is ASCII1/wide2. Native tests the previous reveal step.
        self.accumulator.trunc() - f64::from(self.last_step) * f64::from(self.rate) / 30.0 + 1.0 >= f64::from(self.units)
    }
    pub fn tick(&mut self, step: i32) -> Result<Option<Arc<Image>>, String> {
        self.last_step = step;
        self.accumulator = (self.accumulator + f64::from(step) * f64::from(self.rate) / 30.0).min(50_000_000.0);
        for &[start, end] in &self.instant_ranges {
            if self.accumulator >= f64::from(start) { self.accumulator = self.accumulator.max(f64::from(end)); }
        }
        let visible = self.glyphs.iter().take_while(|g| f64::from(g.reveal_at) <= self.accumulator).count();
        if visible == self.visible { return Ok(None); }
        self.visible = visible;
        self.image().map(Some)
    }
    pub fn image(&self) -> Result<Arc<Image>, String> {
        let mut pixels = vec![0u8; self.width as usize * self.height as usize * 4];
        for glyph in self.glyphs.iter().take(self.visible) {
            let (width, height) = glyph.image.dimensions();
            for sy in 0..height {
                let dy = glyph.y + sy as i32;
                if !(0..self.height as i32).contains(&dy) { continue; }
                for sx in 0..width {
                    let dx = glyph.x + sx as i32;
                    if !(0..self.width as i32).contains(&dx) { continue; }
                    let at = (sy as usize * width as usize + sx as usize) * 4;
                    let source = &glyph.image.bytes()[at..at + 4];
                    let at = (dy as usize * self.width as usize + dx as usize) * 4;
                    let dest = &mut pixels[at..at + 4];
                    let sa = u32::from(source[3]);
                    let da = u32::from(dest[3]);
                    let alpha = sa * 255 + da * (255 - sa);
                    if alpha != 0 {
                        for channel in 0..3 {
                            dest[channel] = ((u32::from(source[channel]) * sa * 255
                                + u32::from(dest[channel]) * da * (255 - sa) + alpha / 2) / alpha) as u8;
                        }
                    }
                    dest[3] = ((alpha + 127) / 255) as u8;
                }
            }
        }
        Image::new(self.width as usize, self.height as usize, pixels).map(Arc::new)
    }
}
