#!/usr/bin/env bash
# Print a batch of distinct ZPL jobs in sequence, on the installation Pi.
#
# Each named item's <id>.zpl in the given directory is submitted in turn via
# print_job.sh, with a short pause between jobs. Unlike print_copies.sh (many
# copies of one job), this loops over N *different* jobs. Hardware
# troubleshooting only; normal batch printing is the printer service's
# print-all job.
#
# Usage (run ON the Pi):
#   print_batch.sh <zpl-dir> <id1> [id2 ...]
#
# Example:
#   print_batch.sh /var/lib/infinite-scroll/complete <id-one> <id-two>
#
# Stops on the first failed/timed-out job rather than continuing blind -
# a stuck write must be cleared (see print_job.sh) before resuming.

set -u

dir="${1:?usage: print_batch.sh <zpl-dir> <id...>}"
shift
SLEEP_BETWEEN="${INFINITE_SCROLL_BATCH_SLEEP:-1}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

for id in "$@"; do
  job="$dir/$id.zpl"
  echo "=== $id ==="
  "$here/print_job.sh" "$job"
  rc=$?
  if [ $rc -ne 0 ]; then
    echo "STOPPED: $id failed (exit $rc). Fix the printer / clear any stuck write, then re-run starting from this id." >&2
    exit $rc
  fi
  sleep "$SLEEP_BETWEEN"
done

echo "All jobs submitted."
