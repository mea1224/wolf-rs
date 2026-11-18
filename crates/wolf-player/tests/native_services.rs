#![allow(dead_code)]
#[path = "../src/assets.rs"]
mod assets;
#[path = "../src/audio.rs"]
mod audio;
#[path = "../src/native_media.rs"]
mod native_media;
#[path = "../src/native_save_payload.rs"]
mod native_save_payload;
#[path = "../src/native_services.rs"]
mod native_services;
#[path = "../src/native_text.rs"]
mod native_text;
#[path = "../src/native_text_win.rs"]
mod native_text_win;
#[path = "../src/native_video.rs"]
mod native_video;

#[test]
fn save_thumbnail_mount_uses_isolated_directory_and_reloads_written_images() {
    use std::sync::atomic::{AtomicU64,Ordering};
    static NEXT:AtomicU64=AtomicU64::new(0);
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let save=root.join(format!("wolf-rs/validation/thumbnail-test-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
    std::fs::create_dir_all(&save).unwrap();
    let mut services=native_services::Services::new(&root,320,180).unwrap();
    services.set_save_root(&save);
    for color in [[21,63,121,255],[17,90,5,255]] {
        wolf_media::Image::new(1,1,color.to_vec()).unwrap().write_png(std::fs::File::create(save.join("ss1.png")).unwrap()).unwrap();
        assert_eq!(services.image_resource("../Save/ss1.png").unwrap().bytes(),color);
    }
    assert!(services.read_resource("../Save/../../Game.ini").is_err());
    assert!(services.read_resource("../Other/ss1.png").is_err());
    std::fs::remove_file(save.join("ss1.png")).unwrap();
    std::fs::remove_dir(save).unwrap();
}

#[test]
fn services_use_configured_viewport_and_host_tick() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut services = native_services::Services::new(&root, 320, 180).unwrap();
    services.tick().unwrap();
    assert_eq!(services.render().unwrap().dimensions(), (320, 180));
    assert!(!services.vm_blocked());
}

#[test]
fn services_snapshot_embeds_structured_media_and_restores_viewport() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut services = native_services::Services::new(&root, 16, 8).unwrap();
    services.tick().unwrap();
    let bytes = services.save().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["version"], 2);
    assert!(value["media"][1].is_object());
    let mut restored = native_services::Services::restore(&root, &bytes).unwrap();
    assert_eq!(
        services.render().unwrap().bytes(),
        restored.render().unwrap().bytes()
    );
    assert_eq!(restored.render().unwrap().dimensions(), (16, 8));
}

#[test]
fn common89_swing_resolves_common_self_range_through_ir_vm_and_host() {
    use wolf_format::{RawCommand, Span, WolfString};
    let command = |opcode, args: Vec<i32>, text: Option<&str>| RawCommand {
        opcode,
        args,
        indent: 0,
        span: Span { start: 0, end: 0 },
        strings: text
            .into_iter()
            .map(|s| WolfString {
                raw: [s.as_bytes(), &[0]].concat(),
                span: Span { start: 0, end: 0 },
            })
            .collect(),
        extension: vec![],
        extension_length: 0,
        terminator: 0,
        movement: None,
    };
    let raw = vec![
        command(121, vec![1600010, 100, 0, 0], None),
        command(121, vec![1600011, 101, 0, 0], None),
        command(
            150,
            vec![48, 100, 0, 1, 1, 1, 255, 2, 3, 100, 0],
            Some("<GRADY-999-999>"),
        ),
        command(
            150,
            vec![48, 101, 0, 1, 1, 1, 255, 5, 3, 100, 0],
            Some("<GRADY-999-999>"),
        ),
        // Common89 PC496, unchanged numeric form from the current archive.
        command(290, vec![48, 10, 1600010, 1600011, 0, 2, 99999, 0], None),
    ];
    let source = wolf_ir::Source {
        asset: "fixture".into(),
        content_blake3: "fixture".into(),
        event: 89,
        page: None,
        command: 492,
        span: [0, 0],
    };
    let program = wolf_ir::compile("common/89".into(), "guide swing".into(), source, &raw).unwrap();
    let library = wolf_ir::Library::new(vec![program]).unwrap();
    let mut vm = wolf_vm::runtime::Vm::new(library, Default::default(), 1, 1).unwrap();
    vm.spawn(wolf_ir::Target::Id("common/89".into()), 1, vec![])
        .unwrap();
    let mut services = native_services::Services::new(std::path::Path::new("."), 8, 8).unwrap();
    let report = vm
        .tick_with_host(vec![], &mut |ctx| services.execute(ctx))
        .unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    for _ in 0..10 {
        services.tick().unwrap();
    }
    let frame = services.render().unwrap();
    let positions: Vec<_> = frame
        .bytes()
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| p[0] != 0)
        .map(|(i, _)| (i % 8, i / 8))
        .collect();
    assert_eq!(positions, vec![(2, 5), (5, 5)]);
    assert_eq!(vm.state().fibers[&1].wait, wolf_vm::runtime::Wait::Done);
}

