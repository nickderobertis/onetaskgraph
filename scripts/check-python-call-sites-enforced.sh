#!/usr/bin/env bash
# Watch scripts/check-relative-interpreter.sh refuse a targeted pass that misses a call site.
#
# test-macos and test-windows put the distribution journey's Python call sites to a relative
# interpreter through scripts/test-python-call-sites.sh alone, so what keeps that pass whole
# is the comparison test-linux makes after running the journey itself. A comparison nobody
# has watched fail is one nobody knows works, so this runs the real check, the real targeted
# pass and the real recording in a scratch copy, over a stand-in journey that starts Python
# from a real script the way the journey does: once as it stands, which must pass, and once
# with that script's line deleted from the targeted pass, which must fail naming it.
set -euo pipefail

fail() {
  echo "check-python-call-sites-enforced: $1" >&2
  echo "check-python-call-sites-enforced: next: $2" >&2
  exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[ -r "$root/scripts/scratch-clone.sh" ] || fail "scripts/scratch-clone.sh is missing" \
  "restore it with 'git checkout -- scripts/scratch-clone.sh', then rerun"
# shellcheck source=scripts/scratch-clone.sh
source "$root/scripts/scratch-clone.sh"
binary=target/debug/onetaskgraph
# Keep the scratch executable's real suffix: Git Bash also finds .exe without spelling it.
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) binary="$binary.exe" ;;
esac
[ -f "$root/$binary" ] || fail "no $binary, which the targeted pass packages" \
  "run 'scripts/nx.sh run onetaskgraph:build', then rerun"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
repo="$tmp/repo"
scratch_clone "$root" "$repo" || fail "could not clone this repository into $repo" "check 'df -h' for free space, then rerun"
copy_tracked_files "$root" "$repo" || fail "could not copy the working tree into $repo" "check 'df -h' for free space, then rerun"
mkdir -p "$repo/target/debug" && cp "$root/$binary" "$repo/$binary" \
  || fail "could not copy $binary into $repo" "check 'df -h' for free space, then rerun"

