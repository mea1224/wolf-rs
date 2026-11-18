#[path = "../src/native_media.rs"]
#[allow(dead_code)] // This focused harness includes the production module without all host consumers.
mod native_media;

use native_media::{MediaOutcome, NativeMedia};
use std::sync::Arc;
use wolf_media::Image;

#[test]
fn horizontal_picture_creation_selects_one_based_cell() {
    let mut media = NativeMedia::new(1, 1).unwrap();
    let sheet =
        Arc::new(Image::new(3, 1, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]).unwrap());
    let command = [0, 1001, 0, 3, 1, 2, 255, 0, 0, 100, 0];
    show(&mut media, &command, sheet);
    assert_eq!(media.picture_dimensions(1001), Some((1, 1)));
    assert_eq!(media.render().unwrap().bytes(), &[0, 255, 0, 255]);
    let mut restored = NativeMedia::restore_state(&media.capture_state().unwrap()).unwrap();
    assert_eq!(restored.render().unwrap().bytes(), &[0, 255, 0, 255]);
    // Common12 PC103 starts a 1..3 loop with 20 updates per frame.
    let animate = [144, 20, 1001, 1001, 1, 3, 0, 0];
    media
        .execute(290, &animate, &animate, "", |_| unreachable!())
        .unwrap();
    assert_eq!(media.render().unwrap().bytes(), &[255, 0, 0, 255]);
    for color in [[0, 255, 0, 255], [0, 0, 255, 255], [255, 0, 0, 255]] {
        for _ in 0..20 {
            media.tick(1).unwrap();
        }
        assert_eq!(media.render().unwrap().bytes(), &color);
    }
}

#[test]
fn selector_zero_color_overlay_fades_out_on_native_ticks() {
    let mut media = NativeMedia::new(1, 1).unwrap();
    show(&mut media, &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0], image([100, 100, 100, 255], 1, 1));
    // CommonEvent 9 PC303: selector0, duration20, picture ID range, RGB80/60/40.
    let effect = [0, 20, 1, 1, 80, 60, 40, 0];
    media.execute(290, &effect, &effect, "", |_| unreachable!()).unwrap();
    assert_eq!(media.render().unwrap().bytes(), &[180, 160, 140, 255]);
    for _ in 0..10 { media.tick(1).unwrap(); }
    assert_eq!(media.render().unwrap().bytes(), &[140, 130, 120, 255]);
    for _ in 0..10 { media.tick(1).unwrap(); }
    assert_eq!(media.render().unwrap().bytes(), &[100, 100, 100, 255]);
}

#[test]
fn frame_animation_attaches_to_delayed_atlas_and_survives_activation() {
    let mut media = NativeMedia::new(1, 1).unwrap();
    let delayed = [
        0, 1, 0, 3, 1, 1, 255, 0, 0, 100, 0, 0, 33554432, 3, 0, 100, 100, 100,
    ];
    let sheet = Arc::new(Image::new(3, 1, vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255,
    ]).unwrap());
    show(&mut media, &delayed, sheet);
    let animation = [144, 20, 1, 1, 1, 3, 0, 0];
    media.execute(290, &animation, &animation, "", |_| unreachable!()).unwrap();
    for _ in 0..4 { media.tick(1).unwrap(); }
    assert_eq!(media.render().unwrap().bytes(), &[255, 0, 0, 255]);
    for _ in 0..20 { media.tick(1).unwrap(); }
    assert_eq!(media.render().unwrap().bytes(), &[0, 255, 0, 255]);
}

#[test]
fn screenshot_picture_captures_scene_and_layers_without_resource_loading() {
    let mut media = NativeMedia::new(2, 1).unwrap();
    media
        .set_scene_image(image([255, 0, 0, 255], 2, 1))
        .unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 1, 0, 100, 0],
        image([0, 255, 0, 255], 1, 1),
    );
    let expected = media.render_scene().unwrap();
    let capture = [0, 2, 0, 1, 1, 1, 255, 0, 0, 100, 0];
    media
        .execute(150, &capture, &capture, "<SCREENSHOT>", |_| {
            panic!("screenshot is not a resource file")
        })
        .unwrap();
    media
        .set_scene_image(image([0, 0, 255, 255], 2, 1))
        .unwrap();
    let delete = [2, 1, 0];
    media
        .execute(150, &delete, &delete, "", |_| unreachable!())
        .unwrap();
    assert_eq!(media.render().unwrap().bytes(), expected.bytes());
    let mut restored = NativeMedia::restore_state(&media.capture_state().unwrap()).unwrap();
    assert_eq!(restored.render().unwrap().bytes(), expected.bytes());
}

#[test]
fn delete_ignores_map_relative_creation_origin_flag() {
    let mut media = NativeMedia::new(2, 1).unwrap();
    show(
        &mut media,
        &[0, 12003, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([255, 0, 0, 255], 1, 1),
    );
    // Common37 PC182: A1AC84 dispatches by the low operation nibble (2).
    // The retained bit25 origin/create variant does not change deletion.
    let delete = [0x02006002, 12003, 0];
    media
        .execute(150, &delete, &delete, "", |_| unreachable!())
        .unwrap();
    assert!(!media.picture_exists(12003));
}

#[test]
fn timing_configuration_rejects_other_native_steps() {
    native_media::validate_timing_configuration(&[0, 0, 0, 0, 60]).unwrap();
    assert!(native_media::validate_timing_configuration(&[]).is_err());
    for byte in [0, 30, 120, 255] {
        assert!(native_media::validate_timing_configuration(&[0, 0, 0, 0, byte]).is_err());
    }
}

fn image(color: [u8; 4], width: usize, height: usize) -> Arc<Image> {
    Arc::new(Image::new(width, height, color.repeat(width * height)).unwrap())
}

#[test]
fn signed_background_layers_compose_with_scene_and_transition_capture() {
    let mut media = NativeMedia::new(3, 1).unwrap();
    let background = [0, -300003, 0, 1, 1, 1, 255, 0, 0, 100, 0];
    show(&mut media, &background, image([255, 0, 0, 255], 3, 1));
    let scene =
        Arc::new(Image::new(3, 1, vec![0, 0, 0, 0, 0, 255, 0, 255, 0, 255, 0, 255]).unwrap());
    media.set_scene_image(scene.clone()).unwrap();
    assert_eq!(media.picture_position(-300003).unwrap(), Some([0, 0]));
    assert_eq!(
        media.render_scene().unwrap().bytes(),
        &[255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255]
    );
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 2, 0, 100, 0],
        image([0, 0, 255, 255], 1, 1),
    );
    let captured = media.render_scene().unwrap();
    assert_eq!(
        captured.bytes(),
        &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]
    );
    let configure = [-1, 65536, 4];
    media
        .execute(160, &configure, &configure, "", |_| unreachable!())
        .unwrap();
    media
        .execute(161, &[], &[], "", |_| unreachable!())
        .unwrap();
    media
        .set_scene_image(image([255, 255, 255, 255], 3, 1))
        .unwrap();
    media
        .execute(162, &[], &[], "", |_| unreachable!())
        .unwrap();
    let mut restored = NativeMedia::restore_state(&media.capture_state().unwrap()).unwrap();
    restored
        .set_scene_image(image([255, 255, 255, 255], 3, 1))
        .unwrap();
    assert_eq!(restored.render().unwrap().bytes(), captured.bytes());
    for _ in 0..5 {
        media.tick(1).unwrap();
        restored.tick(1).unwrap();
        assert_eq!(
            media.render().unwrap().bytes(),
            restored.render().unwrap().bytes()
        );
    }
    let mut unsupported = background;
    unsupported[1] = -1;
    assert!(
        media
            .execute(150, &unsupported, &unsupported, "pic.png", |_| Ok(image(
                [0; 4], 1, 1
            )))
            .unwrap_err()
            .contains("interleaved")
    );
    assert!(!media.picture_exists(-1));
}

