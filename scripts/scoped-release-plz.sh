#!/usr/bin/env bash
# The ONE implementation of which release-plz this repository runs and where it lives.
#
# Every recipe, hook and script here that runs release-plz asks this script for the binary,
# and `just bootstrap` provisions it through this script — so the version is pinned in
# exactly one place, RELEASE_PLZ_VERSION below, and every caller reads it rather than
# restating it. scripts/check-release-pr-sync.sh and scripts/check-release-tooling-selection.sh
# reconcile that pin against the version their stand-ins were recorded from, so moving it
# means re-observing the real tool.
#
# The binary lives in a REPOSITORY-SCOPED location, never on PATH. Bootstrap used to install
# the pin into the caller's global cargo bin directory, and other repositories on the same
# host pin other versions of the same global command — so bootstrapping or publishing one
# repository invalidated another's gate until a person put the right version back (#1990).
# The location is namespaced by tool and version under the user's cache home:
#
#   ${ONETASKGRAPH_TOOLS_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/onetaskgraph/tools}/release-plz/<version>/bin/release-plz
#
# Under the cache home rather than inside the checkout because a fresh worktree, and the
# scratch tree a publication clones, both have to find an already-provisioned binary without
# installing it again; a directory inside the checkout is provisioned once per worktree, and
# a publication's scratch clone would start from nothing on every run. Namespaced by version
# so that moving the pin never overwrites the binary a still-open worktree on the old pin
# resolves, and so two repositories that both adopt this shape can never collide.
# ONETASKGRAPH_TOOLS_HOME exists for the checks, which point it at a scratch tree; nothing
# in production sets it. Whatever release-plz is on PATH is neither consulted nor touched.
#
# Usage:
#   scripts/scoped-release-plz.sh pin      print the pinned version
#   scripts/scoped-release-plz.sh path     print where the scoped binary is, or would be
#   scripts/scoped-release-plz.sh resolve  print that path if it answers the pinned version;
#                                          otherwise say what is missing and exit 69
#   scripts/scoped-release-plz.sh ensure   provision it there if `resolve` would refuse
#   scripts/scoped-release-plz.sh run ...  run the resolved binary with the arguments given
#
# Exit codes: 0; 64 (EX_USAGE) for a call this does not understand; 69 (EX_UNAVAILABLE)
# from `resolve` and `run` when the scoped binary is missing or answers another version,
# and from every subcommand but `pin` when there is no absolute cache home to scope it
# under; 70 (EX_SOFTWARE) when RELEASE_PLZ_VERSION below is not an exact X.Y.Z version,
# which is this script's own defect; 1 from `ensure` when provisioning was attempted and
# failed.
set -euo pipefail

