#!/usr/bin/env bash
# Drive the screenshot tooling this repository owns through its real paths.
#
# `scripts/screenshots-freeze.sh` decides which renderer the capture runs and provisions it,
# `scripts/screenshots-bless.sh` writes the committed digest baseline, and
# `scripts/check-visual-docs.sh` is what fails when a copy of a pin or an image parts from
# its source. Between them they carry a couple of dozen refusals, and every one is a path a
# person meets on a bad day — a renderer that is not there, an archive that would write
# outside the directory it unpacks into, a README embedding an image nobody committed. So
# they are driven here rather than read, exactly as scripts/check-scoped-release-plz.sh
# drives the other scoped tool: against a stand-in tool location and a stand-in installer,
# with NO network and nothing installed on the host touched. The last of the three is
# watched REFUSING, because a guard nobody has seen fail is a guard nobody knows works.
#
# What is stood in for is `curl` (the one thing that reaches a network) and screencomp.
# What is real is both scripts, the archive handling, the scoped layout and the version
# verification.
#
# llmlint: ignore-file[code_lands_in_the_domain_that_owns_it] Every shell script here lives
# under scripts/ because three commands of that project enumerate that one directory, so a
# capture script filed under screenshots/ escapes all three in silence. screenshots/AGENTS.md,
# "Where this machinery lives", is the whole of the reasoning.
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

# The resolver authenticates what it downloads against a digest recorded for the published
# archive, which a stand-in archive of course does not match. So each case that has to get
# PAST that check drives a COPY of the real script carrying its own archive's digest —
# everything else about it is the real thing — and case 15 below drives the UNMODIFIED
# script to prove the verification is not decoration.
digest_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# A copy of the resolver that expects the archive named, with the substitution asserted:
# a copy that silently kept the published digest would make every `ensure` case below refuse
# for the wrong reason and prove nothing about provisioning.
resolver_for() {
  local archive="$1" copy digest
  copy="$scratch/resolver-$(basename "$archive" .tgz).sh"
  digest="$(digest_of "$archive")"
  sed "s|^readonly FREEZE_SHA256_${os}_${architecture}=.*|readonly FREEZE_SHA256_${os}_${architecture}=$digest|" \
    "$FREEZE" > "$copy"
  grep -qF "readonly FREEZE_SHA256_${os}_${architecture}=$digest" "$copy" || fatal \
    "could not point a copy of the resolver at the stand-in archive's digest" \
    "check that scripts/screenshots-freeze.sh still records FREEZE_SHA256_${os}_${architecture} on one line"
  printf '%s\n' "$copy"
}

# Run the resolver against a tool home of this case's choosing, with the stand-in first on
# PATH. The real `freeze` on this machine, if any, is never consulted or touched.
run_freeze() {
  local home="$1" archive="${2:-}" subcommand="$3" script="$FREEZE"
  [ -z "$archive" ] || script="$(resolver_for "$archive")"
  OUTPUT="$(PATH="$STAND_IN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" \
    STAND_IN_ARCHIVE="$archive" bash "$script" "$subcommand" 2>&1)" && STATUS=0 || STATUS=$?
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

# 15. No curl: the archive cannot be fetched, and that is what it says.
readonly NO_CURL="$scratch/no-curl"
mkdir -p "$NO_CURL" || fatal "could not create $NO_CURL" \
  "check the permissions of \$TMPDIR, then rerun"
for tool in env bash sed grep mkdir dirname cat cut tar install rm sha256sum shasum mktemp uname; do
  resolved="$(command -v "$tool" 2>/dev/null)" || continue
  printf '#!%s\nexec "%s" "$@"\n' "$BASH_BIN" "$resolved" > "$NO_CURL/$tool"
  chmod +x "$NO_CURL/$tool"
done
PATH="$NO_CURL" command -v curl >/dev/null 2>&1 && fatal \
  "curl is still reachable from $NO_CURL, so this case cannot pose its question" \
  "report this — the whitelist directory is built here and should hold no curl"
readonly HOME_NO_CURL="$scratch/tools-no-curl"
OUTPUT="$(PATH="$NO_CURL" ONETASKGRAPH_TOOLS_HOME="$HOME_NO_CURL" \
  bash "$FREEZE" ensure 2>&1)" && STATUS=0 || STATUS=$?
[ "$STATUS" -eq 0 ] && fail "'ensure' reported success with no curl to fetch the archive with:"
names "curl" || fail "'ensure' refused without naming curl as what is missing:"

# 16. A download that fails: the URL is named, and nothing is installed.
readonly HOME_NO_DOWNLOAD="$scratch/tools-no-download"
OUTPUT="$(PATH="$STAND_IN:$PATH" ONETASKGRAPH_TOOLS_HOME="$HOME_NO_DOWNLOAD" \
  STAND_IN_ARCHIVE="$scratch/there-is-no-such-archive.tgz" bash "$FREEZE" ensure 2>&1)" \
  && STATUS=0 || STATUS=$?
[ "$STATUS" -eq 0 ] && fail "'ensure' reported success where the download failed:"
names "could not download" || fail "a failed download was not named as one:"
[ ! -x "$HOME_NO_DOWNLOAD/freeze/$PIN/bin/freeze" ] \
  || fail "'ensure' installed a renderer after the download failed:"

# 17. And the verification itself: the REAL script, whose recorded digest is the published
#     archive's, refuses the stand-in response and installs nothing. This is the case the
#     copies above would otherwise have quietly removed.
readonly HOME_UNVERIFIED="$scratch/tools-unverified"
OUTPUT="$(PATH="$STAND_IN:$PATH" ONETASKGRAPH_TOOLS_HOME="$HOME_UNVERIFIED" \
  STAND_IN_ARCHIVE="$ARCHIVES/good.tgz" bash "$FREEZE" ensure 2>&1)" && STATUS=0 || STATUS=$?
[ "$STATUS" -eq 0 ] && fail "'ensure' installed an archive that does not match its recorded digest:"
names "hashes to" || fail "'ensure' refused an unauthenticated archive without saying so:"
[ ! -x "$HOME_UNVERIFIED/freeze/$PIN/bin/freeze" ] \
  || fail "'ensure' refused on the digest but installed a renderer anyway:"

# The baseline writer, in a clone because it writes shots/baseline/<lane>.json.
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
# A zero exit that wrote nothing, which is what case 13b is about: a tool reporting
# success is not the baseline being there.
[ -z "\${STAND_IN_MANIFEST_EMPTY:-}" ] || exit 0
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
    STAND_IN_MANIFEST_FAILS="${2:-}" STAND_IN_MANIFEST_EMPTY="${3:-}" \
    bash scripts/screenshots-bless.sh 2>&1)" \
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

