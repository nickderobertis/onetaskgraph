#!/usr/bin/env bash
# Drive scripts/scoped-release-plz.sh against a stand-in tool location and installer, and
# hold every release-plz invocation of this repository to it.
#
# What #1990 cost is stated in that script's header: bootstrap installed the pin into the
# global cargo bin directory, which other repositories on this host pin other versions of
# the same command into, so bootstrapping one repository invalidated another's gate. The
# cases below prove the replacement from outside, the way its callers meet it:
#
#   1. the location is namespaced by tool and version under the cache home, is the same
#      from every checkout, and is never inside one;
#   2. an empty location is refused naming the tool, the version, the path and the install
#      command — and never with the host-prerequisite marker, which is the hook's alone;
#   3. `ensure` provisions through cargo-binstall when it is there and `cargo install` when
#      it is not — or when binstall could not fetch the binary — into that location and
#      nowhere else, and proves the result by asking it;
#   4. a provisioned location is found again without installing — from the same checkout,
#      and from a fresh one on the same host;
#   5. residue of another version there is replaced, an installer that lands the wrong
#      version or a binary that cannot answer is refused rather than trusted, a failed
#      installation says which installers failed, a host with no cargo is told what to
#      install, a location that cannot be cleared or created is named, the Windows `.exe`
#      layout resolves, and a host with no cache home at all is refused naming the variables;
#   6. a release-plz on PATH is neither consulted nor touched, whatever its version;
#   7. nothing in scripts/, the justfile, the hooks or the workflows invokes release-plz
#      except through that script, so a caller cannot quietly reach for the global one;
#   8. the check that drives the real tool refuses an empty location about the machine, in
#      its own words, without the marker.
#
# The installer is a stand-in because the real one reaches crates.io or GitHub and builds
# for minutes; what is under test is this repository's own script, which is real here, and
# the real installation is proven where the binary is used, by
# scripts/check-real-release-preparation.sh resolving the real pinned tool on every gate.
set -euo pipefail

fatal() {
  echo "check-scoped-release-plz: $1" >&2
  echo "check-scoped-release-plz: next: $2" >&2
  exit 2
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just check' does"
readonly ROOT
readonly RESOLVER="scripts/scoped-release-plz.sh"
readonly MARKER="onevcs: host-prerequisite:"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check provisions into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

pin="$(bash "$ROOT/$RESOLVER" pin)" || fatal \
  "$RESOLVER did not answer 'pin'" "restore that script and rerun"
[[ $pin =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fatal \
  "$RESOLVER pins '$pin', which is not an exact X.Y.Z version" "restore its RELEASE_PLZ_VERSION and rerun"

failures=0
report() {
  failures=$((failures + 1))
  echo "check-scoped-release-plz: $1" >&2
}
quote() { printf '%s\n' "$1" | sed 's/^/    /' >&2; }

# The stand-in installer. `cargo binstall` and `cargo install` are both cargo subcommands,
# so one stand-in `cargo` ahead of the real one on PATH meets both, records what it was
# asked, and writes a release-plz answering `--version` with whatever STANDIN_ANSWERS says —
# the pin unless a case asks for the wrong version. cargo dispatches `cargo binstall` to a
# `cargo-binstall` on PATH, and the resolver asks `command -v cargo-binstall` first, so the
# branch under test is chosen by whether that name is reachable.
readonly STANDIN_BIN="$scratch/standin-bin"
readonly WITH_BINSTALL="$scratch/with-binstall"
readonly INSTALL_LOG="$scratch/installer-calls"
mkdir -p "$STANDIN_BIN" "$WITH_BINSTALL" || fatal \
  "could not create the stand-in directories" "check the permissions of \$TMPDIR, then rerun"
cat > "$STANDIN_BIN/cargo" <<'CARGO'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$STANDIN_INSTALL_LOG"
destination=""
version=""
case "${1:-}" in
  binstall)
    if [ "${STANDIN_BINSTALL_FAILS:-}" = yes ]; then
      echo "cargo stand-in: binstall could not fetch the published binary" >&2
      exit 1
    fi
    shift
    while [ $# -gt 0 ]; do
      case "$1" in
        --install-path) destination="$2"; shift ;;
        --version) version="$2"; shift ;;
      esac
      shift
    done
    ;;
  install)
    if [ "${STANDIN_INSTALL_FAILS:-}" = yes ]; then
      echo "cargo stand-in: the source build failed" >&2
      exit 101
    fi
    shift
    while [ $# -gt 0 ]; do
      case "$1" in
        --root) destination="$2/bin"; shift ;;
        --version) version="$2"; shift ;;
      esac
      shift
    done
    ;;
  *)
    echo "cargo stand-in: unexpected subcommand '${1:-}'" >&2
    exit 2
    ;;
