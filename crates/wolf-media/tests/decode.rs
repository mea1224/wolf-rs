use wolf_media::{Clip, Image};

#[test]
fn png_roundtrip_and_truncation_are_checked() {
    let image = Image::new(2, 1, vec![255, 0, 0, 255, 0, 128, 255, 17]).unwrap();
    let mut encoded = Vec::new();
    image.write_png(&mut encoded).unwrap();
    let decoded = Image::from_png(&encoded).unwrap();
    assert_eq!(decoded.dimensions(), (2, 1));
    assert_eq!(decoded.bytes(), image.bytes());
    for end in 0..encoded.len() {
        assert!(
            Image::from_png(&encoded[..end]).is_err(),
            "truncation at {end}"
        );
    }
}
fn wav() -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend(40u32.to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    for n in [1u16, 1] {
        out.extend(n.to_le_bytes());
    }
    out.extend(8000u32.to_le_bytes());
    out.extend(16000u32.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(4u32.to_le_bytes());
    out.extend((-32768i16).to_le_bytes());
    out.extend(32767i16.to_le_bytes());
    out
}
#[test]
fn mono_wav_decodes_resamples_and_rejects_malformed_chunks() {
    let bytes = wav();
    let clip = Clip::from_wav(&bytes).unwrap();
    assert_eq!(clip.sample_rate(), 8000);
    assert_eq!(clip.frames(), &[[-32768; 2], [32767; 2]]);
    assert_eq!(
        clip.resample(16000).unwrap().frames(),
        &[[-32768; 2], [-32768; 2], [32767; 2], [32767; 2]]
    );
    for end in 0..bytes.len() {
        assert!(Clip::from_wav(&bytes[..end]).is_err());
    }
    let mut bad = bytes.clone();
    bad[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Clip::from_wav(&bad).is_err());
    let mut bad = bytes.clone();
    bad[22] = 0;
    assert!(Clip::from_wav(&bad).is_err());
    let mut bad = bytes;
    bad[34] = 24;
    assert!(Clip::from_wav(&bad).is_err());
    assert!(Clip::from_ogg(b"not an ogg stream").is_err());
}
