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

## Print one or many

Install the scripts on the Pi, then (using the current library path — see
"Installation paths" below):

```sh
scripts/print_job.sh /var/lib/infinite-scroll/complete/<id>.zpl
scripts/print_copies.sh /var/lib/infinite-scroll/complete/<id>.zpl 30
scripts/print_batch.sh /var/lib/infinite-scroll/complete <id-one> <id-two>
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

## Installation paths (current, Rust services)

```text
/var/lib/infinite-scroll/partial/  in-progress uploads (uploader)
/var/lib/infinite-scroll/ready/    uploads ready for conversion (watcher input)
/var/lib/infinite-scroll/complete/ the print library (watcher output, printer's source of truth)
/var/lib/infinite-scroll/failed/   uploads the watcher could not convert
```

The `printer` app (see `crates/printer/src/printer_device.rs`) already
serializes device writes and fails closed on a missing/busy printer, per
this document's original requirement.

Note: `/var/lib/infinite-scroll/posts/` and `/var/lib/infinite-scroll/print-ready/`
were the old Python app's paths, kept on disk for one deploy cycle as a
rollback reference (see Task 13 of `docs/superpowers/plans/2026-09-17-rust-print-services.md`)
but no longer written to.
