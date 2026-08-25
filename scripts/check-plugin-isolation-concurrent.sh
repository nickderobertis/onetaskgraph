#!/usr/bin/env bash
# Exercise the isolation guard in the concurrent shape Nx's full-project sweep creates.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
CARGO_BINARY="$(command -v cargo)"
readonly CARGO_BINARY
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin" "$scratch/calls"

cat > "$scratch/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
call="$(mktemp -p "$CARGO_CALLS" call.XXXXXX)"
: > "$call"
echo "    Blocking waiting for file lock on package cache" >&2
if [ "${CARGO_TEST_INVALID:-0}" = 1 ]; then
  printf '%s' 'not-json-from-cargo'
  exit 0
fi
exec "$REAL_CARGO" "$@"
EOF
chmod +x "$scratch/bin/cargo"

failures=0
pids=()
for run in 1 2 3 4; do
  (
    cd "$ROOT"
    PATH="$scratch/bin:$PATH" CARGO_CALLS="$scratch/calls" REAL_CARGO="$CARGO_BINARY" \
      bash scripts/check-plugin-isolation.sh >"$scratch/run.$run.log" 2>&1
  ) &
  pids+=("$!")
done

for index in "${!pids[@]}"; do
  if ! wait "${pids[$index]}"; then
    run=$((index + 1))
    echo "check-plugin-isolation-concurrent: concurrent guard $run failed:" >&2
    sed 's/^/    /' "$scratch/run.$run.log" >&2
    failures=$((failures + 1))
  fi
done

calls="$(find "$scratch/calls" -type f | wc -l | tr -d ' ')"
if [ "$calls" -ne 4 ]; then
  echo "check-plugin-isolation-concurrent: four guard runs made $calls cargo calls; each guard" >&2
  echo "check-plugin-isolation-concurrent: must read the graph exactly once." >&2
  failures=$((failures + 1))
fi

invalid_output="$(
  cd "$ROOT"
  PATH="$scratch/bin:$PATH" CARGO_CALLS="$scratch/calls" REAL_CARGO="$CARGO_BINARY" \
    CARGO_TEST_INVALID=1 bash scripts/check-plugin-isolation.sh 2>&1
)" && invalid_status=0 || invalid_status=$?
if [ "$invalid_status" -eq 0 ]; then
  echo "check-plugin-isolation-concurrent: the guard passed non-JSON cargo output." >&2
  failures=$((failures + 1))
elif ! grep -qF 'not-json-from-cargo' <<<"$invalid_output"; then
  echo "check-plugin-isolation-concurrent: the guard refused non-JSON cargo output but did not" >&2
  echo "check-plugin-isolation-concurrent: report what it received. It said:" >&2
  printf '%s\n' "$invalid_output" | sed 's/^/    /' >&2
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  echo "check-plugin-isolation-concurrent: $failures expectation(s) failed." >&2
  exit 1
fi
