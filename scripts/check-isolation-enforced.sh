#!/usr/bin/env bash
# Prove that scripts/check-plugin-isolation.sh REFUSES a tree that breaks the rule.
#
# That guard passes on this repository, and it would pass just as quietly if its query
# were wrong, its match never fired, or its exit status were swallowed. A guard nobody has
# watched fail is a guard nobody knows works — and this one is the whole return on
# splitting `onetaskgraph-plugin-api` out of `onetaskgraph-core`, so it is the last one
# that should be taken on trust.
#
# So the tree is really broken, in a scratch clone, ten kinds of way — nine against the
# local guard, case 9 being a table with one row per field the guard reads, and one
# against deny.toml's wrapper restriction, which is the half of this rule that is a
# required check. Each case asserts on the DIAGNOSTIC as well as the exit
# status: a guard that refuses without naming the crate and the path sends the next
# author hunting, which is most of what the guard is for.
#
# It earned its place immediately. Case 3 — a plugin dev-depending on a crate that
# normally depends on the engine — passed the guard as originally written, because asking
# `cargo tree` one edge kind at a time stops following at the first edge of another kind.
# The guard now traverses the union of the kinds in one pass.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# A clone of the committed tree, so the fixtures are torn down by restoring from git and
# cannot leave a forbidden edge behind in anybody's working copy.
#
# Through scripts/scratch-clone.sh, which strips the git environment first. That matters
# more here than anywhere else in this repository: under a git hook GIT_DIR overrides
# `git -C`, so the `git add -A` below and the `git checkout -- .` between cases would
# stage and then DISCARD in the real working copy — a guard that exists to protect the
# tree, rewriting it. This escaped only because the other scratch-clone guard failed
# first and stopped the gate.
# shellcheck source=scripts/scratch-clone.sh
source "$ROOT/scripts/scratch-clone.sh"
scratch_clone "$ROOT" "$scratch/repo"

# Then overlay the working tree's tracked files, and stage them so `git checkout -- .`
# restores to THEM between cases. What is under test has to be the guard as it is right
# now: a clone of HEAD alone would test the last committed guard, so an author repairing
# it would watch this check keep failing against the version they just replaced.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$scratch/repo"
git -C "$scratch/repo" add -A

# Every fixture adds a path edge between crates already in the committed lock file, so
# nothing here needs the network — and a check that reached for it would fail on an
# offline runner for a reason that has nothing to do with what it tests.
export CARGO_NET_OFFLINE=true

failures=0
GUARD_OUTPUT=""
GUARD_STATUS=0

# Run the guard as it is committed, from inside the scratch tree, so what is under test is
# the real script rather than a copy of its logic.
run_guard() {
  GUARD_OUTPUT="$(cd "$scratch/repo" && bash scripts/check-plugin-isolation.sh 2>&1)" \
    && GUARD_STATUS=0 || GUARD_STATUS=$?
}

# Restore the committed tree, including Cargo.lock, and drop any crate a fixture added.
reset_fixture() {
  rm -rf "$scratch/repo/crates/onetaskgraph-bridge" \
    "$scratch/repo/crates/onetaskgraph-phantom"
  git -C "$scratch/repo" checkout --quiet -- .
}

# Add one dependency line to a crate's manifest, creating the section if it has none.
add_dependency() {
  local crate="$1" section="$2" line="$3"
  python3 - "$scratch/repo/crates/$crate/Cargo.toml" "$section" "$line" <<'PY'
import pathlib
import sys

path, section, line = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
text = path.read_text()
header = f"[{section}]\n"
if header not in text:
    text += f"\n{header}"
path.write_text(text.replace(header, f"{header}{line}\n", 1))
PY
}

report_guard_output() {
  printf '%s\n' "$GUARD_OUTPUT" | sed 's/^/    /' >&2
}

