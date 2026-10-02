#!/usr/bin/env bash
# Read-only probe: what does the Arkscan 2054A answer over the validated raw
# USB path (/dev/usb/lp0, usblp)? Run on the Pi with the printer idle.
# Sends ONLY host-status queries (no print, no setting changes) and prints
# whatever comes back. Record the output in docs/printer-settings.md before
# adding any capability to crates/printer/src/printer_settings.rs.
set -euo pipefail
DEV="${1:-/dev/usb/lp0}"
test -c "$DEV" || { echo "$DEV is not a character device" >&2; exit 1; }

query() {
  local label="$1" command="$2"
  echo "== $label ($command)"
  # Open for read first so the reply is not lost, send, wait, dump.
  exec 3<"$DEV"
  printf '%s' "$command" > "$DEV"
  timeout 3 cat <&3 | od -c | head -20 || true
  exec 3<&-
}

query "host status"          "~HS"
query "extended status"      "~HQES"
query "printer config"       "^XA^HH^XZ"
echo "No reply to a query means that query is not readable on this unit."
