import io

import pytest
from PIL import Image

from app.image_pipeline import normalize_uploaded_image


def png_bytes(mode="RGB", size=(100, 200), color="black"):
    image = Image.new(mode, size, color)
    output = io.BytesIO()
    image.save(output, "PNG")
    return output.getvalue()


def test_uploaded_image_is_normalized_to_printer_width():
    image = normalize_uploaded_image(png_bytes())
    assert image.mode == "L"
    assert image.size == (650, 1300)


def test_transparent_upload_is_composited_on_white():
    image = normalize_uploaded_image(png_bytes("RGBA", (10, 10), (0, 0, 0, 0)))
    assert image.getpixel((0, 0)) == 255


def test_invalid_upload_is_rejected():
    with pytest.raises(ValueError, match="valid image"):
        normalize_uploaded_image(b"not an image")


def test_extremely_tall_upload_is_rejected():
    with pytest.raises(ValueError, match="too tall"):
        normalize_uploaded_image(png_bytes(size=(1, 40)))
