//! Portable offline media backends. No WOLF opcode mapping or device API.
use std::{collections::BTreeMap, sync::Arc};
mod decode;

pub struct Image {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}
impl Image {
    /// Straight-alpha RGBA8, composed in byte space (not linear-light).
    pub fn new(width: usize, height: usize, rgba: Vec<u8>) -> Result<Self, String> {
        let size = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(4))
            .ok_or("image size overflow")?;
        if width == 0 || height == 0 || size > 64 << 20 || rgba.len() != size {
            return Err("invalid image dimensions/byte limit".into());
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.rgba
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.rgba
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }
}
pub struct Picture {
    pub image: Arc<Image>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub opacity: u8,
    pub z: i32,
}
#[derive(Default)]
pub struct Pictures {
    pictures: BTreeMap<u32, Picture>,
}
impl Pictures {
    pub fn show(&mut self, id: u32, picture: Picture) -> Result<(), String> {
        if (self.pictures.len() >= 4096 && !self.pictures.contains_key(&id))
            || picture.width == 0
            || picture.height == 0
            || picture.width > 32768
            || picture.height > 32768
        {
            return Err("picture count/size limit".into());
        }
        self.pictures.insert(id, picture);
        Ok(())
    }
    pub fn erase(&mut self, id: u32) {
        self.pictures.remove(&id);
    }
    /// Stable (z, ID) order, clipped nearest scaling, straight-alpha source-over.
    pub fn render(&self, target: &mut Image) {
        let mut ordered: Vec<_> = self.pictures.iter().collect();
        ordered.sort_by_key(|(id, p)| (p.z, **id));
        for (_, p) in ordered {
            let left = i64::from(p.x).max(0).min(target.width as i64) as usize;
            let top = i64::from(p.y).max(0).min(target.height as i64) as usize;
            let right = (i64::from(p.x) + i64::from(p.width))
                .max(0)
                .min(target.width as i64) as usize;
            let bottom = (i64::from(p.y) + i64::from(p.height))
                .max(0)
                .min(target.height as i64) as usize;
            if left == right || top == bottom {
                continue;
            }
            let source_left = (left as i64 - i64::from(p.x)) as usize;
            let direct_rows = p.width as usize == p.image.width;
            // The horizontal nearest-neighbor mapping is identical on every
            // row. Avoid repeating integer divisions for every source pixel.
            let source_x: Vec<_> = if direct_rows {
                Vec::new()
            } else {
                (left..right)
                    .map(|x| {
                        ((x as i64 - i64::from(p.x)) as usize * p.image.width) / p.width as usize
                            * 4
                    })
                    .collect()
            };
            for y in top..bottom {
                let sy =
                    ((y as i64 - i64::from(p.y)) as usize * p.image.height) / p.height as usize;
                let source_row = &p.image.rgba[sy * p.image.width * 4..][..p.image.width * 4];
                let dest_row =
                    &mut target.rgba[(y * target.width + left) * 4..][..(right - left) * 4];
                if direct_rows {
                    let source_row = &source_row[source_left * 4..][..dest_row.len()];
                    // Inspect current bytes: Image::bytes_mut makes a cached
                    // opaque flag invalid without explicit invalidation.
                    if p.opacity == 255 && source_row.chunks_exact(4).all(|s| s[3] == 255) {
                        dest_row.copy_from_slice(source_row);
                        continue;
                    }
                    for (source, dest) in
                        source_row.chunks_exact(4).zip(dest_row.chunks_exact_mut(4))
                    {
                        source_over(source, dest, u32::from(p.opacity));
                    }
                } else {
                    for (&sx, dest) in source_x.iter().zip(dest_row.chunks_exact_mut(4)) {
                        source_over(&source_row[sx..sx + 4], dest, u32::from(p.opacity));
                    }
                }
            }
        }
    }
}

#[inline]
fn source_over(source: &[u8], dest: &mut [u8], opacity: u32) {
    let alpha = (u32::from(source[3]) * opacity + 127) / 255;
    if alpha == 255 {
        dest.copy_from_slice(source);
        return;
    }
    let da = u32::from(dest[3]);
    if alpha == 0 {
        if da == 0 {
            // Preserve the original normalization of invisible RGB to zero.
            dest.fill(0);
        }
        return;
    }
    let inverse = 255 - alpha;
    if da == 255 {
        // out == 65025. Cancel a factor of 255 while preserving the
        // original round-to-nearest integer result exactly.
        for channel in 0..3 {
            dest[channel] =
                ((u32::from(source[channel]) * alpha + u32::from(dest[channel]) * inverse + 127)
                    / 255) as u8;
        }
        return;
    }
    let out = alpha * 255 + da * inverse;
    for channel in 0..3 {
        dest[channel] = ((u32::from(source[channel]) * alpha * 255
            + u32::from(dest[channel]) * da * inverse
            + out / 2)
            / out) as u8;
    }
    dest[3] = ((out + 127) / 255) as u8;
}

