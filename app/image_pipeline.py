"""Normalize uploaded artwork into the validated 650-dot print canvas."""
import io

from PIL import Image, ImageOps, UnidentifiedImageError

from app.render import WIDTH_PX

MAX_UPLOAD_BYTES = 20 * 1024 * 1024
MAX_IMAGE_PIXELS = 50_000_000


def normalize_uploaded_image(data: bytes) -> Image.Image:
    if not data:
        raise ValueError("choose a PNG image")
    if len(data) > MAX_UPLOAD_BYTES:
        raise ValueError("image is larger than 20 MB")

    try:
        image = Image.open(io.BytesIO(data))
        image.load()
    except (UnidentifiedImageError, OSError) as exc:
        raise ValueError("upload is not a valid image") from exc

    if image.width * image.height > MAX_IMAGE_PIXELS:
        raise ValueError("image dimensions are too large")
    image = ImageOps.exif_transpose(image)

    if image.mode in ("RGBA", "LA") or "transparency" in image.info:
        rgba = image.convert("RGBA")
        white = Image.new("RGBA", rgba.size, "white")
        white.alpha_composite(rgba)
        image = white.convert("L")
    else:
        image = image.convert("L")

    height = max(1, round(image.height * WIDTH_PX / image.width))
    if height > 20_000:
        raise ValueError("image is too tall after scaling (maximum 20,000 dots)")
    return image.resize((WIDTH_PX, height), Image.Resampling.LANCZOS)
