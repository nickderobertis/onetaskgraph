#!/usr/bin/env python3
"""Read and update the cross-ecosystem product versions reconciled by workspace checks."""

import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, NewType, Optional, Tuple, Union


SEMANTIC_VERSION_RE = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
    r"(?:-(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
SemanticVersion = NewType("SemanticVersion", str)


@dataclass(frozen=True)
class RegexVersionFile:
    path: Path
    pattern: re.Pattern


@dataclass(frozen=True)
class JsonVersionFile:
    path: Path


VersionFile = Union[RegexVersionFile, JsonVersionFile]
RECONCILED_VERSION_FILES: Tuple[VersionFile, ...] = (
    RegexVersionFile(
        Path("Cargo.toml"),
        re.compile(r'(?m)^(\[workspace\.package\]\nversion\s*=\s*")([^"]+)(")'),
    ),
    RegexVersionFile(
        Path("sdks/python/pyproject.toml"),
        re.compile(r'(?m)^(version\s*=\s*")([^"]+)(")'),
    ),
    RegexVersionFile(
        Path("sdks/python/src/onetaskgraph_sdk/__init__.py"),
        re.compile(r'(?m)^(__version__\s*=\s*")([^"]+)(")'),
    ),
    JsonVersionFile(Path("sdks/typescript/package.json")),
)


# llmlint: ignore[async_typed_clients_at_boundaries] This local manifest read is part of the short-lived synchronous release transaction and has no concurrent work.
def read_json_manifest(path: Path) -> Dict[str, object]:
    """Decode a package manifest only after validating its object boundary."""
    decoded = json.loads(path.read_text())
    if not isinstance(decoded, dict) or not all(isinstance(key, str) for key in decoded):
        raise ValueError(f"{path}: package manifest must be a JSON object")
    return decoded


# llmlint: ignore[async_typed_clients_at_boundaries] This preflight is part of the ordered synchronous release transaction and must finish before mutation begins.
def require_writable_manifests() -> None:
    """Refuse the update before mutation when any reconciled file is not writable."""
    for version_file in RECONCILED_VERSION_FILES:
        with version_file.path.open("r+"):
            pass


# llmlint: ignore[async_typed_clients_at_boundaries, structural_pattern_matching] This short-lived release command has no concurrent work, and Python 3.8 support precludes match/case syntax.
def read_reconciled_versions() -> Dict[str, Optional[SemanticVersion]]:
    """Return the cross-ecosystem versions checked for mutual agreement."""
    versions = {}
    for version_file in RECONCILED_VERSION_FILES:
        path = version_file.path
        if isinstance(version_file, JsonVersionFile):
            package = read_json_manifest(path)
            value = package.get("version")
            versions[path.as_posix()] = (
                SemanticVersion(value)
                if isinstance(value, str) and SEMANTIC_VERSION_RE.fullmatch(value)
                else None
            )
            continue
        match = version_file.pattern.search(path.read_text())
        value = match.group(2) if match else None
        versions[path.as_posix()] = (
            SemanticVersion(value)
            if value is not None and SEMANTIC_VERSION_RE.fullmatch(value)
            else None
        )
    return versions


# llmlint: ignore[async_typed_clients_at_boundaries, structural_pattern_matching] This ordered local transaction is synchronous by design, and Python 3.8 support requires explicit variant checks.
def set_reconciled_versions(version: SemanticVersion) -> None:
    """Set the cross-ecosystem versions checked for mutual agreement."""
    # Validate the whole boundary before changing the first file, so malformed input cannot
    # leave the version set partially updated.
    existing = read_reconciled_versions()
    invalid = [path for path, value in existing.items() if value is None]
    if invalid:
        raise ValueError(
            f"{', '.join(invalid)}: no valid semantic product version could be read; "
            "next: restore its version field and rerun scripts/set-version.sh"
        )
    require_writable_manifests()
    for version_file in RECONCILED_VERSION_FILES:
        path = version_file.path
        if isinstance(version_file, JsonVersionFile):
            package = read_json_manifest(path)
            package["version"] = version
            path.write_text(json.dumps(package, indent=2) + "\n")
            continue
        text = path.read_text()
        updated, count = version_file.pattern.subn(rf"\g<1>{version}\g<3>", text, count=1)
        if count != 1:
            raise ValueError(
                f"{path}: no product version could be read; next: restore its version field "
                "and rerun scripts/set-version.sh"
            )
        path.write_text(updated)


def main() -> int:
    if len(sys.argv) != 3 or sys.argv[1] not in {"check", "set"}:
        print("usage: scripts/product_versions.py check|set VERSION", file=sys.stderr)
        return 2
    operation, expected = sys.argv[1:]
    if not SEMANTIC_VERSION_RE.fullmatch(expected):
        print(f"invalid semantic version: {expected}; next: supply an X.Y.Z version", file=sys.stderr)
        return 2
    validated = SemanticVersion(expected)

    try:
        if operation == "set":
            set_reconciled_versions(validated)
            return 0

        failed = False
        for path, actual in read_reconciled_versions().items():
            if actual != validated:
                print(
                    f"{path} has {actual}; expected {expected}; next: run "
                    f"scripts/set-version.sh {expected}",
                    file=sys.stderr,
                )
                failed = True
        return int(failed)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(
            f"product version files could not be processed: {error}; next: restore the "
            "named manifest and rerun scripts/set-version.sh",
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
