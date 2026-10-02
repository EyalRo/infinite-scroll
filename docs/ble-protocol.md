# Infinite Scroll BLE control protocol — version 1

Normative contract between the Pi service (`crates/bluetooth`, binary
`btcontrol`) and any controller (the Android app in `android/`). The two
sides are developed independently against this document.

No application-level authentication: physical proximity is the access
boundary. The protocol exposes only the operations listed below — there is
no shell, filesystem, or raw printer-command operation, and unknown ops are
rejected.

## Design

- Bluetooth is a control surface. Every operation is carried out by the
  existing `printer` / `uploader` services (the watcher converts uploads);
  the Pi service only translates. Uploads go through the uploader's normal
  `partial/ → ready/` path, then the watcher moves them to `complete/`.
- Small control messages are JSON, fragmented into BLE-sized **frames**.
  Bulk data (artwork) uses a **separate binary characteristic** with
  offsets and a CRC — never base64 in JSON.
- Anything large is paged (library) or streamed (upload), so no message
  needs more than `MAX_MESSAGE_BYTES` = 65 536 bytes reassembled.
- Characteristics carry only change *hints*; state is fetched with requests.

## GATT layout

Service `a4f60001-6b8e-4c1f-9d0a-5e1f0c1a1001`, advertised with local name
`Infinite Scroll`. No pairing/encryption required.

| UUID (`a4f6XXXX-6b8e-4c1f-9d0a-5e1f0c1a1001`) | Name | Properties | Purpose |
|---|---|---|---|
| `0002` | Info | read | JSON: `proto`, `name`, `app`, `max_upload_bytes`, `domains` |
| `0003` | Request | write, write-without-response | framed JSON requests |
| `0004` | Response | notify | framed JSON responses |
| `0005` | Changed | notify | 6 bytes: domain mask u16 LE + revision u32 LE |
| `0006` | Upload | write-without-response | binary artwork chunks |

Client sequence: connect → request MTU (517 recommended) → read **Info**
(check `proto`) → enable notifications on **Response** and **Changed** →
send requests.

Info example: `{"proto":1,"name":"Infinite Scroll","app":"0.1.0","max_upload_bytes":20971520,"domains":{"status":1,"library":2,"printer":4,"scheduler":8}}`

## Framing (Request and Response)

Each GATT write/notification is one frame:

```text
byte 0      flags: bit0 = FIRST, bit1 = LAST
bytes 1–2   message id, u16 little endian
bytes 3…    payload slice (UTF-8 JSON, split anywhere)
```

Payload per frame ≤ min(ATT MTU − 3, 512) − 3 (frame header); the 512 cap is
because Android drops characteristic values longer than 512 bytes regardless of
the MTU, so a frame (and an upload chunk, header included) never exceeds it. A message
that fits in one frame has FIRST|LAST. A FIRST frame restarts reassembly. The
Pi sends the frames of one response contiguously. One request in flight at a
time is the supported client behaviour.

## Envelope

Request: `{"v":1,"id":<u16>,"op":"<name>","args":{…}}`

Success: `{"v":1,"id":<same>,"ok":true,"disposition":"completed"|"accepted","result":{…}}`

Failure: `{"v":1,"id":<same>,"ok":false,"error":{"code":"…","message":"…","details":{…}?}}`

### Accepted vs completed

* `completed` — the Pi finished the operation; `result` is final.
* `accepted` — the Pi has **durably taken ownership** and will carry it out
  without the phone (print jobs, upload commit → conversion). `result`
  describes the pending work (e.g. a job). Poll `print.jobs`, or watch the
  `Changed` characteristic, for progress. Disconnecting is safe.

Error codes: `bad_request`, `unsupported_version`, `unknown_op`,
`invalid_argument`, `not_found`, `conflict`, `unavailable` (a backing
service is down), `backend_error`, `upload_incomplete`, `unsupported`,
`internal`.

## Operations

