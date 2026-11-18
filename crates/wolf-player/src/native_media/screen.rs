//! A1AF40/961710/961940 and A41185..A414E3: map tone and screen flash.
use wolf_media::Image;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct Screen {
    current: [i16; 3],
    target: [i16; 3],
    start: [i16; 3],
    remaining: i16,
    duration: i16,
    flash: [i16; 3],
    flash_remaining: i16,
    flash_duration: i16,
}
impl Default for Screen {
    fn default() -> Self {
        Self { current: [0; 3], target: [0; 3], start: [0; 3], remaining: -1,
            duration: 1, flash: [0; 3], flash_remaining: 0, flash_duration: 1 }
    }
}
impl Screen {
    pub(super) fn restore_native(fields: &std::collections::BTreeMap<usize, Vec<u8>>) -> Result<Self, String> {
        let word = |offset| -> Result<i16, String> {
            let (&start, bytes) = fields.range(..=offset).next_back().ok_or("saved screen field missing")?;
            let bytes = bytes.get(offset-start..offset-start+2).ok_or("saved screen field truncated")?;
            Ok(i16::from_le_bytes(bytes.try_into().unwrap()))
        };
        let triplet = |offset| -> Result<[i16;3], String> { Ok([word(offset)?,word(offset+2)?,word(offset+4)?]) };
        let state = Self { current: triplet(2324)?, target: triplet(2330)?, start: triplet(2336)?,
            remaining: word(2342)?, duration: word(2344)?, flash: triplet(2346)?,
            flash_remaining: word(2352)?, flash_duration: word(2354)? };
        if !state.valid() { return Err("invalid native saved screen effect".into()); }
        Ok(state)
    }
    pub(super) fn execute(&mut self, raw: &[i32], args: &[i32]) -> Result<(), String> {
        if raw.len() != args.len() || !matches!(args.len(), 2 | 5) {
            return Err("151 requires packed RGB/duration or flags/duration/three channels".into());
        }
        let flash = raw[0] & 0x0100_0000 != 0;
        let channels: [i32; 3] = std::array::from_fn(|i| {
            let percent = if args.len() == 2 { (raw[0] >> (8 * i)) & 255 } else { args[i + 2] };
            percent.wrapping_sub(100).wrapping_mul(255) / 100 + if flash { 255 } else { 0 }
        });
        let duration = args[1].wrapping_add(1) as i16;
        // Native stores signed words. Zero would divide by zero when ticking.
        if !flash && duration == 0 { return Err("151 tone duration wraps to zero".into()); }
        if flash {
            self.flash = channels.map(|n| n as i16);
            self.flash_remaining = duration;
            self.flash_duration = duration;
        } else {
            self.start = self.current;
            self.target = channels.map(|n| n as i16);
            self.remaining = duration;
            self.duration = duration;
        }
        Ok(())
    }
    pub(super) fn valid(&self) -> bool {
        (self.remaining < 0 || self.duration != 0)
            && (self.flash_remaining <= 0 || self.flash_duration > 0)
    }
    pub(super) fn tick(&mut self, step: i32) -> bool {
        let before = self.current;
        if self.remaining >= 0 {
            self.current = std::array::from_fn(|i| {
                (i32::from(self.target[i]) + (i32::from(self.start[i]) - i32::from(self.target[i]))
                    * i32::from(self.remaining) / i32::from(self.duration)) as i16
            });
            self.remaining = if self.remaining == 0 { -1 } else { (i32::from(self.remaining) - step).max(0) as i16 };
        }
        let flashing = self.flash_remaining > 0;
        if flashing { self.flash_remaining = (i32::from(self.flash_remaining) - step) as i16; }
        before != self.current || flashing
    }
    /// Tone applies to the map framebuffer before the >=100000 picture bucket.
    pub(super) fn tone(&self, image: &mut Image) {
        if self.current == [0; 3] { return; }
        let tables: [[u8; 256]; 3] = std::array::from_fn(|c| std::array::from_fn(|value| {
            let tone = i32::from(self.current[c]);
            let dark = (value as i32 * (255 + tone.min(0)).clamp(0,255) + 127) / 255;
            (dark + tone.max(0)).clamp(0,255) as u8
        }));
        for p in image.bytes_mut().chunks_exact_mut(4) {
            for (c, channel) in p[..3].iter_mut().enumerate() { *channel = tables[c][*channel as usize]; }
        }
    }
    /// A41408..A414D6: normal alpha rectangle after pictures, before transition.
    pub(super) fn flash(&self, image: &mut Image) {
        if self.flash_remaining <= 0 { return; }
        let alpha = 255 * i32::from(self.flash_remaining) / i32::from(self.flash_duration);
        for p in image.bytes_mut().chunks_exact_mut(4) {
            for (c, channel) in p[..3].iter_mut().enumerate() {
                *channel = ((i32::from(*channel) * (255-alpha) + i32::from(self.flash[c]).clamp(0,255)*alpha +127)/255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixel() -> Image { Image::new(1,1,vec![100,150,200,255]).unwrap() }
    #[test]
    fn tone_uses_current_start_and_native_inclusive_counter() {
        let mut s = Screen::default();
        s.execute(&[0, 1], &[0, 1]).unwrap();
        s.tick(1); assert_eq!(s.current, [0;3]);
        s.tick(1); assert_eq!(s.current, [-128;3]);
        let mut image = pixel(); s.tone(&mut image); assert_eq!(image.bytes(), &[50,75,100,255]);
        s.tick(1); assert_eq!(s.current, [-255;3]);
        s.execute(&[0,1,100,100,100], &[0,1,100,100,100]).unwrap();
        s.tick(1); assert_eq!(s.current, [-255;3]);
        s.tick(1); assert_eq!(s.current, [-127;3]);
        let bytes = serde_json::to_vec(&s).unwrap();
        let mut restored: Screen = serde_json::from_slice(&bytes).unwrap();
        restored.tick(1); assert_eq!(restored.current,[0;3]);
    }
    #[test]
    fn flash_fades_and_does_not_change_tone() {
        let mut s = Screen::default();
        s.execute(&[0x01646464, 1], &[0x01646464,1]).unwrap();
        let mut image = pixel(); s.flash(&mut image); assert_eq!(image.bytes(), &[255;4]);
        s.tick(1); let mut image = pixel(); s.flash(&mut image);
        assert_eq!(image.bytes(), &[177,202,227,255]);
        s.tick(1); let mut image = pixel(); s.flash(&mut image); assert_eq!(image.bytes(),pixel().bytes());
        assert_eq!(s.current,[0;3]);
    }
}
