use crate::{reader::Reader, Domain, Error, Limits, Result};

/// Raw LZ4 block, NOT the DX archive's custom LZ or an LZ4 frame.
pub fn decode_block(input: &[u8], expected: usize, cap: usize) -> Result<Vec<u8>> {
    let limits = Limits::default();
    let mut r = Reader::new(input, 0, Domain::Stored, &limits)?;
    if expected > cap {
        return Err(r.error("LZ4 output size limit"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(expected)
        .map_err(|_| r.error("LZ4 allocation failed"))?;
    fn length(r: &mut Reader<'_>, initial: usize) -> Result<usize> {
        let mut n = initial;
        if initial == 15 {
            loop {
                let v = r.u8()?;
                n = n
                    .checked_add(v as usize)
                    .ok_or_else(|| r.error("LZ4 length overflow"))?;
                if v != 255 {
                    break;
                }
            }
        }
        Ok(n)
    }
    while r.remaining() != 0 {
        let token = r.u8()?;
        let literals = length(&mut r, (token >> 4) as usize)?;
        if literals > expected - out.len() {
            return Err(r.error("LZ4 literal overflow"));
        }
        out.extend_from_slice(r.take(literals)?);
        if r.remaining() == 0 {
            break;
        }
        let offset = u16::from_le_bytes(r.take(2)?.try_into().unwrap()) as usize;
        if offset == 0 || offset > out.len() {
            return Err(r.error("LZ4 invalid backreference"));
        }
        let count = length(&mut r, (token & 15) as usize)?
            .checked_add(4)
            .ok_or_else(|| r.error("LZ4 match length overflow"))?;
        if count > expected - out.len() {
            return Err(r.error("LZ4 match overflow"));
        }
        // Bytewise copy is required for overlapping matches.
        for _ in 0..count {
            out.push(out[out.len() - offset]);
        }
    }
    if out.len() != expected {
        return Err(r.error("LZ4 output length mismatch"));
    }
    Ok(out)
}

pub(crate) fn reconstruct(input: &[u8], prefix: usize, limits: &Limits) -> Result<Vec<u8>> {
    if input.len() > limits.max_stored_bytes {
        return Err(Error::new(0, Domain::Stored, "stored file size limit"));
    }
    let mut r = Reader::new(input, prefix, Domain::Stored, limits)?;
    let size = r.u32()? as usize;
    let compressed = r.u32()? as usize;
    let total = prefix
        .checked_add(size)
        .ok_or_else(|| r.error("reconstructed length overflow"))?;
    if total > limits.max_decoded_bytes {
        return Err(r.error("reconstructed file size limit"));
    }
    let compressed_start = r.pos;
    let block = r.take(compressed)?;
    r.eof()?;
    let payload = decode_block(block, size, limits.max_decoded_bytes).map_err(|mut e| {
        e.offset += compressed_start;
        e
    })?;
    let mut out = Vec::new();
    out.try_reserve_exact(total)
        .map_err(|_| r.error("reconstruction allocation failed"))?;
    out.extend_from_slice(&input[..prefix]);
    out.extend_from_slice(&payload);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_overlap_and_corruption() {
        assert_eq!(
            decode_block(&[0x30, b'a', b'b', b'c'], 3, 10).unwrap(),
            b"abc"
        );
        assert_eq!(
            decode_block(&[0x13, b'a', 1, 0], 8, 10).unwrap(),
            b"aaaaaaaa"
        );
        for block in [&[0, 0, 0][..], &[0xf0], &[0x30, b'a'], &[0x13, b'a', 2, 0]] {
            assert!(decode_block(block, 8, 10).is_err());
        }
        assert!(decode_block(&[], 11, 10).is_err());
    }
}
