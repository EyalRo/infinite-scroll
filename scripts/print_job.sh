#!/usr/bin/env bash
# Print one ZPL job to the Arkscan 2054A over raw USB, on the installation Pi.
#
# This is the validated single-print procedure from the Ops-in-Net Knowledge
# page "Infinite Scroll", extracted into a script and hardened with a bounded
# write timeout after a real incident (2026-09-13): a stalled/offline printer
# left `cat job.zpl > /dev/usb/lp0` hanging indefinitely, blocking the SSH
# session until manually killed. This script times out instead of hanging.
#
# Usage (run ON the Pi, or via: ssh stags@infinite-scroll.local 'bash -s' < print_job.sh -- <job.zpl>):
#   print_job.sh <path-to-zpl-job>
#
# Exit codes: 0 = printed (kernel accepted the write), 1 = preflight failed,
# 2 = write timed out (printer likely offline/paper-out/cover-open), 3 = write failed.

set -u

DEVICE="${INFINITE_SCROLL_LP_DEVICE:-/dev/usb/lp0}"
WRITE_TIMEOUT="${INFINITE_SCROLL_PRINT_TIMEOUT:-15}"

job="${1:?usage: print_job.sh <path-to-zpl-job>}"

if [ ! -s "$job" ]; then
  echo "FAIL: job file missing or empty: $job" >&2
  exit 1
fi

if [ ! -c "$DEVICE" ]; then
  echo "FAIL: $DEVICE not present (printer not enumerated? check lsusb)" >&2
  exit 1
fi

sha=$(sha256sum "$job" | cut -d' ' -f1)
ts=$(date -Iseconds)

if timeout "$WRITE_TIMEOUT" sudo sh -c "cat \"$job\" > \"$DEVICE\""; then
  sync
  echo "OK job=$job sha256=$sha submitted_at=$ts"
  exit 0
else
  rc=$?
  if [ "$rc" -eq 124 ]; then
    echo "FAIL: write to $DEVICE timed out after ${WRITE_TIMEOUT}s (job=$job) - check paper/cover/power on the Arkscan, then kill any stuck 'cat' process writing to $DEVICE before retrying" >&2
    exit 2
  fi
  echo "FAIL: write to $DEVICE failed (job=$job, rc=$rc)" >&2
  exit 3
fi
