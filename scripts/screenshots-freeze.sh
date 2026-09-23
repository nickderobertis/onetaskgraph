#!/usr/bin/env bash
# The ONE implementation of which `freeze` this repository renders its screenshots with,
# and where that binary lives.
#
# `freeze` turns a scene's captured bytes into an SVG, so its version is one of the two
# inputs the committed digest baseline is a function of (the other is the vendored font).
# The pin therefore lives in exactly one place — FREEZE_VERSION below — and the capture
# script, the justfile recipe and .github/workflows/visual-docs.yml all reach the binary
# through this script rather than naming a version of their own. Moving the pin reflows
# every shot: bless the baseline in the same change.
#
# The binary lives in a REPOSITORY-SCOPED location, never on PATH, for the reason
# scripts/scoped-release-plz.sh gives at length: other repositories on this host pin other
# versions of the same global command, and installing one repository's pin globally
# invalidates another's gate until a person puts it back. The location is namespaced by
# tool and version under the user's cache home:
#
#   ${ONETASKGRAPH_TOOLS_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/onetaskgraph/tools}/freeze/<version>/bin/freeze
#
# A `freeze` already on PATH is neither consulted nor touched.
#
# Usage:
#   scripts/screenshots-freeze.sh pin      print the pinned version
#   scripts/screenshots-freeze.sh path     print where the scoped binary is, or would be
#   scripts/screenshots-freeze.sh resolve  print that path if it answers the pinned version;
#                                          otherwise say what is missing and exit 69
#   scripts/screenshots-freeze.sh ensure   provision it there if `resolve` would refuse
#
# Exit codes: 0; 64 (EX_USAGE) for a call this does not understand; 69 (EX_UNAVAILABLE)
# from `resolve` when the scoped binary is missing or answers another version, and from
# every subcommand but `pin` when there is no absolute cache home to scope it under;
# 70 (EX_SOFTWARE) when FREEZE_VERSION is not an exact X.Y.Z version, which is this
# script's own defect; 1 from `ensure` when provisioning was attempted and failed.
set -euo pipefail

readonly FREEZE_VERSION=0.2.2
# The SHA-256 of each published archive of that version, recorded from the real release, so
# what arrives over the network is authenticated before it is unpacked and long before it is
# run — a self-reported `--version` says nothing about what a substituted response would do
# first. `cargo` verifies a registry checksum for the sibling tool
# scripts/scoped-release-plz.sh provisions; this is the same guarantee for a tool that comes
# from a release archive instead. Moving FREEZE_VERSION means re-recording all four:
#   for a in Linux_x86_64 Linux_arm64 Darwin_x86_64 Darwin_arm64; do
#     curl -fsSL "https://github.com/charmbracelet/freeze/releases/download/v<version>/freeze_<version>_$a.tar.gz" | sha256sum
#   done
readonly FREEZE_SHA256_Linux_x86_64=012fdbdd16c0c19570f9052aac34d16d93d7d0d3b565b05374cc59492f53539b
readonly FREEZE_SHA256_Linux_arm64=ba1164c2e6d573af32df13ddc5868f7cdfc8b9992f26f53695787ca5f4274023
readonly FREEZE_SHA256_Darwin_x86_64=7ba01bc3f7f255bee2fe84847e8beeb1caa84914a49f4cf32e0147693d5e385f
readonly FREEZE_SHA256_Darwin_arm64=6936ebad96dda73a6c952d7e945106916c4d4345334828109b4db6ddbb9aabee
# The version is a path component below, and `ensure` clears the directory named by it.
[[ $FREEZE_VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "screenshots-freeze: FREEZE_VERSION is '$FREEZE_VERSION', not an exact X.Y.Z version" >&2
  echo "screenshots-freeze: next: restore the exact pin in scripts/screenshots-freeze.sh" >&2
  exit 70
}

usage() {
  echo "screenshots-freeze: $1" >&2
  echo "screenshots-freeze: next: call it as 'scripts/screenshots-freeze.sh pin|path|resolve|ensure'" >&2
  exit 64
}

[ $# -eq 1 ] || usage "this script takes exactly one subcommand and received $#"
case "$1" in
  pin | path | resolve | ensure) subcommand="$1" ;;
  *) usage "'$1' is not a subcommand of this script" ;;
esac

if [ "$subcommand" = pin ]; then
  printf '%s\n' "$FREEZE_VERSION"
  exit 0
fi

if [ -n "${ONETASKGRAPH_TOOLS_HOME:-}" ]; then
  tools_home="$ONETASKGRAPH_TOOLS_HOME"
elif [ -n "${XDG_CACHE_HOME:-}" ]; then
  tools_home="$XDG_CACHE_HOME/onetaskgraph/tools"
elif [ -n "${HOME:-}" ]; then
  tools_home="$HOME/.cache/onetaskgraph/tools"
else
  echo "screenshots-freeze: none of ONETASKGRAPH_TOOLS_HOME, XDG_CACHE_HOME and HOME is set, so there is no cache home to scope freeze under" >&2
  echo "screenshots-freeze: next: run this from a login environment that sets HOME, or set XDG_CACHE_HOME" >&2
  exit 69
