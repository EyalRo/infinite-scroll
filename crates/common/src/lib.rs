pub mod auth;
pub mod dither;
pub mod http;
pub mod zpl;

pub use zpl::ZplJob;

/// The validated print canvas width (dots). See the Infinite Scroll
/// knowledge page's "Validated printing method" section.
pub const PRINT_WIDTH_PX: u32 = 650;
const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 50_000_000;
const MAX_HEIGHT_PX: u32 = 20_000;

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

    let decoded = image::load_from_memory(bytes)
        .map_err(|error| ConvertError(format!("failed to decode image: {error}")))?;

    if (decoded.width() as u64) * (decoded.height() as u64) > MAX_IMAGE_PIXELS {
        return Err(ConvertError("image dimensions are too large".into()));
    }

    // Flatten any alpha onto white, then convert to grayscale -- matches
    // the Python pipeline's RGBA-composite-then-L-convert behavior for
    // images with transparency, and is a no-op (transparency == fully
    // opaque) for opaque images either way.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([10, 10, 10]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
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
    fn sniffs_png_and_jpeg_by_magic_bytes_not_by_extension() {
        assert!(matches!(sniff_image_format(&tiny_png_bytes(1, 1)), Some(ImageFormatKind::Png)));
        assert!(sniff_image_format(b"garbage").is_none());
    }
}
