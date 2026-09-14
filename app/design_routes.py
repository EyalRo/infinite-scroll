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
    try:
        img = render_post_png(SAMPLE_FIELDS, css_text)
    except Exception as exc:
        return jsonify(error=f"render failed: {exc}"), 502
    return jsonify(image=_png_data_url(img))


@bp.post("/design")
def design_save():
    css_text = request.get_json(force=True).get("css", "")
    path = css_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(css_text)
    return jsonify(success=True)
