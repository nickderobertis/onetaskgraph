#!/usr/bin/env bash
# Reconcile the one-name-per-credential contract against every place that restates it.
#
# There is exactly one name for each of the two hosted credentials — `LINEAR_API_KEY` and
# `GH_PROJECTS_TOKEN` — in the repository secrets, in the local secrets file, in a
# configuration document's `*_env` field, in the documentation and in CI. Nothing anywhere
# translates between spellings, which is the whole reason the rule is worth having: the
# moment a second spelling exists, something has to map between them, and the mapping is
# where a live journey silently reads an empty credential.
#
# That contract is prose in several documents and a literal in two other scripts, so it is
# the kind of thing that drifts one copy at a time. This is the one declaration; every
# restatement below is checked against it. Adding a place that names a credential means
# adding it here, which is the point.
#
# Comments are stripped before a file is scanned, because several of these files name a
# wrong spelling deliberately in order to explain why the right one is right — and a check
# that could not tell an instruction from an explanation would make the explanation
# impossible to write down.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# The gate runs from a git hook and git exports GIT_DIR to hooks, where it overrides the
# repository a plain `git` command would otherwise pick from the working directory. The
# scan below lists tracked files, so left set it would list some other tree's.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

python3 <<'PY'
import re
import subprocess
import sys
from pathlib import Path

# The contract. Each credential's one name, and the spellings that are not it: a second
# spelling is not a typo to be tolerated, it is the failure this check exists to catch.
CREDENTIALS = {
    "LINEAR_API_KEY": ("LINEAR_KEY", "LINEAR_TOKEN", "LINEAR_API_TOKEN"),
    "GH_PROJECTS_TOKEN": (
        "GITHUB_PROJECTS_TOKEN",
        "GH_PROJECTS_API_KEY",
        "GH_PROJECT_TOKEN",
        "GITHUB_PROJECT_TOKEN",
    ),
}

# Every file that has to *state* the contract, and what it is: each names both credentials.
# The other half of the check — that no second spelling exists — is not a list at all. It
# runs over every tracked file, because "there is one name per credential everywhere" is a
# claim about the repository rather than about five chosen files, and a hand-picked subset
# would leave the next file to name one unwatched.
RESTATEMENTS = {
    "README.md": "the credentials section a user reads",
    "docs/plugin-protocol.md": "the protocol document an out-of-tree plugin is written from",
    "gh-secrets.json": "the declaration of the repository secrets this build needs",
    ".github/workflows/live.yml": "the workflow that hands each live job its credential",
    "scripts/check-live-lane.sh": "the live lane's own job-to-credential map",
}

# A file whose own job is to forbid a spelling has to be able to write it down: these are
# left out of the second-spelling scan, and out of it alone. This script is one of them —
# the wrong spellings are its own data.
GUARDS_AGAINST_WRONG_SPELLINGS = {
    "scripts/check-live-lane.sh",
    "scripts/check-credential-names.sh",
}

# `#` starts a comment in YAML, JSON-with-comments this repository does not use, shell and
# Markdown-embedded shell alike; Markdown's own `#` is a heading, which is prose either way.
COMMENT = re.compile(r"(?<!\S)#.*$", re.MULTILINE)


def refuse(problem, next_action):
    """Stop with the exact problem and one concrete thing to do about it."""
    print(f"check-credential-names: {problem}", file=sys.stderr)
    print(f"check-credential-names: {next_action}", file=sys.stderr)
    sys.exit(1)


def readable(path):
    """One tracked file as text, or nothing when it is not text at all."""
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None


problems = []

for relative, what in RESTATEMENTS.items():
    path = Path(relative)
    if not path.exists():
        problems.append(
            f"{relative}: is missing, so {what} no longer states the credential contract"
        )
        continue
    # A file that is there and will not open is not a contract failure — it is this check
    # being unable to make its judgement, which is worth stopping for and saying so.
    text = readable(path)
    if text is None:
        refuse(
            f"could not read {relative}, which is {what}, so the credential contract "
            "cannot be reconciled against it.",
            f"make {relative} readable text again, or, if it moved, name its new path in "
            "the RESTATEMENTS list in this script.",
        )
    code = COMMENT.sub("", text)
    for name in CREDENTIALS:
        if name not in code:
            problems.append(
                f"{relative}: never names {name}, but it is {what}. There is one name per "
                "credential everywhere and nothing translates between spellings."
            )

listing = subprocess.run(["git", "ls-files", "-z"], capture_output=True, text=True, check=False)
if listing.returncode != 0:
    refuse(
        "could not list the tracked files: "
        f"{listing.stderr.strip() or f'git ls-files exited {listing.returncode}'}.",
        "run this from inside the repository's working tree — the scan for a second "
        "spelling covers every tracked file, so it cannot run without that list.",
    )
tracked = [relative for relative in listing.stdout.split("\0") if relative]
if not tracked:
    refuse(
        "git listed no tracked files, so the scan for a second spelling would pass on "
        "anything.",
        "run this from inside the repository's working tree rather than a copy of it.",
    )

for relative in tracked:
    if relative in GUARDS_AGAINST_WRONG_SPELLINGS:
        continue
    text = readable(Path(relative))
    if text is None:
        continue
    code = COMMENT.sub("", text)
    for name, wrong_spellings in CREDENTIALS.items():
        for wrong in wrong_spellings:
            if wrong in code:
                problems.append(
                    f"{relative}: spells the credential {wrong}. Its one name is {name}; "
                    "a second spelling means something has to map between them, and that "
                    "mapping is where a live journey reads an empty credential."
                )

if problems:
    print("check-credential-names: the credential-name contract has drifted.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        "check-credential-names: restore the one name each credential has — LINEAR_API_KEY "
        "and GH_PROJECTS_TOKEN — in the file named above, or, if a place that states the "
        "contract has genuinely moved, update the list in this script so the new place is "
        "the one being checked.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
