pub mod auth;
pub mod dither;
pub mod http;
pub mod zpl;

pub use zpl::ZplJob;

/// The validated print canvas width (dots). See the Infinite Scroll
/// knowledge page's "Validated printing method" section.
pub const PRINT_WIDTH_PX: u32 = 650;
pub const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 50_000_000;
const MAX_HEIGHT_PX: u32 = 20_000;

/// Current time as fractional Unix seconds. Shared by `watcher` and
/// `printer` because values from this function cross process boundaries --
/// `watcher` writes `added_at` into a catalog sidecar, and `printer` later
/// compares it (and its own `next_print_at`/`last_printed_at`) against
/// fresh calls to this same function -- so both sides must agree on the
/// exact clock source and behavior on error.
pub fn now_unix_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[derive(Debug)]
pub enum ImageFormatKind {
    Png,
    Jpeg,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const JPEG_SIGNATURE: [u8; 3] = [0xff, 0xd8, 0xff];

/// Sniffs the magic bytes only -- deliberately not relying on a
/// caller-supplied Content-Type or filename extension, both of which are
/// trivially wrong or spoofable.
pub fn sniff_image_format(bytes: &[u8]) -> Option<ImageFormatKind> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        Some(ImageFormatKind::Png)
    } else if bytes.starts_with(&JPEG_SIGNATURE) {
        Some(ImageFormatKind::Jpeg)
    } else {
        None
    }
}

#[derive(Debug)]
pub struct ConvertError(pub String);

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Decodes, flattens transparency onto white, converts to grayscale,
/// resizes to PRINT_WIDTH_PX preserving aspect ratio, dithers to 1-bit,
/// and packs into a ZPL job. The single entry point the watcher calls for
/// every file it picks up from `ready/`.
///
pub fn normalize_and_convert(bytes: &[u8]) -> Result<ZplJob, ConvertError> {
    if bytes.is_empty() {
        return Err(ConvertError("upload is empty".into()));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ConvertError("image is larger than 20 MB".into()));
    }
    if sniff_image_format(bytes).is_none() {
        return Err(ConvertError("not a PNG or JPEG image".into()));
    }

    // JPEGs from phones commonly store their intended portrait orientation
    // in EXIF rather than rotating the encoded pixels. Read it from the
    // decoder before consuming it, then apply it to the decoded raster.
    // An invalid or absent EXIF block must not make an otherwise printable
    // image fail conversion, so it deliberately falls back to no transform.
    use image::ImageDecoder;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| ConvertError(format!("failed to read image: {error}")))?;
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| ConvertError(format!("failed to decode image: {error}")))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| ConvertError(format!("failed to decode image: {error}")))?;
    decoded.apply_orientation(orientation);

    if (decoded.width() as u64) * (decoded.height() as u64) > MAX_IMAGE_PIXELS {
        return Err(ConvertError("image dimensions are too large".into()));
    }

    // Flatten any alpha onto white, then convert to grayscale. This is a
    // no-op for opaque images; transparent regions print as white paper.
    let rgba = decoded.to_rgba8();
    let mut flattened = image::ImageBuffer::from_pixel(rgba.width(), rgba.height(), image::Rgb([255u8, 255, 255]));
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        let alpha = a as f32 / 255.0;
        let blend = |channel: u8, background: u8| -> u8 {
            (channel as f32 * alpha + background as f32 * (1.0 - alpha)).round() as u8
        };
        flattened.put_pixel(x, y, image::Rgb([blend(r, 255), blend(g, 255), blend(b, 255)]));
    }
    let gray = image::DynamicImage::ImageRgb8(flattened).to_luma8();

    let height = ((gray.height() as f64) * (PRINT_WIDTH_PX as f64) / (gray.width() as f64))
        .round()
        .max(1.0) as u32;
    if height > MAX_HEIGHT_PX {
        return Err(ConvertError(format!("image is too tall after scaling (maximum {MAX_HEIGHT_PX} dots)")));
    }
    let resized = image::imageops::resize(&gray, PRINT_WIDTH_PX, height, image::imageops::FilterType::Lanczos3);

    let bits = dither::floyd_steinberg_1bit(resized.as_raw(), PRINT_WIDTH_PX, height);
    Ok(zpl::pack_to_zpl(&bits, PRINT_WIDTH_PX, height))
}

/// Renders a stored ZPL job back into a viewable PNG -- exactly the 1-bit
/// black/white image that will come out of the printer, not the original
/// upload (which is discarded once converted). The catalog only keeps the
/// packed ZPL text, so this decodes it back out rather than requiring a
/// separately-stored preview image that could drift out of sync with it.
pub fn zpl_to_preview_png(zpl_text: &str) -> Result<Vec<u8>, String> {
    let (bits, width, height) = zpl::unpack_from_zpl(zpl_text).map_err(|error| error.0)?;
    let image = image::GrayImage::from_raw(width, height, bits).ok_or_else(|| "decoded bitmap size mismatch".to_string())?;
    let mut png_bytes = Vec::new();
    image::DynamicImage::ImageLuma8(image)
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|error| format!("failed to encode preview PNG: {error}"))?;
    Ok(png_bytes)
}


