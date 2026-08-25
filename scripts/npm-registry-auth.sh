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
[[ $registry =~ ^https?://[^[:space:]]+$ ]] || usage "invalid registry, which must be an http:// or https:// URL: $registry"
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
# path is printed in the form the local client reads.
if command -v cygpath >/dev/null 2>&1; then config=$(cygpath -w "$config"); fi
printf '%s\n' "$config"
