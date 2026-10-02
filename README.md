# Infinite Scroll

*Infinite Scroll* is a physical art installation by Half Cat Half Pizza: a
ribbon of social-media-style posts printed on thermal paper. The ribbon keeps
growing while the work is exhibited — every so often the installation prints
another image from its library, lengthening the paper that hangs in the
space.

This repository is the software that runs it: a small, self-contained print
system for a Raspberry Pi connected by USB to a thermal label printer, plus a
web interface and an Android app for managing it.

The system's job is deliberately narrow: **pre-rendered images in, thermal
paper out.** It does not generate or interpret posts. The artist supplies
finished PNG or JPEG images; the software stores them, converts them for the
printer, and prints them — on a random timer, on demand, or as a full batch.

## What it does

- **Keeps a library of artwork.** Upload PNG or JPEG images (up to 20 MB);
  each is converted once into a print-ready job and stored. Remove items at
  any time.
- **Prints on a timer.** An optional autoprint mode prints one image every
  *N* to *M* minutes (configurable, up to 7 days). It can walk the library in
  order, or shuffle it and print a full pass without repeats before
  reshuffling. A preview shows what will print next.
- **Prints on demand.** Print one item, several copies, or the whole library
  in one go. Requests become durable jobs: they survive restarts and power
  loss, can be cancelled, and keep running with no one connected.
- **Records what happened.** Every print is logged with its result and an
  estimate of the paper used, with lifetime statistics.
- **Works offline.** After setup the installation needs no Wi-Fi or Internet.
  A Bluetooth Low Energy interface and an Android app provide full control
  from a phone standing next to the installation, including uploading new
  artwork and setting the clock.
- **Prints reliably.** Writes to the printer are serialized and time-bounded,
  so a stuck or unplugged printer cannot hang the system, and print jobs
  retry on transient device errors.

## How it works

```text
            +--------------------+        +--------------------+
            | Web UI (browser)   |        | Android app (BLE)  |
            +---------+----------+        +---------+----------+
                      | HTTP                        | Bluetooth LE
                      v                             v
   upload ->  +--------------+  loopback   +----------------+
              |   printer    |<------------+    btcontrol   |
              | library      |             +----------------+
              | scheduler    |
              | job queue    |    +------------+     +-----------+
              | history      |    |  uploader  |---->|  watcher  |
              +------+-------+    +------------+     +-----------+
                     | raw ZPL          partial/ -> ready/ -> complete/
                     v
              thermal printer (/dev/usb/lp0)
```

Three independent Rust services, one Bluetooth service, and a shared library:

| Component | Responsibility |
|---|---|
| `crates/uploader` | The only way new images enter. Accepts PNG/JPEG over HTTP, checks the real file format from its bytes, and stages it (`partial/` then an atomic move to `ready/`). |
| `crates/watcher` | Has no API. Watches `ready/`, converts each image for the printer, and files it into the library (`complete/`), or `failed/` if it can't be converted. |
| `crates/printer` | Owns the library, the autoprint scheduler, the print-job queue, print history, and all printer I/O. Also serves the web UI. |
| `crates/bluetooth` (`btcontrol`) | A Bluetooth Low Energy (GATT) server that offers the same controls to a phone. It holds no state of its own — it forwards requests to `printer` and `uploader`. |
| `crates/common` | Image conversion, dithering, ZPL encoding, and shared HTTP helpers. |
| `web/library` | The static web interface, built into the `printer` binary. |
| `android/` | The Android control app (Kotlin, Jetpack Compose). |

Bluetooth is a second control surface, not a second application: there is one
library, one scheduler, one queue, and one place that talks to the printer.

### From image to paper

Images are converted into the format the printer needs:

1. Decode the PNG/JPEG, apply JPEG orientation, and flatten transparency onto
   white.
2. Convert to grayscale and scale to **650 dots** wide (about 3.2 inches at
   203 DPI), preserving aspect ratio. Height follows the image.
3. Reduce to 1 bit with **Floyd–Steinberg dithering**.
4. Pack into a ZPL `^GFA` bitmap and write it straight to the printer's USB
   device (`/dev/usb/lp0`).

There is no CUPS, driver, or desktop software in the loop. The details of the
validated method are in [`docs/validated-printing.md`](docs/validated-printing.md).

## Hardware

- A **Raspberry Pi 4** running Raspberry Pi OS (64-bit). Its built-in
  Bluetooth is used for the phone interface.
- An **Arkscan 2054A** direct-thermal printer (203 DPI) connected over USB,
  appearing as `/dev/usb/lp0` through the Linux `usblp` driver. Other
  ZPL-capable printers with a raw USB device may work but are untested.
- For the control app: an Android phone with Bluetooth LE (Android 13 or
  later). It is developed against a recent Pixel.