# The guard must refuse, and its diagnostic must name every term the next author needs in
# order to find the edge without re-deriving it.
expect_refused() {
  local case_name="$1"
  shift
  if [ "$GUARD_STATUS" -eq 0 ]; then
    echo "check-isolation-enforced: $case_name — check-plugin-isolation.sh PASSED a tree that" >&2
    echo "check-isolation-enforced: breaks the isolation rule. The guard is not enforcing it." >&2
    failures=$((failures + 1))
    return
  fi
  local term
  for term in "$@"; do
    # A here-string, not a pipe into `grep -q`: under `pipefail` the early exit of a
    # quiet grep SIGPIPEs its writer, so the pipeline's status can invert on a match.
    # That is the very defect these cases exist to catch — see check-plugin-isolation.sh.
    if ! grep -qF -- "$term" <<<"$GUARD_OUTPUT"; then
      echo "check-isolation-enforced: $case_name — the guard refused, but its diagnostic never" >&2
      echo "check-isolation-enforced: mentions '$term', so it does not say what to go and fix. It said:" >&2
      report_guard_output
      failures=$((failures + 1))
      return
    fi
  done
}

# 0. The control. Without it, a guard that refused every tree — including this one — would
#    satisfy every case below and look like the strongest check in the repository.
run_guard
if [ "$GUARD_STATUS" -ne 0 ]; then
  echo "check-isolation-enforced: the guard refuses the committed tree, so the cases below" >&2
  echo "check-isolation-enforced: would prove nothing. Run 'bash scripts/check-plugin-isolation.sh'" >&2
  echo "check-isolation-enforced: and fix what it reports first. It said:" >&2
  report_guard_output
  exit 1
fi

# 1. The ordinary way the rule breaks: someone wants one helper out of the engine.
add_dependency onetaskgraph-local-md dependencies 'onetaskgraph-core.workspace = true'
run_guard
expect_refused "a plugin declaring the engine as a dependency" \
  onetaskgraph-local-md onetaskgraph-core normal
reset_fixture

# 2. A dev-dependency is the same violation wearing a disguise — it still makes every
#    engine change mark that plugin affected, which is the cost the split exists to avoid.
add_dependency onetaskgraph-linear dev-dependencies 'onetaskgraph-core.workspace = true'
run_guard
expect_refused "a plugin declaring the engine as a dev-dependency" \
  onetaskgraph-linear onetaskgraph-core dev
reset_fixture

# 3. The indirect path, which no manifest scan can see: the plugin names an innocent crate
#    and that crate names the engine. Cargo permits the cycle because the plugin's edge is
#    a dev edge, so this is reachable in practice rather than a contrivance.
mkdir -p "$scratch/repo/crates/onetaskgraph-bridge/src"
cat > "$scratch/repo/crates/onetaskgraph-bridge/Cargo.toml" <<'EOF'
[package]
name = "onetaskgraph-bridge"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true

[dependencies]
onetaskgraph-core.workspace = true
EOF
: > "$scratch/repo/crates/onetaskgraph-bridge/src/lib.rs"
add_dependency onetaskgraph-local-md dev-dependencies \
  'onetaskgraph-bridge = { path = "../onetaskgraph-bridge" }'
run_guard
expect_refused "a plugin reaching the engine at depth two" \
  onetaskgraph-local-md onetaskgraph-bridge onetaskgraph-core
reset_fixture

# 4. The other half of the rule: the contract crate may depend on no crate of this
#    workspace, because everything else here depends on it.
add_dependency onetaskgraph-plugin-api dependencies 'onetaskgraph-core.workspace = true'
run_guard
expect_refused "the contract crate depending on another crate of this workspace" \
  onetaskgraph-plugin-api onetaskgraph-core
reset_fixture

# 5. The rule has a SECOND mechanism, and it is the one that is a required check, so it is
#    watched failing here too rather than only reasoned about: deny.toml permits the engine
#    exactly one wrapper. A dev edge is the case that reaches it — a plugin taking a normal
#    dependency on the engine is a Cargo cycle, which cargo refuses before cargo-deny sees
#    it, whereas Cargo permits a cycle through a dev edge. So the wrapper restriction is
#    what stands between a dev-dependency on the engine and a merge.
add_dependency onetaskgraph-linear dev-dependencies 'onetaskgraph-core.workspace = true'
# llmlint: ignore[work_goes_through_command_surface] `just deny` is the command surface
# for this repository, and it is the wrong tool here twice over: it runs the whole suite,
# including advisories, which reaches the advisory database over a network this check
# deliberately closes (CARGO_NET_OFFLINE above), and it would run against THIS repository
# rather than the scratch clone the fixture lives in. `bans` alone, in that clone, is what
# this case is about.
deny_output="$(cd "$scratch/repo" && cargo deny check bans 2>&1)" \
  && deny_status=0 || deny_status=$?