#[test]
fn map_relative_create_converts_camera_once_and_relative_replacement_changes_space() {
    let mut media = NativeMedia::new(4, 4).unwrap();
    media.set_camera_offset([10, 20]);
    let mut command = [0x02001100, -300003, 0, 1, 1, 1, 255, 2, 2, 100, 0];
    show(&mut media, &command, image([255, 0, 0, 255], 1, 1));
    assert_eq!(media.picture_position(-300003).unwrap(), Some([2, 2]));
    media.set_camera_offset([11, 21]);
    assert_eq!(media.picture_position(-300003).unwrap(), Some([1, 1]));
    assert_eq!(colored_positions(&media, 0), vec![(1, 1)]);
    command[0] |= 0x10000;
    command[7] = 1;
    command[8] = 0;
    show(&mut media, &command, image([255, 0, 0, 255], 1, 1));
    assert_eq!(media.picture_position(-300003).unwrap(), Some([2, 1]));
    command[0] &= !0x02000000;
    show(&mut media, &command, image([255, 0, 0, 255], 1, 1));
    assert_eq!(media.picture_position(-300003).unwrap(), Some([3, 1]));
    media.set_camera_offset([100, 200]);
    assert_eq!(media.picture_position(-300003).unwrap(), Some([3, 1]));
}

#[test]
fn saved_horizontal_animation_restores_phase_and_replays_exactly() {
    let evidence: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../validation/native-save-restore/auto-pictures.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut fields: std::collections::BTreeMap<usize, i32> =
        serde_json::from_value(evidence[1]["fields"].clone()).unwrap();
    for (offset, value) in [
        (456, 0),
        (472, 0),
        (504, 0),
        (768, 0),
        (496, 255),
        (512, 0),
        (544, 100),
        (560, 100),
        (1020, 0),
        (1060, 2),
        (1064, 3),
    ] {
        fields.insert(offset, value);
    }
    let sheet =
        Arc::new(Image::new(3, 1, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]).unwrap());
    let mut media = NativeMedia::new(4, 2).unwrap();
    media
        .restore_native_picture(&fields, b"strip.png", &[], |_| Ok(sheet.clone()))
        .unwrap();
    assert_eq!(media.picture_dimensions(fields[&772]), Some((1, 1)));
    assert_eq!(
        &media.render_scene().unwrap().bytes()[..4],
        &[0, 255, 0, 255]
    );
    media.tick(1).unwrap();
    let mut restored = NativeMedia::restore_state(&media.capture_state().unwrap()).unwrap();
    assert_eq!(restored.viewport(), (4, 2));
    for channel in [2, 2, 2, 0, 0, 0, 1] {
        media.tick(1).unwrap();
        restored.tick(1).unwrap();
        let original = media.render_scene().unwrap();
        assert_eq!(original.bytes()[channel], 255);
        assert_eq!(original.bytes(), restored.render_scene().unwrap().bytes());
    }
    let mut invalid: serde_json::Value =
        serde_json::from_slice(&media.capture_state().unwrap()).unwrap();
    let id = fields[&772].to_string();
    invalid[1]["pictures"][&id]["frames"]["remaining"] = 0.into();
    assert!(NativeMedia::restore_state(&serde_json::to_vec(&invalid).unwrap()).is_err());
}

#[test]
fn audited_auto_save_restores_all_nontext_pictures_and_effects() {
    let validation = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../validation");
    let evidence: serde_json::Value = serde_json::from_slice(
        &std::fs::read(validation.join("native-save-restore/auto-pictures.json")).unwrap(),
    )
    .unwrap();
    let mut media = NativeMedia::new(1280, 720).unwrap();
    let mut restored_count = 0;
    for picture in evidence.as_array().unwrap() {
        let fields: std::collections::BTreeMap<usize, i32> =
            serde_json::from_value(picture["fields"].clone()).unwrap();
        if fields[&764] == 1 {
            continue;
        }
        let source = picture["source"].as_str().unwrap();
        media
            .restore_native_picture(&fields, source.as_bytes(), &[], |path| {
                let bytes = std::fs::read(validation.join("archive-reference").join(path))
                    .map_err(|e| e.to_string())?;
                wolf_media::Image::from_png(&bytes).map(Arc::new)
            })
            .unwrap_or_else(|error| panic!("saved picture {} {source}: {error}", fields[&772]));
        restored_count += 1;
    }
    assert_eq!(restored_count, 12);
    for _ in 0..3 {
        media.tick(1).unwrap();
        media.render_scene().unwrap();
    }
}

#[test]
fn saved_map_relative_picture_queries_and_render_follow_camera_offset() {
    let evidence: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../validation/native-save-restore/auto-pictures.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut fields: std::collections::BTreeMap<usize, i32> =
        serde_json::from_value(evidence[0]["fields"].clone()).unwrap();
    for (offset, value) in [(456, 12), (472, 8), (504, 0), (766, 1), (1020, 0)] {
        fields.insert(offset, value);
    }
    let mut media = NativeMedia::new(8, 8).unwrap();
    media.set_camera_offset([10, 5]);
    media
        .restore_native_picture(&fields, b"pixel.png", &[], |_| {
            Ok(image([255, 0, 0, 255], 1, 1))
        })
        .unwrap();
    assert_eq!(media.picture_position(1000).unwrap(), Some([2, 3]));
    assert_eq!(colored_positions(&media, 0), vec![(2, 3)]);
    let mut restored = NativeMedia::restore_state(&media.capture_state().unwrap()).unwrap();
    assert_eq!(restored.camera_offset(), [10, 5]);
    restored.set_camera_offset([11, 6]);
    assert_eq!(restored.picture_position(1000).unwrap(), Some([1, 2]));
    assert_eq!(colored_positions(&restored, 0), vec![(1, 2)]);
}

