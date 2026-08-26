#!/usr/bin/env bash
# Prove the four line-into-array reads build the arrays they are meant to.
#
# Two ways a read like this goes wrong, and neither shows on this repository's own inputs:
# a line carrying a space arrives as two elements, and a carriage return is kept where it
# was stripped or stripped where it was kept. So each read is driven the way its own
# journey drives it, over a tree whose carrier, crate and plugin names carry a doubled
# space and CRLF endings, and the array is read back out of the diagnostic that script
# itself prints. Doubled, because where a diagnostic joins the array with one space, a
# single-space name could not tell one whole element from two split ones.

set -euo pipefail

fatal() {
  echo "check-line-reads: $1" >&2
  echo "check-line-reads: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

# Two of the fixtures below are directories whose names carry a carriage return, which NTFS
# forbids outright, and the third journey — scripts/check-affected-selection.sh — skips on
# Windows for its own reason, so there would be nothing left to drive. The Linux and macOS
# lanes run this on every pull request, and macOS is the platform the whole conversion is
# about.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-line-reads: skipped on Windows (the fixtures name files with a carriage return, which NTFS forbids); the Linux and macOS lanes gate these reads" >&2
    exit 0
    ;;
esac

# Tested before it is sourced, not merely guarded after: bash 3.2 ends the shell where
# `source` cannot find its file, so the handler a later bash takes never runs there — and
# macos-latest is a 3.2 runner. Case 6 below drives exactly that, on every platform.
# The paths are assembled from $ROOT at runtime, so shellcheck cannot resolve them. Naming
# each file has it follow and check that file (SC1091) rather than skip it unread.
# shellcheck source=scripts/read-lines.sh
if [ ! -r "$ROOT/scripts/read-lines.sh" ] || ! source "$ROOT/scripts/read-lines.sh"; then
  fatal \
    "could not load $ROOT/scripts/read-lines.sh, the read every case below asserts about" \
    "restore it with 'git checkout -- scripts/read-lines.sh', then rerun"
fi
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal \
    "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment case 4 \
would otherwise clone in" \
    "restore that file with 'git checkout -- scripts/scratch-clone.sh', then rerun"
fi

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree these cases mutate" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

failures=0
OUTPUT=""
STATUS=0

# Every fixture below is fatal if it cannot be planted or cleared: a case run against a
# tree the fixture never reached, or one that inherits a fixture the previous case failed to
# clear, reports on something other than the read under test.
fixture() {
  "$@" || fatal \
    "could not $* while planting or clearing a fixture under $scratch" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
}

report_output() {
  printf '%s\n' "$OUTPUT" | sed 's/^/    /' >&2
}

# The captured text is compared with `grep -F`, so a case may assert on a carriage return
# by putting one in the needle — which is how the CR cases below say "unchanged".
expect_refused_naming() {
  local case_name="$1"
  shift
  if [ "$STATUS" -eq 0 ]; then
    echo "check-line-reads: $case_name — the script accepted a tree it must refuse, so the" >&2
    echo "check-line-reads: array it read is not the one it reported on." >&2
    failures=$((failures + 1))
    return
  fi
  local needle
  for needle in "$@"; do
    if ! grep -qF -- "$needle" <<<"$OUTPUT"; then
      echo "check-line-reads: $case_name — the array the script built is not the one it read." >&2
      echo "check-line-reads: expected its diagnostic to contain, byte for byte:" >&2
      printf 'check-line-reads:     %s\n' "$needle" >&2
      echo "check-line-reads: It said:" >&2
      report_output
      failures=$((failures + 1))
      return
    fi
  done
}

expect_absent() {
  local case_name="$1" needle="$2"
  if grep -qF -- "$needle" <<<"$OUTPUT"; then
    echo "check-line-reads: $case_name — the diagnostic contains '$needle', which it can only" >&2
    echo "check-line-reads: do if the read handed the line over in the wrong shape. It said:" >&2
    report_output
    failures=$((failures + 1))
  fi
}

expect_accepted() {
  local case_name="$1"
  if [ "$STATUS" -ne 0 ]; then
    echo "check-line-reads: $case_name — the script refused a tree it must accept. It said:" >&2
    report_output
    failures=$((failures + 1))
  fi
}

# 1. The read itself, over every shape the four callers can hand it.
#
# The three call sites below can only show as much of their array as their own diagnostics
# render, and two of them join it back into one whitespace-separated string before anything
# is printed. This case is where the array is asserted element by element, against the
# shared implementation all four of them run.
fixture="$scratch/lines.txt"
printf 'plain\ntwo  spaces  inside\n leading and trailing \nback\\slash\n\nends-with-cr\r\n' \
  > "$fixture" || fatal \
  "could not write the line fixture at $fixture" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
printf 'no-final-newline' >> "$fixture" || fatal \
  "could not finish the line fixture at $fixture" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

expected=(
  "plain"
  "two  spaces  inside"
  " leading and trailing "
  'back\slash'
  ""
  "$(printf 'ends-with-cr\r')"
  "no-final-newline"
)
read_lines got < "$fixture"
if [ "${#got[@]}" -ne "${#expected[@]}" ]; then
  echo "check-line-reads: read_lines built ${#got[@]} elements from ${#expected[@]} lines." >&2
  echo "check-line-reads: a line carrying a space has been split into words, or a line has" >&2
  echo "check-line-reads: been dropped. It read:" >&2
  printf 'check-line-reads:     <%s>\n' "${got[@]}" >&2
  failures=$((failures + 1))
else
  for index in "${!expected[@]}"; do
    if [ "${got[$index]}" != "${expected[$index]}" ]; then
      echo "check-line-reads: read_lines element $index is <${got[$index]}>, expected" >&2
      echo "check-line-reads: <${expected[$index]}>. That is the array every one of the four" >&2
      echo "check-line-reads: callers gets, so fix scripts/read-lines.sh rather than a caller." >&2
      failures=$((failures + 1))
    fi
  done
fi

# 2. scripts/check-distribution-contract.sh — both of its reads, driven over a real tree.
contract="$scratch/contract"
mkdir -p "$contract" || fatal \
  "could not create the scratch tree at $contract" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
# The WORKING tree's tracked files: what is under test is the read as it is right now.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$contract" || fatal \
  "could not copy $ROOT's tracked files into $contract (see the tar or git output above)" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

run_contract() {
  OUTPUT="$(cd "$contract" && bash scripts/check-distribution-contract.sh 2>&1)" \
    && STATUS=0 || STATUS=$?
}

carrier="$contract/npm/platforms/darwin-arm64"
spaced_carrier="$contract/npm/platforms/darwin  arm64"
cr_carrier="$contract/npm/platforms/$(printf 'darwin-arm64\r')"
spaced_crate="$contract/crates/spaced name"
cr_crate="$contract/crates/$(printf 'crlf-crate\r')"

# 2a. The control. Without it every refusal below could come from something else entirely.
run_contract
if [ "$STATUS" -ne 0 ]; then
  echo "check-line-reads: scripts/check-distribution-contract.sh refuses the tree under test," >&2
  echo "check-line-reads: so the cases below would prove nothing. Run" >&2
  echo "check-line-reads: 'bash scripts/check-distribution-contract.sh' and fix what it reports" >&2
  echo "check-line-reads: first. It said:" >&2
  report_output
  exit 1
fi

# 2b. A carrier directory whose name carries two spaces. The drift report joins the array
#     with one space, so only the second one survives the join as evidence that the name
#     arrived as a single element.
fixture mv "$carrier" "$spaced_carrier"
run_contract
expect_refused_naming "the carrier inventory over a name carrying spaces" \
  "npm carriers are 'darwin  arm64 "
fixture mv "$spaced_carrier" "$carrier"

# 2c. A carrier directory whose name ends in a carriage return. Nothing strips it here and
#     nothing may: `find` reports what the filesystem holds, and a drift report that quietly
#     dropped the CR would name a directory that does not exist.
fixture mv "$carrier" "$cr_carrier"
run_contract
expect_refused_naming "the carrier inventory over a name ending in a carriage return" \
  "npm carriers are '$(printf 'darwin-arm64\r') "
fixture mv "$cr_carrier" "$carrier"

# 2d. A crate directory whose name carries a space. This read's diagnostic prints one
#     element on its own, so a single space is evidence enough — a split would report
#     'spaced' and stop.
fixture mkdir -p "$spaced_crate"
fixture touch "$spaced_crate/Cargo.toml"
run_contract
expect_refused_naming "the crate inventory over a name carrying a space" \
  "spaced name missing from release-plz package inventory"
fixture rm -rf "$spaced_crate"

# 2e. A crate directory whose name ends in a carriage return, kept as `mapfile -t` kept it.
fixture mkdir -p "$cr_crate"
fixture touch "$cr_crate/Cargo.toml"
run_contract
expect_refused_naming "the crate inventory over a name ending in a carriage return" \
  "$(printf 'crlf-crate\r') missing from release-plz package inventory"
fixture rm -rf "$cr_crate"

# 3. scripts/check-plugin-isolation.sh — the plugin set, read through `tr -d '\r'`.
#
# Its producer is scripts/plugin-crates.sh, and on Windows that producer's stdout is CRLF,
# which is the whole reason the `tr` is in the pipeline. So the fixture is that producer,
# replaced in the scratch tree with one that prints the same names with the line ending a
# case wants — the read under test is untouched.
#
# Note what this site can and cannot show: it hands python one whitespace-joined string, so
# past that point a whole element and two split ones are the same bytes. Case 1 above is
# where element identity is pinned; what is proved here is that a spaced line and a CRLF
# line reach the guard as the same data, and that the CR never survives into a crate name.
read_lines PLUGIN_NAMES < <(bash "$ROOT/scripts/plugin-crates.sh")

# printf's own argument list, so a name carrying spaces stays one argument in the file too.
write_plugin_stub() {
  local path="$1" terminator="$2"
  shift 2
  {
    echo '#!/usr/bin/env bash'
    printf "printf '%%s%s'" "$terminator"
    printf ' %q' "$@"
    printf '\n'
  } > "$path" || fatal \
    "could not write the plugin-crates stub at $path, so that case was never driven" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
}

run_isolation() {
  OUTPUT="$(bash "$contract/scripts/check-plugin-isolation.sh" 2>&1)" \
    && STATUS=0 || STATUS=$?
}

stub="$contract/scripts/plugin-crates.sh"

# 3a. The control: the real names, LF-terminated, through the stub rather than the producer.
#     If this refuses, the stub is wrong and nothing below means anything.
write_plugin_stub "$stub" '\n' "${PLUGIN_NAMES[@]}"
run_isolation
expect_accepted "the plugin set with LF line endings"
lf_clean="$OUTPUT"

# 3b. The same names, CRLF-terminated, as python hands them over on Windows. A carriage
#     return surviving the read would leave every name matching no package in the graph.
write_plugin_stub "$stub" '\r\n' "${PLUGIN_NAMES[@]}"
run_isolation
expect_accepted "the plugin set with CRLF line endings"
if [ "$OUTPUT" != "$lf_clean" ]; then
  echo "check-line-reads: the plugin set read differently from CRLF input than from LF input." >&2
  echo "check-line-reads: The CRLF run said:" >&2
  report_output
  failures=$((failures + 1))
fi

# 3c. A line carrying spaces, CRLF-terminated: both at once. The guard names what it could
#     not find, and it must name it without a carriage return attached.
write_plugin_stub "$stub" '\r\n' "${PLUGIN_NAMES[@]}" 'phantom  plugin'
run_isolation
expect_refused_naming "a spaced, CRLF-terminated plugin line" \
  "check-plugin-isolation: phantom is no package of this workspace" \
  "check-plugin-isolation: plugin is no package of this workspace"
expect_absent "a spaced, CRLF-terminated plugin line" "$(printf 'plugin\r')"
crlf_phantom="$OUTPUT"

# 3d. The same line with LF endings must produce the same diagnostic, byte for byte. That
#     is what "the `tr` and the read leave CRLF input indistinguishable" means.
write_plugin_stub "$stub" '\n' "${PLUGIN_NAMES[@]}" 'phantom  plugin'
run_isolation
if [ "$OUTPUT" != "$crlf_phantom" ]; then
  echo "check-line-reads: the same plugin set refused differently with LF endings than with" >&2
  echo "check-line-reads: CRLF endings, so the carriage return is reaching the report." >&2
  echo "check-line-reads: The LF run said:" >&2
  report_output
  failures=$((failures + 1))
fi

# 4. scripts/check-affected-selection.sh — the plugin set again, and the one site whose
#    diagnostic prints one element per line. A spaced name that arrived split would be
#    reported twice, by halves, so this case pins element identity end to end.
#
#    It needs a git repository to clone from and a node_modules to copy, so the fixture is
#    a scratch clone with the working tree's files committed on top of it.
if [ ! -d "$ROOT/node_modules" ]; then
  echo "check-line-reads: case 4 could not run without $ROOT/node_modules, so the affected-selection plugin read was not checked" >&2
  echo "check-line-reads: next: run 'just bootstrap', then rerun 'just script-check' for the full check" >&2
else
  selection="$scratch/selection"
  scratch_clone "$ROOT" "$selection" || fatal \
    "could not clone $ROOT into $selection" \
    "see the scratch-clone diagnostic above, then rerun"
  (cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$selection" || fatal \
    "could not copy $ROOT's tracked files over the clone at $selection" \
    "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

  write_plugin_stub "$selection/scripts/plugin-crates.sh" '\r\n' \
    "${PLUGIN_NAMES[@]}" 'phantom  plugin'

  # The check clones its own scratch tree from this one and Nx reads git history there, so
  # the fixture has to be committed rather than merely written.
  fixture git -C "$selection" config user.email "check-line-reads@invalid"
  fixture git -C "$selection" config user.name "check-line-reads"
  fixture git -C "$selection" add -A
  fixture git -C "$selection" commit --quiet --no-verify -m "test: plugin set fixture"
  # A real copy rather than a symlink, for the reason that check gives at its own copy: Nx
  # locates its workspace root from where it is installed as well as from the directory it
  # runs in, and a symlink here would resolve back to this worktree.
  cp -a "$ROOT/node_modules" "$selection/node_modules" || fatal \
    "could not copy node_modules into $selection" \
    "run 'just bootstrap' so node_modules exists, then rerun"

  OUTPUT="$(bash "$selection/scripts/check-affected-selection.sh" 2>&1)" \
    && STATUS=0 || STATUS=$?
  expect_refused_naming "a spaced, CRLF-terminated plugin line, one element per report line" \
    "expected phantom  plugin to be selected, but it was not."
  # Had the carriage return survived, every real name would have missed its expectation too.
  expect_absent "a spaced, CRLF-terminated plugin line" "expected onetaskgraph-linear"
fi

# 5. The array name read_lines is handed. It reaches an `eval` — bash 3.2 has no
#    `declare -n` — so the name is checked against the shell-identifier grammar before it
#    gets there, and that check is the only thing standing between a caller's typo and
#    evaluated shell. A caller that mistypes the name must be told the NAME is the problem,
#    or the report reads as the input having been wrong.
run_read_lines_name() {
  OUTPUT="$(read_lines "$1" < /dev/null 2>&1)" && STATUS=0 || STATUS=$?
}

expect_name_refused() {
  local case_name="$1"
  if [ "$STATUS" -eq 0 ]; then
    echo "check-line-reads: $case_name — read_lines accepted a name it must refuse, so whatever" >&2
    echo "check-line-reads: that name carried reached 'eval'." >&2
    failures=$((failures + 1))
    return
  fi
  if ! grep -qF -- "is not a shell variable name" <<<"$OUTPUT"; then
    echo "check-line-reads: $case_name — read_lines failed without saying the name was the" >&2
    echo "check-line-reads: problem, so a caller's typo reads as the input being wrong. It said:" >&2
    report_output
    failures=$((failures + 1))
  fi
}

run_read_lines_name ""
expect_name_refused "no array name at all"
run_read_lines_name "9lives"
expect_name_refused "an array name opening with a digit"
run_read_lines_name "with-a-hyphen"
expect_name_refused "an array name carrying a hyphen"
# A name that is shell rather than a name. Nothing here asserts on the message alone: what
# the case is for is that the command inside it never ran.
run_read_lines_name "names=x; touch $scratch/evaluated #"
expect_name_refused "an array name carrying shell of its own"
if [ -e "$scratch/evaluated" ]; then
  echo "check-line-reads: read_lines evaluated the name it was handed — the file that name" >&2
  echo "check-line-reads: asked for now exists. The grammar check before the 'eval' in" >&2
  echo "check-line-reads: scripts/read-lines.sh is what has to refuse this." >&2
  failures=$((failures + 1))
fi

# 6. Each caller's report when the helper it sources is not there to be loaded. Every one of
#    them sources before it does any work of its own, so this is the first thing a checkout
#    missing that file hits — and bash's own "No such file or directory" names the sourcing
#    line rather than the file to restore.
#
#    Each is driven twice, and the second run is the one that matters. bash 3.2 ends the
#    shell outright where `source` cannot find its file: the handler bash 5 takes never runs
#    there, so a caller that guards its load AFTER the fact says nothing on macos-latest,
#    which is a 3.2 runner. `set -o posix` is that behaviour in a bash this repository's
#    Linux and Windows lanes actually have — which is what puts a defect only macOS could
#    otherwise report in front of all three.
sever() {
  local tree="$scratch/$1" helper="$2"
  fixture mkdir -p "$tree"
  fixture rm -rf "$tree/scripts"
  fixture cp -a "$ROOT/scripts" "$tree/scripts"
  fixture rm -f "$tree/scripts/$helper"
}

sever severed read-lines.sh
sever severed-clone scratch-clone.sh

expect_load_failure() {
  local tree="$1" helper="$2" script="$3"
  shift 3
  local mode
  for mode in "" "-o posix"; do
    local label="scripts/$script"
    [ -z "$mode" ] || label="$label under 'set $mode'"
    # shellcheck disable=SC2086 # $mode is this check's own literal, and it is two words
    # when it is set at all — quoting it would hand bash one option named "-o posix".
    OUTPUT="$(cd "$scratch/$tree" && bash $mode "scripts/$script" 2>&1)" \
      && STATUS=0 || STATUS=$?
    if [ "$STATUS" -eq 0 ]; then
      echo "check-line-reads: $label passed with scripts/$helper removed, so it" >&2
      echo "check-line-reads: cleared a tree without ever reading the set it checks." >&2
      failures=$((failures + 1))
      continue
    fi
    local needle
    for needle in "$@"; do
      if ! grep -qF -- "$needle" <<<"$OUTPUT"; then
        echo "check-line-reads: $label lost scripts/$helper without saying so: its" >&2
        echo "check-line-reads: report never mentions '$needle', so it does not name the file to" >&2
        echo "check-line-reads: put back. It said:" >&2
        report_output
        failures=$((failures + 1))
        break
      fi
    done
  done
}

expect_load_failure severed read-lines.sh check-distribution-contract.sh \
  "could not load scripts/read-lines.sh" "git checkout -- scripts/read-lines.sh"
expect_load_failure severed read-lines.sh check-plugin-isolation.sh \
  "check-plugin-isolation: could not load" "git checkout -- scripts/read-lines.sh"
expect_load_failure severed read-lines.sh check-affected-selection.sh \
  "check-affected-selection: could not load" "git checkout -- scripts/read-lines.sh"
# This check's own two loads, which are the same shape and would regress the same way.
expect_load_failure severed read-lines.sh check-line-reads.sh \
  "check-line-reads: could not load" "git checkout -- scripts/read-lines.sh"
expect_load_failure severed-clone scratch-clone.sh check-line-reads.sh \
  "check-line-reads: could not load" "git checkout -- scripts/scratch-clone.sh"
expect_load_failure severed-clone scratch-clone.sh check-distribution-contract-enforced.sh \
  "could not load" "git checkout -- scripts/scratch-clone.sh"

if [ "$failures" -ne 0 ]; then
  echo "check-line-reads: $failures case(s) failed." >&2
  echo "check-line-reads: one of the four reads no longer builds the array it used to. Compare" >&2
  echo "check-line-reads: scripts/read-lines.sh against the shape the failing case reported —" >&2
  echo "check-line-reads: a split element means IFS is no longer cleared for the read, and a" >&2
  echo "check-line-reads: changed carriage return means the loop is stripping more than the" >&2
  echo "check-line-reads: newline." >&2
  exit 1
fi
