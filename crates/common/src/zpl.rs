/// A complete, self-contained ZPL job ready to write to the printer device.
#[derive(Debug)]
pub struct ZplJob {
    pub text: String,
    pub width: u32,
    pub height: u32,
}

/// Packs a 1-bit image (one byte per pixel, 0 = black, non-zero = white --
/// the shape `dither::to_1bit` produces) into a self-contained ZPL `^GFA`
/// job. Matches the validated contract exactly: 8 dots per byte, MSB
/// first, black = 1 bit, short rows padded with trailing zero bits,
/// uppercase hex.
pub fn pack_to_zpl(bits: &[u8], width: u32, height: u32) -> ZplJob {
    let row_bytes = (width as usize + 7) / 8;
    let total_bytes = row_bytes * height as usize;
    let mut out = vec![0u8; total_bytes];

    for y in 0..height as usize {
        let mut byte = 0u8;
        let mut bitcount = 0u8;
        let mut out_idx = y * row_bytes;
        for x in 0..width as usize {
            let black = if bits[y * width as usize + x] == 0 { 1u8 } else { 0u8 };
            byte = (byte << 1) | black;
            bitcount += 1;
            if bitcount == 8 {
                out[out_idx] = byte;
                out_idx += 1;
                byte = 0;
                bitcount = 0;
            }
        }
        if bitcount > 0 {
            byte <<= 8 - bitcount;
            out[out_idx] = byte;
        }
    }

    let hexdata = out.iter().map(|b| format!("{b:02X}")).collect::<String>();
    let text = format!(
        "^XA\n^PW{width}\n^LL{height}\n^LH0,0\n^FO0,0\n^GFA,{total_bytes},{total_bytes},{row_bytes},{hexdata}\n^XZ\n"
    );
    ZplJob { text, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_a_2x2_checkerboard_with_msb_first_and_trailing_zero_padding() {
        // black, white
        // white, black  -- as a flat row-major 1-bit-per-pixel buffer (0 = black)
        let bits = [0u8, 255, 255, 0];
        let job = pack_to_zpl(&bits, 2, 2);
        // row_bytes = ceil(2/8) = 1, total_bytes = 2
        // row 0: black(1) white(0) -> bits "10" -> padded "10000000" -> 0x80
        // row 1: white(0) black(1) -> bits "01" -> padded "01000000" -> 0x40
        assert!(job.text.contains("^GFA,2,2,1,8040\n"));
        assert!(job.text.starts_with("^XA\n^PW2\n^LL2\n"));
        assert!(job.text.ends_with("^XZ\n"));
    }

    #[test]
    fn packs_a_full_byte_row_with_no_padding() {
        let bits = [0u8, 0, 0, 0, 0, 0, 0, 0]; // 8 black pixels, 1 row
        let job = pack_to_zpl(&bits, 8, 1);
        assert!(job.text.contains("^GFA,1,1,1,FF\n"));
    }
}