#[test]
fn snapshot_replays_mask_delayed_commands_and_tween_and_rejects_bad_effects() {
    let mut media = NativeMedia::new(8, 8).unwrap();
    show(
        &mut media,
        &[0, 1, 4, 1, 1, 1, 255, 0, 0, 100, 0],
        image([80, 40, 20, 255], 4, 4),
    );
    for command in [[112, 2, 1, 1, 50, 50, 1, 0], [16, 4, 1, 1, 40, -40, 0, 0]] {
        media
            .execute(290, &command, &command, "", |_| unreachable!())
            .unwrap();
    }
    let movement = [
        1, 1, 2, 0, 0, 1, 255, 2, 2, 100, 0, 0, 0x02000000, 3, 0, 100, 100, 100,
    ];
    media
        .execute(150, &movement, &movement, "", |_| unreachable!())
        .unwrap();
    let fill = [67, 0, 0, 0, 0, 0, 0];
    media
        .execute(290, &fill, &fill, "", |_| unreachable!())
        .unwrap();
    let mask = [83, 0, 0, 0, 0, 0, 0];
    media
        .execute(290, &mask, &mask, "mask.png", |_| {
            Ok(image([0, 0, 0, 255], 3, 3))
        })
        .unwrap();
    let enable = [19, 0, 1, 1, 0, 0, 0];
    media
        .execute(290, &enable, &enable, "", |_| unreachable!())
        .unwrap();
    media.tick(1).unwrap();
    let bytes = media.capture_state().unwrap();
    let mut restored = NativeMedia::restore_state(&bytes).unwrap();
    for _ in 0..12 {
        assert_eq!(
            media.render_scene().unwrap().bytes(),
            restored.render_scene().unwrap().bytes()
        );
        media.tick(1).unwrap();
        restored.tick(1).unwrap();
    }
    let mut bad: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    bad[1]["pictures"]["1"]["scale_pulse"]["duration"] = i32::MAX.into();
    assert!(NativeMedia::restore_state(&serde_json::to_vec(&bad).unwrap()).is_err());
    bad[0] = 999.into();
    assert!(NativeMedia::restore_state(&serde_json::to_vec(&bad).unwrap()).is_err());
}

#[test]
fn additive_and_multiply_pictures_apply_both_color_passes() {
    for (blend, expected) in [(1, [160, 140, 150]), (3, [30, 16, 20])] {
        let mut media = NativeMedia::new(2, 2).unwrap();
        show(
            &mut media,
            &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
            image([100, 100, 100, 255], 1, 1),
        );
        show(
            &mut media,
            &[blend << 8, 2, 0, 1, 1, 1, 255, 0, 0, 100, 0],
            image([50, 50, 50, 255], 1, 1),
        );
        let color = [16, 0, 2, 2, 20, -20, 0, 0];
        media
            .execute(290, &color, &color, "", |_| unreachable!())
            .unwrap();
        assert_eq!(&media.render_scene().unwrap().bytes()[..3], &expected);
    }
}

fn show(media: &mut NativeMedia, args: &[i32], value: Arc<Image>) {
    assert_eq!(
        media
            .execute(150, args, args, "Picture/arbitrary.png", |_| Ok(
                value.clone()
            ))
            .unwrap(),
        MediaOutcome::Applied
    );
}

fn colored_positions(media: &NativeMedia, channel: usize) -> Vec<(u32, u32)> {
    let frame = media.render_scene().unwrap();
    let width = frame.dimensions().0;
    frame
        .bytes()
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| pixel[channel] != 0)
        .map(|(i, _)| (i as u32 % width, i as u32 / width))
        .collect()
}

#[test]
fn dialog_signed_axes_delay_and_scale_preservation() {
    let mut media = NativeMedia::new(12, 12).unwrap();
    let asymmetric = Arc::new(Image::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap());
    // Common18 PC15: mode3 reads independent X/Y, with a mirrored X start.
    let create = [
        0x301000, 1, 0, 1, 1, 1, 255, 6, 6, -100, 0, 0, 0x02000000, 0, 0, 100, 100, 100, 100,
    ];
    show(&mut media, &create, asymmetric);
    assert_eq!(colored_positions(&media, 0), vec![(6, 6)]);
    assert_eq!(colored_positions(&media, 1), vec![(5, 6)]);
    let expand = [
        0x301001, 1, 0, 0, 0, 1, 255, 6, 6, 200, 0, 0, 0x02000000, 2, 0, 100, 100, 100, 200,
    ];
    media
        .execute(150, &expand, &expand, "", |_| unreachable!())
        .unwrap();
    media.tick(1).unwrap();
    media.tick(1).unwrap();
    media.tick(1).unwrap();
    assert_eq!(colored_positions(&media, 0).len(), 4);
    assert_eq!(colored_positions(&media, 1).len(), 4);
    // Mode4/rotation sentinel preserve both scales and existing rotation.
    let preserve = [0x401001, 1, 0, 0, 0, 1, 255, 6, 6, -1_000_000, -1_000_000];
    media
        .execute(150, &preserve, &preserve, "", |_| unreachable!())
        .unwrap();
    assert_eq!(colored_positions(&media, 0).len(), 4);
}

#[test]
fn map_particle_move_preserves_opacity_target_during_fade() {
    let mut media = NativeMedia::new(12, 12).unwrap();
    show(
        &mut media,
        &[0x1000, 1, 0, 1, 1, 1, 0, 6, 6, 100, 0],
        image([255, 0, 0, 255], 1, 1),
    );
    let fade = [0x1001, 1, 10, 0, 0, 1, 255, 6, 6, 100, 0];
    media
        .execute(150, &fade, &fade, "", |_| unreachable!())
        .unwrap();
    media.tick(5).unwrap();
    // Common38 PC69 keeps opacity/scale while applying a relative move.
    let movement = [4263681, 1, 10, 0, 0, 1, -1_000_000, 1, 0, -1_000_000, 0];
    media
        .execute(150, &movement, &movement, "", |_| unreachable!())
        .unwrap();
    media.tick(10).unwrap();
    let pixels = media.render().unwrap();
    assert!(
        pixels
            .bytes()
            .chunks_exact(4)
            .any(|pixel| pixel == [255, 0, 0, 255])
    );
}

