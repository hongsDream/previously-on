"""Tiny no-dependency runner for this fixture's plain assertion tests."""

from contextlib import contextmanager
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
import sys
import traceback


@contextmanager
def raises(expected):
    try:
        yield
    except expected:
        return
    raise AssertionError(f"expected {expected.__name__}")


def _run(path):
    spec = spec_from_file_location("fixture_test", path)
    module = module_from_spec(spec)
    spec.loader.exec_module(module)
    failures = 0
    for name in sorted(dir(module)):
        value = getattr(module, name)
        if name.startswith("test_") and callable(value):
            try:
                value()
                print(f"PASSED {name}")
            except Exception:
                failures += 1
                print(f"FAILED {name}")
                traceback.print_exc()
    return failures


if __name__ == "__main__":
    selected = [arg for arg in sys.argv[1:] if not arg.startswith("-")]
    if not selected:
        selected = ["tests/test_retry.py"]
    failures = sum(_run(Path(path)) for path in selected)
    raise SystemExit(1 if failures else 0)
