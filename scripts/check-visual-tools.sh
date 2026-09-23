#!/usr/bin/env bash
# Drive the two screenshot tools this repository owns through their real paths.
#
# `scripts/screenshots-freeze.sh` decides which renderer the capture runs and provisions it;
# `scripts/screenshots-bless.sh` writes the committed digest baseline. Between them they
# carry a dozen refusals, and every one of them is a path a person meets on a bad day — a
# renderer that is not there, an archive that would write outside the directory it is
# unpacked into, a baseline that could not be written. So they are driven here rather than
# read, exactly as scripts/check-scoped-release-plz.sh drives the other scoped tool: against
# a stand-in tool location and a stand-in installer, with NO network and nothing installed
# on the host touched.
#
# What is stood in for is `curl` (the one thing that reaches a network) and screencomp.
# What is real is both scripts, the archive handling, the scoped layout and the version
# verification.
set -euo pipefail

fatal() {
  echo "check-visual-tools: $1" >&2
  echo "check-visual-tools: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just check' does"
readonly ROOT
readonly FREEZE="$ROOT/scripts/screenshots-freeze.sh"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check works in" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

BASH_BIN="$(command -v bash)" || fatal \
  "could not resolve the bash this check runs the stand-ins with" \
  "install bash, or run this check from a shell whose PATH carries it"
readonly BASH_BIN

failures=0
OUTPUT=""
STATUS=0

fail() {
  echo "check-visual-tools: $1" >&2
  printf '%s\n' "$OUTPUT" | sed 's/^/    /' >&2
  failures=$((failures + 1))
}

names() { grep -qF -- "$1" <<<"$OUTPUT"; }

# --- The renderer resolver ---------------------------------------------------------------
PIN="$(bash "$FREEZE" pin)" || fatal \
  "scripts/screenshots-freeze.sh could not answer its own pin" \
  "read its diagnostic; the pin is FREEZE_VERSION in that file"
readonly PIN
[[ $PIN =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fatal \
  "the renderer pin is '$PIN', which is not an exact X.Y.Z version" \
  "restore FREEZE_VERSION in scripts/screenshots-freeze.sh"

# The archive name the script will ask for, derived the same way it derives it, so this
# check needs no second copy of that mapping.
case "$(uname -s)" in
  Linux) os=Linux ;;
  Darwin) os=Darwin ;;
  *)
    echo "check-visual-tools: skipped on $(uname -s), where the renderer publishes no archive and the resolver refuses by design" >&2
    exit 0
    ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) architecture=x86_64 ;;
  arm64 | aarch64) architecture=arm64 ;;
  *) fatal "this machine is $(uname -m), which the renderer publishes no archive for" \
      "run this check on an x86_64 or arm64 machine" ;;
esac
readonly INNER="freeze_${PIN}_${os}_${architecture}"

# A stand-in renderer that answers the pin, and the archives the stand-in curl serves.
readonly ARCHIVES="$scratch/archives"
mkdir -p "$ARCHIVES/good/$INNER" "$ARCHIVES/escaping/$INNER" "$ARCHIVES/linked/$INNER" \
  "$ARCHIVES/wrong/$INNER" || fatal "could not build the stand-in archives under $ARCHIVES" \
  "check the permissions of \$TMPDIR, then rerun"