#[test]
fn picture_rotation_is_in_tenths_of_a_degree() {
    let mut media = NativeMedia::new(12, 12).unwrap();
    show(
        &mut media,
        &[0x1000, 1, 0, 1, 1, 1, 255, 6, 6, 100, 900],
        image([255, 0, 0, 255], 4, 2),
    );
    let points = colored_positions(&media, 0);
    assert_eq!(points.len(), 8);
    assert_eq!(points.iter().map(|p| p.0).min(), Some(5));
    assert_eq!(points.iter().map(|p| p.0).max(), Some(6));
    assert_eq!(points.iter().map(|p| p.1).min(), Some(4));
    assert_eq!(points.iter().map(|p| p.1).max(), Some(7));
}

#[test]
fn settings_color_transition_retargets_and_resource_replacement_keeps_effects() {
    let mut media = NativeMedia::new(16, 8).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 1, 1, 100, 0],
        image([50, 50, 50, 255], 2, 2),
    );
    let effect = [16, 4, 1, 1, 40, 0, 0, 0];
    media
        .execute(290, &effect, &effect, "", |_| unreachable!())
        .unwrap();
    media.tick(1).unwrap();
    media.tick(1).unwrap();
    let sample =
        |media: &NativeMedia, x: usize| media.render_scene().unwrap().bytes()[(16 + x) * 4];
    assert_eq!(sample(&media, 1), 60);
    // Replacing a resource with relative position preserves its in-flight color state.
    show(
        &mut media,
        &[0x10000, 1, 0, 1, 1, 1, 255, 3, 0, 100, 0],
        image([50, 50, 50, 255], 2, 2),
    );
    assert_eq!(sample(&media, 4), 60);
    let fade = [16, 2, 1, 1, 0, 0, 0, 0];
    media
        .execute(290, &fade, &fade, "", |_| unreachable!())
        .unwrap();
    assert_eq!(sample(&media, 4), 60);
    media.tick(1).unwrap();
    assert_eq!(sample(&media, 4), 55);
    media.tick(1).unwrap();
    assert_eq!(sample(&media, 4), 50);
}

#[test]
fn scale_pulse_repeats_after_cooldown_and_zero_duration_disables() {
    let mut media = NativeMedia::new(16, 8).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([255, 0, 0, 255], 4, 1),
    );
    let effect = [112, 2, 1, 1, 100, 0, 2, 0];
    media
        .execute(290, &effect, &effect, "", |_| unreachable!())
        .unwrap();
    for expected in [4, 6, 8, 6, 4, 4, 4, 6] {
        assert_eq!(colored_positions(&media, 0).len(), expected);
        media.tick(1).unwrap();
    }
    let stop = [112, 0, 1, 1, 100, 0, 0, 0];
    media
        .execute(290, &stop, &stop, "", |_| unreachable!())
        .unwrap();
    assert_eq!(colored_positions(&media, 0).len(), 4);
}

#[test]
fn cached_render_matches_fresh_composition_through_effects_and_delayed_commands() {
    let mut media = NativeMedia::new(16, 8).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([200, 100, 50, 255], 4, 2),
    );
    // Pulse has a genuinely static cooldown. A delayed color operation must
    // invalidate that cached frame even when it completes in a single tick.
    for effect in [
        [112, 2, 1, 1, 100, 0, 4, 0],
        [16, 0, 1, 1, -50, 0, 0, 5],
        [48, 2, 1, 1, 1, 1, 2, 0],
    ] {
        media.execute(290, &effect, &effect, "", |_| unreachable!()).unwrap();
    }
    for _ in 0..32 {
        let cached = media.render().unwrap();
        let mut fresh = NativeMedia::restore_state(&media.capture_state().unwrap()).unwrap();
        assert_eq!(cached.bytes(), fresh.render().unwrap().bytes());
        media.tick(1).unwrap();
    }
}

#[test]
fn screen_mask_uses_blue_and_range_state_survives_replacement() {
    let mut media = NativeMedia::new(4, 2).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([255, 0, 0, 255], 4, 2),
    );
    let fill = [67, 0, 0, 0, 0, 0, 0];
    media
        .execute(290, &fill, &fill, "", |_| unreachable!())
        .unwrap();
    let source = Arc::new(Image::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap());
    let load = [83, 0, 0, 0, 1, 0, 0];
    media
        .execute(290, &load, &load, "mask.png", |_| Ok(source.clone()))
        .unwrap();
    let enable = [19, 0, 1, 1, 0, 0, 0];
    media
        .execute(
            290,
            &enable,
            &enable,
            "retained-unused-string",
            |_| unreachable!(),
        )
        .unwrap();
    assert_eq!(colored_positions(&media, 0), vec![(1, 0)]);
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([0, 255, 0, 255], 4, 2),
    );
    assert_eq!(colored_positions(&media, 1), vec![(1, 0)]);
    let reset = [3, 0, 0, 0, 0, 0, 0];
    media
        .execute(290, &reset, &reset, "", |_| unreachable!())
        .unwrap();
    assert_eq!(colored_positions(&media, 1).len(), 8);
    let option = wolf_media::Image::from_png(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../validation/archive-reference/SystemGraphic/option_mask.png"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        option
            .bytes()
            .chunks_exact(4)
            .all(|p| matches!(p[2], 0 | 255))
    );
}

#[test]
fn queue_clear_range_preserves_visible_state_and_cancels_delayed_commands() {
    let mut media = NativeMedia::new(8, 8).unwrap();
    for id in 1..=3 {
        show(
            &mut media,
            &[0, id, 0, 1, 1, 1, 255, id, 1, 100, 0],
            image([255, 0, 0, 255], 1, 1),
        );
        let movement = [
            1, id, 0, 0, 0, 1, 255, id, 5, 100, 0, 0, 0x02000000, 3, 0, 100, 100, 100,
        ];
        media
            .execute(150, &movement, &movement, "", |_| unreachable!())
            .unwrap();
    }
    // Common19 PC289 retains text-content/origin bits when clearing a range.
    let clear = [0x1001023, 2, 1];
    media
        .execute(150, &clear, &clear, "", |_| unreachable!())
        .unwrap();
    for _ in 0..4 {
        media.tick(1).unwrap();
    }
    assert_eq!(colored_positions(&media, 0), vec![(1, 1), (2, 1), (3, 5)]);
}