esac
[ -n "$destination" ] && [ -n "$version" ] || {
  echo "cargo stand-in: no destination or no version in: $*" >&2
  exit 2
}
mkdir -p "$destination"
if [ "${STANDIN_BROKEN_BINARY:-}" = yes ]; then
  printf '#!/usr/bin/env bash\nexit 1\n' > "$destination/release-plz"
else
  printf '#!/usr/bin/env bash\necho "release-plz %s"\n' "${STANDIN_ANSWERS:-$version}" > "$destination/release-plz"
fi
chmod +x "$destination/release-plz"
CARGO
printf '#!/usr/bin/env bash\nexit 0\n' > "$WITH_BINSTALL/cargo-binstall"
chmod +x "$STANDIN_BIN/cargo" "$WITH_BINSTALL/cargo-binstall" || fatal \
  "could not make the stand-ins executable" "check the permissions of \$TMPDIR, then rerun"
export STANDIN_INSTALL_LOG="$INSTALL_LOG"

# The ambient PATH with every directory that carries cargo-binstall subtracted, so the
# "host without it" case is that host whatever this machine has installed. Whole entries
# rather than a whitelist of links, because Git Bash on Windows needs the DLLs beside its
# own installation.
path_without_tool() {
  local tool="$1" entry result=""
  local IFS=:
  for entry in $PATH; do
    [ -n "$entry" ] || continue
    ( PATH="$entry"; hash -r 2>/dev/null; command -v "$tool" >/dev/null 2>&1 ) && continue
    result="${result:+$result:}$entry"
  done
  printf '%s' "$result"
}
PATH_WITHOUT_BINSTALL="$(path_without_tool cargo-binstall)"
readonly PATH_WITHOUT_BINSTALL
if ( PATH="$PATH_WITHOUT_BINSTALL"; hash -r 2>/dev/null; command -v cargo-binstall >/dev/null 2>&1 ); then
  fatal "cargo-binstall is still reachable after dropping every directory that carries one, so the source-build case cannot pose its question" \
    "run 'command -v cargo-binstall' and remove any shell function or alias that reaches past the PATH scan"
fi

# Run the resolver from a checkout, under a tool home this case chose, with the installer
# stand-ins ahead of everything real. `WITH` is the directory that makes cargo-binstall
# reachable, or empty for a host without it — and then the ambient PATH is the one with
# cargo-binstall subtracted, so "without" means without.
OUTPUT=""
STATUS=0
resolver() {
  local where="$1" home="$2" with="$3" base="$PATH"
  shift 3
  [ -n "$with" ] || base="$PATH_WITHOUT_BINSTALL"
  OUTPUT="$(cd "$where" && PATH="${with:+$with:}$STANDIN_BIN:$base" ONETASKGRAPH_TOOLS_HOME="$home" \
    bash "$RESOLVER" "$@" 2>&1)" && STATUS=0 || STATUS=$?
}
installer_calls() { [ -f "$INSTALL_LOG" ] && wc -l < "$INSTALL_LOG" | tr -d ' ' || echo 0; }

# A second checkout of the same tree, for the fresh-worktree cases.
readonly OTHER="$scratch/other-checkout"
mkdir -p "$OTHER" || fatal "could not create $OTHER" "check the permissions of \$TMPDIR, then rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$OTHER" || fatal \
  "could not copy $ROOT's tracked files into $OTHER" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

