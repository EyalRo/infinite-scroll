# Post Composer Web App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Flask web app, deployed on the Infinite Scroll installation Pi, that lets you fill in a post's details and print it over the existing validated ZPL/USB pipeline, plus a Design page to read/write the CSS used to render posts.

**Architecture:** A single Flask app with two feature areas — compose/print (`app/routes.py`) and CSS design (`app/design_routes.py`) — sharing a render pipeline (`app/render.py`: Jinja2 HTML + CSS → WeasyPrint → PDF → `pdftoppm` → content-cropped grayscale PNG) and the existing dither/pack/print pipeline (`app/dither.py`, `app/printer.py`). Runs as a systemd service directly on the Pi (required for direct `/dev/usb/lp0` access).

**Tech Stack:** Python 3, Flask, WeasyPrint (HTML/CSS → PDF), poppler's `pdftoppm` (PDF → PNG), Pillow (crop + dither), Jinja2, pytest.

## Global Constraints

- Printer device: `/dev/usb/lp0`, raw USB, no CUPS/vendor driver.
- Raster width: 650 dots, matches the physically-validated media fit.
- ZPL: `^GFA` bitmap command, `^PW650`, content-driven `^LL<height>`, same packing contract already validated (row-packed 8 dots/byte, black = 1 bit, uppercase hex).
- Dithering: Floyd-Steinberg (`Image.convert("1")`, PIL's default for that conversion).
- Web app: LAN-only, no authentication, binds `0.0.0.0:8080`.
- Print write: bounded timeout (~15s), never allowed to block the request indefinitely.
- Print writes are serialized — never two concurrent writes to `/dev/usb/lp0`.
- Icons: Lucide (ISC license) — exactly `thumbs-up`, `message-circle`, `repeat-2`, `send`. No other icons.
- No reproduction of LinkedIn's actual logo mark or brand color — layout/typography facsimile only.
- The disclaimer footer (`FICTIONAL TEST POST · NOT LINKEDIN DATA` / `FICTIONAL SATIRE`) always renders, regardless of the current CSS.
- Only CSS is user-editable through the Design page — the HTML structure (Jinja2 template) is fixed, not exposed to editing.
- Composed/printed posts are **not** persisted into any shared/dynamic-printing pool — print-once only.
- Every successful print is recorded at `/var/lib/infinite-scroll/print-ready/<slug>-<timestamp>.{png,zpl}` with sha256 logged.

---

## File Structure

```
infinite-scroll/
├── app/
│   ├── __init__.py             # Flask app factory, registers blueprints
│   ├── config.py                # env-driven paths + "current CSS" lookup
│   ├── render.py                # HTML/CSS -> content-cropped grayscale PNG
│   ├── dither.py                # 1-bit Floyd-Steinberg + ZPL packing
│   ├── printer.py               # serialized, bounded-timeout /dev/usb/lp0 writes
│   ├── record.py                # slugify + print-ready record-keeping
│   ├── routes.py                # GET /, POST /print
│   ├── design_routes.py         # GET /design, POST /design/preview, POST /design
│   ├── icons.py                 # bundled Lucide SVG glyphs
│   ├── default_template.css     # seed/fallback CSS (LinkedIn facsimile)
│   └── templates/
│       ├── post.html.j2         # fixed post structure (used by render.py)
│       ├── compose.html         # compose form page
│       └── design.html          # CSS editor + live preview page
├── tests/
│   ├── fixtures/sample_post.json
│   ├── test_app.py
│   ├── test_config.py
│   ├── test_render.py
│   ├── test_dither.py
│   ├── test_pipeline.py
│   ├── test_printer.py
│   ├── test_record.py
│   ├── test_routes.py
│   └── test_design_routes.py
├── deploy/
│   └── infinite-scroll-webapp.service
├── requirements.txt
├── requirements-dev.txt
├── pyproject.toml
├── run.py
└── docs/superpowers/{specs,plans}/...   # already exists
```

**Local dev/test environment:** this sandbox has no system Python; use Nix:

```bash
nix-shell -p 'python3.withPackages(ps: [ps.flask ps.weasyprint ps.pillow ps.jinja2 ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest -v"
```

(Already confirmed to resolve: flask, weasyprint, pillow, jinja2, pytest all install cleanly via this command.)

---

### Task 1: Project scaffolding

**Files:**
- Create: `requirements.txt`
- Create: `requirements-dev.txt`
- Create: `pyproject.toml`
- Create: `app/__init__.py`
- Create: `run.py`
- Test: `tests/test_app.py`

**Interfaces:**
- Produces: `app.create_app() -> flask.Flask`, with a `GET /health` route returning `{"status": "ok"}`.

- [ ] **Step 1: Create the dependency and pytest-path files**

`requirements.txt`:
```
flask>=3.0,<4
weasyprint>=62,<70
Pillow>=10,<11
Jinja2>=3.1,<4
```

`requirements-dev.txt`:
```
-r requirements.txt
pytest>=8,<10
```

`pyproject.toml`:
```toml
[tool.pytest.ini_options]
pythonpath = ["."]
```

- [ ] **Step 2: Write the failing test**

`tests/test_app.py`:
```python
from app import create_app


def test_health_endpoint_returns_ok():
    client = create_app().test_client()
    resp = client.get("/health")
    assert resp.status_code == 200
    assert resp.get_json() == {"status": "ok"}
```

- [ ] **Step 3: Run it to confirm it fails**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.flask ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_app.py -v"`
Expected: FAIL — `ModuleNotFoundError: No module named 'app'` (package doesn't exist yet).

- [ ] **Step 4: Create the app package**

`app/__init__.py`:
```python
from flask import Flask


def create_app() -> Flask:
    app = Flask(__name__)

    @app.get("/health")
    def health():
        return {"status": "ok"}

    return app
```

`run.py`:
```python
from app import create_app

app = create_app()

if __name__ == "__main__":
    app.run(host="0.0.0.0", port=8080)
```

- [ ] **Step 5: Run the test to confirm it passes**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.flask ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_app.py -v"`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
cd /home/stags/Source/infinite-scroll
git add requirements.txt requirements-dev.txt pyproject.toml app/__init__.py run.py tests/test_app.py
git commit -m "Add Flask app scaffolding with health check"
```

---

### Task 2: Render pipeline — HTML/CSS to grayscale PNG

**Files:**
- Create: `app/icons.py`
- Create: `app/templates/post.html.j2`
- Create: `app/default_template.css`
- Create: `app/render.py`
- Create: `app/config.py`
- Create: `tests/fixtures/sample_post.json`
- Test: `tests/test_render.py`
- Test: `tests/test_config.py`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces:
  - `app.render.DEFAULT_CSS_PATH: pathlib.Path`
  - `app.render.render_post_png(fields: dict, css_text: str) -> PIL.Image.Image` — grayscale (`"L"` mode), 650px wide, content-cropped height. `fields` keys: `name`, `title`, `avatar_initials` (all `str`), `body` (`str`, paragraphs separated by `"\n\n"`), `reactions`, `comments`, `reposts` (all `int`).
  - `app.config.css_path() -> pathlib.Path`
  - `app.config.print_ready_dir() -> pathlib.Path`
  - `app.config.current_css() -> str` — reads the saved CSS file if present, else `DEFAULT_CSS_PATH`.

- [ ] **Step 1: Add the bundled icon glyphs**

`app/icons.py`:
```python
"""Icon glyphs bundled from the Lucide icon set (ISC license, https://lucide.dev).
Each is a self-contained inline SVG string, stroke-based, sized via CSS
(see .icon in default_template.css)."""

THUMBS_UP = (
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" '
    'stroke-linecap="round" stroke-linejoin="round" class="icon">'
    '<path d="M7 10v12"/>'
    '<path d="M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2h0a3.13 3.13 0 0 1 3 3.88Z"/>'
    '</svg>'
)

MESSAGE_CIRCLE = (
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" '
    'stroke-linecap="round" stroke-linejoin="round" class="icon">'
    '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>'
    '</svg>'
)

REPEAT_2 = (
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" '
    'stroke-linecap="round" stroke-linejoin="round" class="icon">'
    '<path d="m2 9 3-3 3 3"/>'
    '<path d="M13 18H7a2 2 0 0 1-2-2V6"/>'
    '<path d="m22 15-3 3-3-3"/>'
    '<path d="M11 6h6a2 2 0 0 1 2 2v10"/>'
    '</svg>'
)

SEND = (
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" '
    'stroke-linecap="round" stroke-linejoin="round" class="icon">'
    '<path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/>'
    '<path d="m21.854 2.147-10.94 10.939"/>'
    '</svg>'
)
```

(These are best-effort reproductions of the real Lucide path data from memory — if pixel-perfect fidelity to the current published icon set matters, re-copy the four SVGs from https://lucide.dev before Task 8's deployment; functionally any valid stroke-based SVG here works fine with the pipeline.)

- [ ] **Step 2: Write the fixed HTML structure**

`app/templates/post.html.j2`:
```jinja2
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
@page { size: 650px 20000px; margin: 0; }
body { margin: 0; }
{{ css|safe }}
</style>
</head>
<body>
<div class="post">
  <div class="header">
    <div class="avatar">{{ avatar_initials }}</div>
    <div class="header-text">
      <div class="name">{{ name }}</div>
      <div class="title">{{ title }}</div>
      <div class="meta">2h &middot; Public</div>
    </div>
  </div>
  <div class="body">
    {% for para in body_paragraphs %}
    <p>{{ para }}</p>
    {% endfor %}
  </div>
  <div class="divider"></div>
  <div class="stats">{{ reactions_fmt }} reactions &middot; {{ comments_fmt }} comments &middot; {{ reposts_fmt }} reposts</div>
  <div class="divider"></div>
  <div class="button-row">
    <div class="button">{{ icons.THUMBS_UP|safe }}<span>Like</span></div>
    <div class="button">{{ icons.MESSAGE_CIRCLE|safe }}<span>Comment</span></div>
    <div class="button">{{ icons.REPEAT_2|safe }}<span>Repost</span></div>
    <div class="button">{{ icons.SEND|safe }}<span>Send</span></div>
  </div>
  <div class="disclaimer">
    <div>FICTIONAL TEST POST &middot; NOT LINKEDIN DATA</div>
    <div>FICTIONAL SATIRE</div>
  </div>
</div>
</body>
</html>
```

Note: `name`, `title`, `avatar_initials`, and each entry in `body_paragraphs` are **not** marked `|safe`, so Jinja2's autoescaping (enabled in Step 4) HTML-escapes them — these come from user form input. `css` and the `icons.*` values are code-controlled (a file on disk, and constants in `app/icons.py`), so they're explicitly marked `|safe`.

- [ ] **Step 3: Write the default CSS (LinkedIn facsimile)**

`app/default_template.css`:
```css
.post {
  width: 650px;
  box-sizing: border-box;
  padding: 24px;
  font-family: sans-serif;
  color: #000;
  background: #fff;
}

.header {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.avatar {
  flex: 0 0 56px;
  width: 56px;
  height: 56px;
  border: 3px solid #000;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 700;
  font-size: 22px;
}

.header-text {
  flex: 1;
}

.name {
  font-weight: 700;
  font-size: 20px;
  line-height: 1.3;
}

.title, .meta, .stats {
  opacity: 0.7;
}

.title {
  font-size: 15px;
  line-height: 1.3;
}

.meta {
  font-size: 13px;
  margin-top: 2px;
}

.body {
  margin-top: 16px;
  font-size: 17px;
  line-height: 1.45;
}

.body p {
  margin: 0 0 14px 0;
}

.body p:last-child {
  margin-bottom: 0;
}

.divider {
  border-top: 2px solid #000;
  margin: 16px 0;
}

.stats {
  font-size: 14px;
}

.button-row {
  display: flex;
  justify-content: space-between;
}

.button {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
  font-weight: 600;
}

.icon {
  width: 20px;
  height: 20px;
}

.disclaimer {
  margin-top: 20px;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.02em;
}

.disclaimer div {
  margin-bottom: 4px;
}

.disclaimer div:last-child {
  margin-bottom: 0;
}
```

- [ ] **Step 4: Write the render pipeline**

`app/render.py`:
```python
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
```

- [ ] **Step 5: Write `app/config.py`**

```python
"""Environment-driven paths and the "current CSS" lookup, centralized so
the compose and design routes agree on where things live."""
import os
from pathlib import Path

from app.render import DEFAULT_CSS_PATH


def css_path() -> Path:
    return Path(
        os.environ.get(
            "INFINITE_SCROLL_CSS_PATH",
            "/var/lib/infinite-scroll/webapp/template.css",
        )
    )


def print_ready_dir() -> Path:
    return Path(
        os.environ.get(
            "INFINITE_SCROLL_PRINT_READY_DIR",
            "/var/lib/infinite-scroll/print-ready",
        )
    )


def current_css() -> str:
    path = css_path()
    if path.exists():
        return path.read_text()
    return DEFAULT_CSS_PATH.read_text()
```

- [ ] **Step 6: Add the test fixture**

`tests/fixtures/sample_post.json`:
```json
{
  "name": "Test Poster",
  "title": "Quality Assurance · Determinism Enthusiast",
  "avatar_initials": "TP",
  "body": "This is a fixed sample body used only for pipeline regression testing.\n\nIt has two paragraphs.",
  "reactions": 1234,
  "comments": 56,
  "reposts": 7
}
```

- [ ] **Step 7: Write the render tests**

`tests/test_render.py`:
```python
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
```

- [ ] **Step 8: Write the config tests**

`tests/test_config.py`:
```python
from pathlib import Path

from app import config
from app.render import DEFAULT_CSS_PATH


def test_css_path_defaults_to_the_pi_webapp_location(monkeypatch):
    monkeypatch.delenv("INFINITE_SCROLL_CSS_PATH", raising=False)
    assert config.css_path() == Path("/var/lib/infinite-scroll/webapp/template.css")


def test_css_path_honors_env_override(monkeypatch, tmp_path):
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(tmp_path / "custom.css"))
    assert config.css_path() == tmp_path / "custom.css"


def test_current_css_falls_back_to_default_when_no_file_saved(monkeypatch, tmp_path):
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(tmp_path / "missing.css"))
    assert config.current_css() == DEFAULT_CSS_PATH.read_text()


def test_current_css_reads_the_saved_file_when_present(monkeypatch, tmp_path):
    css_file = tmp_path / "saved.css"
    css_file.write_text(".post { color: green; }")
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(css_file))
    assert config.current_css() == ".post { color: green; }"