# 13b. And the tool REPORTING success without writing the file: a different failure from 13,
#      because nothing exits non-zero, so the only thing between a reader and a `git add` of
#      a path that is not there is bless checking what it is about to name.
rm -f "$CLONE/shots/baseline/$LANE.json"
run_bless "$STAND_IN:$PATH" "" 1
[ "$STATUS" -eq 0 ] && fail "'bless' reported a refreshed baseline that was never written:"
names "shots/baseline/$LANE.json" || fail "a baseline that was not written is not named:"
[ ! -f "$CLONE/shots/baseline/$LANE.json" ] \
  || fail "the stand-in wrote a baseline in the case that is about it not writing one:"

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

# The baseline the cases above deliberately removed, back: the cases below are about what a
# capture or a mutation does to a tree that is otherwise whole, and one already failing for
# another reason proves nothing about either.
git -C "$CLONE" checkout -- shots/baseline || fatal \
  "could not restore the committed baseline in the clone" \
  "report this; the clone is scratch and can be re-created"

# The capture's own input refusals, driven in the clone. Each is reached before the capture
# builds or renders anything, which is what lets them run here for nothing — and each asks
# for no build as well, so that a reordering which put the build first would cost a case a
# release build rather than quietly passing.
run_capture() {
  OUTPUT="$(cd "$CLONE" && SHOTS_OUT="$1" SCREENSHOTS_NO_BUILD="${2:-}" \
    ONETASKGRAPH_TOOLS_HOME="${3:-$HOME_GOOD}" bash scripts/screenshots.sh 2>&1)" \
    && STATUS=0 || STATUS=$?
}

# 18. An absolute SHOTS_OUT names a directory outside this clone.
run_capture /tmp/not-under-shots 1
[ "$STATUS" -eq 0 ] && fail "the capture accepted an absolute SHOTS_OUT it would then remove:"
names "shots/" || fail "an out-of-tree SHOTS_OUT was refused without naming where one may point:"
[ -d /tmp/not-under-shots ] && fail "the capture created /tmp/not-under-shots before refusing it:"

# 19. And one that walks back out with `..`.
run_capture shots/../../elsewhere 1
[ "$STATUS" -eq 0 ] && fail "the capture accepted a SHOTS_OUT walking out of shots/:"
names ".." || fail "an escaping SHOTS_OUT was refused without naming what is wrong with it:"

# 20. A symlink at a component under shots/ redirects the removal, which the lexical checks
#     above cannot see: the resolved path is what decides.
mkdir -p "$scratch/elsewhere" "$CLONE/shots"
ln -sfn "$scratch/elsewhere" "$CLONE/shots/redirected"
run_capture shots/redirected/x86_64 1
[ "$STATUS" -eq 0 ] && fail "the capture wrote through a symlinked component under shots/:"
names "resolves to" || fail "a redirected SHOTS_OUT was refused without saying where it resolved:"
rm -f "$CLONE/shots/redirected"

