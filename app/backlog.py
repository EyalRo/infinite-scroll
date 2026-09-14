"""SQLite-backed print queue and durable scheduler settings."""
import json
import sqlite3
import threading
import time
from pathlib import Path

_db_lock = threading.RLock()

SCHEMA = """
CREATE TABLE IF NOT EXISTS backlog_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK(kind IN ('linkedin', 'image')),
    label TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT '{}',
    image_path TEXT,
    position INTEGER NOT NULL,
    print_count INTEGER NOT NULL DEFAULT 0,
    last_printed_at REAL,
    created_at REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS scheduler_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    enabled INTEGER NOT NULL DEFAULT 0,
    min_minutes REAL NOT NULL DEFAULT 15,
    max_minutes REAL NOT NULL DEFAULT 20,
    ordering TEXT NOT NULL DEFAULT 'sequential',
    run_mode TEXT NOT NULL DEFAULT 'once',
    next_print_at REAL,
    last_item_id INTEGER,
    last_error TEXT
);
INSERT OR IGNORE INTO scheduler_settings(singleton) VALUES (1);
"""


def initialize(db_path: Path) -> None:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    with _db_lock, sqlite3.connect(db_path) as db:
        db.executescript(SCHEMA)


def _connect(db_path: Path):
    db = sqlite3.connect(db_path)
    db.row_factory = sqlite3.Row
    return db


def add_item(db_path: Path, kind: str, label: str, payload=None, image_path=None) -> dict:
    initialize(db_path)
    with _db_lock, _connect(db_path) as db:
        position = db.execute("SELECT COALESCE(MAX(position), 0) + 1 FROM backlog_items").fetchone()[0]
        cursor = db.execute(
            "INSERT INTO backlog_items(kind,label,payload,image_path,position,created_at) VALUES(?,?,?,?,?,?)",
            (kind, label, json.dumps(payload or {}), image_path, position, time.time()),
        )
        item_id = cursor.lastrowid
    return get_item(db_path, item_id)


def _item(row) -> dict:
    result = dict(row)
    result["payload"] = json.loads(result["payload"])
    return result


def get_item(db_path: Path, item_id: int) -> dict | None:
    initialize(db_path)
    with _db_lock, _connect(db_path) as db:
        row = db.execute("SELECT * FROM backlog_items WHERE id=?", (item_id,)).fetchone()
    return _item(row) if row else None


def list_items(db_path: Path) -> list[dict]:
    initialize(db_path)
    with _db_lock, _connect(db_path) as db:
        rows = db.execute("SELECT * FROM backlog_items ORDER BY position, id").fetchall()
    return [_item(row) for row in rows]


def delete_item(db_path: Path, item_id: int) -> dict | None:
    item = get_item(db_path, item_id)
    if not item:
        return None
    with _db_lock, _connect(db_path) as db:
        db.execute("DELETE FROM backlog_items WHERE id=?", (item_id,))
    return item


def move_item(db_path: Path, item_id: int, direction: str) -> bool:
    items = list_items(db_path)
    index = next((i for i, item in enumerate(items) if item["id"] == item_id), None)
    if index is None:
        return False
    target = index - 1 if direction == "up" else index + 1
    if target < 0 or target >= len(items):
        return False
    with _db_lock, _connect(db_path) as db:
        db.execute("UPDATE backlog_items SET position=? WHERE id=?", (items[target]["position"], item_id))
        db.execute("UPDATE backlog_items SET position=? WHERE id=?", (items[index]["position"], items[target]["id"]))
    return True


def settings(db_path: Path) -> dict:
    initialize(db_path)
    with _db_lock, _connect(db_path) as db:
        return dict(db.execute("SELECT * FROM scheduler_settings WHERE singleton=1").fetchone())


def update_settings(db_path: Path, **values) -> dict:
    initialize(db_path)
    allowed = {"enabled", "min_minutes", "max_minutes", "ordering", "run_mode", "next_print_at", "last_item_id", "last_error"}
    values = {key: value for key, value in values.items() if key in allowed}
    if values:
        assignments = ", ".join(f"{key}=?" for key in values)
        with _db_lock, _connect(db_path) as db:
            db.execute(f"UPDATE scheduler_settings SET {assignments} WHERE singleton=1", tuple(values.values()))
    return settings(db_path)


def mark_printed(db_path: Path, item_id: int, printed_at: float) -> None:
    with _db_lock, _connect(db_path) as db:
        db.execute(
            "UPDATE backlog_items SET print_count=print_count+1,last_printed_at=? WHERE id=?",
            (printed_at, item_id),
        )
