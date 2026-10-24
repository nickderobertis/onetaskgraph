"""The timing spike's harness modules (`spike/harness/`), which this package extends unchanged.

Imported here, once, so every reuse of the timing spike's workload generator, 40% quota rule,
proxy and leak scan names the same modules rather than a copy of them. `spike/harness` is on
the import path wherever this package runs: `pythonpath` for the tests, `extra-paths` for the
type checker, and `PYTHONPATH` from `spike/gaps.sh` and from the proxy's launcher.
"""

from __future__ import annotations

from pathlib import Path

import proxy
import quota
import sanitize
import workload

HARNESS = Path(__file__).resolve().parents[2] / "harness"

__all__ = ["HARNESS", "proxy", "quota", "sanitize", "workload"]
