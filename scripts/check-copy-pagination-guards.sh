#!/usr/bin/env bash
# Fail when a pagination loop on the copy path is missing the cursor-repeat guard or the
# page bound — or when either is spelled a second time instead of called.
#
# `engine/fetch.rs` decided both once: a source answering a cursor with the cursor it was
# given is refused rather than waited on, and so is a page longer than the one asked for.
# This holds the copy path to them, enumerating no loop, so one added later is caught here
# rather than by an operator watching a copy spin. Two halves, because the copy path pages
# at two levels:
#
#   * A loop reading a page **from a plugin** — through `ResolvedSource::source()` — owes
#     both refusals itself. Nothing between that answer and the loop has looked at it.
#   * A loop reading one **from a verb of this engine whose bound is demonstrable** owes
#     the cursor-repeat guard and must NOT call the page bound: such a verb ends in the
#     assembly that stops at the budget, so a bound there could never fail, and a guard
#     that cannot fail reads like the guard.
#
# "Demonstrable" is load-bearing: being a method of this engine bounds nothing, so a page
# from a new `self.some_page(..)` that assembles its own answer must not be exempted by
# how the call is spelled. Every engine method a paginating loop names is FOLLOWED to its
# definition and through what it calls in turn, and counts as bounded only on reaching the
# one page assembly this engine has — read here, with the budget stop it rests on. A verb
# that does not reach it, or that cannot be found, is not bounded.
#
# Named exemptions are what this deliberately does not have.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

COPY_PATH="crates/onetaskgraph-core/src/engine/copy.rs" \
AUTHORITY="crates/onetaskgraph-core/src/engine/fetch.rs" \
ENGINE="crates/onetaskgraph-core/src/engine/mod.rs" \
python3 <<'PY'
import os
import pathlib
import re
import sys

copy_path = os.environ["COPY_PATH"]
authority_path = os.environ["AUTHORITY"]
engine_path = os.environ["ENGINE"]


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
    ("pub(crate) fn unrepeated", "the cursor-repeat refusal"),
    ("pub(crate) fn fits", "the page bound"),
):
    if wanted not in authority:
        fail(
            f"{authority_path} defines no `{wanted}`, so {what} has no single "
            "implementation for the copy path to share",
            f"keep {what} there as `{wanted}`, or update this check to read it wherever "
            "it moved — and move every caller in the same change.",
        )

# The stop the whole engine-verb exemption below rests on: the merge that assembles a page
# refuses to hand back more rows than the budget it was asked for. Read rather than
# assumed, because the day it stops being true is the day a loop reading an engine verb
# needs its own bound back.
MERGE_BOUND = re.compile(r"items\.len\(\)\s*as\s*u64\s*>=\s*u64::from\(budget\)")
if MERGE_BOUND.search(authority) is None:
    fail(
        f"{pathlib.PurePath(authority_path).as_posix()} no longer shows the merge stopping "
        "at the budget it was asked for, so no loop reading a verb of this engine can be "
        "excused its own page bound",
        "restore that stop in the merge, or — if a page is bounded some other way now — "
        "teach this check to read the new one and give every exempted loop its own "
        "`fits` until you have.",
    )

engine = read(engine_path)
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
# What makes a loop a pagination loop: it builds a page request, or it reads the `next`
# field off the page it got back. Deliberately NOT a list of the methods that serve a
# page — such a list is this contract restated in a second place, and it goes stale the
# day a paging method is added or renamed, which is the day it would matter. Every walk
# advances by reading a `next`, whatever it called to get one. The negative lookahead is
# what keeps an iterator's `.next()` — a call, not a field — from reading as one.
PAGING = re.compile(r"request_for\(|PageRequest|\.next\b(?!\()")


def body_of(text, opened):
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

# Where a loop's page comes from, which is what decides which half of the rule it owes.
# A plugin is reached through `ResolvedSource::source()` and nowhere else, so that call is
# the marker; a verb of this engine is reached through `self`. A loop showing neither is
# read as reading a plugin, because the safe reading of "cannot tell" is the strict one.
FROM_PLUGIN = re.compile(r"\.source\(\)")
ENGINE_CALL = re.compile(r"\bself\.(\w+)\(")

