from unittest.mock import patch

from app import backlog
from app.printer import PrintResult
from app.scheduler import BacklogScheduler, choose_item


class FixedRandom:
    def choice(self, items):
        return items[-1]

    def uniform(self, minimum, maximum):
        return (minimum + maximum) / 2


def test_choose_item_supports_once_loop_and_random(tmp_path):
    db = tmp_path / "queue.sqlite3"
    a = backlog.add_item(db, "linkedin", "A", {})
    b = backlog.add_item(db, "linkedin", "B", {})
    items = backlog.list_items(db)
    assert choose_item(items, {"run_mode": "once", "ordering": "sequential", "last_item_id": None})["id"] == a["id"]
    assert choose_item(items, {"run_mode": "loop", "ordering": "sequential", "last_item_id": a["id"]})["id"] == b["id"]
    assert choose_item(items, {"run_mode": "loop", "ordering": "random", "last_item_id": a["id"]}, FixedRandom())["id"] == b["id"]


def test_due_tick_prints_one_and_schedules_next(tmp_path):
    db = tmp_path / "queue.sqlite3"
    item = backlog.add_item(db, "linkedin", "A", {})
    backlog.update_settings(db, enabled=1, min_minutes=15, max_minutes=20, next_print_at=99)
    scheduler = BacklogScheduler(db, clock=lambda: 100, rng=FixedRandom())
    with patch("app.scheduler.print_item", return_value=(PrintResult(True, "printed"), {})):
        scheduler.tick()
    assert backlog.get_item(db, item["id"])["print_count"] == 1
    assert backlog.settings(db)["next_print_at"] == 100 + 17.5 * 60


def test_once_mode_stops_after_all_items_print(tmp_path):
    db = tmp_path / "queue.sqlite3"
    item = backlog.add_item(db, "linkedin", "A", {})
    backlog.mark_printed(db, item["id"], 50)
    backlog.update_settings(db, enabled=1, run_mode="once", next_print_at=99)
    BacklogScheduler(db, clock=lambda: 100).tick()
    assert backlog.settings(db)["enabled"] == 0
