"""Serialized, bounded-timeout writes to the Arkscan's raw USB device.

A write that blocks forever (stuck printer, no reader) must not hang the
whole web app, and two prints must never write to the device concurrently
-- both were observed as real failure modes while printing manually.
"""
import os
import stat
import threading
from dataclasses import dataclass


@dataclass
class PrintResult:
    success: bool
    message: str


_print_lock = threading.Lock()


def _device_path() -> str:
    return os.environ.get("INFINITE_SCROLL_PRINTER_DEVICE", "/dev/usb/lp0")


def _preflight(zpl_text: str, device_path: str) -> str | None:
    if not zpl_text:
        return "job is empty"
    if not os.path.exists(device_path):
        return f"printer device {device_path} not present"
    if not stat.S_ISCHR(os.stat(device_path).st_mode):
        return f"{device_path} is not a character device"
    return None


def print_zpl(zpl_text: str, device_path: str | None = None, timeout_s: float = 15.0) -> PrintResult:
    device_path = device_path or _device_path()

    error = _preflight(zpl_text, device_path)
    if error:
        return PrintResult(success=False, message=error)

    if not _print_lock.acquire(blocking=False):
        return PrintResult(success=False, message="printer busy: a previous print is still in progress")

    done = threading.Event()
    outcome: dict = {}

    def _write():
        try:
            with open(device_path, "wb") as f:
                f.write(zpl_text.encode("ascii"))
                f.flush()
            outcome["ok"] = True
        except OSError as exc:
            outcome["ok"] = False
            outcome["error"] = str(exc)
        finally:
            done.set()
            _print_lock.release()

    threading.Thread(target=_write, daemon=True).start()

    if done.wait(timeout_s):
        if outcome.get("ok"):
            return PrintResult(success=True, message="printed")
        return PrintResult(success=False, message=f"write failed: {outcome.get('error')}")

    return PrintResult(
        success=False,
        message=f"write to printer timed out after {timeout_s:.0f}s — check paper, cover, and power on the Arkscan",
    )
