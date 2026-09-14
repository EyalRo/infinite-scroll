"""1-bit Floyd-Steinberg conversion and ZPL ^GFA packing.

Reuses the exact conversion contract validated and documented on the
Ops-in-Net "Infinite Scroll" Knowledge page: 8 dots per byte, black = 1
bit, uppercase hex, self-contained ^GFA job.
"""
from dataclasses import dataclass

from PIL import Image


def to_1bit(img: Image.Image) -> Image.Image:
    """Convert a grayscale image to 1-bit using Floyd-Steinberg dithering
    (PIL's default when converting L -> 1)."""
    return img.convert("L").convert("1")


@dataclass
class ZplJob:
    text: str
    total_bytes: int
    row_bytes: int
    width: int
    height: int


def pack_to_zpl(img_1bit: Image.Image) -> ZplJob:
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
    zpl_text = (
        "^XA\n"
        f"^PW{w}\n"
        f"^LL{h}\n"
        "^LH0,0\n"
        "^FO0,0\n"
        f"^GFA,{total_bytes},{total_bytes},{row_bytes},{hexdata}\n"
        "^XZ\n"
    )
    return ZplJob(text=zpl_text, total_bytes=total_bytes, row_bytes=row_bytes, width=w, height=h)