fi
# `ensure` removes a directory two levels beneath this root, so it has to be absolute
# before anything is created or cleared under it.
case "$tools_home" in
  /* | [A-Za-z]:/* | [A-Za-z]:\\*) ;;
  *)
    echo "screenshots-freeze: the tool home '$tools_home' is not an absolute path, so nothing is provisioned or removed under it" >&2
    echo "screenshots-freeze: next: set ONETASKGRAPH_TOOLS_HOME, XDG_CACHE_HOME or HOME to an absolute directory" >&2
    exit 69
    ;;
esac
readonly scoped_root="$tools_home/freeze/$FREEZE_VERSION"
readonly scoped_bin="$scoped_root/bin/freeze"

if [ "$subcommand" = path ]; then
  printf '%s\n' "$scoped_bin"
  exit 0
fi

# What the scoped binary answers, or the empty string when there is nothing there to ask.
installed_version() {
  local answer
  [ -x "$scoped_bin" ] || return 0
  answer="$("$scoped_bin" --version 2>/dev/null)" || answer=""
  printf '%s\n' "${answer//$'\r'/}"
}

# The prebuilt archive's binary answers `freeze version v0.2.2 (80921ba)` where one built
# from source answers `freeze version v0.2.2`. So exactly those two answers are the pin: the
# version alone, or the version and a build commit in parentheses and nothing after it.
resolved() {
  local answer
  answer="$(installed_version)"
  [[ "$answer" =~ ^freeze\ version\ v"$FREEZE_VERSION"( \([0-9a-f]+\))?$ ]]
}

readonly install_command="bash scripts/screenshots-freeze.sh ensure"

if [ "$subcommand" = resolve ]; then
  if ! resolved; then
    found="$(installed_version)"
    echo "screenshots-freeze: freeze $FREEZE_VERSION is not provisioned at $scoped_bin (found: ${found:-nothing})" >&2
    echo "screenshots-freeze: next: run '$install_command' — or 'just screenshots-tools', which does — to install it there; the freeze on PATH, if any, is neither used nor changed" >&2
    exit 69
  fi
  printf '%s\n' "$scoped_bin"
  exit 0
fi

# ensure. Idempotent: an already-provisioned binary at the pinned version is left alone.
resolved && exit 0

# freeze publishes a prebuilt archive per platform, so this needs no Go toolchain. The
# asset name is `freeze_<version>_<Os>_<Arch>.tar.gz` and the binary sits one directory
# down inside it.
case "$(uname -s)" in
  Linux) os=Linux ;;
  Darwin) os=Darwin ;;
  *)
    echo "screenshots-freeze: freeze publishes no prebuilt archive for $(uname -s), so the renderer cannot be provisioned here" >&2
    echo "screenshots-freeze: next: capture on Linux, or in the container .github/workflows/visual-docs.yml names; CI is the backstop either way" >&2
    exit 1
    ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) architecture=x86_64 ;;
  arm64 | aarch64) architecture=arm64 ;;
  *)
    echo "screenshots-freeze: freeze publishes no prebuilt archive for $(uname -m), so the renderer cannot be provisioned here" >&2
    echo "screenshots-freeze: next: capture on an x86_64 or arm64 machine, or in the container .github/workflows/visual-docs.yml names" >&2
    exit 1
    ;;
esac
readonly archive="freeze_${FREEZE_VERSION}_${os}_${architecture}.tar.gz"
readonly url="https://github.com/charmbracelet/freeze/releases/download/v${FREEZE_VERSION}/${archive}"

# A version-namespaced directory holding another version is residue, not a cache hit.
if [ -e "$scoped_root" ]; then
  rm -rf "$scoped_root" || {
    echo "screenshots-freeze: could not clear $scoped_root, which holds something other than freeze $FREEZE_VERSION" >&2
    echo "screenshots-freeze: next: remove that directory by hand, then rerun '$install_command'" >&2
    exit 1
  }
fi
mkdir -p "$scoped_root/bin" || {
  echo "screenshots-freeze: could not create $scoped_root/bin" >&2
  echo "screenshots-freeze: next: check the permissions of $tools_home, then rerun '$install_command'" >&2
  exit 1
}

unpacked="$(mktemp -d)" || {
  echo "screenshots-freeze: could not create a temporary directory to unpack $archive into" >&2
  echo "screenshots-freeze: next: check the permissions of \$TMPDIR and 'df -h', then rerun '$install_command'" >&2
  exit 1
}
trap 'rm -rf "$unpacked"' EXIT

command -v curl >/dev/null 2>&1 || {
  echo "screenshots-freeze: curl is not on PATH, so $url cannot be fetched" >&2
  echo "screenshots-freeze: next: install curl, then rerun '$install_command'" >&2
  exit 1
}
curl -fsSL -o "$unpacked/$archive" "$url" || {
  echo "screenshots-freeze: could not download $url" >&2
  echo "screenshots-freeze: next: check the network and that the release carries $archive, then rerun '$install_command'" >&2
  exit 1
}
# The recorded digest for this platform, FIRST: tar below is a parser, and a response
# nothing has authenticated should not reach one. A
# platform with no digest recorded is not provisioned here at all, rather than provisioned
# unverified.
expected_digest_name="FREEZE_SHA256_${os}_${architecture}"
expected_digest="${!expected_digest_name:-}"
if [ -z "$expected_digest" ]; then
  echo "screenshots-freeze: no archive digest is recorded for ${os}_${architecture}, so the download cannot be authenticated" >&2
  echo "screenshots-freeze: next: record FREEZE_SHA256_${os}_${architecture} in scripts/screenshots-freeze.sh from the real release, or capture on a platform that has one" >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  actual_digest="$(sha256sum "$unpacked/$archive" | cut -d' ' -f1)"
elif command -v shasum >/dev/null 2>&1; then
  actual_digest="$(shasum -a 256 "$unpacked/$archive" | cut -d' ' -f1)"
else
  echo "screenshots-freeze: neither sha256sum nor shasum is on PATH, so $archive cannot be authenticated" >&2
  echo "screenshots-freeze: next: install coreutils (or perl's shasum), then rerun '$install_command'" >&2
  exit 1
fi
if [ "$actual_digest" != "$expected_digest" ]; then
  echo "screenshots-freeze: $archive hashes to $actual_digest, not the recorded $expected_digest" >&2
  echo "screenshots-freeze: next: do not unpack it. Either the download was corrupted — rerun '$install_command' — or the published archive for v$FREEZE_VERSION has changed, which is a thing to report rather than to accept" >&2
  exit 1
fi

# What arrives over the network is not trusted to stay inside the directory it is unpacked
# into: a member naming an absolute path or walking up with `..` is refused before anything
# is written. (The archive is fetched over TLS from the release of the pinned version, which
# is the trust model scripts/scoped-release-plz.sh already provisions its own tool under.)
# `2>/dev/null` on the listings alone: tar warns on stderr about the very member names
# these two cases are looking for ("Removing leading `../'"), and that warning on a path
# this script goes on to refuse would read as the refusal having gone wrong. A listing that
# actually fails is caught by its exit status below.
members="$(tar -tzf "$unpacked/$archive" 2>/dev/null)" || {
  echo "screenshots-freeze: could not read the contents of $unpacked/$archive" >&2
  echo "screenshots-freeze: next: delete it and rerun '$install_command'" >&2
  exit 1
}
case "$members" in
  /* | *$'\n'/* | *..*)
    echo "screenshots-freeze: $archive carries a member with an absolute path or a '..' segment, which would write outside $unpacked" >&2
    echo "screenshots-freeze: next: do not unpack it; report the published archive for v$FREEZE_VERSION" >&2
    exit 1
    ;;
esac
# And no link member, which the name listing above cannot show: a symbolic or hard link
# unpacked first is a door a later member writes through, outside this directory. The
# verbose listing spells the type in the first character of each line, `l` or `h` for the
# two kinds of link.
detailed="$(tar -tvzf "$unpacked/$archive" 2>/dev/null)" || {
  echo "screenshots-freeze: could not read the member types of $unpacked/$archive" >&2
  echo "screenshots-freeze: next: delete it and rerun '$install_command'" >&2
  exit 1
}
case "$detailed" in
  [lh]* | *$'\n'[lh]*)
    echo "screenshots-freeze: $archive carries a link member, which could redirect a later member outside $unpacked" >&2
    echo "screenshots-freeze: next: do not unpack it; report the published archive for v$FREEZE_VERSION" >&2
    exit 1
    ;;
esac
tar -xzf "$unpacked/$archive" -C "$unpacked" || {
  echo "screenshots-freeze: could not unpack $unpacked/$archive" >&2
  echo "screenshots-freeze: next: delete it and rerun '$install_command'" >&2
  exit 1
}
extracted="$unpacked/freeze_${FREEZE_VERSION}_${os}_${architecture}/freeze"
[ -f "$extracted" ] || extracted="$unpacked/freeze"
[ -f "$extracted" ] || {
  echo "screenshots-freeze: $archive unpacked but carries no freeze binary at $extracted" >&2
  echo "screenshots-freeze: next: check the published archive's layout for v$FREEZE_VERSION, then rerun '$install_command'" >&2
  exit 1
}
install -m 0755 "$extracted" "$scoped_bin" || {
  echo "screenshots-freeze: could not install $extracted as $scoped_bin" >&2
  echo "screenshots-freeze: next: check the permissions and free space of $scoped_root, then rerun '$install_command'" >&2
  exit 1
}

# What was installed is proven by asking it, rather than by the installer's exit status.
if ! resolved; then
  found="$(installed_version)"
  echo "screenshots-freeze: the archive was installed, but $scoped_bin answers '${found:-nothing}' rather than 'freeze version v$FREEZE_VERSION'" >&2
  echo "screenshots-freeze: next: inspect that directory and rerun '$install_command'" >&2
  exit 1
fi