# The one page assembly this engine has, and the thing that makes it an assembly rather
# than a name: it hands its rows to the merge under a budget, and the merge stops there.
# A verb reaching this is bounded; a verb that does not is not, however it is spelled.
ASSEMBLY = "finish"
ASSEMBLY_CALL = re.compile(r"\.%s\(" % ASSEMBLY)
ASSEMBLY_MERGES = re.compile(r"\bmerge\([^)]*\bbudget\b")


def body_after(text, index):
    """The braced body opening at or after `index`, or None when there is no whole one."""
    try:
        opened = text.index("{", index)
    except ValueError:
        return None
    depth = 0
    for at in range(opened, len(text)):
        if text[at] == "{":
            depth += 1
        elif text[at] == "}":
            depth -= 1
            if depth == 0:
                return text[opened + 1 : at]
    return None


def defined(name):
    """The body of `fn name`, wherever on the copy path or in the engine module it is."""
    wanted = re.compile(r"\bfn\s+%s\s*[<(]" % re.escape(name))
    for text in (engine, source):
        found = wanted.search(text)
        if found is not None:
            return body_after(text, found.end() - 1)
    return None


assembly = defined(ASSEMBLY)
if assembly is None or ASSEMBLY_MERGES.search(assembly) is None:
    fail(
        f"{pathlib.PurePath(engine_path).as_posix()} defines no `fn {ASSEMBLY}` that hands "
        "its rows to the merge under a budget, so this check cannot tell a bounded verb of "
        "this engine from an unbounded one",
        "keep that assembly there, or teach this check to read whichever one bounds a page "
        "now — until then every loop taking its page from a verb of this engine owes its "
        "own `fits`.",
    )


def bounded(name, seen=None):
    """Whether `self.name(..)` yields a page this engine has already bounded.

    Followed rather than assumed, and followed through the engine methods that method
    calls in turn: what makes a verb bounded is reaching the assembly above, not being
    reached through `self`. A method this check cannot find is not bounded — the safe
    reading of "cannot tell" is the strict one here too.
    """
    seen = seen or set()
    if name in seen:
        return False
    seen.add(name)
    body = defined(name)
    if body is None:
        return False
    if ASSEMBLY_CALL.search(body) is not None:
        return True
    return any(bounded(called, seen) for called in set(ENGINE_CALL.findall(body)))


problems = []
for line, opened, body in loops:
    own = body
    for other_line, other_opened, other_body in loops:
        if other_opened > opened and other_body in own:
            own = own.replace(other_body, "")
    if PAGING.search(own) is None:
        continue
    if "unrepeated(" not in own:
        problems.append(f"{posix}:{line} paginates without the cursor-repeat guard")

    called = sorted(set(ENGINE_CALL.findall(own)))
    unbounded = [name for name in called if not bounded(name)]
    if FROM_PLUGIN.search(own) is not None:
        why = "reads a plugin's page"
    elif not called:
        why = "takes its page from neither a plugin nor a verb of this engine"
    elif unbounded:
        why = (
            "takes its page from `self."
            + "`, `self.".join(unbounded)
            + "`, which does not reach this engine's bounded page assembly"
        )
    else:
        why = None

    if why is not None and "fits(" not in own:
        problems.append(f"{posix}:{line} {why}, and has no page bound of its own")
    if why is None and "fits(" in own:
        problems.append(
            f"{posix}:{line} bounds a page this engine's own verb already bounded, so that "
            "call can never fail"
        )

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
        "a pagination loop on the copy path does not hold the half of the rule it owes",
        "call `unrepeated(page.next.as_ref(), asked.as_ref(), \"<what is being walked>\")` in "
        "every loop above, and `fits(page.items.len(), request.limit)` in each one whose "
        f"page this engine has not already bounded — both from {authority_path}. Only a loop "
        "taking its page from a verb that reaches this engine's own page assembly is excused "
        "the bound, and such a loop must not restate it.",
    )
PY