```

- [ ] **Step 9: Run the tests**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.flask ps.weasyprint ps.pillow ps.jinja2 ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_render.py tests/test_config.py -v"`
Expected: PASS (4 tests).

If a font-related WeasyPrint warning or a "no font found" style error appears, add `dejavu_fonts` and `fontconfig` to the `-p` list on the `nix-shell` invocation and re-run — font availability is environment-specific and does not change any code in this task.

- [ ] **Step 10: Commit**

```bash
cd /home/stags/Source/infinite-scroll
git add app/icons.py app/templates/post.html.j2 app/default_template.css app/render.py app/config.py tests/fixtures/sample_post.json tests/test_render.py tests/test_config.py
git commit -m "Add HTML/CSS render pipeline with LinkedIn-facsimile default template"
```

---

### Task 3: Dithering, ZPL packing, and the pipeline regression test

**Files:**
- Create: `app/dither.py`
- Test: `tests/test_dither.py`
- Test: `tests/test_pipeline.py`

**Interfaces:**
- Consumes: `app.render.render_post_png`, `app.render.DEFAULT_CSS_PATH` (Task 2).
- Produces:
  - `app.dither.to_1bit(img: PIL.Image.Image) -> PIL.Image.Image` (mode `"1"`)
  - `app.dither.ZplJob` — dataclass with `text: str`, `total_bytes: int`, `row_bytes: int`, `width: int`, `height: int`
  - `app.dither.pack_to_zpl(img_1bit: PIL.Image.Image) -> ZplJob`

