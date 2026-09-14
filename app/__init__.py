from flask import Flask

from app.design_routes import bp as design_bp
from app.routes import bp as compose_bp


def create_app() -> Flask:
    app = Flask(__name__)
    app.register_blueprint(compose_bp)
    app.register_blueprint(design_bp)

    @app.get("/health")
    def health():
        return {"status": "ok"}

    return app