#[test]
fn real_archive_picture_executes_through_vm_context_and_services() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    if !root.join("Data/BasicData.wolf").exists() {
        eprintln!("private game fixture absent");
        return;
    }
    let bytes = assets::read_member(&root, "BasicData/CommonEvent.dat").unwrap();
    let bytes = bytes
        .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
        .unwrap_or(&bytes);
    let parsed = wolf_format::parse_common(bytes, &Default::default()).unwrap();
    let event = parsed.value.events.iter().find(|e| e.id == 23).unwrap();
    let raw = &event.commands[50];
    assert_eq!(raw.opcode, 150);
    assert_eq!(raw.strings[0].text().unwrap(), "Picture/krachware.png");
    let program = wolf_ir::compile(
        "service-real-picture".into(),
        "real Common23 PC50".into(),
        wolf_ir::Source {
            asset: "BasicData/CommonEvent.dat".into(),
            content_blake3: "private-fixture".into(),
            event: 23,
            page: None,
            command: 50,
            span: [0, 0],
        },
        std::slice::from_ref(raw),
    )
    .unwrap();
    let mut vm = wolf_vm::runtime::Vm::new(
        wolf_ir::Library::new(vec![program]).unwrap(),
        Default::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(
        wolf_ir::Target::Id("service-real-picture".into()),
        0,
        vec![],
    )
    .unwrap();
    let mut services = native_services::Services::new(&root, 1280, 720).unwrap();
    let report = vm
        .tick_with_host(vec![], &mut |context| {
            context.set_number(1600010, 10000)?;
            services.execute(context)
        })
        .unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    for _ in 0..20 {
        services.tick().unwrap();
    }
    let reference =
        wolf_media::Image::from_png(&assets::read_member(&root, "Picture/krachware.png").unwrap())
            .unwrap();
    let rendered = services.render().unwrap();
    assert_eq!(rendered.bytes(), reference.bytes());
    let output = root.join("wolf-rs/validation/native-media/service-real-picture.png");
    rendered
        .write_png(std::fs::File::create(output).unwrap())
        .unwrap();
}

#[test]
fn original_intro_block_routes_audio_and_picture_commands_without_skipping() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    if !root.join("Data/BasicData.wolf").exists() {
        eprintln!("private game fixture absent");
        return;
    }
    let bytes = assets::read_member(&root, "BasicData/CommonEvent.dat").unwrap();
    let bytes = bytes
        .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
        .unwrap_or(&bytes);
    let parsed = wolf_format::parse_common(bytes, &Default::default()).unwrap();
    let event = parsed.value.events.iter().find(|e| e.id == 23).unwrap();
    let program = wolf_ir::compile(
        "original-intro-block".into(),
        "original Common23 PC45..55".into(),
        wolf_ir::Source {
            asset: "BasicData/CommonEvent.dat".into(),
            content_blake3: "private-fixture".into(),
            event: 23,
            page: None,
            command: 45,
            span: [0, 0],
        },
        &event.commands[45..56],
    )
    .unwrap();
    let mut vm = wolf_vm::runtime::Vm::new(
        wolf_ir::Library::new(vec![program]).unwrap(),
        Default::default(),
        1,
        1,
    )
    .unwrap();
    vm.spawn(
        wolf_ir::Target::Id("original-intro-block".into()),
        1,
        vec![],
    )
    .unwrap();
    let mut services = native_services::Services::new(&root, 1280, 720).unwrap();
    let mut executed = Vec::new();
    // Real platform constructor 961551/547/53D sets masters to100.
    vm.set_system(102, 100).unwrap();
    for _ in 0..1000 {
        let report = vm
            .tick_with_host(vec![], &mut |context| {
                let result = services.execute(context)?;
                if result {
                    executed.push(context.instruction.raw.as_ref().unwrap().opcode);
                }
                Ok(result)
            })
            .unwrap();
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        services.tick().unwrap();
    }
    assert_eq!(executed, vec![150, 140, 150, 150, 150]);
    let reference =
        wolf_media::Image::from_png(&assets::read_member(&root, "Picture/guideline.png").unwrap())
            .unwrap();
    assert_eq!(services.render().unwrap().bytes(), reference.bytes());
}
#[path = "../src/native_audio.rs"]
mod native_audio;
