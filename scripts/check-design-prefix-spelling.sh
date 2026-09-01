#!/usr/bin/env bash
# Fail when anything spells the GitHub Projects design-title prefix differently from
# `DESIGN_TITLE_PREFIX`, the plugin constant that defines it.
#
# Prose has to show a reader the exact bytes they will type, so the documents below restate
# the prefix. What a restatement without a gate costs is silence: move the constant and they
# go on telling people to type the old prefix, while the suite still passes because it reads
# the constant.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

AUTHORITY="$ROOT/crates/onetaskgraph-github-projects/src/lib.rs" \
python3 <<'PY'
import os
import pathlib
import re
import sys

NAME = "check-design-prefix-spelling"
authority_path = os.environ["AUTHORITY"]

# The documents that show a reader the prefix they will type. Each one is a restatement of
# the constant, and each one is reconciled with it below.
DOCUMENTS = ["README.md", "docs/metadata.md", "docs/follow-ups.md", "AGENTS.md"]


def fail(problem, action):
    """A named problem and a concrete next action, which is all a guard owes its reader."""
    print(f"{NAME}: {problem}", file=sys.stderr)
    print(f"{NAME}: {action}", file=sys.stderr)
    sys.exit(1)


def read(path, what):
    try:
        return open(path, encoding="utf-8").read()
    except (OSError, UnicodeDecodeError) as error:
        fail(
            f"could not read {what}: {error}",
            f"restore {path} — it is one of the files this check reconciles — then re-run "
            "'just check'.",
        )


authority_source = read(authority_path, "the plugin's own spelling")
found = re.search(
    r'DESIGN_TITLE_PREFIX:\s*&str\s*=\s*"([^"]+)"',
    authority_source,
)
if found is None:
    fail(
        f"{authority_path} defines no `DESIGN_TITLE_PREFIX` literal",
        'spell it as `pub const DESIGN_TITLE_PREFIX: &str = "...";` there, or update this '
        "check to read it wherever it moved.",
    )
authority = found.group(1)

problems = []
restatements = 0

for relative in DOCUMENTS:
    text = read(relative, f"the document {relative}")
    for number, line in enumerate(text.splitlines(), start=1):
        # Only a code span: prose about "the design prefix" names no bytes, and the bytes
        # are what drift. A span naming the prefix is one whose content starts `DESIGN`,
        # which is what makes this catch a stale spelling rather than every backtick.
        for span in re.findall(r"`([^`]*)`", line):
            if not span.startswith("DESIGN"):
                continue
            restatements += 1
            if span != authority:
                problems.append(
                    f"{relative}:{number} spells {span!r}, and the plugin's "
                    f"`DESIGN_TITLE_PREFIX` is {authority!r}"
                )

# Forward slashes on every platform: python renders a path with the running platform's
# separator, and a guard that names `crates\...` on one runner cannot be asserted against.
try:
    sources = sorted(pathlib.Path("crates").glob("*/**/*.rs"))
except OSError as error:
    fail(
        f"could not list the Rust sources this check reads: {error}",
        "run it from a checkout of this repository, where crates/ is the workspace's own "
        "directory of crates, then re-run 'just check'.",
    )
for source in sources:
    if source.resolve() == pathlib.Path(authority_path).resolve():
        continue
    text = read(source.as_posix(), f"the Rust source {source.as_posix()}")
    for number, line in enumerate(text.splitlines(), start=1):
        # `DESIGN:` with its colon, so an identifier that merely begins with the word — a
        # fixture id, a test name — is not mistaken for a spelling of the prefix.
        for literal in re.findall(r'"(DESIGN:[^"]*)"', line):
            restatements += 1
            if literal != authority:
                problems.append(
                    f"{source.as_posix()}:{number} spells {literal!r}, and the plugin's "
                    f"`DESIGN_TITLE_PREFIX` is {authority!r}"
                )
            else:
                problems.append(
                    f"{source.as_posix()}:{number} restates the prefix as a Rust literal; "
                    "read `onetaskgraph_github_projects::DESIGN_TITLE_PREFIX` instead"
                )

if restatements == 0:
    fail(
        "no document names the design-title prefix at all, so this check reconciles "
        "nothing",
        "either the prefix is no longer a contract a person types — in which case delete "
        f"this check with it — or {', '.join(DOCUMENTS)} lost the paragraph that shows a "
        "reader the bytes; restore it.",
    )

if problems:
    for problem in problems:
        print(f"{NAME}: {problem}", file=sys.stderr)
    fail(
        "the design-title prefix is spelled two ways",
        "bring each spelling above to the plugin's, or move the constant and every "
        "spelling of it in the same change — a document telling a person to type the old "
        "prefix fails nothing else at all.",
    )
PY