| Op | Args | Disposition | Result |
|---|---|---|---|
| `sys.status` | – | completed | `proto`, `clock.unix_ms`, `services.{printer,uploader}`, `printer` (printer-service status or null), `printer_error`, `bluetooth.version` |
| `sys.clock.get` | – | completed | `unix_ms` |
| `sys.clock.set` | `unix_ms` (int, 2026–2100) | completed | `unix_ms`, `previous_unix_ms`, `scheduler_rescheduled` |
| `library.list` | `offset`?, `limit`? (≤50, default 20) | completed | `total`, `offset`, `items[]`, `next_offset` |
| `library.get` | `id` | completed | `item` |
| `library.thumbnail` | `id`, `width`? (32–320, default 160) | completed | `id`, `format` (`jpeg`), `width`, `height`, `data` (base64). A small grayscale JPEG, ~5 KB, for list views; the full preview is ~90 KB |
| `library.delete` | `id` | completed | `success`, `removed` |
| `upload.begin` | `size`, `name`? | completed | `upload_id`, `size`, `received` |
| `upload.status` | `upload_id` | completed | `received`, `size`, `error` |
| `upload.commit` | `upload_id`, `crc32` | **accepted** | `item_id`, `filename`, `name` |
| `upload.abort` | `upload_id` | completed | `{}` |
| `print.item` | `id`, `copies`? (1–100) | **accepted** | `job` |
| `print.all` | `copies`? (1–100) | **accepted** | `job` (`conflict` if one is active) |
| `print.jobs` | – | completed | `jobs[]` (active + last 20 finished) |
| `print.cancel` | `job_id` | completed | `job` |
| `print.history` | `limit`? (≤200) | completed | `recent[]` newest first |
| `stats.get` | – | completed | see below |
| `sched.get` | – | completed | `enabled`, `min_minutes`, `max_minutes`, `ordering`, `window_enabled`, `window_start`, `window_end`, `next_print_at`, `last_item_id`, `last_error` |
| `sched.set` | `enabled`?, `min_minutes`?, `max_minutes`?, `ordering`? (`random`/`sequential`), `window_enabled`?, `window_start`?, `window_end`? (minutes since local midnight, 0–1439, not equal) | completed | new schedule |
| `sched.preview` | `count`? (1–20) | completed | `ordering`, `exact`, `picks[]` |
| `printer.capabilities` | – | completed | `schema`, `settings[]` |
| `printer.settings.get` | – | completed | `schema`, `values{}` |
| `printer.settings.set` | `key`, `value` (int) | completed | `success` |
| `printer.print_config` | – | completed | `success`; the printer prints its own settings label |
| `wifi.status` | – | completed | `connected`, `ssid`, `signal` (0–100), `security`, `attempt` `{state, ssid, error}` (`state`: `idle`/`connecting`/`connected`/`failed`) |
| `wifi.scan` | – | completed | `networks[]` `{ssid, signal, security (open/wpa/enterprise), in_use}`: rescans (a few seconds), one entry per name with its strongest signal, current network first |
| `wifi.connect` | `ssid`, `password`? (8–63 chars or 64 hex; omit for an open network) | **accepted** | `ssid`; poll `wifi.status` for the outcome |

Times are Unix seconds (floats) except `clock`, which is milliseconds.
`library.list` items: `id`, `original_filename`, `added_at`, `print_count`,
`last_printed_at`. A job: `id`, `kind` (`item`/`all`), `items[]`, `copies`,
`done`, `skipped`, `state` (`queued`/`running`/`done`/`failed`/`cancelled`),
`created_at`, `finished_at`, `error`.

Autoprint window: with `window_enabled` (default on, 10:00–16:00 Pi local time) a scheduled print only fires between `window_start` (inclusive) and `window_end` (exclusive); a window with `start > end` wraps midnight. A print that falls due outside the window waits for the next opening. Manual and queued prints ignore it.

`sched.preview` for `sequential` is exact; for `random` (`exact:false`) it is
one sample of what the scheduler may do — same as the web preview.

### Jobs are Pi-owned

