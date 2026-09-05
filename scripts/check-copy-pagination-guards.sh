#!/usr/bin/env bash
# Fail when a pagination loop on the copy path is missing the cursor-repeat guard or the
# page bound — or when either is spelled a second time instead of called.
#
# A source that answers a cursor with the cursor it was given is a loop that never ends,
# and from outside a command that will never finish looks exactly like one still working:
# the operator watching it cannot tell. The engine already decided everywhere else that
# this is a refusal rather than a wait, and it already decided that a page longer than the
# one asked for is refused rather than held. `engine/fetch.rs` is where both decisions are
# implemented, once.
#
# This holds the copy path to them. It enumerates no loop: every loop in the file that
# reads a page has to call both, so a loop added later is caught here rather than by an
# operator watching a copy spin.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

COPY_PATH="crates/onetaskgraph-core/src/engine/copy.rs" \
AUTHORITY="crates/onetaskgraph-core/src/engine/fetch.rs" \
python3 <<'PY'
import os
import pathlib
import re
import sys

copy_path = os.environ["COPY_PATH"]
authority_path = os.environ["AUTHORITY"]


def fail(problem, action):
    """A named problem and a concrete next action, which is all a guard owes its reader."""
    print(f"check-copy-pagination-guards: {problem}", file=sys.stderr)
    print(f"check-copy-pagination-guards: {action}", file=sys.stderr)
    sys.exit(1)


def read(path):
    try:
        return pathlib.Path(path).read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        fail(
            f"could not read {path}: {error}",
            "run this from a checkout of this repository, where that file is the engine "
            "source it names, then re-run 'just check'.",
        )


authority = read(authority_path)
# The two refusals live there once each. A copy path calling something else would satisfy
# every loop check below while refusing in its own words, which is the drift this exists
# to stop.
for wanted, what in (
    ("pub(crate) fn advances", "the cursor-repeat refusal"),
    ("pub(crate) fn fits", "the page bound"),
):
    if wanted not in authority:
        fail(
            f"{authority_path} defines no `{wanted}`, so {what} has no single "
            "implementation for the copy path to share",
            f"keep {what} there as `{wanted}`, or update this check to read it wherever "
            "it moved — and move every caller in the same change.",
        )

source = read(copy_path)

# Forward slashes on every platform: python renders a path with the running platform's
# separator, and a guard that names `crates\...` on one runner cannot be asserted against.
posix = pathlib.PurePath(copy_path).as_posix()

# A second spelling of either refusal is what makes two loops drift apart, so the words
# themselves may appear only where they are implemented.
for phrase, what in (
    ("cursor it was given", "the cursor-repeat refusal"),
    ("may return fewer than it was asked for", "the page bound"),
):
    for number, line in enumerate(source.splitlines(), start=1):
        if phrase in line:
            fail(
                f"{posix}:{number} spells {what} itself",
                f"call the one in {authority_path} instead — two spellings of one "
                "refusal are two things that drift apart.",
            )

# A loop's own text, with any loop nested inside it removed: each loop answers for
# itself, so an inner loop's guard may not stand in for the outer one's.
LOOP = re.compile(r"^[ \t]*(?:\}\s*)?(?:loop\s*|while\b[^{]*)\{[ \t]*$", re.MULTILINE)
# What makes a loop a pagination loop: it asks something for a page, or it advances a
# cursor or a token to ask again.
PAGING = re.compile(
    r"query_tasks\(|query_projects\(|query_documents\(|labels\(|task_dependencies\(|"
    r"project_dependencies\(|request_for\(|PageRequest|self\.tasks\(|self\.projects\(|"
    r"self\.documents\(|\.next\b"
)


def body_of(text, opened):
    """The text between the brace at `opened` and the one that closes it."""
    depth = 0
    for index in range(opened, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return text[opened + 1 : index]
    fail(
        f"{posix} has an unbalanced brace: a loop opened at offset {opened} never closes",
        "restore that file as compiling Rust source — this check cannot read a loop it "
        "cannot find the end of — then re-run 'just check'.",
    )
    return ""


loops = []
for found in LOOP.finditer(source):
    opened = source.index("{", found.end() - 1)
    line = source.count("\n", 0, found.start()) + 1
    loops.append((line, opened, body_of(source, opened)))

problems = []
for line, opened, body in loops:
    own = body
    for other_line, other_opened, other_body in loops:
        if other_opened > opened and other_body in own:
            own = own.replace(other_body, "")
    if PAGING.search(own) is None:
        continue
    missing = [
        what
        for call, what in (("advances(", "the cursor-repeat guard"), ("fits(", "the page bound"))
        if call not in own
    ]
    if missing:
        problems.append(f"{posix}:{line} paginates without {' and without '.join(missing)}")

if not loops:
    fail(
        f"{posix} contains no loop at all, so this check proved nothing",
        "point this check at the copy path wherever it moved to — a check that reads no "
        "loop passes whatever the copy path does.",
    )

if problems:
    for problem in problems:
        print(f"check-copy-pagination-guards: {problem}", file=sys.stderr)
    fail(
        "a pagination loop on the copy path would walk a misbehaving source for ever",
        "call `advances(page.next.as_ref(), asked.as_ref(), \"<what is being walked>\")` "
        "and `fits(page.items.len(), request.limit)` in each loop above, both from "
        f"{authority_path} — a source that repeats its cursor is refused there rather "
        "than walked, and a source that overruns its page is refused rather than held.",
    )
PY
