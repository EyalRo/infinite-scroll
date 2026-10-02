# Testing handoff — Bluetooth control, print jobs, Android app

For an agent (or person) with **SSH access to the Pi** (`infinite-scroll.local`,
user `stags`) and, for Part C, a Pixel phone and a dev machine. Nothing in this
feature has run on real hardware yet. Your job is to find out what is broken,
fix what is clearly in scope, and report the rest. Read this whole document
before touching the Pi.

Related: `docs/ble-protocol.md` (the contract), `docs/printer-settings.md`
(settings verification), `android/README.md`, `deploy/deploy.sh`.

## What was built (and what is unverified)

| Component | State |
|---|---|
| `crates/printer`: persistent job queue (`/print`, `/print-all`, `/jobs`), print history/stats (`/history`, `/stats`), scheduler preview (`/preview`), capability-gated printer settings (`/printer/*`) | Unit-tested; smoke-tested against a fake device (`/dev/null`). **Never run against the real Arkscan.** |
| `crates/bluetooth` → `btcontrol` BLE GATT server | Protocol/handler unit-tested (27 tests). Cross-compiled and deployed to the Pi (2026-10-02): advertises under BlueZ and a laptop can connect with no pairing prompt. **Never tested with the Android app.** |
| `android/protocol` (Kotlin, pure JVM) | 11 unit tests pass. |
| `android/app` (Compose UI + GATT client) | **Never compiled** (written without an Android SDK). Expect a first-build fix-up pass. |
| Printer density/darkness | Verified on hardware and exposed (`darkness`, plus a `print_config` action); see `docs/printer-settings.md`. |

## Ground rules

1. **Paper and ink are real.** Prints consume thermal paper. Use the smallest
   test images (e.g. 650×100 px). Load paper first; keep it short-run.
2. **Do not lose the installation's data.** Before any test that changes state,
   back up (Part A.2). Restore at the end. Use throwaway test images and delete
   only items you created.
3. **Autoprint must be disabled** when you start and must be left as you found
   it. Record its original state.
4. **No secrets in output, logs, commits or this report.** Tokens live in
   `/etc/infinite-scroll/*.env`. Read them into shell variables; never `cat` them
   into your transcript.
5. **Do not add generic command passthrough** (shell, raw printer commands,
   filesystem access) to fix a test. The protocol exposes named operations only.
6. Failure to follow Part B's rule — **only add a printer setting after it is
   verified on the hardware** — is the most likely way to ship a wrong feature.
7. Report honestly: "could not test X because Y" is a valid result; "passed"
   without evidence is not.

## Part A — Deploy and Pi-side tests (SSH)

### A.1 Preconditions

On the Pi:

```sh
ssh stags@infinite-scroll.local
systemctl is-active uploader watcher printer          # note current state
lsusb | grep 2d84:b468                                 # printer present?
test -c /dev/usb/lp0 && echo "device ok"
bluetoothctl --version; systemctl is-active bluetooth  # BlueZ present and running
rfkill list                                            # Bluetooth must not be blocked
ls /etc/infinite-scroll/                               # printer.env uploader.env (+ bluetooth.env to create)
```

Check that `/etc/infinite-scroll/printer.env` defines **both** `PRINTER_TOKEN`
and `UPLOADER_TOKEN` (the printer now proxies uploads and fails to start
without `UPLOADER_TOKEN`) — check by variable name only:
`sudo grep -c '^UPLOADER_TOKEN=' /etc/infinite-scroll/printer.env`.

Create `/etc/infinite-scroll/bluetooth.env` (mode 600, root-owned) containing
`PRINTER_TOKEN=` and `UPLOADER_TOKEN=` with the **same values** as the existing
env files:

```sh
sudo sh -c 'umask 077; { grep "^PRINTER_TOKEN=" /etc/infinite-scroll/printer.env;
  grep "^UPLOADER_TOKEN=" /etc/infinite-scroll/uploader.env; } > /etc/infinite-scroll/bluetooth.env'
sudo wc -l /etc/infinite-scroll/bluetooth.env   # expect 2
```

