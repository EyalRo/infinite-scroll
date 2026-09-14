# Validated raw-USB printing

This is the adopted baseline for *Infinite Scroll*. It was physically tested
on 2026-09-13, including a successful 30-copy run.

## Printer path

- Arkscan 2054A, 203 DPI direct thermal
- USB vendor/product ID: `2d84:b468`
- Linux driver: `usblp`
- Raw character device: `/dev/usb/lp0`
- No CUPS or vendor driver

Confirm enumeration before every run:

```sh
lsusb | grep '2d84:b468'
test -c /dev/usb/lp0
```

If either check fails, stop. Never claim a post was printed when the device
was missing or the write failed.

## Adopted raster contract

- canvas width: **650 dots** (about 3.20 inches at 203 DPI)
- tested media width: approximately 3.8 inches
- prototype side margins: 48 dots each
- height: content-driven
- monochrome conversion: Pillow `L -> 1`, Floyd-Steinberg dithering
- ZPL: `^PW650`, content-driven `^LL`, origin `^LH0,0` / `^FO0,0`
- bitmap: `^GFA`, black thermal dots encoded as `1` bits

For width `W` and height `H`:

```text
row_bytes   = ceil(W / 8)
total_bytes = row_bytes * H
^GFA,total_bytes,total_bytes,row_bytes,UPPERCASE_HEX_DATA
```

Widths of 812, 760, 700, and 650 dots were physically tried. The 650-dot
version was approved. Keep width as one explicit configuration value and
change it only when the media, printer, or desired margins change.

## Generate fixtures

From the repository root:

```sh
nix-shell tools/shell.nix --run \
  "python3 tools/generate_post.py tools/specs.json outputs"
```

The generator produces grayscale PNG, 1-bit PNG, ZPL, and a manifest with
dimensions, sizes, and SHA-256 hashes. `tools/specs.json` contains fictional
satire only; it is not scraped social-network data.

## Print one or many

Install the scripts on the Pi, then:

```sh
scripts/print_job.sh /var/lib/infinite-scroll/print-ready/dino-y-saur.zpl
scripts/print_copies.sh /var/lib/infinite-scroll/print-ready/dino-y-saur.zpl 30
scripts/print_batch.sh /var/lib/infinite-scroll/print-ready slug-one slug-two
```

`print_job.sh` verifies the file and device, calculates a source hash, and
bounds the raw write with a timeout. `print_copies.sh` submits one complete
`^XA ... ^XZ` document per copy with the validated 0.5-second spacing.
`print_batch.sh` submits different named jobs. Both batch forms stop on the
first error.

A completed write means the kernel accepted the bytes; it does not prove that
paper physically emerged. Production monitoring should add printer-state,
paper-out, cover-open, disconnect, and physical-completion handling where the
hardware exposes those signals.

## Installation paths

```text
/var/lib/infinite-scroll/posts/       artist-supplied source images
/var/lib/infinite-scroll/print-ready/ generated PNG/ZPL and print records
```

The future service should log the source identity/hash, generated-job hash,
submission timestamp, copy number, and write result. It must fail closed on a
missing printer or write error and must serialize writes to prevent interleaved
ZPL jobs.
