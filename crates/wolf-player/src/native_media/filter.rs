//! 9A4D7F -> 7755B0(filter1) -> 777200 -> 8F6B20.
//! Two separable passes, normalized exp(-0.5*i*i/strength) weights,
//! paired linear samples at +/-1.5, 3.5, ... and clamp-to-edge addressing.
use wolf_media::Image;

pub(super) fn blur(source: &Image, amount: i32) -> Result<Image, String> {
    let amount = amount.clamp(0, 16);
    let (w, h) = source.dimensions();
    let (w, h) = (w as usize, h as usize);
    if amount == 0 { return Image::new(w, h, source.bytes().to_vec()); }
    let count = if amount < 8 { 4 } else if amount < 12 { 8 } else { 16 };
    let strength = (amount * (amount + 1)) as f32 / 10.0;
    let mut weights = [0.0_f32; 16];
    let mut total = 0.0;
    for (i, weight) in weights[..count].iter_mut().enumerate() {
        *weight = (-0.5 * (i * i) as f32 / strength).exp();
        total += *weight * if i == 0 { 1.0 } else { 2.0 };
    }
    for weight in &mut weights[..count] { *weight /= total; }
    let mut input = source.bytes().to_vec();
    let mut output = vec![0; input.len()];
    for axis in 0..2 {
        for y in 0..h {
            for x in 0..w {
                let at = (y * w + x) * 4;
                let mut color: [f32; 4] = std::array::from_fn(|c| input[at+c] as f32 * weights[0]);
                for (i, &weight) in weights[1..count].iter().enumerate() {
                    let distance = (i * 2 + 1) as isize;
                    for delta in [-distance, -distance-1, distance, distance+1] {
                        let sx = if axis == 0 { (x as isize + delta).clamp(0, w as isize-1) as usize } else { x };
                        let sy = if axis == 1 { (y as isize + delta).clamp(0, h as isize-1) as usize } else { y };
                        let sample = (sy*w+sx)*4;
                        for c in 0..4 { color[c] += input[sample+c] as f32 * (weight * 0.5); }
                    }
                }
                for c in 0..4 { output[at+c] = color[c].round().clamp(0.0,255.0) as u8; }
            }
        }
        std::mem::swap(&mut input, &mut output);
    }
    Image::new(w, h, input)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blur_preserves_constant_and_spreads_impulse_symmetrically() {
        let uniform = Image::new(5, 3, [40,80,120,255].repeat(15)).unwrap();
        assert_eq!(blur(&uniform, 16).unwrap().bytes(), uniform.bytes());
        let mut pixels = vec![0; 17*17*4];
        pixels[(8*17+8)*4..][..4].copy_from_slice(&[255;4]);
        let source = Image::new(17,17,pixels).unwrap();
        assert_eq!(blur(&source, 0).unwrap().bytes(), source.bytes());
        let result = blur(&source, 2).unwrap();
        assert!(result.bytes()[(8*17+8)*4] < 255);
        assert!(result.bytes()[(8*17+7)*4] > 0);
        for y in 0..17 { for x in 0..17 {
            assert_eq!(result.bytes()[(y*17+x)*4], result.bytes()[(y*17+16-x)*4]);
        }}
    }
}
