#!/usr/bin/env python3
"""
Infinite Scroll test-post generator (reference/prototype).

Reproduces the conversion contract documented on the Ops-in-Net Knowledge
page "Infinite Scroll":

    pre-rendered raster
      -> normalize/wrap to a 650-dot canvas
      -> 1-bit Floyd-Steinberg conversion
      -> pack rows left-to-right, 8 dots per byte
      -> encode black dots as 1 bits
      -> uppercase hex
      -> self-contained ZPL ^GFA job

Generates fictional/satirical "professional network" posts. Content is
entirely invented parody, not scraped or attributed to any real account.
"""
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

CANVAS_W = 650
MARGIN = 48
CONTENT_W = CANVAS_W - 2 * MARGIN
WHITE = 255
BLACK = 0


def font_path(pattern, filename):
    """Resolve fonts portably on NixOS and Raspberry Pi OS."""
    if shutil.which("fc-match"):
        match = subprocess.check_output(
            ["fc-match", "-f", "%{file}", pattern], text=True
        ).strip()
        if match:
            return match
    fallback = Path("/usr/share/fonts/truetype/dejavu") / filename
    if fallback.is_file():
        return str(fallback)
    raise RuntimeError(f"cannot find {pattern}; install DejaVu fonts and fontconfig")


F_REGULAR = font_path("DejaVu Sans", "DejaVuSans.ttf")
F_BOLD = font_path("DejaVu Sans:style=Bold", "DejaVuSans-Bold.ttf")


def wrap_text(draw, text, font, max_width):
    words = text.split()
    lines, cur = [], ""
    for w in words:
        trial = (cur + " " + w).strip()
        if draw.textlength(trial, font=font) <= max_width:
            cur = trial
        else:
            if cur:
                lines.append(cur)
            cur = w
    if cur:
        lines.append(cur)
    return lines


def render_post(name, title, body, reactions, comments, reposts, avatar_initials):
    f_name = ImageFont.truetype(F_BOLD, 30)
    f_title = ImageFont.truetype(F_REGULAR, 20)
    f_meta = ImageFont.truetype(F_REGULAR, 17)
    f_body = ImageFont.truetype(F_REGULAR, 22)
    f_stat = ImageFont.truetype(F_REGULAR, 18)
    f_label = ImageFont.truetype(F_BOLD, 16)
    f_avatar = ImageFont.truetype(F_BOLD, 26)

    scratch = Image.new("L", (CANVAS_W, 50), WHITE)
    d = ImageDraw.Draw(scratch)

    y = MARGIN
    avatar_d = 64
    avatar_x = MARGIN

    header_text_x = avatar_x + avatar_d + 16
    header_text_w = CONTENT_W - avatar_d - 16

    name_lines = wrap_text(d, name, f_name, header_text_w)
    title_lines = wrap_text(d, title, f_title, header_text_w)

    header_h = max(avatar_d, len(name_lines) * 36 + len(title_lines) * 26 + 22)
    y += header_h + 22

    body_lines = []
    for para in body.split("\n\n"):
        body_lines.extend(wrap_text(d, para, f_body, CONTENT_W))
        body_lines.append("")
    if body_lines and body_lines[-1] == "":
        body_lines.pop()

    body_h = len(body_lines) * 30
    y += body_h + 26

    y += 2  # rule
    y += 20
    y += 2  # rule
    y += 46  # reaction row
    y += 30  # footer disclaimer lines
    y += 22
    y += MARGIN

    total_h = y
    img = Image.new("L", (CANVAS_W, total_h), WHITE)
    d = ImageDraw.Draw(img)

    cy = MARGIN
    d.ellipse([avatar_x, cy, avatar_x + avatar_d, cy + avatar_d], outline=BLACK, width=3)
    bbox = d.textbbox((0, 0), avatar_initials, font=f_avatar)
    tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
    d.text(
        (avatar_x + avatar_d / 2 - tw / 2 - bbox[0], cy + avatar_d / 2 - th / 2 - bbox[1]),
        avatar_initials,
        font=f_avatar,
        fill=BLACK,
    )

    ty = cy
    for line in name_lines:
        d.text((header_text_x, ty), line, font=f_name, fill=BLACK)
        ty += 36
    for line in title_lines:
        d.text((header_text_x, ty), line, font=f_title, fill=BLACK)
        ty += 26
    d.text((header_text_x, ty), "2h · Edited · Public", font=f_meta, fill=BLACK)

    y = MARGIN + header_h + 22
    for line in body_lines:
        d.text((MARGIN, y), line, font=f_body, fill=BLACK)
        y += 30

    y += 6
    d.line([(MARGIN, y), (CANVAS_W - MARGIN, y)], fill=BLACK, width=2)
    y += 20
    stat_line = f"{reactions:,} reactions · {comments:,} comments · {reposts:,} reposts"
    d.text((MARGIN, y), stat_line, font=f_stat, fill=BLACK)
    y += 26
    d.line([(MARGIN, y), (CANVAS_W - MARGIN, y)], fill=BLACK, width=2)
    y += 20

    label1 = "FICTIONAL TEST POST · NOT LINKEDIN DATA"
    label2 = "FICTIONAL SATIRE"
    d.text((MARGIN, y), label1, font=f_label, fill=BLACK)
    y += 24
    d.text((MARGIN, y), label2, font=f_label, fill=BLACK)

    return img


