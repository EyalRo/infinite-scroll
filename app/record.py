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