- [ ] **Step 1: Write the dither/pack module**

`app/dither.py`:
```python
"""1-bit Floyd-Steinberg conversion and ZPL ^GFA packing.

Reuses the exact conversion contract validated and documented on the
Ops-in-Net "Infinite Scroll" Knowledge page: 8 dots per byte, black = 1
bit, uppercase hex, self-contained ^GFA job.
"""
from dataclasses import dataclass

from PIL import Image


def to_1bit(img: Image.Image) -> Image.Image:
    """Convert a grayscale image to 1-bit using Floyd-Steinberg dithering
    (PIL's default when converting L -> 1)."""
    return img.convert("L").convert("1")


@dataclass
class ZplJob:
    text: str
    total_bytes: int
    row_bytes: int
    width: int
    height: int


def pack_to_zpl(img_1bit: Image.Image) -> ZplJob:
    w, h = img_1bit.size
    row_bytes = (w + 7) // 8
    total_bytes = row_bytes * h
    px = img_1bit.load()
    out = bytearray(total_bytes)
    idx = 0
    for y in range(h):
        byte = 0
        bitcount = 0
        for x in range(w):
            black = 1 if px[x, y] == 0 else 0
            byte = (byte << 1) | black
            bitcount += 1
            if bitcount == 8:
                out[idx] = byte
                idx += 1
                byte = 0
                bitcount = 0
        if bitcount:
            byte <<= (8 - bitcount)
            out[idx] = byte
            idx += 1
    hexdata = out.hex().upper()
    zpl_text = (
        "^XA\n"
        f"^PW{w}\n"
        f"^LL{h}\n"
        "^LH0,0\n"
        "^FO0,0\n"
        f"^GFA,{total_bytes},{total_bytes},{row_bytes},{hexdata}\n"
        "^XZ\n"
    )
    return ZplJob(text=zpl_text, total_bytes=total_bytes, row_bytes=row_bytes, width=w, height=h)
```

- [ ] **Step 2: Write deterministic packing tests**

