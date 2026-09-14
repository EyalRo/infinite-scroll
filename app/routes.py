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
