"""A superseded sibling of every build pool-maintenance/measure.sh lists, and nothing else.

supersede.py LISTING
    Each listed entry named <unit>-<hash> is copied, bytes, modes and links as they are, to
    <unit>-<new hash>: to cargo-prune-superseded, another build of the same unit. Its own
    script so check-measure.sh can hold these copies to a real superseded build.
"""

import hashlib
import os
import re
import shutil
import sys

NAMED = re.compile(
    r"^(?P<unit>.+-)(?P<hash>[0-9a-f]{16}|[0-9a-z]{13})(?P<rest>(\..*)?)$"
)
BASE36 = "0123456789abcdefghijklmnopqrstuvwxyz"


def sibling(old: str) -> str:
    digest = hashlib.sha256(f"superseded:{old}".encode()).digest()
    if len(old) == 16 and all(c in "0123456789abcdef" for c in old):
        return digest.hex()[:16]
    return "".join(BASE36[b % 36] for b in digest[: len(old)])


with open(sys.argv[1], encoding="utf-8") as listing:
    entries = [line.rstrip("\n") for line in listing if line.strip()]
for entry in entries:
    directory, name = os.path.split(entry)
    named = NAMED.match(name)
    if not named:
        continue
    copy = os.path.join(
        directory, named["unit"] + sibling(named["hash"]) + named["rest"]
    )
    if os.path.lexists(copy):
        sys.exit(
            f"pool-maintenance/measure.sh: the superseded copy {copy} of {entry} already exists, so this directory already holds a sibling it cannot tell from a real build; remove {copy}, then re-run."
        )
    if os.path.isdir(entry) and not os.path.islink(entry):
        shutil.copytree(entry, copy, symlinks=True)
    else:
        shutil.copy2(entry, copy, follow_symlinks=False)
