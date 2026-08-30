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
# Inputs: NODE_AUTH_TOKEN (required); NPM_REGISTRY (optional — the public registry by
# default; the check above is the only thing that sets it); and NPM_CARRIERS (optional —
# `dist/carriers`, where the release workflow downloads the per-platform tarballs).
# llmlint: ignore-file[new_code_lands_in_a_project] scripts/ is deliberately outside the
# Nx project graph (AGENTS.md, Conventions): Nx maps no project to it, which is why the
# justfile invokes these from recipes of its own. Nothing here escapes the gate — it
# runs from `.github/workflows/release.yml`, and `scripts/check-npm-publish.sh` drives
# it from `just distribution-test` — so the graph's absence costs an optimisation rather
# than the coverage this rule protects.
set -euo pipefail

# Every failure below this point says what went wrong and what to do about it. This one
# exists for the failures that are the environment rather than the publication: an
# unwritable npmrc or a checkout this cannot find is not something npm's own diagnostic
# names an action for, and a release job stopping on a bare shell message reads as the
# publication having refused.
fatal() {
  echo "publish-npm: $1" >&2
  echo "publish-npm: next: $2" >&2
  exit 70
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run this from a checkout of this repository, as .github/workflows/release.yml does"
readonly ROOT
cd "$ROOT" || fatal \
  "could not enter $ROOT" \
  "check that directory's permissions, then re-run"

readonly CARRIERS="${NPM_CARRIERS:-dist/carriers}"
registry=${NPM_REGISTRY:-https://registry.npmjs.org/}
readonly registry

# Both arrive from the environment, so both are checked here rather than by npm several
# publications later. A registry that is not an http(s) URL is handed to npm as a flag it
# reads its own way — and the npmrc built from it below authenticates nothing — so the
# publication fails in npm's words about a value this script chose. A carriers directory
# that is not there is a download step that did not run, which npm otherwise reports one
# tarball at a time, after the first carrier has already landed at the real registry.
#
# The scheme alone is not enough to make it a URL npm can reach: `https://` on its own, or
# one carrying a space, satisfies a prefix test and then fails inside npm. So the host is
# required here too, and `npm-registry-auth.sh` is handed nothing this did not accept.
readonly REGISTRY_URL='^https?://[^[:space:]/?#]+(/[^[:space:]]*)?$'
if ! [[ $registry =~ $REGISTRY_URL ]]; then
  echo "NPM_REGISTRY must be an http or https URL naming a host (received: $registry)" >&2
  echo "next: unset NPM_REGISTRY to publish to https://registry.npmjs.org/" >&2
  exit 64
fi
[ -d "$CARRIERS" ] || {
  echo "no carrier directory at $CARRIERS" >&2
  echo "next: download the release workflow's carrier-* artifacts into it, or set" >&2
  echo "next: NPM_CARRIERS to the directory holding the per-platform .tgz files" >&2
  exit 64
}

# Read through a default before its length is taken: under `set -u` an unset
# NODE_AUTH_TOKEN makes `${#NODE_AUTH_TOKEN}` end the script on bash's own
# "unbound variable", replacing the message below — the one case this guard exists for.
token=${NODE_AUTH_TOKEN:-}
[ -n "$token" ] || {
  echo "NPM_TOKEN is required (received ${#token} characters)" >&2
  echo "next: set the NPM_TOKEN repository secret — gh-secrets.json declares it" >&2
  exit 1
}

NPM_CONFIG_USERCONFIG=$(scripts/npm-registry-auth.sh "$registry") || fatal \
  "could not write the npmrc that authenticates to $registry" \
  "fix what scripts/npm-registry-auth.sh reported above — without that file npm packs \
every tarball in full and then fails ENEEDAUTH, exactly as though it were logged out"
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

# Every operand is read and checked before any of them is published. A tarball the
# download step never produced, or a package directory that is not in this checkout,
# stops the publication at the fourth carrier otherwise — three carriers after the first
# has already landed at the registry, which is the half-published release this script is
# shaped to avoid and the state a re-run then has to be safe against.
carrier_specs=()
carrier_files=()
absent=""
for package in npm/platforms/*; do
  # A manifest node cannot read leaves these empty rather than ending the script: node
  # has already said why on stderr, and the guards below are what name the file to fix.
  name=$(node -p "require('./$package/package.json').name") || true
  version=$(node -p "require('./$package/package.json').version") || true
  platform=${package##*/}
  [[ $name == "@onetaskgraph/cli-$platform" ]] || {
    echo "invalid carrier name: $name" >&2
    echo "next: set \"name\" to @onetaskgraph/cli-$platform in npm/platforms/$platform/package.json" >&2
    exit 64
  }
  printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$' || {
    echo "invalid carrier version: $version" >&2
    echo "next: run 'scripts/set-version.sh <VERSION>', which brings every manifest —" >&2
    echo "next: npm/platforms/$platform/package.json included — to one semantic version" >&2
    exit 64
  }
  tarball="$CARRIERS/onetaskgraph-cli-${platform}-${version}.tgz"
  [ -f "$tarball" ] || absent="$absent
  $tarball"
  carrier_specs+=("$name@$version")
  carrier_files+=("$tarball")
done

# Unreadable for the same reason and handled the same way: the version guards below name
# the manifest and the command that sets it.
cli_version=$(node -p "require('./npm/cli/package.json').version") || true
sdk_version=$(node -p "require('./sdks/typescript/package.json').version") || true
printf '%s\n' "$cli_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$' || {
  echo "invalid CLI version: $cli_version" >&2
  echo "next: run 'scripts/set-version.sh <VERSION>' to set npm/cli/package.json's" >&2
  echo "next: version, then rerun" >&2
  exit 64
}
printf '%s\n' "$sdk_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$' || {
  echo "invalid TypeScript SDK version: $sdk_version" >&2
  echo "next: run 'scripts/set-version.sh <VERSION>' to set sdks/typescript/package.json's" >&2
  echo "next: version, then rerun" >&2
  exit 64
}
for directory in ./npm/cli ./sdks/typescript; do
  [ -d "$directory" ] || absent="$absent
  $directory"
done
[ -z "$absent" ] || {
  echo "these package operands are not there:$absent" >&2
  echo "next: for a tarball, run the release workflow's carrier download (or 'npm pack')" >&2
  echo "next: into $CARRIERS; for a directory, run this from a full checkout" >&2
  exit 64
}

# Nothing above published anything, so from here every operand is known to exist.
index=0
while [ "$index" -lt "${#carrier_specs[@]}" ]; do
  publish_if_absent "${carrier_specs[$index]}" "${carrier_files[$index]}"
  index=$((index + 1))
done
publish_if_absent "@onetaskgraph/cli@$cli_version" ./npm/cli
publish_if_absent "@onetaskgraph/sdk@$sdk_version" ./sdks/typescript
