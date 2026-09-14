import io
from unittest.mock import patch

from PIL import Image

from app import create_app
from app.printer import PrintResult


FORM = {"kind": "linkedin", "action": "enqueue", "name": "Queue Person", "title": "Artist", "avatar_initials": "QP", "body": "Hello", "reactions": "1", "comments": "2", "reposts": "3"}


def configure(monkeypatch, tmp_path):
    monkeypatch.setenv("INFINITE_SCROLL_BACKLOG_DB", str(tmp_path / "backlog.sqlite3"))
    monkeypatch.setenv("INFINITE_SCROLL_UPLOADS_DIR", str(tmp_path / "uploads"))
    monkeypatch.setenv("INFINITE_SCROLL_PRINT_READY_DIR", str(tmp_path / "printed"))


def test_linkedin_post_can_be_added_listed_moved_and_deleted(tmp_path, monkeypatch):
    configure(monkeypatch, tmp_path)
    client = create_app().test_client()
    first = client.post("/api/submit", data=FORM).get_json()["item"]
    second_form = {**FORM, "name": "Second"}
    second = client.post("/api/submit", data=second_form).get_json()["item"]
    assert len(client.get("/api/backlog").get_json()["items"]) == 2
    assert client.post(f"/api/backlog/{second['id']}/move", json={"direction": "up"}).get_json()["success"]
    assert client.delete(f"/api/backlog/{first['id']}").status_code == 200


def test_png_can_be_added_to_backlog(tmp_path, monkeypatch):
    configure(monkeypatch, tmp_path)
    output = io.BytesIO(); Image.new("RGB", (20, 30), "black").save(output, "PNG")
    client = create_app().test_client()
    response = client.post("/api/submit", data={"kind": "image", "action": "enqueue", "image_label": "Picture", "image": (io.BytesIO(output.getvalue()), "picture.png")})
    assert response.status_code == 200
    item = response.get_json()["item"]
    assert item["kind"] == "image"
    assert (tmp_path / "uploads").iterdir()


def test_scheduler_settings_validation_and_update(tmp_path, monkeypatch):
    configure(monkeypatch, tmp_path)
    client = create_app().test_client()
    bad = client.post("/api/scheduler", json={"enabled": True, "min_minutes": 20, "max_minutes": 10})
    assert bad.status_code == 400
    good = client.post("/api/scheduler", json={"enabled": True, "min_minutes": 15, "max_minutes": 20, "ordering": "random", "run_mode": "loop"})
    assert good.status_code == 200
    assert good.get_json()["scheduler"]["enabled"] == 1


def test_print_now_does_not_leave_item_in_backlog(tmp_path, monkeypatch):
    configure(monkeypatch, tmp_path)
    with patch("app.routes.print_item", return_value=(PrintResult(True, "printed"), {})):
        response = create_app().test_client().post("/api/submit", data={**FORM, "action": "print"})
    assert response.status_code == 200
    assert create_app().test_client().get("/api/backlog").get_json()["items"] == []
