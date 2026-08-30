#!/usr/bin/env bash
# Publish this repository's npm packages: the five platform carriers, the launcher that
# selects between them, and the TypeScript SDK.
#
# A script rather than a block of YAML inside .github/workflows/release.yml, because this
# is the one publication path nothing could drive before it ran for real: a release is the
# only thing that triggers that workflow, so a defect here was discoverable only by cutting
# a version. `scripts/check-npm-publish.sh` runs exactly this, against a registry it stands
# up itself, and reads back the packages it would send.
#
# Two invariants this keeps, each with a defect behind it:
#
#   * **Every package operand is an explicit local directory or file.** `npm/cli` without
#     the `./` is npm's shorthand for the GitHub repository `npm/cli` — npm's own CLI — so
#     a release would publish somebody else's package.
#   * **Authentication is configured, not merely exported.** npm reads no credential from
#     the environment on its own: `NODE_AUTH_TOKEN` works only through an npmrc naming it.
#     A job that exports the variable and nothing else packs every tarball in full and then
#     fails ENEEDAUTH, exactly as though it were logged out.
#
# A package already at the registry at that exact version is left alone rather than
# republished, so a release that partly published can be re-run.
#
# Inputs: NODE_AUTH_TOKEN (required), and NPM_REGISTRY (optional — the public registry by
# default; the check above is the only thing that sets it).
# llmlint: ignore-file[new_code_lands_in_a_project] scripts/ is deliberately outside the
# Nx project graph (AGENTS.md, Conventions): Nx maps no project to it, which is why the
# justfile invokes these from recipes of its own. Nothing here escapes the gate — it
# runs from `.github/workflows/release.yml`, and `scripts/check-npm-publish.sh` drives
# it from `just distribution-test` — so the graph's absence costs an optimisation rather
# than the coverage this rule protects.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly CARRIERS="${NPM_CARRIERS:-dist/carriers}"
registry=${NPM_REGISTRY:-https://registry.npmjs.org/}
readonly registry

# Read through a default before its length is taken: under `set -u` an unset
# NODE_AUTH_TOKEN makes `${#NODE_AUTH_TOKEN}` end the script on bash's own
# "unbound variable", replacing the message below — the one case this guard exists for.
token=${NODE_AUTH_TOKEN:-}
[ -n "$token" ] || {
  echo "NPM_TOKEN is required (received ${#token} characters)" >&2
  echo "next: set the NPM_TOKEN repository secret — gh-secrets.json declares it" >&2
  exit 1
}

NPM_CONFIG_USERCONFIG=$(scripts/npm-registry-auth.sh "$registry")
export NPM_CONFIG_USERCONFIG

readonly SCRATCH="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"

# publish_if_absent <name@version> <local directory or tarball>
publish_if_absent() {
  spec=$1
  shift
  error="$SCRATCH/npm-view-error.$$"
  if npm view "$spec" --registry "$registry" >/dev/null 2>"$error"; then return; fi
  if grep -Eq 'E404|404 Not Found' "$error"; then
    # npm reports a successful publication in a dozen lines of tarball inventory. That is
    # progress, not the answer, so it is held and replayed only when the publish fails —
    # at which point every line of it is worth reading.
    published="$SCRATCH/npm-publish-output.$$"
    if ! npm publish "$@" --registry "$registry" --access public > "$published" 2>&1; then
      cat "$published" >&2
      echo "npm refused to publish $spec" >&2
      echo "next: fix what npm reported above, then re-run — a package already at this" >&2
      echo "next: version is left alone rather than republished, so this is safe to retry" >&2
      exit 1
    fi
    return
  fi
  cat "$error" >&2
  echo "could not query npm for $spec" >&2
  echo "next: check that $registry is reachable and the token is valid, then re-run" >&2
  exit 69
}

for package in npm/platforms/*; do
  name=$(node -p "require('./$package/package.json').name")
  version=$(node -p "require('./$package/package.json').version")
  platform=${package##*/}
  [[ $name == "@onetaskgraph/cli-$platform" ]] || {
    echo "invalid carrier name: $name" >&2
    exit 64
  }
  printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$' || {
    echo "invalid carrier version: $version" >&2
    exit 64
  }
  publish_if_absent "$name@$version" "$CARRIERS/onetaskgraph-cli-${platform}-${version}.tgz"
done

cli_version=$(node -p "require('./npm/cli/package.json').version")
sdk_version=$(node -p "require('./sdks/typescript/package.json').version")
printf '%s\n' "$cli_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$' || {
  echo "invalid CLI version: $cli_version" >&2
  exit 64
}
printf '%s\n' "$sdk_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$' || {
  echo "invalid TypeScript SDK version: $sdk_version" >&2
  exit 64
}
publish_if_absent "@onetaskgraph/cli@$cli_version" ./npm/cli
publish_if_absent "@onetaskgraph/sdk@$sdk_version" ./sdks/typescript
