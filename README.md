# Infinite Scroll

Technical implementation for Half Cat Half Pizza's *Infinite Scroll*
installation — a physical ribbon of fictional/satirical "professional
network" posts, printed on thermal paper. Full design context lives on the
Ops-in-Net Knowledge page: https://ops.in.net/account/knowledge/infinite-scroll

Installation hardware: Raspberry Pi 4 (`infinite-scroll.local`) wired over
USB to an Arkscan 2054A direct-thermal printer. Validated print contract:
650-dot canvas width, Floyd-Steinberg 1-bit dithering, raw ZPL `^GFA`
bitmap, written directly to `/dev/usb/lp0` (no CUPS, no vendor driver).

## Layout

- `docs/superpowers/specs/` — design specs (brainstormed + approved before
  implementation). Start here for anything not yet built.
- `docs/raspberry-pi-bringup.md` — SD imaging, cloud-init customization,
  first-boot checks, EEPROM/OS upgrades, and recovery access.
- `docs/validated-printing.md` — the physically validated 650-dot raster,
  ZPL encoding, raw-USB transport, and single/repeated/batch procedures.
- `tools/generate_post.py` — reference/prototype generator: renders a
  fictional post (name/title/body/counts) to PNG, dithers it, and packs it
  into a self-contained ZPL job, per the documented conversion contract.
  Superseded by the post-composer web app once built (see the spec in
  `docs/`), but kept as the reference implementation of the pipeline.
  Requires Pillow — run via `nix-shell tools/shell.nix --run "python3 tools/generate_post.py tools/specs.json <outdir>"`.
- `tools/specs.json` — six fictional parody-post fixtures, including the
  physically validated Dino Y. Saur post and the five-post 2026-09-13 batch.
  None reference real people, accounts, or platforms.
- `tools/manifest.json` — sizes/dimensions/sha256 of the five-post batch's
  PNG/1-bit-PNG/ZPL artifacts (historical record only; the artifacts themselves
  live on the Pi at `/var/lib/infinite-scroll/print-ready/`, not duplicated
  here).
- `scripts/print_job.sh` — the validated single-job print procedure
  (preflight checks + bounded write timeout), run on the Pi against
  `/dev/usb/lp0`. Hardened after a 2026-09-13 incident where an
  offline/stalled printer caused an unbounded write to hang the SSH session.
- `scripts/print_batch.sh` — loops `print_job.sh` over several distinct
  jobs, stopping on the first failure instead of printing blind.
- `scripts/print_copies.sh` — safely submits one job N times; the accepted
  30-copy run uses its default 0.5-second spacing.

## Status

- Manual pipeline (generate → dither → ZPL → raw USB print): validated and
  in use.
- Post-composer web app (form + print button + CSS design editor, per
  `docs/superpowers/specs/2026-09-13-post-composer-webapp-design.md`):
  implemented and deployed on the Pi as `infinite-scroll-webapp.service` at
  `http://infinite-scroll.local:8080` (LAN only).
- Automatic/dynamic printing service: not yet started.

## Web studio

The LAN-only control surface supports two source types: structured fictional
professional-network posts rendered through the editable thermal stylesheet,
and uploaded raster artwork normalized to the same 650-dot print width. Either
can be printed immediately or added to a persistent SQLite backlog.

The backlog scheduler releases one item at a time after a random delay inside
a configurable minute range. It can follow queue order or choose randomly,
and can stop after every item has printed once or loop continuously. Queue
contents, print counts, scheduler settings, and the next deadline survive
service restarts. Saved state lives under `/var/lib/infinite-scroll/`; it is
not part of application deployments.

## Quick start

Generate all fictional fixtures:

```sh
nix-shell tools/shell.nix --run \
  "python3 tools/generate_post.py tools/specs.json outputs"
```

On the Pi, print one approved job or repeat it 30 times:

```sh
scripts/print_job.sh /var/lib/infinite-scroll/print-ready/dino-y-saur.zpl
scripts/print_copies.sh /var/lib/infinite-scroll/print-ready/dino-y-saur.zpl 30
```
