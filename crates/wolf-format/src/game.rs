//! Structural Game.dat decoding. Numeric startup semantics remain unverified.
use crate::{reader::Reader, Domain, Error, Limits, Result, Span, WolfString};

#[derive(Debug)]
pub struct GameConfig {
    pub configuration: Vec<u8>,
    pub title: WolfString,
    pub font: WolfString,
    pub fallback_fonts: Vec<WolfString>,
    pub default_character_graphic: WolfString,
    pub extra_strings: Vec<WolfString>,
    pub declared_size: u32,
    pub unknown_size: u32,
    pub unknown_words: Vec<u8>,
    pub internal_offsets: [u32; 2],
    pub opaque_tail: Vec<u8>,
    pub opaque_tail_span: Span,
}

/// Accepts the decoded member, including the optional archive protection notice.
/// Spans are relative to the Game.dat header, after that notice.
pub fn parse_game(bytes: &[u8], limits: &Limits) -> Result<GameConfig> {
    if bytes.len() > limits.max_stored_bytes {
        return Err(Error::new(0, Domain::Stored, "Game.dat stored size limit"));
    }
    let bytes = bytes
        .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
        .unwrap_or(bytes);
    let mut r = Reader::new(bytes, 0, Domain::Stored, limits)?;
    r.expect(&[0, 0x57, 0, 0, 0x4f, 0x4c, 0, 0x46, 0x4d, 0x55])?;
    let n = r.count()?;
    let configuration = r.blob(n)?;
    let strings = r.u32()?;
    if !matches!(strings, 8 | 9 | 13 | 14) {
        return Err(r.error("unsupported Game.dat string layout"));
    }
    let title = r.raw_string()?;
    if r.string()?.text().unwrap_or("") != "0000-0000" {
        return Err(r.error("invalid Game.dat magic string"));
    }
    let key_bytes = r.count()?;
    r.take(key_bytes)?; // Consume protection metadata without exposing it.
    let font = r.raw_string()?;
    let mut fallback_fonts = Vec::new();
    for _ in 0..3 {
        fallback_fonts.push(r.raw_string()?);
    }
    let default_character_graphic = r.raw_string()?;
    let mut extra_strings = Vec::new();
    for _ in 8..strings {
        extra_strings.push(r.raw_string()?);
    }
    let declared_size = r.u32()?;
    if declared_size as usize != bytes.len().saturating_sub(1) {
        return Err(r.error("Game.dat declared size mismatch"));
    }
    let unknown_size = r.u32()?;
    let words = r.count()?;
    let word_bytes = words
        .checked_mul(2)
        .ok_or_else(|| r.error("word size overflow"))?;
    let unknown_words = r.blob(word_bytes)?;
    let internal_offsets = [r.u32()?, r.u32()?];
    let tail_start = r.pos;
    let opaque_tail = r.blob(r.remaining())?;
    let opaque_tail_span = r.span(tail_start);
    r.eof()?;
    Ok(GameConfig {
        configuration,
        title,
        font,
        fallback_fonts,
        default_character_graphic,
        extra_strings,
        declared_size,
        unknown_size,
        unknown_words,
        internal_offsets,
        opaque_tail,
        opaque_tail_span,
    })
}