if [ "$deny_status" -eq 0 ]; then
  echo "check-isolation-enforced: deny.toml accepted a plugin that dev-depends on the engine." >&2
  echo "check-isolation-enforced: restore the wrappers entry for onetaskgraph-core in deny.toml —" >&2
  echo "check-isolation-enforced: it is the half of this rule that is a required check." >&2
  failures=$((failures + 1))
else
  for term in "error[banned]" onetaskgraph-core onetaskgraph-linear; do
    if ! grep -qF -- "$term" <<<"$deny_output"; then
      echo "check-isolation-enforced: cargo-deny refused, but not for the reason this case is" >&2
      echo "check-isolation-enforced: about — its output never mentions '$term'. It said:" >&2
      printf '%s\n' "$deny_output" | sed 's/^/    /' >&2
      failures=$((failures + 1))
      break
    fi
  done
fi
reset_fixture

# 6. The guard reads the plugin set from the layer:plugin tags and the dependency graph
#    from cargo, and those are two different sources. A name in one and not the other
#    means the rule cannot be checked for that crate at all — which must be a refusal,
#    because the alternative is a plugin that quietly stops being checked the moment its
#    project.json name drifts from the Cargo package name underneath it.
mkdir -p "$scratch/repo/crates/onetaskgraph-phantom/src"
cat > "$scratch/repo/crates/onetaskgraph-phantom/Cargo.toml" <<'EOF'
[package]
name = "onetaskgraph-shadow"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
EOF
: > "$scratch/repo/crates/onetaskgraph-phantom/src/lib.rs"
cat > "$scratch/repo/crates/onetaskgraph-phantom/project.json" <<'EOF'
{
  "name": "onetaskgraph-phantom",
  "projectType": "library",
  "tags": ["layer:plugin"]
}
EOF
run_guard
expect_refused "a layer:plugin crate that is no package of the workspace" \
  onetaskgraph-phantom layer:plugin
reset_fixture

# 7. The guard answers "at any depth" from the RESOLVED graph, and a workspace whose graph
#    does not resolve has no such answer to give. Cargo refuses to resolve a cycle whose
#    every edge is normal, and this fixture is one: the engine depends on every plugin, so
#    a plugin reaching the engine through a normal edge at depth two closes the ring. The
#    manifests alone see nothing wrong here — no plugin names the engine — so a guard that
#    treated an unresolvable graph as a clean one would pass this tree, which is precisely
#    a plugin reaching the engine at depth two.
mkdir -p "$scratch/repo/crates/onetaskgraph-bridge/src"
cat > "$scratch/repo/crates/onetaskgraph-bridge/Cargo.toml" <<'EOF'
[package]
name = "onetaskgraph-bridge"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true

[dependencies]
onetaskgraph-core.workspace = true
EOF
: > "$scratch/repo/crates/onetaskgraph-bridge/src/lib.rs"
add_dependency onetaskgraph-local-md dependencies \
  'onetaskgraph-bridge = { path = "../onetaskgraph-bridge" }'
run_guard
expect_refused "a workspace whose dependency graph does not resolve" \
  "does not resolve" onetaskgraph-local-md onetaskgraph-bridge onetaskgraph-core
reset_fixture

# 8. A manifest cargo cannot parse is the other way this guard can end up with nothing to
#    read. It must refuse and name the crate: the failure that matters is a guard that
#    treats "I could not look" as "I looked and it was clean".
printf '\nthis is not toml\n' >> "$scratch/repo/crates/onetaskgraph-local-md/Cargo.toml"
run_guard
expect_refused "a manifest cargo cannot parse" \
  "could not read the workspace manifests" onetaskgraph-local-md
reset_fixture

