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
    patterns: Tuple[re.Pattern, ...]


@dataclass(frozen=True)
class JsonVersionFile:
    path: Path
    fields: Tuple[Tuple[str, ...], ...]


VersionFile = Union[RegexVersionFile, JsonVersionFile]
RECONCILED_VERSION_FILES: Tuple[VersionFile, ...] = (
    RegexVersionFile(
        Path("Cargo.toml"),
        (
            re.compile(r'(?m)^(\[workspace\.package\]\nversion\s*=\s*")([^"]+)(")'),
            re.compile(
                r'(?m)^(onetaskgraph[^= ]*\s*=\s*\{[^\n]*version\s*=\s*")([^"]+)(")'
            ),
        ),
    ),
    *(
        RegexVersionFile(
            Path(f"crates/{crate}/Cargo.toml"),
            (re.compile(r'(?m)^(version\s*=\s*")([^"]+)(")'),),
        )
        for crate in (
            "onetaskgraph",
            "onetaskgraph-core",
            "onetaskgraph-github-projects",
            "onetaskgraph-in-memory",
            "onetaskgraph-linear",
            "onetaskgraph-local-md",
            "onetaskgraph-plugin-api",
        )
    ),
    RegexVersionFile(
        Path("pyproject.toml"),
        (re.compile(r'(?m)^(version\s*=\s*")([^"]+)(")'),),
    ),
    RegexVersionFile(
        Path("sdks/python/pyproject.toml"),
        (
            re.compile(r'(?m)^(version\s*=\s*")([^"]+)(")'),
            re.compile(
                r'(?m)^(dependencies\s*=\s*\[[^\n]*onetaskgraph-cli==)([^" ]+)(")'
            ),
        ),
    ),
    RegexVersionFile(
        Path("sdks/python/src/onetaskgraph_sdk/__init__.py"),
        (re.compile(r'(?m)^(__version__\s*=\s*")([^"]+)(")'),),
    ),
    RegexVersionFile(
        Path("sdks/typescript/src/index.ts"),
        (re.compile(r'(?m)^(export const VERSION\s*=\s*")([^"]+)(";)'),),
    ),
    RegexVersionFile(
        Path("bun.lock"),
        (
            re.compile(r'(?m)^(\s+"version": ")([^"]+)(",)$'),
            re.compile(r'(?m)^(\s+"@onetaskgraph/cli": ")([^"]+)(",)$'),
        ),
    ),
    JsonVersionFile(
        Path("sdks/typescript/package.json"),
        (("version",), ("optionalDependencies", "@onetaskgraph/cli")),
    ),
    JsonVersionFile(
        Path("npm/cli/package.json"),
        (
            ("version",),
            *(
                ("optionalDependencies", name)
                for name in (
                    "@onetaskgraph/cli-linux-x64",
                    "@onetaskgraph/cli-linux-arm64",
                    "@onetaskgraph/cli-darwin-x64",
                    "@onetaskgraph/cli-darwin-arm64",
                    "@onetaskgraph/cli-win32-x64",
                )
            ),
        ),
    ),
    *(
        JsonVersionFile(Path(f"npm/platforms/{platform}/package.json"), (("version",),))
        for platform in (
            "darwin-arm64",
            "darwin-x64",
            "linux-arm64",
            "linux-x64",
            "win32-x64",
        )
    ),
)


# llmlint: ignore[async_typed_clients_at_boundaries] This local manifest read is part of the short-lived synchronous release transaction and has no concurrent work.
def read_json_manifest(path: Path) -> Dict[str, object]:
    """Decode a package manifest only after validating its object boundary."""
    decoded = json.loads(path.read_text())
    if not isinstance(decoded, dict) or not all(
        isinstance(key, str) for key in decoded
    ):
        raise ValueError(f"{path}: package manifest must be a JSON object")
    return decoded


# llmlint: ignore[async_typed_clients_at_boundaries] This preflight is part of the ordered synchronous release transaction and must finish before mutation begins.
def require_writable_manifests() -> None:
    """Refuse the update before mutation when any reconciled file is not writable."""
    for version_file in RECONCILED_VERSION_FILES:
        with version_file.path.open("r+"):
            pass