`tests/test_dither.py`:
```python
from PIL import Image

from app.dither import pack_to_zpl, to_1bit


def test_to_1bit_converts_mode_and_preserves_size():
    img = Image.new("L", (10, 6), 128)
    out = to_1bit(img)
    assert out.mode == "1"
    assert out.size == (10, 6)


def test_pack_to_zpl_packs_a_known_bit_pattern():
    # Row 0: alternating black/white starting black -> 10101010 = 0xAA
    # Row 1: all black -> 11111111 = 0xFF
    img = Image.new("1", (8, 2), 255)
    for x in range(8):
        img.putpixel((x, 0), 0 if x % 2 == 0 else 255)
    for x in range(8):
        img.putpixel((x, 1), 0)

    job = pack_to_zpl(img)

    assert job.width == 8
    assert job.height == 2
    assert job.row_bytes == 1
    assert job.total_bytes == 2
    assert job.text.startswith("^XA\n^PW8\n^LL2\n^LH0,0\n^FO0,0\n")
    assert "^GFA,2,2,1,AAFF\n" in job.text
    assert job.text.rstrip().endswith("^XZ")
```

- [ ] **Step 3: Run the dither tests**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.pillow ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_dither.py -v"`
Expected: PASS (2 tests).

- [ ] **Step 4: Write the end-to-end pipeline regression test (bootstrap form)**

`tests/test_pipeline.py`:
```python
import hashlib
import json
from pathlib import Path

from app.dither import pack_to_zpl, to_1bit
from app.render import DEFAULT_CSS_PATH, render_post_png

FIXTURE = json.loads((Path(__file__).parent / "fixtures" / "sample_post.json").read_text())

# Golden hash of the ZPL job text produced from FIXTURE + the default CSS.
# Filled in below once, from this test's own failure message (see Task 3,
# Step 5 of the implementation plan) — this is a regression guard against
# the template, default CSS, or packing logic silently changing.
EXPECTED_SHA256 = "PENDING"


def test_pipeline_is_stable_for_the_fixed_sample_post():
    css = DEFAULT_CSS_PATH.read_text()
    img = render_post_png(FIXTURE, css)
    bitimg = to_1bit(img)
    job = pack_to_zpl(bitimg)
    actual = hashlib.sha256(job.text.encode("ascii")).hexdigest()

    if EXPECTED_SHA256 == "PENDING":
        raise AssertionError(f"first run — set EXPECTED_SHA256 to: {actual}")
    assert actual == EXPECTED_SHA256
```

- [ ] **Step 5: Run once to capture the golden hash**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.flask ps.weasyprint ps.pillow ps.jinja2 ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_pipeline.py -v"`
Expected: FAIL, with an assertion message like `first run — set EXPECTED_SHA256 to: <64 hex chars>`.

Copy that hash into `EXPECTED_SHA256` in `tests/test_pipeline.py`, replacing `"PENDING"`.

- [ ] **Step 6: Re-run to confirm it now passes**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.flask ps.weasyprint ps.pillow ps.jinja2 ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_pipeline.py -v"`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
cd /home/stags/Source/infinite-scroll
git add app/dither.py tests/test_dither.py tests/test_pipeline.py
git commit -m "Add dithering, ZPL packing, and a pipeline regression test"
```

---

### Task 4: Printer module — serialized, bounded-timeout writes

**Files:**
- Create: `app/printer.py`
- Test: `tests/test_printer.py`

**Interfaces:**
- Consumes: nothing from earlier tasks (pure I/O module).
- Produces:
  - `app.printer.PrintResult` — dataclass with `success: bool`, `message: str`
  - `app.printer.print_zpl(zpl_text: str, device_path: str | None = None, timeout_s: float = 15.0) -> PrintResult`

- [ ] **Step 1: Write the printer module**

`app/printer.py`:
```python
"""Serialized, bounded-timeout writes to the Arkscan's raw USB device.

A write that blocks forever (stuck printer, no reader) must not hang the
whole web app, and two prints must never write to the device concurrently
-- both were observed as real failure modes while printing manually.
"""
import os
import stat
import threading
from dataclasses import dataclass


@dataclass
class PrintResult:
    success: bool
    message: str


_print_lock = threading.Lock()


def _device_path() -> str:
    return os.environ.get("INFINITE_SCROLL_PRINTER_DEVICE", "/dev/usb/lp0")


def _preflight(zpl_text: str, device_path: str) -> str | None:
    if not zpl_text:
        return "job is empty"
    if not os.path.exists(device_path):
        return f"printer device {device_path} not present"
    if not stat.S_ISCHR(os.stat(device_path).st_mode):
        return f"{device_path} is not a character device"
    return None


def print_zpl(zpl_text: str, device_path: str | None = None, timeout_s: float = 15.0) -> PrintResult:
    device_path = device_path or _device_path()

    error = _preflight(zpl_text, device_path)
    if error:
        return PrintResult(success=False, message=error)

    if not _print_lock.acquire(blocking=False):
        return PrintResult(success=False, message="printer busy: a previous print is still in progress")

    done = threading.Event()
    outcome: dict = {}

    def _write():
        try:
            with open(device_path, "wb") as f:
                f.write(zpl_text.encode("ascii"))
                f.flush()
            outcome["ok"] = True
        except OSError as exc:
            outcome["ok"] = False
            outcome["error"] = str(exc)
        finally:
            done.set()
            _print_lock.release()

    threading.Thread(target=_write, daemon=True).start()

    if done.wait(timeout_s):
        if outcome.get("ok"):
            return PrintResult(success=True, message="printed")
        return PrintResult(success=False, message=f"write failed: {outcome.get('error')}")

    return PrintResult(
        success=False,
        message=f"write to printer timed out after {timeout_s:.0f}s — check paper, cover, and power on the Arkscan",
    )
```

Note: on timeout, the lock is deliberately **not** released by `print_zpl` itself — only the background thread's `finally` releases it, once the write actually finishes (however long that takes). This is what makes the next request see "printer busy" instead of racing a second write onto the same device.

- [ ] **Step 2: Write the printer tests**

`tests/test_printer.py`:
```python
import os
from unittest.mock import patch

from app.printer import PrintResult, print_zpl


