#!/usr/bin/env bash
# The measurement of onetaskgraph-pool-maintenance-seconds (budgets.yaml beside this file),
# and nothing else runs it.
#
# The figure is the pass a regular post-run maintenance faces — one ordinary run's worth of
# new superseded builds — never a first pass over accumulated history, so this sets that
# state up before it times anything:
#   (a) runs the ordinary gate build, so a build a source change since this checkout last
#       built has superseded is backlog too, and then the declared maintenance, untimed, to
#       clear that backlog — repeating the two until a pass removes nothing, because one
#       pass over a warm directory is not a steady state. The pruner groups every build
#       named for one unit (a crate's library, its test binary, its build script and its
#       run, and each feature set it is built with), and of those older than the window
#       keeps only the newest; a directory restored from a cache holds the ordinary builds
#       at their old modification times, so a pass removes some the gate still needs, the
#       next build makes them anew, and that build supersedes the old sibling the pass
#       had kept. Left there, the timed pass in (d) removed those siblings too;
#   (b) builds the workspace's test targets under an alternative profile setting that gives
#       every unit, this repository's own crates included, a new metadata hash in the same
#       profile directory, then backdates exactly the entries that build created to a date
#       past the 2-day window and older than anything a slot holds, so they stand in for a
#       run's builds that have since been superseded;
#   (c) runs the ordinary gate build, so each unit's newest build is the ordinary one;
#   (d) times one declared pass, which removes the alternative build.
# Both passes are the justfile's `pool-maintenance` recipe, the command onevcs.toml declares.
# Only (d) reports the figure, through cargo-prune-superseded's own onebudgetspec report;
# (a)'s seconds are appended to that report's detail as separate evidence, and the figure is
# left as the pruner wrote it. onebudgetspec alone judges it.
#
# It really prunes this checkout's target directory and builds it twice: run it only in a
# worktree whose target directory is idle.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
cd "$ROOT"

if ! command -v cargo-prune-superseded >/dev/null 2>&1; then
  echo "pool-maintenance/measure.sh: cargo-prune-superseded is not on PATH; run 'just pool-maintenance-tool', then re-run." >&2
  exit 1
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
# The lists of entries, by unit name and metadata hash, before and after each step. Kept
# under target/ (whose root the pruner never touches) so the evidence outlives the run.
readonly EVIDENCE="target/pool-maintenance-evidence"
rm -rf "$EVIDENCE"
mkdir -p "$EVIDENCE"
# Past the 2-day window, and older than any build a long-lived slot can hold, so no
# ordinary build is ever older than the stand-in for a superseded one.
readonly SUPERSEDED_AT="2001-01-01T00:00:00"

# Every build entry the pruner selects among, one per line as <profile>/<dir>/<entry>:
# the top-level entries of deps/, .fingerprint/, build/ and incremental/ in every profile
# directory under target/. Their names carry the unit name and metadata hash.
list_entries() {
  local fingerprint profile dir
  find target -name .fingerprint -type d -prune 2>/dev/null | while IFS= read -r fingerprint; do
    profile="$(dirname "$fingerprint")"
    for dir in deps .fingerprint build incremental; do
      [ -d "$profile/$dir" ] || continue
      (cd "$profile/$dir" && ls -A) | sed "s|^|$profile/$dir/|"
    done
  done | LC_ALL=C sort
}

# The gate's own test build of every crate, without running a test.
ordinary_build() {
  cargo test --quiet --workspace --all-features --locked --no-run
}

# The same build with line-table debuginfo in place of .cargo/config.toml's none. The
# profile is part of every unit's metadata hash, so each unit gets a new build beside the
# ordinary one, in the same target/debug the ordinary build writes.
alternative_build() {
  CARGO_PROFILE_DEV_DEBUG=line-tables-only cargo test --quiet --workspace --all-features --locked --no-run
}

seconds_reported() {
  python3 -c 'import json, sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["value"])' "$1"
}

