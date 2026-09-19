#!/usr/bin/env bash
# Refuse to publish a crate whose workspace siblings would resolve, on the registry as it
# stands, to any version but the one this workspace releases them at.
#
# These pre-1.0 crates are lock-step: an exhaustive match or a struct literal in one is
# written against the others' exact release, so a consumer that takes one sibling a patch
# behind the rest compiles nothing — onetaskgraph-local-md 0.2.30 resolved
# onetaskgraph-plugin-api 0.2.32 and failed on an uncovered StatusCategory::Queued, with no
# lockfile error to point at the cause. The manifests pin every sibling exactly, and this is
# the check that the registry agrees before a crate is pushed where nothing can take it back.
#
# Cargo itself does the resolving, over the crate's own requirements as `cargo metadata`
# reports them, so what is judged is the answer a consumer would get rather than a second
# implementation of semver. The index is crates.io's unless ONETASKGRAPH_CRATES_INDEX_URL
# names another, which is how scripts/check-crate-sibling-resolution.sh supplies a registry
# answer of its own.
#
# Quiet on success. Exit 1 when a sibling resolves elsewhere or not at all, 64 when the call
# itself was wrong.
set -euo pipefail
usage() { echo "usage: scripts/crate-sibling-resolution.sh CRATE" >&2; echo "next: $1" >&2; exit 64; }
refuse() { echo "crate-sibling-resolution: $1" >&2; echo "crate-sibling-resolution: next: $2" >&2; exit 1; }
[[ $# -eq 1 ]] || usage "name the one workspace crate about to be published, as .github/workflows/release.yml does"
crate=$1
[[ $crate =~ ^[A-Za-z0-9][A-Za-z0-9_-]*$ ]] || usage "invalid crate name: $crate"
index=${ONETASKGRAPH_CRATES_INDEX_URL:-}
[[ -z $index || $index =~ ^sparse\+https?://[^[:space:]\"]+/$ ]] || usage \
  "ONETASKGRAPH_CRATES_INDEX_URL must be a sparse index URL ending in '/', such as sparse+https://index.crates.io/: $index"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
metadata="$(cd "$root" && cargo metadata --no-deps --format-version 1)" || refuse \
  "cargo metadata could not read this workspace" "fix the manifest error above and rerun"

scratch="$(mktemp -d)" || refuse "could not create a scratch dependent" "check the permissions of \$TMPDIR and rerun"
trap 'rm -rf "$scratch"' EXIT

# The dependent: one dependency per sibling the crate's consumers compile against — normal
# and build edges, never dev ones, which a consumer's build never resolves — each under the
# crate's own requirement. What each should resolve to goes beside it in `expected`.
printf '%s' "$metadata" > "$scratch/metadata.json" || refuse \
  "could not write the workspace metadata into $scratch" "check the permissions of \$TMPDIR and rerun"
if ! CRATE="$crate" SCRATCH="$scratch" INDEX="$index" python3 - <<'PY'
import json
import os
import pathlib

crate = os.environ["CRATE"]
scratch = pathlib.Path(os.environ["SCRATCH"])
index = os.environ["INDEX"]
metadata = json.loads((scratch / "metadata.json").read_text(encoding="utf-8"))
packages = {package["name"]: package for package in metadata["packages"]}
if crate not in packages:
    raise SystemExit(f"{crate} is not a package of this workspace")
siblings = {}
for dependency in packages[crate]["dependencies"]:
    if dependency["kind"] == "dev" or dependency["name"] not in packages:
        continue
    siblings[dependency["name"]] = dependency["req"]
lines = [
    "[package]",
    'name = "sibling-resolution"',
    'version = "0.0.0"',
    'edition = "2021"',
    "publish = false",
    "",
    "[workspace]",
    "",
    "[dependencies]",
]
for name, requirement in sorted(siblings.items()):
    lines.append(
        f"{json.dumps(name)} = {{ version = {json.dumps(requirement)}, default-features = false }}"
    )
(scratch / "src").mkdir()
(scratch / "src" / "lib.rs").write_text("")
(scratch / "Cargo.toml").write_text("\n".join(lines) + "\n")
if index:
    (scratch / ".cargo").mkdir()
    (scratch / ".cargo" / "config.toml").write_text(
        '[source.crates-io]\nreplace-with = "checked"\n\n'
        f"[source.checked]\nregistry = {json.dumps(index)}\n"
    )
with open(scratch / "expected", "w", encoding="utf-8", newline="\n") as expected:
    for name in sorted(siblings):
        expected.write(f"{name} {packages[name]['version']} {siblings[name]}\n")
PY
then
  refuse "could not read $crate's sibling requirements out of cargo metadata" \
    "name a package of this workspace, as the publish order in .github/workflows/release.yml does"
fi
[ -s "$scratch/expected" ] || exit 0

if ! resolution="$(cd "$scratch" && cargo generate-lockfile 2>&1)"; then
  printf '%s\n' "$resolution" >&2
  refuse "$crate's sibling requirements do not resolve on the registry (cargo's answer is above)" \
    "publish the siblings it names first, in the order .github/workflows/release.yml gives, or restore the version they were released at"
fi

# Every version the lockfile holds for each sibling, which is more than one when a sibling
# reaches another at a second version — exactly the split this check exists to stop.
if ! SCRATCH="$scratch" python3 - <<'PY'
import os
import pathlib
import tomllib

scratch = pathlib.Path(os.environ["SCRATCH"])
lock = tomllib.loads((scratch / "Cargo.lock").read_text(encoding="utf-8"))
resolved = {}
for package in lock.get("package", []):
    resolved.setdefault(package["name"], set()).add(package["version"])
with open(scratch / "drift", "w", encoding="utf-8", newline="\n") as drift:
    for line in (scratch / "expected").read_text(encoding="utf-8").splitlines():
        name, version, requirement = line.split(" ", 2)
        found = sorted(resolved.get(name, ()))
        if found != [version]:
            reached = ", ".join(found) or "nothing"
            drift.write(f"  {name}, required as {requirement!r}, resolves to {reached}, not {version}\n")
PY
then
  refuse "could not read the lockfile cargo resolved for $crate's siblings" "rerun; if it persists, read $scratch/Cargo.lock by hand"
fi
[ ! -s "$scratch/drift" ] || refuse "$crate would reach a sibling at a version it was not released with:
$(cat "$scratch/drift")" \
  "require every workspace sibling exactly, as '=<workspace version>' in the root Cargo.toml's [workspace.dependencies], and rerun the release"
