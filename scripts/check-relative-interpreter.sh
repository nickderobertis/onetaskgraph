#!/usr/bin/env bash
# Prove the gate's python-bearing scripts survive a relative interpreter entry on PATH.
#
# `command -v python3` answers with the PATH entry it matched, and that entry may be
# relative: a virtualenv activation, a direnv, or an orchestration host with no absolute
# worktree path to write when it builds a PATH all produce one. Bash then caches the answer
# and carries it into every subshell, including the ones these scripts open with a working
# directory of their own — where the same spelling names nothing and the invocation dies
# `No such file or directory`, exit 127.
#
# That is not a warning. It is indistinguishable from the command being absent, and each of
# these three scripts reported it as its own subject having failed: a product-version helper
# that exited 127 "expected 2", a path-spelling scan that "did not name" what it was looking
# for, and live-session paths that "do not all consult the decision". The first of those
# refused this repository's own merge path.
#
# So each script is resolved once and absolutely, and this is where that is proven — by
# running the real scripts, from the directory the gate runs them in, with a relative entry
# first on PATH. Nothing stands in for the scripts; the shim stands in only for the host,
# which is the variable under test. Case 1 refuses to let the rest pass vacuously, because a
# simulation that did not take would look exactly like a pass.
#
# The cost is stated rather than discovered: this runs three real checks a second time, about
# five minutes, of which scripts/test-distribution.sh is half. It buys the only evidence there
# is — the defect is invisible to every host whose PATH is absolute, which is all three of the
# required lanes, so nothing else in this gate can see it — and it is paid only when scripts/
# changes, which is what selects the project this check belongs to.
set -euo pipefail

fatal() {
  echo "check-relative-interpreter: $1" >&2
  echo "check-relative-interpreter: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT
cd "$ROOT"

real_python3="$(command -v python3)" || fatal \
  "no python3 on PATH, and every script below reports through it" \
  "install python3, or run 'just bootstrap', then rerun"
# Absolute, because the shim below execs it from wherever a case has arrived at.
case $real_python3 in
  /* | ?:[\\/]*) ;;
  *) real_python3=$(cd "$(dirname "$real_python3")" && pwd)/$(basename "$real_python3") ;;
esac
readonly real_python3

# The probe lives under target/ so the entry that names it is relative to this repository's
# root — the shape the defect needs — on all three platforms and without a path this script
# has to compute. A scratch directory elsewhere would need a relative spelling of $TMPDIR,
# which on Windows may be on another drive and have none at all.
readonly probe="target/relative-interpreter-probe"
cleanup() {
  rm -rf "$ROOT/$probe"
}
trap cleanup EXIT

rm -rf "$ROOT/$probe"
mkdir -p "$ROOT/$probe/bin" || fatal \
  "could not create the probe interpreter directory at $probe" \
  "check the permissions of $ROOT/target and 'df -h' for free space, then rerun"
printf '#!/usr/bin/env bash\nexec "%s" "$@"\n' "$real_python3" > "$ROOT/$probe/bin/python3"
chmod +x "$ROOT/$probe/bin/python3"

export PATH="$probe/bin:$PATH"

# Case 1: the simulation took. Two halves, because either one alone would pass on a host
# this cannot reproduce: the entry this shell matches has to be the relative one, and a
# command resolved through it has to break on a change of working directory — which is the
# failure the scripts below must not have.
matched="$(command -v python3)" || fatal \
  "the probe interpreter is not on PATH, so nothing below is being simulated" \
  "check that $probe/bin/python3 was created and is executable, then rerun"
case $matched in
  /* | ?:[\\/]*)
    fatal \
      "python3 still resolves absolutely ($matched), so the simulation did not take" \
      "check that $probe/bin precedes every absolute python entry in PATH, then rerun"
    ;;
esac

# The defect itself, put to a shell of its own: resolve python3 here, change directory, run
# it again. Exit 127 is the whole of what the scripts below must not do, so anything else —
# including success — means this host is not the one they have to survive.
probe_status=0
bash -c 'python3 -c "pass" || exit 64; cd "$1" || exit 65; python3 -c "pass"' \
  _ "$ROOT/$probe" 2>/dev/null || probe_status=$?
case $probe_status in
  127) ;;
  0)
    fatal \
      "a relative interpreter survived a change of working directory, so this host cannot reproduce the defect" \
      "run this check from a shell whose PATH is the gate's, as 'just script-check' does"
    ;;
  64)
    fatal \
      "the probe interpreter at $probe/bin/python3 does not run, so nothing below is being simulated" \
      "run it by hand and fix what it reports, then rerun"
    ;;
  *)
    fatal \
      "the probe could not be put to a change of working directory (exit $probe_status)" \
      "check that $ROOT/$probe is a directory this shell can enter, then rerun"
    ;;
esac

# Cases 2-4: the real scripts, from the directory the gate runs them in.
failures=0
for script in \
  scripts/test-distribution.sh \
  scripts/check-guard-path-spelling.sh \
  scripts/check-live-lane-selection.sh; do
  if output="$(bash "$script" 2>&1)"; then
    continue
  fi
  failures=$((failures + 1))
  echo "check-relative-interpreter: $script failed with a relative python3 first on PATH." >&2
  echo "check-relative-interpreter: it said:" >&2
  printf '%s\n' "$output" | sed 's/^/    /' >&2
done

if [ "$failures" -ne 0 ]; then
  echo "check-relative-interpreter: $failures script(s) failed." >&2
  echo "check-relative-interpreter: next: a 'No such file or directory' or an exit 127 above" >&2
  echo "check-relative-interpreter: means that script resolves its interpreter in one working" >&2
  echo "check-relative-interpreter: directory and runs it from another. Resolve it once and" >&2
  echo "check-relative-interpreter: absolutely where the script resolves it, as its siblings do." >&2
  exit 1
fi