#[test]
fn delayed_replacement_keeps_original_until_due_and_delete_ignores_origin_variant() {
    let mut media = NativeMedia::new(8, 4).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([255, 0, 0, 255], 1, 1),
    );
    let replace = [
        0, 1, 0, 1, 1, 1, 255, 2, 0, 100, 0, 0, 0x02000000, 2, 0, 100, 100, 100,
    ];
    show(&mut media, &replace, image([0, 255, 0, 255], 1, 1));
    for _ in 0..2 {
        media.tick(1).unwrap();
        assert_eq!(colored_positions(&media, 0), vec![(0, 0)]);
        assert!(colored_positions(&media, 1).is_empty());
    }
    media.tick(1).unwrap();
    assert_eq!(colored_positions(&media, 1), vec![(2, 0)]);
    let delete = [0x1001012, 1, 2, 0, 0, 1];
    media
        .execute(150, &delete, &delete, "", |_| unreachable!())
        .unwrap();
    media.tick(2).unwrap();
    assert!(!media.picture_exists(1));
}

#[test]
fn swing_uses_native_triangle_rounding_direction_count_and_range() {
    let mut media = NativeMedia::new(16, 16).unwrap();
    for (id, channel) in [(10, 0), (11, 1), (12, 2)] {
        let mut color = [0, 0, 0, 255];
        color[channel] = 255;
        show(
            &mut media,
            &[0, id, 0, 1, 1, 1, 255, 6, 6, 100, 0],
            image(color, 1, 1),
        );
    }
    // Reversed inclusive IDs affect 10 and 11. Even N first travels negative.
    let effect = [48, 3, 11, 10, 2, -2, 2, 0];
    media
        .execute(290, &effect, &effect, "", |_| unreachable!())
        .unwrap();
    // 99213B truncates the remainder before subtracting it from amplitude;
    // it differs from truncating amplitude * elapsed / duration on the way out.
    let positions = [
        (6, 6),
        (5, 7),
        (4, 8),
        (4, 8),
        (5, 7),
        (6, 6),
        (6, 6),
        (7, 5),
        (8, 4),
        (8, 4),
        (7, 5),
        (6, 6),
        (6, 6),
    ];
    for (i, expected) in positions.into_iter().enumerate() {
        if i != 0 {
            media.tick(1).unwrap();
        }
        // ID11 covers ID10 wherever their swing coincides.
        let green = colored_positions(&media, 1);
        if expected == (6, 6) {
            assert!(green.is_empty());
        } else {
            assert_eq!(green, vec![expected], "host update {i}");
        }
        assert_eq!(colored_positions(&media, 2), vec![(6, 6)]);
    }
    media.tick(1).unwrap();
    assert!(colored_positions(&media, 1).is_empty()); // finite N=2 has finished
}

#[test]
fn swing_threshold_is_infinite_and_stop_resets_without_changing_base_geometry() {
    let mut media = NativeMedia::new(12, 12).unwrap();
    show(
        &mut media,
        &[0, 10, 0, 1, 1, 1, 255, 5, 5, 100, 0],
        image([255, 0, 0, 255], 1, 1),
    );
    let effect = [48, 0, 10, 10, 2, 0, 100_000]; // T clamps to1
    media
        .execute(290, &effect, &effect, "", |_| unreachable!())
        .unwrap();
    for expected in [3, 5, 7, 5, 3, 5, 7, 5] {
        media.tick(7).unwrap(); // native effect updates once, not seven times
        assert_eq!(colored_positions(&media, 0), vec![(expected, 5)]);
    }
    media.tick(0).unwrap(); // effect counter is independent of interpolation step
    assert_eq!(colored_positions(&media, 0), vec![(3, 5)]);
    let stop = [48, 10, 10, 10, 0, 2, 0, 0];
    media
        .execute(290, &stop, &stop, "", |_| unreachable!())
        .unwrap();
    assert_eq!(colored_positions(&media, 0), vec![(5, 5)]);
}

#[test]
fn unsupported_effect_is_rejected_before_mutation_and_absent_ranges_are_empty() {
    let mut media = NativeMedia::new(8, 8).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 3, 3, 100, 0],
        image([255, 0, 0, 255], 1, 1),
    );
    let effect = [48, 2, 1, 1, 2, 0, 3, 0];
    media
        .execute(290, &effect, &effect, "", |_| unreachable!())
        .unwrap();
    media.tick(1).unwrap();
    let before = media.render_scene().unwrap().bytes().to_vec();
    for (index, value) in [(0, 0x60), (1, 1_000_001), (4, i32::MAX), (7, 1_000_001)] {
        let mut bad = effect;
        bad[index] = value;
        assert!(
            media
                .execute(290, &bad, &bad, "", |_| unreachable!())
                .is_err()
        );
        assert_eq!(media.render_scene().unwrap().bytes(), before);
    }
    for (first, last) in [(-10, -1), (10, 20)] {
        let mut absent = effect;
        absent[2] = first;
        absent[3] = last;
        media
            .execute(290, &absent, &absent, "", |_| unreachable!())
            .unwrap();
        assert_eq!(media.render_scene().unwrap().bytes(), before);
    }
    let moved = [1, 1, 0, 1, 1, 1, 255, 4, 3, 100, 0];
    media
        .execute(150, &moved, &moved, "", |_| unreachable!())
        .unwrap();
    assert_eq!(colored_positions(&media, 0), vec![(5, 3)]); // move retains phase
    let replacement = [0, 1, 0, 1, 1, 1, 255, 4, 3, 100, 0];
    media
        .execute(
            150,
            &replacement,
            &replacement,
            "Picture/replacement.png",
            |_| Ok(image([255, 0, 0, 255], 1, 1)),
        )
        .unwrap();
    assert_eq!(colored_positions(&media, 0), vec![(5, 3)]);
}