readonly RELEASE_PLZ_VERSION=0.3.160
# The version is a path component below, and `ensure` clears the directory named by it.
[[ $RELEASE_PLZ_VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "scoped-release-plz: RELEASE_PLZ_VERSION is '$RELEASE_PLZ_VERSION', not an exact X.Y.Z version" >&2
  echo "scoped-release-plz: next: restore the exact pin in scripts/scoped-release-plz.sh" >&2
  exit 70
}

usage() {
  echo "scoped-release-plz: $1" >&2
  echo "scoped-release-plz: next: call it as 'scripts/scoped-release-plz.sh pin|path|resolve|ensure|run <args...>'" >&2
  exit 64
}

[ $# -ge 1 ] || usage "this script takes a subcommand and received none"
subcommand="$1"
shift
case "$subcommand" in
  pin | path | resolve | ensure) [ $# -eq 0 ] || usage "'$subcommand' takes no arguments and received $#" ;;
  run) [ $# -ge 1 ] || usage "'run' takes the arguments to hand release-plz and received none" ;;
  *) usage "'$subcommand' is not a subcommand of this script" ;;
esac

if [ "$subcommand" = pin ]; then
  printf '%s\n' "$RELEASE_PLZ_VERSION"
  exit 0
fi

if [ -n "${ONETASKGRAPH_TOOLS_HOME:-}" ]; then
  tools_home="$ONETASKGRAPH_TOOLS_HOME"
elif [ -n "${XDG_CACHE_HOME:-}" ]; then
  tools_home="$XDG_CACHE_HOME/onetaskgraph/tools"
elif [ -n "${HOME:-}" ]; then
  tools_home="$HOME/.cache/onetaskgraph/tools"
else
  echo "scoped-release-plz: none of ONETASKGRAPH_TOOLS_HOME, XDG_CACHE_HOME and HOME is set, so there is no cache home to scope release-plz under" >&2
  echo "scoped-release-plz: next: run this from a login environment that sets HOME, or set XDG_CACHE_HOME" >&2
  exit 69
fi
# The root comes from the environment and `ensure` removes a directory two levels beneath
# it, so it has to be an absolute path — `/…`, or `C:/…` and `C:\…` from a Windows shell —
# before anything is created or cleared under it. A relative one would resolve against
# whatever directory the caller happened to be in.
case "$tools_home" in
  /* | [A-Za-z]:/* | [A-Za-z]:\\*) ;;
  *)
    echo "scoped-release-plz: the tool home '$tools_home' is not an absolute path, so nothing is provisioned or removed under it" >&2
    echo "scoped-release-plz: next: set ONETASKGRAPH_TOOLS_HOME, XDG_CACHE_HOME or HOME to an absolute directory" >&2
    exit 69
    ;;
esac
readonly scoped_root="$tools_home/release-plz/$RELEASE_PLZ_VERSION"
readonly scoped_bin="$scoped_root/bin/release-plz"

# `cargo install` writes `release-plz.exe` on Windows, and `[ -x ]` there does not add the
# suffix the way the shell does when it runs a command.
binary_path() {
  if [ ! -x "$scoped_bin" ] && [ -x "$scoped_bin.exe" ]; then
    printf '%s\n' "$scoped_bin.exe"
  else
    printf '%s\n' "$scoped_bin"
  fi
}

if [ "$subcommand" = path ]; then
  binary_path
  exit 0
fi

# What the scoped binary answers, or the empty string when there is nothing there to ask.
# The carriage return a Windows binary may end its answer with is stripped in the shell
# rather than by `tr`, so resolving needs nothing on PATH but the binary itself.
installed_version() {
  local bin answer
  bin="$(binary_path)"
  [ -x "$bin" ] || return 0
  answer="$("$bin" --version 2>/dev/null)" || answer=""
  printf '%s\n' "${answer//$'\r'/}"
}

resolved() {
  [ "$(installed_version)" = "release-plz $RELEASE_PLZ_VERSION" ]
}

install_command="bash scripts/scoped-release-plz.sh ensure"
readonly install_command

# The refusal `resolve` and `run` share. It names the tool, the version, the location and
# the command that provisions it, because a caller that cannot go on without the binary has
# nothing else to tell the person reading its log.
refuse_unresolved() {
  local found
  found="$(installed_version)"
  echo "scoped-release-plz: release-plz $RELEASE_PLZ_VERSION is not provisioned at $(binary_path) (found: ${found:-nothing})" >&2
  echo "scoped-release-plz: next: run '$install_command' from a checkout of this repository — or 'just bootstrap', which does — to install it there; the release-plz on PATH, if any, is neither used nor changed" >&2
  exit 69
}

if [ "$subcommand" = resolve ]; then
  resolved || refuse_unresolved
  binary_path
  exit 0
fi

if [ "$subcommand" = run ]; then
  resolved || refuse_unresolved
  exec "$(binary_path)" "$@"
fi

# ensure. Idempotent: an already-provisioned binary at the pinned version is left alone.
resolved && exit 0

# A version-namespaced directory that holds another version is residue, not a cache hit:
# it is cleared so the installer below starts from nothing, rather than asking each
# installer how it treats a binary already in its way.
if [ -e "$scoped_root" ]; then
  rm -rf "$scoped_root" || {
    echo "scoped-release-plz: could not clear $scoped_root, which holds something other than release-plz $RELEASE_PLZ_VERSION" >&2
    echo "scoped-release-plz: next: remove that directory by hand, then rerun '$install_command'" >&2
    exit 1
  }
fi
mkdir -p "$scoped_root/bin" || {
  echo "scoped-release-plz: could not create $scoped_root/bin" >&2
  echo "scoped-release-plz: next: check the permissions of $tools_home, then rerun '$install_command'" >&2
  exit 1
}

# cargo-binstall downloads the published binary in seconds; `cargo install` builds it from
# source in minutes. Both are given `--version` and a destination inside the scoped root and
# nothing else, so neither touches the global cargo bin directory — and binstall is also told
# `--no-track`, because by default it records what it installed in the GLOBAL
# .crates.toml, which is what a later `cargo install` of the same crate reads.
binstall_output=""
installed_by=""
if command -v cargo-binstall >/dev/null 2>&1; then
  if binstall_output="$(cargo binstall release-plz --version "$RELEASE_PLZ_VERSION" \
    --install-path "$scoped_root/bin" --no-confirm --no-track 2>&1)"; then
    installed_by="cargo binstall"
  else
    # A binstall that could not fetch the published binary is not the end: the build from
    # source below is what a host without cargo-binstall does anyway. One line says why
    # the minutes are being spent; binstall's own output is shown only if the build fails
    # too, so a provisioning that succeeds does not end under a diagnostic for a path it
    # recovered from.
    # llmlint: ignore[tool_output_is_signal] The one line is the signal: a bootstrap that takes eight minutes rather than eight seconds has to say which installer it fell back from, or the slow path reads as the tool hanging.
    echo "scoped-release-plz: cargo binstall could not install release-plz $RELEASE_PLZ_VERSION; building it from source instead" >&2
  fi
fi
if [ -z "$installed_by" ]; then
  command -v cargo >/dev/null 2>&1 || {
    echo "scoped-release-plz: cargo is not on PATH, so release-plz $RELEASE_PLZ_VERSION cannot be installed" >&2
    echo "scoped-release-plz: next: install the Rust toolchain from https://rustup.rs, then rerun '$install_command'" >&2
    exit 1
  }
  install_output=""
  install_output="$(cargo install release-plz --version "$RELEASE_PLZ_VERSION" --locked \
    --root "$scoped_root" 2>&1)" || {
    [ -z "$binstall_output" ] || printf '%s\n' "$binstall_output" >&2
    printf '%s\n' "$install_output" >&2
    echo "scoped-release-plz: release-plz $RELEASE_PLZ_VERSION installation into $scoped_root failed" >&2
    echo "scoped-release-plz: next: fix the installer diagnostic above and rerun '$install_command'" >&2
    exit 1
  }
  installed_by="cargo install"
fi

# What was installed is proven by asking it, not by the installer's exit status: an
# installer that put a different version there, or a binary that cannot start, is what the
# gate would otherwise discover later and report as this tree being broken.
if ! resolved; then
  found="$(installed_version)"
  echo "scoped-release-plz: $installed_by finished, but $(binary_path) answers '${found:-nothing}' rather than 'release-plz $RELEASE_PLZ_VERSION'" >&2
  echo "scoped-release-plz: next: inspect that directory and the installer's output, then rerun '$install_command'" >&2
  exit 1
fi
