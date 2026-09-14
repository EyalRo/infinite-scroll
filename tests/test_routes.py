from unittest.mock import patch

from app import create_app
from app.printer import PrintResult

SAMPLE_FORM = {
    "name": "Test Poster",
    "title": "Testing · QA",
    "avatar_initials": "TP",
    "body": "Paragraph one.\n\nParagraph two.",
    "reactions": "10",
    "comments": "2",
    "reposts": "1",
}


def test_compose_form_renders():
    client = create_app().test_client()
    resp = client.get("/")
    assert resp.status_code == 200
    assert b"Compose a post" in resp.data


def test_print_rejects_missing_required_field():
    client = create_app().test_client()
    form = dict(SAMPLE_FORM)
    del form["name"]
    resp = client.post("/print", data=form)
    assert resp.status_code == 400
    assert "name" in resp.get_json()["message"]


def test_print_success_saves_a_record(tmp_path, monkeypatch):
    monkeypatch.setenv("INFINITE_SCROLL_PRINT_READY_DIR", str(tmp_path))
    with patch("app.routes.print_zpl", return_value=PrintResult(success=True, message="printed")):
        client = create_app().test_client()
        resp = client.post("/print", data=SAMPLE_FORM)

    assert resp.status_code == 200
    body = resp.get_json()
    assert body["success"] is True
    assert body["record"]["png_path"].startswith(str(tmp_path))


def test_print_failure_does_not_save_a_record(tmp_path, monkeypatch):
    monkeypatch.setenv("INFINITE_SCROLL_PRINT_READY_DIR", str(tmp_path))
    with patch("app.routes.print_zpl", return_value=PrintResult(success=False, message="printer busy")):
        client = create_app().test_client()
        resp = client.post("/print", data=SAMPLE_FORM)

    assert resp.status_code == 502
    body = resp.get_json()
    assert body["success"] is False
    assert body["record"] is None
    assert list(tmp_path.iterdir()) == []
