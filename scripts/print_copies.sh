#!/usr/bin/env bash
# Submit one validated ZPL job repeatedly, stopping on the first failure.
#
# Usage (on the Pi):
#   print_copies.sh <job.zpl> <copies>
#
# The 2026-09-13 acceptance run printed 30 copies with a 0.5-second pause.

set -u

job="${1:?usage: print_copies.sh <job.zpl> <copies>}"
copies="${2:?usage: print_copies.sh <job.zpl> <copies>}"
sleep_between="${INFINITE_SCROLL_COPY_SLEEP:-0.5}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "$copies" in
  ''|*[!0-9]*) echo "FAIL: copies must be a positive integer" >&2; exit 1 ;;
esac
if [ "$copies" -lt 1 ]; then
  echo "FAIL: copies must be at least 1" >&2
  exit 1
fi

for copy in $(seq 1 "$copies"); do
  "$here/print_job.sh" "$job" || exit $?
  echo "sent $copy/$copies"
  if [ "$copy" -lt "$copies" ]; then
    sleep "$sleep_between"
  fi
done

echo "All $copies jobs submitted."
