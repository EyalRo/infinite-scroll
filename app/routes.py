"""Dashboard, immediate-print, backlog, and scheduler HTTP routes."""
import io
import random
import time
import uuid
from pathlib import Path

from flask import Blueprint, current_app, jsonify, render_template, request

from app import backlog
from app.config import backlog_db_path, current_css, print_ready_dir, uploads_dir
from app.dither import pack_to_zpl, to_1bit
from app.image_pipeline import normalize_uploaded_image
from app.printer import print_zpl, printer_available
from app.record import save_print_job, slugify
from app.render import render_post_png
from app.scheduler import print_item

bp = Blueprint("compose", __name__)
REQUIRED_FIELDS = ["name", "title", "avatar_initials", "body"]
INT_FIELDS = ["reactions", "comments", "reposts"]


def _parse_fields(form) -> dict:
    fields = {key: form.get(key, "").strip() for key in REQUIRED_FIELDS}
    for key in INT_FIELDS:
        raw = form.get(key, "0").strip()
        fields[key] = max(0, int(raw)) if raw.isdigit() else 0
    return fields


def _png_bytes(image) -> bytes:
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    return buffer.getvalue()


def _notify_scheduler():
    scheduler = current_app.extensions.get("backlog_scheduler")
    if scheduler:
        scheduler.notify()


def _scheduler_payload():
    items = backlog.list_items(backlog_db_path())
    config = backlog.settings(backlog_db_path())
    config["enabled"] = bool(config["enabled"])
    config["items"] = len(items)
    config["unprinted"] = sum(item["print_count"] == 0 for item in items)
    config["printer_online"] = printer_available()
    return config


@bp.get("/")
def compose_form():
    return render_template("compose.html")


@bp.post("/print")
def do_print():
    """Compatibility endpoint for immediate structured-post printing."""
    fields = _parse_fields(request.form)
    missing = [key for key in REQUIRED_FIELDS if not fields[key]]
    if missing:
        return jsonify(success=False, message=f"missing required field(s): {', '.join(missing)}", record=None), 400
    try:
        image = render_post_png(fields, current_css())
        job = pack_to_zpl(to_1bit(image))
    except Exception as exc:
        return jsonify(success=False, message=f"render failed: {exc}", record=None), 502
    result = print_zpl(job.text)
    record = None
    if result.success:
        record = save_print_job(slugify(fields["name"]), _png_bytes(image), job.text.encode("ascii"), print_ready_dir())
    return jsonify(success=result.success, message=result.message, record=record), 200 if result.success else 502


@bp.post("/api/submit")
def submit_item():
    kind = request.form.get("kind", "linkedin")
    action = request.form.get("action", "enqueue")
    if kind not in {"linkedin", "image"} or action not in {"enqueue", "print"}:
        return jsonify(success=False, message="invalid submission"), 400

    image_path = None
    if kind == "linkedin":
        payload = _parse_fields(request.form)
        missing = [key for key in REQUIRED_FIELDS if not payload[key]]
        if missing:
            return jsonify(success=False, message=f"Complete: {', '.join(missing)}"), 400
        label = payload["name"]
    else:
        upload = request.files.get("image")
        data = upload.read() if upload else b""
        try:
            normalized = normalize_uploaded_image(data)
        except ValueError as exc:
            return jsonify(success=False, message=str(exc)), 400
        payload = {"original_name": Path(upload.filename or "artwork.png").name}
        label = request.form.get("image_label", "").strip() or Path(payload["original_name"]).stem or "Artwork"
        directory = uploads_dir()
        directory.mkdir(parents=True, exist_ok=True)
        image_path = directory / f"{uuid.uuid4().hex}.png"
        normalized.save(image_path, format="PNG")

    if action == "enqueue":
        item = backlog.add_item(backlog_db_path(), kind, label, payload, str(image_path) if image_path else None)
        _notify_scheduler()
        return jsonify(success=True, message="Added to backlog", item=item)

    transient = {"id": 0, "kind": kind, "label": label, "payload": payload, "image_path": str(image_path) if image_path else None}
    result, record = print_item(transient)
    if image_path:
        image_path.unlink(missing_ok=True)
    return jsonify(success=result.success, message=result.message, record=record), 200 if result.success else 502


@bp.get("/api/backlog")
def api_backlog():
    return jsonify(items=backlog.list_items(backlog_db_path()), scheduler=_scheduler_payload())


@bp.delete("/api/backlog/<int:item_id>")
def api_delete(item_id):
    item = backlog.delete_item(backlog_db_path(), item_id)
    if not item:
        return jsonify(success=False, message="item not found"), 404
    if item["image_path"]:
        try:
            Path(item["image_path"]).unlink(missing_ok=True)
        except OSError:
            pass
    return jsonify(success=True)


@bp.post("/api/backlog/<int:item_id>/move")
def api_move(item_id):
    direction = (request.get_json(silent=True) or {}).get("direction")
    if direction not in {"up", "down"}:
        return jsonify(success=False, message="direction must be up or down"), 400
    return jsonify(success=backlog.move_item(backlog_db_path(), item_id, direction))


@bp.post("/api/backlog/<int:item_id>/print")
def api_print_item(item_id):
    item = backlog.get_item(backlog_db_path(), item_id)
    if not item:
        return jsonify(success=False, message="item not found"), 404
    result, record = print_item(item)
    if result.success:
        backlog.mark_printed(backlog_db_path(), item_id, time.time())
    return jsonify(success=result.success, message=result.message, record=record), 200 if result.success else 502


@bp.post("/api/scheduler")
def api_scheduler():
    data = request.get_json(silent=True) or {}
    try:
        minimum = float(data.get("min_minutes", 15))
        maximum = float(data.get("max_minutes", 20))
    except (TypeError, ValueError):
        return jsonify(success=False, message="Delay must be a number"), 400
    ordering = data.get("ordering", "sequential")
    run_mode = data.get("run_mode", "once")
    enabled = bool(data.get("enabled"))
    if minimum <= 0 or maximum < minimum or maximum > 10080:
        return jsonify(success=False, message="Use a valid delay range (max 7 days)"), 400
    if ordering not in {"sequential", "random"} or run_mode not in {"once", "loop"}:
        return jsonify(success=False, message="Invalid scheduler mode"), 400
    next_at = time.time() + random.uniform(minimum, maximum) * 60 if enabled else None
    config = backlog.update_settings(
        backlog_db_path(), enabled=int(enabled), min_minutes=minimum, max_minutes=maximum,
        ordering=ordering, run_mode=run_mode, next_print_at=next_at, last_error=None,
    )
    _notify_scheduler()
    return jsonify(success=True, scheduler=config)
