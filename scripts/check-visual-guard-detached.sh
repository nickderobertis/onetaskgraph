#!/usr/bin/env bash
# Run this project's two checks again from a DETACHED checkout, which is what CI runs them
# in and what no developer machine reproduces.
#
# `actions/checkout` leaves the workspace detached at the ref under test and keeps no local
# branch at all, and both of this project's checks clone that workspace into a scratch tree.
# Two things about such a clone differ from one taken on a person's machine, and neither is
# visible from a checkout that is on a branch:
#
#   * it inherits no branches, so it has no `origin/HEAD` — and a `git merge-base` naming
#     one answers empty whatever it is asked about, which turns an assertion written
#     against it into one that cannot fail;
#   * it arrives already detached at the commit its own checkout then asks for, so that
#     checkout moves HEAD nowhere, writes no "moving from" reflog entry, and `git checkout -`
#     — which is `@{-1}`, read out of that reflog — fails with
#     "pathspec '-' did not match any file(s) known to git".
#
# That second one is not hypothetical: it is how scripts/check-visual-guard.sh came to pass
# on every machine a person ran it from and refuse this repository's own branch on the
# ubuntu lane, reported as the guard having broken rather than as the runner's checkout
# being shaped differently. Nothing local could see it, so this is that lane's condition
# brought here.
#
# The subjects are real and unmodified; the only stand-in is for the host, which is the
# variable under test. Case 1 refuses to let the rest pass vacuously, because a copy still
# on a branch would look exactly like a pass. It also covers the check `nx` has never
# reached on a runner — screenshots:test is serial, so the guard check failing meant
# scripts/check-visual-tools.sh had never run under CI's checkout at all.
#
# llmlint: ignore-file[code_lands_in_the_domain_that_owns_it] three commands of the `scripts` project enumerate that one directory; screenshots/AGENTS.md, "Where this machinery lives", is why.
set -euo pipefail

fatal() {
  echo "check-visual-guard-detached: $1" >&2
  echo "check-visual-guard-detached: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just check' does"
readonly ROOT

# The path is built from $ROOT at run time, so ShellCheck cannot follow it; the directive
# names the file it resolves to. Tested before it is sourced rather than guarded after:
# bash 3.2 ends the shell where `source` cannot find its file, so the handler after `||`
# never runs there and the reader is told nothing about what to restore.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check clones into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The stand-in runner checkout. Cloned through scratch_clone for the reason every guard
# here does — the gate runs from a hook, and git exports GIT_DIR to hooks, where it
# overrides `git -C` — and then put into the shape actions/checkout leaves: detached, with
# every local branch removed.
readonly DETACHED="$scratch/runner"
scratch_clone "$ROOT" "$DETACHED" || fatal \
  "could not clone this repository into $DETACHED" \
  "check 'git status' here and the free space on \$TMPDIR, then rerun"
while read -r branch; do
  [ -z "$branch" ] && continue
  git -C "$DETACHED" branch --quiet --delete --force "$branch" >/dev/null 2>&1 || fatal \
    "could not remove the local branch '$branch' from $DETACHED" \
    "report this; the copy has to hold no branch, which is what a runner's checkout holds"
done <<EOF
$(git -C "$DETACHED" for-each-ref --format='%(refname:short)' refs/heads)
EOF
# And the WORKING tree's tracked files over the top, exactly as the subjects themselves do
# with the clones they take: what is under test is the two checks as they are right now,
# not as they were last committed.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$DETACHED" || fatal \
  "could not copy $ROOT's tracked files over the copy at $DETACHED" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

# Case 1: the simulation took. Three halves rather than one, because each alone would pass
# on a host this cannot reproduce — and the third is the one that matters, since it is the
# property the subjects actually meet rather than the two that produce it.
git -C "$DETACHED" symbolic-ref --quiet HEAD >/dev/null 2>&1 && fatal \
  "the stand-in checkout at $DETACHED is on a branch, so it is not the shape a runner is in" \
  "report this; 'git checkout --detach' should have left HEAD naming a commit"
remaining="$(git -C "$DETACHED" for-each-ref --format='%(refname)' refs/heads)"
[ -z "$remaining" ] || fatal \
  "the stand-in checkout at $DETACHED still carries local branches, so it is not the shape a runner is in" \
  "report this; every refs/heads there had to be removed, and these are left: $remaining"
readonly PROBE="$scratch/probe"
scratch_clone "$DETACHED" "$PROBE" || fatal \
  "could not clone the stand-in checkout at $DETACHED into $PROBE" \
  "report this; a scratch clone of it is what the two checks below take"
git -C "$PROBE" rev-parse --verify --quiet origin/HEAD >/dev/null 2>&1 && fatal \
  "a scratch clone of $DETACHED resolved origin/HEAD, so this host does not reproduce a runner's checkout" \
  "report this; the two checks below would pass here for a reason CI does not have"

# Case 2: the real checks, from there. Their own diagnostics are what a failure prints —
# this check adds only which shape they were run in.
for subject in check-visual-guard.sh check-visual-tools.sh; do
  (cd "$DETACHED" && bash "scripts/$subject") || fatal \
    "scripts/$subject refused when run from a detached checkout, though it passes from one on a branch" \
    "read its diagnostic above; something in it reads a branch, an origin/HEAD or a reflog entry a runner's checkout does not have"
done
