# Printer settings (Arkscan 2054A) — status

The BLE protocol and `crates/printer` define a capability mechanism
(`printer.capabilities`, `printer.settings.*`). A setting is added only after
it has been shown to work on the physical printer over the deployed path
(`/dev/usb/lp0` via `usblp`, ZPL) — not assumed from the vendor's Windows
software, and not through CUPS.

## Verified on hardware (2026-10-02)

Printer: `B4B-2054A`, firmware `V1.040.4668`, 650-dot print width.

| Capability | Command | Result |
|---|---|---|
| Read status | `~HS` | Replies on the return channel (three `STX…ETX` lines). |
| Identify | `^XA^HZA^XZ` | Replies `B4B-2054A,V1.040.4668,8.0,5083KB`. |
| **Read settings** | `^XA^HH^XZ` | Replies with the full configuration report as text; **does not print**. Includes `DARKNESS`. |
| **Darkness (write)** | `~SD<0–30>` | Takes effect; read back via `^HH` (`10.0` → `20.0` → `25.0` → `10.0`). |
| **Print settings label** | `~WC` | Prints the printer's configuration label. |

Not supported or not useful on this firmware: `~HQES`, `~HI`, `~HM`, `~HD`,
`^HZO`, SGD `getvar` (no reply); `^MD` (accepted, no visible change in the
`^HH` darkness value).

Notes:

- usblp allows only one opener at a time: open the device once read/write
  (`exec 3<>/dev/usb/lp0`); a separate read open makes the write fail with
  "Device or resource busy". The printer service does this in
  `printer_device::query`.
- Whether `~SD` survives a printer power cycle is **unknown**, so the printer
  service stores the chosen darkness in its state and prepends `~SD<n>` to
  every job.
- The default darkness on this unit was `10.0`.

## Exposed

- `darkness` (0–30): read from the printer, set with verification.
- `print_config` action: prints the settings label.

## Procedure for adding another setting

1. On the Pi, with the printer idle: `scripts/probe_printer_settings.sh`
   sends read-only host-status queries and prints any reply.
2. Test the write with a visibly different value and read it back (`^HH`).
3. Record findings above.
4. Add a typed entry in `crates/printer/src/printer_settings.rs` (`key`,
   range, step, a dedicated read and write). Do not add a generic "send
   command" path.
