import os

from flask import Flask

from app.config import backlog_db_path
from app.design_routes import bp as design_bp
from app.routes import bp as compose_bp
from app.scheduler import BacklogScheduler


def create_app(start_scheduler: bool | None = None) -> Flask:
    app = Flask(__name__)
    app.config["MAX_CONTENT_LENGTH"] = 20 * 1024 * 1024
    app.register_blueprint(compose_bp)
    app.register_blueprint(design_bp)

    @app.get("/health")
    def health():
        return {"status": "ok"}

    if start_scheduler is None:
        start_scheduler = os.environ.get("INFINITE_SCROLL_START_SCHEDULER") == "1"
    scheduler = BacklogScheduler(backlog_db_path())
    app.extensions["backlog_scheduler"] = scheduler
    if start_scheduler:
        scheduler.start()

    return app