def test_print_zpl_rejects_empty_job(tmp_path):
    device = tmp_path / "lp0"
    device.write_bytes(b"")
    result = print_zpl("", device_path=str(device))
    assert result == PrintResult(success=False, message="job is empty")


def test_print_zpl_rejects_missing_device(tmp_path):
    missing = tmp_path / "does-not-exist"
    result = print_zpl("^XA^XZ", device_path=str(missing))
    assert result.success is False
    assert "not present" in result.message


def test_print_zpl_rejects_non_char_device(tmp_path):
    regular_file = tmp_path / "not-a-device"
    regular_file.write_bytes(b"")
    with patch("app.printer.stat.S_ISCHR", return_value=False):
        result = print_zpl("^XA^XZ", device_path=str(regular_file))
    assert result.success is False
    assert "not a character device" in result.message


def test_print_zpl_writes_when_device_accepts_data(tmp_path):
    device = tmp_path / "fake-lp0"
    device.write_bytes(b"")
    with patch("app.printer.stat.S_ISCHR", return_value=True):
        result = print_zpl("^XA^XZ", device_path=str(device))
    assert result == PrintResult(success=True, message="printed")
    assert device.read_bytes() == b"^XA^XZ"


def test_print_zpl_times_out_on_a_stuck_write(tmp_path):
    fifo_path = tmp_path / "stuck-lp0"
    os.mkfifo(fifo_path)
    with patch("app.printer.stat.S_ISCHR", return_value=True):
        result = print_zpl("^XA^XZ", device_path=str(fifo_path), timeout_s=0.5)
    assert result.success is False
    assert "timed out" in result.message


def test_print_zpl_reports_busy_while_a_stuck_write_holds_the_lock(tmp_path):
    fifo_path = tmp_path / "stuck-lp0-2"
    os.mkfifo(fifo_path)
    with patch("app.printer.stat.S_ISCHR", return_value=True):
        first = print_zpl("^XA^XZ", device_path=str(fifo_path), timeout_s=0.3)
        second = print_zpl("^XA^XZ", device_path=str(fifo_path), timeout_s=0.3)
    assert first.success is False
    assert "timed out" in first.message
    assert second.success is False
    assert "busy" in second.message
```

- [ ] **Step 3: Run the tests**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_printer.py -v"`
Expected: PASS (6 tests). The two FIFO-based tests take ~0.3–0.5s each (real wall-clock timeout) — that's expected, not a hang.

- [ ] **Step 4: Commit**

```bash
cd /home/stags/Source/infinite-scroll
git add app/printer.py tests/test_printer.py
git commit -m "Add serialized, bounded-timeout printer writes"
```

---

### Task 5: Record-keeping

**Files:**
- Create: `app/record.py`
- Test: `tests/test_record.py`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `app.record.slugify(name: str) -> str`
  - `app.record.save_print_job(slug: str, png_bytes: bytes, zpl_bytes: bytes, base_dir: pathlib.Path) -> dict` — returns `{"png_path": str, "zpl_path": str, "png_sha256": str, "zpl_sha256": str, "timestamp": str}`

- [ ] **Step 1: Write the record module**

`app/record.py`:
```python
"""Record-keeping for printed posts: slugify names and persist each job's
PNG + ZPL under the print-ready directory, matching the convention already
used for the manually-printed batch documented on the Knowledge page."""
import hashlib
import re
import time
from pathlib import Path


def slugify(name: str) -> str:
    slug = name.strip().lower()
    slug = re.sub(r"[^a-z0-9]+", "-", slug)
    return slug.strip("-") or "post"


def save_print_job(slug: str, png_bytes: bytes, zpl_bytes: bytes, base_dir: Path) -> dict:
    base_dir = Path(base_dir)
    base_dir.mkdir(parents=True, exist_ok=True)
    timestamp = time.strftime("%Y%m%dT%H%M%S")
    png_path = base_dir / f"{slug}-{timestamp}.png"
    zpl_path = base_dir / f"{slug}-{timestamp}.zpl"
    png_path.write_bytes(png_bytes)
    zpl_path.write_bytes(zpl_bytes)
    return {
        "png_path": str(png_path),
        "zpl_path": str(zpl_path),
        "png_sha256": hashlib.sha256(png_bytes).hexdigest(),
        "zpl_sha256": hashlib.sha256(zpl_bytes).hexdigest(),
        "timestamp": timestamp,
    }
```

- [ ] **Step 2: Write the tests**

`tests/test_record.py`:
```python
import hashlib
from pathlib import Path

from app.record import save_print_job, slugify


def test_slugify_lowercases_and_replaces_non_alnum():
    assert slugify("Reginald Huxtable-Vance III") == "reginald-huxtable-vance-iii"


def test_slugify_falls_back_to_post_for_empty_input():
    assert slugify("   ") == "post"


def test_save_print_job_writes_both_files_and_returns_matching_hashes(tmp_path):
    png_bytes = b"png-bytes"
    zpl_bytes = b"zpl-bytes"
    result = save_print_job("brayden-steelworth", png_bytes, zpl_bytes, tmp_path)

    assert Path(result["png_path"]).read_bytes() == png_bytes
    assert Path(result["zpl_path"]).read_bytes() == zpl_bytes
    assert result["png_sha256"] == hashlib.sha256(png_bytes).hexdigest()
    assert result["zpl_sha256"] == hashlib.sha256(zpl_bytes).hexdigest()
    assert result["png_path"].endswith(".png")
    assert result["zpl_path"].endswith(".zpl")
```

- [ ] **Step 3: Run the tests**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_record.py -v"`
Expected: PASS (3 tests).

- [ ] **Step 4: Commit**

```bash
cd /home/stags/Source/infinite-scroll
git add app/record.py tests/test_record.py
git commit -m "Add print-job record-keeping"
```

---

### Task 6: Compose routes

**Files:**
- Create: `app/routes.py`
- Create: `app/templates/compose.html`
- Modify: `app/__init__.py` (register the blueprint)
- Test: `tests/test_routes.py`