Add user `stags` to group `bluetooth` if `id stags` lacks it
(`sudo usermod -aG bluetooth stags`).

### A.2 Back up state

```sh
sudo tar czf ~/is-backup-$(date +%F-%H%M).tgz /var/lib/infinite-scroll
cat /var/lib/infinite-scroll/printer/state.json   # record enabled/min/max/ordering
```

### A.3 Build and deploy

From a dev machine with the repo and Nix (see README "Deploying"):

```sh
./deploy/deploy.sh
```

This cross-compiles for `aarch64-unknown-linux-gnu`. `btcontrol` is built
separately: if it fails, the other services still deploy and the script exits
non-zero (the web-facing services are never blocked by it). **Likely failure point:** `btcontrol` depends on `bluer`, which
needs libdbus; `libdbus-sys` is configured with the `vendored` feature so it
builds from source with the cross C compiler. If this fails (missing `CC`,
autotools, link errors), either fix the build or build natively on the Pi
(`cargo build --release -p bluetooth` after installing `build-essential`) and
copy the binary to `/home/stags/infinite-scroll/bin/btcontrol`. Record which.

Verify afterwards:

```sh
systemctl is-active uploader watcher printer btcontrol
journalctl -u btcontrol -n 50 --no-pager     # expect "GATT service ... advertising"
```

Expected `btcontrol` log lines: `using adapter hci0 (...)`, `GATT service
a4f60001-... advertising`. Common failures: D-Bus permission denied (group
`bluetooth`), "adapter not found"/powered off (rfkill, `bluetoothctl power on`),
`CAP_SYS_TIME` / `ProtectSystem` unit issues.

### A.4 Printer API tests (loopback, real device)

```sh
PT=$(sudo sed -n 's/^PRINTER_TOKEN=//p' /etc/infinite-scroll/printer.env)
H="Authorization: Bearer $PT"; P=http://127.0.0.1:8082
curl -s -H "$H" $P/status | python3 -m json.tool
```

Create a tiny test image locally (PNG, 650×100), upload via the **uploader**
(`UPLOADER_TOKEN`, `POST http://127.0.0.1:8081/uploads`, body = file bytes),
wait ~5 s, and confirm it appears in `GET /catalog` and
`/var/lib/infinite-scroll/complete/`. Then:

| # | Test | Expected |
|---|---|---|
| A4-1 | `GET /status` | `printer.connected` true, `busy` false, `error` null, `catalog_count` correct |
| A4-2 | `POST /print {"id":ID,"copies":2}` | 202 `accepted`; **2 physical prints** emerge; job ends `done` 2/2; item `print_count` +2 |
| A4-3 | `POST /print-all {}` with 2–3 items | 202; every item prints once, in catalog order; second `POST /print-all` while active → **409** with the existing job |
| A4-4 | `GET /history?limit=5`, `GET /stats` | one record per physical print, `origin` `job`; `paper_mm` > 0; `prints_ok` matches paper that actually came out |
| A4-5 | `POST /catalog/ID/print` (web "print now") | prints; appears in history as `manual`; counted in `manual_ok` |
| A4-6 | `GET /preview?count=3` | sequential: `exact` true. Random: picks match the order the scheduler later prints when autoprint runs |
| A4-7 | `DELETE /jobs/JOBID` during a multi-copy job | job `cancelled`; prints stop within one unit |
| A4-8 | Restart persistence: queue 10 copies, `sudo systemctl restart printer` after ~3 prints | job resumes and finishes; total physical prints = 10 (**note any duplicate or missing print** — one unit in flight at restart may repeat; record what happens) |
| A4-9 | Reboot persistence: queue 10 copies, `sudo reboot` mid-job | after boot the job continues without any client |
| A4-10 | Failure retry: open the printer cover/remove paper mid-job | job stays `running` with `error` set, retries every ~15 s; restoring paper resumes; after 5 consecutive failures state becomes `failed` (let it happen once, then re-queue) |
| A4-11 | Printer unplugged | `/status` `printer.connected` false; `POST /print` accepted (queued) but fails/ retries visibly; nothing hangs |
| A4-12 | Autoprint + job collision: enable autoprint with 1-minute interval, queue a long job | prints interleave without overlapping garbage on paper; no "printer busy" errors in history |
| A4-13 | Delete an item that is part of an active print-all | job completes; that unit counted in `skipped` |
| A4-14 | Invalid input: copies 0, 101, missing id, id `../x`, unknown id | 400/404, never 500 |
| A4-15 | Existing web UI (`http://infinite-scroll.local:8082/`) still works: list, upload, remove, print now, timer, preview | unchanged behaviour |

