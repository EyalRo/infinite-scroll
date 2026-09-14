import threading

import pytest


@pytest.fixture(autouse=True)
def reset_printer_lock():
    """Reset the global printer lock before each test to ensure test isolation.

    Background threads from timeout tests continue running after the test
    function returns. This fixture creates a fresh lock before each test.
    """
    import app.printer

    # Replace the global lock with a fresh one before each test
    app.printer._print_lock = threading.Lock()
    yield