## Controlling it

### Web interface

The `printer` service serves a mobile-friendly page at its root (port 8082 by
default) for browsing the library, uploading and removing artwork, printing an
item immediately, configuring the print timer, and previewing upcoming prints.

### Bluetooth and the Android app

The Android app connects to the installation over Bluetooth LE and offers
everything the web interface does, plus clock synchronisation (a Raspberry Pi
has no battery-backed clock, so it needs the time after a power loss when
offline). The app clearly distinguishes a command the installation has
*accepted* — which it will carry out on its own, even if the phone
disconnects — from one that has *completed*.

The Bluetooth interface exposes only the specific operations the project
defines. It has no shell, file, or raw printer access, and no pairing or
login: physical proximity is the access boundary.

The protocol is documented in [`docs/ble-protocol.md`](docs/ble-protocol.md);
the app is described in [`android/README.md`](android/README.md).

### Printer settings

Settings such as print darkness are exposed only once they have been confirmed
to work on the physical printer over this same raw-USB path. None are
confirmed yet; see [`docs/printer-settings.md`](docs/printer-settings.md) for
the procedure and status.

## Building and testing

You need a recent stable Rust toolchain.

```sh
cargo build --release     # all services
cargo test --workspace    # unit tests
```

`btcontrol` links against D-Bus (it talks to BlueZ); the build vendors
libdbus, so a C compiler is the only extra requirement. The Android protocol
library is plain Kotlin and tests anywhere:

```sh
cd android && ./gradlew :protocol:test
```

Building the Android app itself needs the Android SDK; see
[`android/README.md`](android/README.md).

## Running

Each service is configured through environment variables and is meant to run
under a process supervisor such as systemd. Example unit files are in
[`deploy/`](deploy/).

| Service | Required | Optional (default) |
|---|---|---|
| `uploader` | `UPLOADER_TOKEN`, `UPLOADER_PARTIAL_DIR`, `UPLOADER_READY_DIR` | `BIND_ADDR` (`127.0.0.1:8081`) |
| `watcher` | `WATCHER_READY_DIR`, `WATCHER_COMPLETE_DIR`, `WATCHER_FAILED_DIR` | `WATCHER_POLL_SECONDS` (`3`) |
| `printer` | `PRINTER_TOKEN`, `UPLOADER_TOKEN`, `PRINTER_STATE_PATH`, `PRINTER_COMPLETE_DIR`, `PRINTER_FAILED_DIR` | `PRINTER_DEVICE_PATH` (`/dev/usb/lp0`), `PRINTER_READY_DIR`, `UPLOADER_INTERNAL_ADDR` (`127.0.0.1:8081`), `BIND_ADDR` (`127.0.0.1:8082`) |
| `btcontrol` | `PRINTER_TOKEN`, `UPLOADER_TOKEN` | `PRINTER_URL`, `UPLOADER_URL`, `BLE_LOCAL_NAME` (`Infinite Scroll`) |

The token values are shared secrets between the services; generate your own
and keep them out of version control. `btcontrol` needs BlueZ running and
membership of the `bluetooth` group; it also uses the `CAP_SYS_TIME`
capability so the phone can set the clock.

**Security note.** The HTTP services are designed to sit on a trusted network
or behind an authenticating reverse proxy, not directly on the open Internet:
besides the bearer token, the `printer` and `uploader` APIs accept any request
that carries a `Cf-Access-Jwt-Assertion` header, on the assumption that a
proxy has already authenticated it. Do not expose their ports directly.

[`deploy/deploy.sh`](deploy/deploy.sh) is the script used for the original
installation: it cross-compiles for the Pi (using Nix) and installs the
services over SSH. Treat it as a reference for your own setup.

## Manual printing

For hardware troubleshooting independent of the services, the scripts in
[`scripts/`](scripts/) write a stored print job straight to the printer
device:

```sh
scripts/print_job.sh    /var/lib/infinite-scroll/complete/<id>.zpl
scripts/print_copies.sh /var/lib/infinite-scroll/complete/<id>.zpl 30
```

## Repository layout

```text
crates/      Rust workspace: common, uploader, watcher, printer, bluetooth
web/library  Static web interface (embedded in the printer binary)
android/     Android control app and its pure-Kotlin protocol library
deploy/      Example systemd units and a deployment script
scripts/     Manual print and printer-probe helpers
docs/        Protocol, validated print method, printer-settings notes
```

## Status

The services are implemented and covered by unit tests, and the print path has
been validated on the physical printer. The Bluetooth service and Android app
are newer: their protocol layers are tested, but they have not yet been
exercised end to end on real hardware, and the Android app has not yet been
built against the Android SDK. Auto-print is off by default until you enable
it.

## License

No license file has been added yet.