**Interfaces:**
- Consumes: `app.config.current_css`, `app.config.print_ready_dir` (Task 2); `app.dither.to_1bit`, `app.dither.pack_to_zpl` (Task 3); `app.printer.print_zpl` (Task 4); `app.record.save_print_job`, `app.record.slugify` (Task 5); `app.render.render_post_png` (Task 2).
- Produces: Flask blueprint `app.routes.bp` with `GET /` and `POST /print` (JSON response: `{"success": bool, "message": str, "record": dict | None}`).

- [ ] **Step 1: Write the compose routes**

`app/routes.py`:
```python
"""Compose-and-print routes: GET / renders the form, POST /print renders,
dithers, packs, and prints one post."""
import io

from flask import Blueprint, jsonify, render_template, request

from app.config import current_css, print_ready_dir
from app.dither import pack_to_zpl, to_1bit
from app.printer import print_zpl
from app.record import save_print_job, slugify
from app.render import render_post_png

bp = Blueprint("compose", __name__)

REQUIRED_FIELDS = ["name", "title", "avatar_initials", "body"]
INT_FIELDS = ["reactions", "comments", "reposts"]


def _parse_fields(form) -> dict:
    fields = {key: form.get(key, "").strip() for key in REQUIRED_FIELDS}
    for key in INT_FIELDS:
        raw = form.get(key, "0").strip()
        fields[key] = int(raw) if raw.isdigit() else 0
    return fields


def _png_bytes(img) -> bytes:
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    return buf.getvalue()


@bp.get("/")
def compose_form():
    return render_template("compose.html")


@bp.post("/print")
def do_print():
    fields = _parse_fields(request.form)
    missing = [key for key in REQUIRED_FIELDS if not fields[key]]
    if missing:
        return jsonify(
            success=False,
            message=f"missing required field(s): {', '.join(missing)}",
            record=None,
        ), 400

    img = render_post_png(fields, current_css())
    bitimg = to_1bit(img)
    job = pack_to_zpl(bitimg)

    result = print_zpl(job.text)

    record = None
    if result.success:
        slug = slugify(fields["name"])
        record = save_print_job(slug, _png_bytes(img), job.text.encode("ascii"), print_ready_dir())

    status_code = 200 if result.success else 502
    return jsonify(success=result.success, message=result.message, record=record), status_code
```

- [ ] **Step 2: Write the compose form page**

`app/templates/compose.html`:
```html
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Infinite Scroll — Compose</title>
<style>
  body { font-family: sans-serif; max-width: 640px; margin: 40px auto; }
  label { display: block; margin-top: 12px; font-weight: 600; }
  input, textarea { width: 100%; box-sizing: border-box; padding: 6px; font-size: 15px; }
  textarea { height: 160px; }
  button { margin-top: 16px; padding: 10px 20px; font-size: 15px; }
  #status { margin-top: 12px; font-weight: 600; }
  nav a { margin-right: 16px; }
</style>
</head>
<body>
<nav><a href="/">Compose</a><a href="/design">Design</a></nav>
<h1>Compose a post</h1>
<form id="compose-form">
  <label>Name <input name="name" required></label>
  <label>Title <input name="title" required></label>
  <label>Avatar initials <input name="avatar_initials" maxlength="3" required></label>
  <label>Body (blank line between paragraphs) <textarea name="body" required></textarea></label>
  <label>Reactions <input name="reactions" type="number" value="0"></label>
  <label>Comments <input name="comments" type="number" value="0"></label>
  <label>Reposts <input name="reposts" type="number" value="0"></label>
  <button type="submit">Print</button>
</form>
<div id="status"></div>
<script>
document.getElementById("compose-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const status = document.getElementById("status");
  status.textContent = "printing…";
  const formData = new FormData(e.target);
  const resp = await fetch("/print", { method: "POST", body: formData });
  const data = await resp.json();
  status.textContent = data.success ? "printed ✓" : `failed — ${data.message}`;
});
</script>
</body>
</html>
```

- [ ] **Step 3: Register the blueprint**

Modify `app/__init__.py`:
```python
from flask import Flask

from app.routes import bp as compose_bp


def create_app() -> Flask:
    app = Flask(__name__)
    app.register_blueprint(compose_bp)

    @app.get("/health")
    def health():
        return {"status": "ok"}

    return app
```

- [ ] **Step 4: Write the route tests**

`tests/test_routes.py`:
```python
from unittest.mock import patch

from app import create_app
from app.printer import PrintResult

SAMPLE_FORM = {
    "name": "Test Poster",
    "title": "Testing · QA",
    "avatar_initials": "TP",
    "body": "Paragraph one.\n\nParagraph two.",
    "reactions": "10",
    "comments": "2",
    "reposts": "1",
}


def test_compose_form_renders():
    client = create_app().test_client()
    resp = client.get("/")
    assert resp.status_code == 200
    assert b"Compose a post" in resp.data


def test_print_rejects_missing_required_field():
    client = create_app().test_client()
    form = dict(SAMPLE_FORM)
    del form["name"]
    resp = client.post("/print", data=form)
    assert resp.status_code == 400
    assert "name" in resp.get_json()["message"]


def test_print_success_saves_a_record(tmp_path, monkeypatch):
    monkeypatch.setenv("INFINITE_SCROLL_PRINT_READY_DIR", str(tmp_path))
    with patch("app.routes.print_zpl", return_value=PrintResult(success=True, message="printed")):
        client = create_app().test_client()
        resp = client.post("/print", data=SAMPLE_FORM)

    assert resp.status_code == 200
    body = resp.get_json()
    assert body["success"] is True
    assert body["record"]["png_path"].startswith(str(tmp_path))


def test_print_failure_does_not_save_a_record(tmp_path, monkeypatch):
    monkeypatch.setenv("INFINITE_SCROLL_PRINT_READY_DIR", str(tmp_path))
    with patch("app.routes.print_zpl", return_value=PrintResult(success=False, message="printer busy")):
        client = create_app().test_client()
        resp = client.post("/print", data=SAMPLE_FORM)

    assert resp.status_code == 502
    body = resp.get_json()
    assert body["success"] is False
    assert body["record"] is None
    assert list(tmp_path.iterdir()) == []
```

