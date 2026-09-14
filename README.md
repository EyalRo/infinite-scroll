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
- `tools/generate_post.py` — reference/prototype generator: renders a
  fictional post (name/title/body/counts) to PNG, dithers it, and packs it
  into a self-contained ZPL job, per the documented conversion contract.
  Superseded by the post-composer web app once built (see the spec in
  `docs/`), but kept as the reference implementation of the pipeline.
  Requires Pillow — run via `nix-shell tools/shell.nix --run "python3 tools/generate_post.py tools/specs.json <outdir>"`.
- `tools/specs.json` — the 5 parody-post definitions used for the
  2026-09-13 batch (Brayden Steelworth, Persimmon Vale, Chad Ironframe,
  Willow Sterling-Cho, Reginald Huxtable-Vance III). All fictional/satirical;
  none reference real people, accounts, or platforms.
- `tools/manifest.json` — sizes/dimensions/sha256 of the PNG/1-bit-PNG/ZPL
  artifacts generated for that batch (record only; the artifacts themselves
  live on the Pi at `/var/lib/infinite-scroll/print-ready/`, not duplicated
  here).
- `scripts/print_job.sh` — the validated single-job print procedure
  (preflight checks + bounded write timeout), run on the Pi against
  `/dev/usb/lp0`. Hardened after a 2026-09-13 incident where an
  offline/stalled printer caused an unbounded write to hang the SSH session.
- `scripts/print_batch.sh` — loops `print_job.sh` over several distinct
  jobs, stopping on the first failure instead of printing blind.

## Status

- Manual pipeline (generate → dither → ZPL → raw USB print): validated and
  in use.
- Post-composer web app (form + print button + CSS design editor, per
  `docs/superpowers/specs/2026-09-13-post-composer-webapp-design.md`):
  designed, not yet implemented.
- Automatic/dynamic printing service: not yet started.
