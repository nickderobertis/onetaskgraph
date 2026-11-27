"""The ordinary build's own entries, for pool-maintenance/measure.sh and nothing else.

ordinary_builds.py hashes < cargo-json > hashes.txt
    The metadata hash of every unit `cargo ... --message-format=json` reports using,
    fresh or rebuilt: the 16-hex `-<hash>` in an artifact's name or a build script's
    output directory.
ordinary_builds.py entries HASHES < listing
    Each listed `<profile>/<dir>/<name>` entry whose name carries one of those hashes.
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
from pathlib import Path

HASH = re.compile(r"-([0-9a-f]{16})(?:\.|$)")


def hashes() -> None:
    found: set[str] = set()
    for line in sys.stdin:
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if not isinstance(message, dict):
            continue
        paths = list(message.get("filenames") or [])
        paths += [message[key] for key in ("executable", "out_dir") if message.get(key)]
        for path in paths:
            for part in re.split(r"[/\\]", path):
                if match := HASH.search(part):
                    found.add(match.group(1))
    sys.stdout.write("".join(f"{value}\n" for value in sorted(found)))


def ordinary(listing: str) -> list[tuple[str, re.Match[str]]]:
    known = set(Path(listing).read_text(encoding="utf-8").split())
    chosen = []
    for line in sys.stdin:
        entry = line.rstrip("\n")
        if (match := HASH.search(entry.rsplit("/", 1)[-1])) and match.group(1) in known:
            chosen.append((entry, match))
    return chosen


def copy(listing: str) -> None:
    for entry, match in ordinary(listing):
        folder, name = entry.rsplit("/", 1)
        stand_in = hashlib.sha256(match.group(1).encode()).hexdigest()[:16]
        target = Path(folder) / (
            name[: match.start(1)] + stand_in + name[match.end(1) :]
        )
        source = Path(entry)
        if target.exists() or target.is_symlink():
            continue
        if source.is_dir() and not source.is_symlink():
            shutil.copytree(source, target, symlinks=True)
        else:
            shutil.copy2(source, target, follow_symlinks=False)


def main() -> None:
    arguments = sys.argv[1:]
    if arguments == ["hashes"]:
        hashes()
    elif len(arguments) == 2 and arguments[0] == "entries":
        sys.stdout.write("".join(f"{entry}\n" for entry, _ in ordinary(arguments[1])))
    elif len(arguments) == 2 and arguments[0] == "copy":
        copy(arguments[1])
    else:
        raise SystemExit(
            "usage: ordinary_builds.py hashes | entries HASHES | copy HASHES"
        )


if __name__ == "__main__":
    main()