def floyd_steinberg_1bit(img_l):
    return img_l.convert("1")  # PIL dithers L->1 with Floyd-Steinberg by default


def pack_to_zpl(img_1bit, gap_after_generate=True):
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
    zpl = (
        "^XA\n"
        f"^PW{w}\n"
        f"^LL{h}\n"
        "^LH0,0\n"
        "^FO0,0\n"
        f"^GFA,{total_bytes},{total_bytes},{row_bytes},{hexdata}\n"
        "^XZ\n"
    )
    return zpl, total_bytes, row_bytes


def sha256_bytes(b):
    return hashlib.sha256(b).hexdigest()


def generate(slug, name, title, body, reactions, comments, reposts, avatar_initials, outdir):
    outdir = Path(outdir)
    outdir.mkdir(parents=True, exist_ok=True)

    img_gray = render_post(name, title, body, reactions, comments, reposts, avatar_initials)
    gray_path = outdir / f"{slug}.png"
    img_gray.save(gray_path)

    img_1bit = floyd_steinberg_1bit(img_gray)
    bit_path = outdir / f"{slug}-1bit.png"
    img_1bit.save(bit_path)

    zpl, total_bytes, row_bytes = pack_to_zpl(img_1bit)
    zpl_path = outdir / f"{slug}.zpl"
    zpl_path.write_bytes(zpl.encode("ascii"))

    report = {
        "slug": slug,
        "name": name,
        "png": str(gray_path),
        "png_size": gray_path.stat().st_size,
        "png_sha256": sha256_bytes(gray_path.read_bytes()),
        "png_dims": img_gray.size,
        "bit_png": str(bit_path),
        "bit_png_size": bit_path.stat().st_size,
        "bit_png_sha256": sha256_bytes(bit_path.read_bytes()),
        "zpl": str(zpl_path),
        "zpl_size": zpl_path.stat().st_size,
        "zpl_sha256": sha256_bytes(zpl_path.read_bytes()),
        "dims": img_1bit.size,
    }
    return report


if __name__ == "__main__":
    import json

    spec_path = sys.argv[1]
    outdir = sys.argv[2]
    with open(spec_path) as f:
        specs = json.load(f)

    reports = []
    for spec in specs:
        r = generate(outdir=outdir, **spec)
        reports.append(r)
        print(f"{r['slug']}: png={r['png_size']}B 1bit={r['bit_png_size']}B zpl={r['zpl_size']}B dims={r['dims']}")

    with open(Path(outdir) / "manifest.json", "w") as f:
        json.dump(reports, f, indent=2)