printf '#!%s\necho "freeze version v%s (abc1234)"\n' "$BASH_BIN" "$PIN" > "$ARCHIVES/good/$INNER/freeze"
printf '#!%s\necho "freeze version v0.0.0"\n' "$BASH_BIN" > "$ARCHIVES/wrong/$INNER/freeze"
cp "$ARCHIVES/good/$INNER/freeze" "$ARCHIVES/escaping/$INNER/freeze"
cp "$ARCHIVES/good/$INNER/freeze" "$ARCHIVES/linked/$INNER/freeze"
chmod +x "$ARCHIVES"/*/"$INNER"/freeze
( cd "$ARCHIVES/good" && tar -czf "$ARCHIVES/good.tgz" "$INNER" )
( cd "$ARCHIVES/wrong" && tar -czf "$ARCHIVES/wrong.tgz" "$INNER" )
# One member that walks up out of the directory it is unpacked into, and one that is a
# symbolic link — the two shapes the resolver refuses before it unpacks anything. `-P` is
# what makes the first possible at all: without it tar quietly strips the leading `../` and
# the archive would carry nothing to refuse, which is a case that passes while posing no
# question. Both are asserted below rather than assumed.
printf 'outside\n' > "$ARCHIVES/outside"
( cd "$ARCHIVES/escaping" && tar -P -czf "$ARCHIVES/escaping.tgz" "$INNER" ../outside ) 2>/dev/null
ln -sf /etc/passwd "$ARCHIVES/linked/$INNER/link"
( cd "$ARCHIVES/linked" && tar -czf "$ARCHIVES/linked.tgz" "$INNER" )

# The preconditions of cases 8 and 9. A stand-in that did not take would look exactly like
# a passing case, so this refuses to run instead.
#
# The listings are captured and matched IN THIS SHELL rather than piped into `grep -q`:
# under `pipefail` a quiet grep exits at its first match, tar dies of SIGPIPE, and the
# pipeline reports tar's death — so the same archive passed or failed by timing. `2>/dev/null`
# because tar warns about the very member name case 8 is looking for, and a warning on a
# case that passes reads as something having gone wrong.
escaping_members="$(tar -tzf "$ARCHIVES/escaping.tgz" 2>/dev/null)" || fatal \
  "could not list the stand-in escaping archive" "report this; the archive is built above"
case "$escaping_members" in
  *..*) ;;
  *) fatal \
    "the stand-in escaping archive carries no '..' member, so the case about one proves nothing" \
    "report this: tar on this machine may strip '../' even under -P" ;;
esac
linked_members="$(tar -tvzf "$ARCHIVES/linked.tgz" 2>/dev/null)" || fatal \
  "could not list the stand-in linked archive" "report this; the archive is built above"
case "$linked_members" in
  l* | *$'\n'l*) ;;
  *) fatal \
    "the stand-in linked archive carries no link member, so the case about one proves nothing" \
    "report this: tar on this machine may store a symlink as its target's contents" ;;
esac

# The stand-in installer: `curl -fsSL -o <target> <url>` copies the archive this case chose
# and records that it was asked, so an idempotent `ensure` can be told from a second fetch.
readonly STAND_IN="$scratch/bin"
mkdir -p "$STAND_IN" || fatal "could not create $STAND_IN" \
  "check the permissions of \$TMPDIR, then rerun"
cat > "$STAND_IN/curl" <<STUB
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$*" >> "$scratch/curl.calls"
target=""
while [ \$# -gt 0 ]; do
  [ "\$1" = "-o" ] && { target="\$2"; shift 2; continue; }
  shift
done
[ -n "\$target" ] || exit 64
cp "\${STAND_IN_ARCHIVE:?the case has to name an archive}" "\$target"
STUB
chmod +x "$STAND_IN/curl" || fatal "could not make the stand-in curl executable" \
  "check the permissions of \$TMPDIR, then rerun"

# Run the resolver against a tool home of this case's choosing, with the stand-in first on
# PATH. The real `freeze` on this machine, if any, is never consulted or touched.
run_freeze() {
  local home="$1" archive="${2:-}" subcommand="$3"
  OUTPUT="$(PATH="$STAND_IN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" \
    STAND_IN_ARCHIVE="$archive" bash "$FREEZE" "$subcommand" 2>&1)" && STATUS=0 || STATUS=$?
}

readonly HOME_EMPTY="$scratch/tools-empty"
mkdir -p "$HOME_EMPTY"

# 1. A call it does not understand refuses with EX_USAGE rather than guessing.
run_freeze "$HOME_EMPTY" "" install
[ "$STATUS" -eq 64 ] || fail "an unknown subcommand exited $STATUS, expected 64 (EX_USAGE):"

# 2. `path` names the scoped location, under the tool home and namespaced by version.
run_freeze "$HOME_EMPTY" "" path
[ "$STATUS" -eq 0 ] || fail "'path' exited $STATUS:"
names "$HOME_EMPTY/freeze/$PIN/bin/freeze" \
  || fail "'path' does not name the version-namespaced scoped location:"

# 3. Nothing provisioned: `resolve` refuses with EX_UNAVAILABLE and says how to provision.
run_freeze "$HOME_EMPTY" "" resolve
[ "$STATUS" -eq 69 ] || fail "'resolve' with nothing provisioned exited $STATUS, expected 69:"
for term in "$PIN" "screenshots-freeze.sh ensure" "on PATH"; do
  names "$term" || fail "'resolve' refused without naming '$term':"
done

# 4. `ensure` provisions from the archive, and `resolve` then answers with the binary.
readonly HOME_GOOD="$scratch/tools-good"
run_freeze "$HOME_GOOD" "$ARCHIVES/good.tgz" ensure
[ "$STATUS" -eq 0 ] || fail "'ensure' could not provision the renderer from its archive:"
run_freeze "$HOME_GOOD" "" resolve
[ "$STATUS" -eq 0 ] || fail "'resolve' refused a renderer 'ensure' had just provisioned:"
names "$HOME_GOOD/freeze/$PIN/bin/freeze" || fail "'resolve' did not print the scoped binary:"

# 5. And it is idempotent: a second `ensure` fetches nothing.
: > "$scratch/curl.calls"
run_freeze "$HOME_GOOD" "$ARCHIVES/good.tgz" ensure
[ "$STATUS" -eq 0 ] || fail "a second 'ensure' failed where the renderer was already there:"
[ ! -s "$scratch/curl.calls" ] \
  || fail "a second 'ensure' fetched the archive again, so every gate run would: $(cat "$scratch/curl.calls")"

# 6. Residue at another version is cleared rather than trusted.
printf '#!%s\necho "freeze version v0.0.0"\n' "$BASH_BIN" > "$HOME_GOOD/freeze/$PIN/bin/freeze"
chmod +x "$HOME_GOOD/freeze/$PIN/bin/freeze"
run_freeze "$HOME_GOOD" "$ARCHIVES/good.tgz" resolve
[ "$STATUS" -eq 69 ] || fail "'resolve' accepted a binary answering another version (exit $STATUS):"
run_freeze "$HOME_GOOD" "$ARCHIVES/good.tgz" ensure
[ "$STATUS" -eq 0 ] || fail "'ensure' could not replace a binary answering another version:"
run_freeze "$HOME_GOOD" "" resolve
[ "$STATUS" -eq 0 ] || fail "the replaced renderer still does not resolve:"

# 7. An archive whose binary answers another version is a failed provisioning, named.
readonly HOME_WRONG="$scratch/tools-wrong"
run_freeze "$HOME_WRONG" "$ARCHIVES/wrong.tgz" ensure
[ "$STATUS" -eq 0 ] && fail "'ensure' accepted an archive whose renderer answers another version:"
names "0.0.0" || fail "'ensure' refused the wrong version without saying what it found:"

# 8. An archive that would write outside the directory it is unpacked into is refused, and
#    nothing is installed.
readonly HOME_ESCAPING="$scratch/tools-escaping"
run_freeze "$HOME_ESCAPING" "$ARCHIVES/escaping.tgz" ensure
[ "$STATUS" -eq 0 ] && fail "'ensure' unpacked an archive carrying an escaping member:"
[ ! -x "$HOME_ESCAPING/freeze/$PIN/bin/freeze" ] \
  || fail "'ensure' refused the escaping archive but installed a renderer anyway:"

# 9. And one carrying a link member, which a name listing cannot show.
readonly HOME_LINKED="$scratch/tools-linked"
run_freeze "$HOME_LINKED" "$ARCHIVES/linked.tgz" ensure
[ "$STATUS" -eq 0 ] && fail "'ensure' unpacked an archive carrying a link member:"
names "link" || fail "'ensure' refused the linked archive without saying what was wrong:"

# 10. A tool home that is not an absolute path: nothing is created or removed under it.
OUTPUT="$(PATH="$STAND_IN:$PATH" ONETASKGRAPH_TOOLS_HOME="relative/tools" \
  bash "$FREEZE" ensure 2>&1)" && STATUS=0 || STATUS=$?
[ "$STATUS" -eq 69 ] || fail "a relative tool home exited $STATUS, expected 69:"
names "absolute" || fail "a relative tool home was refused without saying why:"

# --- The baseline writer -----------------------------------------------------------------
# In a clone, because it writes shots/baseline/<lane>.json.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
readonly CLONE="$scratch/repo"
scratch_clone "$ROOT" "$CLONE" || fatal \
  "could not clone this repository into $CLONE" \
  "check 'git status' here and the free space on \$TMPDIR, then rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$CLONE" || fatal \
  "could not copy $ROOT's tracked files over the clone at $CLONE" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

LANE="$(sed -n 's/^arches *= *\[ *"\([^"]*\)" *\].*/\1/p' "$CLONE/screencomp.toml")"
[ -n "$LANE" ] || fatal "could not read the lane out of [capture].arches in screencomp.toml" \
  "restore the single lane there, then rerun"
