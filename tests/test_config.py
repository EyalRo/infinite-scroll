from pathlib import Path

from app import config
from app.render import DEFAULT_CSS_PATH


def test_css_path_defaults_to_the_pi_webapp_location(monkeypatch):
    monkeypatch.delenv("INFINITE_SCROLL_CSS_PATH", raising=False)
    assert config.css_path() == Path("/var/lib/infinite-scroll/webapp/template.css")


def test_css_path_honors_env_override(monkeypatch, tmp_path):
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(tmp_path / "custom.css"))
    assert config.css_path() == tmp_path / "custom.css"


def test_current_css_falls_back_to_default_when_no_file_saved(monkeypatch, tmp_path):
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(tmp_path / "missing.css"))
    assert config.current_css() == DEFAULT_CSS_PATH.read_text()


def test_current_css_reads_the_saved_file_when_present(monkeypatch, tmp_path):
    css_file = tmp_path / "saved.css"
    css_file.write_text(".post { color: green; }")
    monkeypatch.setenv("INFINITE_SCROLL_CSS_PATH", str(css_file))
    assert config.current_css() == ".post { color: green; }"
