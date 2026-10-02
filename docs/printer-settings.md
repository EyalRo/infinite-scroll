# Printer settings (Arkscan 2054A) — status

**Nothing is verified yet.** The BLE protocol and `crates/printer` define a
capability mechanism (`printer.capabilities`, `printer.settings.*`), but the
capability list is intentionally empty. A setting is added only after it has
been shown to be readable and/or writable on the physical printer over the
deployed path (`/dev/usb/lp0` via `usblp`, ZPL) — not assumed from the
vendor's Windows software, and not through CUPS.

This sandbox has no printer, so no probing could be done here.

## Procedure

1. On the Pi, with the printer idle and loaded:
   `scripts/probe_printer_settings.sh` — sends only read-only host-status
   queries (`~HS`, `~HQES`, `^HH`) and prints any reply. Whether anything
   comes back also tells us whether `usblp` gives a usable return channel.
2. Density/darkness: ZPL defines `~SD` (0–30) and `^MD` (relative −30…30),
   but support on this firmware is unconfirmed. Test with one small print at
   a visibly different value and compare; record whether the value persists
   across a power cycle, and whether a read-back exists (`^HH` or `~HS`
   output).
3. Record findings below (query, reply, behaviour, firmware if reported).
4. Only then add a typed entry in `crates/printer/src/printer_settings.rs`
   (`key`, range, step, a dedicated read and a dedicated write). Do not add
   a generic "send command" path.

## Findings

_None recorded._
