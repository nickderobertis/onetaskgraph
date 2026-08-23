#!/usr/bin/env bash
# Prove that scripts/check-plugin-isolation.sh REFUSES a tree that breaks the rule.
#
# That guard passes on this repository, and it would pass just as quietly if its query
# were wrong, its match never fired, or its exit status were swallowed. A guard nobody has
# watched fail is a guard nobody knows works — and this one is the whole return on
# splitting `onetaskgraph-plugin-api` out of `onetaskgraph-core`, so it is the last one
# that should be taken on trust.
#
# So the forbidden edge is introduced for real, in a scratch clone, five ways — four
# against the local guard and one against deny.toml's wrapper restriction, which is the
# half of this rule that is a required check. Each case asserts on the DIAGNOSTIC as well
# as the exit status: a guard that refuses without naming the crate and the path sends the
# next author hunting, which is most of what the guard is for.
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
git clone --quiet --shared --no-checkout "$ROOT" "$scratch/repo"
git -C "$scratch/repo" checkout --quiet "$(git -C "$ROOT" rev-parse HEAD)"

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
  rm -rf "$scratch/repo/crates/onetaskgraph-bridge"
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
#    satisfy all four cases below and look like the strongest check in the repository.
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

if [ "$failures" -ne 0 ]; then
  echo "check-isolation-enforced: $failures case(s) failed." >&2
  echo "check-isolation-enforced: a violation of the engine-isolation rule now gets through the" >&2
  echo "check-isolation-enforced: mechanism each case above names. Repair that mechanism rather" >&2
  echo "check-isolation-enforced: than relaxing the case: the two cover different moments, so a" >&2
  echo "check-isolation-enforced: hole in either is a hole in the rule AGENTS.md states." >&2
  exit 1
fi
