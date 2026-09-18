#!/usr/bin/env bash
# Drive scripts/crate-sibling-resolution.sh — the step publish-crates runs before each
# `cargo publish` — against a registry answer this check supplies, and prove it refuses.
#
# The registry is a sparse index of static files served on loopback, holding every crate of
# this workspace at the version the tree releases AND at the patch after it: the newest
# registry graph a release meets once any later patch exists, which is what a caret resolves
# against. Real cargo does the resolving. Nothing here reaches crates.io, and CARGO_HOME is a
# scratch directory, so the index this writes cache for is never the user's.
set -euo pipefail

fatal() {
  echo "check-crate-sibling-resolution: $1" >&2
  echo "check-crate-sibling-resolution: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-check' does"
for tool in cargo git python3; do
  command -v "$tool" >/dev/null 2>&1 || fatal "$tool is not on PATH" "run 'just bootstrap', then rerun"
done

scratch="$(mktemp -d)" || fatal "could not create a scratch directory" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
server_pid=
cleanup() {
  if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
  rm -rf "$scratch"
}
trap cleanup EXIT

# The path below is built from $ROOT at run time, so ShellCheck cannot follow it.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore that file with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
# Git exports GIT_DIR to hooks and it overrides `git -C`; the gate runs from pre-push.
scratch_clone_strip_git_env

# The WORKING tree's tracked files, so what is under test is the manifests and the script as
# they are now, and the cases below can break a copy rather than the checkout.
mkdir -p "$scratch/repo" || fatal "could not create $scratch/repo" "check \$TMPDIR permissions, then rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$scratch/repo" || fatal \
  "could not copy $ROOT's tracked files into $scratch/repo" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"
cd "$scratch/repo"

metadata="$(cargo metadata --no-deps --format-version 1)" || fatal \
  "cargo metadata could not read the tree under test" "fix the manifest error above and rerun"

# The registry answer: config.json, and one index file per workspace crate with two
# releases, the tree's version and the patch after it. A crate's own dependencies are not
# the subject here, so each release declares none and the answer stays this small.
mkdir -p "$scratch/registry" || fatal "could not create the registry directory" "check \$TMPDIR permissions, then rerun"
printf '%s' "$metadata" > "$scratch/metadata.json" || fatal \
  "could not write the workspace metadata" "check \$TMPDIR permissions, then rerun"
if ! SCRATCH="$scratch" python3 - <<'PY'
import json
import os
import pathlib

scratch = pathlib.Path(os.environ["SCRATCH"])
registry = scratch / "registry"
(registry / "config.json").write_text(json.dumps({"dl": "http://127.0.0.1/unused"}))
metadata = json.loads((scratch / "metadata.json").read_text(encoding="utf-8"))
for package in metadata["packages"]:
    name, version = package["name"], package["version"]
    major, minor, patch = (int(part) for part in version.split("."))
    path = registry / name[:2] / name[2:4] / name
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as index:
        for release in (version, f"{major}.{minor}.{patch + 1}"):
            entry = {"name": name, "vers": release, "deps": [], "cksum": "0" * 64,
                     "features": {}, "yanked": False}
            index.write(json.dumps(entry) + "\n")
PY
then
  fatal "could not write the registry answer" "check \$TMPDIR permissions and the workspace version, then rerun"
fi
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' crates/onetaskgraph/Cargo.toml | head -n1)"
[[ $version =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || fatal \
  "crates/onetaskgraph/Cargo.toml has no plain X.Y.Z version ('$version')" "restore that manifest's version and rerun"
newer="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.$((BASH_REMATCH[3] + 1))"

# Served over loopback on a port the kernel picks. This is the one process this check
# starts, so it is the one it stops.
python3 -u -m http.server 0 --bind 127.0.0.1 --directory "$scratch/registry" > "$scratch/server.log" 2>&1 &
server_pid=$!
port=
for _ in $(seq 1 100); do
  port="$(sed -n 's/.* port \([0-9][0-9]*\).*/\1/p' "$scratch/server.log" | head -n1)"
  [ -n "$port" ] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.1
done
[ -n "$port" ] || fatal "the loopback registry never reported its port: $(cat "$scratch/server.log")" \
  "check that python3 can bind 127.0.0.1, then rerun"
export ONETASKGRAPH_CRATES_INDEX_URL="sparse+http://127.0.0.1:$port/"
export CARGO_HOME="$scratch/cargo-home"
export CARGO_NET_OFFLINE=false

failures=0
OUTPUT=""
STATUS=0
run_check() {
  OUTPUT="$(bash scripts/crate-sibling-resolution.sh "$1" 2>&1)" && STATUS=0 || STATUS=$?
}
expect_refused() {
  local label="$1" want_status="$2" want="$3"
  if [ "$STATUS" -ne "$want_status" ]; then
    echo "check-crate-sibling-resolution: FAILED ($label): exited $STATUS, expected $want_status. It said:" >&2
    printf '%s\n' "$OUTPUT" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  elif ! grep -Fq -- "$want" <<< "$OUTPUT"; then
    echo "check-crate-sibling-resolution: FAILED ($label): refused without naming '$want'. It said:" >&2
    printf '%s\n' "$OUTPUT" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  fi
}

# 1. The tree as it stands: every published crate, in the order the release job publishes
#    them, resolves each sibling to the version it is released with although a newer patch
#    of every one of them is on the registry.
order="$(sed -n 's/^ *for crate in \(onetaskgraph[a-z -]*\); do$/\1/p' .github/workflows/release.yml)"
[ -n "$order" ] || fatal "could not read the publish order out of .github/workflows/release.yml" \
  "keep publish-crates' 'for crate in …; do' loop on one line, which is what this check reads"
for crate in $order; do
  run_check "$crate"
  if [ "$STATUS" -ne 0 ] || [ -n "$OUTPUT" ]; then
    echo "check-crate-sibling-resolution: FAILED ($crate over the tree as it stands): exited $STATUS. It said:" >&2
    printf '%s\n' "$OUTPUT" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  fi
done

# 2. The defect itself: a caret requirement on a sibling. The registry's newest patch
#    satisfies it, so a consumer would take a plugin-api the crate was never built with.
cp Cargo.toml "$scratch/Cargo.toml.orig"
perl -pi -e 's/^(onetaskgraph-plugin-api = \{[^\n]*version = ")=/$1/' Cargo.toml
grep -q "^onetaskgraph-plugin-api = .*version = \"$version\"" Cargo.toml || fatal \
  "could not put a caret requirement on onetaskgraph-plugin-api in the scratch tree" \
  "keep the root Cargo.toml's plugin-api entry on one line, which is what this case edits"
run_check onetaskgraph-local-md
expect_refused "a caret requirement meeting a newer patch" 1 \
  "onetaskgraph-plugin-api, required as '^$version', resolves to $newer, not $version"
cp "$scratch/Cargo.toml.orig" Cargo.toml

# 3. An exact requirement on a release the registry does not hold: the sibling was not
#    published first, and the crate cannot be resolved by anyone.
index="$scratch/registry/on/et/onetaskgraph-plugin-api"
cp "$index" "$scratch/plugin-api.index.orig"
grep -v "\"vers\": \"$version\"" "$scratch/plugin-api.index.orig" > "$index" || true
run_check onetaskgraph-local-md
expect_refused "a sibling absent from the registry" 1 "do not resolve on the registry"
cp "$scratch/plugin-api.index.orig" "$index"

# 4. A call the script cannot act on is refused as a call, not as a resolution.
ONETASKGRAPH_CRATES_INDEX_URL="https://index.crates.io" run_check onetaskgraph-local-md
expect_refused "an index URL that is not a sparse one" 64 "must be a sparse index URL"
run_check onetaskgraph-not-a-crate
expect_refused "a crate this workspace does not have" 1 "could not read onetaskgraph-not-a-crate's sibling requirements"

if [ "$failures" -ne 0 ]; then
  echo "check-crate-sibling-resolution: $failures case(s) failed; scripts/crate-sibling-resolution.sh no longer stops a crate whose siblings resolve elsewhere" >&2
  echo "check-crate-sibling-resolution: next: repair that script so each case above is refused as described" >&2
  exit 1
fi
