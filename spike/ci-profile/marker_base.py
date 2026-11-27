#!/usr/bin/env python3
"""Give Nx a comparison whose diff is the demonstration's marker comment alone.

    eval "$(python3 spike/ci-profile/marker_base.py)"

A demonstration commit carries both a prototype (a crate split, say) and the one-line marker
comment in `crates/onetaskgraph-core/src/lib.rs` that makes it select what an engine change
selects. Compared against the session head, the prototype's own files (manifests, a new crate)
widen the selection. This builds two local commits, never pushed: a base whose tree is HEAD's
with the marker removed, on HEAD, and a head with HEAD's own tree on that base. It prints the
`export NX_BASE=… NX_HEAD=…` that makes `nx affected` see the marker alone, while the checkout
Nx reads projects from is HEAD's, prototype included. Run identically on a control, the
comparison is the same, so both sides select the same tasks.

Exit status: 0 printed; 1 HEAD's `lib.rs` does not end with the marker, or git refused (named).
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile

LIB = "crates/onetaskgraph-core/src/lib.rs"
MARKER_TEXT = (
    "spike-onetaskgraph-profile: a no-behaviour edit, so this demonstration selects what an engine change selects."
)
MARKER = "\n// " + MARKER_TEXT + "\n"
IDENTITY = {
    "GIT_AUTHOR_NAME": "spike",
    "GIT_AUTHOR_EMAIL": "spike@localhost",
    "GIT_COMMITTER_NAME": "spike",
    "GIT_COMMITTER_EMAIL": "spike@localhost",
}


def git(*args: str, data: bytes | None = None, env: dict[str, str] | None = None) -> str:
    done = subprocess.run(
        ["git", *args], input=data, capture_output=True, env={**os.environ, **IDENTITY, **(env or {})}, check=False
    )
    if done.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)}: {done.stderr.decode(errors='replace').strip()}")
    return done.stdout.decode().strip()


def main() -> int:
    try:
        text = subprocess.run(["git", "show", f"HEAD:{LIB}"], capture_output=True, check=True).stdout
        if not text.endswith(MARKER.encode()):
            print(f"marker_base: HEAD's {LIB} does not end with the demonstration marker", file=sys.stderr)
            return 1
        blob = git("hash-object", "-w", "--stdin", data=text[: -len(MARKER.encode())])
        with tempfile.TemporaryDirectory() as scratch:
            index = {"GIT_INDEX_FILE": os.path.join(scratch, "index")}
            git("read-tree", "HEAD", env=index)
            git("update-index", "--cacheinfo", f"100644,{blob},{LIB}", env=index)
            tree = git("write-tree", env=index)
        base = git("commit-tree", tree, "-p", "HEAD", "-m", "spike: the demonstration without its marker")
        head = git("commit-tree", "HEAD^{tree}", "-p", base, "-m", "spike: the demonstration's marker")
    except (subprocess.CalledProcessError, RuntimeError) as error:
        print(f"marker_base: {error}", file=sys.stderr)
        return 1
    # Bytes, so Windows' text-mode stdout adds no carriage return to what the shell evaluates.
    sys.stdout.buffer.write(f"export NX_BASE={base} NX_HEAD={head}\n".encode())
    return 0


if __name__ == "__main__":
    sys.exit(main())
