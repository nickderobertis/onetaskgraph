#!/usr/bin/env bash
# Prove the live-lane selector rejects a successful Nx query that did not return projects.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

mkdir -p "$scratch/scripts"
cp "$ROOT/scripts/test-live.sh" "$scratch/scripts/test-live.sh"

failures=0
fail() {
  echo "check-test-live-boundary: $1" >&2
  failures=$((failures + 1))
}

run_case() {
  local name="$1" stdout="$2" stderr="$3" nx_status="${4:-0}" expected_status="${5:-2}"

  cat > "$scratch/scripts/nx.sh" <<'SH'
#!/usr/bin/env bash
printf '%s' "${FAKE_NX_STDOUT:-}"
printf '%s' "${FAKE_NX_STDERR:-}" >&2
exit "${FAKE_NX_STATUS:-0}"
SH
  chmod +x "$scratch/scripts/nx.sh"

  local diagnostic status
  diagnostic="$(
    FAKE_NX_STDOUT="$stdout" FAKE_NX_STDERR="$stderr" FAKE_NX_STATUS="$nx_status" \
      "$scratch/scripts/test-live.sh" workspace 2>&1
  )" && status=0 || status=$?

  if [ "$status" -ne "$expected_status" ]; then
    fail "$name output exited $status instead of $expected_status. Diagnostic: $diagnostic"
  fi
  case "$diagnostic" in
    *"'./scripts/nx.sh show projects --json'"*) ;;
    *) fail "$name output failed without naming the command. Diagnostic: $diagnostic" ;;
  esac
  case "$diagnostic" in
    *"test-live: stdout:"*"test-live: stderr:"*) ;;
    *) fail "$name output failed without reporting stdout and stderr separately. Diagnostic: $diagnostic" ;;
  esac
  if [ "$expected_status" -eq 1 ]; then
    case "$diagnostic" in
      *"run 'just bootstrap', then retry"*) ;;
      *) fail "$name output failed without a concrete bootstrap action. Diagnostic: $diagnostic" ;;
    esac
  else
    case "$diagnostic" in
      *"fix the workspace project listing, then retry"*) ;;
      *) fail "$name output failed without a concrete listing action. Diagnostic: $diagnostic" ;;
    esac
    case "$diagnostic" in
      *"test-live: validation:"*) ;;
      *) fail "$name output failed without its validation detail. Diagnostic: $diagnostic" ;;
    esac
  fi
  case "$diagnostic" in
    *Traceback*) fail "$name output leaked a Python traceback. Diagnostic: $diagnostic" ;;
  esac
  if [ -n "$stdout" ]; then
    case "$diagnostic" in
      *"$stdout"*) ;;
      *) fail "$name output was not replayed in the diagnostic. Diagnostic: $diagnostic" ;;
    esac
  else
    case "$diagnostic" in
      *"<empty>"*) ;;
      *) fail "$name output was not identified as empty. Diagnostic: $diagnostic" ;;
    esac
  fi
  if [ -n "$stderr" ]; then
    case "$diagnostic" in
      *"$stderr"*) ;;
      *) fail "$name stderr was not preserved for the diagnostic. Diagnostic: $diagnostic" ;;
    esac
  else
    case "$diagnostic" in
      *"test-live: stderr:"$'\n'"<empty>"*) ;;
      *) fail "$name stderr was not identified as empty. Diagnostic: $diagnostic" ;;
    esac
  fi
}

run_case "empty" "" "nx install notice"
run_case "unparseable" "not-json" "nx warning"
run_case "non-zero" "partial output" "nx failed" 9 1
run_case "non-zero-empty-stderr" "partial output" "" 9 1
run_case "wrong-shape" '{"workspace": true}' "nx warning"
run_case "empty-array" '[]' "nx warning"
run_case "non-string" '["workspace", 7]' "nx warning"
run_case "empty-name" '[""]' "nx warning"
run_case "multiline-name" '["workspace\nother"]' "nx warning"
run_case "comma-name" '["workspace,other"]' "nx warning"
run_case "control-name" '["workspace\u0001"]' "nx warning"
run_case "del-name" '["workspace\u007f"]' "nx warning"

# A valid listing must reach Nx, and a genuine target failure must remain a failure.
cat > "$scratch/scripts/nx.sh" <<'SH'
#!/usr/bin/env bash
if [ "${1:-}" = "show" ]; then
  printf '["workspace"]\n'
  echo "locked install completed" >&2
  exit 0
fi
echo "live journey failed" >&2
exit 17
SH
chmod +x "$scratch/scripts/nx.sh"
journey_diagnostic="$("$scratch/scripts/test-live.sh" workspace 2>&1)" \
  && journey_status=0 || journey_status=$?
if [ "$journey_status" -eq 0 ]; then
  fail "a genuine live journey failure was reported as success."
fi
case "$journey_diagnostic" in
  *"live journey failed"*) ;;
  *) fail "a genuine live journey failure was hidden. Diagnostic: $journey_diagnostic" ;;
esac
case "$journey_diagnostic" in
  *"locked install completed"*)
    fail "stderr from a successful project query leaked into the live target. Diagnostic: $journey_diagnostic"
    ;;
esac

# An unknown requested name must render known names literally, without splitting or globbing.
cat > "$scratch/scripts/nx.sh" <<'SH'
#!/usr/bin/env bash
printf '["alpha *", "beta space"]\n'
SH
chmod +x "$scratch/scripts/nx.sh"
unknown_diagnostic="$("$scratch/scripts/test-live.sh" missing 2>&1)" \
  && unknown_status=0 || unknown_status=$?
if [ "$unknown_status" -ne 3 ]; then
  fail "an unknown requested project exited $unknown_status instead of 3."
fi
if ! printf '%s\n' "$unknown_diagnostic" | grep -qxF '  alpha *' \
  || ! printf '%s\n' "$unknown_diagnostic" | grep -qxF '  beta space'; then
  fail "known project names were not rendered literally. Diagnostic: $unknown_diagnostic"
fi
case "$unknown_diagnostic" in
  *"rerun with one of the project names listed above"*) ;;
  *) fail "the unknown-project diagnostic gave no concrete next action: $unknown_diagnostic" ;;
esac

unsafe_diagnostic="$("$scratch/scripts/test-live.sh" $'bad\nname' 2>&1)" \
  && unsafe_status=0 || unsafe_status=$?
if [ "$unsafe_status" -ne 3 ]; then
  fail "an unsafe unknown project exited $unsafe_status instead of 3."
fi
case "$unsafe_diagnostic" in
  *"$'bad\\nname' is not a project"*) ;;
  *) fail "an unsafe unknown project was not shell-escaped. Diagnostic: $unsafe_diagnostic" ;;
esac

if [ "$failures" -ne 0 ]; then
  echo "check-test-live-boundary: $failures expectation(s) failed." >&2
  echo "check-test-live-boundary: fix scripts/test-live.sh, then rerun this check." >&2
  exit 1
fi
