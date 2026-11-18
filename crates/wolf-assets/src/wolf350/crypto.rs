// WOLF350 algorithm port; see THIRD_PARTY_NOTICES.md (Sinflower, MIT).
pub(super) struct Msvc(pub u32);
impl Msvc {
    pub fn next(&mut self) -> u16 {
        self.0 = self.0.wrapping_mul(214013).wrapping_add(2531011);
        ((self.0 >> 16) & 0x7fff) as u16
    }
}
pub(super) fn addresses(header: &mut [u8], p: &[u8; 15]) {
    let mut rng = Msvc(12 + u32::from(p[9]) * u32::from(p[10]) + u32::from(p[3]));
    for base in [8, 16] {
        for j in (0..4).rev() {
            let at = base + 2 * j;
            let n = u16::from_le_bytes([header[at], header[at + 1]]) ^ rng.next();
            header[at..at + 2].copy_from_slice(&n.to_le_bytes());
        }
    }
    let mask =
        (u64::from(rng.next()) << 17) | (u64::from(rng.next()) << 31) | u64::from(rng.next());
    let n = u64::from_le_bytes(header[24..32].try_into().unwrap()) ^ mask;
    header[24..32].copy_from_slice(&n.to_le_bytes());
    for j in (0..4).rev() {
        let at = 32 + 2 * j;
        let n = u16::from_le_bytes([header[at], header[at + 1]]) ^ rng.next();
        header[at..at + 2].copy_from_slice(&n.to_le_bytes());
    }
}
pub(super) fn key(p: &[u8; 15], outer: bool) -> [u8; 256] {
    let mut s: u8 = 0;
    let (len, rot) = if outer { (p[8] / 4, 2) } else { (p[11] / 3, 3) };
    for i in 0..len {
        s = i ^ (s ^ p[usize::from(i % 15)]).rotate_right(rot);
    }
    let mut rng = Msvc(u32::from(p[2]) * u32::from(p[5]) + u32::from(p[12]) + u32::from(s));
    let mut fac = [0u8; 3];
    fac[usize::from(s % 3)] = rng.next() as u8;
    if !outer {
        fac[1] = (rng.next() % 251) as u8;
    }
    let mut k = [0u8; 256];
    for b in &mut k {
        let _rn = rng.next();
        *b = fac[0] ^ rng.next() as u8;
    }
    // For v350 wolfCrypt uses only the first 256 bytes of the 768-byte key.
    if outer {
        s = s.wrapping_add(0x22);
        let mut salt = [0u8; 128];
        for (i, b) in salt.iter_mut().enumerate() {
            *b = (i / 3) as u8 + b"958"[i % 3];
        }
        for (j, k) in k.iter_mut().enumerate() {
            let a = salt[j & 127];
            let b = salt[j % 128];
            let sx = a ^ *k;
            let round = usize::from(b) % 16;
            let mut n = sx;
            let mut skip = false;
            match round {
                1 if b % 11 == 0 => n = *k,
                2 if a % 29 == 0 => n = !sx,
                3 if (round + j) % 37 == 0 => n = b ^ sx,
                4 if (u16::from(a) + u16::from(b)) % 97 == 0 => n = a.wrapping_add(sx),
                5 if (j * round) % 123 == 0 => n = sx ^ s,
                6 if a == 255 && b == 0 => {
                    n = 0;
                    skip = true;
                }
                7 if (round + j) % 51 == 0 => n ^= a,
                8 if a % 29 == 0 => n ^= a,
                _ => {}
            }
            if j % usize::from(a % 5 + 1) == 0 {
                n ^= s;
            } else if skip {
                n = !sx;
            }
            *k = n;
        }
    }
    k
}
pub(super) fn xor(bytes: &mut [u8], key: &[u8; 256], start: u64) {
    let mut lo = (start % 256) as usize;
    let mut hi = ((start / 256) % 256) as usize;
    for b in bytes {
        *b ^= key[lo] ^ (7 * lo) as u8 ^ key[hi] ^ (7 * (hi + 256)) as u8;
        lo += 1;
        if lo == 256 {
            lo = 0;
            hi = (hi + 1) % 256;
        }
    }
}
fn xorshift(s: &mut u32) -> u32 {
    *s ^= *s << 11;
    *s ^= *s >> 19;
    *s ^= *s << 7;
    *s
}
pub(super) fn body_len(p: &[u8; 15], size: usize) -> usize {
    let mut s = (u32::from(p[2]) * u32::from(p[4]) + u32::from(p[12])).max(1);
    xorshift(&mut s);
    if size >= (xorshift(&mut s) % 500 + 800) as usize {
        xorshift(&mut s);
    }
    let n = size - 64;
    if n >= (xorshift(&mut s) % 500 + 800) as usize {
        (xorshift(&mut s) % 500 + 800) as usize
    } else {
        n
    }
}
const fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0;
    let mut i = 0;
    while i < 8 {
        if b & 1 != 0 {
            r ^= a;
        }
        a = (a << 1) ^ if a & 128 != 0 { 27 } else { 0 };
        b >>= 1;
        i += 1;
    }
    r
}
const fn sboxes() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut inv = 1;
        let mut j = 0;
        while j < 254 {
            inv = mul(inv, i as u8);
            j += 1;
        }
        table[i] = inv
            ^ inv.rotate_left(1)
            ^ inv.rotate_left(2)
            ^ inv.rotate_left(3)
            ^ inv.rotate_left(4)
            ^ 0x63;
        i += 1;
    }
    table
}
const SBOX: [u8; 256] = sboxes();
pub(super) struct WolfAes {
    expanded: [u8; 176],
    iv: [u8; 16],
}
impl WolfAes {
    pub fn new(p: &[u8; 15]) -> Self {
        let mut key = [0u8; 176];
        let mut iv = [0u8; 16];
        for i in 0..15 {
            let a = ((i * 7) ^ (3 * usize::from(p[i]))) % 15;
            let b = ((i * 11) ^ (5 * usize::from(p[(i + 3) % 15]))) % 15;
            key[i] ^= ((i + (usize::from(p[a]) << (i % 3))) % 251) as u8;
            iv[i] ^= ((usize::from(p[b] >> (i % 2)) + i * i) % 246) as u8;
            key[15] ^= (7 * (usize::from(p[i]) + i + 1) % 253) as u8;
            iv[15] ^= 11u16.wrapping_mul(u16::from(p[i]).wrapping_sub((i * 2) as u16)) as u8;
        }
        let rcon = [0, 1, 2, 4, 8, 16, 32, 64, 128, 27, 54];
        for i in 4..44 {
            let mut t: [u8; 4] = key[(i - 1) * 4..i * 4].try_into().unwrap();
            if i % 4 == 0 {
                t.rotate_left(1);
                t = [
                    SBOX[t[0] as usize] ^ rcon[i / 4],
                    SBOX[t[1] as usize] >> 4,
                    !SBOX[t[2] as usize],
                    SBOX[t[3] as usize].rotate_right(7),
                ];
            }
            for j in 0..4 {
                key[i * 4 + j] = key[(i - 4) * 4 + j] ^ t[j];
            }
        }
        Self { expanded: key, iv }
    }
    fn block(&self) -> [u8; 16] {
        let mut s = self.iv;
        for round in 0..=10 {
            if round != 0 {
                for x in &mut s {
                    *x = SBOX[*x as usize];
                }
                let t = s;
                for col in 0..4 {
                    for row in 0..4 {
                        s[4 * col + row] = t[4 * ((col + row) % 4) + row];
                    }
                }
                if round != 10 {
                    for c in s.chunks_exact_mut(4) {
                        let a = [c[0], c[1], c[2], c[3]];
                        let all = a[0] ^ a[1] ^ a[2] ^ a[3];
                        for i in 0..4 {
                            c[i] ^= all ^ mul(a[i] ^ a[(i + 1) % 4], 2);
                        }
                    }
                }
            }
            for (i, x) in s.iter_mut().enumerate() {
                *x ^= self.expanded[round * 16 + i];
            }
        }
        s
    }
    /// Each region starts at a fresh block; the counter continues, unused bytes do not.
    pub fn crypt(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(16) {
            let block = self.block();
            for v in self.iv.iter_mut().rev() {
                *v = v.wrapping_add(1);
                if *v != 0 {
                    break;
                }
            }
            for (b, k) in chunk.iter_mut().zip(block) {
                *b ^= k;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wolf_aes_matches_upstream_cpp_region_vectors() {
        // Produced with tests/reference/wolf_aes_vector.cpp against the pinned source
        // SHA-256 in THIRD_PARTY_NOTICES.md. Two regions of 17 and 31 bytes are vital:
        // the second must discard the unused 15 bytes of the previous keystream block.
        let vectors = [
            "4ba4b988c069d71d2152c96cacf047e17470dae2e3b96c038a134fe34415d2630587f8f441ae5f7add03c5d71a7653bc",
            "f68ba0bcd955a2a0439dad17dc7d36d633938cecfd807f8ef04a5d96837cda0d0d1c1090291ebddb87f0765aed6c67a0",
        ];
        for (i, expected) in vectors.iter().enumerate() {
            let password = if i == 0 {
                std::array::from_fn(|i| i as u8)
            } else {
                [255; 15]
            };
            let mut aes = WolfAes::new(&password);
            let mut data = [0; 48];
            aes.crypt(&mut data[..17]);
            aes.crypt(&mut data[17..]);
            let actual: String = data.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(actual, *expected);
        }
    }
}