def _json_field(package: Dict[str, object], field: Tuple[str, ...]) -> object:
    value: object = package
    for part in field:
        if not isinstance(value, dict):
            return None
        value = value.get(part)
    return value


# llmlint: ignore[async_typed_clients_at_boundaries] This short-lived gate inventory performs an ordered local tree scan and has no concurrent work.
def discover_product_version_files() -> Tuple[Path, ...]:
    """Discover release-owned manifests and public product-version constants."""
    discovered = []
    for path in Path(".").glob("**/Cargo.toml"):
        if path == Path("Cargo.toml") or path.parent.parent == Path("crates"):
            discovered.append(path)
    for path in Path(".").glob("**/pyproject.toml"):
        text = path.read_text()
        if re.search(r'(?m)^name\s*=\s*"onetaskgraph(?:-sdk|-cli)?"$', text):
            discovered.append(path)
    for path in Path(".").glob("**/package.json"):
        if "node_modules" in path.parts:
            continue
        package = read_json_manifest(path)
        name = package.get("name")
        if isinstance(name, str) and name.startswith("@onetaskgraph/"):
            discovered.append(path)
    for path in Path(".").glob("**/bun.lock"):
        text = path.read_text()
        if '"name": "@onetaskgraph/' in text:
            discovered.append(path)
    for path in Path(".").glob("**/*"):
        if (
            not path.is_file()
            or path.suffix not in {".py", ".ts"}
            or "node_modules" in path.parts
            or "generated" in path.parts
            or any(part.startswith(".") for part in path.parts)
        ):
            continue
        text = path.read_text()
        if re.search(r"(?m)^(?:__version__|export const VERSION)\s*=", text):
            discovered.append(path)
    return tuple(sorted(set(discovered)))


def unregistered_product_version_files() -> Tuple[Path, ...]:
    """Return discovered product-version files absent from the release inventory."""
    registered = {version_file.path for version_file in RECONCILED_VERSION_FILES}
    return tuple(
        path for path in discover_product_version_files() if path not in registered
    )


# llmlint: ignore[async_typed_clients_at_boundaries, structural_pattern_matching] This local manifest read is part of the ordered release transaction, and Python 3.8 support requires explicit variant checks.
def _read_version_values(version_file: VersionFile) -> Tuple[object, ...]:
    path = version_file.path
    if isinstance(version_file, JsonVersionFile):
        package = read_json_manifest(path)
        return tuple(_json_field(package, field) for field in version_file.fields)
    text = path.read_text()
    return tuple(
        match.group(2)
        for pattern in version_file.patterns
        for match in pattern.finditer(text)
    )


# llmlint: ignore[async_typed_clients_at_boundaries, structural_pattern_matching] This short-lived release command has no concurrent work, and Python 3.8 support precludes match/case syntax.
def read_reconciled_versions() -> Dict[str, Optional[SemanticVersion]]:
    """Return the cross-ecosystem versions checked for mutual agreement."""
    versions = {}
    for version_file in RECONCILED_VERSION_FILES:
        path = version_file.path
        values = _read_version_values(version_file)
        value = values[0] if values and len(set(values)) == 1 else None
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
    invalid = [
        version_file.path.as_posix()
        for version_file in RECONCILED_VERSION_FILES
        if not (values := _read_version_values(version_file))
        or any(
            not isinstance(value, str) or not SEMANTIC_VERSION_RE.fullmatch(value)
            for value in values
        )
    ]
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
            for field in version_file.fields:
                target = package
                for part in field[:-1]:
                    nested = target.get(part)
                    if not isinstance(nested, dict):
                        raise ValueError(
                            f"{path}: missing JSON product-version field {'.'.join(field)}"
                        )
                    target = nested
                target[field[-1]] = version
            path.write_text(json.dumps(package, indent=2) + "\n")
            continue
        text = path.read_text()
        updated = text
        for pattern in version_file.patterns:
            updated, count = pattern.subn(rf"\g<1>{version}\g<3>", updated)
            if count == 0:
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
        print(
            f"invalid semantic version: {expected}; next: supply an X.Y.Z version",
            file=sys.stderr,
        )
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
