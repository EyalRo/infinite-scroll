# Infinite Scroll

Technical implementation for Half Cat Half Pizza's *Infinite Scroll*
installation — a physical ribbon of uploaded artwork, printed on thermal
paper. Full design context lives on the Ops-in-Net Knowledge page:
https://ops.in.net/account/knowledge/infinite-scroll

Installation hardware: Raspberry Pi 4 (`infinite-scroll.local`) wired over
USB to an Arkscan 2054A direct-thermal printer. Validated print contract:
650-dot canvas width, Floyd-Steinberg 1-bit dithering, raw ZPL `^GFA`
bitmap, written directly to `/dev/usb/lp0` (no CUPS, no vendor driver).

## Architecture

Three independent Rust services plus a static frontend, each with a single
responsibility:

- **`uploader`** (`crates/uploader`) — the only public write path. Accepts
  PNG/JPG uploads over HTTP (no base64), sniffs the format, and stages them
  through `partial/` → `ready/` via an atomic rename. No other
  responsibilities.
- **`watcher`** (`crates/watcher`) — has no HTTP API at all. Polls `ready/`,
  converts each new image to the validated print contract (Floyd-Steinberg
  dither, ZPL `^GFA` packing), and moves the result into `complete/` — the
  print library. Runs autonomously; nothing calls it.
- **`printer`** (`crates/printer`) — owns the library (list/remove) and the
  autoprint scheduler, and does the actual physical print. Small footprint
  and resilient by design, since after initial setup the installation goes
  permanently offline. Also serves the static frontend directly (embedded
  at compile time), so no separate webserver is needed on the Pi.
- **Static frontend** (`web/library/`) — mobile-first library management UI
  (upload, remove, autoprint settings) plus a non-physical preview that
  simulates the scheduler's next picks. Calls `uploader` and `printer`
  directly from the browser; no dedicated backend of its own.

All three services are reachable on the public internet, gated by Cloudflare
Access (human session + service token), matching the pattern already used
for MediaWatch:

| Hostname | Service | Port |
|---|---|---|
| `upload.infinite-scroll.art.virtualdino.com` | uploader | 8081 |
| `printer.infinite-scroll.art.virtualdino.com` | printer (API) | 8082 |
| `library.infinite-scroll.art.virtualdino.com` | printer (static frontend) | 8082 |

Shared conversion/dithering/ZPL/auth/HTTP-helper code lives in
`crates/common`, used by all three binaries.

## Layout

- `crates/common/`, `crates/uploader/`, `crates/watcher/`, `crates/printer/`
  — the four workspace crates described above.
- `web/library/` — the static frontend (`index.html`, `style.css`,
  `app.js`), also embedded directly into the `printer` binary at compile
  time.
- `deploy/` — systemd units for all three services and `deploy.sh`, which
  cross-compiles to `aarch64-unknown-linux-gnu` and deploys to
  `infinite-scroll.local` over SSH.
- `docs/cloudflare-tunnel-access-setup.md` — the manual Cloudflare Tunnel
  ingress, DNS, and Access Application steps needed to expose the three
  hostnames above (not automatable from a dev sandbox; requires access to
  the Tunnel connector host and full Cloudflare API scope).
- `docs/raspberry-pi-bringup.md` — SD imaging, cloud-init customization,
  first-boot checks, EEPROM/OS upgrades, and recovery access. Hardware
  bring-up only; unrelated to which application code runs on the Pi.
- `docs/validated-printing.md` — the physically validated 650-dot raster,
  ZPL encoding, and raw-USB transport contract that `crates/common`
  implements, plus standalone manual print scripts for hardware debugging.
- `scripts/print_job.sh`, `scripts/print_copies.sh`, `scripts/print_batch.sh`
  — standalone manual fallback scripts that write a ZPL job straight to
  `/dev/usb/lp0`, bypassing the Rust services entirely. Useful for hardware
  troubleshooting independent of whether the services are healthy.
- `docs/superpowers/plans/2026-09-17-rust-print-services.md` — the
  implementation plan this codebase was built from.

## Status

- All three services (`uploader`, `watcher`, `printer`) are implemented,
  tested, and deployed to `infinite-scroll.local` via systemd
  (`deploy/*.service`, installed by `deploy/deploy.sh`).
- Cloudflare Tunnel ingress, DNS, and Access Applications for the three
  public hostnames: **not yet done** — see
  `docs/cloudflare-tunnel-access-setup.md` for the required manual steps.
  Until this is complete, the services are reachable only on the Pi's LAN
  address, not the public hostnames above.
- The `ops.in.net` MCP layer (`plugins/virtualdino`, `mcp`) still targets
  the old Flask app's API contract and has not yet been updated to call
  these services — blocked on the Cloudflare setup above, since it needs
  the real hostnames and service-token credentials first.
- Autoprint defaults to disabled (`printer/state.json`'s `enabled: false`)
  until the installation is ready to go live.

## Deploying

```sh
./deploy/deploy.sh
```

Cross-compiles the workspace to `aarch64-unknown-linux-gnu`, patches the
resulting binaries' ELF interpreter to match the Pi's actual glibc (nixpkgs'
cross-compiled glibc bakes in a Nix store path that doesn't exist on the
Pi's Debian filesystem), copies them and the systemd units to
`infinite-scroll.local`, restarts all three services, and runs health
checks. Requires SSH access to `infinite-scroll.local` and pre-existing
`/etc/infinite-scroll/{uploader,printer}.env` files on the Pi (see Step 5 of
Task 10 in the implementation plan for how those were generated).

## Manual print fallback

For hardware troubleshooting independent of the Rust services, on the Pi:

```sh
scripts/print_job.sh /var/lib/infinite-scroll/complete/<id>.zpl
scripts/print_copies.sh /var/lib/infinite-scroll/complete/<id>.zpl 30
```

See `docs/validated-printing.md` for the full raster/ZPL contract these
scripts and `crates/common` both implement.