# The stand-in journey: starts Python itself, and runs the real workspace check from the root.
cat >"$repo/scripts/test-distribution.sh" <<'JOURNEY'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
[ -f target/debug/onetaskgraph ] || [ -f target/debug/onetaskgraph.exe ]
python_bin="$(command -v python3)"
case $python_bin in /*) ;; *) python_bin="$(cd "$(dirname "$python_bin")" && pwd)/$(basename "$python_bin")" ;; esac
scratch="$(mktemp -d)"
(cd "$scratch" && "$python_bin" -c 'pass')
rmdir "$scratch"
[ "$*" = --interpreter-only ] && exit 0
bash scripts/check-workspace-config.sh
JOURNEY
git -C "$repo" add -A >/dev/null || fail "could not stage the scratch copy in $repo" "check 'df -h', then rerun"

run_check() {
  (cd "$repo" && bash scripts/check-relative-interpreter.sh scripts/test-distribution.sh) >"$tmp/out" 2>&1 \
    && status=0 || status=$?
}

run_check
[ "$status" -eq 0 ] || fail "the comparison refused a targeted pass that reaches every site the journey does (exit $status): $(cat "$tmp/out")" \
  "run 'bash scripts/check-relative-interpreter.sh scripts/test-distribution.sh' and fix what it reports"

# Reproduce Git Bash's extensionless file lookup while Python still opens literal paths.
# Only the host's uname and file lookup differ; the targeted pass and packaging stay real.
if [ "$binary" = target/debug/onetaskgraph ]; then
  mv "$repo/$binary" "$repo/$binary.exe"
  mkdir "$tmp/windows-host"
  real_uname="$(command -v uname)"
  printf '#!/bin/sh\nif [ "$1" = -s ]; then echo MINGW64_NT; else exec "%s" "$@"; fi\n' "$real_uname" > "$tmp/windows-host/uname"
  chmod +x "$tmp/windows-host/uname"
  cat > "$tmp/windows-host/bash-env" <<'HOST'
function [() {
  if builtin [ "$#" -eq 3 ] && builtin [ "$1" = -f ]; then
    case "$2" in
      */target/debug/onetaskgraph | target/debug/onetaskgraph)
        builtin [ -f "$2.exe" ] && return 0 ;;
    esac
  fi
  builtin [ "$@"
}
HOST
  (cd "$repo" && PATH="$tmp/windows-host:$PATH" BASH_ENV="$tmp/windows-host/bash-env" \
    bash scripts/test-python-call-sites.sh) > "$tmp/windows-out" 2>&1 \
    && status=0 || status=$?
  [ "$status" -eq 0 ] || fail "the targeted pass could not package the explicit .exe under Git Bash file lookup: $(cat "$tmp/windows-out")" \
    "spell the executable's .exe suffix before handing its path to Python"
  cp "$repo/scripts/test-python-call-sites.sh" "$tmp/targeted-with-suffix"
  sed 's/binary="$binary.exe"/:/' "$tmp/targeted-with-suffix" > "$repo/scripts/test-python-call-sites.sh"
  (cd "$repo" && PATH="$tmp/windows-host:$PATH" BASH_ENV="$tmp/windows-host/bash-env" \
    bash scripts/test-python-call-sites.sh) > "$tmp/windows-missing-suffix" 2>&1 \
    && status=0 || status=$?
  [ "$status" -ne 0 ] && grep -q 'No such file or directory.*onetaskgraph' "$tmp/windows-missing-suffix" \
    || fail "the simulated host did not expose Python refusing an extensionless .exe path: $(cat "$tmp/windows-missing-suffix")" \
      "keep the Git Bash file lookup simulation and literal Python file read in this journey"
  cp "$tmp/targeted-with-suffix" "$repo/scripts/test-python-call-sites.sh"
  mv "$repo/$binary.exe" "$repo/$binary"
fi

grep -v 'scripts/check-workspace-config.sh"' "$repo/scripts/test-python-call-sites.sh" >"$tmp/targeted"
cmp -s "$tmp/targeted" "$repo/scripts/test-python-call-sites.sh" \
  && fail "found no scripts/check-workspace-config.sh site in scripts/test-python-call-sites.sh to delete" \
    "update this check to delete the line that runs a script the journey starts Python from"
cp "$tmp/targeted" "$repo/scripts/test-python-call-sites.sh"
git -C "$repo" add -A >/dev/null || fail "could not stage the scratch copy in $repo" "check 'df -h', then rerun"
run_check
[ "$status" -ne 0 ] || fail "the comparison accepted a targeted pass missing scripts/check-workspace-config.sh: $(cat "$tmp/out")" \
  "restore the comparison at the end of scripts/check-relative-interpreter.sh"
grep -q 'did not reach:' "$tmp/out" && grep -q '    scripts/check-workspace-config.sh (running stdin ' "$tmp/out" \
  || fail "the comparison refused the missed site without naming it and its program: $(cat "$tmp/out")" \
    "make scripts/check-relative-interpreter.sh name each script it missed and what that script ran"

# Missing evidence must refuse the interpreter call itself. A directory at the record's
# path forces a real append error even on hosts running the tests as root.
mkdir -p "$tmp/unwritable/call-sites.failure"
printf '%s\n' failure >"$tmp/unwritable/current"
bash -c '. "$1" "$2" -c pass' _ "$root/scripts/python-call-site.sh" "$tmp/unwritable" >"$tmp/record-error" 2>&1 \
  && status=0 || status=$?
[ "$status" -ne 0 ] || fail "the recorder accepted an unwritable call-site record" \
  "make scripts/python-call-site.sh refuse recording errors"
