#!/usr/bin/env bash
# Prove the guards that report a path name it the same way whatever separator the host
# spells paths with.
#
# The guard reports through python, and python renders a path with the running platform's
# separator. So the same guard that says `scripts/check-distribution-contract.sh` on Linux
# and macOS said `scripts\check-distribution-contract.sh` on the Windows runner: the right
# file, the right line, the right builtin, in a spelling no case of
# scripts/check-bash4-array-builtins-enforced.sh matched — which failed `check
# (windows-latest)` on a guard that was doing its job, and which no Linux or macOS lane
# could reproduce.
#
# This is that lane, on this one. The real guards and the real enforcement are run twice over
# real fixtures: once as the host renders paths, and once through a python whose pathlib
# spells them the way Windows does. Nothing stands in for the scan — only for the platform,
# which is the variable under test.
#
# It covers every guard here that discovers files through pathlib and then names them,
# because a path a guard renders is also a path it compares: `check-store-fixtures.sh`
# matches what it discovered against a manifest keyed by forward slashes, so on the Windows
# runner it reported all seven of this repository's fixtures as ones it had never been told
# about and failed `check (windows-latest)` a second time on this same class.
set -euo pipefail

fatal() {
  echo "check-guard-path-spelling: $1" >&2
  echo "check-guard-path-spelling: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

# On Windows the guard's own pathlib already spells with a backslash, so the run `just
# script-check` does there IS the native case this simulates — case 10 of
# scripts/check-bash4-array-builtins-enforced.sh pins the normalised spelling on that
# platform directly. Simulating it a second time would only put this shim's portability on
# the required check it exists to protect.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-guard-path-spelling: skipped on Windows (its own path flavour is the one this simulates; the enforcement's nested case pins the spelling here natively)" >&2
    exit 0
    ;;
esac

# Held in a variable rather than written into the fixture below, because the guard scans
# this file too and reads a line rather than parsing it. Naming it through a variable keeps
# this file ordinary text instead of something the guard would need a skip list for.
readonly BUILTIN="mapfile"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check mutates" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The WORKING tree's scripts, not HEAD's: what is under test is the guard as it is right
# now, so an author repairing it does not watch this check keep failing against the version
# they just replaced.
cp -a "$ROOT/scripts" "$scratch/scripts" || fatal \
  "could not copy $ROOT/scripts into $scratch" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

real_python3="$(command -v python3)" || fatal \
  "no python3 on PATH, and the guard under test reports through it" \
  "install python3, or run 'just bootstrap', then rerun"
readonly real_python3

failures=0
OUTPUT=""
STATUS=0

fixture() {
  "$@" || fatal \
    "could not $* while planting or clearing a fixture under $scratch" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
}

report_output() {
  printf '%s\n' "$OUTPUT" | sed 's/^/    /' >&2
}

# Replace one literal substring of the scratch guard. python3 rather than `sed -i`, whose
# in-place spelling differs between GNU and BSD and so would fail on the macOS runner. The
# real python3 by absolute path, never the shim below: this rewrites the fixture, it is not
# part of what is being simulated.
substitute() {
  "$real_python3" - "$1" "$2" "$3" <<'PY' || fatal \
    "the helper that rewrites the scratch guard did not finish, so that case was never run" \
    "run 'python3 --version' to confirm a working python3 is on PATH, then rerun"
import pathlib
import sys

path, before, after = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
text = path.read_text(encoding="utf-8")
if before not in text:
    print(
        f"check-guard-path-spelling: {path} no longer contains the text this case rewrites,\n"
        f"check-guard-path-spelling: so the case would prove nothing:\n"
        f"    {before}\n"
        "check-guard-path-spelling: next: update the case to the text that replaced it.",
        file=sys.stderr,
    )
    raise SystemExit(1)
path.write_text(text.replace(before, after, 1), encoding="utf-8")
PY
}

# A python3 that hands the scan a path renderer spelling with a backslash, put in front of
# the one the guard finds on PATH. It wraps real pathlib objects rather than reaching into
# pathlib itself, because `Path.rglob` builds each child out of `str(parent)` and a patched
# `__str__` would leave it walking a tree that does not exist — the scan would then find
# nothing and this check would pass on a guard it never put anything to.
#
# What the wrapper reproduces is the Windows contract for exactly the three the scan uses:
# `str(path)` carries the platform's separator, `path.as_posix()` carries forward slashes,
# and `os.fspath(path)` opens the real file either way. Everything else is the real object's.
fixture mkdir -p "$scratch/bin"
cat > "$scratch/bin/foreign-separator.py" <<'PY' || fatal \
  "could not write the foreign-separator launcher in $scratch/bin, so nothing would be simulated" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
import os
import pathlib
import sys

native_path = pathlib.Path


class ForeignPath:
    """A path that spells itself the way the Windows runner's pathlib does."""

    def __init__(self, real):
        self._real = real

    def __str__(self):
        return str(self._real).replace("/", "\\")

    def __fspath__(self):
        return str(self._real)

    def as_posix(self):
        return self._real.as_posix()

    def __lt__(self, other):
        return self._real < other._real

    def rglob(self, pattern):
        return (ForeignPath(found) for found in self._real.rglob(pattern))

    # Spelled out for the same reason `rglob` is: both hand back an iterator rather than a
    # path, so `__getattr__`'s "wrap a PurePath result" would let the elements through
    # unwrapped and a scan using this one would walk real paths outside the simulation.
    def glob(self, pattern):
        return (ForeignPath(found) for found in self._real.glob(pattern))

    # Operators are looked up on the type, never through `__getattr__`, so a scan that
    # joins a path the ordinary way would raise here instead of being simulated.
    def __truediv__(self, other):
        return ForeignPath(self._real / other)

    # Anything the scan reaches for that is not spelled above is the real object's, with a
    # path it hands back wrapped again — so a scan that grows a new call stays simulated
    # rather than quietly stepping outside the simulation.
    def __getattr__(self, name):
        attribute = getattr(self._real, name)
        if isinstance(attribute, pathlib.PurePath):
            return ForeignPath(attribute)
        if callable(attribute):
            def wrapped(*args, **kwargs):
                result = attribute(*args, **kwargs)
                return ForeignPath(result) if isinstance(result, pathlib.PurePath) else result
            return wrapped
        return attribute


pathlib.Path = lambda *parts: ForeignPath(native_path(*parts))

real, args = sys.argv[1], sys.argv[2:]
# Only a script on stdin is the shape this simulates; anything else is passed straight
# through to the real interpreter, unpatched, so a caller doing something else is unaffected.
if not args or args[0] != "-":
    os.execv(real, [real] + args)
sys.argv = args
exec(compile(sys.stdin.read(), "<stdin>", "exec"), {"__name__": "__main__"})
PY
{
  echo '#!/usr/bin/env bash'
  echo 'set -euo pipefail'
  printf 'exec %q %q %q "$@"\n' \
    "$real_python3" "$scratch/bin/foreign-separator.py" "$real_python3"
} > "$scratch/bin/python3" || fatal \
  "could not write the foreign-separator python3 in $scratch/bin, so nothing would be simulated" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
fixture chmod +x "$scratch/bin/python3"

run_native() {
  OUTPUT="$(bash "$scratch/scripts/$1" 2>&1)" && STATUS=0 || STATUS=$?
}

run_foreign() {
  OUTPUT="$(PATH="$scratch/bin:$PATH" bash "$scratch/scripts/$1" 2>&1)" \
    && STATUS=0 || STATUS=$?
}

readonly GUARD="check-bash4-array-builtins.sh"
readonly ENFORCEMENT="check-bash4-array-builtins-enforced.sh"
# One directory down, so the path it is named by carries more than one separator.
readonly PLANTED="scripts/nested/deep-helper.sh"
readonly FOREIGN_PLANTED='scripts\nested\deep-helper.sh'

# Counts the failure and returns 0: every case below calls this directly, so a non-zero
# return would end the run under `set -e` — before the later cases, and before the closing
# report that is where this check says what to go and fix.
expect_naming() {
  local case_name="$1"
  shift
  if [ "$STATUS" -eq 0 ]; then
    echo "check-guard-path-spelling: $case_name — the guard passed a script that uses a bash 4" >&2
    echo "check-guard-path-spelling: array builtin, so this case reports on nothing." >&2
    failures=$((failures + 1))
    return
  fi
  local needle
  for needle in "$@"; do
    if ! grep -qF -- "$needle" <<<"$OUTPUT"; then
      echo "check-guard-path-spelling: $case_name — the guard refused, but its diagnostic never" >&2
      echo "check-guard-path-spelling: mentions '$needle'. It said:" >&2
      report_output
      failures=$((failures + 1))
      return
    fi
  done
}

# 1. The guard as the host renders paths. The baseline every case below is compared against,
#    and the control: without it a guard that named nothing at all would satisfy case 2.
fixture mkdir -p "$scratch/scripts/nested"
{
  echo '#!/usr/bin/env bash'
  echo 'set -euo pipefail'
  echo "$BUILTIN -t names < /dev/null"
  echo 'printf "%s\n" "${names[@]}"'
} > "$scratch/$PLANTED" || fatal \
  "could not write the nested fixture at $scratch/$PLANTED, so nothing would be put to the guard" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

run_native "$GUARD"
expect_naming "the guard on this host's own separator" "$PLANTED"
native_report="$OUTPUT"

# 2. The same guard, the same fixture, on a host that spells paths with a backslash. Byte
#    for byte the same report, or the enforcement's cases — and a reader pasting the path
#    back into the bash `just` runs on Windows — are reading a different one on that lane.
run_foreign "$GUARD"
expect_naming "the guard on a backslash-separator host" "$PLANTED"
if [ "$OUTPUT" != "$native_report" ]; then
  echo "check-guard-path-spelling: the guard reported differently on a backslash-separator host" >&2
  echo "check-guard-path-spelling: than on this one, so what it names depends on where it runs." >&2
  echo "check-guard-path-spelling: The backslash run said:" >&2
  report_output
  failures=$((failures + 1))
fi
if grep -qF -- "$FOREIGN_PLANTED" <<<"$OUTPUT"; then
  echo "check-guard-path-spelling: the guard named the script as '$FOREIGN_PLANTED'," >&2
  echo "check-guard-path-spelling: which is neither what its other lanes report nor a path the" >&2
  echo "check-guard-path-spelling: reader can paste back into the bash 'just' runs there." >&2
  failures=$((failures + 1))
fi

# 3. The control on the simulation itself. Cases 2 and 5 would both pass against a shim that
#    changed nothing, so the normalisation is removed from the scratch guard and the
#    backslash spelling has to appear — which is both what the Windows runner really printed
#    and the evidence that `display_path` is what stops it.
substitute "$scratch/scripts/$GUARD" "return path.as_posix()" "return str(path)"
run_foreign "$GUARD"
if ! grep -qF -- "$FOREIGN_PLANTED" <<<"$OUTPUT"; then
  echo "check-guard-path-spelling: with the path normalisation removed, the guard still did not" >&2
  echo "check-guard-path-spelling: name '$FOREIGN_PLANTED' on a backslash-separator" >&2
  echo "check-guard-path-spelling: host — so the simulation is not reproducing that host and the" >&2
  echo "check-guard-path-spelling: cases either side of this one prove nothing. It said:" >&2
  report_output
  failures=$((failures + 1))
fi

# 4. And the enforcement misses it, exactly as `check (windows-latest)` did: the guard
#    refuses, correctly, while every case asserting on a path reports it as having refused
#    without saying what to fix.
fixture rm -rf "$scratch/scripts/nested"
run_foreign "$ENFORCEMENT"
if [ "$STATUS" -eq 0 ]; then
  echo "check-guard-path-spelling: with the path normalisation removed, the enforcement passed on" >&2
  echo "check-guard-path-spelling: a backslash-separator host — so nothing there would have caught" >&2
  echo "check-guard-path-spelling: the failure this whole check is about." >&2
  failures=$((failures + 1))
elif ! grep -qF -- "scripts/check-distribution-contract.sh" <<<"$OUTPUT"; then
  echo "check-guard-path-spelling: with the path normalisation removed, the enforcement failed on a" >&2
  echo "check-guard-path-spelling: backslash-separator host without naming the spelling it expected," >&2
  echo "check-guard-path-spelling: so this case is no longer reproducing that failure. It said:" >&2
  report_output
  failures=$((failures + 1))
fi
fixture cp "$ROOT/scripts/$GUARD" "$scratch/scripts/$GUARD"

# 5. The whole enforcement, restored, on the backslash-separator host: every case has to pass
#    there as it does here. That is what "its cases assert against one spelling rather than
#    the host's" means, and it is the check `check (windows-latest)` runs.
run_foreign "$ENFORCEMENT"
if [ "$STATUS" -ne 0 ]; then
  echo "check-guard-path-spelling: the enforcement fails on a backslash-separator host, so one of" >&2
  echo "check-guard-path-spelling: its cases is asserting on the spelling this host happens to use" >&2
  echo "check-guard-path-spelling: rather than on the one the guard normalises to. It said:" >&2
  report_output
  failures=$((failures + 1))
fi

# The other guard that discovers files through pathlib and then compares what it found
# against forward-slash keys of its own. It reads fixtures rather than scripts, so it needs
# a root carrying both: its own copy of the guard, and the real fixture files at the paths
# the guard's manifest names them by.
readonly STORE_GUARD="check-store-fixtures.sh"
# One of the fixtures its manifest keys, spelled the way the Windows runner rendered it.
readonly FOREIGN_FIXTURE='crates\onetaskgraph-linear\tests\fixtures\issues.json'

fixture mkdir -p "$scratch/store/scripts"
fixture cp "$ROOT/scripts/$STORE_GUARD" "$scratch/store/scripts/$STORE_GUARD"
# A glob rather than `git ls-files`: this runs from a hook too, where git exports GIT_DIR
# and would answer about a repository other than the one being checked.
for fixture_file in "$ROOT"/crates/*/tests/fixtures/*.json; do
  [ -f "$fixture_file" ] || fatal \
    "found no fixture under $ROOT/crates/*/tests/fixtures/, so the store guard would refuse for want of input rather than for the spelling these cases are about" \
    "restore the fixtures, or teach scripts/$STORE_GUARD where they moved to, then rerun"
  relative="${fixture_file#"$ROOT"/}"
  fixture mkdir -p "$scratch/store/$(dirname "$relative")"
  fixture cp "$fixture_file" "$scratch/store/$relative"
done

run_store_native() {
  OUTPUT="$(bash "$scratch/store/scripts/$STORE_GUARD" 2>&1)" && STATUS=0 || STATUS=$?
}

run_store_foreign() {
  OUTPUT="$(PATH="$scratch/bin:$PATH" bash "$scratch/store/scripts/$STORE_GUARD" 2>&1)" \
    && STATUS=0 || STATUS=$?
}

# 6. The store guard on this host's own separator. The baseline: without it, case 7 would be
#    satisfied by a guard that is broken on every platform equally.
run_store_native
if [ "$STATUS" -ne 0 ]; then
  echo "check-guard-path-spelling: scripts/$STORE_GUARD refuses this repository's own fixtures" >&2
  echo "check-guard-path-spelling: on this host, so the cases below would be reading a failure" >&2
  echo "check-guard-path-spelling: that has nothing to do with path spelling. It said:" >&2
  report_output
  failures=$((failures + 1))
fi

# 7. The same guard, the same fixtures, on a host that spells paths with a backslash. This is
#    the case `check (windows-latest)` runs, and the one it failed.
run_store_foreign
if [ "$STATUS" -ne 0 ]; then
  echo "check-guard-path-spelling: scripts/$STORE_GUARD refuses this repository's own fixtures on" >&2
  echo "check-guard-path-spelling: a backslash-separator host while passing on this one, so what" >&2
  echo "check-guard-path-spelling: it compares depends on where it runs. It said:" >&2
  report_output
  failures=$((failures + 1))
fi

# 8. The control on the simulation, as case 3 is for the guard above: with the normalisation
#    removed the backslash spelling has to come back, which is both what the Windows runner
#    really printed and the evidence that `as_posix()` is what stops it.
substitute "$scratch/store/scripts/$STORE_GUARD" "path.as_posix()" "str(path)"
run_store_foreign
if [ "$STATUS" -eq 0 ]; then
  echo "check-guard-path-spelling: with the path normalisation removed, scripts/$STORE_GUARD still" >&2
  echo "check-guard-path-spelling: passed on a backslash-separator host — so the simulation is not" >&2
  echo "check-guard-path-spelling: reproducing that host and case 7 proves nothing." >&2
  failures=$((failures + 1))
elif ! grep -qF -- "$FOREIGN_FIXTURE" <<<"$OUTPUT"; then
  echo "check-guard-path-spelling: with the path normalisation removed, scripts/$STORE_GUARD refused" >&2
  echo "check-guard-path-spelling: on a backslash-separator host without naming '$FOREIGN_FIXTURE'," >&2
  echo "check-guard-path-spelling: so this case is no longer reproducing that failure. It said:" >&2
  report_output
  failures=$((failures + 1))
fi
fixture cp "$ROOT/scripts/$STORE_GUARD" "$scratch/store/scripts/$STORE_GUARD"

if [ "$failures" -ne 0 ]; then
  echo "check-guard-path-spelling: $failures case(s) failed." >&2
  echo "check-guard-path-spelling: repair the reporting in the guard the case names rather than" >&2
  echo "check-guard-path-spelling: relaxing the case: every path a guard renders goes through" >&2
  echo "check-guard-path-spelling: as_posix(), so that all three required lanes read — and compare" >&2
  echo "check-guard-path-spelling: against — the same spelling." >&2
  exit 1
fi
