#!/usr/bin/env bash
# Watch this repository's own tests catch the defect its fixtures used to hide.
#
# The two checks beside this one prove the *rule* is enforced — every store fixture carries
# the discriminating properties, and a fixture lacking one is refused. Neither runs a test,
# so neither answers the question that matters: whether the difference the fixtures now
# carry reaches an assertion. Under a fixture spelling an item's id and its title the same
# string, `title: required_str(content, "id")` shipped green.
#
# So this reintroduces exactly that substitution, in a scratch copy, and asserts the crate's
# real tests go red on it — after running them unmutated, without which a suite that could
# not compile at all would look like the strictest suite in the repository.
set -euo pipefail

# ONE live session per lane per run, and this step is where a third would come from: it runs
# the WHOLE onetaskgraph-github-projects package, twice, and that package's `tests/live.rs`
# is an ordinary test of it — from outside the affected-selection fan-out, so with the
# ambient credentials it opened a session on every gate of every branch. Nothing is lost:
# what this asserts is that a fixture substitution turns the suite red, which reaches no API.
# The demand goes with the credentials, or the skip that clearing produces would fail this
# step for a session it is deliberately not running. AGENTS.md records the rest.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

fatal() {
  echo "check-fixture-discrimination: $1" >&2
  echo "check-fixture-discrimination: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-test' does"
readonly ROOT
cd "$ROOT" || fatal \
  "could not enter $ROOT" \
  "check that directory's permissions, then rerun"

command -v cargo >/dev/null 2>&1 || fatal \
  "cargo is not installed, so the suite this check mutates cannot be run" \
  "install the Rust toolchain from https://rustup.rs, then rerun"

# The path below is built from $ROOT at run time, so ShellCheck cannot follow it; the
# directive names the file it resolves to. Tested before it is sourced rather than guarded
# after: bash 3.2 ends the shell where `source` cannot find its file, so the handler after
# `||` never runs there.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
scratch_clone_strip_git_env

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check mutates" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The WORKING tree's tracked files, for the same reason check-store-fixtures-enforced.sh
# takes them: what is under test is the fixtures and the suite as they are right now, so an
# author repairing either does not watch this keep failing against the version they replaced.
mkdir -p "$scratch/repo" || fatal \
  "could not create the scratch tree at $scratch/repo" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
copy_tracked_files "$ROOT" "$scratch/repo" || fatal \
  "could not copy $ROOT's tracked files into $scratch/repo" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

# Its own target directory, inside the scratch tree, for what cargo hands back — so nothing
# it builds is uplifted over this repository's own artifacts — and this repository's build
# directory for everything cargo builds on the way, so every registry dependency the gate's
# own test build left is reused rather than compiled again, which on the Windows runner was
# most of this check's time. Cargo hashes a workspace crate by its path relative to the
# workspace, and does not rebuild one for a changed CARGO_MANIFEST_DIR, so a crate of the copy
# built into this repository's slot would bake in a path that is deleted when this exits and
# be read as fresh by the next build here. So every workspace crate of the copy gets a profile
# of its own — a codegen-unit count no build of this repository uses, with nothing observable
# — which hashes it apart, and the copy's files are touched so each of those is built afresh
# for this tree rather than taken from an earlier run's.
readonly CRATE=onetaskgraph-github-projects
# The copy's crate lives under a directory of its own name. Sharing the build directory puts
# the copy's test executables beside this repository's, each with a dep-info file naming its
# sources by their path relative to the workspace, and the last of them is the mutated suite.
# A test that finds a sibling crate's test executable by the source its dep-info names —
# live_pair.rs finds the plugin's `plugin` and `live` that way — would otherwise take the
# copy's newest one, mutated, whenever this ran beside or before it, as test-macos and
# test-windows run it. Under another directory no copy's dep-info names this crate's sources.
readonly COPY_DIR="crates/$CRATE-fixture-discrimination"
mv "$scratch/repo/crates/$CRATE" "$scratch/repo/$COPY_DIR" || fatal \
  "could not move the copy's crate to $COPY_DIR" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
COPY_MANIFEST="$scratch/repo/Cargo.toml" COPY_CRATE="$CRATE" COPY_TO="$COPY_DIR" python3 - <<'PY' || fatal \
  "could not point the copy's workspace dependency at $COPY_DIR" \
  "the message above names what changed; update this check to the root manifest's shape now"
import os
import pathlib

path = pathlib.Path(os.environ["COPY_MANIFEST"])
before = f'path = "crates/{os.environ["COPY_CRATE"]}"'
text = path.read_text(encoding="utf-8")
if text.count(before) != 1:
    raise SystemExit(f"check-fixture-discrimination: {before} appears {text.count(before)} times in Cargo.toml, not once")
path.write_text(text.replace(before, f'path = "{os.environ["COPY_TO"]}"'), encoding="utf-8")
PY
readonly SOURCE="$COPY_DIR/src/lib.rs"
METADATA="$(cd "$ROOT" && cargo metadata --no-deps --format-version 1)" || fatal \
  "could not read $ROOT's cargo metadata, which names the build directory this check shares" \
  "run 'cargo metadata --no-deps' in $ROOT and fix what it reports, then rerun"
BUILD_DIR="$(printf '%s' "$METADATA" \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])' \
  | tr -d '\r')"
readonly BUILD_DIR
# What this run writes into the shared build directory is judged against this mark at the end.
touch "$scratch/started" || fatal \
  "could not mark when this run began in $scratch" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
APART=()
while IFS= read -r member; do
  [ -n "$member" ] || continue
  APART+=(--config "profile.dev.package.$member.codegen-units=255"
    --config "profile.test.package.$member.codegen-units=255")
done < <(printf '%s' "$METADATA" | python3 -c '
import json, sys
metadata = json.load(sys.stdin)
members = set(metadata["workspace_members"])
for package in metadata["packages"]:
    if package["id"] in members:
        print(package["name"])
' | tr -d '\r')
[ -n "$BUILD_DIR" ] && [ "${#APART[@]}" -gt 0 ] || fatal \
  "could not read $ROOT's target directory and workspace members from cargo metadata" \
  "run 'cargo metadata --no-deps' in $ROOT and fix what it reports, then rerun"
readonly APART
find "$scratch/repo" -type f -exec touch {} + || fatal \
  "could not touch the scratch copy's files, which keeps its crates from an earlier run's" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

SUITE_OUTPUT=""
SUITE_STATUS=0

run_suite() {
  # llmlint: ignore[work_goes_through_command_surface] the suite has to run inside the
  # scratch copy, against its own target directory — `just test` there would need an Nx
  # this copy deliberately never installs, and running it here would displace the build
  # of the repository under test. The same reason cargo is invoked directly in
  # scripts/check-isolation-enforced.sh and scripts/check-real-release-preparation.sh.
  SUITE_OUTPUT="$(cd "$scratch/repo" \
    && CARGO_TARGET_DIR="$scratch/target" CARGO_BUILD_BUILD_DIR="$BUILD_DIR" \
      cargo test -q -p "$CRATE" "${APART[@]}" 2>&1)" \
    && SUITE_STATUS=0 || SUITE_STATUS=$?
}

report_suite_output() {
  printf '%s\n' "$SUITE_OUTPUT" | sed 's/^/    /' >&2
}

run_suite
if [ "$SUITE_STATUS" -ne 0 ]; then
  echo "check-fixture-discrimination: $CRATE's tests do not pass on the tree under test, so" >&2
  echo "check-fixture-discrimination: the case below would prove nothing — a suite that is" >&2
  echo "check-fixture-discrimination: already red goes red under any mutation. It said:" >&2
  report_suite_output
  echo "check-fixture-discrimination: next: run 'cargo test -p $CRATE' and fix what it reports" >&2
  echo "check-fixture-discrimination: first, then rerun." >&2
  exit 1
fi

# The defect exactly as it shipped: the board item's own identifier written into the
#    field its title belongs in. One site, and the count is asserted rather than assumed —
#    a substitution that matched nothing would leave the suite green and read as the
#    fixtures having caught nothing.
readonly BEFORE='let raw_title = required_str(content, "title")?;'
readonly AFTER='let raw_title = required_str(content, "id")?;'
MUTATION_SOURCE="$scratch/repo/$SOURCE" MUTATION_BEFORE="$BEFORE" MUTATION_AFTER="$AFTER" \
  python3 - <<'PY' || fatal \
  "could not reintroduce the id-where-a-title-belongs substitution" \
  "the message above names what changed; update BEFORE in this check to the site's shape now"
import os
import pathlib
import sys

path = pathlib.Path(os.environ["MUTATION_SOURCE"])
before, after = os.environ["MUTATION_BEFORE"], os.environ["MUTATION_AFTER"]
text = path.read_text(encoding="utf-8")
found = text.count(before)
if found != 1:
    print(
        f"check-fixture-discrimination: the line this check substitutes appears {found} "
        f"times in {path.name}, not once:",
        file=sys.stderr,
    )
    print(f"check-fixture-discrimination:   {before}", file=sys.stderr)
    raise SystemExit(1)
path.write_text(text.replace(before, after), encoding="utf-8")
PY

run_suite
if [ "$SUITE_STATUS" -eq 0 ]; then
  echo "check-fixture-discrimination: $CRATE reads every board item's identifier into the" >&2
  echo "check-fixture-discrimination: field its title belongs in, and its tests still pass." >&2
  echo "check-fixture-discrimination: That is the defect this release repaired, and the" >&2
  echo "check-fixture-discrimination: fixtures exist to separate it from the correct answer." >&2
  echo "check-fixture-discrimination: next: add an assertion over a title whose value differs" >&2
  echo "check-fixture-discrimination: from its item's identifier — 'bash scripts/check-store-fixtures.sh'" >&2
  echo "check-fixture-discrimination: names the fixtures that already carry such an item." >&2
  exit 1
fi

# A suite that fails to BUILD fails too, and would satisfy the case above while proving
# nothing about any assertion. `test result: FAILED` is only ever printed by a harness that
# compiled, linked and ran.
case "$SUITE_OUTPUT" in
  *"test result: FAILED"*) ;;
  *)
    echo "check-fixture-discrimination: the substitution stopped $CRATE compiling rather than" >&2
    echo "check-fixture-discrimination: failing an assertion, so nothing here says the fixtures" >&2
    echo "check-fixture-discrimination: discriminate. It said:" >&2
    report_suite_output
    echo "check-fixture-discrimination: next: update AFTER in this check so the substitution is" >&2
    echo "check-fixture-discrimination: type-correct — it must change what a test reads, not the" >&2
    echo "check-fixture-discrimination: build." >&2
    exit 1
    ;;
