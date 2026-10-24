"""The timing spike's harness modules (`spike/harness/`), which this package extends unchanged.

Imported here, once, so every reuse of the timing spike's workload generator, 40% quota rule
and leak scan names the same modules rather than a copy of them. `spike/harness` is on
the import path wherever this package runs: `pytest.ini` for the tests, `ty.toml` for the
type checker, and `PYTHONPATH` from `spike/gaps/gaps.sh` and from the proxy's launcher.
"""

from __future__ import annotations

from pathlib import Path

import quota
import sanitize
import workload

HARNESS = Path(__file__).resolve().parents[2] / "harness"

__all__ = ["HARNESS", "quota", "sanitize", "workload"]
