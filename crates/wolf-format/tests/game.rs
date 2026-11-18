use wolf_format::{parse_game, Limits};

#[test]
fn real_game_config_and_truncation() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../docs/evidence/data_copies/BasicData/Game.dat");
    let bytes = std::fs::read(path).expect("local Game.dat evidence fixture");
    let config = parse_game(&bytes, &Limits::default()).unwrap();
    assert_eq!(config.title.text().unwrap(), "ワンダーランドに帰らないで");
    assert_eq!(config.configuration.len(), 43);
    assert_eq!(config.opaque_tail_span.start, 484);
    assert_eq!(config.opaque_tail.len(), 28_935);
    assert_eq!(
        config.default_character_graphic.text().unwrap(),
        "CharaChip/chiex2.png"
    );
    for end in [0, 9, 14, 100, 483, bytes.len() - 1] {
        assert!(parse_game(&bytes[..end], &Limits::default()).is_err());
    }
    let mut notice = b"Extracting data from encrypted files violates the guidelines.\0".to_vec();
    notice.extend_from_slice(&bytes);
    assert_eq!(
        parse_game(&notice, &Limits::default())
            .unwrap()
            .opaque_tail_span,
        config.opaque_tail_span
    );
    let mut bad = bytes;
    bad[config.title.span.start + 4] = 0xff;
    let raw = parse_game(&bad, &Limits::default()).unwrap();
    assert!(raw.title.text().is_err());
    assert_eq!(raw.title.raw[0], 0xff);
    bad[0] = 1;
    assert!(parse_game(&bad, &Limits::default()).is_err());
}
