"""HTML/CSS -> content-cropped grayscale PNG for one Infinite Scroll post.

This owns everything upstream of dithering: filling in the HTML skeleton,
running WeasyPrint, rasterizing with pdftoppm, and cropping to content
height. app/dither.py picks up from the returned image.
"""
import subprocess
import tempfile
from pathlib import Path

from jinja2 import Environment, FileSystemLoader
from PIL import Image, ImageChops
from weasyprint import HTML

from app import icons

APP_DIR = Path(__file__).parent
TEMPLATE_DIR = APP_DIR / "templates"
DEFAULT_CSS_PATH = APP_DIR / "default_template.css"

WIDTH_PX = 650
# 96 DPI makes 1 CSS px equal 1 raster px, so a "650px" page renders at
# exactly WIDTH_PX wide with no rescaling needed.
RASTER_DPI = 96

_env = Environment(loader=FileSystemLoader(str(TEMPLATE_DIR)), autoescape=True)


def _format_int(n: int) -> str:
    return f"{n:,}"


def render_html(fields: dict, css_text: str) -> str:
    template = _env.get_template("post.html.j2")
    return template.render(
        name=fields["name"],
        title=fields["title"],
        avatar_initials=fields["avatar_initials"],
        body_paragraphs=fields["body"].split("\n\n"),
        reactions_fmt=_format_int(fields["reactions"]),
        comments_fmt=_format_int(fields["comments"]),
        reposts_fmt=_format_int(fields["reposts"]),
        css=css_text,
        icons=icons,
    )


def render_post_png(fields: dict, css_text: str) -> Image.Image:
    html_text = render_html(fields, css_text)

    with tempfile.TemporaryDirectory() as tmp:
        pdf_path = Path(tmp) / "post.pdf"
        HTML(string=html_text, base_url=str(APP_DIR)).write_pdf(str(pdf_path))

        out_prefix = Path(tmp) / "post"
        subprocess.run(
            [
                "pdftoppm",
                "-png",
                "-r", str(RASTER_DPI),
                "-singlefile",
                str(pdf_path),
                str(out_prefix),
            ],
            check=True,
            capture_output=True,
        )
        png_path = out_prefix.with_suffix(".png")
        raw = Image.open(png_path)
        raw.load()  # force read before the tempdir is cleaned up

    gray = raw.convert("L")
    if gray.width != WIDTH_PX:
        gray = gray.resize((WIDTH_PX, round(gray.height * WIDTH_PX / gray.width)))

    inverted = ImageChops.invert(gray)
    bbox = inverted.getbbox()
    bottom = bbox[3] if bbox else gray.height
    return gray.crop((0, 0, WIDTH_PX, bottom))
