//! Exact byte-space kernels. SIMD changes throughput, not alpha/rounding rules.

pub(super) fn row(dst: &mut [u8], src: &[u8], opacity: u32, mode: u32, tint: [i32; 3]) {
    debug_assert_eq!(dst.len(), src.len());
    if tint.iter().any(|&v| v > 255) { return scalar(dst, src, opacity, mode, tint); }
    #[cfg(target_arch = "x86_64")]
    {
        // SSE2 is required by x86-64. Each call handles bounded complete pixels.
        unsafe {
            if std::arch::is_x86_feature_detected!("avx2") {
                avx2::row(dst, src, opacity, mode, tint);
            } else {
                x86::row(dst, src, opacity, mode, tint);
            }
        };
    }
    #[cfg(not(target_arch = "x86_64"))]
    scalar(dst, src, opacity, mode, tint);
}

fn scalar(dst: &mut [u8], src: &[u8], opacity: u32, mode: u32, tint: [i32; 3]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let a = (u32::from(s[3]) * opacity + 127) / 255;
        for c in 0..3 {
            d[c] = super::tinted_channel(d[c], s[c], a, mode, tint[c]);
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use std::arch::x86_64::*;

    #[inline]
    unsafe fn rounded_255(v: __m128i) -> __m128i {
        unsafe {
            let v = _mm_add_epi16(v, _mm_set1_epi16(128));
            _mm_srli_epi16::<8>(_mm_add_epi16(v, _mm_srli_epi16::<8>(v)))
        }
    }

    #[inline]
    unsafe fn half<const MODE: u32, const TINT: bool, const POSITIVE: bool>(s: __m128i, d: __m128i, opacity: __m128i, negative: __m128i, positive: __m128i) -> __m128i {
        unsafe {
            let a = _mm_shufflehi_epi16::<255>(_mm_shufflelo_epi16::<255>(s));
            let a = rounded_255(_mm_mullo_epi16(a, opacity));
            let raw_product = _mm_mullo_epi16(s, a);
            let base = _mm_mullo_epi16(s, negative);
            let base = _mm_srli_epi16::<8>(_mm_add_epi16(_mm_add_epi16(base, _mm_set1_epi16(1)), _mm_srli_epi16::<8>(base)));
            let product = if TINT { _mm_mullo_epi16(base, a) } else { raw_product };
            let first = match MODE {
                1 => _mm_add_epi16(d, rounded_255(product)),
                2 => _mm_subs_epu16(d, rounded_255(product)),
                3 => {
                    let factor = _mm_add_epi16(product, _mm_mullo_epi16(
                        _mm_sub_epi16(_mm_set1_epi16(255), a), _mm_set1_epi16(255)));
                    mul_div_65025(d, factor)
                }
                _ => rounded_255(_mm_add_epi16(product, _mm_mullo_epi16(d,
                    _mm_sub_epi16(_mm_set1_epi16(255), a)))),
            };
            if POSITIVE { _mm_add_epi16(first, mul_div_65025(raw_product, positive)) } else { first }
        }
    }

    #[inline]
    unsafe fn mul_div_65025(a: __m128i, b: __m128i) -> __m128i {
        unsafe {
            let lo = _mm_mullo_epi16(a, b);
            let hi = _mm_mulhi_epu16(a, b);
            let divide = |v| {
                // Exact integer numerators below 2^24; rounding error is less
                // than the distance 1/65025 to the adjacent integer quotient.
                let v = _mm_add_epi32(v, _mm_set1_epi32(32512));
                _mm_cvttps_epi32(_mm_div_ps(_mm_cvtepi32_ps(v), _mm_set1_ps(65025.0)))
            };
            _mm_packs_epi32(divide(_mm_unpacklo_epi16(lo, hi)), divide(_mm_unpackhi_epi16(lo, hi)))
        }
    }

    unsafe fn kernel<const MODE: u32, const TINT: bool, const POSITIVE: bool>(dst: &mut [u8], src: &[u8], opacity: u32, tint: [i32; 3]) {
        unsafe {
            let end = dst.len() / 16 * 16;
            let zero = _mm_setzero_si128();
            let alpha_mask = _mm_set1_epi32(0xff000000_u32 as i32);
            let opacity_v = _mm_set1_epi16(opacity as i16);
            let negative = tint.map(|t| (255 + t.min(0)).clamp(0,255) as i16);
            let positive = tint.map(|t| t.max(0) as i16);
            let negative = _mm_setr_epi16(negative[0],negative[1],negative[2],255,negative[0],negative[1],negative[2],255);
            let positive = _mm_setr_epi16(positive[0],positive[1],positive[2],0,positive[0],positive[1],positive[2],0);
            for at in (0..end).step_by(16) {
                let s = _mm_loadu_si128(src.as_ptr().add(at).cast());
                let d = _mm_loadu_si128(dst.as_ptr().add(at).cast());
                let lo = half::<MODE, TINT, POSITIVE>(_mm_unpacklo_epi8(s, zero), _mm_unpacklo_epi8(d, zero), opacity_v, negative, positive);
                let hi = half::<MODE, TINT, POSITIVE>(_mm_unpackhi_epi8(s, zero), _mm_unpackhi_epi8(d, zero), opacity_v, negative, positive);
                let rgb = _mm_packus_epi16(lo, hi);
                let out = _mm_or_si128(_mm_andnot_si128(alpha_mask, rgb), _mm_and_si128(alpha_mask, d));
                _mm_storeu_si128(dst.as_mut_ptr().add(at).cast(), out);
            }
            super::scalar(&mut dst[end..], &src[end..], opacity, MODE, tint);
        }
    }

    pub(super) unsafe fn row(dst: &mut [u8], src: &[u8], opacity: u32, mode: u32, tint: [i32; 3]) {
        unsafe {
            macro_rules! dispatch {
                ($negative:literal, $positive:literal) => {
                    match mode {
                        1 => kernel::<1, $negative, $positive>(dst, src, opacity, tint),
                        2 => kernel::<2, $negative, $positive>(dst, src, opacity, tint),
                        3 => kernel::<3, $negative, $positive>(dst, src, opacity, tint),
                        _ => kernel::<0, $negative, $positive>(dst, src, opacity, tint),
                    }
                };
            }
            // The two tint passes are independent; a zero additive term needs no division.
            match (tint.iter().any(|&t| t < 0), tint.iter().any(|&t| t > 0)) {
                (false, false) => dispatch!(false, false),
                (true, false) => dispatch!(true, false),
                (false, true) => dispatch!(false, true),
                (true, true) => dispatch!(true, true),
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::*;

    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn rounded_255(v: __m256i) -> __m256i {
            let v = _mm256_add_epi16(v, _mm256_set1_epi16(128));
            _mm256_srli_epi16::<8>(_mm256_add_epi16(v, _mm256_srli_epi16::<8>(v)))
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn half<const MODE: u32, const TINT: bool, const POSITIVE: bool>(s: __m256i, d: __m256i, opacity: __m256i, negative: __m256i, positive: __m256i) -> __m256i {
        unsafe {
            let a = _mm256_shufflehi_epi16::<255>(_mm256_shufflelo_epi16::<255>(s));
            let a = rounded_255(_mm256_mullo_epi16(a, opacity));
            let raw_product = _mm256_mullo_epi16(s, a);
            let base = _mm256_mullo_epi16(s, negative);
            let base = _mm256_srli_epi16::<8>(_mm256_add_epi16(_mm256_add_epi16(base, _mm256_set1_epi16(1)), _mm256_srli_epi16::<8>(base)));
            let product = if TINT { _mm256_mullo_epi16(base, a) } else { raw_product };
            let first = match MODE {
                1 => _mm256_add_epi16(d, rounded_255(product)),
                2 => _mm256_subs_epu16(d, rounded_255(product)),
                3 => {
                    let factor = _mm256_add_epi16(product, _mm256_mullo_epi16(
                        _mm256_sub_epi16(_mm256_set1_epi16(255), a), _mm256_set1_epi16(255)));
                    mul_div_65025(d, factor)
                }
                _ => rounded_255(_mm256_add_epi16(product, _mm256_mullo_epi16(d,
                    _mm256_sub_epi16(_mm256_set1_epi16(255), a)))),
            };
            if POSITIVE { _mm256_add_epi16(first, mul_div_65025(raw_product, positive)) } else { first }
        }
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn mul_div_65025(a: __m256i, b: __m256i) -> __m256i {
            let lo = _mm256_mullo_epi16(a, b);
            let hi = _mm256_mulhi_epu16(a, b);
            let divide = |v| {
                // Exact integer numerators below 2^24; rounding error is less
                // than the distance 1/65025 to the adjacent integer quotient.
                let v = _mm256_add_epi32(v, _mm256_set1_epi32(32512));
                _mm256_cvttps_epi32(_mm256_div_ps(_mm256_cvtepi32_ps(v), _mm256_set1_ps(65025.0)))
            };
            _mm256_packs_epi32(divide(_mm256_unpacklo_epi16(lo, hi)), divide(_mm256_unpackhi_epi16(lo, hi)))
    }

    #[target_feature(enable = "avx2")]
    unsafe fn kernel<const MODE: u32, const TINT: bool, const POSITIVE: bool>(dst: &mut [u8], src: &[u8], opacity: u32, tint: [i32; 3]) {
        unsafe {
            let end = dst.len() / 32 * 32;
            let zero = _mm256_setzero_si256();
            let alpha_mask = _mm256_set1_epi32(0xff000000_u32 as i32);
            let opacity_v = _mm256_set1_epi16(opacity as i16);
            let negative = tint.map(|t| (255 + t.min(0)).clamp(0,255) as i16);
            let positive = tint.map(|t| t.max(0) as i16);
            let negative = _mm256_setr_epi16(negative[0],negative[1],negative[2],255,negative[0],negative[1],negative[2],255,negative[0],negative[1],negative[2],255,negative[0],negative[1],negative[2],255);
            let positive = _mm256_setr_epi16(positive[0],positive[1],positive[2],0,positive[0],positive[1],positive[2],0,positive[0],positive[1],positive[2],0,positive[0],positive[1],positive[2],0);
            for at in (0..end).step_by(32) {
                let s = _mm256_loadu_si256(src.as_ptr().add(at).cast());
                let d = _mm256_loadu_si256(dst.as_ptr().add(at).cast());
                let lo = half::<MODE, TINT, POSITIVE>(_mm256_unpacklo_epi8(s, zero), _mm256_unpacklo_epi8(d, zero), opacity_v, negative, positive);
                let hi = half::<MODE, TINT, POSITIVE>(_mm256_unpackhi_epi8(s, zero), _mm256_unpackhi_epi8(d, zero), opacity_v, negative, positive);
                let rgb = _mm256_packus_epi16(lo, hi);
                let out = _mm256_or_si256(_mm256_andnot_si256(alpha_mask, rgb), _mm256_and_si256(alpha_mask, d));
                _mm256_storeu_si256(dst.as_mut_ptr().add(at).cast(), out);
            }
            super::scalar(&mut dst[end..], &src[end..], opacity, MODE, tint);
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn row(dst: &mut [u8], src: &[u8], opacity: u32, mode: u32, tint: [i32; 3]) {
        unsafe {
            macro_rules! dispatch {
                ($negative:literal, $positive:literal) => {
                    match mode {
                        1 => kernel::<1, $negative, $positive>(dst, src, opacity, tint),
                        2 => kernel::<2, $negative, $positive>(dst, src, opacity, tint),
                        3 => kernel::<3, $negative, $positive>(dst, src, opacity, tint),
                        _ => kernel::<0, $negative, $positive>(dst, src, opacity, tint),
                    }
                };
            }
            // The two tint passes are independent; a zero additive term needs no division.
            match (tint.iter().any(|&t| t < 0), tint.iter().any(|&t| t > 0)) {
                (false, false) => dispatch!(false, false),
                (true, false) => dispatch!(true, false),
                (false, true) => dispatch!(false, true),
                (true, true) => dispatch!(true, true),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn vector_matches_scalar_alpha_blends_and_tail() {
        let mut seed = 27_u32;
        let mut bytes = |n| (0..n).map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as u8
        }).collect::<Vec<_>>();
        let source = bytes(4099 * 4);
        let background = bytes(source.len());
        for tint in [[0;3], [-500,-1,-128], [1,128,255], [-45,200,-255], [510,1000,-4]] {
        for mode in 0..=3 {
            for opacity in 0..=255 {
                let mut actual = background.clone();
                let mut expected = background.clone();
                super::row(&mut actual, &source, opacity, mode, tint);
                super::scalar(&mut expected, &source, opacity, mode, tint);
                assert_eq!(actual, expected, "mode={mode} opacity={opacity} tint={tint:?}");
                #[cfg(target_arch = "x86_64")]
                if tint.iter().all(|&v| v <= 255) {
                    let mut baseline = background.clone();
                    unsafe { super::x86::row(&mut baseline, &source, opacity, mode, tint); }
                    assert_eq!(baseline, expected, "SSE2 mode={mode} opacity={opacity} tint={tint:?}");
                }
            }
        }
    }
    }
}
