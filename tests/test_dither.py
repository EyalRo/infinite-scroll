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
