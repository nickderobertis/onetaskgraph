#!/usr/bin/env bash
# Drive .githooks/pre-push in a worktree that carries no installed dependencies.
#
# The hook runs the complete gate, and a gate that meets an unprovisioned worktree fails on
# an environment problem while wearing the words of a push rejection: the person reads "the
# gate failed", goes looking at their change, and finds nothing wrong with it — because
# nothing about it was ever checked. Every publication from one orchestration host was
# refused for twenty-nine minutes on exactly that.
#
# So the hook owes one of two things in a fresh worktree, and this drives the real hook to
# see which it does:
#
#   1. It provisions what this worktree can heal and runs the gate; or
#   2. it declines, naming the provisioning that is missing, and does NOT run the gate.
#
# The cases below drive both, and then the one precondition the hook owes on its own
# account: `just` itself absent, which no provisioner it could call would be reached to
# report.
#
# Then the pinned release-plz, which is the machine's to supply and which the provisioner
# resolves from the repository-scoped location scripts/scoped-release-plz.sh owns. Its
# refusal is spelled for a reader that is not a person: `onevcs: host-prerequisite: <what
# is missing and how to install it>` is the marker onevcs's docs/contract.md states for a
# merge-path hook refusing on a missing host tool, which is how the engine on this host
# settles it without spending a worker's retries
# (https://github.com/nickderobertis/onepipeline/issues/360). Cases 4 to 6 hold the hook to
# exactly one such line when that tool is missing or at the wrong version, naming the tool,
# the version and the install command, and to none at all when it is there.
#
# `just` is stubbed where it is present. What is under test is the hook's provisioning decision,
# and a real `just gate` here would be this repository's whole gate run twice — from inside
# itself, since this check is one of the things that gate runs.
set -euo pipefail