readonly LANE

# A stand-in screencomp whose `manifest` writes the file, or fails, as the case chooses.
cat > "$STAND_IN/screencomp" <<STUB
#!/usr/bin/env bash
set -uo pipefail
[ "\${1:-}" = "manifest" ] || exit 64
[ -z "\${STAND_IN_MANIFEST_FAILS:-}" ] || exit 1
while [ \$# -gt 0 ]; do
  [ "\$1" = "--output" ] && { printf '{"schema":1,"shots":[]}\n' > "\$2"; exit 0; }
  shift
done
exit 64
STUB
chmod +x "$STAND_IN/screencomp" || fatal "could not make the stand-in screencomp executable" \
  "check the permissions of \$TMPDIR, then rerun"

run_bless() {
  OUTPUT="$(cd "$CLONE" && PATH="${1:-$STAND_IN:$PATH}" \
    STAND_IN_MANIFEST_FAILS="${2:-}" bash scripts/screenshots-bless.sh 2>&1)" \
    && STATUS=0 || STATUS=$?
}

# 11. Nothing captured: there is nothing to bless, and it says which directory is empty.
run_bless
[ "$STATUS" -eq 0 ] && fail "'bless' wrote a baseline with no capture to bless:"
names "shots/current/$LANE" || fail "'bless' refused without naming the capture it wanted:"
names "just screenshots" || fail "'bless' refused without naming how to make one:"

# 12. A capture is there and the tool writes the baseline.
mkdir -p "$CLONE/shots/current/$LANE"
printf '{"schema":1,"shots":[]}\n' > "$CLONE/shots/current/$LANE/captures.json"
rm -f "$CLONE/shots/baseline/$LANE.json"
run_bless
[ "$STATUS" -eq 0 ] || fail "'bless' failed over a real capture:"
[ -f "$CLONE/shots/baseline/$LANE.json" ] \
  || fail "'bless' reported success without writing shots/baseline/$LANE.json:"
names "shots/baseline/$LANE.json" || fail "'bless' did not say which baseline it refreshed:"

# 13. The tool failing is not a blessed baseline: it says so rather than exiting through
#     `set -e` with nothing but the tool's own words.
rm -f "$CLONE/shots/baseline/$LANE.json"
run_bless "$STAND_IN:$PATH" 1
[ "$STATUS" -eq 0 ] && fail "'bless' reported success where the manifest step failed:"
names "was not written" || fail "a failed manifest step was not named as such:"
[ ! -f "$CLONE/shots/baseline/$LANE.json" ] \
  || fail "'bless' wrote a baseline after the manifest step failed:"

# 14. screencomp absent: named, rather than a 'command not found' through `set -e`.
readonly NO_SCREENCOMP="$scratch/no-screencomp"
mkdir -p "$NO_SCREENCOMP" || fatal "could not create $NO_SCREENCOMP" \
  "check the permissions of \$TMPDIR, then rerun"
for tool in env bash sed grep mkdir dirname cat; do
  resolved="$(command -v "$tool" 2>/dev/null)" || continue
  printf '#!%s\nexec "%s" "$@"\n' "$BASH_BIN" "$resolved" > "$NO_SCREENCOMP/$tool"
  chmod +x "$NO_SCREENCOMP/$tool"
done
PATH="$NO_SCREENCOMP" command -v screencomp >/dev/null 2>&1 && fatal \
  "screencomp is still reachable from $NO_SCREENCOMP, so this case cannot pose its question" \
  "report this — the whitelist directory is built here and should hold no screencomp"
run_bless "$NO_SCREENCOMP"
[ "$STATUS" -eq 0 ] && fail "'bless' succeeded with no screencomp installed:"
names "screencomp" || fail "'bless' refused without naming the tool that is missing:"

if [ "$failures" -ne 0 ]; then
  echo "check-visual-tools: $failures expectation(s) failed." >&2
  echo "check-visual-tools: repair scripts/screenshots-freeze.sh or scripts/screenshots-bless.sh" >&2
  echo "check-visual-tools: so each refusal says what is wrong and what to do about it." >&2
  exit 1
fi
