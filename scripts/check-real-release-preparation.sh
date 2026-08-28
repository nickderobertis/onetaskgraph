#!/usr/bin/env bash
# Drive the real pinned release-plz through both decisions without contacting GitHub.
set -euo pipefail

fail() {
  echo "check-real-release-preparation: $1" >&2
  echo "check-real-release-preparation: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fail \
  "could not resolve the repository root" "run this from a checkout of the repository"

# Both production jobs that run release preparation use ubuntu-latest. On Windows, Git
# applies this hermetic fixture's insteadOf transport rewrite before release-plz chooses a
# forge, so the pinned tool sees the local bare repository rather than the deliberately
# GitHub-shaped public origin and refuses it. Linux and macOS retain the real-checkout,
# real-release-plz journey below; Windows has no production release path for it to model.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-real-release-preparation: skipped on Windows (release preparation runs only on ubuntu-latest, and Git rewrites this hermetic fixture's GitHub-shaped origin before release-plz detects its forge there); the Linux and macOS lanes gate both real release decisions" >&2
    exit 0
    ;;
esac

[ -f "$ROOT/scripts/scratch-clone.sh" ] || fail \
  "scripts/scratch-clone.sh is missing, so hook-exported git state cannot be cleared" \
  "restore scripts/scratch-clone.sh and rerun"
# shellcheck source=scripts/scratch-clone.sh
source "$ROOT/scripts/scratch-clone.sh"
# Git exports repository-routing variables to hooks, and they override every later `git
# -C`. Clear them before the first git command so this check addresses its scratch clone
# even when distribution-check is running inside pre-push.
scratch_clone_strip_git_env
pinned="$(sed -n 's/.*release-plz@\([^ ,]*\).*/\1/p' "$ROOT/.github/workflows/release-plz.yml" | head -n1)" || fail \
  "could not read the workflow's release-plz pin" "restore the readable workflow and rerun"
[[ $pinned =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "the release workflow has no exact X.Y.Z release-plz pin ('$pinned')" "restore its exact tool pin and rerun"
[ "$(release-plz --version 2>/dev/null || true)" = "release-plz $pinned" ] || fail \
  "release-plz $pinned is not installed, so the real preparation cannot be exercised" \
  "run 'just bootstrap', which installs the workflow's pinned tool, then rerun"
release_plz_bin="$(command -v release-plz)" || fail \
  "could not resolve the installed release-plz" "run 'just bootstrap', then rerun"
for tool in git gh perl python3 uv; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH" "run 'just bootstrap', then rerun"
done

scratch="$(mktemp -d)" || fail "could not create a scratch directory" "check temporary-directory permissions"
recorder_pid=
cleanup() {
  if [ -n "$recorder_pid" ]; then
    kill "$recorder_pid" 2>/dev/null || true
  fi
  rm -rf "$scratch"
}
trap cleanup EXIT
mkdir -p "$scratch/bin" "$scratch/state" || fail "could not create fixture state directories" "check scratch-directory permissions"
# Every destination this journey is allowed to reach is inside the fixture, and all of them
# are filesystem paths: the bare origin next door, the `gh` shim, the released tree the
# registry lookup now reads. So nothing legitimate ever opens an HTTP connection, and a
# proxy that records and refuses one costs nothing while it holds. Point every proxy
# variable at it, clear the bypass lists, and read its log after each phase below: a
# non-empty log names the host that reached past the fixture, which is what a bare 401 or a
# registry timeout will not.
if ! cat > "$scratch/off-fixture-recorder.py" <<'RECORDER'
"""Record and refuse every HTTP destination, so a host outside the fixture is named."""

import socket
import sys

log_path, port_path = sys.argv[1], sys.argv[2]
server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", 0))
server.listen(64)
with open(port_path, "w") as handle:
    handle.write(str(server.getsockname()[1]))
    handle.flush()

def record(destination):
    # Whatever opened this connection chose the text, so keep only printable non-space
    # characters and bound the length: a destination is what the log is read for, and an
    # embedded newline or control sequence would forge or hide a second entry.
    safe = "".join(c for c in destination if c.isprintable() and not c.isspace())[:200]
    with open(log_path, "a") as handle:
        handle.write((safe or "<unnamed destination>") + "\n")
        handle.flush()


while True:
    connection, _ = server.accept()
    try:
        connection.settimeout(5)
        request = b""
        while b"\r\n" not in request and len(request) < 8192:
            chunk = connection.recv(4096)
            if not chunk:
                break
            request += chunk
        line = request.split(b"\r\n", 1)[0].decode("utf-8", "replace")
        fields = line.split(" ")
        record(fields[1] if len(fields) > 1 else line)
        connection.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    except OSError as error:
        # A connection that failed mid-read still left the fixture, and dropping it would
        # let exactly that attempt pass as a clean run. Record it as the attempt it is.
        record("<destination unread: %s>" % error)
    finally:
        connection.close()
RECORDER
then
  fail "could not create the off-fixture destination recorder" "check scratch-directory permissions and free space"
fi
offsite_log="$scratch/state/off-fixture-destinations"
recorder_port="$scratch/state/recorder-port"
recorder_diagnostics="$scratch/state/recorder-diagnostics"
python3 "$scratch/off-fixture-recorder.py" "$offsite_log" "$recorder_port" \
  > "$recorder_diagnostics" 2>&1 &
recorder_pid=$!
recorder_waits=0
while [ ! -s "$recorder_port" ] && [ "$recorder_waits" -lt 100 ]; do
  sleep 0.1
  recorder_waits=$((recorder_waits + 1))
done
if [ ! -s "$recorder_port" ]; then
  if [ -s "$recorder_diagnostics" ]; then
    sed 's/^/    /' "$recorder_diagnostics" >&2
  fi
  fail "the off-fixture destination recorder did not come up, so nothing would notice a request leaving the fixture" \
    "fix what it reported above; with no output it never started, so check that python3 can bind a loopback port here"
fi
recorder_listen_port="$(cat "$recorder_port")" || fail \
  "could not read the port the off-fixture destination recorder bound" "check scratch-directory permissions and rerun"
if ! [[ $recorder_listen_port =~ ^[1-9][0-9]*$ ]] || [ "$recorder_listen_port" -gt 65535 ]; then
  fail "the off-fixture destination recorder reported '$recorder_listen_port', which is not a TCP port" \
    "restore the recorder so it writes the port it bound, then rerun"
fi
recorder_url="http://127.0.0.1:$recorder_listen_port"
export http_proxy="$recorder_url" https_proxy="$recorder_url" all_proxy="$recorder_url"
export HTTP_PROXY="$recorder_url" HTTPS_PROXY="$recorder_url" ALL_PROXY="$recorder_url"
export no_proxy="" NO_PROXY=""
repo="$scratch/repo"
remote="$scratch/origin.git"
hooks="$scratch/hooks"
fixture_base=fixture-main
scratch_clone "$ROOT" "$repo" || fail \
  "could not clone the finished tree" "fix what scratch-clone reported above and rerun"
# The clone carries this repository's own history, and one unreleased feat or fix anywhere
# in it is enough for release-plz to select a package version — which sends preparation down
# `release-plz release-pr` and out to the real api.github.com, the one boundary this fixture
# cannot serve. Hermetic by luck of what has landed is not hermetic, so the fixture branch is
# a root of its own: the released baseline below, the tooling-only commit this journey is
# about, and nothing this repository happens to be carrying.
git -C "$repo" checkout --quiet --orphan "$fixture_base" || fail \
  "could not root the fixture branch $fixture_base at an orphan commit" \
  "check the cloned commit and rerun"
# The user's hooks belong to the checkout under review, not to fixture setup. Point this
# scratch repository at an empty hook directory so its synthetic commits and local pushes
# cannot recursively launch the complete gate when the check itself runs from a hook.
mkdir -p "$hooks" || fail "could not create the empty fixture hook directory" "check scratch-directory permissions"
git -C "$repo" config core.hooksPath "$hooks" || fail \
  "could not isolate the fixture from repository hooks" "check the scratch repository and rerun"
# Exercise the working tree under review. It is this fixture's released baseline: every
# crate is at the version the boundary tags below name, so no crate has anything waiting to
# be released and release-plz has no package bump to select.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fail \
  "could not copy the tracked working tree into the checkout" "check git, tar and free space, then rerun"
released_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)" || fail \
  "could not read the fixture's released version" "restore the binary manifest and rerun"
[[ $released_version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail \
  "the fixture has no plain X.Y.Z released version ('$released_version')" "restore the binary manifest and rerun"
git -C "$repo" add -A || fail "could not stage the released baseline" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet \
  -m "chore: release v$released_version" || fail \
  "could not commit the released baseline" "check the scratch repository and rerun"
baseline="$(git -C "$repo" rev-parse HEAD)" || fail \
  "could not read the released baseline commit" "check the scratch repository and rerun"
# The checkout under test can be either an ordinary main commit, whose manifest version has
# already been tagged, or a release pull request, whose version has not. Neither shape should
# determine this fixture's history: discard every tag the clone carried and put this
# fixture's own boundary — the workspace tag and every package's — on the baseline, so each
# crate reads as released and the only commit after it is the tooling-only change below.
git -C "$repo" for-each-ref --format='delete %(refname)' refs/tags | \
  git -C "$repo" update-ref --stdin || fail \
  "could not clear inherited tags from the tooling-only fixture" "check the scratch repository and rerun"
package_names="$(cd "$repo" && cargo metadata --no-deps --format-version 1 | python3 -c \
  'import json, sys; print(" ".join(package["name"] for package in json.load(sys.stdin)["packages"]))')" || fail \
  "could not read the fixture's package inventory" "repair Cargo metadata and rerun"
git -C "$repo" tag "v$released_version" "$baseline" || fail \
  "could not set the tooling-only release boundary" "check the scratch repository and rerun"
for crate in $package_names; do
  [ "$crate" = onetaskgraph ] && continue
  git -C "$repo" tag "$crate-v$released_version" "$baseline" || fail \
    "could not put $crate's release boundary on the baseline" "check the scratch repository and rerun"
done
# release-plz compares each package with its released copy, and left alone it downloads that
# copy from crates.io — a host outside this fixture, and the last one this journey reached.
# The real tool takes the comparison from a local checkout of the released version instead,
# which is what the baseline above is, so export it once and point every `update` at it. The
# registry lag the recovery cases turn on stops depending on what crates.io happens to hold
# and becomes this fixture's own: every package is released at the baseline's version and
# nowhere else.
registry_snapshot="$scratch/registry"
mkdir -p "$registry_snapshot" || fail \
  "could not create the fixture's released-version snapshot" "check scratch-directory permissions"
git -C "$repo" archive "$baseline" | tar -xf - -C "$registry_snapshot" || fail \
  "could not export the released version release-plz compares against" \
  "check git, tar and free space, then rerun"
[ -f "$registry_snapshot/Cargo.toml" ] || fail \
  "the released-version snapshot has no workspace manifest, so release-plz would fall back to crates.io" \
  "check that the baseline commit carries Cargo.toml and rerun"
# Semver compatibility is gated independently. Disabling it in this scratch-only config
# keeps this journey focused on selection and proposal rather than compiling every crate
# twice before either decision can be observed.
perl -pi -e 's/^semver_check = true$/semver_check = false/' "$repo/release-plz.toml" || fail \
  "could not disable the unrelated scratch semver pass" "check Perl and rerun"
git -C "$repo" add -A || fail "could not stage the tooling-only fixture" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet \
  -m "fix(release): prepare tooling-only releases" || fail \
  "could not commit the tooling-only fixture" "check the scratch repository and rerun"
git init --quiet --bare "$remote" || fail "could not create the local origin" "check scratch-directory permissions"
# release-plz chooses its forge from the configured remote URL. Keep that public seam
# GitHub-shaped, while Git's URL rewrite routes every transport operation to the local
# bare repository so this journey remains hermetic.
fixture_origin=https://github.com/check/onetaskgraph-release-fixture.git
git -C "$repo" config "url.$remote.insteadOf" "$fixture_origin" || fail \
  "could not route the fixture origin to its local repository" "check git and rerun"
git -C "$repo" remote set-url origin "$fixture_origin" || fail \
  "could not give the fixture a GitHub-shaped origin" "check git and rerun"
git -C "$repo" push --quiet --set-upstream origin HEAD || fail "could not seed the local origin" "check git and rerun"
git --git-dir="$remote" symbolic-ref HEAD "refs/heads/$fixture_base" || fail \
  "could not set the local origin's default branch" "check the scratch repository and rerun"
git -C "$repo" remote set-head origin "$fixture_base" || fail \
  "could not align the clone's default remote branch with $fixture_base" "check the scratch repository and rerun"
git -C "$repo" switch --quiet --detach "$fixture_base" || fail \
  "could not detach the tooling-only checkout" "check the scratch repository and rerun"

if ! cat > "$scratch/bin/release-plz" <<'RELEASE_PLZ'
#!/usr/bin/env bash
set -euo pipefail
read_manifest_version() {
  sed -n 's/^version = "\([^"]*\)"/\1/p' crates/onetaskgraph/Cargo.toml | head -n1
}
if [ "${1:-}" = update ]; then
  # The selector decides from what this run does to the manifest, and nothing downstream
  # can report which branch it took. Record it here so a refusal below can name it.
  before="$(read_manifest_version)"
  status=0
  "$REAL_RELEASE_PLZ" "$@" --forge github \
    --registry-manifest-path "$FIXTURE_REGISTRY_MANIFEST" || status=$?
  printf '%s -> %s\n' "$before" "$(read_manifest_version)" > "$GH_FIXTURE_STATE/selection"
  exit "$status"
fi
if [ "${1:-}" = release-pr ]; then
  # `release-pr` asks api.github.com for this repository's open pull requests. The fixture's
  # insteadOf rule rewrites git transport and cannot touch an HTTP API call, and GIT_TOKEN
  # is the literal string fixture-token — so reaching here is the hermetic premise failing,
  # not a credential missing, and the bare 401 GitHub answers with says the opposite.
  decision="$(cat "$GH_FIXTURE_STATE/selection" 2>/dev/null || true)"
  printf '%s\n' "${decision:-<no update recorded>}" > "$GH_FIXTURE_STATE/forge-api-attempt"
  echo "release-plz fixture: refusing 'release-pr': it calls the real GitHub API, which this hermetic fixture does not serve" >&2
  echo "release-plz fixture: the selector answered 'release-plz selected ${decision:-<no update recorded>}'" >&2
  exit 1
fi
exec "$REAL_RELEASE_PLZ" "$@"
RELEASE_PLZ
then
  fail "could not create the release-plz fixture launcher" "check scratch-directory permissions and free space"
fi
if ! cat > "$scratch/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-} ${2:-}" in
  "pr list") [ -s "$GH_FIXTURE_STATE/proposals" ] && echo 41 || true ;;
  "pr create") printf '%s\n' "$*" >> "$GH_FIXTURE_STATE/proposals"; echo "http://example.invalid/pull/41" ;;
  *) echo "gh fixture: unexpected call: $*" >&2; exit 2 ;;
esac
GH
then
  fail "could not create the gh fixture" "check scratch-directory permissions and free space"
fi
chmod +x "$scratch/bin/release-plz" "$scratch/bin/gh" || fail \
  "could not make the command fixtures executable" "check scratch-directory permissions"
export GH_FIXTURE_STATE="$scratch/state"
export GIT_TOKEN=fixture-token
export GITHUB_REF_NAME="$fixture_base"
# Git's insteadOf routing can be applied before release-plz detects the forge on Windows.
# State the forge at this hermetic fixture boundary while still delegating the update to
# the exact pinned binary verified above.
export REAL_RELEASE_PLZ="$release_plz_bin"
export FIXTURE_REGISTRY_MANIFEST="$registry_snapshot/Cargo.toml"
# set-version.sh re-locks the Python projects after every bump, and uv reads its index for
# that unless told the answer is already written down. It is: uv.lock pins every dependency
# and the only thing changing here is the workspace's own version, so these locks resolve
# from the checked-in files with an empty cache. Without this the journey reaches pypi.org,
# which is as far outside the fixture as crates.io was.
export UV_OFFLINE=1
export PATH="$scratch/bin:$PATH"

forge_attempt="$scratch/state/forge-api-attempt"
# Every transport this fixture offers is local — git is rewritten to a bare repository next
# door and `gh` is the shim above — so `release-plz release-pr`'s HTTP call to
# api.github.com is the one way out of it, and the launcher above records the attempt rather
# than letting it leave. Read that record after every preparation run, before the run's own
# assertions, so this journey fails naming the premise that broke.
guard_hermetic() {
  if [ -s "$offsite_log" ]; then
    fail "the hermetic premise was violated: this journey reached past the fixture to $(sort -u "$offsite_log" | tr '\n' ' ')" \
      "keep every destination inside the fixture — the local origin, the gh shim, and the released-version snapshot release-plz compares against"
  fi
  [ -e "$forge_attempt" ] || return 0
  fail "the hermetic premise was violated: preparation reached 'release-plz release-pr', which asks api.github.com for this fixture's open pull requests, after the selector answered 'release-plz selected $(cat "$forge_attempt")'" \
    "keep an unreleased releasable commit out of this fixture's history, so selection stays on the release-tooling and registry-recovery branches this journey proposes through gh"
}

# run_preparation <log> <problem> <next action>
run_preparation() {
  local log="$1" problem="$2" next="$3" status=0
  (cd "$repo" && scripts/prepare-release-pr.sh) > "$log" 2>&1 || status=$?
  guard_hermetic
  if [ "$status" -ne 0 ]; then
    sed 's/^/    /' "$log" >&2
    fail "$problem" "$next"
  fi
}

# The version the run says it proposed, read from its own decision line rather than
# recomputed: what is under test includes which version it chose. Held to the same semantic
# version grammar prepare-release-pr.sh holds the manifest to, because everything below
# builds a release branch name out of this: a value that is not a version is the decision
# line having changed shape, and the journey has to say so rather than compare refs that
# never existed.
prepared_version() {
  local version
  version="$(sed -n 's/^prepare-release-pr: .* at v\([0-9][0-9A-Za-z.+-]*\)$/\1/p' "$1" | head -n1)"
  [ -n "$version" ] || fail "the preparation's decision line does not name the version it proposed: $(cat "$1")" \
    "keep the proposal line ending in the version, which the workflow log and this journey both read"
  [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || fail \
    "the preparation's decision line names '$version', which is not the semantic version its own manifest check requires" \
    "keep the proposal line ending in the X.Y.Z version prepare-release-pr.sh validated, which is what names the release branch"
  printf '%s' "$version"
}

# A release is never described twice, and never under a version the pull request does not
# propose: an entry release-plz writes while a version is still being selected is headed by
# that package's own next version, which set-version.sh then normalises away.
# Writing no entry at all is not this function's business — the release-tooling and
# registry-recovery proposals reach the branch without one, because the run that writes the
# changelog is the one that builds the release commit, and neither of those is that run.
# changelogs_describe_this_release_at_most_once <base commit> <branch> <released version>
changelogs_describe_this_release_at_most_once() {
  local base="$1" branch="$2" released="$3" crate changelog added heading
  for changelog_path in "$repo"/crates/*/CHANGELOG.md; do
    crate="$(basename "$(dirname "$changelog_path")")"
    changelog="crates/$crate/CHANGELOG.md"
    git -C "$repo" show "$branch:$changelog" > "$scratch/branch-changelog" 2>/dev/null || continue
    git -C "$repo" show "$base:$changelog" > "$scratch/base-changelog" 2>/dev/null || : > "$scratch/base-changelog"
    grep '^## \[' "$scratch/branch-changelog" > "$scratch/branch-headings" || : > "$scratch/branch-headings"
    grep '^## \[' "$scratch/base-changelog" > "$scratch/base-headings" || : > "$scratch/base-headings"
    added=0
    while IFS= read -r heading; do
      grep -qxF "$heading" "$scratch/base-headings" && continue
      added=$((added + 1))
      case "$heading" in
        "## [$released]"*) ;;
        *) fail "$branch carries $changelog holding '$heading', which describes this release under a version it does not propose — its manifests say $released" \
          "select the version without writing a changelog, and let the run that builds the release commit write the entry for the version finally selected" ;;
      esac
    done < "$scratch/branch-headings"
    [ "$added" -le 1 ] || fail \
      "$branch carries $changelog holding $added new entries for one set of changes" \
      "write the release's changelog entry once, for the version finally selected"
  done
}

tooling_base="$(git -C "$repo" rev-parse HEAD)" || fail \
  "could not read the tooling-only head" "check the scratch repository and rerun"
case_log="$scratch/tooling.log"
run_preparation "$case_log" "the real preparation failed for the tooling-only head" \
  "fix the phase named above and rerun"
grep -qF "proposed release pull request" "$case_log" || fail \
  "the tooling-only run did not report a proposal" "repair the fallback proposal path and rerun"
[ -s "$scratch/state/proposals" ] || fail "the tooling-only run never proposed a pull request" \
  "repair the fallback proposal path and rerun"
(cd "$repo" && scripts/set-version.sh --check) || fail "the proposed tree has version drift" \
  "run scripts/set-version.sh with the selected version and carry every changed manifest"
tooling_version="$(prepared_version "$case_log")"
tooling_branch="release-plz-$tooling_version"
changelogs_describe_this_release_at_most_once "$tooling_base" "$tooling_branch" "$tooling_version"
tooling_tree="$(git -C "$repo" rev-parse "$tooling_branch^{tree}")" || fail \
  "could not read the tree the tooling-only run put on $tooling_branch" "check the scratch repository and rerun"

git -C "$repo" switch --quiet --detach "$fixture_base" || fail "could not restore a detached $fixture_base before the update case" "check the scratch repository and rerun"
unset GITHUB_REF_NAME
# What a run that got as far as selecting a version leaves behind: bumped manifests nobody
# committed. The workflow's next push arrives at a fresh checkout, but a rerun here does not,
# and preparing again has to regenerate the release from the base rather than compute it from
# what the last run wrote — release-plz refuses to decide anything from a dirty checkout, and
# a version decided from already-bumped manifests is a second release, not the same one.
if ! (cd "$repo" && scripts/set-version.sh "$tooling_version") > "$scratch/leftover.log" 2>&1; then
  sed 's/^/    /' "$scratch/leftover.log" >&2
  fail "could not leave the checkout as an interrupted preparation leaves it" \
    "fix what set-version.sh reports above and rerun"
fi
[ -n "$(git -C "$repo" status --porcelain)" ] || fail \
  "selection left the checkout clean, so the update case no longer models a rerun over what the last run wrote" \
  "check what the selector writes and put an uncommitted preparation back into this case"
case_log="$scratch/update.log"
run_preparation "$case_log" "the real preparation failed while updating the existing proposal over the tree the last one left" \
  "fix the phase named above and rerun"
grep -qF "updated release pull request #41" "$case_log" || fail \
  "the existing proposal was not updated visibly" "repair the existing-branch and existing-PR path and rerun"
[ "$(wc -l < "$scratch/state/proposals")" -eq 1 ] || fail \
  "updating the existing proposal created a duplicate" "reuse the release branch and open pull request"
[ "$(prepared_version "$case_log")" = "$tooling_version" ] || fail \
  "preparing again over the same base proposed $(prepared_version "$case_log") where the first preparation proposed $tooling_version" \
  "regenerate the release from the base, so a rerun decides what a single run decided"
changelogs_describe_this_release_at_most_once "$tooling_base" "$tooling_branch" "$tooling_version"
[ "$(git -C "$repo" rev-parse "$tooling_branch^{tree}")" = "$tooling_tree" ] || fail \
  "preparing again over the same base put a different tree on $tooling_branch than a single preparation leaves" \
  "regenerate the release from the base rather than adding to what the last run left"

git -C "$repo" switch --quiet "$fixture_base" || fail "could not restore $fixture_base before the partial-publish case" "check the scratch repository and rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$repo" || fail \
  "could not restore the tracked working tree for the partial-publish case" "check git, tar and free space, then rerun"
perl -pi -e 's/^semver_check = true$/semver_check = false/' "$repo/release-plz.toml" || fail \
  "could not disable the partial-publish fixture's semver pass" "check Perl and rerun"
# A partly published release is one the registry is behind, and lag is the whole of what a
# registry can say. Pin it here rather than inheriting whatever crates.io happens to hold:
# once the checkout's own version is published, an unpinned fixture never reaches the
# recovery decision at all, and this journey stops covering the branch it exists for while
# still passing. The boundary below is a release this repository will never publish.
lagged_version=900.0.0
(cd "$repo" && scripts/set-version.sh "$lagged_version") || fail \
  "could not put the partial-publish fixture beyond every registry version" \
  "fix what set-version.sh named above and rerun"
git -C "$repo" add -A || fail "could not stage the partial-publish release boundary" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet \
  -m "chore: release v$lagged_version" || fail \
  "could not commit the partial-publish release boundary" "check the scratch repository and rerun"
git -C "$repo" tag --force "v$lagged_version" >/dev/null || fail "could not set the partial-publish release boundary" "check the scratch repository and rerun"
for crate in $package_names; do
  [ "$crate" = onetaskgraph ] && continue
  git -C "$repo" tag --force "$crate-v$lagged_version" >/dev/null || fail \
    "could not set the partial-publish tag for $crate" "check the scratch repository and rerun"
done
printf '\n' >> "$repo/crates/onetaskgraph-core/src/lib.rs" || fail "could not modify the partial-publish fixture" "check scratch-directory permissions"
git -C "$repo" add crates/onetaskgraph-core/src/lib.rs || fail "could not stage the partial-publish fixture" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet -m "fix(core): partial-publish recovery fixture" || fail \
  "could not commit the partial-publish fixture" "check the scratch repository and rerun"
recovery_version="${lagged_version%.*}.$((${lagged_version##*.} + 1))"
# Observe the selector's own decision before a proposal is built from it. The proposal
# cannot say which branch chose the version, and the branch is what this case covers: with
# the fixture's lag unpinned it stopped reaching recovery at all and still passed.
selector_log="$scratch/partial-publish-selector.log"
if ! (cd "$repo" && scripts/select-release-version.sh) > "$selector_log" 2>&1; then
  sed 's/^/    /' "$selector_log" >&2
  fail "the real selector failed for the partly published head" "fix what it reports above and rerun"
fi
grep -qF "select-release-version: registry recovery selected $lagged_version -> $recovery_version" "$selector_log" || fail \
  "the partly published head reached some decision other than registry recovery: $(cat "$selector_log")" \
  "keep this fixture's registry lag pinned so the recovery branch is what this journey drives"
git -C "$repo" checkout --quiet -- . || fail \
  "could not restore the fixture after observing the selector's decision" "check the scratch repository and rerun"

# The earlier cases leave an open fixture PR. Remove only that scratch state so registry
# recovery proves its fresh-proposal branch as well as the update branch exercised above.
: > "$scratch/state/proposals" || fail "could not clear the scratch proposal state" "check scratch-directory permissions and rerun"
recovery_base="$(git -C "$repo" rev-parse HEAD)" || fail \
  "could not read the partly published head" "check the scratch repository and rerun"
case_log="$scratch/partial-publish.log"
run_preparation "$case_log" "the real preparation failed for the partly published head" \
  "fix the phase named above and rerun"
grep -qF "proposed release pull request" "$case_log" || fail \
  "the partly published run did not create a registry-recovery proposal" "advance beyond the tagged version and open its release pull request"
[ "$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)" = "$recovery_version" ] || fail \
  "the partly published run did not select $recovery_version" "advance every crate to the patch after the attempted release"
[ "$(wc -l < "$scratch/state/proposals")" -eq 1 ] || fail \
  "the partly published run created a duplicate pull request" "reuse the existing release pull request during recovery"
changelogs_describe_this_release_at_most_once "$recovery_base" "release-plz-$recovery_version" "$recovery_version"

# The other head the same registry lag describes: merging a release pull request pushes the
# default branch, `release-plz release` tags a version seconds before any registry can hold
# it, and what is there to release is the pipeline's own release commit and nothing else.
# Proposing here is what released v0.2.4 and v0.2.5 from no source change at all, and
# auto-merge fed each proposal straight back in as the next push.
git -C "$repo" switch --quiet "$fixture_base" || fail \
  "could not restore $fixture_base before the release-loop case" "check the scratch repository and rerun"
loop_version="${lagged_version%.*}.$((${lagged_version##*.} + 2))"
(cd "$repo" && scripts/set-version.sh "$loop_version") || fail \
  "could not prepare the release-loop fixture" "fix what set-version.sh named above and rerun"
git -C "$repo" add -A || fail "could not stage the release-loop fixture" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet \
  -m "chore: release v$loop_version" || fail \
  "could not commit the release-loop fixture" "check the scratch repository and rerun"
# Only the release's own tag, which is what a `release-plz release` that did not finish
# leaves behind — and so the head hardest to tell from a publish worth recovering. The real
# tool reports the same registry lag here that it reports for the partly published head
# above; the two are separated by what has landed since the boundary and by nothing else.
git -C "$repo" tag --force "v$loop_version" >/dev/null || fail \
  "could not tag the release-loop fixture" "check the scratch repository and rerun"
: > "$scratch/state/proposals" || fail "could not clear the scratch proposal state" "check scratch-directory permissions and rerun"
case_log="$scratch/release-loop.log"
run_preparation "$case_log" \
  "the real preparation failed for a head carrying only this pipeline's own release commit" \
  "fix the phase named above and rerun"
grep -qF "no release pull request proposed: the registry lags this repository's own release" "$case_log" || fail \
  "the real preparation proposed a release from a head carrying only this pipeline's own release commit" \
  "decline registry recovery unless release-plz.toml's own release_commits policy accepts a commit since the boundary"
[ ! -s "$scratch/state/proposals" ] || fail \
  "the release-loop head opened a pull request, which is the loop that published v0.2.4 and v0.2.5" \
  "propose nothing while the registry lags only this repository's own release"
[ "$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)" = "$loop_version" ] || fail \
  "the release-loop head advanced a manifest past $loop_version" \
  "leave every manifest at its released version when recovery is declined"

# The boundary a successful release leaves behind lives on the origin: `release-plz release`
# creates the tag there, and this same job then runs preparation from a checkout made before
# that tag existed. So every run after a release reaches the selector with the boundary it
# names absent locally — which refused v0.2.2 and v0.2.6 and made a workflow that fails
# exactly when it worked. Resolving that boundary must decide what the local one decided, so
# one head is driven both ways and the two decisions are compared rather than described.
git -C "$repo" switch --quiet "$fixture_base" || fail \
  "could not restore $fixture_base before the remote-only boundary case" "check the scratch repository and rerun"
# The declining run above left release-plz's changelog edit in the tree, and the real tool
# refuses to decide anything from a dirty checkout.
git -C "$repo" checkout --quiet -- . || fail \
  "could not restore the fixture before the remote-only boundary case" "check the scratch repository and rerun"
printf '\n' >> "$repo/crates/onetaskgraph-core/src/lib.rs" || fail \
  "could not modify the remote-only boundary fixture" "check scratch-directory permissions"
git -C "$repo" add crates/onetaskgraph-core/src/lib.rs || fail \
  "could not stage the remote-only boundary fixture" "check the scratch repository and rerun"
git -C "$repo" -c user.name=check -c user.email=check@example.invalid commit --quiet \
  -m "fix(core): recover from a boundary only the origin holds" || fail \
  "could not commit the remote-only boundary fixture" "check the scratch repository and rerun"
remote_only_version="${loop_version%.*}.$((${loop_version##*.} + 1))"

local_boundary_log="$scratch/local-boundary-selector.log"
if ! (cd "$repo" && scripts/select-release-version.sh) > "$local_boundary_log" 2>&1; then
  sed 's/^/    /' "$local_boundary_log" >&2
  fail "the real selector failed with its boundary tag in the checkout" "fix what it reports above and rerun"
fi
grep -qF "select-release-version: registry recovery selected $loop_version -> $remote_only_version" "$local_boundary_log" || fail \
  "the boundary-in-checkout run reached some decision other than registry recovery: $(cat "$local_boundary_log")" \
  "keep this fixture's registry lag pinned so both boundary shapes drive the same decision"
git -C "$repo" checkout --quiet -- . || fail \
  "could not restore the fixture after the boundary-in-checkout run" "check the scratch repository and rerun"

publish_boundary_only_to_origin() {
  git -C "$repo" push --quiet origin "refs/tags/v$loop_version" || fail \
    "could not publish the boundary tag to the fixture origin" "check the scratch repository and rerun"
  git -C "$repo" tag -d "v$loop_version" >/dev/null || fail \
    "could not remove the local boundary tag" "check the scratch repository and rerun"
  if git -C "$repo" rev-parse --verify --quiet "refs/tags/v$loop_version" >/dev/null; then
    fail "the boundary tag is still in the checkout, so this case does not model a post-release run" \
      "remove the local tag before driving the remote-only boundary"
  fi
  git -C "$repo" ls-remote --exit-code origin "refs/tags/v$loop_version" >/dev/null || fail \
    "the fixture origin does not hold the boundary tag, so this case would refuse for the wrong reason" \
    "publish the boundary tag to the fixture origin before driving it"
}
publish_boundary_only_to_origin

remote_boundary_log="$scratch/remote-boundary-selector.log"
if ! (cd "$repo" && scripts/select-release-version.sh) > "$remote_boundary_log" 2>&1; then
  sed 's/^/    /' "$remote_boundary_log" >&2
  fail "the real selector refused a boundary the origin holds, which is every run after a release cuts its tag" \
    "resolve the release boundary from the origin before refusing it as unknown"
fi
diff "$local_boundary_log" "$remote_boundary_log" >/dev/null || fail \
  "resolving the boundary from the origin changed the selector's decision: $(cat "$remote_boundary_log")" \
  "resolve the boundary into the same ref, so the commits since it are the same set either way"
git -C "$repo" checkout --quiet -- . || fail \
  "could not restore the fixture after the remote-only selector run" "check the scratch repository and rerun"

# And the whole preparation path over the same head, which is what the workflow runs.
publish_boundary_only_to_origin
: > "$scratch/state/proposals" || fail "could not clear the scratch proposal state" "check scratch-directory permissions and rerun"
remote_boundary_base="$(git -C "$repo" rev-parse HEAD)" || fail \
  "could not read the head whose boundary only the origin holds" "check the scratch repository and rerun"
case_log="$scratch/remote-boundary.log"
run_preparation "$case_log" \
  "the real preparation failed for a head whose release boundary is only on the origin" \
  "fix the phase named above and rerun"
grep -qF "proposed release pull request" "$case_log" || fail \
  "the remote-only boundary run did not propose a release" \
  "resolve the boundary from the origin and propose the recovery it selects"
[ "$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)" = "$remote_only_version" ] || fail \
  "the remote-only boundary run did not select $remote_only_version" \
  "advance every crate to the patch after the attempted release"
changelogs_describe_this_release_at_most_once "$remote_boundary_base" "release-plz-$remote_only_version" "$remote_only_version"

# A boundary neither the checkout nor the origin holds is genuinely unknown, and is still
# refused naming both places it was looked for: guessing one proposes a version against
# nothing at all.
git -C "$repo" switch --quiet "$fixture_base" || fail \
  "could not restore $fixture_base before the missing-boundary case" "check the scratch repository and rerun"
git -C "$repo" tag -d "v$loop_version" >/dev/null || fail \
  "could not remove the boundary tag the preparation run resolved" "check the scratch repository and rerun"
git -C "$repo" push --quiet --delete origin "refs/tags/v$loop_version" || fail \
  "could not remove the boundary tag from the fixture origin" "check the scratch repository and rerun"
: > "$scratch/state/proposals" || fail "could not clear the scratch proposal state" "check scratch-directory permissions and rerun"
case_log="$scratch/missing-boundary.log"
missing_status=0
(cd "$repo" && scripts/prepare-release-pr.sh) > "$case_log" 2>&1 || missing_status=$?
guard_hermetic
[ "$missing_status" -ne 0 ] || fail \
  "the real preparation accepted a boundary neither the checkout nor the origin holds" \
  "keep refusing an unresolvable release boundary rather than guessing one"
grep -qF "v$loop_version is in neither this checkout nor origin, so the release boundary is unknown" "$case_log" || fail \
  "the missing-boundary refusal did not name the tag and where it was looked for: $(cat "$case_log")" \
  "refuse naming the missing boundary tag, this checkout and the origin"
[ ! -s "$scratch/state/proposals" ] || fail \
  "the missing-boundary head opened a pull request from a boundary nothing holds" \
  "propose nothing when the release boundary cannot be resolved"