#[test]
fn common14_generated_picture_commands_preserve_colors_and_layer_order() {
    let mut media = NativeMedia::new(1280, 720).unwrap();
    show(
        &mut media,
        &[0, 10, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([200, 100, 50, 255], 1280, 720),
    );
    // Common14 PC6: actual raw fields; only CommonSelf14 is resolved here.
    let raw = [816, 999998, 0, 1280, 720, 1, 1600014, 0, 0, 100, 0];
    let mut args = raw;
    args[6] = 255;
    media
        .execute(150, &raw, &args, "<GRADY-555-555>", |_| unreachable!())
        .unwrap();
    assert_eq!(&media.render().unwrap().bytes()[..4], &[114, 57, 28, 255]);
    // Common14 PC10 adds the second gradient after the multiply layer.
    let raw = [304, 999999, 0, 1280, 720, 1, 1600014, 0, 0, 100, 0];
    let mut args = raw;
    args[6] = 255;
    media
        .execute(150, &raw, &args, "<GRADY-000-725>", |_| unreachable!())
        .unwrap();
    let frame = media.render().unwrap();
    assert_eq!(&frame.bytes()[..4], &[114, 57, 28, 255]);
    assert_eq!(&frame.bytes()[360 * 1280 * 4..][..4], &[215, 86, 100, 255]);
    assert_eq!(&frame.bytes()[719 * 1280 * 4..][..4], &[255, 114, 172, 255]);
    assert_eq!(media.picture_path(999998), Some("<GRADY-555-555>"));
}

#[test]
fn native_picture_fades_on_host_ticks_and_erases_after_duration() {
    let mut media = NativeMedia::new(4, 2).unwrap();
    let args = [0, 10000, 20, 1, 1, 1, 255, 0, 0, 100, 0];
    show(&mut media, &args, image([255, 255, 255, 255], 4, 2));
    assert_eq!(media.render().unwrap().bytes()[0], 0);
    media.tick(10).unwrap();
    assert_eq!(media.render().unwrap().bytes()[0], 128);
    media.tick(10).unwrap();
    assert_eq!(media.render().unwrap().bytes()[0], 255);
    let erase = [2, 10000, 20];
    media
        .execute(150, &erase, &erase, "", |_| unreachable!())
        .unwrap();
    media.tick(10).unwrap();
    assert_eq!(media.picture_count(), 1);
    assert_eq!(media.render().unwrap().bytes()[0], 127);
    media.tick(10).unwrap();
    assert_eq!(media.picture_count(), 0);
    assert_eq!(media.render().unwrap().bytes()[0], 0);
}

#[test]
fn resolved_id_and_center_anchor_drive_order_and_pixels() {
    let mut media = NativeMedia::new(6, 4).unwrap();
    let raw = [4096, 1600010, 0, 1, 1, 1, 255, 3, 2, 100, 0];
    let mut resolved = raw;
    resolved[1] = 10000;
    media
        .execute(150, &raw, &resolved, "Picture/custom.png", |_| {
            Ok(image([220, 30, 10, 255], 2, 2))
        })
        .unwrap();
    assert_eq!(media.picture_path(10000), Some("Picture/custom.png"));
    assert_eq!(media.picture_path(1600010), None);
    let frame = media.render().unwrap();
    assert_eq!(&frame.bytes()[(6 + 2) * 4..][..4], &[220, 30, 10, 255]);
    assert_eq!(&frame.bytes()[(6 + 1) * 4..][..4], &[0, 0, 0, 255]);
    // Replacing a decoded movie frame preserves command geometry.
    media
        .replace_frame(10000, image([1, 2, 3, 255], 2, 2))
        .unwrap();
    assert_eq!(
        &media.render().unwrap().bytes()[(6 + 2) * 4..][..4],
        &[1, 2, 3, 255]
    );
}

#[test]
fn captured_transition_is_independent_and_retries_until_draw_completion() {
    let mut media = NativeMedia::new(2, 2).unwrap();
    let args = [0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0];
    show(&mut media, &args, image([255, 0, 0, 255], 2, 2));
    let configure = [-1, 65536, 4];
    media
        .execute(160, &configure, &configure, "", |_| unreachable!())
        .unwrap();
    media
        .execute(161, &[], &[], "", |_| unreachable!())
        .unwrap();
    show(&mut media, &args, image([0, 0, 255, 255], 2, 2));
    media
        .execute(162, &[], &[], "", |_| unreachable!())
        .unwrap();
    assert!(media.vm_blocked());
    assert_eq!(&media.render().unwrap().bytes()[..4], &[255, 0, 0, 255]);
    assert_eq!(
        media
            .execute(161, &[], &[], "", |_| unreachable!())
            .unwrap(),
        MediaOutcome::Retry
    );
    media.tick(1).unwrap(); // timer 3; draw timer 2, not 3
    assert_eq!(&media.render().unwrap().bytes()[..4], &[127, 0, 128, 255]);
    media.tick(3).unwrap();
    assert!(media.transition_active()); // cleared by native draw stage
    assert_eq!(&media.render().unwrap().bytes()[..4], &[0, 0, 255, 255]);
    assert!(!media.vm_blocked());
}

#[test]
fn unsupported_command_does_not_mutate_visible_state() {
    let mut media = NativeMedia::new(2, 2).unwrap();
    let args = [0, 2, 0, 1, 1, 1, 255, 0, 0, 100, 0];
    show(&mut media, &args, image([17, 18, 19, 255], 2, 2));
    let before = media.render().unwrap().bytes().to_vec();
    let mut unsupported = args;
    unsupported[0] = 0x40; // unsupported content mode fails before calling loader
    assert!(
        media
            .execute(150, &unsupported, &unsupported, "text", |_| unreachable!())
            .is_err()
    );
    assert!(
        media
            .execute(160, &[2, 0, 30], &[2, 0, 30], "", |_| unreachable!())
            .is_err()
    );
    assert_eq!(media.render().unwrap().bytes(), before);
    assert_eq!(media.dimensions(), (2, 2));
    assert!(native_media::native_loop_name("Picture/title_loop.mp4"));
    assert!(!native_media::native_loop_name("Picture/title_Loop.mp4"));
}

#[test]
fn text_picture_uses_prepared_layout_and_ignores_image_division_fields() {
    let mut media = NativeMedia::new(4, 2).unwrap();
    let args = [32, 20, 0, 0, 0, 1, 255, 0, 0, 100, 0];
    let text = "\\>\\f[22]OK";
    media
        .execute(150, &args, &args, text, |supplied| {
            assert_eq!(supplied, text);
            Ok(image([90, 80, 70, 255], 4, 2))
        })
        .unwrap();
    assert_eq!(
        media.render().unwrap().bytes(),
        &[90, 80, 70, 255].repeat(8)
    );
    assert_eq!(media.picture_dimensions(20), Some((4, 2)));
}

#[test]
fn common89_window_skin_and_delayed_range_move_keep_both_layers() {
    let mut media = NativeMedia::new(20, 20).unwrap();
    let mut pixels = Vec::new();
    for row in 0..3 {
        for col in 0..3 {
            pixels.extend_from_slice(&[10 + row * 30 + col * 10, 0, 0, 255]);
        }
    }
    let skin = Arc::new(Image::new(3, 3, pixels).unwrap());
    let window = [48, 16000, 0, 7, 5, 1, 255, 0, 0, 100, 0];
    media
        .execute(150, &window, &window, "SystemGraphic/window2.png", |_| {
            Ok(skin.clone())
        })
        .unwrap();
    let frame = media.render().unwrap();
    let at = |x: usize, y: usize| frame.bytes()[(y * 20 + x) * 4];
    assert_eq!(
        [at(0, 0), at(6, 0), at(0, 4), at(6, 4), at(3, 2)],
        [10, 30, 70, 90, 50]
    );
    assert_eq!(media.picture_dimensions(16000), Some((7, 5)));
    show(
        &mut media,
        &[0, 16001, 0, 1, 1, 1, 0, 10, 15, 100, 0],
        image([255, 255, 255, 255], 1, 1),
    );
    let first = [
        16846593, 16001, 10, 0, 0, 1, 150, 0, -3, 100, 0, 0, 33554432, 0, 16000, 100, 100, 100,
    ];
    media
        .execute(150, &first, &first, "", |_| unreachable!())
        .unwrap();
    let second = [
        16846593, 16001, 10, 0, 0, 1, 255, 0, -1, 100, 0, 0, 16777216, 10, 16000,
    ];
    media
        .execute(150, &second, &second, "", |_| unreachable!())
        .unwrap();
    assert_eq!(media.picture_count(), 2);
    media.tick(10).unwrap();
    assert_eq!(media.render().unwrap().bytes()[(12 * 20 + 10) * 4], 150);
    media.tick(1).unwrap(); // native checks delay before subtraction
    media.tick(9).unwrap();
    assert_eq!(media.render().unwrap().bytes()[(11 * 20 + 10) * 4], 255);
}

#[test]
fn real_archive_intro_frames_are_not_preselected_by_the_host() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reference = root.join("validation/archive-reference/Picture");
    if !reference.is_dir() {
        eprintln!(
            "private native media archive fixture unavailable: {}",
            reference.display()
        );
        return;
    }
    let mut media = NativeMedia::new(1280, 720).unwrap();
    let args = [0, 10000, 20, 1, 1, 1, 255, 0, 0, 100, 0];
    // Arbitrary order deliberately differs from the game script.
    for name in ["guideline.png", "krachware.png", "kuro.png"] {
        let decoded =
            Arc::new(Image::from_png(&std::fs::read(reference.join(name)).unwrap()).unwrap());
        media
            .execute(150, &args, &args, &format!("Picture/{name}"), |_| {
                Ok(decoded.clone())
            })
            .unwrap();
        media.tick(20).unwrap();
        assert_eq!(media.render().unwrap().bytes(), decoded.bytes());
    }
}