A job is persisted in `jobs.json` before it is acknowledged. The worker
resumes after a restart from the recorded progress, retries device errors
(paper out, cover open) every 15 s, and marks the job `failed` after 5
consecutive failures. Scheduled autoprint continues independently and
shares the printer with jobs. Items deleted after a job is accepted are
skipped (`skipped`).

### Upload procedure

1. `upload.begin {size, name}` → `upload_id`.
2. Stream the file on the **Upload** characteristic. Each write:
   `session u16 LE | offset u32 LE | data` (data ≤ min(MTU − 3, 512) − 6). Offsets must
   be contiguous; chunks that overlap data already held are accepted, so a
   resend from any earlier offset is safe.
3. Optionally `upload.status` to read `received` (and `error` if a chunk was
   lost); resend from `received`.
4. `upload.commit {upload_id, crc32}` (CRC-32/ISO-HDLC of the whole file, as
   an unsigned integer). The Pi verifies length and CRC, hands the bytes to
   the uploader, and answers `accepted` with the staged `item_id`. The item
   appears in `library.list` once the watcher converts it (seconds);
   `failed_count` in `sys.status` rises if conversion fails.

Only PNG and JPEG are accepted (checked by the uploader from the bytes);
limit 20 MB. One upload session at a time; a new `upload.begin` replaces the
previous session; idle sessions expire after 120 s.

### Change notifications

`Changed` carries `mask` (u16 LE) and a revision counter. Bits: `1` status,
`2` library, `4` printer/jobs, `8` scheduler. On a hint, re-request the
corresponding data (`sys.status`; `library.list`; `print.jobs` +
`sys.status`; `sched.get`). Hints are best-effort (the Pi polls its services
every 2 s while a client is subscribed); clients should also refresh on
connect and after their own commands.

### Statistics (`stats.get`)

`library_items`, `failed_conversions`, `prints_ok`, `prints_failed`,
`scheduled_ok`, `job_ok`, `manual_ok` (web "print now"), `paper_mm` (estimated from each job's `^LL` at 203
DPI), `counters_since`, `item_print_total`. Counters start when this
version first ran; earlier prints are not backfilled. The previous app's
"immediate vs backlog" split does not exist in the Rust services and is not
reported.

### Wi-Fi

`wifi.connect` creates a NetworkManager profile (`infinite-scroll-<ssid>`), brings it up (up to 30 s) and returns at once as *accepted*; the outcome is `wifi.status.attempt`. If the network fails, the profile is deleted and the Pi re-activates the network it was on, so a wrong password never strands it; Bluetooth is unaffected either way. WPA-personal and open networks only (the app does not offer `enterprise` networks). The password is passed to `nmcli` and is never logged or returned. NetworkManager authorizes the service through `deploy/50-infinite-scroll-wifi.rules`: NM's stock polkit rule only trusts active login sessions, which a service does not have.

### Printer settings

Capability-based and versioned. `printer.capabilities` lists only settings
confirmed against the physical Arkscan 2054A over the validated raw-USB
path, each as `{key, label, min, max, step, readable, writable}`, plus
`actions[]` (`{key, label}`) for one-shot commands. Currently:

- setting `darkness` (0–30, step 1): `printer.settings.get` reads the live value
  back from the printer; `printer.settings.set` writes it, verifies it by
  reading it back, and the Pi re-applies it with every job so a printer power
  cycle cannot revert it. A failure to reach the printer is `error` in the
  `printer.settings.get` result (with empty `values`), not a failed request.
- action `print_config` (`printer.print_config`): the printer prints its own
  settings label.

`printer.settings.set` for any other key returns `unsupported`. A controller
must render the settings UI from `printer.capabilities` and show nothing for
what is not listed.

## Versioning

`proto` in Info and `v` in every message is the major version. Within a
version, new ops, new result fields, new capabilities and new error codes
may appear; clients must ignore what they do not know. Anything else bumps
`v`; the Pi answers an unsupported `v` with `unsupported_version` and
`details.supported`.