- [ ] **Step 5: Run the tests**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.flask ps.weasyprint ps.pillow ps.jinja2 ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_routes.py -v"`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
cd /home/stags/Source/infinite-scroll
git add app/routes.py app/templates/compose.html app/__init__.py tests/test_routes.py
git commit -m "Add compose form and print route"
```

---

### Task 7: Design routes

**Files:**
- Create: `app/design_routes.py`
- Create: `app/templates/design.html`
- Modify: `app/__init__.py` (register the second blueprint)
- Test: `tests/test_design_routes.py`

**Interfaces:**
- Consumes: `app.config.css_path`, `app.config.current_css` (Task 2); `app.render.render_post_png` (Task 2).
- Produces: Flask blueprint `app.design_routes.bp` with `GET /design`, `POST /design/preview` (JSON `{"image": "data:image/png;base64,..."}`), `POST /design` (JSON `{"success": true}`).

- [ ] **Step 1: Write the design routes**

`app/design_routes.py`:
```python
"""CSS design editor routes: view/edit the stylesheet used to render posts,
with a live preview rendered against a fixed sample post."""
import base64
import io

from flask import Blueprint, jsonify, render_template, request

from app.config import css_path, current_css
from app.render import render_post_png

bp = Blueprint("design", __name__)

SAMPLE_FIELDS = {
    "name": "Sample Poster",
    "title": "Preview Persona · Design Sandbox",
    "avatar_initials": "SP",
    "body": (
        "This is sample body text so you can see how paragraph spacing "
        "and line length look.\n\nA second paragraph, for good measure."
    ),
    "reactions": 12345,
    "comments": 678,
    "reposts": 90,
}


def _png_data_url(img) -> str:
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    encoded = base64.b64encode(buf.getvalue()).decode("ascii")
    return f"data:image/png;base64,{encoded}"


@bp.get("/design")
def design_editor():
    return render_template("design.html", css_text=current_css())


@bp.post("/design/preview")
def design_preview():
    css_text = request.get_json(force=True).get("css", "")
    img = render_post_png(SAMPLE_FIELDS, css_text)
    return jsonify(image=_png_data_url(img))


@bp.post("/design")
def design_save():
    css_text = request.get_json(force=True).get("css", "")
    path = css_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(css_text)
    return jsonify(success=True)
```

- [ ] **Step 2: Write the design editor page**

`app/templates/design.html`:
```html
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Infinite Scroll — Design</title>
<style>
  body { font-family: sans-serif; margin: 0; display: flex; height: 100vh; }
  nav { position: absolute; top: 12px; left: 12px; }
  nav a { margin-right: 16px; }
  .editor, .preview { flex: 1; padding: 60px 20px 20px; box-sizing: border-box; overflow: auto; }
  textarea { width: 100%; height: 90%; font-family: monospace; font-size: 13px; box-sizing: border-box; }
  #save-status { margin-top: 8px; font-weight: 600; }
  .preview img { max-width: 100%; border: 1px solid #ccc; }
</style>
</head>
<body>
<nav><a href="/">Compose</a><a href="/design">Design</a></nav>
<div class="editor">
  <h2>CSS</h2>
  <textarea id="css-editor">{{ css_text }}</textarea>
  <br>
  <button id="save-btn">Save</button>
  <span id="save-status"></span>
</div>
<div class="preview">
  <h2>Live preview</h2>
  <img id="preview-img" alt="preview">
</div>
<script>
const editor = document.getElementById("css-editor");
const previewImg = document.getElementById("preview-img");
const saveStatus = document.getElementById("save-status");
let debounceTimer = null;

async function renderPreview() {
  const resp = await fetch("/design/preview", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ css: editor.value }),
  });
  const data = await resp.json();
  previewImg.src = data.image;
}

editor.addEventListener("input", () => {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(renderPreview, 500);
});

document.getElementById("save-btn").addEventListener("click", async () => {
  const resp = await fetch("/design", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ css: editor.value }),
  });
  const data = await resp.json();
  saveStatus.textContent = data.success ? "saved ✓" : "save failed";
  setTimeout(() => { saveStatus.textContent = ""; }, 2000);
});

renderPreview();
</script>
</body>
</html>
```

- [ ] **Step 3: Register the second blueprint**

Modify `app/__init__.py`:
```python
from flask import Flask

from app.design_routes import bp as design_bp
from app.routes import bp as compose_bp


def create_app() -> Flask:
    app = Flask(__name__)
    app.register_blueprint(compose_bp)
    app.register_blueprint(design_bp)

    @app.get("/health")
    def health():
        return {"status": "ok"}

    return app
```

- [ ] **Step 4: Write the design route tests**

`tests/test_design_routes.py`:
```python
from app import create_app


def test_design_editor_shows_current_css(tmp_path, monkeypatch):
    css_file = tmp_path / "template.css"
    css_file.write_text(".post { color: red; }")
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(css_file))

    client = create_app().test_client()
    resp = client.get("/design")
    assert resp.status_code == 200
    assert b".post { color: red; }" in resp.data


def test_design_editor_falls_back_to_default_css_when_none_saved(tmp_path, monkeypatch):
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(tmp_path / "does-not-exist.css"))
    client = create_app().test_client()
    resp = client.get("/design")
    assert resp.status_code == 200
    assert b".post {" in resp.data


def test_design_preview_returns_a_data_url_png():
    client = create_app().test_client()
    resp = client.post("/design/preview", json={"css": ".post { color: #000; }"})
    assert resp.status_code == 200
    assert resp.get_json()["image"].startswith("data:image/png;base64,")


def test_design_save_writes_the_css_file(tmp_path, monkeypatch):
    css_file = tmp_path / "webapp" / "template.css"
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(css_file))
    client = create_app().test_client()
    resp = client.post("/design", json={"css": ".post { color: blue; }"})
    assert resp.status_code == 200
    assert resp.get_json()["success"] is True
    assert css_file.read_text() == ".post { color: blue; }"
```

- [ ] **Step 5: Run the tests**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.flask ps.weasyprint ps.pillow ps.jinja2 ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest tests/test_design_routes.py -v"`
Expected: PASS (4 tests).