#[test]
fn delayed_create_and_relative_moves_keep_insertion_order_and_origin() {
    let mut media = NativeMedia::new(10, 10).unwrap();
    let create = [
        4096, 100, 0, 1, 1, 1, 255, 5, 5, 100, 0, 0, 33554432, 2, 0, 100, 100, 100,
    ];
    let movement = [
        65537, 100, 0, 0, 0, 1, 255, 2, -2, 100, 0, 0, 33554432, 2, 0, 100, 100, 100,
    ];
    show(&mut media, &create, image([255, 1, 2, 255], 2, 2));
    media
        .execute(150, &movement, &movement, "", |_| unreachable!())
        .unwrap();
    media.tick(2).unwrap();
    assert_eq!(media.picture_count(), 1); // native placeholder exists before resource creation
    assert!(media.picture_exists(100));
    assert_eq!(media.picture_dimensions(100), Some((0, 0)));
    assert!(media.picture_path(100).is_none());
    media.tick(1).unwrap();
    let frame = media.render().unwrap();
    assert_eq!(media.picture_count(), 1);
    // Center(5,5) + (2,-2), then subtract origin(1,1) -> top-left(6,2).
    assert_eq!(&frame.bytes()[(2 * 10 + 6) * 4..][..4], &[255, 1, 2, 255]);
    assert_eq!(&frame.bytes()[(2 * 10 + 5) * 4..][..4], &[0, 0, 0, 255]);
}

#[test]
fn delayed_range_delete_selects_existing_ids_and_uses_predecrement_delay() {
    let mut media = NativeMedia::new(4, 4).unwrap();
    for id in [10, 12, 14] {
        show(
            &mut media,
            &[0, id, 0, 1, 1, 1, 255, 0, 0, 100, 0],
            image([255, 0, 0, 255], 1, 1),
        );
    }
    // Common89 shape, reversed inclusive endpoints and ignored relative bit.
    let delete = [0x0101_0002, 12, 0, 30, 0, 10];
    media
        .execute(150, &delete, &delete, "", |_| unreachable!())
        .unwrap();
    show(
        &mut media,
        &[0, 11, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([255, 0, 0, 255], 1, 1),
    );
    media.tick(30).unwrap();
    assert!(media.picture_path(10).is_some());
    assert!(media.picture_path(12).is_some());
    media.tick(1).unwrap();
    assert!(media.picture_path(10).is_none());
    assert!(media.picture_path(12).is_none());
    assert!(media.picture_path(11).is_some());
    assert!(media.picture_path(14).is_some());
}

#[test]
fn repeat_delete_does_not_restart_and_queue_retains_deleted_object() {
    let mut media = NativeMedia::new(1, 1).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([255, 0, 0, 255], 1, 1),
    );
    for command in [&[2, 1, 0, 10][..], &[2, 1, 4][..]] {
        media
            .execute(150, command, command, "", |_| unreachable!())
            .unwrap();
    }
    media.tick(2).unwrap();
    media
        .execute(150, &[2, 1, 4], &[2, 1, 4], "", |_| unreachable!())
        .unwrap();
    media.tick(2).unwrap();
    assert!(media.picture_path(1).is_none());
    assert_eq!(media.picture_count(), 1); // deleted native object still owns a queue
    assert_eq!(media.render().unwrap().bytes(), &[0, 0, 0, 255]);
    media.tick(6).unwrap();
    media.tick(1).unwrap();
    assert_eq!(media.picture_count(), 0);
}

