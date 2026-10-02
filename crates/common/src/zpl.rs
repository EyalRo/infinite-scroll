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
            let black = if bits[y * width as usize + x] == 0 {
                1u8
            } else {
                0u8
            };
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
    ZplJob {
        text,
        width,
        height,
    }
}

#[derive(Debug)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Reverses `pack_to_zpl`, reconstructing the same one-byte-per-pixel
/// buffer (0 = black, 255 = white) it was built from. Only needs to parse
/// the exact `^PW`/`^LL`/`^GFA` shape this module writes -- it's used
/// solely to re-render a stored job as a preview image, not to interpret
/// arbitrary ZPL from elsewhere.
pub fn unpack_from_zpl(text: &str) -> Result<(Vec<u8>, u32, u32), DecodeError> {
    let width: u32 = text
        .lines()
        .find_map(|line| line.strip_prefix("^PW"))
        .ok_or_else(|| DecodeError("missing ^PW field".into()))?
        .parse()
        .map_err(|_| DecodeError("invalid ^PW field".into()))?;
    let height: u32 = text
        .lines()
        .find_map(|line| line.strip_prefix("^LL"))
        .ok_or_else(|| DecodeError("missing ^LL field".into()))?
        .parse()
        .map_err(|_| DecodeError("invalid ^LL field".into()))?;

    let gfa_line = text
        .lines()
        .find(|line| line.starts_with("^GFA,"))
        .ok_or_else(|| DecodeError("missing ^GFA field".into()))?;
    let mut fields = gfa_line["^GFA,".len()..].splitn(4, ',');
    fields.next(); // byte count (== total bytes, redundant with the next field)
    fields.next(); // total bytes
    let row_bytes: usize = fields
        .next()
        .ok_or_else(|| DecodeError("malformed ^GFA field".into()))?
        .parse()
        .map_err(|_| DecodeError("invalid row byte count".into()))?;
    let hexdata = fields
        .next()
        .ok_or_else(|| DecodeError("malformed ^GFA field".into()))?
        .trim();

    if hexdata.len() % 2 != 0 {
        return Err(DecodeError("hex data has an odd length".into()));
    }
    let packed: Vec<u8> = (0..hexdata.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hexdata[i..i + 2], 16)
                .map_err(|_| DecodeError("invalid hex data".into()))
        })
        .collect::<Result<_, _>>()?;
    if packed.len() < row_bytes * height as usize {
        return Err(DecodeError(
            "hex data is shorter than width/height imply".into(),
        ));
    }

    let mut bits = vec![255u8; (width as usize) * (height as usize)];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let byte = packed[y * row_bytes + x / 8];
            let bit = (byte >> (7 - (x % 8))) & 1;
            bits[y * width as usize + x] = if bit == 1 { 0 } else { 255 };
        }
    }
    Ok((bits, width, height))
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

    #[test]
    fn unpack_reverses_pack_for_a_checkerboard_with_a_padded_row() {
        // 3x2, so row_bytes = ceil(3/8) = 1 -- exercises trailing padding bits too.
        let bits = [0u8, 255, 0, 255, 0, 255];
        let job = pack_to_zpl(&bits, 3, 2);
        let (roundtripped, width, height) = unpack_from_zpl(&job.text).unwrap();
        assert_eq!((width, height), (3, 2));
        assert_eq!(roundtripped, bits);
    }

    #[test]
    fn unpack_rejects_text_missing_expected_fields() {
        assert!(unpack_from_zpl("not zpl at all").is_err());
    }
}
