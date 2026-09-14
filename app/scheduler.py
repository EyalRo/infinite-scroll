"""One-at-a-time backlog scheduler using the existing serialized printer."""
import logging
import random
import threading
import time
from pathlib import Path

from app import backlog
from app.config import current_css, print_ready_dir
from app.dither import pack_to_zpl, to_1bit
from app.image_pipeline import normalize_uploaded_image
from app.printer import PrintResult, print_zpl
from app.record import save_print_job, slugify
from app.render import render_post_png

log = logging.getLogger(__name__)


def render_item(item: dict):
    if item["kind"] == "linkedin":
        return render_post_png(item["payload"], current_css())
    return normalize_uploaded_image(Path(item["image_path"]).read_bytes())


def print_item(item: dict) -> tuple[PrintResult, dict | None]:
    try:
        image = render_item(item)
        job = pack_to_zpl(to_1bit(image))
    except Exception as exc:
        return PrintResult(False, f"render failed: {exc}"), None
    result = print_zpl(job.text)
    if not result.success:
        return result, None

    import io
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    record = save_print_job(
        slugify(item["label"]), buffer.getvalue(), job.text.encode("ascii"), print_ready_dir()
    )
    return result, record


def choose_item(items: list[dict], config: dict, rng=random) -> dict | None:
    eligible = items if config["run_mode"] == "loop" else [item for item in items if item["print_count"] == 0]
    if not eligible:
        return None
    if config["ordering"] == "random":
        alternatives = [item for item in eligible if item["id"] != config["last_item_id"]]
        return rng.choice(alternatives or eligible)
    last_id = config["last_item_id"]
    if last_id is not None:
        for item in eligible:
            if item["id"] == last_id:
                index = eligible.index(item)
                return eligible[(index + 1) % len(eligible)]
    return eligible[0]


class BacklogScheduler:
    def __init__(self, db_path: Path, clock=time.time, rng=random):
        self.db_path = Path(db_path)
        self.clock = clock
        self.rng = rng
        self.wake = threading.Event()
        self.stop_event = threading.Event()
        self.thread = None

    def start(self):
        backlog.initialize(self.db_path)
        if self.thread and self.thread.is_alive():
            return
        self.thread = threading.Thread(target=self._run, name="backlog-scheduler", daemon=True)
        self.thread.start()

    def stop(self):
        self.stop_event.set()
        self.wake.set()

    def notify(self):
        self.wake.set()

    def _schedule_next(self, config):
        delay = self.rng.uniform(config["min_minutes"], config["max_minutes"]) * 60
        return backlog.update_settings(self.db_path, next_print_at=self.clock() + delay)

    def tick(self):
        config = backlog.settings(self.db_path)
        if not config["enabled"]:
            return
        if config["next_print_at"] is None:
            self._schedule_next(config)
            return
        if self.clock() < config["next_print_at"]:
            return
        item = choose_item(backlog.list_items(self.db_path), config, self.rng)
        if item is None:
            backlog.update_settings(self.db_path, enabled=0, next_print_at=None, last_error=None)
            return
        result, _record = print_item(item)
        now = self.clock()
        if result.success:
            backlog.mark_printed(self.db_path, item["id"], now)
            config = backlog.update_settings(self.db_path, last_item_id=item["id"], last_error=None)
        else:
            config = backlog.update_settings(self.db_path, last_error=result.message)
            log.error("scheduled print failed for item %s: %s", item["id"], result.message)
        self._schedule_next(config)

    def _run(self):
        while not self.stop_event.is_set():
            try:
                self.tick()
            except Exception:
                log.exception("backlog scheduler tick failed")
            self.wake.wait(1.0)
            self.wake.clear()