grep -q 'could not append the Python call site' "$tmp/record-error" \
  && grep -q 'next: check free space and write permissions' "$tmp/record-error" \
  || fail "the recorder refused without the failed operation and repair: $(cat "$tmp/record-error")" \
    "name the recording operation and how to repair its destination"

# The recorder names the program past the interpreter's options, through a probe shim built as
# scripts/check-relative-interpreter.sh builds it, and the real interpreter still runs after it:
# a module, and a script behind -W and -X. A lone -W or -X is recorded and left to the
# interpreter, which refuses it, rather than looping on a shift that cannot consume it.
probe="$tmp/options"
mkdir -p "$probe/bin" || fail "could not create $probe" "check 'df -h', then rerun"
printf '%s\n' options >"$probe/current"
real_python3="$(command -v python3)" || fail "python3 is not on PATH" "install python3 and rerun"
case $real_python3 in
  /* | ?:[\\/]*) ;;
  *) real_python3="$(cd "$(dirname "$real_python3")" && pwd)/$(basename "$real_python3")" ;;
esac
printf '#!/usr/bin/env bash\n. "%s" "%s" "$@"\nexec "%s" "$@"\n' \
  "$root/scripts/python-call-site.sh" "$probe" "$real_python3" >"$probe/bin/python3"
chmod +x "$probe/bin/python3"
mkdir -p "$tmp/sites/scripts"
# It prints what the options delivered, which no interpreter default decides.
printf '%s\n' 'import sys' 'print("ran", sys.warnoptions, sorted(sys._xoptions))' >"$tmp/sites/scripts/site.py"
caller="$tmp/sites/scripts/caller.sh"
printf '%s\n' '#!/usr/bin/env bash' '"$@"' >"$caller"
ran() {
  local out
  out="$(cd "$tmp/sites" && bash "$caller" "$probe/bin/python3" "$@" 2>&1 | tr -d '\r')" \
    || fail "the probe interpreter failed for '$*': $out" "make scripts/python-call-site.sh hand every invocation on"
  printf '%s' "$out"
}
[ "$(ran -m json.tool <<<'{}')" = "{}" ] || fail "-m json.tool did not run through the recorder" "keep stdin and the arguments intact"
[ "$(ran -W ignore scripts/site.py)" = "ran ['ignore'] []" ] || fail "-W ignore scripts/site.py did not run through the recorder" "keep the arguments intact"
[ "$(ran -X utf8 scripts/site.py)" = "ran [] ['utf8']" ] || fail "-X utf8 scripts/site.py did not run through the recorder" "keep the arguments intact"
# The lone options run as this script's own child, by a process id it holds, so a recorder
# that loops is stopped here after a minute rather than by a `timeout` macOS does not ship.
for lone in -W -X; do
  (cd "$tmp/sites" && exec "$probe/bin/python3" "$lone") >"$tmp/lone" 2>&1 &
  lone_pid=$!
  waited=0
  while kill -0 "$lone_pid" 2>/dev/null && [ "$waited" -lt 60 ]; do
    sleep 1
    waited=$((waited + 1))
  done
  if kill -0 "$lone_pid" 2>/dev/null; then
    kill "$lone_pid" 2>/dev/null || true
    wait "$lone_pid" 2>/dev/null || true
    fail "a lone $lone hung the recorder" "record an option given no argument and stop parsing"
  fi
  status=0
  wait "$lone_pid" || status=$?
  [ "$status" -ne 0 ] || fail "a lone $lone was not refused by the interpreter" "hand the invocation on unchanged"
done
expected="$(printf 'scripts/caller.sh\t%s\n' '-m json.tool' scripts/site.py scripts/site.py
printf 'scripts/check-python-call-sites-enforced.sh\t%s\n' '-W without its argument' '-X without its argument')"
[ "$(tr -d '\r' <"$probe/call-sites.options")" = "$expected" ] \
  || fail "the recorder named these call sites: $(cat "$probe/call-sites.options")" \
    "record the caller and the program past -W and -X, and -m with its module"
