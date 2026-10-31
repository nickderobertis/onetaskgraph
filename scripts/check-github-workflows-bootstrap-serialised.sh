#!/usr/bin/env bash
# Watch the real `just github-workflows-bootstrap` serialise: scripts/ci-job.sh runs a job's
# phases side by side, and two of them can each reach github-workflows:bootstrap at once, so two
# of these recipes started together must never install at the same time. A stand-in `bun` on
# PATH records each install and says when one began while another was still running; the recipe,
# its script and the lock are the real ones. It also holds that both installs still run, that
# they install what the recipe says, and that a failing install fails the recipe.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
fail() {
  echo "check-github-workflows-bootstrap-serialised: $1" >&2
  echo "check-github-workflows-bootstrap-serialised: next: run github-workflows-bootstrap's install through scripts/github-workflows-bootstrap.sh's with_file_lock, then rerun 'bash scripts/check-github-workflows-bootstrap-serialised.sh'" >&2
  exit 1
}
command -v just >/dev/null 2>&1 || fail "just is not on PATH; bootstrap installs it"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/bin"
# Each install holds a marker directory for two seconds; mkdir is atomic, so a second install
# finding the marker already there began while the first was running.
cat >"$scratch/bin/bun" <<STUB
#!/usr/bin/env bash
record="$scratch/record"
echo "args: \$*" >>"\$record"
mkdir "$scratch/installing" 2>/dev/null || echo overlap >>"\$record"
sleep 2
rmdir "$scratch/installing" 2>/dev/null || true
echo done >>"\$record"
[ ! -f "$scratch/fail" ] || exit 7
STUB
chmod +x "$scratch/bin/bun"

run_recipe() {
  PATH="$scratch/bin:$PATH" just --justfile "$ROOT/justfile" --working-directory "$ROOT" \
    github-workflows-bootstrap >"$scratch/out.$1" 2>&1
}

run_recipe a & first=$!
run_recipe b & second=$!
wait "$first" || fail "the first of two concurrent recipes failed: $(cat "$scratch/out.a")"
wait "$second" || fail "the second of two concurrent recipes failed: $(cat "$scratch/out.b")"
! grep -qx overlap "$scratch/record" ||
  fail "two concurrent github-workflows-bootstrap recipes installed at the same time, the race that failed test-macos with EEXIST"
[ "$(grep -cx "done" "$scratch/record")" -eq 2 ] ||
  fail "two concurrent recipes did not both install: $(tr '\n' ' ' <"$scratch/record")"
[ "$(grep -cx "args: install --cwd $ROOT/.github --frozen-lockfile" "$scratch/record")" -eq 2 ] ||
  fail "the recipe did not run 'bun install --cwd <root>/.github --frozen-lockfile': $(tr '\n' ' ' <"$scratch/record")"

: >"$scratch/fail"
if run_recipe c; then
  fail "a failing install left the recipe succeeding"
fi
