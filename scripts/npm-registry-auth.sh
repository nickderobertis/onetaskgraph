#!/usr/bin/env bash
# Writes the npm configuration that makes NODE_AUTH_TOKEN usable, for the release workflow,
# and prints its path for NPM_CONFIG_USERCONFIG.
# npm reads no credential from the environment on its own: NODE_AUTH_TOKEN is a convention
# that works only through an npmrc naming it as the registry's auth token, which is what
# actions/setup-node writes. A job that exports the variable and nothing else packs every
# tarball in full and then fails ENEEDAUTH, exactly as though it were logged out.
# The file names the variable; the token's value never reaches it.
set -euo pipefail
usage() { echo "usage: scripts/npm-registry-auth.sh [REGISTRY_URL]" >&2; echo "next: $1" >&2; exit 64; }
[[ $# -le 1 ]] || usage "pass at most one registry URL, as .github/workflows/release.yml does"
registry=${1:-https://registry.npmjs.org/}
# The authority becomes npm's auth key, so a URL without a host would key the token to
# nothing and publish anonymously. "everything up to the next slash" is not that check: it
# reads `https://?registry=x`, `https://#registry` and `https://token@` as authorities,
# though each has an empty host, and keys the token to the punctuation. So the host is
# spelled out — a bracketed IPv6 literal, or a name of host characters bounded by
# alphanumerics — with an optional numeric port. A pattern held in a variable because bash
# 3.2 reads a quoted one on this operator's right as a literal string.
registry_pattern='^https?://(\[[0-9A-Fa-f:.]+\]|[0-9A-Za-z]([0-9A-Za-z._-]*[0-9A-Za-z])?)(:[0-9]+)?(/[^[:space:]]*)?$'
[[ $registry =~ $registry_pattern ]] || usage "invalid registry, which must be an http:// or https:// URL with a host: $registry"
# npm keys auth by the registry's scheme-less URL, and matches it with a trailing slash.
[[ $registry == */ ]] || registry="$registry/"
directory=${ONETASKGRAPH_NPM_CONFIG_DIR:-${RUNNER_TEMP:-}}
if [[ -z $directory ]]; then
  directory=$(mktemp -d) || { echo "could not create a directory for the npm configuration" >&2; echo "next: inspect temporary-directory permissions and free space" >&2; exit 1; }
fi
mkdir -p "$directory" || { echo "could not create the npm configuration directory: $directory" >&2; echo "next: point ONETASKGRAPH_NPM_CONFIG_DIR at a writable directory" >&2; exit 1; }
config="$directory/.npmrc"
(
  umask 077
  printf 'registry=%s\n//%s:_authToken=${NODE_AUTH_TOKEN}\n' "$registry" "${registry#*://}" > "$config"
) || { echo "could not write the npm configuration: $config" >&2; echo "next: point ONETASKGRAPH_NPM_CONFIG_DIR at a writable directory" >&2; exit 1; }
# npm on Windows is a native program that cannot resolve this shell's POSIX paths, so the
# path is printed in the form the local client reads. The Windows lane of the install-path
# job is what runs this branch: it drives the real npm client through this configuration.
if command -v cygpath >/dev/null 2>&1; then
  config=$(cygpath -w "$config") || { echo "could not express the npm configuration path for this platform: $config" >&2; echo "next: report the cygpath failure above; scripts/npm-registry-auth.sh needs a path the local npm client can read" >&2; exit 1; }
fi
printf '%s\n' "$config" || { echo "could not report the npm configuration path: $config" >&2; echo "next: rerun scripts/npm-registry-auth.sh with a writable standard output, which is what the caller reads the path from" >&2; exit 1; }
