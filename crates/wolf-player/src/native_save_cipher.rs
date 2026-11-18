//! GamePro 3.50 save format 3 envelope; derived from 5267A0 / 47EA30.
//! This validates the envelope only. Callers must parse the entire snapshot.

pub(crate) struct SaveEnvelope {
    pub identity: String,
    pub version: u16,
    pub snapshot: Vec<u8>,
    pub header: [u8; 20],
}

pub(crate) fn decode(input: &[u8]) -> Result<SaveEnvelope, String> {
    if input.len() < 139 || input.len() > 16 * 1024 * 1024 {
        return Err("save format 3 size outside supported bounds".into());
    }
    if input[8] != 3 || input[6] != 85 {
        return Err("only native save format 3 with UTF-8 strings is supported".into());
    }
    let mut bytes = input.to_vec();
    // 52AC34 / 52CD7A: both offsets depend on the save header, not filename.
    let a = 20 + usize::from(bytes[11]) % 30;
    let b = 80 + usize::from(bytes[14]) % 40;
    for i in 0..20 {
        bytes.swap(a + i, b + i);
    }
    let mut seed = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[5]);
    seed ^= seed << 13;
    seed ^= seed >> 17;
    seed ^= seed << 5;
    let key = mt_key(seed);
    for (i, byte) in bytes.iter_mut().enumerate().skip(20) {
        *byte ^= key[i % 128];
    }
    let expanded = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let compressed = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    if expanded > 16 * 1024 * 1024 || compressed != bytes.len() - 28 {
        return Err("invalid native save compressed sizes".into());
    }
    let decoded = wolf_format::decode_block(&bytes[28..], expanded, 16 * 1024 * 1024)
        .map_err(|e| format!("native save LZ4: {e}"))?;
    if decoded.len() < 6 || decoded[0] != 25 {
        return Err("native save opening marker missing".into());
    }
    let n = i16::from_le_bytes(decoded[1..3].try_into().unwrap());
    if n <= 0 || 5 + n as usize > decoded.len() {
        return Err("invalid native save identity length".into());
    }
    let end = 3 + n as usize;
    if decoded[end - 1] != 0 || decoded[3..end - 1].contains(&0) {
        return Err("native save identity is not one NUL-terminated string".into());
    }
    let identity = std::str::from_utf8(&decoded[3..end - 1])
        .map_err(|e| format!("native save identity UTF-8: {e}"))?
        .to_owned();
    let version = u16::from_le_bytes(decoded[end..end + 2].try_into().unwrap());
    if version > 159 {
        return Err(format!(
            "native save version {version} exceeds engine limit 159"
        ));
    }
    Ok(SaveEnvelope {
        identity,
        version,
        snapshot: decoded[end + 2..].to_vec(),
        header: input[..20].try_into().unwrap(),
    })
}

/// Preserve the original format-3 header and invert the verified reader. A
/// literal-only raw LZ4 block preserves all snapshot bytes, including unknown
/// fields. This writes patched snapshots, not newly synthesized engine saves.
pub(crate) fn encode(envelope: &SaveEnvelope) -> Result<Vec<u8>, String> {
    if envelope.header[8] != 3 || envelope.header[6] != 85 || !matches!(envelope.version, 158 | 159)
    {
        return Err(
            "native save encode requires retained UTF-8 format-3 version-158/159 header".into(),
        );
    }
    let identity_length = envelope
        .identity
        .len()
        .checked_add(1)
        .ok_or("save identity overflow")?;
    if identity_length > i16::MAX as usize || envelope.identity.contains('\0') {
        return Err("native save identity invalid".into());
    }
    let mut plain = vec![25];
    plain.extend_from_slice(&(identity_length as i16).to_le_bytes());
    plain.extend_from_slice(envelope.identity.as_bytes());
    plain.push(0);
    plain.extend_from_slice(&envelope.version.to_le_bytes());
    plain.extend_from_slice(&envelope.snapshot);
    if plain.len() > 16 * 1024 * 1024 - 100_000 {
        return Err("native save encoded size bound".into());
    }
    let mut compressed = Vec::with_capacity(plain.len() + plain.len() / 255 + 2);
    compressed.push((plain.len().min(15) as u8) << 4);
    if plain.len() >= 15 {
        let mut left = plain.len() - 15;
        while left >= 255 {
            compressed.push(255);
            left -= 255;
        }
        compressed.push(left as u8);
    }
    compressed.extend_from_slice(&plain);
    let mut bytes = envelope.header.to_vec();
    bytes.extend_from_slice(&(plain.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&compressed);
    if bytes.len() < 139 {
        return Err("native save encoded payload too short for format-3 swap".into());
    }
    let mut seed = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[5]);
    seed ^= seed << 13;
    seed ^= seed >> 17;
    seed ^= seed << 5;
    let key = mt_key(seed);
    for (i, byte) in bytes.iter_mut().enumerate().skip(20) {
        *byte ^= key[i % 128];
    }
    let a = 20 + usize::from(bytes[11]) % 30;
    let b = 80 + usize::from(bytes[14]) % 40;
    for i in 0..20 {
        bytes.swap(a + i, b + i);
    }
    Ok(bytes)
}

fn mt_key(seed: u32) -> [u8; 128] {
    // 56C160 initialization; 56D030 first twist and temper. Only 128 draws
    // are requested by 48B003, so later engine PRNG state is unnecessary.
    let mut state = [0u32; 624];
    state[0] = seed;
    for i in 1..624 {
        state[i] = 1812433253u32
            .wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30))
            .wrapping_add(i as u32);
    }
    for i in 0..624 {
        let y = (state[i] & 0x80000000) | (state[(i + 1) % 624] & 0x7fffffff);
        state[i] = state[(i + 397) % 624] ^ (y >> 1) ^ if y & 1 != 0 { 0x9908b0df } else { 0 };
    }
    std::array::from_fn(|i| {
        let mut y = state[i];
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;
        y as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_system_save_matches_offline_instruction_interpretation() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let original = std::fs::read(root.join("Save/system.sav")).unwrap();
        let evidence =
            std::fs::read(root.join("wolf-rs/validation/native-save/payload.bin")).unwrap();
        let decoded = decode(&original).unwrap();
        assert_eq!(decoded.identity, "ワンダーランドに帰らないで");
        assert_eq!(decoded.version, 158);
        assert_eq!(decoded.snapshot, evidence[45..]);
        assert!(decode(&original[..100]).is_err());
    }
}
