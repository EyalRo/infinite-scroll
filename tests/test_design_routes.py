from app import create_app


def test_design_editor_shows_current_css(tmp_path, monkeypatch):
    css_file = tmp_path / "template.css"
    css_file.write_text(".post { color: red; }")
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(css_file))

    client = create_app().test_client()
    resp = client.get("/design")
    assert resp.status_code == 200
    assert b".post { color: red; }" in resp.data


def test_design_editor_falls_back_to_default_css_when_none_saved(tmp_path, monkeypatch):
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(tmp_path / "does-not-exist.css"))
    client = create_app().test_client()
    resp = client.get("/design")
    assert resp.status_code == 200
    assert b".post {" in resp.data


def test_design_preview_returns_a_data_url_png():
    client = create_app().test_client()
    resp = client.post("/design/preview", json={"css": ".post { color: #000; }"})
    assert resp.status_code == 200
    assert resp.get_json()["image"].startswith("data:image/png;base64,")


def test_design_save_writes_the_css_file(tmp_path, monkeypatch):
    css_file = tmp_path / "webapp" / "template.css"
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(css_file))
    client = create_app().test_client()
    resp = client.post("/design", json={"css": ".post { color: blue; }"})
    assert resp.status_code == 200
    assert resp.get_json()["success"] is True
    assert css_file.read_text() == ".post { color: blue; }"
