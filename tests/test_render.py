import json
from pathlib import Path

from app.render import DEFAULT_CSS_PATH, render_html, render_post_png

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


def test_render_html_escapes_user_supplied_fields():
    """Verify that Jinja2 autoescaping protects user-supplied fields from XSS."""
    css = DEFAULT_CSS_PATH.read_text()

    # Create fields with adversarial HTML-significant characters in user fields
    fields = FIXTURE.copy()
    fields["name"] = '<script>alert("xss")</script>'
    fields["title"] = "Title with <b>markup</b>"
    fields["avatar_initials"] = "A&B"
    fields["body"] = "Body with <img src=x onerror=alert(1)>\n\nAnother para with <"

    html = render_html(fields, css)

    # Assert that raw unescaped tags are NOT present
    assert "<script>" not in html
    assert "<b>" not in html
    assert '<img src=x onerror=' not in html

    # Assert that escaped forms ARE present
    assert "&lt;script&gt;" in html
    assert "&lt;b&gt;" in html
    assert "&lt;img" in html
    assert "&amp;B" in html
    assert "&lt;" in html  # Closing angle bracket escaped