# (a) Clear any backlog, untimed, until a pass over a fresh ordinary build removes nothing:
# each pass's seconds go to a file of this run's own. A pass after the first removes only an
# old build the build before it superseded by making a sibling anew, and what a build makes
# stays inside the window from then on, so this settles within a few passes; a directory
# that does not is an error, not a figure.
readonly BACKLOG_PASSES_AT_MOST=5
backlog_seconds=""
pass=0
while :; do
  pass=$((pass + 1))
  if [ "$pass" -gt "$BACKLOG_PASSES_AT_MOST" ]; then
    echo "pool-maintenance/measure.sh: $BACKLOG_PASSES_AT_MOST untimed passes each removed builds, so the target directory never reached a steady state to time a pass from; see $EVIDENCE/backlog-removed-$BACKLOG_PASSES_AT_MOST.txt for the last pass's removals." >&2
    exit 1
  fi
  ordinary_build
  list_entries > "$EVIDENCE/backlog-before-$pass.txt"
  ONEBUDGETSPEC_RESULT="$scratch/backlog-$pass.json" just pool-maintenance >&2
  list_entries > "$EVIDENCE/backlog-after-$pass.txt"
  backlog_seconds="${backlog_seconds:+$backlog_seconds, }$(seconds_reported "$scratch/backlog-$pass.json" | tr -d '\r')"
  LC_ALL=C comm -23 "$EVIDENCE/backlog-before-$pass.txt" "$EVIDENCE/backlog-after-$pass.txt" \
    > "$EVIDENCE/backlog-removed-$pass.txt"
  [ -s "$EVIDENCE/backlog-removed-$pass.txt" ] || break
done

# (b) One run's worth of builds that the next ordinary build supersedes.
list_entries > "$EVIDENCE/before-alternative.txt"
alternative_build
list_entries > "$EVIDENCE/after-alternative.txt"
LC_ALL=C comm -13 "$EVIDENCE/before-alternative.txt" "$EVIDENCE/after-alternative.txt" \
  > "$EVIDENCE/alternative-created.txt"
if [ ! -s "$EVIDENCE/alternative-created.txt" ]; then
  echo "pool-maintenance/measure.sh: the alternative build created no new entry, so there is nothing superseded to time; check that CARGO_PROFILE_DEV_DEBUG still changes metadata hashes." >&2
  exit 1
fi
while IFS= read -r entry; do
  find "$entry" -exec touch -h -d "$SUPERSEDED_AT" {} +
done < "$EVIDENCE/alternative-created.txt"

# (c) The ordinary build, newest for every unit.
ordinary_build
list_entries > "$EVIDENCE/before-timed-pass.txt"

# (d) The timed pass, reporting through the pruner itself.
just pool-maintenance >&2
list_entries > "$EVIDENCE/after-timed-pass.txt"

LC_ALL=C comm -23 "$EVIDENCE/before-timed-pass.txt" "$EVIDENCE/after-timed-pass.txt" \
  > "$EVIDENCE/removed.txt"
LC_ALL=C comm -23 "$EVIDENCE/removed.txt" "$EVIDENCE/alternative-created.txt" \
  > "$EVIDENCE/removed-not-alternative.txt"
LC_ALL=C comm -23 "$EVIDENCE/before-timed-pass.txt" "$EVIDENCE/alternative-created.txt" \
  > "$EVIDENCE/ordinary.txt"
LC_ALL=C comm -23 "$EVIDENCE/ordinary.txt" "$EVIDENCE/after-timed-pass.txt" \
  > "$EVIDENCE/ordinary-missing.txt"
# The state above is what makes the figure the steady-state pass; a run where it did not
# hold measured something else, so it is an error rather than a figure.
if [ ! -s "$EVIDENCE/removed.txt" ]; then
  echo "pool-maintenance/measure.sh: the timed pass removed nothing, so it did not face a run's superseded builds; see $EVIDENCE." >&2
  exit 1
fi
if [ -s "$EVIDENCE/removed-not-alternative.txt" ] || [ -s "$EVIDENCE/ordinary-missing.txt" ]; then
  echo "pool-maintenance/measure.sh: the timed pass removed entries the alternative build did not create; see $EVIDENCE/removed-not-alternative.txt and $EVIDENCE/ordinary-missing.txt." >&2
  exit 1
fi
echo "pool-maintenance/measure.sh: the timed pass removed $(wc -l < "$EVIDENCE/removed.txt" | tr -d ' ') of the alternative build's $(wc -l < "$EVIDENCE/alternative-created.txt" | tr -d ' ') entries and none of the ordinary build's $(wc -l < "$EVIDENCE/ordinary.txt" | tr -d ' '); lists in $EVIDENCE." >&2

if [ -n "${ONEBUDGETSPEC_RESULT:-}" ]; then
  python3 - "$ONEBUDGETSPEC_RESULT" "$backlog_seconds" <<'PY'
import json
import sys

path, backlog = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as handle:
    result = json.load(handle)
result["detail"] = (
    f"{result.get('detail', '')}; untimed backlog passes before it, not counted: {backlog} s"
)
with open(path, "w", encoding="utf-8", newline="\n") as handle:
    json.dump(result, handle)
PY
fi
