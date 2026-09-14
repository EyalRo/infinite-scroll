from app import backlog


def test_backlog_persists_mixed_items_and_order(tmp_path):
    db = tmp_path / "queue.sqlite3"
    first = backlog.add_item(db, "linkedin", "First", {"name": "First"})
    second = backlog.add_item(db, "image", "Artwork", {}, "/tmp/art.png")
    assert [item["kind"] for item in backlog.list_items(db)] == ["linkedin", "image"]

    assert backlog.move_item(db, second["id"], "up") is True
    assert [item["label"] for item in backlog.list_items(db)] == ["Artwork", "First"]
    assert backlog.move_item(db, 999, "up") is False

    backlog.mark_printed(db, first["id"], 1234)
    assert backlog.get_item(db, first["id"])["print_count"] == 1


def test_scheduler_settings_are_durable(tmp_path):
    db = tmp_path / "queue.sqlite3"
    backlog.update_settings(db, enabled=1, min_minutes=45, max_minutes=60, ordering="random", run_mode="loop")
    config = backlog.settings(db)
    assert config["enabled"] == 1
    assert (config["min_minutes"], config["max_minutes"]) == (45, 60)
    assert (config["ordering"], config["run_mode"]) == ("random", "loop")