# 21. SCREENSHOTS_NO_BUILD means no build, so with nothing to drive it refuses rather than
#     building behind the caller's back.
run_capture shots/current/x86_64 1
[ "$STATUS" -eq 0 ] && fail "the capture succeeded with SCREENSHOTS_NO_BUILD and no binary:"
names "SCREENSHOTS_NO_BUILD" || fail "the refusal does not name the variable that asked for no build:"
names "cargo build" || fail "the refusal does not say how to get a binary to capture:"

# 21b. `shots` ITSELF is not a lane directory: `shots/.` resolves there, and the capture
#      removes what SHOTS_OUT names — which would take the committed baseline with it.
run_capture shots/. 1
[ "$STATUS" -eq 0 ] && fail "the capture accepted the shots root itself as its output directory:"
[ -f "$CLONE/shots/baseline/$LANE.json" ] \
  || fail "the committed baseline is gone after a capture was pointed at the shots root:"

# 22. Without the vendored font the renderer would fetch one over the network and the bytes
#      would stop being reproducible, so the capture refuses rather than rendering.
mv "$CLONE/screenshots/fonts/JetBrainsMono-Regular.ttf" "$scratch/font.ttf"
run_capture shots/current/x86_64 1
[ "$STATUS" -eq 0 ] && fail "the capture ran with no vendored font to render with:"
names "font" || fail "a missing vendored font was refused without naming it:"
mv "$scratch/font.ttf" "$CLONE/screenshots/fonts/JetBrainsMono-Regular.ttf"

# The reconciliation check, watched refusing. Each case mutates ONE governed file in the
# clone, runs the real check there, and restores it: a check that stopped noticing would
# otherwise pass every gate while the copies it exists for drifted apart.
run_visual_docs() {
  OUTPUT="$(cd "$CLONE" && bash scripts/check-visual-docs.sh 2>&1)" && STATUS=0 || STATUS=$?
}

restore() {
  git -C "$CLONE" checkout -- "$1" || fatal \
    "could not restore $1 in the clone" "report this; the clone is scratch and can be re-created"
}

# A precondition: it has to pass on the tree as it stands, or the refusals below say nothing.
run_visual_docs
[ "$STATUS" -eq 0 ] || fail "check-visual-docs.sh does not pass on this tree, so the mutations below prove nothing:"

# 23. The two screencomp versions in the workflow part.
sed -i.bak 's/^\( *screencomp-version: \)v.*/\1v0.0.1/' "$CLONE/.github/workflows/visual-docs.yml"
rm -f "$CLONE/.github/workflows/visual-docs.yml.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "the workflow's two screencomp versions parted and the check passed:"
names "screencomp" || fail "the check refused the parted screencomp pins without naming the tool:"
restore .github/workflows/visual-docs.yml

# 24. An image the README embeds is not committed.
rm -f "$CLONE/docs/screenshots/task-list.svg"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "the README embeds an image this tree does not carry and the check passed:"
names "task-list.svg" || fail "the check refused the missing image without naming it:"
restore docs/screenshots

# 25. The renderer pin gets a second spelling.
printf '\n# freeze 9.9.9 is what this repository renders with\nfreeze-version := "9.9.9"\n' >> "$CLONE/justfile"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "a second spelling of the renderer pin landed in the justfile and the check passed:"
names "FREEZE_VERSION" || fail "the check refused the second pin without naming where the pin lives:"
restore justfile

# 26. Two lanes in screencomp.toml. One baseline is committed and the guard classifies one
#     lane on every host, so a second lane is one nothing gates.
sed -i.bak 's/^\(arches *= *\[.*\)\]/\1, "aarch64"]/' "$CLONE/screencomp.toml"
rm -f "$CLONE/screencomp.toml.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "screencomp.toml declared two lanes and the check passed:"
names "arches" || fail "the check refused a second lane without naming where the lane is declared:"
restore screencomp.toml

# 27. The workflow restates the lane. screencomp reads [capture].arches itself to fan out
#     its matrix, so a copy there is a second statement nothing reconciles.
printf '\n# the %s lane is the one this workflow captures\n' "$LANE" >> "$CLONE/.github/workflows/visual-docs.yml"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "the workflow restated the lane and the check passed:"
names "$LANE" || fail "the check refused the restated lane without naming it:"
restore .github/workflows/visual-docs.yml

# 28. The vendored font's licence goes missing from beside the font it covers.
mv "$CLONE/screenshots/fonts/JetBrainsMono-OFL.txt" "$scratch/ofl.txt"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "the vendored font lost its licence and the check passed:"
names "JetBrainsMono-OFL.txt" || fail "the check refused the missing licence without naming it:"
mv "$scratch/ofl.txt" "$CLONE/screenshots/fonts/JetBrainsMono-OFL.txt"

