use crate::{Clip, Image};
use std::io::Cursor;

impl Image {
    /// Signature-driven decoding: game resources use both PNG and JPEG.
    pub fn from_encoded(bytes: &[u8]) -> Result<Self, String> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Self::from_png(bytes); }
        if !bytes.starts_with(&[0xff,0xd8]) { return Err("unsupported image signature".into()); }
        if bytes.len() > 64 << 20 { return Err("JPEG input byte limit".into()); }
        let options = zune_core::options::DecoderOptions::default()
            .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGBA)
            .set_max_width(8192).set_max_height(8192);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(bytes),options);
        decoder.decode_headers().map_err(|e|e.to_string())?;
        let (width,height) = decoder.dimensions().ok_or("JPEG dimensions missing")?;
        if width == 0 || height == 0 || width.checked_mul(height).is_none_or(|n|n>16_000_000) { return Err("JPEG pixel limit".into()); }
        let pixels = decoder.decode().map_err(|e|e.to_string())?;
        Self::new(width,height,pixels)
    }
    pub fn write_png(&self, output: impl std::io::Write) -> Result<(), String> {
        let (w, h) = self.dimensions();
        let mut encoder = png::Encoder::new(output, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(self.bytes())
            .map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())
    }
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 64 << 20 {
            return Err("PNG input byte limit".into());
        }
        let mut decoder =
            png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: 64 << 20 });
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let info = reader.info();
        let pixels = u64::from(info.width) * u64::from(info.height);
        if pixels == 0 || pixels > 16_000_000 || info.animation_control.is_some() {
            return Err("PNG dimensions/animation unsupported".into());
        }
        let size = reader.output_buffer_size().ok_or("PNG output overflow")?;
        if size > 64 << 20 {
            return Err("PNG output byte limit".into());
        }
        let mut decoded = vec![0; size];
        let frame = reader.next_frame(&mut decoded).map_err(|e| e.to_string())?;
        let data = &decoded[..frame.buffer_size()];
        let channels = match frame.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            _ => return Err("unexpanded PNG palette".into()),
        };
        let mut rgba = Vec::with_capacity(pixels as usize * 4);
        for p in data.chunks_exact(channels) {
            let value = match channels {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            };
            rgba.extend_from_slice(&value);
        }
        reader.finish().map_err(|e| e.to_string())?;
        Self::new(frame.width as usize, frame.height as usize, rgba)
    }
}

impl Clip {
    /// Bounded Ogg Vorbis decode. Mono expands equally into both output channels.
    pub fn from_ogg(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 64 << 20 {
            return Err("OGG input byte limit".into());
        }
        let mut reader = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes))
            .map_err(|e| e.to_string())?;
        let channels = reader.ident_hdr.audio_channels as usize;
        let rate = reader.ident_hdr.audio_sample_rate;
        if !(1..=2).contains(&channels) || !(8000..=192000).contains(&rate) {
            return Err("OGG channel/rate unsupported".into());
        }
        let mut frames = Vec::new();
        while let Some(packet) = reader.read_dec_packet_itl().map_err(|e| e.to_string())? {
            if reader.ident_hdr.audio_channels as usize != channels
                || reader.ident_hdr.audio_sample_rate != rate
                || packet.len() % channels != 0
            {
                return Err("OGG stream format changed".into());
            }
            if frames.len() + packet.len() / channels > 16_000_000 {
                return Err("OGG decoded frame limit".into());
            }
            frames.extend(
                packet
                    .chunks_exact(channels)
                    .map(|s| [s[0], s[channels - 1]]),
            );
        }
        Self::new(rate, frames)
    }

    /// RIFF PCM16 mono/stereo only. Rejects ambiguous duplicate chunks.
    pub fn from_wav(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 12
            || bytes.len() > 64 << 20
            || &bytes[..4] != b"RIFF"
            || &bytes[8..12] != b"WAVE"
        {
            return Err("invalid WAV header/limit".into());
        }
        let u32le = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
        if u32le(&bytes[4..8]) as u64 + 8 != bytes.len() as u64 {
            return Err("WAV RIFF size mismatch".into());
        }
        let mut fmt = None;
        let mut data = None;
        let mut at = 12;
        while at < bytes.len() {
            let header = bytes.get(at..at + 8).ok_or("truncated WAV chunk")?;
            let len = u32le(&header[4..]) as usize;
            at += 8;
            let end = at.checked_add(len).ok_or("WAV chunk overflow")?;
            let chunk = bytes.get(at..end).ok_or("truncated WAV data")?;
            match &header[..4] {
                b"fmt " => {
                    if fmt.replace(chunk).is_some() {
                        return Err("duplicate WAV fmt".into());
                    }
                }
                b"data" => {
                    if data.replace(chunk).is_some() {
                        return Err("duplicate WAV data".into());
                    }
                }
                _ => {}
            }
            at = end.checked_add(len % 2).ok_or("WAV padding overflow")?;
            if at > bytes.len() {
                return Err("missing WAV padding".into());
            }
        }
        let fmt = fmt.ok_or("missing WAV fmt")?;
        let data = data.ok_or("missing WAV data")?;
        if fmt.len() != 16 && !(fmt.len() == 18 && fmt[16..] == [0, 0]) {
            return Err("unsupported WAV fmt extension".into());
        }
        let u16le = |at| u16::from_le_bytes([fmt[at], fmt[at + 1]]);
        let channels = u16le(2) as usize;
        let rate = u32le(&fmt[4..8]);
        if u16le(0) != 1
            || !(1..=2).contains(&channels)
            || u16le(14) != 16
            || !(8000..=192000).contains(&rate)
            || u16le(12) as usize != channels * 2
            || u32le(&fmt[8..12]) != rate * channels as u32 * 2
            || data.len() % (channels * 2) != 0
            || data.len() / (channels * 2) > 16_000_000
        {
            return Err("unsupported/inconsistent WAV PCM format".into());
        }
        Self::new(
            rate,
            data.chunks_exact(channels * 2)
                .map(|s| {
                    let left = i16::from_le_bytes([s[0], s[1]]);
                    [
                        left,
                        if channels == 1 {
                            left
                        } else {
                            i16::from_le_bytes([s[2], s[3]])
                        },
                    ]
                })
                .collect(),
        )
    }
}