/// Smallest and largest thumbnail width a client may ask for (pixels).
pub const THUMBNAIL_MIN_WIDTH: u32 = 32;
pub const THUMBNAIL_MAX_WIDTH: u32 = 320;

/// A small grayscale JPEG of a stored ZPL job, for list views. The 1-bit
/// print bitmap is box-averaged down, so halftone dots blend back into grays
/// and the thumbnail looks like the finished print rather than a noisy
/// aliased dither. Returns `(jpeg bytes, width, height)`.
pub fn zpl_to_thumbnail_jpeg(zpl_text: &str, width: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let (bits, full_width, full_height) = zpl::unpack_from_zpl(zpl_text).map_err(|error| error.0)?;
    let image = image::GrayImage::from_raw(full_width, full_height, bits).ok_or_else(|| "decoded bitmap size mismatch".to_string())?;
    let width = width.clamp(THUMBNAIL_MIN_WIDTH, THUMBNAIL_MAX_WIDTH).min(full_width.max(1));
    let height = ((full_height as f64) * (width as f64) / (full_width.max(1) as f64)).round().max(1.0) as u32;
    let small = image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 70)
        .encode(small.as_raw(), width, height, image::ExtendedColorType::L8)
        .map_err(|error| format!("failed to encode thumbnail: {error}"))?;
    Ok((jpeg, width, height))
}

/// Standard (RFC 4648) base64 with padding, for carrying binary in JSON.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (input, expected) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64_encode(input.as_bytes()), expected);
        }
    }

    #[test]
    fn thumbnail_is_a_small_jpeg_with_the_requested_width() {
        let (width, height) = (650u32, 400u32);
        let bits: Vec<u8> = (0..width * height).map(|i| if (i / 8) % 2 == 0 { 0 } else { 255 }).collect();
        let zpl = zpl::pack_to_zpl(&bits, width, height).text;
        let (jpeg, w, h) = zpl_to_thumbnail_jpeg(&zpl, 160).unwrap();
        assert_eq!((w, h), (160, 98));
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert!(jpeg.len() < 8_000, "thumbnail too large for BLE: {}", jpeg.len());
        // Out-of-range widths are clamped rather than rejected.
        assert_eq!(zpl_to_thumbnail_jpeg(&zpl, 5000).unwrap().1, THUMBNAIL_MAX_WIDTH);
    }

    fn tiny_png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([10, 10, 10]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn jpeg_with_orientation(width: u32, height: u32, orientation: u8) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([10, 20, 30]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&image::DynamicImage::ImageRgb8(image))
            .unwrap();

        // APP1 / Exif block containing one little-endian Orientation entry.
        // JPEG segment length includes its two-byte length field, not marker.
        let exif = [
            b'E',
            b'x',
            b'i',
            b'f',
            0,
            0,
            b'I',
            b'I',
            42,
            0,
            8,
            0,
            0,
            0,
            1,
            0,
            0x12,
            0x01,
            3,
            0,
            1,
            0,
            0,
            0,
            orientation,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        let mut oriented = Vec::with_capacity(jpeg.len() + exif.len() + 4);
        oriented.extend_from_slice(&jpeg[..2]); // SOI
        oriented.extend_from_slice(&[0xff, 0xe1, 0, (exif.len() as u8) + 2]);
        oriented.extend_from_slice(&exif);
        oriented.extend_from_slice(&jpeg[2..]);
        oriented
    }

    #[test]
    fn rejects_empty_input() {
        let error = normalize_and_convert(&[]).unwrap_err();
        assert_eq!(error.0, "upload is empty");
    }

    #[test]
    fn rejects_bytes_that_are_not_png_or_jpeg() {
        let error = normalize_and_convert(b"not an image").unwrap_err();
        assert_eq!(error.0, "not a PNG or JPEG image");
    }

    #[test]
    fn converts_a_tiny_png_to_the_validated_print_width() {
        let job = normalize_and_convert(&tiny_png_bytes(100, 50)).unwrap();
        assert_eq!(job.width, PRINT_WIDTH_PX);
        assert_eq!(job.height, 325); // 50 * 650 / 100
        assert!(job.text.starts_with("^XA\n^PW650\n"));
    }

    #[test]
    fn honors_exif_orientation_before_calculating_print_height() {
        // The stored pixels are landscape, but EXIF orientation 6 makes the
        // displayed image portrait. Height must therefore reflect 20x40,
        // not the encoded 40x20 dimensions.
        let job = normalize_and_convert(&jpeg_with_orientation(40, 20, 6)).unwrap();
        assert_eq!((job.width, job.height), (PRINT_WIDTH_PX, 1_300));
    }

    #[test]
    fn sniffs_png_and_jpeg_by_magic_bytes_not_by_extension() {
        assert!(matches!(sniff_image_format(&tiny_png_bytes(1, 1)), Some(ImageFormatKind::Png)));
        assert!(sniff_image_format(b"garbage").is_none());
    }
}
