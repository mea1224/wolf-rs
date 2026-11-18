// Bounded DXArchive Huffman and LZ readers, algorithm by Takumi Yamada.
use super::{bad, range, Result};
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Result<u64> {
        if n > 64
            || self
                .pos
                .checked_add(n)
                .is_none_or(|p| p > self.data.len().saturating_mul(8))
        {
            return Err(bad("truncated Huffman header"));
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u64::from((self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Ok(v)
    }
}
pub(super) fn huffman(src: &[u8], cap: usize) -> Result<Vec<u8>> {
    let mut b = Bits { data: src, pos: 0 };
    let n = b.read(6)? as usize + 1;
    let size = usize::try_from(b.read(n)?).map_err(|_| bad("Huffman size overflow"))?;
    let n = b.read(6)? as usize + 1;
    let stored = usize::try_from(b.read(n)?).map_err(|_| bad("Huffman size overflow"))?;
    if size > cap {
        return Err(bad("Huffman output limit"));
    }
    let mut weights = [0u64; 511];
    let mut prev = 0u16;
    for (i, w) in weights[..256].iter_mut().enumerate() {
        let n = (b.read(3)? as usize + 1) * 2;
        let minus = b.read(1)? != 0;
        let delta = b.read(n)? as u16;
        prev = if i == 0 {
            delta
        } else if minus {
            prev.wrapping_sub(delta)
        } else {
            prev.wrapping_add(delta)
        };
        *w = u64::from(prev);
    }
    let header_size = b.pos.div_ceil(8);
    if src.len().checked_sub(header_size) != Some(stored) {
        return Err(bad("Huffman stored length mismatch"));
    }
    let payload = range(src, header_size, stored)?;
    let mut used = [false; 511];
    let mut children = [[0usize; 2]; 511];
    for at in 256..511 {
        let mut first = None;
        let mut second = None;
        for i in 0..at {
            if used[i] {
                continue;
            }
            if first.is_none_or(|j| weights[j] > weights[i]) {
                second = first;
                first = Some(i);
            } else if second.is_none_or(|j| weights[j] > weights[i]) {
                second = Some(i);
            }
        }
        let (a, c) = (first.unwrap(), second.unwrap());
        used[a] = true;
        used[c] = true;
        children[at] = [a, c];
        weights[at] = weights[a] + weights[c];
    }
    let mut out = Vec::with_capacity(size);
    let mut pos = 0usize;
    while out.len() < size {
        let mut at = 510;
        while at >= 256 {
            let byte = *payload
                .get(pos / 8)
                .ok_or_else(|| bad("truncated Huffman payload"))?;
            at = children[at][usize::from((byte >> (pos % 8)) & 1)];
            pos += 1;
        }
        out.push(at as u8);
    }
    Ok(out)
}
pub(super) fn lz(src: &[u8], cap: usize) -> Result<Vec<u8>> {
    let head = range(src, 0, 9)?;
    let size = u32::from_le_bytes(head[0..4].try_into().unwrap()) as usize;
    let stored = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
    if size > cap || stored != src.len() || stored < 9 {
        return Err(bad("DX LZ header/limit"));
    }
    let key = head[8];
    let mut p = 9;
    let mut out = Vec::with_capacity(size);
    while p < src.len() {
        // Check before pushing: a malformed extra literal must not grow the allocation.
        if out.len() == size {
            return Err(bad("DX LZ output overrun"));
        }
        let byte = src[p];
        p += 1;
        if byte != key {
            out.push(byte);
        } else {
            let byte = *src.get(p).ok_or_else(|| bad("DX LZ code truncated"))?;
            p += 1;
            if byte == key {
                out.push(key);
            } else {
                let code = if byte > key { byte - 1 } else { byte };
                let mut len = usize::from(code >> 3);
                if code & 4 != 0 {
                    len |=
                        usize::from(*src.get(p).ok_or_else(|| bad("DX LZ length truncated"))?) << 5;
                    p += 1;
                }
                len += 4;
                let n = usize::from(code & 3) + 1;
                if n == 4 {
                    return Err(bad("reserved DX LZ distance mode"));
                }
                let mut distance = 0;
                for (i, b) in range(src, p, n)?.iter().enumerate() {
                    distance |= usize::from(*b) << (8 * i);
                }
                p += n;
                distance += 1;
                if distance > out.len() || len > size.saturating_sub(out.len()) {
                    return Err(bad("DX LZ reference out of bounds"));
                }
                for _ in 0..len {
                    out.push(out[out.len() - distance]);
                }
            }
        }
        if out.len() > size {
            return Err(bad("DX LZ output overrun"));
        }
    }
    if out.len() != size {
        return Err(bad("DX LZ output size mismatch"));
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lz_overlap_and_bad_distance() {
        let mut x = vec![5, 0, 0, 0, 13, 0, 0, 0, 255, b'a', 255, 0, 0];
        assert_eq!(lz(&x, 5).unwrap(), b"aaaaa");
        assert!(lz(&x, 4).is_err());
        for end in 0..x.len() {
            assert!(lz(&x[..end], 5).is_err());
        }
        x[12] = 9;
        assert!(lz(&x, 5).is_err());
        assert!(lz(&x[..8], 5).is_err());
    }
    #[test]
    fn lz_rejects_reserved_modes_and_output_overrun() {
        let reserved = [5, 0, 0, 0, 15, 0, 0, 0, 255, b'a', 255, 3, 0, 0, 0];
        assert!(lz(&reserved, 5).is_err());
        let excess_literal = [0, 0, 0, 0, 10, 0, 0, 0, 255, b'a'];
        assert!(lz(&excess_literal, 5).is_err());
        let missing_escape = [1, 0, 0, 0, 10, 0, 0, 0, 255, 255];
        assert!(lz(&missing_escape, 5).is_err());
    }
    #[test]
    fn bounded_huffman_headers() {
        for n in 0..80 {
            assert!(huffman(&vec![255; n], 1024).is_err());
        }
    }
    #[test]
    fn huffman_equal_weights_bit_order_and_exact_stored_size() {
        // An independently described stream: all 256 leaves have weight 1, so the
        // stable tie order gives byte A (01000001) its 8-bit code in LSB-first storage.
        let mut bits = Vec::new();
        let mut write = |n: usize, v: u64| {
            for shift in (0..n).rev() {
                bits.push(((v >> shift) & 1) as u8);
            }
        };
        write(6, 0); // 1-bit decoded length
        write(1, 1);
        write(6, 1); // 2-bit stored length, including encoder's final padding byte
        write(2, 2);
        for i in 0..256 {
            write(3, 0); // 2-bit weight delta
            write(1, 0);
            write(2, u64::from(i == 0));
        }
        let mut encoded = vec![0; bits.len().div_ceil(8)];
        for (i, bit) in bits.into_iter().enumerate() {
            encoded[i / 8] |= bit << (7 - i % 8);
        }
        encoded.extend_from_slice(&[b'A'.reverse_bits(), 0]);
        assert_eq!(huffman(&encoded, 1).unwrap(), b"A");
        assert!(huffman(&encoded, 0).is_err());
        let mut excess = encoded.clone();
        excess.push(0);
        assert!(huffman(&excess, 1).is_err());
        for end in 0..encoded.len() {
            assert!(huffman(&encoded[..end], 1).is_err());
        }
    }
}