# 9. cargo's document is this guard's one input, and the guard reads named fields out of
#    it. Every shape those fields cannot be read from must be a refusal that names the
#    document, the format version and the field — not a Python traceback, and above all
#    not a pass. No manifest can produce such a shape while cargo honours
#    `--format-version 1`, so the boundary itself is what these cases replace: a cargo
#    earlier on PATH that answers `metadata` with a document of this table's choosing and
#    delegates everything else to the real one.
shim="$scratch/shim"
mkdir -p "$shim"
cat > "$shim/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = "metadata" ]; then
  printf '%s\n' "$SHIM_METADATA"
  exit 0
fi
exec "$REAL_CARGO" "$@"
EOF
chmod +x "$shim/cargo"

# One row per refusal the scan can emit: the document to hand over, and the message it
# must come back with.
shim_cases=(
  'not a document at all|is not JSON'
  '[]|holds a document that is not an object'
  '{"workspace_members": []}|holds a packages field that is not an array'
  '{"packages": {}, "workspace_members": []}|holds a packages field that is not an array'
  '{"packages": []}|holds a workspace_members field that is not an array'
  '{"packages": [1], "workspace_members": []}|holds a package that is not an object'
  '{"packages": [{"name": "a", "version": "1", "dependencies": []}], "workspace_members": []}|holds a package id that is not a string'
  '{"packages": [{"id": "a", "version": "1", "dependencies": []}], "workspace_members": []}|holds a package name that is not a string'
  '{"packages": [{"id": "a", "name": "a", "dependencies": []}], "workspace_members": []}|holds a package version that is not a string'
  '{"packages": [{"id": "a", "name": "a", "version": "1"}], "workspace_members": []}|holds a package dependencies field that is not an array'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": [1]}], "workspace_members": []}|holds a package dependency that is not an object'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": [{}]}], "workspace_members": []}|holds a package dependency name that is not a string'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": [{"name": "b", "kind": 1}]}], "workspace_members": []}|holds a package dependency kind that is not a string'
  '{"packages": [], "workspace_members": [1]}|holds a workspace member that is not a string'
  '{"packages": [], "workspace_members": ["ghost"]}|names a workspace member that is no package of the same document'
  '{"packages": [], "workspace_members": [], "resolve": []}|holds a resolve section that is not an object'
  '{"packages": [], "workspace_members": [], "resolve": {}}|holds a resolve nodes field that is not an array'
  '{"packages": [], "workspace_members": [], "resolve": {"nodes": [1]}}|holds a resolve node that is not an object'
  '{"packages": [], "workspace_members": [], "resolve": {"nodes": [{}]}}|holds a resolve node id that is not a string'
  '{"packages": [], "workspace_members": [], "resolve": {"nodes": [{"id": "a"}]}}|holds a resolve node deps field that is not an array'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": []}], "workspace_members": [], "resolve": {"nodes": [{"id": "a", "deps": [1]}]}}|holds a resolve dependency that is not an object'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": []}], "workspace_members": [], "resolve": {"nodes": [{"id": "a", "deps": [{}]}]}}|holds a resolve dependency pkg that is not a string'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": []}], "workspace_members": [], "resolve": {"nodes": [{"id": "a", "deps": [{"pkg": "ghost"}]}]}}|resolves a dependency on no package of the same document'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": []}], "workspace_members": [], "resolve": {"nodes": [{"id": "a", "deps": [{"pkg": "a", "dep_kinds": {}}]}]}}|holds a dep_kinds field that is not an array'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": []}], "workspace_members": [], "resolve": {"nodes": [{"id": "a", "deps": [{"pkg": "a", "dep_kinds": [1]}]}]}}|holds a dep_kinds entry that is not an object'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": []}], "workspace_members": [], "resolve": {"nodes": [{"id": "a", "deps": [{"pkg": "a", "dep_kinds": [{"kind": 1}]}]}]}}|holds a dep_kinds kind that is not a string'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": []}], "workspace_members": ["a"], "resolve": {"nodes": []}}|resolves no node for a workspace member'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": [{"name": "b", "kind": "weird"}]}], "workspace_members": []}|holds a dependency kind that cargo does not define'
  '{"packages": [{"id": "a", "name": "a", "version": "1", "dependencies": []}, {"id": "b", "name": "b", "version": "1", "dependencies": []}], "workspace_members": [], "resolve": {"nodes": [{"id": "a", "deps": [{"pkg": "b"}]}]}}|resolves a dependency on a package with no node of its own'
)