fatal() {
  echo "check-pre-push-provisioning: $1" >&2
  echo "check-pre-push-provisioning: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just check' does"
readonly ROOT

# The path below is built from $ROOT at run time, so ShellCheck cannot follow it; the
# directive names the file it resolves to. Tested before it is sourced rather than guarded
# after: bash 3.2 ends the shell where `source` cannot find its file, so the handler after
# `||` never runs there and the reader is told nothing about what to restore.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
scratch_clone_strip_git_env

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check clones into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

readonly CLONE="$scratch/worktree"
scratch_clone "$ROOT" "$CLONE" || fatal \
  "could not clone this repository into $CLONE" \
  "see the diagnostic above; a clone is what makes this an unprovisioned worktree"

# The clone carries HEAD, and what is under test is the hook as it is right now — so the
# WORKING tree's tracked files go over the top of it. The clone is still what supplies the
# `.git` directory the hook's own `git rev-parse --show-toplevel` needs to find its root.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$CLONE" || fatal \
  "could not copy $ROOT's tracked files over the clone at $CLONE" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

# A clone has none of it, but say so rather than assume it: a check that quietly ran against
# a provisioned tree would pass while proving nothing.
for installed in node_modules sdks/python/.venv; do
  [ ! -e "$CLONE/$installed" ] || fatal \
    "the scratch clone already carries $installed, so it is not the unprovisioned worktree this check needs" \
    "report this — scripts/scratch-clone.sh checks out tracked files only, so an installed directory there is untracked and should be in .gitignore"
done

readonly STUB_BIN="$scratch/stub-bin"
readonly GATE_MARKER="$scratch/gate-was-run"
mkdir -p "$STUB_BIN" || fatal \
  "could not create the stub directory at $STUB_BIN" \
  "check the permissions of \$TMPDIR, then rerun"

# The stub `just`. It records that the hook reached the gate and succeeds, so case 1 below
# distinguishes "provisioned and ran the gate" from "never got that far".
cat > "$STUB_BIN/just" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$GATE_MARKER"
STUB
chmod +x "$STUB_BIN/just" || fatal \
  "could not make the stub 'just' executable" \
  "check the permissions of \$TMPDIR, then rerun"

BASH_BIN="$(command -v bash)" || fatal \
  "could not resolve the bash this check runs the hook with" \
  "install bash, or run this check from a shell whose PATH carries it"
readonly BASH_BIN

# Put one tool into a whitelist directory, as a shim that runs the real one where it is.
#
# A symlink cannot do this on every platform this gate runs on: git-bash on Windows copies
# instead of linking, and a copied `bash.exe` cannot find the runtime libraries that sit
# beside the original — so cases 2 and 3 below ran a bash that could not start, and read
# its silence as the hook having said nothing about what was missing. A shim leaves every
# binary where it is and decides only which names this PATH can reach. Its interpreter is
# named absolutely because `bash` itself is one of the names being decided.
shim() {
  printf '#!%s\nexec "%s" "$@"\n' "$BASH_BIN" "$3" > "$1/$2" \
    && chmod +x "$1/$2"
}

# The scoped tool location each case runs the hook under. The provisioner resolves
# release-plz from it and never from PATH, so what a case says about that tool is said by
# what it puts here: a stand-in answering the pin, one answering another version, or
# nothing. Relocated through ONETASKGRAPH_TOOLS_HOME so the machine's own state, which
# `just bootstrap` may or may not have provisioned, is not what these cases read.
readonly PIN="$(bash "$ROOT/scripts/scoped-release-plz.sh" pin)"
[[ $PIN =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fatal \
  "scripts/scoped-release-plz.sh pins '$PIN', which is not an exact X.Y.Z version" \
  "restore its RELEASE_PLZ_VERSION and rerun"
readonly TOOLS_PROVISIONED="$scratch/tools-provisioned"
readonly TOOLS_WRONG_VERSION="$scratch/tools-wrong-version"
readonly TOOLS_EMPTY="$scratch/tools-empty"
plant_release_plz() {
  local home="$1" answers="$2" dir
  dir="$home/release-plz/$PIN/bin"
  mkdir -p "$dir" \
    && printf '#!%s\necho "release-plz %s"\n' "$BASH_BIN" "$answers" > "$dir/release-plz" \
    && chmod +x "$dir/release-plz"
}
plant_release_plz "$TOOLS_PROVISIONED" "$PIN" || fatal \
  "could not plant the pinned release-plz stand-in under $TOOLS_PROVISIONED" \
  "check the permissions of \$TMPDIR, then rerun"
plant_release_plz "$TOOLS_WRONG_VERSION" "0.0.0" || fatal \
  "could not plant the wrong-version release-plz stand-in under $TOOLS_WRONG_VERSION" \
  "check the permissions of \$TMPDIR, then rerun"
mkdir -p "$TOOLS_EMPTY" || fatal \
  "could not create the empty tool location at $TOOLS_EMPTY" \
  "check the permissions of \$TMPDIR, then rerun"
readonly HOST_PREREQUISITE="onevcs: host-prerequisite: "

failures=0
HOOK_OUTPUT=""
HOOK_STATUS=0

# Run the real hook out of the scratch clone, under a PATH this case chose and a scoped tool
# location — the provisioned one unless the case names another.
run_hook() {
  rm -f "$GATE_MARKER"
  # By absolute path: what each case varies is what the hook can *find*, and a hook that
  # could not be started at all would report that as the same silence. Its stdin is empty:
  # git feeds the hook its ref records there, and a hook that provisions goes on to read
  # them, so an inherited stdin left open would hold that case until it was interrupted.
  HOOK_OUTPUT="$(PATH="$1" ONETASKGRAPH_TOOLS_HOME="${2:-$TOOLS_PROVISIONED}" "$BASH_BIN" "$CLONE/.githooks/pre-push" 2>&1 </dev/null)" \
    && HOOK_STATUS=0 || HOOK_STATUS=$?
}

marker_lines() {
  grep -c -- "^$HOST_PREREQUISITE" <<<"$HOOK_OUTPUT" || true
}

report_hook_output() {
  printf '%s\n' "$HOOK_OUTPUT" | sed 's/^/    /' >&2
}

names() {
  grep -qF -- "$1" <<<"$HOOK_OUTPUT"
}

# 1. The whole point: an unprovisioned worktree gets one of the two acceptable outcomes.
#    Which one depends on the machine this runs on — a host with bun and uv reachable
#    provisions, one without them names what it cannot supply — and both are correct. What
#    is refused is the third outcome, the one that shipped: the gate running anyway and its
#    failure reading as a rejection of the push.
run_hook "$STUB_BIN:$PATH"
if [ "$HOOK_STATUS" -eq 0 ]; then
  if [ ! -f "$GATE_MARKER" ]; then
    echo "check-pre-push-provisioning: the hook succeeded in an unprovisioned worktree without" >&2
    echo "check-pre-push-provisioning: ever running the gate, so a push would go out unchecked." >&2
    report_hook_output
    failures=$((failures + 1))
  elif ! grep -qF -- "gate" "$GATE_MARKER"; then
    echo "check-pre-push-provisioning: the hook ran 'just' but not its gate recipe:" >&2
    sed 's/^/    just /' "$GATE_MARKER" >&2
    failures=$((failures + 1))
  fi
elif ! names "provisioned"; then
  echo "check-pre-push-provisioning: the hook refused an unprovisioned worktree without saying" >&2
  echo "check-pre-push-provisioning: that provisioning is what is missing, so its refusal reads" >&2
  echo "check-pre-push-provisioning: as a rejection of the push. It said:" >&2
  report_hook_output
  failures=$((failures + 1))
elif [ -f "$GATE_MARKER" ]; then
  echo "check-pre-push-provisioning: the hook declined for want of provisioning and ran the gate" >&2
  echo "check-pre-push-provisioning: anyway, so the gate's own failure is what the person sees." >&2
  report_hook_output
  failures=$((failures + 1))
fi

# 2. One tool only the machine can supply is gone. A git hook must not install a toolchain,
#    so the hook owes the name of what is absent — and must not reach the gate, whose
#    failure would be about bun and read as being about the push.
#
# The sanitised PATH holds one directory of symlinks, so `bun` is absent however many
# directories of the real PATH carry a copy of it.
readonly CLEAN_BIN="$scratch/clean-bin"
mkdir -p "$CLEAN_BIN" || fatal \
  "could not create the sanitised bin directory at $CLEAN_BIN" \
  "check the permissions of \$TMPDIR, then rerun"
cp "$STUB_BIN/just" "$CLEAN_BIN/just" || fatal \
  "could not place the stub 'just' in the sanitised bin directory" \
  "check the permissions of \$TMPDIR, then rerun"
# Everything the hook and the provisioner reach for, except bun. `env` and `bash` are here
# because the hook is a bash script run through this PATH; the rest is what
# scripts/provision-gate.sh looks up.
for tool in env bash sed grep git python3 uv cargo cargo-llvm-cov cargo-nextest cargo-deny cargo-machete node; do
  resolved="$(command -v "$tool" 2>/dev/null)" || continue
  shim "$CLEAN_BIN" "$tool" "$resolved" || fatal \
    "could not shim $tool into $CLEAN_BIN" \
    "check the permissions of \$TMPDIR, then rerun"
done
# The sanitisation is a precondition of the case, not part of what it proves: a PATH that
# still reaches bun would make the hook's silence about it look like a passing case.
PATH="$CLEAN_BIN" command -v bun >/dev/null 2>&1 && fatal \
  "bun is still reachable from $CLEAN_BIN, so this case cannot pose the question it asks" \
  "report this — the whitelist directory is built here and should hold no bun"
PATH="$CLEAN_BIN" command -v git >/dev/null 2>&1 || fatal \
  "git is not reachable from $CLEAN_BIN, so the hook would fail on that rather than on bun" \
  "install git, or report this if it is installed — the shims above should have found it"

run_hook "$CLEAN_BIN"
if [ "$HOOK_STATUS" -eq 0 ]; then
  echo "check-pre-push-provisioning: the hook accepted a worktree with no bun installed, so the" >&2
  echo "check-pre-push-provisioning: gate it reports as green never had an Nx to run." >&2
  report_hook_output
  failures=$((failures + 1))
else
  for term in bun "provisioned"; do
    if ! names "$term"; then
      echo "check-pre-push-provisioning: bun is absent and the hook refused, but its diagnostic" >&2
      echo "check-pre-push-provisioning: never mentions '$term', so it does not say what to go and" >&2
      echo "check-pre-push-provisioning: install. It said:" >&2
      report_hook_output
      failures=$((failures + 1))
    fi
  done
  if [ -f "$GATE_MARKER" ]; then
    echo "check-pre-push-provisioning: bun is absent and the hook ran the gate regardless, so what" >&2
    echo "check-pre-push-provisioning: the person sees is Nx failing rather than bun missing." >&2
    report_hook_output
    failures=$((failures + 1))
  fi
fi

# 3. `just` itself is absent. It is the hook's own precondition — the hook cannot invoke
#    the provisioner and then the gate without it — so the hook owes its own refusal
#    rather than the provisioner's, and it must say that nothing has been checked.
readonly NO_JUST="$scratch/no-just-bin"
mkdir -p "$NO_JUST" || fatal \
  "could not create the just-free bin directory at $NO_JUST" \
  "check the permissions of \$TMPDIR, then rerun"
for tool in env bash sed grep git python3 uv cargo bun node; do
  resolved="$(command -v "$tool" 2>/dev/null)" || continue
  shim "$NO_JUST" "$tool" "$resolved" || fatal \
    "could not shim $tool into $NO_JUST" \
    "check the permissions of \$TMPDIR, then rerun"
done
PATH="$NO_JUST" command -v just >/dev/null 2>&1 && fatal \
  "just is still reachable from $NO_JUST, so this case cannot pose the question it asks" \
  "report this — the whitelist directory is built here and should hold no just"

run_hook "$NO_JUST"
if [ "$HOOK_STATUS" -eq 0 ]; then
  echo "check-pre-push-provisioning: the hook accepted a worktree with no 'just' installed," >&2
  echo "check-pre-push-provisioning: so the gate it reports as green was never run at all." >&2
  report_hook_output
  failures=$((failures + 1))
else
  for term in just "has been checked"; do
    if ! names "$term"; then
      echo "check-pre-push-provisioning: 'just' is absent and the hook refused, but its" >&2
      echo "check-pre-push-provisioning: diagnostic never mentions '$term', so it reads as a" >&2
      echo "check-pre-push-provisioning: rejection of the push rather than a missing tool. It said:" >&2
      report_hook_output
      failures=$((failures + 1))
    fi
  done
fi

# 4. The pinned release-plz is not in the scoped location. It is the machine's to supply,
#    like bun, and the refusal carries the marker: exactly one line, naming the tool, the
#    version and the install command, before the hook says nothing has been checked — and
#    the gate is not reached.
expect_host_prerequisite() {
  local case_name="$1"
  if [ "$HOOK_STATUS" -eq 0 ]; then
    echo "check-pre-push-provisioning: the hook accepted a worktree with $case_name, so the gate it" >&2
    echo "check-pre-push-provisioning: reports as green would have run the real release preparation on nothing." >&2
    report_hook_output
    failures=$((failures + 1))
    return
  fi
  if [ "$(marker_lines)" -ne 1 ]; then
    echo "check-pre-push-provisioning: with $case_name the hook printed $(marker_lines) line(s) starting" >&2
    echo "check-pre-push-provisioning: '$HOST_PREREQUISITE', where the contract is exactly one. It said:" >&2
    report_hook_output
    failures=$((failures + 1))
  else
    local marker
    marker="$(grep -- "^$HOST_PREREQUISITE" <<<"$HOOK_OUTPUT")"
    for term in "release-plz" "$PIN" "scripts/scoped-release-plz.sh ensure"; do
      if ! grep -qF -- "$term" <<<"$marker"; then
        echo "check-pre-push-provisioning: with $case_name the marker line never names '$term', so the" >&2
        echo "check-pre-push-provisioning: engine reading it is not told what to install. It said:" >&2
        printf '    %s\n' "$marker" >&2
        failures=$((failures + 1))
      fi
    done
  fi
  for term in "provisioned" "has been checked"; do
    if ! names "$term"; then
      echo "check-pre-push-provisioning: with $case_name the hook refused without saying '$term', so" >&2
      echo "check-pre-push-provisioning: its refusal reads as a rejection of the push. It said:" >&2
      report_hook_output
      failures=$((failures + 1))
    fi
  done
  if [ -f "$GATE_MARKER" ]; then
    echo "check-pre-push-provisioning: with $case_name the hook ran the gate regardless, so what the" >&2
    echo "check-pre-push-provisioning: person sees is the release preparation failing rather than the tool missing." >&2
    report_hook_output
    failures=$((failures + 1))
  fi
}

run_hook "$STUB_BIN:$PATH" "$TOOLS_EMPTY"
expect_host_prerequisite "no release-plz in the scoped location"

# 5. A release-plz is there but answers another version — the residue a moved pin leaves,
#    or a hand-placed binary. The same one line, because a wrong version is as much the
#    machine's to fix as an absent one.
run_hook "$STUB_BIN:$PATH" "$TOOLS_WRONG_VERSION"
expect_host_prerequisite "release-plz 0.0.0 in the scoped location"

# 6. The tool is there at the pinned version. Whatever else the machine lacks, the marker
#    is not printed: it claims a host problem to an engine that will stop retrying on it,
#    so it is owed only when that tool is what is missing.
run_hook "$STUB_BIN:$PATH" "$TOOLS_PROVISIONED"
if [ "$(marker_lines)" -ne 0 ]; then
  echo "check-pre-push-provisioning: the pinned release-plz is in the scoped location and the hook" >&2
  echo "check-pre-push-provisioning: still printed the host-prerequisite marker. It said:" >&2
  report_hook_output
  failures=$((failures + 1))
fi
# The marker is also not what bun's absence gets: that refusal already names what to
# install, and the marker is one tool's.
run_hook "$CLEAN_BIN" "$TOOLS_EMPTY"
if [ "$(marker_lines)" -ne 0 ]; then
  echo "check-pre-push-provisioning: with bun absent and release-plz absent the hook printed the" >&2
  echo "check-pre-push-provisioning: host-prerequisite marker, where bun's refusal comes first and is" >&2
  echo "check-pre-push-provisioning: not what the marker is for. It said:" >&2
  report_hook_output
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  echo "check-pre-push-provisioning: $failures case(s) failed." >&2
  echo "check-pre-push-provisioning: repair .githooks/pre-push or scripts/provision-gate.sh so an" >&2
  echo "check-pre-push-provisioning: unprovisioned worktree either provisions or says what it needs." >&2
  exit 1
fi
