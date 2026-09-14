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
EXPECTED_SHA256 = "c463923786d2484f35432224c5fd5e79bcf56d583d8543094ba706127740c3b5"


def test_pipeline_is_stable_for_the_fixed_sample_post():
    css = DEFAULT_CSS_PATH.read_text()
    img = render_post_png(FIXTURE, css)
    bitimg = to_1bit(img)
    job = pack_to_zpl(bitimg)
    actual = hashlib.sha256(job.text.encode("ascii")).hexdigest()

    if EXPECTED_SHA256 == "PENDING":
        raise AssertionError(f"first run — set EXPECTED_SHA256 to: {actual}")
    assert actual == EXPECTED_SHA256