# A table that mirrors another file drifts, and a drifted table is a field nobody checks
# any more while this still reports ten green cases. So it is reconciled against the scan
# itself, both ways, before a single row runs: the messages the scan can emit are read out
# of its source, and a message with no row — or a row whose message the scan can no longer
# emit — fails here, by name.
shim_drift="$(
  printf '%s\n' "${shim_cases[@]##*|}" \
    | python3 -c '
import pathlib
import re
import sys

source = pathlib.Path(sys.argv[1]).read_text()
scan = source.split("readonly ISOLATION_SCAN=" + chr(39), 1)[1].split(chr(10) + chr(39) + chr(10), 1)[0]

SHAPES = {"mapping": "an object", "array": "an array", "text": "a string", "kind_of": "a string"}
emitted = set()
for match in re.finditer(r"refuse\(\"([^\"]+)\"\)", scan):
    emitted.add(match.group(1))
for match in re.finditer(r"refuse\(f\"([^\"{]+)\{", scan):
    emitted.add(match.group(1).rstrip(": "))
for helper, what in re.findall(r"\b(mapping|array|text|kind_of)\([^" + chr(10) + r"]*?, \"([^\"]+)\"\)", scan):
    emitted.add("holds " + what + " that is not " + SHAPES[helper])

covered = {line.strip() for line in sys.stdin if line.strip()}
for message in sorted(emitted - covered):
    print("the scan can refuse with \"" + message + "\", and no row reaches it")
for message in sorted(covered - emitted):
    print("a row expects \"" + message + "\", which the scan can no longer emit")
' "$scratch/repo/scripts/check-plugin-isolation.sh"
)"
if [ -n "$shim_drift" ]; then
  echo "check-isolation-enforced: the malformed-document table and the scan have drifted:" >&2
  printf '%s\n' "$shim_drift" | sed 's/^/    /' >&2
  echo "check-isolation-enforced: add the row, or drop it — a field the scan establishes" >&2
  echo "check-isolation-enforced: with no row is a field nobody has watched it refuse." >&2
  failures=$((failures + 1))
fi

for shim_case in "${shim_cases[@]}"; do
  GUARD_OUTPUT="$(cd "$scratch/repo" \
    && PATH="$shim:$PATH" REAL_CARGO="$(command -v cargo)" \
       SHIM_METADATA="${shim_case%%|*}" bash scripts/check-plugin-isolation.sh 2>&1)" \
    && GUARD_STATUS=0 || GUARD_STATUS=$?
  expect_refused "cargo handing over a document that ${shim_case##*|}" \
    "could not read the document" "--format-version 1" "${shim_case##*|}"
done
rm -rf "$shim"
reset_fixture

# 10. The plugin set is the guard's other input, and it arrives from another script. A
#     producer that failed must not read as a workspace with no plugins in it: an empty
#     set passes every check below while checking no crate at all, which is the quietest
#     way this guard can stop working. Two ways it fails to arrive, and the second is the
#     one a status check alone would miss — a producer that succeeds and prints nothing
#     looks exactly like a workspace with no plugins in it.
rm -f "$scratch/repo/scripts/plugin-crates.sh"
run_guard
expect_refused "the plugin set producer failing outright" \
  "could not read the plugin set" plugin-crates.sh
reset_fixture

cat > "$scratch/repo/scripts/plugin-crates.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
run_guard
expect_refused "the plugin set arriving empty" \
  "could not read the plugin set" plugin-crates.sh
reset_fixture

if [ "$failures" -ne 0 ]; then
  echo "check-isolation-enforced: $failures case(s) failed." >&2
  echo "check-isolation-enforced: a violation of the engine-isolation rule now gets through the" >&2
  echo "check-isolation-enforced: mechanism each case above names. Repair that mechanism rather" >&2
  echo "check-isolation-enforced: than relaxing the case: the two cover different moments, so a" >&2
  echo "check-isolation-enforced: hole in either is a hole in the rule AGENTS.md states." >&2
  exit 1
fi
