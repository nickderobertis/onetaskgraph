#!/usr/bin/env bash
# Print, one command per line, every build test-linux makes of this workspace's Rust units,
# read from where each is declared rather than restated: each crate's `test` target's cargo
# commands from its project.json, with --no-run; the instrumented run scripts/rust-coverage.sh
# makes of each crate, with a filter selecting no test (cargo-llvm-cov refuses --no-report
# beside --no-run); and the distribution journey's release build.
#
#   scripts/pr-build-commands.sh
#
# scripts/check-warm-rust-cache.sh holds scripts/warm-rust-cache.sh to these shapes, and
# scripts/check-warm-rust-cache-reuse.sh runs them over what the warmer built.
set -euo pipefail

fail() {
  echo "pr-build-commands: $1" >&2
  echo "pr-build-commands: next: $2" >&2
  exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
python_bin="$(command -v python3)" || fail "python3 is not on PATH" "install python3 and re-run"
case $python_bin in
  /* | ?:[\\/]*) ;;
  *) python_bin=$(cd "$(dirname "$python_bin")" && pwd)/$(basename "$python_bin") ;;
esac

"$python_bin" - <<'PY' | tr -d '\r'
import glob, json, re, sys

out = []
for path in sorted(glob.glob("crates/*/project.json")):
    options = json.load(open(path, encoding="utf-8")).get("targets", {}).get("test", {}).get("options", {})
    for command in options.get("commands") or [options.get("command", "")]:
        command = command if isinstance(command, str) else command.get("command", "")
        if command.startswith("cargo test "):
            out.append(command.replace(" --quiet", "") + " --no-run")
coverage = open("scripts/rust-coverage.sh", encoding="utf-8").read()
run = re.search(r'run="\$\((cargo llvm-cov\b.*?) 2>&1\)"', coverage, re.S)
if not run or '"$CRATE"' not in run.group(1):
    sys.exit("pr-build-commands: scripts/rust-coverage.sh has no `run=\"$(cargo llvm-cov ... \"$CRATE\" ... 2>&1)\"`; next: point this at its instrumented run")
instrumented = " ".join(run.group(1).replace("\\\n", " ").split())
crates = sorted({m for c in out for m in re.findall(r"(?:-p|--package) (\S+)", c)})
if not crates:
    sys.exit("pr-build-commands: no crate's test target runs `cargo test`; next: restore the crates' test targets")
for crate in crates:
    out.append(instrumented.replace('"$CRATE"', crate) + " -E none() --no-tests=pass")
out.append("bash scripts/test-distribution.sh --release-build-only")
# llmlint: ignore[tool_output_is_signal] Standard output is this script's machine-read interface: the two checks that call it read one build command per line.
print("\n".join(out))
PY
