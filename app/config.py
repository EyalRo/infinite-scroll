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
