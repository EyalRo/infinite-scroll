import os
from unittest.mock import patch

from app.printer import PrintResult, print_zpl


def test_print_zpl_rejects_empty_job(tmp_path):
    device = tmp_path / "lp0"
    device.write_bytes(b"")
    result = print_zpl("", device_path=str(device))
    assert result == PrintResult(success=False, message="job is empty")


def test_print_zpl_rejects_missing_device(tmp_path):
    missing = tmp_path / "does-not-exist"
    result = print_zpl("^XA^XZ", device_path=str(missing))
    assert result.success is False
    assert "not present" in result.message


def test_print_zpl_rejects_non_char_device(tmp_path):
    regular_file = tmp_path / "not-a-device"
    regular_file.write_bytes(b"")
    with patch("app.printer.stat.S_ISCHR", return_value=False):
        result = print_zpl("^XA^XZ", device_path=str(regular_file))
    assert result.success is False
    assert "not a character device" in result.message


def test_print_zpl_writes_when_device_accepts_data(tmp_path):
    device = tmp_path / "fake-lp0"
    device.write_bytes(b"")
    with patch("app.printer.stat.S_ISCHR", return_value=True):
        result = print_zpl("^XA^XZ", device_path=str(device))
    assert result == PrintResult(success=True, message="printed")
    assert device.read_bytes() == b"^XA^XZ"


def test_print_zpl_times_out_on_a_stuck_write(tmp_path):
    fifo_path = tmp_path / "stuck-lp0"
    os.mkfifo(fifo_path)
    with patch("app.printer.stat.S_ISCHR", return_value=True):
        result = print_zpl("^XA^XZ", device_path=str(fifo_path), timeout_s=0.5)
    assert result.success is False
    assert "timed out" in result.message


def test_print_zpl_reports_busy_while_a_stuck_write_holds_the_lock(tmp_path):
    fifo_path = tmp_path / "stuck-lp0-2"
    os.mkfifo(fifo_path)
    with patch("app.printer.stat.S_ISCHR", return_value=True):
        first = print_zpl("^XA^XZ", device_path=str(fifo_path), timeout_s=0.3)
        second = print_zpl("^XA^XZ", device_path=str(fifo_path), timeout_s=0.3)
    assert first.success is False
    assert "timed out" in first.message
    assert second.success is False
    assert "busy" in second.message