- [ ] **Step 6: Run the full test suite**

Run: `nix-shell -p 'python3.withPackages(ps: [ps.flask ps.weasyprint ps.pillow ps.jinja2 ps.pytest])' --run "cd /home/stags/Source/infinite-scroll && pytest -v"`
Expected: PASS, all tests across all files.

- [ ] **Step 7: Commit**

```bash
cd /home/stags/Source/infinite-scroll
git add app/design_routes.py app/templates/design.html app/__init__.py tests/test_design_routes.py
git commit -m "Add CSS design editor with live preview"
```

---

### Task 8: Deploy to the Pi

**Files:**
- Create: `deploy/infinite-scroll-webapp.service`

**Interfaces:**
- Consumes: the whole `app/` package and `requirements.txt` from earlier tasks.
- Produces: a running `infinite-scroll-webapp.service` on `infinite-scroll.local`, reachable at `http://infinite-scroll.local:8080`.

- [ ] **Step 1: Write the systemd unit**

`deploy/infinite-scroll-webapp.service`:
```ini
[Unit]
Description=Infinite Scroll post composer web app
After=network.target

[Service]
Type=simple
User=stags
SupplementaryGroups=lp
WorkingDirectory=/home/stags/infinite-scroll-webapp
Environment=INFINITE_SCROLL_CSS_PATH=/var/lib/infinite-scroll/webapp/template.css
Environment=INFINITE_SCROLL_PRINT_READY_DIR=/var/lib/infinite-scroll/print-ready
ExecStart=/home/stags/infinite-scroll-webapp/venv/bin/python /home/stags/infinite-scroll-webapp/run.py
Restart=on-failure
RestartSec=2

[Install]
WantedBy=multi-user.target
```

`SupplementaryGroups=lp` grants the service access to `/dev/usb/lp0` (group-owned `root:lp`) without running as root or shelling out to `sudo` — the same access the manual `sudo cat > /dev/usb/lp0` procedure needed, granted structurally instead.

- [ ] **Step 2: Commit**

```bash
cd /home/stags/Source/infinite-scroll
git add deploy/infinite-scroll-webapp.service
git commit -m "Add systemd unit for the webapp"
```

- [ ] **Step 3: Copy the repo to the Pi**

```bash
rsync -av --exclude='.git' --exclude='__pycache__' --exclude='*.pyc' \
  /home/stags/Source/infinite-scroll/ stags@infinite-scroll.local:/home/stags/infinite-scroll-webapp/
```

Expected: file listing ends with a summary line (e.g. `sent N bytes ... total size is ...`) and no `rsync error`.

- [ ] **Step 4: Install system and Python dependencies on the Pi**

```bash
ssh stags@infinite-scroll.local '
set -e
sudo apt-get update
sudo apt-get install -y python3-venv python3-pip poppler-utils \
  libpango-1.0-0 libpangocairo-1.0-0 libgdk-pixbuf2.0-0 libffi-dev \
  shared-mime-info fonts-dejavu-core
cd /home/stags/infinite-scroll-webapp
python3 -m venv venv
./venv/bin/pip install --upgrade pip
./venv/bin/pip install -r requirements.txt
'
```

Expected: exits without error; last `pip install` line ends with `Successfully installed ...` including `flask`, `weasyprint`, `Pillow`, `Jinja2`.

- [ ] **Step 5: Seed the default CSS and create the webapp state directory**

```bash
ssh stags@infinite-scroll.local '
set -e
sudo mkdir -p /var/lib/infinite-scroll/webapp
sudo chown stags:stags /var/lib/infinite-scroll/webapp
cp /home/stags/infinite-scroll-webapp/app/default_template.css /var/lib/infinite-scroll/webapp/template.css
'
```

- [ ] **Step 6: Install and start the systemd service**

```bash
ssh stags@infinite-scroll.local '
set -e
sudo cp /home/stags/infinite-scroll-webapp/deploy/infinite-scroll-webapp.service /etc/systemd/system/infinite-scroll-webapp.service
sudo systemctl daemon-reload
sudo systemctl enable --now infinite-scroll-webapp.service
sleep 2
sudo systemctl status infinite-scroll-webapp.service --no-pager
'
```

Expected: status shows `Active: active (running)`.

- [ ] **Step 7: Verify from outside the Pi**

```bash
curl -sf http://infinite-scroll.local:8080/health
```

Expected: `{"status":"ok"}`

---

### Task 9: Manual acceptance pass

**Files:** none (verification only).

- [ ] **Step 1: Print one real post end-to-end**

Open `http://infinite-scroll.local:8080/` in a browser on the LAN, fill in all fields with real values, click Print. Confirm the UI reports `printed ✓` and that the Arkscan physically produces the post on paper, matching what's on screen (allowing for monochrome dithering).

- [ ] **Step 2: Confirm the print-ready record was written**

```bash
ssh stags@infinite-scroll.local 'ls -la /var/lib/infinite-scroll/print-ready/ | tail -5'
```

Expected: a `<slug>-<timestamp>.png` and `<slug>-<timestamp>.zpl` matching the post just printed.

- [ ] **Step 3: Edit the CSS and confirm the live preview updates**

Open `http://infinite-scroll.local:8080/design`, change a visible property (e.g. `.name { font-size: 20px; }` → `32px`), and confirm the preview image on the right visibly updates within ~1s of pausing typing, without clicking Save.

- [ ] **Step 4: Save the CSS change and confirm it persists**

Click Save, confirm `saved ✓` appears, then reload `/design` and confirm the textarea still shows the edited CSS (not the original default).

- [ ] **Step 5: Print again and confirm the style change took**

Go back to `/`, print another post, and confirm the physical printout reflects the CSS change from Step 3 (e.g. the name is visibly larger).

- [ ] **Step 6: Update the Knowledge page**

Add a short entry to the Ops-in-Net "Infinite Scroll" Knowledge page (`https://ops.in.net/account/knowledge/infinite-scroll`) noting the compose/design web app now exists, its URL, and where its CSS and print-ready records live — following the same documentation style already used there for the manual batch-printing procedure.
