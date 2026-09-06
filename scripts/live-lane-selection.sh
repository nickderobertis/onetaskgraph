#!/usr/bin/env bash
# The one answer to: can this diff reach one live plugin's behaviour?
#
# The tests that reach a real API are ordinary tests of an ordinary `test` target, so what
# decides whether a session is opened is which projects the diff selects. That is nearly
# enough. The hole it leaves is a release: release-plz bumps every crate's `version`, the
# changelogs and the lockfiles, which touches this plugin's own directory AND `Cargo.lock`
# — a `sharedGlobals` input, so affected selection marks every project in the workspace.
# A version bump exercises no plugin behaviour, and one live GitHub Projects session costs
# about a fifth of the account's hourly GraphQL allowance
# (crates/onetaskgraph-github-projects/session-cost.md), so a release must not draw it.
# It already did: the default branch has been red since the last release with the budget
# exhausted outright, which refuses ordinary reads against this repository too.
#
# So this is the second half of the selection, and it is ONE implementation because every
# caller has to give the same answer: `just test` consults it, and the change-request, the
# default-branch and the local pre-push paths all reach the live lane through that recipe.
# scripts/check-live-lane-selection.sh drives all of that.
#
# ## The contract
#
# It is asked about one live plugin crate and one base ref, and it answers `run` or
# `not-selected`.
#
# Its only `not-selected` is this: inside that crate's own directory the diff changes
# nothing but the `version` field of that crate's `Cargo.toml` and that crate's
# `CHANGELOG.md`; and outside that directory it changes nothing but version lines — a line
# that is the same line with the old version substituted for the new — of the workspace's
# manifests and lockfiles, plus changelogs. Any other change, INCLUDING any other edit to
# that same manifest, is `run`.
#
# "Manifest" is read as every version-bearing file `scripts/set-version.sh` writes rather
# than as `Cargo.toml` alone, because a release moves all of them together: the last one
# also rewrote `bun.lock`, `uv.lock`, `pyproject.toml`, the six `package.json` files and
# the two SDK version constants. Reading it narrowly would answer `run` for the very
# commit this exists for. What is NOT widened is the permission itself: a line qualifies
# only when it is byte-for-byte the old line with the version transition applied, so a
# dependency added beside a version bump, or a feature toggled in the same file, is `run`.
#
# Any question it cannot answer is `run`. It fails toward spending the budget, never
# toward skipping the lane — an unresolvable base, an unreadable blob, a diff it cannot
# explain and a crate it has never heard of all answer `run`, with the reason on stderr.
#
# ## Usage
#
#   scripts/live-lane-selection.sh <crate> [base]   -> prints `run` or `not-selected`
#   scripts/live-lane-selection.sh --nx-exclusions [base]
#                                                   -> prints Nx's `--exclude=<names>` for
#                                                      every live crate this diff cannot
#                                                      reach, and nothing when there are
#                                                      none, so a recipe can splice it in
#
# `base` defaults to $NX_BASE, and then to nx.json's own `defaultBase`, so the recipes and
# Nx compare against the same commit without either restating the other's default.
#
# A caller reads the answer rather than the exit status: only the exact word
# `not-selected` on stdout means do not run, so a crash, an empty answer or anything else
# leaves the lane running.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly MODE="${1:?usage: scripts/live-lane-selection.sh <crate>|--nx-exclusions [base]}"

# The crate argument names a crate of this workspace, not a path the caller chooses. Held
# to Cargo's own package-name grammar first, exactly as scripts/check-live-decline.sh holds
# its own, so nothing that is not a package name can be spliced into a path below.
case "$MODE" in
  --nx-exclusions) ;;
  *[!a-z0-9_-]* | "" | -*)
    echo "live-lane-selection: $MODE is neither --nx-exclusions nor a cargo package name (lowercase, digits, hyphens and underscores)." >&2
    echo "live-lane-selection: pass the name of a crate of this workspace that has a live session, or --nx-exclusions." >&2
    exit 2
    ;;
esac

BASE="${2:-${NX_BASE:-}}"
export ONETASKGRAPH_LIVE_LANE_MODE="$MODE"
export ONETASKGRAPH_LIVE_LANE_BASE="$BASE"

python3 - <<'PY'
import json
import os
import re
import subprocess
import sys
from difflib import SequenceMatcher
from pathlib import Path

MODE = os.environ["ONETASKGRAPH_LIVE_LANE_MODE"]
BASE = os.environ["ONETASKGRAPH_LIVE_LANE_BASE"]

RUN = "run"
NOT_SELECTED = "not-selected"

# A version this repository could be at. `scripts/set-version.sh` holds every manifest to
# exactly this grammar, so a transition it did not write is not one this decision explains.
SEMVER = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$"
)


def say(line):
    """One line of the reason, on stderr, where the answer on stdout is not the place."""
    print(f"live-lane-selection: {line}", file=sys.stderr)


