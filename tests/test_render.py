import json
from pathlib import Path

from app.render import DEFAULT_CSS_PATH, render_post_png

FIXTURE = json.loads((Path(__file__).parent / "fixtures" / "sample_post.json").read_text())


def test_render_post_png_produces_a_650_wide_image():
    css = DEFAULT_CSS_PATH.read_text()
    img = render_post_png(FIXTURE, css)
    assert img.mode == "L"
    assert img.width == 650
    assert 100 < img.height < 20000


def test_render_post_png_is_deterministic():
    css = DEFAULT_CSS_PATH.read_text()
    img1 = render_post_png(FIXTURE, css)
    img2 = render_post_png(FIXTURE, css)
    assert img1.tobytes() == img2.tobytes()
