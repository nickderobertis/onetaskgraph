#!/usr/bin/env bash
# Watch scripts/check-bash4-array-builtins.sh refuse what it is meant to refuse.
#
# A text scan keeps passing after it has stopped matching what it describes — a reworded
# pattern, a shape it never covered, a file it stopped reading — so both spellings of the
# builtin are put to it here in every command position a line scan gets wrong, in a scratch
# copy of scripts/. So is the shape it must NOT catch: the name in a comment, which is how
# every script that avoids the builtin explains itself.
set -euo pipefail

fatal() {
  echo "check-bash4-array-builtins-enforced: $1" >&2
  echo "check-bash4-array-builtins-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

# The two names this check reintroduces are held in variables rather than written into the
# case strings below, because the guard scans this file too and it reads a line rather than
# parsing it: a name reached after a space is a command as far as a line scan can tell,
# whether or not a quote opened earlier. Naming them through variables keeps every case
# below ordinary text instead of something the guard would have to be taught to skip — and
# a guard with a skip list is a guard the next author writes around.
readonly BUILTIN="mapfile"
readonly SYNONYM="readarray"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check mutates" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The WORKING tree's scripts, not HEAD's: what is under test is the guard as it is right
# now, so an author repairing it does not watch this check keep failing against the version
# they just replaced.
cp -a "$ROOT/scripts" "$scratch/scripts" || fatal \
  "could not copy $ROOT/scripts into $scratch" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

failures=0
GUARD_OUTPUT=""
GUARD_STATUS=0

run_guard() {
  GUARD_OUTPUT="$(bash "$scratch/scripts/check-bash4-array-builtins.sh" 2>&1)" \
    && GUARD_STATUS=0 || GUARD_STATUS=$?
}

restore() {
  cp "$ROOT/scripts/$1" "$scratch/scripts/$1" || fatal \
    "could not restore scripts/$1 in the scratch copy, so every case after this one would run against a tree still carrying the previous mutation" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
}

# Every fixture the cases below plant is fatal if it cannot be planted or cleared: a case
# run against a tree the fixture never reached, or a leftover fixture the next case then
# trips over, would report on something other than the guard.
fixture() {
  "$@" || fatal \
    "could not $* while planting or clearing a fixture in $scratch/scripts" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
}

# Replace one literal substring of a scratch script. python3 rather than `sed -i`, whose
# in-place spelling differs between GNU and BSD and so would fail on the macOS runner.
substitute() {
  python3 - "$scratch/scripts/$1" "$2" "$3" <<'PY' || fatal \
    "the helper that rewrites a scratch script did not finish, so that case was never put to the guard" \
    "run 'python3 --version' to confirm a working python3 is on PATH, then rerun"
import pathlib
import sys

path, before, after = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
text = path.read_text(encoding="utf-8")
if before not in text:
    print(
        f"check-bash4-array-builtins-enforced: {path} no longer contains the text this case\n"
        f"check-bash4-array-builtins-enforced: rewrites, so the case would prove nothing:\n"
        f"    {before}\n"
        "check-bash4-array-builtins-enforced: next: update the case to the text that replaced it.",
        file=sys.stderr,
    )
    raise SystemExit(1)
path.write_text(text.replace(before, after, 1), encoding="utf-8")
PY
}

report_guard_output() {
  printf '%s\n' "$GUARD_OUTPUT" | sed 's/^/    /' >&2
}

expect_refused() {
  local case_name="$1"
  shift
  if [ "$GUARD_STATUS" -eq 0 ]; then
    echo "check-bash4-array-builtins-enforced: $case_name — the guard passed a script that uses" >&2
    echo "check-bash4-array-builtins-enforced: a bash 4 array builtin. On macos-latest that script" >&2
    echo "check-bash4-array-builtins-enforced: ends on 'command not found', and the required check" >&2
    echo "check-bash4-array-builtins-enforced: it runs under goes red on whatever it was proving." >&2
    failures=$((failures + 1))
    return
  fi
  local term
  for term in "$@"; do
    # A here-string rather than a pipe into `grep -q`: under `pipefail` a quiet grep's early
    # exit SIGPIPEs its writer, which can invert the pipeline's status on a match.
    if ! grep -qF -- "$term" <<<"$GUARD_OUTPUT"; then
      echo "check-bash4-array-builtins-enforced: $case_name — the guard refused, but its" >&2
      echo "check-bash4-array-builtins-enforced: diagnostic never mentions '$term', so it does" >&2
      echo "check-bash4-array-builtins-enforced: not say what to go and fix. It said:" >&2
      report_guard_output
      failures=$((failures + 1))
      return
    fi
  done
}

expect_passed() {
  local case_name="$1"
  if [ "$GUARD_STATUS" -ne 0 ]; then
    echo "check-bash4-array-builtins-enforced: $case_name — the guard refused a script that only" >&2
    echo "check-bash4-array-builtins-enforced: NAMES the builtin. Every script that avoids it" >&2
    echo "check-bash4-array-builtins-enforced: explains why in prose, so a guard this literal is" >&2
    echo "check-bash4-array-builtins-enforced: one nobody can keep working around. It said:" >&2
    report_guard_output
    failures=$((failures + 1))
  fi
}

# 0. The control. Without it, a guard that refused every tree — including this one — would
#    satisfy every refusal case below and look like the strictest check in the repository.
run_guard
if [ "$GUARD_STATUS" -ne 0 ]; then
  echo "check-bash4-array-builtins-enforced: the guard refuses the scripts under test, so the" >&2
  echo "check-bash4-array-builtins-enforced: cases below would prove nothing. Run" >&2
  echo "check-bash4-array-builtins-enforced: 'bash scripts/check-bash4-array-builtins.sh' and fix" >&2
  echo "check-bash4-array-builtins-enforced: what it reports first. It said:" >&2
  report_guard_output
  exit 1
fi

# 1. The defect itself, restored at the line it was found on: the carrier-inventory read
#    that failed `install path (macos-latest)`, spelled the way it was before this
#    conversion.
substitute check-distribution-contract.sh \
  'read_lines packages < "$packages_file"' \
  "$BUILTIN"' -t packages < "$packages_file"'
run_guard
expect_refused "the carrier inventory read back on the bash 4 builtin" \
  "scripts/check-distribution-contract.sh" "$BUILTIN is a bash 4 builtin" \
  "read_lines <array-name>"
# and it has to say WHICH line, or the report is a search rather than a location. The
# number is matched as a pattern rather than as a literal, because pinning the literal
# would make an edit anywhere above the read fail this case instead of the guard.
if ! grep -qE 'scripts/check-distribution-contract\.sh:[0-9]+:' <<<"$GUARD_OUTPUT"; then
  echo "check-bash4-array-builtins-enforced: the guard named the file but no line in it, so its" >&2
  echo "check-bash4-array-builtins-enforced: report does not say where to go. It said:" >&2
  report_guard_output
  failures=$((failures + 1))
fi
restore check-distribution-contract.sh

# 2. The second spelling. It is the same builtin under another name, so a guard that knew
#    only the first would let the identical failure back in.
substitute check-plugin-isolation.sh \
  'read_lines PLUGINS < <(bash' \
  "$SYNONYM"' -t PLUGINS < <(bash'
run_guard
expect_refused "the same builtin under its other name" \
  "scripts/check-plugin-isolation.sh" "$SYNONYM is a bash 4 builtin" \
  "$SYNONYM -t PLUGINS"
restore check-plugin-isolation.sh

# 3. Not at the start of a line. A later author tidying a read into a conditional puts the
#    builtin mid-line, and a guard anchored at column one would never see it.
substitute check-affected-selection.sh \
  'read_lines PLUGINS < <(bash' \
  'if true; then '"$BUILTIN"' -t PLUGINS < <(bash'
run_guard
expect_refused "the builtin reached through a conditional, mid-line" \
  "scripts/check-affected-selection.sh" "$BUILTIN is a bash 4 builtin"
restore check-affected-selection.sh

# 4. Inside a command substitution, where `(` rather than whitespace precedes it.
{
  echo '#!/usr/bin/env bash'
  echo 'set -euo pipefail'
  echo "names=\$($SYNONYM -t x < /dev/null)"
} > "$scratch/scripts/substituted.sh" || fatal \
  "could not write the command-substitution fixture in $scratch/scripts, so this case was never put to the guard" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
run_guard
expect_refused "the builtin opening a command substitution" \
  "scripts/substituted.sh" "$SYNONYM is a bash 4 builtin"
fixture rm -f "$scratch/scripts/substituted.sh"

# 5. Straight onto a redirection, with no space between. `mapfile<file` fills the default
#    array and is a real spelling, so a pattern that insisted on whitespace after the name
#    would pass it — which is how a scan starts covering less than it says it does.
{
  echo '#!/usr/bin/env bash'
  echo 'set -euo pipefail'
  echo "$BUILTIN<\"\$1\""
  echo 'printf "%s" "${MAPFILE[@]}"'
} > "$scratch/scripts/redirected.sh" || fatal \
  "could not write the redirection fixture in $scratch/scripts, so this case was never put to the guard" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
run_guard
expect_refused "the builtin reading straight from a redirection" \
  "scripts/redirected.sh" "$BUILTIN is a bash 4 builtin"
fixture rm -f "$scratch/scripts/redirected.sh"

# 6. A new script carrying no .sh suffix. It is a bash script by its shebang and the macOS
#    runner will treat it as one, so the scan has to find it by that too.
{
  echo '#!/usr/bin/env bash'
  echo 'set -euo pipefail'
  echo "$BUILTIN -t names < /dev/null"
  echo 'printf "%s\n" "${names[@]}"'
} > "$scratch/scripts/extension-less-helper" || fatal \
  "could not write the extensionless fixture in $scratch/scripts, so this case was never put to the guard" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
run_guard
expect_refused "a shell script found by its shebang rather than its suffix" \
  "scripts/extension-less-helper" "$BUILTIN is a bash 4 builtin"
fixture rm -f "$scratch/scripts/extension-less-helper"

# 7. The shape the guard must NOT refuse: the name in a comment. Every converted call site
#    says in prose which builtin it is avoiding, and scripts/read-lines.sh opens with it.
{
  echo '#!/usr/bin/env bash'
  echo "# $BUILTIN -t would be the natural spelling here, and $SYNONYM is the same builtin"
  echo '# under another name. Neither exists on bash 3.2, so this reads its lines otherwise.'
  echo 'set -euo pipefail'
} > "$scratch/scripts/prose-only.sh" || fatal \
  "could not write the prose fixture in $scratch/scripts, so this case was never put to the guard" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
run_guard
expect_passed "a comment naming both spellings"
fixture rm -f "$scratch/scripts/prose-only.sh"

# 8. A name that opens a quoted string is data, and reads as data to the scan too, because
#    the quote is what precedes it.
{
  echo '#!/usr/bin/env bash'
  echo 'set -euo pipefail'
  echo "substitute \"\$1\" 'read_lines packages' '$BUILTIN -t packages'"
} > "$scratch/scripts/quoted-open.sh" || fatal \
  "could not write the quoted-string fixture in $scratch/scripts, so this case was never put to the guard" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
run_guard
expect_passed "a quoted string that opens with the name"
fixture rm -f "$scratch/scripts/quoted-open.sh"

# 9. The same name reached after a space inside a quoted string IS refused, and that is the
#    documented limit of a line scan rather than a defect: nothing short of parsing the
#    shell can tell that quote from a command boundary, and refusing is the safe direction.
#    The way out is to name it through a variable, which is what this file does throughout.
{
  echo '#!/usr/bin/env bash'
  echo 'set -euo pipefail'
  echo "echo \"the $BUILTIN builtin does not exist on bash 3.2\""
} > "$scratch/scripts/quoted-midline.sh" || fatal \
  "could not write the mid-string fixture in $scratch/scripts, so this case was never put to the guard" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
run_guard
expect_refused "the name reached after a space inside a quoted string" \
  "scripts/quoted-midline.sh" "$BUILTIN is a bash 4 builtin"
fixture rm -f "$scratch/scripts/quoted-midline.sh"

if [ "$failures" -ne 0 ]; then
  echo "check-bash4-array-builtins-enforced: $failures case(s) failed." >&2
  echo "check-bash4-array-builtins-enforced: repair scripts/check-bash4-array-builtins.sh rather" >&2
  echo "check-bash4-array-builtins-enforced: than relaxing the case above: the whole point of" >&2
  echo "check-bash4-array-builtins-enforced: that guard is that a bash 3.2 failure stays" >&2
  echo "check-bash4-array-builtins-enforced: invisible until a macOS job happens to enter the" >&2
  echo "check-bash4-array-builtins-enforced: script that carries it." >&2
  exit 1
fi
