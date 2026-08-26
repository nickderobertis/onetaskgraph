#!/usr/bin/env python3
"""Read and update every published copy of the product version."""

import json
import re
import sys
from pathlib import Path
from typing import Dict, Optional


VERSION_FILES = {
    Path("Cargo.toml"): re.compile(r'(?m)^(\[workspace\.package\]\nversion\s*=\s*")([^"]+)(")'),
    Path("sdks/python/pyproject.toml"): re.compile(r'(?m)^(version\s*=\s*")([^"]+)(")'),
    Path("sdks/python/src/onetaskgraph_sdk/__init__.py"): re.compile(
        r'(?m)^(__version__\s*=\s*")([^"]+)(")'
    ),
    Path("sdks/typescript/package.json"): None,
}


def read_product_versions() -> Dict[str, Optional[str]]:
    """Return every version-bearing path and its declared version."""
    versions = {}
    for path, pattern in VERSION_FILES.items():
        if pattern is None:
            versions[str(path)] = json.loads(path.read_text()).get("version")
            continue
        match = pattern.search(path.read_text())
        versions[str(path)] = match.group(2) if match else None
    return versions


def set_product_versions(version: str) -> None:
    """Set every published product version, preserving each file's format."""
    for path, pattern in VERSION_FILES.items():
        if pattern is None:
            package = json.loads(path.read_text())
            package["version"] = version
            path.write_text(json.dumps(package, indent=2) + "\n")
            continue
        text = path.read_text()
        updated, count = pattern.subn(rf"\g<1>{version}\g<3>", text, count=1)
        if count != 1:
            raise ValueError(f"{path}: no product version could be read")
        path.write_text(updated)


def main() -> int:
    if len(sys.argv) != 3 or sys.argv[1] not in {"check", "set"}:
        print("usage: scripts/product_versions.py check|set VERSION", file=sys.stderr)
        return 2
    operation, expected = sys.argv[1:]
    if operation == "set":
        set_product_versions(expected)
        return 0

    failed = False
    for path, actual in read_product_versions().items():
        if actual != expected:
            print(f"{path} has {actual}; expected {expected}", file=sys.stderr)
            failed = True
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
