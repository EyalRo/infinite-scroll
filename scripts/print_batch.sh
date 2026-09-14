#!/usr/bin/env bash
# Print a batch of distinct ZPL jobs in sequence, on the installation Pi.
#
# This is the loop actually used to print the 5-post "parody batch"
# (2026-09-13): each named slug's <slug>.zpl in a print-ready directory is
# submitted in turn via print_job.sh, with a short pause between jobs. Unlike
# the single-copy-repeated batch documented on the Knowledge page (30 copies
# of one job), this loops over N *different* jobs.
#
# Usage (run ON the Pi):
#   print_batch.sh <print-ready-dir> <slug1> [slug2 ...]
#
# Example:
#   print_batch.sh /var/lib/infinite-scroll/print-ready \
#     brayden-steelworth persimmon-vale chad-ironframe willow-sterling-cho reginald-huxtable-vance
#
# Stops on the first failed/timed-out job rather than continuing blind -
# a stuck write must be cleared (see print_job.sh) before resuming.

set -u

dir="${1:?usage: print_batch.sh <print-ready-dir> <slug...>}"
shift
SLEEP_BETWEEN="${INFINITE_SCROLL_BATCH_SLEEP:-1}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

for slug in "$@"; do
  job="$dir/$slug.zpl"
  echo "=== $slug ==="
  "$here/print_job.sh" "$job"
  rc=$?
  if [ $rc -ne 0 ]; then
    echo "STOPPED: $slug failed (exit $rc). Fix the printer / clear any stuck write, then re-run starting from this slug." >&2
    exit $rc
  fi
  sleep "$SLEEP_BETWEEN"
done

echo "All jobs submitted."
