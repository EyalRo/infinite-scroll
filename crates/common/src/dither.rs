/// Converts a grayscale (one byte per pixel, 0-255) buffer to 1-bit using
/// Floyd-Steinberg error diffusion -- the same algorithm PIL's `L -> 1`
/// conversion uses (its documented default), which is what the validated
/// print contract was built and physically tested against.
///
/// Output: one byte per pixel, `0` = black, `255` = white -- same shape
/// `zpl::pack_to_zpl` consumes, kept byte-per-pixel (not bit-packed) so the
/// error-diffusion arithmetic stays simple and bounds-obviously-correct.
pub fn floyd_steinberg_1bit(gray: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let mut errors: Vec<f32> = gray.iter().map(|&p| p as f32).collect();
    let mut out = vec![0u8; width * height];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let old = errors[idx].clamp(0.0, 255.0);
            let new = if old < 128.0 { 0.0 } else { 255.0 };
            out[idx] = new as u8;
            let err = old - new;

            let mut push = |dx: isize, dy: isize, weight: f32| {
                let nx = x as isize + dx;
                let ny = y as isize + dy;
                if nx >= 0 && (nx as usize) < width && ny >= 0 && (ny as usize) < height {
                    errors[ny as usize * width + nx as usize] += err * weight;
                }
            };
            push(1, 0, 7.0 / 16.0);
            push(-1, 1, 3.0 / 16.0);
            push(0, 1, 5.0 / 16.0);
            push(1, 1, 1.0 / 16.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uniform_black_image_stays_black() {
        let gray = vec![0u8; 16];
        let out = floyd_steinberg_1bit(&gray, 4, 4);
        assert!(out.iter().all(|&p| p == 0));
    }

    #[test]
    fn a_uniform_white_image_stays_white() {
        let gray = vec![255u8; 16];
        let out = floyd_steinberg_1bit(&gray, 4, 4);
        assert!(out.iter().all(|&p| p == 255));
    }

    #[test]
    fn a_mid_gray_image_produces_a_mix_of_black_and_white() {
        let gray = vec![128u8; 400];
        let out = floyd_steinberg_1bit(&gray, 20, 20);
        let black = out.iter().filter(|&&p| p == 0).count();
        let white = out.iter().filter(|&&p| p == 255).count();
        assert!(black > 50 && white > 50, "expected a dithered mix, got {black} black / {white} white");
    }
}
