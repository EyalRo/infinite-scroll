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