Evidence to capture: command, output, and a photo/description of the paper for
every printing test.

## Part B — Printer settings investigation (density/darkness)

Goal: find out what the physical Arkscan 2054A answers/accepts over the
**deployed path** (`/dev/usb/lp0`, ZPL). No CUPS, no vendor software.

1. Stop the services that write to the device so nothing interleaves:
   `sudo systemctl stop printer btcontrol` (restart them at the end).
2. Run `scripts/probe_printer_settings.sh` (read-only queries `~HS`, `~HQES`,
   `^HH`). Save raw output. **If nothing comes back, `usblp` may not give a
   readable return channel** — record that; it decides whether settings can be
   *read* at all.
3. Density: ZPL `~SD<0–30>` sets darkness, `^MD<-30…30>` adjusts relative to
   the stored value. Whether this firmware honours them is **unknown**. Test
   with one small, identical job printed at, e.g., `~SD05`, `~SD15`, `~SD25`
   (send `printf '~SD15' > /dev/usb/lp0` and then the job). Compare the
   printouts side by side. Record: does darkness visibly change? Does it
   persist after power-cycling the printer (print again without `~SD`)? Is
   there any read-back (`^HH` config dump showing darkness)?
4. Write findings into `docs/printer-settings.md` ("Findings") with exact
   commands, replies (hex via `od -c`), and observed behaviour.
5. **Only if** a setting is confirmed, implement it as a typed capability in
   `crates/printer/src/printer_settings.rs` (key, label, min/max/step,
   readable/writable, a dedicated write — and a read if one exists), with unit
   tests, then deploy and re-run Part C's Printer-tab test. Do not add a generic
   command endpoint. If nothing is confirmed, leave the capability list empty
   and say so.
6. Also record: printer firmware/model reported by `~HS`/`^HH`, media settings,
   anything that makes prints differ after a power cycle.

## Part C — Bluetooth and Android

### C.1 Pi-side BLE sanity (no phone yet)

```sh
sudo systemctl status btcontrol --no-pager
bluetoothctl show | grep -E 'Powered|Discoverable|UUID'
sudo btmgmt info | head -20          # adapter supports LE / advertising
```