esac

# The copy's test executables in the shared build directory name its sources under COPY_DIR
# and never this crate's own, which is what keeps a test that finds the plugin's test
# executables by their dep-info off the copy's. Only files naming COPY_DIR are the copy's: this
# repository's own build of the crate may be writing beside them while this runs.
copies="$(find "$BUILD_DIR/debug/deps" -name '*.d' -newer "$scratch/started" -type f \
  -exec grep -l "$COPY_DIR/" {} + 2>/dev/null || true)"
if [ -z "$copies" ]; then
  echo "check-fixture-discrimination: no dep-info written by this run names $COPY_DIR/, so the" >&2
  echo "check-fixture-discrimination: copy was not built from the directory it was moved to." >&2
  echo "check-fixture-discrimination: next: check that the copy's crate is moved to COPY_DIR before" >&2
  echo "check-fixture-discrimination: run_suite and that BUILD_DIR is the build directory cargo uses, then rerun." >&2
  exit 1
fi
named=""
while IFS= read -r copy; do
  if grep -q "crates/$CRATE/" "$copy"; then
    named="$copy"
    break
  fi
done <<EOF
$copies
EOF
if [ -n "$named" ]; then
  echo "check-fixture-discrimination: the copy's build wrote $named, which names" >&2
  echo "check-fixture-discrimination: crates/$CRATE/, so a test that finds that crate's test" >&2
  echo "check-fixture-discrimination: executables by the sources they name could take the copy's" >&2
  echo "check-fixture-discrimination: mutated one." >&2
  echo "check-fixture-discrimination: next: build the copy's crate from a directory of another name" >&2
  echo "check-fixture-discrimination: (COPY_DIR in this check), then rerun." >&2
  exit 1
fi