def git(*arguments):
    """A git command's stdout, or `None` when it could not be run or refused."""
    try:
        finished = subprocess.run(
            ["git", *arguments],
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return None
    if finished.returncode != 0:
        return None
    return finished.stdout


def default_base():
    """Nx's own `defaultBase`, so the recipes and Nx compare against the same commit."""
    try:
        configuration = json.loads(Path("nx.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    base = configuration.get("defaultBase")
    return base if isinstance(base, str) and base else None


def live_crates():
    """Every crate whose project.json says it has a live session, by its `live:` tag.

    The tag rather than a list written here: scripts/check-live-lane.sh already reconciles
    those tags against the journeys both ways, so a plugin that gains a session is covered
    without an edit in this file.
    """
    names = []
    for path in sorted(Path("crates").glob("*/project.json")):
        try:
            project = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as problem:
            say(f"could not read {path.as_posix()}: {problem}.")
            return None
        tags = project.get("tags", [])
        if not isinstance(tags, list):
            say(f"{path.as_posix()} has a `tags` that is not a list.")
            return None
        if any(isinstance(tag, str) and tag.startswith("live:") for tag in tags):
            name = project.get("name")
            if not isinstance(name, str) or not name:
                say(f"{path.as_posix()} is tagged `live:` but declares no name.")
                return None
            names.append(name)
    return sorted(names)


def section_of(lines):
    """The TOML section each line sits directly inside, as a list parallel to `lines`.

    `[[package]]` comes back as `package`, which is what makes a lockfile entry's version
    readable by the same rule as a manifest's. What this is for is telling
    `[package] version` — the one field a release bumps — apart from a `version` under
    `[dependencies.something]`, which is a dependency change wearing the same spelling.
    """
    sections = []
    current = ""
    for line in lines:
        stripped = line.strip()
        header = re.fullmatch(r"\[\[?([^\[\]]+)\]\]?", stripped)
        if header:
            current = header.group(1).strip()
        sections.append(current)
    return sections


def changed_pairs(old_text, new_text):
    """The (old line, new line) pairs this change consists of, or `None`.

    `None` when the change is anything but a replacement of the same number of lines: an
    inserted or deleted line is a line a version bump did not write, whatever else the file
    contains, and there is nothing to substitute into it.
    """
    old_lines = old_text.splitlines()
    new_lines = new_text.splitlines()
    pairs = []
    for tag, i1, i2, j1, j2 in SequenceMatcher(
        a=old_lines, b=new_lines, autojunk=False
    ).get_opcodes():
        if tag == "equal":
            continue
        if tag != "replace" or (i2 - i1) != (j2 - j1):
            return None
        for offset in range(i2 - i1):
            pairs.append((i1 + offset, old_lines[i1 + offset], new_lines[j1 + offset]))
    return pairs


def package_version_line(line, section):
    """Whether this line is the `version` field of a `[package]`-shaped section."""
    return section in ("package", "workspace.package") and re.match(
        r"^\s*version\s*=", line
    )


def version_value(text, wanted_sections=("package", "workspace.package")):
    """The first `version` string declared directly in one of `wanted_sections`."""
    lines = text.splitlines()
    for line, section in zip(lines, section_of(lines)):
        if section not in wanted_sections:
            continue
        found = re.match(r'^\s*version\s*=\s*"([^"]*)"', line)
        if found:
            return found.group(1)
    return None


class Unanswerable(Exception):
    """A question this decision cannot answer, which is therefore answered `run`."""


def base_commit():
    """The commit `BASE` names, or `Unanswerable`."""
    base = BASE or default_base()
    if not base:
        raise Unanswerable(
            "no base ref was given and nx.json declares no defaultBase, so there is "
            "nothing to compare this tree against"
        )
    resolved = git("rev-parse", "--verify", "--quiet", f"{base}^{{commit}}")
    if not resolved or not resolved.strip():
        raise Unanswerable(
            f"{base} does not resolve to a commit in this repository, so what this diff "
            "changes cannot be read"
        )
    return base, resolved.strip()


def changed_files(base):
    """`(status, path)` for every file this tree differs from `base` in, or `Unanswerable`.

    The working tree rather than HEAD, which is what `nx affected` compares by default, so
    a contributor running the gate over uncommitted work gets the same answer as CI gets
    over the commit it checked out.
    """
    listing = git("diff", "--name-status", "-z", base, "--")
    if listing is None:
        raise Unanswerable(
            f"git could not diff this tree against {base}, so what changed is unknown"
        )
    fields = [field for field in listing.split("\0") if field]
    changes = []
    index = 0
    while index < len(fields):
        status = fields[index]
        # A rename or a copy is reported with two paths. Neither is a version bump, so the
        # decision below refuses it on its status; both paths are consumed so the walk
        # stays aligned with the record boundaries.
        if status[:1] in ("R", "C"):
            changes.append((status[:1], fields[index + 2] if index + 2 < len(fields) else ""))
            index += 3
            continue
        if index + 1 >= len(fields):
            raise Unanswerable(
                "git's name-status output ended in the middle of a record, so what "
                "changed cannot be read"
            )
        changes.append((status[:1], fields[index + 1]))
        index += 2
    return changes


def blob(base, path):
    """`path` as it is at `base`, or `Unanswerable`."""
    text = git("show", f"{base}:{path}")
    if text is None:
        raise Unanswerable(f"{path} could not be read at {base}")
    return text


def worktree(path):
    """`path` as it is now, or `Unanswerable`."""
    try:
        return Path(path).read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as problem:
        raise Unanswerable(f"{path} could not be read here: {problem}") from None


def transition(base, changes):
    """The one `(old, new)` version this diff moves every manifest through, or `None`.

    Read out of the manifests the diff itself changed rather than passed in, and refused
    when they disagree: two different transitions in one diff is not a release, and the
    substitution below would then explain lines no bump wrote.
    """
    moves = set()
    for status, path in changes:
        if status != "M" or Path(path).name != "Cargo.toml":
            continue
        old = version_value(blob(base, path))
        new = version_value(worktree(path))
        if old is None or new is None or old == new:
            continue
        moves.add((old, new))
    if len(moves) != 1:
        return None
    old, new = moves.pop()
    if not SEMVER.match(old) or not SEMVER.match(new):
        return None
    return old, new


def decide(crate, base, changes):
    """`(verdict, reason)` for one crate over one already-read diff."""
    directory = f"crates/{crate}/"
    if not Path(directory).is_dir():
        raise Unanswerable(
            f"{directory} is not a directory of this repository, so this is not a crate "
            "whose behaviour a diff could reach"
        )
    if not changes:
        return RUN, "this tree is identical to the base, so there is no diff to read"

    move = transition(base, changes)

    for status, path in changes:
        name = Path(path).name
        inside = path.startswith(directory)
        if name == "CHANGELOG.md":
            # A changelog is prose about what already shipped. It is the one file a
            # release may add, rewrite or remove without reaching any behaviour.
            continue
        if status != "M":
            return RUN, (
                f"{path} was added, removed or renamed, which no version bump does"
            )
        if inside and name != "Cargo.toml":
            return RUN, (
                f"{path} is {crate}'s own source, so this diff reaches that plugin's "
                "behaviour"
            )
        old_text = blob(base, path)
        new_text = worktree(path)
        pairs = changed_pairs(old_text, new_text)
        if pairs is None:
            return RUN, (
                f"{path} gained or lost lines, which is more than a version bump"
            )
        if inside:
            sections = section_of(old_text.splitlines())
            for index, old_line, _ in pairs:
                if index >= len(sections) or not package_version_line(
                    old_line, sections[index]
                ):
                    return RUN, (
                        f"{path} changes {old_line.strip()!r}, which is not the "
                        f"`version` field of {crate}'s own package"
                    )
        if move is None:
            return RUN, (
                f"{path} changed and this diff moves no single version, so there is "
                "nothing that would explain the change as a bump"
            )
        old_version, new_version = move
        for _, old_line, new_line in pairs:
            if old_line.replace(old_version, new_version) != new_line:
                return RUN, (
                    f"{path} changes {old_line.strip()!r} into {new_line.strip()!r}, "
                    f"which is not that line with {old_version} bumped to {new_version}"
                )

    old_version, new_version = move if move else ("", "")
    return NOT_SELECTED, (
        f"every change is the {old_version} -> {new_version} version bump and its "
        "changelogs, so nothing here reaches this plugin's behaviour"
        if move
        else "this diff changes nothing but changelogs, which reach no behaviour"
    )


def report_not_selected(crate, base, reason):
    """Say, in the lane's own words, that it was not selected — and that it is not a skip.

    Distinct from a missing credential on purpose. `ONETASKGRAPH_LIVE_REQUIRED` keeps the
    meaning it has: it demands a credential where one was expected, and a run given none
    still fails through it. A defect in THIS decision therefore cannot make an absent
    credential read as an unselected lane, and a reader can tell the two apart from the
    first line.
    """
    say(f"the {crate} live lane was NOT SELECTED by this diff against {base}.")
    say(f"{reason} — so no live session is opened and its GraphQL budget is not drawn.")
    say(
        "this is not a missing credential and not a skip: a credential absent where one "
        "was expected still fails the run through ONETASKGRAPH_LIVE_REQUIRED."
    )


def answer(crate):
    """One crate's verdict, with every unanswerable question answered `run`."""
    try:
        base, resolved = base_commit()
        changes = changed_files(resolved)
        verdict, reason = decide(crate, resolved, changes)
    except Unanswerable as question:
        say(f"cannot tell whether the {crate} live lane is reachable: {question}.")
        say("answering `run`, because this decision fails toward spending the budget.")
        return RUN
    if verdict == NOT_SELECTED:
        report_not_selected(crate, base, reason)
    return verdict


if MODE == "--nx-exclusions":
    crates = live_crates()
    if crates is None:
        say("answering with no exclusions, because this decision fails toward running.")
        sys.exit(0)
    excluded = [crate for crate in crates if answer(crate) == NOT_SELECTED]
    if excluded:
        print(f"--exclude={','.join(excluded)}")
else:
    print(answer(MODE))
PY