From any **other** BLE central (a laptop with `bluetoothctl scan on`, or the
phone's nRF Connect app), confirm a device named **Infinite Scroll** advertises
service `a4f60001-6b8e-4c1f-9d0a-5e1f0c1a1001` and connects **without a pairing
prompt**.

### C.2 Protocol check with a generic BLE tool (nRF Connect, or similar)

Independent of our app. Connect, **request MTU 247 or higher**, then:

1. Read `a4f60002-…` (Info): expect JSON with `"proto":1`, `max_upload_bytes`.
2. Enable notifications on `a4f60004-…` (Response) and `a4f60005-…` (Changed).
3. Write (hex) to `a4f60003-…` (Request) — frame `03 | id(2, LE) | JSON`:
   - `sys.status`: `0301007b2276223a312c226964223a312c226f70223a227379732e737461747573222c2261726773223a7b7d7d`
   - `library.list` (limit 5): `0302007b2276223a312c226964223a322c226f70223a226c6962726172792e6c697374222c2261726773223a7b226f6666736574223a302c226c696d6974223a357d7d`
4. Expect Response notifications whose first byte is `03` (single frame) or
   `01 … 00/02` (multi-frame), id `01 00` / `02 00`, followed by JSON
   `{"v":1,"id":1,"ok":true,"disposition":"completed","result":{…}}`.
   Multi-frame responses: flags `01` = first, `02` = last, `03` = both.

Record whether notifications of full response size arrive (they depend on the
negotiated MTU) and whether the unknown-op error (`{"op":"shell.exec"}`) is
returned as `unknown_op`.

### C.3 Build and install the Android app

Requirements (on a dev machine): JDK 17, Android SDK with **platform 36** and
matching build-tools (the Pixel runs Android 17 / API 37; raise
`compileSdk`/`targetSdk` in `android/app/build.gradle.kts` if the 37 platform is
installed), `adb`.

```sh
cd android
export ANDROID_HOME=$HOME/Android/Sdk     # or write sdk.dir=... into local.properties
./gradlew :protocol:test                   # must pass (11 tests) — sanity first
./gradlew :app:assembleDebug
adb devices                                # phone listed, authorised
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Phone setup: Settings → About → tap *Build number* 7× → Developer options →
USB debugging (or Wireless debugging → pair). Bluetooth on. On first launch
accept the **Nearby devices** permission.

**Expect the first compile to need fixes** — the module was written blind. Fix
only what the compiler/linter requires (API changes, missing imports, theme/
resource problems, deprecated `kotlinOptions`), keep changes minimal, and list
every fix in your report. Do not redesign.

### C.4 App acceptance tests

Use a throwaway test image and keep the printer loaded. "Pi evidence" means
confirm via SSH (`journalctl -u btcontrol -f`, `curl` to the printer API) in
addition to what the UI shows.

| # | Test | Expected |
|---|---|---|
| C-1 | Scan → Connect | Pi appears; connects with no pairing prompt; Status tab populates; `btcontrol` logs the subscription |
| C-2 | Status tab | Printer/uploader "running"; library count right; stats match `/stats` |
| C-3 | Clock sync. First on the Pi: `sudo timedatectl set-ntp false; sudo date -s '2026-01-02 03:04:05'`. Tap **Set Pi clock from phone** | Pi time ≈ phone time (check `date`); toast says "Done"; **restore afterwards:** `sudo timedatectl set-ntp true` |
| C-3b | Clock sync with autoprint enabled and a >1 min correction | schedule re-armed (`next_print_at` recomputed), no burst of prints |
| C-4 | Library tab lists items with print counts | matches `GET /catalog` |
| C-5 | Upload a PNG (<1 MB) | progress bar; message says **Accepted** (not "done"); item appears within seconds; `ready/` → `complete/` path followed (watch `ls`) |
| C-6 | Upload a JPEG phone photo (3–8 MB) | accepted; appears; prints upright (EXIF) |
| C-7 | Upload a non-image / >20 MB file | clear error; nothing staged |
| C-8 | Upload then walk out of range/disable BT mid-transfer; reconnect and retry | no partial item in the library; retry succeeds |
| C-9 | Delete an item | confirmation dialog; gone from library and `complete/` |
| C-10 | Print item ×3 | message says **Accepted … queued**; 3 prints emerge; Printing tab shows progress; history shows 3 |
| C-11 | **Print all, then disconnect immediately** (tap Disconnect, close the app, turn phone BT off) | Pi keeps printing the whole library (verify by `curl /jobs` and on paper). Reconnect: job shown `done` |
| C-12 | Cancel an active job from the Printing tab | stops promptly |
| C-13 | Schedule tab: enable autoprint, change min/max (e.g. 1–2 min), ordering | persists on the Pi (`state.json`), `next_print_at` shown, prints actually fire at the configured interval; set back to the original state afterwards |
| C-14 | Preview matches the next real prints | for sequential: exact; for random: matches while the shuffle queue holds |
| C-15 | Printer tab | connection/activity correct; with no verified settings shows "No adjustable printer settings have been verified…" (or the verified controls if Part B added any, and changing them changes the printout) |
| C-16 | Kill/restart `btcontrol` while connected | app returns to the connect screen / shows an error; reconnect works; running print jobs unaffected |
| C-17 | Stop `printer` service (`sudo systemctl stop printer`) while connected | app shows "A service on the Pi is not running"; Status shows printer NOT RUNNING; restart recovers without reconnecting the phone |
| C-18 | Change notifications: while the app is open, run `curl -X POST …/print` over SSH | the Printing/Status tabs update on their own within ~3 s |
| C-19 | Phone permission denied / Bluetooth off | sensible message, no crash |
| C-20 | Rotate screen, background the app, return | state intact or cleanly refreshed |

### C.5 Offline test (the point of the feature)

Prove the installation works with **no Wi-Fi and no Internet**:

- Do **not** drop Wi-Fi while you depend on it for SSH. Either work from a
  local console/serial, or schedule an automatic restore first:
  `sudo systemd-run --on-active=20m nmcli radio wifi on`, then
  `sudo nmcli radio wifi off` (use `rfkill block wifi` if NetworkManager is not
  used). Confirm no route: `ping -c1 1.1.1.1` fails.
- With Wi-Fi off, repeat C-1, C-3, C-5, C-10, C-11, C-13. Everything must work
  over Bluetooth alone, and a power cycle (`sudo reboot` via a local console, or
  after the auto-restore) must bring `btcontrol`, `printer`, queued jobs and the
  library back with no network.

## Part D — Things to examine while you are in there

- `journalctl -u btcontrol` — look for warnings: dropped responses, bad frames,
  upload gaps. Note the effective MTU the phone negotiates (Pixel should reach
  ~517) and whether notification chunking behaves at the 23-byte default.
- Throughput: time a 1 MB upload over BLE. Report seconds. If it is
  unacceptably slow (>2 min), note the limiting step (write-without-response
  pacing in `GattTransport.write`, connection interval, MTU).
- Memory/CPU of `btcontrol` while idle and during upload (`systemctl status`,
  `top`). Idle poll is every 2 s only while a client is subscribed.
- Security surface: confirm only the five characteristics exist
  (`bluetoothctl` → `menu gatt` / nRF Connect), no pairing is required, and
  there is no way to reach a shell or arbitrary printer command (try unknown
  ops, oversized frames, a request id/op/args of the wrong types, ids like
  `../etc/passwd`). Expect structured errors, never a crash.
- Concurrency: two phones connected at once (supported behaviour is one;
  record what happens).

## Reporting

Deliver a report (markdown, in the repo as `docs/test-report-YYYY-MM-DD.md` or
as the agent's final message) with:

1. **Environment**: Pi OS/kernel, BlueZ version, printer firmware if known,
   phone model/Android build, commit hash tested.
2. **Results table**: every test ID → pass / fail / blocked / not run, with
   evidence (command + output excerpt, or what was observed on paper).
3. **Defects**: for each — steps to reproduce, expected vs actual, logs,
   suspected cause, severity (blocks deployment / degrades / cosmetic), and
   whether you fixed it (commit hash) or left it.
4. **Printer settings findings** (Part B) — verified capabilities only.
5. **Changes you made** to code, units, or Pi config, so they can be reviewed
   and reverted.
6. **Restoration**: confirm autoprint, NTP, Wi-Fi, services, and `state.json`
   are back to how you found them, and that your backup is intact.

## Definition of done for the feature

- A.4 tests all pass on the real printer, including restart/reboot persistence.
- C-1, C-5, C-10, C-11, C-13, and the offline run all pass on a Pixel.
- `PRINT_ALL` provably continues after the phone disconnects.
- Clock sync works with NTP disabled and survives a reboot only as far as the
  Pi's hardware allows — **note:** a Raspberry Pi 4 has no battery-backed RTC,
  so the clock resets to the last saved time on every power loss; the phone
  must re-sync after each power-up. Confirm and document this behaviour (and
  whether `fake-hwclock` restored a sensible time).
- Printer settings: either a verified capability is implemented and tested, or
  `docs/printer-settings.md` records exactly what was tried and why nothing was
  added.
