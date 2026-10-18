#!/usr/bin/env bash
set -euo pipefail
trap 'echo "version update failed; next: fix the reported manifest or lock error and rerun scripts/set-version.sh <VERSION>" >&2' ERR

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
cargo_metadata() {
  local log
  log=$(mktemp)
  if cargo metadata "$@" >/dev/null 2>"$log"; then
    rm "$log"
  else
    cat "$log" >&2
    rm "$log"
    echo "cargo metadata could not validate the workspace; next: fix the manifest error above and rerun scripts/set-version.sh" >&2
    return 1
  fi
}
binary_manifest=crates/onetaskgraph/Cargo.toml
current=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$binary_manifest" | head -n1)
[[ $current =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || {
  echo "$binary_manifest has no valid semantic version; next: restore its X.Y.Z version and rerun" >&2; exit 2;
}

if [[ ${1:-} == --check ]]; then
  if [[ $# -ne 1 ]]; then
    echo "unexpected extra arguments after --check; next: pass --check by itself" >&2
    exit 2
  fi
  cargo_metadata --locked --format-version 1
  uv lock --project . --check --quiet
  uv lock --project sdks/python --check --quiet
  expected=$current
else
  if [[ $# -eq 0 ]]; then
    echo "usage: scripts/set-version.sh VERSION | --check; next: supply an X.Y.Z version or use --check" >&2
    exit 2
  fi
  if [[ $# -ne 1 ]]; then
    echo "unexpected extra arguments; next: pass exactly one X.Y.Z version or use --check" >&2
    exit 2
  fi
  expected=$1
  [[ $expected =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || {
    echo "invalid semantic version: $expected; expected X.Y.Z, then rerun scripts/set-version.sh $expected" >&2; exit 2;
  }
fi

fail=0
check_value() { [[ $1 == "$expected" ]] || { echo "$2 has $1; expected $expected" >&2; fail=1; }; }

if [[ ${1:-} == --check ]]; then
  python3 scripts/product_versions.py check "$expected" || fail=1
  for manifest in crates/*/Cargo.toml; do
    # An inherited version is [workspace.package]'s, which product_versions.py checked above.
    grep -qx 'version.workspace = true' "$manifest" && continue
    value=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$manifest" | head -n1)
    check_value "$value" "$manifest"
  done
  # Each sibling is pinned exactly, `=X.Y.Z`: these crates are released in lock step, and a
  # caret lets a consumer resolve one of them against another's newer, incompatible patch.
  while IFS= read -r value; do
    [[ $value == "=$expected" ]] || { echo "Cargo.toml path dependency has '$value'; expected the exact requirement '=$expected'" >&2; fail=1; }
  done < <(sed -n 's/.*path = "crates\/[^" ]*", version = "\([^"]*\)".*/\1/p' Cargo.toml)
  check_value "$(sed -n 's/^version = "\([^"]*\)"/\1/p' pyproject.toml | head -n1)" pyproject.toml
  check_value "$(sed -n 's/.*onetaskgraph-cli==\([^" ]*\).*/\1/p' sdks/python/pyproject.toml)" 'Python SDK CLI pin'
  node -e 'const fs=require("fs"); const v=process.argv[1]; for (const f of ["npm/cli/package.json",...fs.readdirSync("npm/platforms").map(x=>`npm/platforms/${x}/package.json`)]) { const p=JSON.parse(fs.readFileSync(f)); if(p.version!==v) throw Error(`${f} has ${p.version}; expected ${v}`); for(const [n,x] of Object.entries(p.optionalDependencies||{})) if(x!==v) throw Error(`${f} ${n} pin has ${x}; expected ${v}`) }' "$expected" || fail=1
  if [[ $fail -ne 0 ]]; then echo "version drift found; next: run scripts/set-version.sh $expected" >&2; fi
  exit "$fail"
fi

python3 scripts/product_versions.py set "$expected"
# Refresh workspace package versions without re-resolving unrelated dependencies.
cargo_metadata --format-version 1
# uv re-resolves every dependency when a workspace member's version moves, and with an empty
# cache and no network (UV_OFFLINE, as the release journeys run it) that fails although no
# dependency changed. Writing the members' own new version into uv.lock first leaves `uv
# lock` nothing to resolve, so it only confirms the lock is consistent.
python3 - "$expected" <<'PY'
import re
import sys
from pathlib import Path

lock = Path("uv.lock")
text = lock.read_text(encoding="utf-8")
member = re.compile(
    r'(\[\[package\]\]\nname = "[^"]+"\nversion = ")[^"]*("\nsource = \{ editable = "[^"]*" \}\n)'
)
lock.write_bytes(member.sub(lambda m: m.group(1) + sys.argv[1] + m.group(2), text).encode("utf-8"))
PY
uv lock --project . --quiet
uv lock --project sdks/python --quiet
