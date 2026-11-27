"""The ordinary build's own entries, for pool-maintenance/measure.sh and nothing else.

ordinary_builds.py hashes < cargo-json > hashes.txt
    The metadata hash of every unit `cargo ... --message-format=json` reports using,
    fresh or rebuilt: the 16-hex `-<hash>` in an artifact's name or a build script's
    output directory.
ordinary_builds.py entries HASHES < listing
    Each listed `target/<profile>/<dir>/<name>` entry whose name carries one of those hashes.
ordinary_builds.py copy HASHES < listing
    A sibling of each such entry under a stand-in hash derived from its own, byte for
    byte a copy: to cargo-prune-superseded, which reads only names and modification
    times, another build of the same unit.
"""

from __future__ import annotations

import hashlib
import json
import re
import shutil
import sys
from pathlib import Path, PurePosixPath
from typing import NamedTuple, NewType

BuildHash = NewType("BuildHash", str)

HASH = re.compile(r"-([0-9a-f]{16})(?:\.|$)")
WHOLE_HASH = re.compile(r"[0-9a-f]{16}")


class Entry(NamedTuple):
    """One listed build entry of the ordinary build, and where its hash sits in its name."""

    folder: PurePosixPath
    name: str
    start: int
    end: int

    @property
    def path(self) -> Path:
        return Path(self.folder / self.name)

    def stand_in(self) -> Path:
        hashed = self.name[self.start : self.end]
        sibling = hashlib.sha256(hashed.encode()).hexdigest()[:16]
        return Path(
            self.folder / (self.name[: self.start] + sibling + self.name[self.end :])
        )


def reported_paths(message: object) -> list[str]:
    """The artifact paths one cargo message names, refusing a field of the wrong shape."""
    if not isinstance(message, dict):
        return []
    filenames = message.get("filenames", [])
    if not isinstance(filenames, list) or not all(
        isinstance(f, str) for f in filenames
    ):
        raise SystemExit(
            f"ordinary_builds.py: cargo reported filenames that are not strings: {filenames!r}"
        )
    paths = list(filenames)
    for key in ("executable", "out_dir"):
        value = message.get(key)
        if value is not None and not isinstance(value, str):
            raise SystemExit(
                f"ordinary_builds.py: cargo reported a {key} that is not a string: {value!r}"
            )
        if value:
            paths.append(value)
    return paths


def hashes() -> None:
    found: set[BuildHash] = set()
    for line in sys.stdin:
        try:
            message = json.loads(line)
        except ValueError:
            continue
        for path in reported_paths(message):
            for part in re.split(r"[/\\]", path):
                if match := HASH.search(part):
                    found.add(BuildHash(match.group(1)))
    sys.stdout.write("".join(f"{value}\n" for value in sorted(found)))


def known_hashes(listing: str) -> set[BuildHash]:
    words = Path(listing).read_text(encoding="utf-8").split()
    if bad := [word for word in words if not WHOLE_HASH.fullmatch(word)]:
        raise SystemExit(
            f"ordinary_builds.py: {listing} holds lines that are not build hashes: {bad[:3]!r}"
        )
    return {BuildHash(word) for word in words}


def ordinary(listing: str) -> list[Entry]:
    """The listed entries of the ordinary build; each must be `target/<profile>/<dir>/<name>`."""
    known = known_hashes(listing)
    chosen = []
    for line in sys.stdin:
        path = PurePosixPath(line.rstrip("\n"))
        if (
            path.is_absolute()
            or path.parts[:1] != ("target",)
            or ".." in path.parts
            or len(path.parts) < 4
        ):
            raise SystemExit(
                f"ordinary_builds.py: a listed entry is not inside target/: {str(path)!r}"
            )
        match = HASH.search(path.name)
        if match and BuildHash(match.group(1)) in known:
            chosen.append(Entry(path.parent, path.name, match.start(1), match.end(1)))
    return chosen


def copy(listing: str) -> None:
    for entry in ordinary(listing):
        source, target = entry.path, entry.stand_in()
        if target.exists() or target.is_symlink():
            continue
        if source.is_dir() and not source.is_symlink():
            shutil.copytree(source, target, symlinks=True)
        else:
            shutil.copy2(source, target, follow_symlinks=False)


def main() -> None:
    match sys.argv[1:]:
        case ["hashes"]:
            hashes()
        case ["entries", listing]:
            sys.stdout.write(
                "".join(f"{entry.path.as_posix()}\n" for entry in ordinary(listing))
            )
        case ["copy", listing]:
            copy(listing)
        case _:
            raise SystemExit(
                "usage: ordinary_builds.py hashes | entries HASHES | copy HASHES"
            )


if __name__ == "__main__":
    main()