# 1. The location.
home="$scratch/home-1"
resolver "$ROOT" "$home" ""  path
expected="$home/release-plz/$pin/bin/release-plz"
if [ "$STATUS" -ne 0 ] || [ "$OUTPUT" != "$expected" ]; then
  report "the scoped path under ONETASKGRAPH_TOOLS_HOME=$home is '$OUTPUT', expected $expected — the location has to be namespaced by tool and version so two pins on one host never collide"
fi
resolver "$OTHER" "$home" "" path
[ "$OUTPUT" = "$expected" ] || report "a second checkout resolves the scoped path to '$OUTPUT' rather than $expected, so a fresh worktree would not find what the last one provisioned"
case "$OUTPUT" in
  "$ROOT"/* | "$OTHER"/*) report "the scoped path $OUTPUT is inside a checkout, so every worktree and every publication's scratch clone would provision its own" ;;
esac
cache_home="$scratch/xdg"
OUTPUT="$(cd "$ROOT" && env -u ONETASKGRAPH_TOOLS_HOME XDG_CACHE_HOME="$cache_home" bash "$RESOLVER" path 2>&1)" || true
[ "$OUTPUT" = "$cache_home/onetaskgraph/tools/release-plz/$pin/bin/release-plz" ] || report \
  "with no ONETASKGRAPH_TOOLS_HOME the scoped path is '$OUTPUT', which is not under XDG_CACHE_HOME/onetaskgraph/tools — the cache home is where a worktree-independent location has to live"
OUTPUT="$(cd "$ROOT" && env -u ONETASKGRAPH_TOOLS_HOME -u XDG_CACHE_HOME HOME="$scratch/fake-home" bash "$RESOLVER" path 2>&1)" || true
[ "$OUTPUT" = "$scratch/fake-home/.cache/onetaskgraph/tools/release-plz/$pin/bin/release-plz" ] || report \
  "with neither ONETASKGRAPH_TOOLS_HOME nor XDG_CACHE_HOME the scoped path is '$OUTPUT', expected it under \$HOME/.cache/onetaskgraph/tools"

# 2. An empty location is refused, in full, and without the marker.
resolver "$ROOT" "$home" "" resolve
if [ "$STATUS" -ne 69 ]; then
  report "resolving an empty location exited $STATUS rather than 69 (EX_UNAVAILABLE). It said:"
  quote "$OUTPUT"
fi
for term in "release-plz $pin" "$expected" "scoped-release-plz.sh ensure"; do
  grep -qF -- "$term" <<<"$OUTPUT" || {
    report "the refusal of an empty location never names '$term', so a reader is not told what is missing, where, or how to install it. It said:"
    quote "$OUTPUT"
  }
done
if grep -qF -- "$MARKER" <<<"$OUTPUT"; then
  report "the resolver printed the host-prerequisite marker itself; that line is scripts/provision-gate.sh's alone, because a check that reads the tree must never emit it"
fi
[ ! -e "$home" ] || report "resolving created $home, but a resolve writes nothing"
resolver "$ROOT" "$home" "" run --version
[ "$STATUS" -eq 69 ] || report "'run' against an empty location exited $STATUS rather than 69"

# 3a. Provisioning through cargo-binstall.
rm -f "$INSTALL_LOG"
resolver "$ROOT" "$home" "$WITH_BINSTALL" ensure
if [ "$STATUS" -ne 0 ] || [ -n "$OUTPUT" ]; then
  report "ensure with cargo-binstall reachable exited $STATUS and said '$OUTPUT'; it has to provision quietly"
fi
if [ "$(installer_calls)" != 1 ] || ! grep -qF -- "binstall release-plz --version $pin --install-path $home/release-plz/$pin/bin" "$INSTALL_LOG" \
  || ! grep -qF -- "--no-track" "$INSTALL_LOG"; then
  report "ensure did not make exactly one cargo-binstall call into the scoped location with --no-track. The installer saw:"
  quote "$(cat "$INSTALL_LOG" 2>/dev/null)"
fi
[ -x "$expected" ] || report "ensure reported success but $expected is not there"
resolver "$ROOT" "$home" "" resolve
[ "$STATUS" -eq 0 ] && [ "$OUTPUT" = "$expected" ] || report "after ensure, resolve answered '$OUTPUT' (status $STATUS) rather than $expected"
resolver "$ROOT" "$home" "" run --version
[ "$STATUS" -eq 0 ] && [ "$OUTPUT" = "release-plz $pin" ] || report "'run --version' answered '$OUTPUT' (status $STATUS) rather than the scoped binary's 'release-plz $pin'"

# 4. Found again without installing — here, and from a fresh checkout.
rm -f "$INSTALL_LOG"
resolver "$ROOT" "$home" "$WITH_BINSTALL" ensure
[ "$STATUS" -eq 0 ] && [ "$(installer_calls)" = 0 ] || report "a second ensure reinstalled (status $STATUS, $(installer_calls) installer call(s)); an already-provisioned pin is left alone"
resolver "$OTHER" "$home" "$WITH_BINSTALL" ensure
[ "$STATUS" -eq 0 ] && [ "$(installer_calls)" = 0 ] || report "a fresh checkout's ensure reinstalled (status $STATUS, $(installer_calls) installer call(s)); a host that holds the pin already provisions nothing"
resolver "$OTHER" "$home" "" resolve
[ "$STATUS" -eq 0 ] && [ "$OUTPUT" = "$expected" ] || report "a fresh checkout resolved '$OUTPUT' (status $STATUS) rather than the binary the first one provisioned"

# 3b. Provisioning without cargo-binstall builds from source, into the same place.
home="$scratch/home-2"
expected="$home/release-plz/$pin/bin/release-plz"
rm -f "$INSTALL_LOG"
resolver "$ROOT" "$home" "" ensure
[ "$STATUS" -eq 0 ] && [ -z "$OUTPUT" ] || report "ensure without cargo-binstall exited $STATUS and said '$OUTPUT'; it has to fall back to cargo install quietly"
if [ "$(installer_calls)" != 1 ] || ! grep -qF -- "install release-plz --version $pin --locked --root $home/release-plz/$pin" "$INSTALL_LOG"; then
  report "ensure without cargo-binstall did not make exactly one 'cargo install --locked --root <scoped root>' call. The installer saw:"
  quote "$(cat "$INSTALL_LOG" 2>/dev/null)"
fi
resolver "$ROOT" "$home" "" resolve
[ "$STATUS" -eq 0 ] && [ "$OUTPUT" = "$expected" ] || report "after the source build, resolve answered '$OUTPUT' (status $STATUS) rather than $expected"

# 3c. cargo-binstall is there but cannot fetch the binary: the build from source recovers,
#     into the same place, saying in one line which installer it fell back from.
home="$scratch/home-2b"
expected="$home/release-plz/$pin/bin/release-plz"
rm -f "$INSTALL_LOG"
OUTPUT="$(cd "$ROOT" && PATH="$WITH_BINSTALL:$STANDIN_BIN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" STANDIN_BINSTALL_FAILS=yes \
  bash "$RESOLVER" ensure 2>&1)" && STATUS=0 || STATUS=$?
if [ "$STATUS" -ne 0 ] || [ "$(installer_calls)" != 2 ] || ! grep -qF -- "install release-plz --version $pin --locked --root $home/release-plz/$pin" "$INSTALL_LOG"; then
  report "ensure with a failing cargo-binstall exited $STATUS with $(installer_calls) installer call(s); it has to fall back to 'cargo install --root <scoped root>'. It said:"
  quote "$OUTPUT"
  quote "$(cat "$INSTALL_LOG" 2>/dev/null)"
fi
[ "$(printf '%s\n' "$OUTPUT" | grep -c .)" -eq 1 ] && grep -qF -- "building it from source instead" <<<"$OUTPUT" || {
  report "ensure recovered from a failing cargo-binstall but did not say so in exactly one line. It said:"
  quote "$OUTPUT"
}
resolver "$ROOT" "$home" "" resolve
[ "$STATUS" -eq 0 ] && [ "$OUTPUT" = "$expected" ] || report "after the fallback build, resolve answered '$OUTPUT' (status $STATUS) rather than $expected"

# 5a. Residue of another version at the scoped path is replaced.
printf '#!/usr/bin/env bash\necho "release-plz 0.0.0"\n' > "$expected"
resolver "$ROOT" "$home" "" resolve
if [ "$STATUS" -ne 69 ] || ! grep -qF -- "found: release-plz 0.0.0" <<<"$OUTPUT"; then
  report "a wrong-version binary at the scoped path was not refused naming what it answers (status $STATUS). It said:"
  quote "$OUTPUT"
fi
rm -f "$INSTALL_LOG"
resolver "$ROOT" "$home" "$WITH_BINSTALL" ensure
[ "$STATUS" -eq 0 ] && [ "$(installer_calls)" = 1 ] || report "ensure over wrong-version residue exited $STATUS with $(installer_calls) installer call(s); the residue has to be replaced"
resolver "$ROOT" "$home" "" run --version
[ "$OUTPUT" = "release-plz $pin" ] || report "after replacing the residue, the scoped binary answers '$OUTPUT'"

# 5b. An installer that lands the wrong version is refused, not trusted.
home="$scratch/home-3"
resolver_wrong() {
  OUTPUT="$(cd "$ROOT" && PATH="$WITH_BINSTALL:$STANDIN_BIN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" STANDIN_ANSWERS=0.0.0 \
    bash "$RESOLVER" ensure 2>&1)" && STATUS=0 || STATUS=$?
}
resolver_wrong
if [ "$STATUS" -eq 0 ] || ! grep -qF -- "release-plz 0.0.0" <<<"$OUTPUT" || ! grep -qF -- "release-plz $pin" <<<"$OUTPUT"; then
  report "an installer that landed release-plz 0.0.0 was not refused naming both versions (status $STATUS). It said:"
  quote "$OUTPUT"
fi
resolver "$ROOT" "$home" "" resolve
[ "$STATUS" -eq 69 ] || report "after a wrong-version installation, resolve exited $STATUS rather than refusing"

# 6. A release-plz on PATH, at the pinned version, is neither consulted nor touched.
readonly GLOBAL_BIN="$scratch/global-bin"
mkdir -p "$GLOBAL_BIN" || fatal "could not create $GLOBAL_BIN" "check the permissions of \$TMPDIR, then rerun"
printf '#!/usr/bin/env bash\necho "release-plz %s"\n' "$pin" > "$GLOBAL_BIN/release-plz"
chmod +x "$GLOBAL_BIN/release-plz"
global_before="$(cat "$GLOBAL_BIN/release-plz")"
home="$scratch/home-4"
OUTPUT="$(cd "$ROOT" && PATH="$GLOBAL_BIN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" bash "$RESOLVER" resolve 2>&1)" && STATUS=0 || STATUS=$?
[ "$STATUS" -eq 69 ] || report "with the pinned version on PATH and nothing in the scoped location, resolve exited $STATUS rather than refusing — the global command is never consulted"
rm -f "$INSTALL_LOG"
OUTPUT="$(cd "$ROOT" && PATH="$GLOBAL_BIN:$WITH_BINSTALL:$STANDIN_BIN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" bash "$RESOLVER" ensure 2>&1)" && STATUS=0 || STATUS=$?
[ "$STATUS" -eq 0 ] && [ "$(installer_calls)" = 1 ] || report "with the pinned version on PATH, ensure exited $STATUS with $(installer_calls) installer call(s); it has to provision the scoped copy regardless"
[ "$(cat "$GLOBAL_BIN/release-plz")" = "$global_before" ] || report "ensure rewrote the release-plz on PATH"
if grep -v -- "$home/" "$INSTALL_LOG" | grep -q .; then
  report "an installer call named a destination outside the scoped home:"
  quote "$(cat "$INSTALL_LOG")"
fi
OUTPUT="$(cd "$ROOT" && PATH="$GLOBAL_BIN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" bash "$RESOLVER" resolve 2>&1)" || true
[ "$OUTPUT" = "$home/release-plz/$pin/bin/release-plz" ] || report "resolve answered '$OUTPUT' rather than the scoped copy, with the pinned version on PATH"

# 5c. An installer that lands a binary which cannot answer at all is refused the same way,
#     and so is such a binary found at the scoped path.
home="$scratch/home-3b"
OUTPUT="$(cd "$ROOT" && PATH="$WITH_BINSTALL:$STANDIN_BIN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" STANDIN_BROKEN_BINARY=yes \
  bash "$RESOLVER" ensure 2>&1)" && STATUS=0 || STATUS=$?
if [ "$STATUS" -eq 0 ] || ! grep -qF -- "answers 'nothing'" <<<"$OUTPUT"; then
  report "an installer that landed a binary which cannot start was not refused naming that it answers nothing (status $STATUS). It said:"
  quote "$OUTPUT"
fi
resolver "$ROOT" "$home" "" resolve
[ "$STATUS" -eq 69 ] && grep -qF -- "found: nothing" <<<"$OUTPUT" || report "a scoped binary that cannot start was not refused as 'found: nothing' (status $STATUS): $OUTPUT"

# 5d. The source build fails: refused naming the failure, and — when binstall failed first —
#     binstall's own diagnostic is kept beside it, so a reader sees both causes.
home="$scratch/home-3c"
OUTPUT="$(cd "$ROOT" && PATH="$WITH_BINSTALL:$STANDIN_BIN:$PATH" ONETASKGRAPH_TOOLS_HOME="$home" STANDIN_BINSTALL_FAILS=yes STANDIN_INSTALL_FAILS=yes \
  bash "$RESOLVER" ensure 2>&1)" && STATUS=0 || STATUS=$?
for term in "installation into $home/release-plz/$pin failed" "binstall could not fetch the published binary" "the source build failed"; do
  [ "$STATUS" -ne 0 ] && grep -qF -- "$term" <<<"$OUTPUT" || {
    report "a source build failing after binstall failed was not refused naming '$term' (status $STATUS). It said:"
    quote "$OUTPUT"
  }
done
[ ! -e "$home/release-plz/$pin/bin/release-plz" ] || report "a failed installation left a binary at the scoped path"

# 5e. No cargo at all, and no cargo-binstall: nothing can install, and the refusal names
#     the toolchain to install. The stand-in cargo is left off PATH for this one case.
home="$scratch/home-3d"
PATH_WITHOUT_CARGO="$(path_without_tool cargo)"
if ( PATH="$PATH_WITHOUT_CARGO"; hash -r 2>/dev/null; command -v cargo >/dev/null 2>&1 || command -v cargo-binstall >/dev/null 2>&1 ); then
  report "cargo or cargo-binstall is still reachable after dropping every directory that carries cargo, so the no-toolchain case cannot pose its question"
else
  OUTPUT="$(cd "$ROOT" && PATH="$PATH_WITHOUT_CARGO" ONETASKGRAPH_TOOLS_HOME="$home" bash "$RESOLVER" ensure 2>&1)" && STATUS=0 || STATUS=$?
  [ "$STATUS" -eq 1 ] && grep -qF -- "cargo is not on PATH" <<<"$OUTPUT" && grep -qF -- "rustup.rs" <<<"$OUTPUT" || {
    report "ensure with no cargo on PATH exited $STATUS without naming the toolchain to install. It said:"
    quote "$OUTPUT"
  }
fi

# 5f. A location that cannot be cleared or created is refused naming it. Posed by taking
#     write permission away from the tool's directory, which a root user and a Windows
#     filesystem do not honour — so the case says when it could not be posed rather than
#     passing quietly.
home="$scratch/home-3e"
mkdir -p "$home/release-plz/$pin/bin"
printf '#!/usr/bin/env bash\necho "release-plz 0.0.0"\n' > "$home/release-plz/$pin/bin/release-plz"
chmod +x "$home/release-plz/$pin/bin/release-plz"
chmod a-w "$home/release-plz"
if rm -rf "$home/release-plz/$pin" 2>/dev/null; then
  echo "check-scoped-release-plz: note: this filesystem or user ignores a read-only directory, so the unclearable-residue and uncreatable-directory cases were not posed here" >&2
  chmod u+w "$home/release-plz"
else
  resolver "$ROOT" "$home" "$WITH_BINSTALL" ensure
  [ "$STATUS" -eq 1 ] && grep -qF -- "could not clear $home/release-plz/$pin" <<<"$OUTPUT" || {
    report "residue that cannot be cleared was not refused naming it (status $STATUS). It said:"
    quote "$OUTPUT"
  }
  chmod u+w "$home/release-plz"
  rm -rf "$home/release-plz/$pin"
  chmod a-w "$home/release-plz"
  resolver "$ROOT" "$home" "$WITH_BINSTALL" ensure
  [ "$STATUS" -eq 1 ] && grep -qF -- "could not create $home/release-plz/$pin/bin" <<<"$OUTPUT" || {
    report "a scoped directory that cannot be created was not refused naming it (status $STATUS). It said:"
    quote "$OUTPUT"
  }
  chmod u+w "$home/release-plz"
fi

# 5g. The Windows layout: `cargo install` writes release-plz.exe there, and the resolver
#     names that file rather than the bare name bash would resolve to it on Windows alone.
#     Posed with a file of that name, which is the whole of what the resolution reads.
home="$scratch/home-3f"
mkdir -p "$home/release-plz/$pin/bin"
printf '#!/usr/bin/env bash\necho "release-plz %s"\n' "$pin" > "$home/release-plz/$pin/bin/release-plz.exe"
chmod +x "$home/release-plz/$pin/bin/release-plz.exe"
resolver "$ROOT" "$home" "" resolve
[ "$STATUS" -eq 0 ] && [ "$OUTPUT" = "$home/release-plz/$pin/bin/release-plz.exe" ] || report "a release-plz.exe at the scoped path resolved to '$OUTPUT' (status $STATUS) rather than itself"
rm -f "$INSTALL_LOG"
resolver "$ROOT" "$home" "$WITH_BINSTALL" ensure
[ "$STATUS" -eq 0 ] && [ "$(installer_calls)" = 0 ] || report "ensure reinstalled over a provisioned release-plz.exe (status $STATUS, $(installer_calls) installer call(s))"

# 5h. No cache home at all: nothing to scope under, and the refusal says which variables
#     would supply one.
OUTPUT="$(cd "$ROOT" && env -u ONETASKGRAPH_TOOLS_HOME -u XDG_CACHE_HOME -u HOME bash "$RESOLVER" path 2>&1)" && STATUS=0 || STATUS=$?
[ "$STATUS" -eq 69 ] && grep -qF -- "none of ONETASKGRAPH_TOOLS_HOME, XDG_CACHE_HOME and HOME is set" <<<"$OUTPUT" || {
  report "with no cache-home variable set, the resolver exited $STATUS without naming them. It said:"
  quote "$OUTPUT"
}

# A tool home that is not an absolute path names nothing this may create or clear under.
resolver "$ROOT" "relative/tools" "" path
[ "$STATUS" -eq 69 ] && grep -qF -- "not an absolute path" <<<"$OUTPUT" || report "a relative ONETASKGRAPH_TOOLS_HOME was not refused (status $STATUS): $OUTPUT"
resolver "$ROOT" "relative/tools" "$WITH_BINSTALL" ensure
[ "$STATUS" -eq 69 ] && [ ! -e "$ROOT/relative" ] || report "ensure under a relative tool home exited $STATUS and left $ROOT/relative behind"

# Usage.
resolver "$ROOT" "$home" "" frobnicate
[ "$STATUS" -eq 64 ] || report "an unknown subcommand exited $STATUS rather than 64 (EX_USAGE)"
resolver "$ROOT" "$home" "" run
[ "$STATUS" -eq 64 ] || report "'run' with nothing to run exited $STATUS rather than 64 (EX_USAGE)"

# 7. Every invocation goes through the resolver. Command positions only: the stand-ins and
#    the diagnostics in the checks name the tool in prose and in strings, which is not a
#    call. Enumerated from the tree rather than listed, so a caller added later is held too.
if ! (cd "$ROOT" && python3 - "$RESOLVER" "$MARKER" <<'PY'
import re
import subprocess
import sys
from pathlib import Path

resolver, marker = sys.argv[1], sys.argv[2]
tracked = subprocess.run(
    ["git", "ls-files", "-z", "--", "scripts", "justfile", ".githooks", ".github/workflows"],
    check=True, capture_output=True,
).stdout.decode("utf-8").split("\0")
# A bare `release-plz` at a command position: the start of a line or of a compound, after
# `if`/`!`, inside `$( )`, as a `run:` step, or looked up with `command -v`. Single-quoted
# text is dropped first — nothing runs inside it, and the checks carry workflow fixtures
# there — while double-quoted text is kept, because `"$(release-plz ...)"` is a call.
SINGLE_QUOTED = re.compile(r"'[^']*'")
INVOCATION = re.compile(
    r"(?:^|[;&|(]|\$\(|\brun:)\s*(?:if\s+!?\s*|!\s*|env\s+(?:\S+=\S+\s+)*)?release-plz(?=\s|$)"
    r"|command -v release-plz\b"
)
problems = []
marker_sites = []
for path in tracked:
    if not path:
        continue
    text = Path(path).read_text(encoding="utf-8", errors="replace")
    for number, line in enumerate(text.splitlines(), 1):
        code = "" if line.lstrip().startswith("#") else SINGLE_QUOTED.sub("", line)
        if path not in (resolver, "scripts/check-scoped-release-plz.sh") and INVOCATION.search(code):
            problems.append(f"{path}:{number}: {line.strip()}")
        if marker in line and not line.lstrip().startswith("#"):
            marker_sites.append(path)
if problems:
    print("check-scoped-release-plz: a release-plz invocation goes around the resolver:", file=sys.stderr)
    for problem in problems:
        print(f"    {problem}", file=sys.stderr)
    print(f"check-scoped-release-plz: next: resolve it through {resolver} instead, so it runs the scoped pinned binary", file=sys.stderr)
    raise SystemExit(1)
# The provisioner that emits the marker, and the checks that read it: two that drive the
# hook and assert it is printed exactly once when the pinned tool is missing, and
# check-visual-guard.sh, which asserts the screenshot guard NEVER prints it — a missing
# renderer is not a host problem the engine should stop retrying on.
expected = {
    "scripts/provision-gate.sh",
    "scripts/check-scoped-release-plz.sh",
    "scripts/check-pre-push-provisioning.sh",
    "scripts/check-visual-guard.sh",
}
stray = sorted(set(marker_sites) - expected)
if stray:
    print("check-scoped-release-plz: the host-prerequisite marker is spelled outside the provisioner and its checks:", file=sys.stderr)
    for site in stray:
        print(f"    {site}", file=sys.stderr)
    print("check-scoped-release-plz: next: only scripts/provision-gate.sh emits that line, because a check that reads the tree must never claim a host problem", file=sys.stderr)
    raise SystemExit(1)
if "scripts/provision-gate.sh" not in marker_sites:
    print("check-scoped-release-plz: scripts/provision-gate.sh no longer spells the host-prerequisite marker, so the pre-push gate cannot report a missing release-plz as a host problem", file=sys.stderr)
    raise SystemExit(1)
PY
); then
  failures=$((failures + 1))
fi

# 8. The check that drives the real tool refuses an empty location about the machine, and
#    without the marker. It reads the candidate tree, so a marker from it would tell the
#    engine on this host to stop retrying a tree it never finished checking.
real_output="$(cd "$ROOT" && ONETASKGRAPH_TOOLS_HOME="$scratch/home-empty" bash scripts/check-real-release-preparation.sh 2>&1)" && real_status=0 || real_status=$?
case "$real_output" in
  *"skipped on Windows"*) ;;
  *)
    if [ "$real_status" -eq 0 ]; then
      report "scripts/check-real-release-preparation.sh passed with nothing in the scoped location, so it drove something other than the scoped pinned tool"
    elif ! grep -qF -- "release-plz $pin is not provisioned" <<<"$real_output"; then
      report "scripts/check-real-release-preparation.sh refused an empty scoped location without naming it. It said:"
      quote "$real_output"
    fi
    if grep -qF -- "$MARKER" <<<"$real_output"; then
      report "scripts/check-real-release-preparation.sh printed the host-prerequisite marker; it reads the candidate tree, so that line is not its to print"
    fi
    ;;
esac

if [ "$failures" -ne 0 ]; then
  echo "check-scoped-release-plz: $failures case(s) failed." >&2
  echo "check-scoped-release-plz: repair scripts/scoped-release-plz.sh or the caller named above; the" >&2
  echo "check-scoped-release-plz: scoped location is what keeps this repository's pin off the host-global one." >&2
  exit 1
fi
