use std::{hint::black_box, sync::Arc, time::Instant};
use wolf_media::{Image, Picture, Pictures};

// Frozen pre-optimization implementation: an independent byte-for-byte oracle
// for clipping, nearest sampling, layer ordering, and straight alpha rounding.
fn reference(layers: &[(u32, Picture)], target: &mut Image) {
    let (tw, th) = target.dimensions();
    let mut ordered: Vec<_> = layers.iter().collect();
    ordered.sort_by_key(|(id, p)| (p.z, *id));
    for (_, p) in ordered {
        let (sw, sh) = p.image.dimensions();
        let left = i64::from(p.x).clamp(0, i64::from(tw));
        let top = i64::from(p.y).clamp(0, i64::from(th));
        let right = (i64::from(p.x) + i64::from(p.width)).clamp(0, i64::from(tw));
        let bottom = (i64::from(p.y) + i64::from(p.height)).clamp(0, i64::from(th));
        for y in top..bottom {
            for x in left..right {
                let sx = (x - i64::from(p.x)) as usize * sw as usize / p.width as usize;
                let sy = (y - i64::from(p.y)) as usize * sh as usize / p.height as usize;
                let source = &p.image.bytes()[(sy * sw as usize + sx) * 4..][..4];
                let dest =
                    &mut target.bytes_mut()[(y as usize * tw as usize + x as usize) * 4..][..4];
                let alpha = (u32::from(source[3]) * u32::from(p.opacity) + 127) / 255;
                let da = u32::from(dest[3]);
                let out = alpha * 255 + da * (255 - alpha);
                for channel in 0..3 {
                    dest[channel] = if out == 0 {
                        0
                    } else {
                        ((u32::from(source[channel]) * alpha * 255
                            + u32::from(dest[channel]) * da * (255 - alpha)
                            + out / 2)
                            / out) as u8
                    };
                }
                dest[3] = ((out + 127) / 255) as u8;
            }
        }
    }
}

fn pictures(layers: &[(u32, Picture)]) -> Pictures {
    let mut result = Pictures::default();
    for (id, p) in layers {
        result
            .show(
                *id,
                Picture {
                    image: p.image.clone(),
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                    opacity: p.opacity,
                    z: p.z,
                },
            )
            .unwrap();
    }
    result
}

#[test]
fn every_source_and_destination_alpha_matches_original_integer_formula() {
    let mut source = Vec::with_capacity(256 * 256 * 4);
    let mut dest = Vec::with_capacity(source.capacity());
    for da in 0..=255u8 {
        for sa in 0..=255u8 {
            source.extend_from_slice(&[sa.wrapping_mul(37), da.wrapping_mul(71), 255, sa]);
            dest.extend_from_slice(&[da.wrapping_mul(13), 255, sa.wrapping_mul(29), da]);
        }
    }
    let source = Arc::new(Image::new(256, 256, source).unwrap());
    for opacity in [0, 1, 2, 127, 128, 254, 255] {
        let layers = [(
            1,
            Picture {
                image: source.clone(),
                x: 0,
                y: 0,
                width: 256,
                height: 256,
                opacity,
                z: 0,
            },
        )];
        let mut expected = Image::new(256, 256, dest.clone()).unwrap();
        let mut actual = Image::new(256, 256, dest.clone()).unwrap();
        reference(&layers, &mut expected);
        pictures(&layers).render(&mut actual);
        assert_eq!(actual.bytes(), expected.bytes(), "opacity={opacity}");
    }
}

#[test]
fn clipped_opaque_rows_scaled_samples_and_stable_order_match_reference() {
    let mut opaque = Image::new(
        9,
        7,
        (0..63).flat_map(|p| [p, 251 - p, p * 3, 255]).collect(),
    )
    .unwrap();
    // Mutation must be visible to the opacity test; no stale cached metadata.
    opaque.bytes_mut()[3] = 0;
    opaque.bytes_mut()[4 * 13 + 3] = 127;
    let mixed = Arc::new(opaque);
    let solid = Arc::new(Image::new(9, 7, vec![255; 9 * 7 * 4]).unwrap());
    for (width, height) in [(9, 7), (9, 11), (16, 11), (4, 3)] {
        for (x, y) in [
            (-12, -9),
            (-3, -2),
            (0, 0),
            (5, 4),
            (14, 11),
            (i32::MAX, i32::MIN),
        ] {
            let layers = [
                (
                    8,
                    Picture {
                        image: mixed.clone(),
                        x,
                        y,
                        width,
                        height,
                        opacity: 255,
                        z: 1,
                    },
                ),
                (
                    2,
                    Picture {
                        image: solid.clone(),
                        x: 1,
                        y: 1,
                        width: 9,
                        height: 7,
                        opacity: 255,
                        z: 1,
                    },
                ),
                (
                    12,
                    Picture {
                        image: mixed.clone(),
                        x: 3,
                        y: -1,
                        width: 4,
                        height: 10,
                        opacity: 127,
                        z: 0,
                    },
                ),
            ];
            let initial: Vec<_> = (0..17 * 13)
                .flat_map(|p| [p as u8, 29, 211, if p % 2 == 0 { 255 } else { 0 }])
                .collect();
            let mut expected = Image::new(17, 13, initial.clone()).unwrap();
            let mut actual = Image::new(17, 13, initial).unwrap();
            reference(&layers, &mut expected);
            pictures(&layers).render(&mut actual);
            assert_eq!(
                actual.bytes(),
                expected.bytes(),
                "rect=({x},{y},{width},{height})"
            );
        }
    }
}

#[test]
#[ignore = "manual release benchmark; timing is informational, byte equality is asserted"]
fn compare_720p_composition_cost() {
    let (width, height) = (1280usize, 720usize);
    let source = Arc::new(
        Image::new(
            width,
            height,
            (0..width * height)
                .flat_map(|p| [p as u8, (p / 17) as u8, 89, 255])
                .collect(),
        )
        .unwrap(),
    );
    for opacity in [255, 137] {
        let layers = [(
            1,
            Picture {
                image: source.clone(),
                x: 0,
                y: 0,
                width: width as u32,
                height: height as u32,
                opacity,
                z: 0,
            },
        )];
        let optimized = pictures(&layers);
        let initial = vec![255; width * height * 4];
        let mut expected = Image::new(width, height, initial.clone()).unwrap();
        let mut actual = Image::new(width, height, initial).unwrap();
        let start = Instant::now();
        for _ in 0..30 {
            reference(black_box(&layers), black_box(&mut expected));
        }
        let original = start.elapsed();
        let start = Instant::now();
        for _ in 0..30 {
            optimized.render(black_box(&mut actual));
        }
        let improved = start.elapsed();
        assert_eq!(actual.bytes(), expected.bytes());
        println!("720p opacity {opacity}, 30 frames: original={original:?}, optimized={improved:?}, speedup={:.2}x",
            original.as_secs_f64()/improved.as_secs_f64());
    }
}