#[test]
fn title_type9_zero_duration_preserves_base_picture() {
    let mut media = NativeMedia::new(1, 1).unwrap();
    show(
        &mut media,
        &[0, 10020, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([17, 33, 55, 255], 1, 1),
    );
    let disable = [144, 0, 10020, 10020, 1, 8, 0, 0];
    media
        .execute(290, &disable, &disable, "", |_| unreachable!())
        .unwrap();
    media.tick(8).unwrap();
    assert_eq!(media.render().unwrap().bytes(), &[17, 33, 55, 255]);
    let mut unsupported = disable;
    unsupported[1] = 1;
    assert!(
        media
            .execute(290, &unsupported, &unsupported, "", |_| unreachable!())
            .is_err()
    );
}

#[test]
fn particle_move_preserves_rotation_and_additive_blend_then_deletes() {
    let mut media = NativeMedia::new(8, 8).unwrap();
    show(
        &mut media,
        &[256, 10100, 0, 1, 1, 1, 255, 2, 6, 100, 0],
        image([80, 30, 10, 255], 1, 1),
    );
    let movement = [69377, 10100, 2, 0, 0, 1, 255, 1, -4, 100, -1000000];
    media
        .execute(150, &movement, &movement, "", |_| unreachable!())
        .unwrap();
    let delete = [2, 10100, 0, 2];
    media
        .execute(150, &delete, &delete, "", |_| unreachable!())
        .unwrap();
    media.tick(2).unwrap();
    assert_eq!(colored_positions(&media, 0), vec![(3, 2)]);
    media.tick(1).unwrap();
    assert!(colored_positions(&media, 0).is_empty());
}

#[test]
fn completed_delete_keeps_due_commands_bound_to_the_deleted_object() {
    let mut media = NativeMedia::new(1, 1).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([255, 0, 0, 255], 1, 1),
    );
    media
        .execute(150, &[2, 1, 0, 1], &[2, 1, 0, 1], "", |_| unreachable!())
        .unwrap();
    let movement = [1, 1, 0, 0, 0, 1, 255, 0, 0, 100, 0, 0, 16777216, 1, 0];
    media
        .execute(150, &movement, &movement, "", |_| unreachable!())
        .unwrap();
    media.tick(1).unwrap();
    // The unsupported revival is diagnosed against the existing tombstone,
    // rather than having silently collected/rebound the native object early.
    assert!(
        media
            .tick(1)
            .unwrap_err()
            .contains("move of deleted picture")
    );
}

#[test]
fn menu_effects_keep_base_geometry_and_native_integer_retarget() {
    let mut media = NativeMedia::new(20, 20).unwrap();
    show(
        &mut media,
        &[4096, 1, 0, 1, 1, 1, 255, 10, 10, 100, 0],
        image([100, 0, 0, 255], 4, 4),
    );
    let scale = [192, 2, 1, 1, 200, 100, 0];
    media
        .execute(290, &scale, &scale, "", |_| unreachable!())
        .unwrap();
    media.tick(1).unwrap();
    let points = colored_positions(&media, 0);
    assert_eq!(
        (points.len(), points[0], *points.last().unwrap()),
        (24, (7, 8), (12, 11))
    );
    let shift = [32, 3, 1, 1, 10, 0, 0];
    media
        .execute(290, &shift, &shift, "", |_| unreachable!())
        .unwrap();
    media.tick(1).unwrap();
    let reset = [32, 2, 1, 1, 0, 0, 0];
    media
        .execute(290, &reset, &reset, "", |_| unreachable!())
        .unwrap();
    // Integer snapshot is 10 - trunc(10*2/3) = 4, unlike trunc(draw)=3.
    assert_eq!(colored_positions(&media, 0)[0], (10, 8));
    media.tick(1).unwrap();
    assert_eq!(colored_positions(&media, 0)[0], (8, 8));
}

#[test]
fn menu_color_pulse_uses_texture_color_and_repeats() {
    let mut media = NativeMedia::new(1, 1).unwrap();
    show(
        &mut media,
        &[0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0],
        image([100, 40, 0, 255], 1, 1),
    );
    let pulse = [80, 20, 1, 1, 50, 50, 50];
    media
        .execute(290, &pulse, &pulse, "", |_| unreachable!())
        .unwrap();
    assert_eq!(&media.render().unwrap().bytes()[..3], &[150, 60, 0]);
    for _ in 0..20 {
        media.tick(1).unwrap();
    }
    assert_eq!(&media.render().unwrap().bytes()[..3], &[100, 40, 0]);
    for _ in 0..20 {
        media.tick(1).unwrap();
    }
    assert_eq!(&media.render().unwrap().bytes()[..3], &[150, 60, 0]);
}

#[test]
fn delayed_create_receives_effects_and_keeps_their_elapsed_phase() {
    let mut media = NativeMedia::new(10, 10).unwrap();
    let create = [
        4096, 1, 0, 1, 1, 1, 255, 5, 5, 100, 0, 0, 33554432, 3, 0, 100, 100, 100,
    ];
    show(&mut media, &create, image([100, 0, 0, 255], 2, 2));
    let scale = [192, 2, 1, 1, 200, 100, 0];
    let pulse = [80, 4, 1, 1, 50, 50, 50];
    for effect in [&scale, &pulse] {
        media
            .execute(290, effect, effect, "", |_| unreachable!())
            .unwrap();
    }
    for _ in 0..3 {
        media.tick(1).unwrap();
    }
    assert!(colored_positions(&media, 0).is_empty());
    media.tick(1).unwrap();
    let points = colored_positions(&media, 0);
    assert_eq!(
        (points.len(), points[0], *points.last().unwrap()),
        (8, (3, 4), (6, 5))
    );
    assert_eq!(media.render().unwrap().bytes()[(4 * 10 + 3) * 4], 100); // pulse already reached phase4
}

#[test]
fn mask_transition_uses_red_channel_threshold_and_soft_edge() {
    let mut media = NativeMedia::new(3, 1).unwrap();
    let picture = [0, 1, 0, 1, 1, 1, 255, 0, 0, 100, 0];
    show(&mut media, &picture, image([255, 0, 0, 255], 3, 1));
    let mask = Arc::new(
        Image::new(
            3,
            1,
            vec![0, 255, 200, 255, 128, 0, 255, 255, 255, 100, 0, 255],
        )
        .unwrap(),
    );
    let configure = [2, 65536, 4];
    media
        .execute(
            160,
            &configure,
            &configure,
            "SystemGraphic/mask.png",
            |_| Ok(mask.clone()),
        )
        .unwrap();
    media
        .execute(161, &[], &[], "", |_| unreachable!())
        .unwrap();
    show(&mut media, &picture, image([0, 0, 255, 255], 3, 1));
    media
        .execute(162, &[], &[], "", |_| unreachable!())
        .unwrap();
    assert_eq!(media.render().unwrap().bytes(), &[255, 0, 0, 255].repeat(3));
    media.tick(1).unwrap();
    assert_eq!(
        media.render().unwrap().bytes(),
        &[0, 0, 255, 255, 124, 0, 131, 255, 255, 0, 0, 255]
    );
    media.tick(3).unwrap();
    assert_eq!(media.render().unwrap().bytes(), &[0, 0, 255, 255].repeat(3));
    assert!(!media.vm_blocked());
}