# 29. The committed baseline stops carrying a shot a scene captures — which is what a
#     renamed scene looks like before anyone re-blesses.
sed -i.bak 's/"task-deps"/"task-deps-renamed"/' "$CLONE/shots/baseline/$LANE.json"
rm -f "$CLONE/shots/baseline/$LANE.json.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "the baseline lost a scene's shot and the check passed:"
names "task-deps" || fail "the check refused the missing shot without naming the scene:"
names "screenshots-bless" || fail "the check refused the missing shot without saying how to re-bless:"
restore "shots/baseline/$LANE.json"

# 30. A committed capture the README embeds nowhere. Every image sits in the section that
#     explains the surface it shows, or it is not committed at all.
cp "$CLONE/docs/screenshots/task-deps.svg" "$CLONE/docs/screenshots/task-orphan.svg"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "a capture the README embeds nowhere was committed and the check passed:"
names "task-orphan.svg" || fail "the check refused the orphaned capture without naming it:"
rm -f "$CLONE/docs/screenshots/task-orphan.svg"

# 31. Alt text that names the command rather than describing the picture. The images carry
#     the README's meaning for a reader who cannot see them.
sed -i.bak 's|!\[[^]]*\](docs/screenshots/task-list\.svg)|![task list](docs/screenshots/task-list.svg)|' "$CLONE/README.md"
rm -f "$CLONE/README.md.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "an image was left with alt text too short to describe it and the check passed:"
names "alt text" || fail "the check refused the unusable alt text without saying what was wrong:"
restore README.md

# 32. The hero stops being the still of `task list` under the title.
sed -i.bak 's|(docs/screenshots/task-list\.svg)|(docs/screenshots/task-deps.svg)|' "$CLONE/README.md"
rm -f "$CLONE/README.md.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "the README's first image stopped being the task-list still and the check passed:"
names "first image" || fail "the check refused the displaced hero without naming it as the first image:"
restore README.md

# 33. Drift becomes a warning. Without `fail-on-drift` the pictures can quietly stop being
#     true, which is the whole thing this adoption is for.
sed -i.bak 's/^\( *fail-on-drift: *\)true/\1false/' "$CLONE/.github/workflows/visual-docs.yml"
rm -f "$CLONE/.github/workflows/visual-docs.yml.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "the workflow stopped failing on drift and the check passed:"
names "fail-on-drift" || fail "the check refused a non-blocking drift gate without naming the input:"
restore .github/workflows/visual-docs.yml

# 34. The capture container parts from the toolchain this repository pins. The capture
#     builds the real release binary, so the compiler is an input to the rendered bytes.
sed -i.bak 's/^\( *container: *rust:\)[0-9][0-9.]*-/\10.0.0-/' "$CLONE/.github/workflows/visual-docs.yml"
rm -f "$CLONE/.github/workflows/visual-docs.yml.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "the capture container parted from rust-toolchain.toml and the check passed:"
names "rust-toolchain.toml" || fail "the check refused the parted container without naming where the version lives:"
restore .github/workflows/visual-docs.yml

# 35. A regenerated capture tree stops being ignored, which is how one gets committed.
sed -i.bak '\|^/shots/current/$|d' "$CLONE/.gitignore"
rm -f "$CLONE/.gitignore.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "a regenerated capture tree stopped being ignored and the check passed:"
names "/shots/current/" || fail "the check refused the un-ignored capture tree without naming it:"
restore .gitignore

# 36. A recipe puts the capture inside the gate. It is informational, and the gate reaching
#     it would put a multi-minute render on every push.
sed -i.bak 's/^check: format-check/check: screenshots format-check/' "$CLONE/justfile"
rm -f "$CLONE/justfile.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "'just check' reached the capture and the check passed:"
names "just check" || fail "the check refused the gate reaching the capture without naming the recipe:"
restore justfile

# 37. An Nx target runs the capture. `nx affected` reaches every target from the gate, so
#     one that captures is the same defect one recipe deeper.
sed -i.bak 's|bun run biome format --write screenshots|bash scripts/screenshots.sh|' "$CLONE/screenshots/project.json"
rm -f "$CLONE/screenshots/project.json.bak"
run_visual_docs
[ "$STATUS" -eq 0 ] && fail "an Nx target ran the capture and the check passed:"
names "No Nx target may" || fail "the check refused the capturing target without saying no target may:"
restore screenshots/project.json

if [ "$failures" -ne 0 ]; then
  echo "check-visual-tools: $failures expectation(s) failed." >&2
  echo "check-visual-tools: repair scripts/screenshots-freeze.sh, scripts/screenshots.sh," >&2
  echo "check-visual-tools: scripts/screenshots-bless.sh or scripts/check-visual-docs.sh so that" >&2
  echo "check-visual-tools: each refusal above happens and says what is wrong and what to do." >&2
  exit 1
fi