#[derive(Clone)]
pub struct Clip {
    sample_rate: u32,
    stereo: Arc<[[i16; 2]]>,
}
impl Clip {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn frames(&self) -> &[[i16; 2]] {
        &self.stereo
    }
    /// Explicit nearest resampling for device preview, not original WOLF mixing.
    pub fn resample(&self, rate: u32) -> Result<Self, String> {
        self.resample_playback(rate, 100)
    }
    /// Resample a percentage playback frequency into a fixed-rate device stream.
    pub fn resample_playback(&self, rate: u32, pitch: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&rate) {
            return Err("invalid resample rate".into());
        }
        if pitch == 0 { return Err("zero playback frequency percentage".into()); }
        if rate == self.sample_rate && pitch == 100 { return Ok(self.clone()); }
        let source_step = u64::from(self.sample_rate) * u64::from(pitch);
        let target_step = u64::from(rate) * 100;
        let count =
            (self.stereo.len() as u64 * target_step).div_ceil(source_step);
        if count > 16_000_000 {
            return Err("resample frame limit".into());
        }
        Self::new(
            rate,
            (0..count)
                .map(|i| self.stereo[(i * source_step / target_step) as usize])
                .collect(),
        )
    }
    pub fn new(sample_rate: u32, stereo: Vec<[i16; 2]>) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) || stereo.is_empty() || stereo.len() > 16_000_000
        {
            return Err("invalid PCM sample rate/frame limit".into());
        }
        Ok(Self {
            sample_rate,
            stereo: stereo.into(),
        })
    }
}
struct Voice {
    clip: Arc<Clip>,
    cursor: usize,
    gain: u16,
    looping: bool,
    paused: bool,
}
pub struct Mixer {
    sample_rate: u32,
    voices: BTreeMap<u32, Voice>,
}
impl Mixer {
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("invalid mixer sample rate".into());
        }
        Ok(Self {
            sample_rate,
            voices: BTreeMap::new(),
        })
    }
    /// gain 0..=256, replacing an existing channel restarts it at frame zero.
    /// Sample-rate mismatches fail rather than silently changing pitch.
    pub fn play(
        &mut self,
        channel: u32,
        clip: Arc<Clip>,
        gain: u16,
        looping: bool,
    ) -> Result<(), String> {
        if gain > 256
            || clip.sample_rate != self.sample_rate
            || (self.voices.len() >= 256 && !self.voices.contains_key(&channel))
        {
            return Err("audio gain/sample rate/voice limit".into());
        }
        self.voices.insert(
            channel,
            Voice {
                clip,
                cursor: 0,
                gain,
                looping,
                paused: false,
            },
        );
        Ok(())
    }
    pub fn stop(&mut self, channel: u32) {
        self.voices.remove(&channel);
    }
    /// Restore a source-frame position after loading a clip at this mixer's rate.
    pub fn seek(&mut self, channel: u32, frame: u64) -> Result<(), String> {
        let voice = self
            .voices
            .get_mut(&channel)
            .ok_or("unknown audio channel")?;
        let length = voice.clip.stereo.len() as u64;
        voice.cursor = if voice.looping {
            (frame % length) as usize
        } else {
            frame.min(length) as usize
        };
        Ok(())
    }
    pub fn pause(&mut self, channel: u32, paused: bool) -> Result<(), String> {
        self.voices
            .get_mut(&channel)
            .ok_or("unknown audio channel")?
            .paused = paused;
        Ok(())
    }
    pub fn gain(&mut self, channel: u32, gain: u16) -> Result<(), String> {
        if gain > 256 {
            return Err("audio gain limit".into());
        }
        self.voices
            .get_mut(&channel)
            .ok_or("unknown audio channel")?
            .gain = gain;
        Ok(())
    }
    /// Caller supplies bounded output; returns normally finished channel IDs.
    /// i64 accumulation avoids order-dependent intermediate clipping.
    pub fn render(&mut self, output: &mut [[i16; 2]]) -> Result<Vec<u32>, String> {
        if output.len() > 1_000_000 {
            return Err("audio output frame limit".into());
        }
        let mut finished: Vec<_> = self
            .voices
            .iter()
            .filter(|(_, voice)| !voice.looping && voice.cursor == voice.clip.stereo.len())
            .map(|(&id, _)| id)
            .collect();
        for frame in output {
            let mut sum = [0i64; 2];
            for (&id, v) in &mut self.voices {
                if v.paused || v.cursor == v.clip.stereo.len() {
                    continue;
                }
                for (c, value) in sum.iter_mut().enumerate() {
                    *value += i64::from(v.clip.stereo[v.cursor][c]) * i64::from(v.gain);
                }
                v.cursor += 1;
                if v.cursor == v.clip.stereo.len() {
                    if v.looping {
                        v.cursor = 0;
                    } else {
                        finished.push(id);
                    }
                }
            }
            for c in 0..2 {
                frame[c] = (sum[c] / 256).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
            }
        }
        for id in &finished {
            self.voices.remove(id);
        }
        Ok(finished)
    }
}
