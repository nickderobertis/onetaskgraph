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
[ -f "$root/$binary" ] || binary="$binary.exe"
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
