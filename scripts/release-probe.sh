#!/usr/bin/env bash
# What a public registry serves right now for one artifact this repository publishes.
#
#   usage: release-probe.sh <registry>:<name>
#
# Exactly three answers, and keeping the last two apart is the whole point:
#
#   * exit 0 with one line on stdout — the version that registry serves now;
#   * exit 0 with nothing on stdout — that registry has no release of it yet;
#   * a non-zero exit with the reason on stderr — not answered. Exit 2 is
#     reserved for a usage error, which is the caller having asked wrongly rather
#     than anything about a release; every other refusal exits 1.
#
# **A probe is not a gate.** It rules on no change and refuses no publication. It
# answers what version is out there, and a caller decides what that means.
#
# **"Not answered" is not "not released".** A consumer holds indefinitely on the
# first and stops holding on the second, so nothing here degrades a registry that
# did not answer into an empty answer: "no release" is only ever a registry saying
# so — a 404, or a `max_stable_version` the registry itself set to null — never a
# body this script could not read, never a tool it could not run, and never an
# identifier it does not recognise. Reporting a failed lookup as "nothing
# published" is the single most damaging thing this script can get wrong.
#
# The identifiers it answers for are exactly the `[[target]]` ids of
# release-targets.toml, resolved from this script's own location rather than from
# $PWD — a probe answering about whatever repository it was started in is a probe
# answering about the wrong artifact. A `covers` id is not among them: nothing
# waits on one by name, so asking about one is not answered rather than answered
# emptily.
#
# The request each lookup makes and the field it reads out of the answer are a
# third party's interface, pinned with its provenance in
# config/registry-interfaces.toml; the URL templates and paths below are spelled
# to match it byte for byte, and scripts/check-release-probe.sh fails when they
# part. That check also drives all three answers here against stood-in registry
# documents built from that pin, so what makes them trustworthy is the pin rather
# than a registry being reachable.
#
# What it may assume, and nothing beyond it: it is spawned as a direct subprocess
# with no shell interposed, with an environment carrying little more than PATH and
# HOME, and no credential of any kind. Every target here is on a public registry,
# so an unauthenticated read is all it needs and all it may need. `curl -q`
# follows from the same rule — a ~/.curlrc is the caller's configuration, not this
# probe's.
#
# `curl` and `python3` are the whole of what it runs, so a host missing one is
# told which rather than failing somewhere inside a pipeline. python3 because a
# registry document is JSON and has to be *parsed*: a pattern that matched the
# wrong occurrence of a version-shaped key would answer a version nobody
# published, which is the failure mode this file exists to make impossible. Its
# bound is curl's own: one request per invocation, --max-time 25, well inside the
# sixty seconds a caller allows. There is no retry — a second attempt could double
# that — and a transient failure is "not answered", which the caller re-asks later.
set -euo pipefail

# Identifies this probe to crates.io, which answers 403 to a request that does not
# say who is asking. The other two registries do not require it; sending it anyway
# keeps one shape for all three.
readonly USER_AGENT="onetaskgraph-release-probe (+https://github.com/nickderobertis/onetaskgraph)"
readonly CONNECT_TIMEOUT=10
readonly MAX_TIME=25

# Not answered: the reason, and what the caller does about it. Both on stderr,
# because stdout is the answer and an empty one means something specific.
refuse() {
  printf 'release-probe: %s\n' "$1" >&2
  printf 'release-probe: next: %s\n' "$2" >&2
  exit "${3:-1}"
}

# The repository this probe belongs to, from the script's own path.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." 2>/dev/null && pwd)" || refuse \
  "could not reach the repository root from ${BASH_SOURCE[0]}" \
  "run this from a checkout of this repository, at scripts/release-probe.sh"
readonly ROOT
readonly DECLARATION="$ROOT/release-targets.toml"

[ $# -eq 1 ] || refuse \
  "takes exactly one registry-qualified identifier, and was given $#" \
  "run 'scripts/release-probe.sh <registry>:<name>', e.g. 'scripts/release-probe.sh crate:onetaskgraph'" 2
identifier="$1"

# The two external tools, named before anything needs them: a host without one has
# not answered, and saying which is missing is the whole of what it can be told.
for tool in curl python3; do
  command -v "$tool" >/dev/null 2>&1 || refuse \
    "$tool is not on PATH, so no registry can be read" \
    "install $tool, or put it on the PATH this probe is spawned with"
done

[ -r "$DECLARATION" ] || refuse \
  "$DECLARATION is missing or unreadable, so nothing says what this repository publishes" \
  "restore release-targets.toml from git; it is this repository's one declaration of what it releases"

# The declared targets, read with a real TOML parser rather than scanned: what the
# document *is* — every required field, every identifier, every short name — is
# held by scripts/check-release-targets.sh and by the canonical reader itself, and
# what matters here is only that this identifier is one a consumer waits on.
declared="$(python3 - "$identifier" "$DECLARATION" <<'PY'
import sys

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python older than 3.11
    print("this python3 has no tomllib, so the declaration cannot be read", file=sys.stderr)
    raise SystemExit(3)

identifier, path = sys.argv[1], sys.argv[2]
try:
    with open(path, "rb") as handle:
        document = tomllib.load(handle)
except (OSError, tomllib.TOMLDecodeError) as problem:
    print(f"{path} could not be read as TOML: {problem}", file=sys.stderr)
    raise SystemExit(3)

ids = [
    target["id"]
    for target in document.get("target", [])
    if isinstance(target, dict) and isinstance(target.get("id"), str)
]
if not ids:
    print(f"{path} declares no release target at all", file=sys.stderr)
    raise SystemExit(4)
if identifier not in ids:
    # Not answered rather than empty, deliberately: an identifier this repository
    # does not release says nothing about whether anything was released.
    print(f"'{identifier}' is not a release target this repository declares", file=sys.stderr)
    print("declared: " + ", ".join(ids), file=sys.stderr)
    raise SystemExit(5)
print(identifier)
PY
)" || case "$?" in
  3) refuse "the declaration could not be read (its reason is above)" \
    "install Python 3.11 or newer, or repair release-targets.toml so it parses as TOML" ;;
  4) refuse "$DECLARATION declares no release target at all" \
    "declare what this repository publishes as a [[target]] table with an id of <registry>:<name>" ;;
  5) refuse "that identifier is not one this repository declares (the declared set is above)" \
    "ask for one of the declared ids — or, if this repository has started publishing it, declare it as a [[target]] in release-targets.toml" ;;
  *) refuse "the declaration could not be read" \
    "check that python3 runs and that release-targets.toml is readable" ;;
esac
[ "$declared" = "$identifier" ] || refuse \
  "the declaration reader answered '$declared' for '$identifier'" \
  "re-run; nothing about a release has been established"

registry="${identifier%%:*}"
name="${identifier#*:}"

# The name becomes a path segment of a registry URL, so it is held to what a
# registry serves before it is spelled into one: either a plain name, or npm's
# scoped `@scope/name`. A leading `@` commits it to the scoped form and is decided
# there in full, which is what refuses `@`, `@/cli`, `@scope/` and a second slash.
case "$name" in
  @*)
    scope="${name%%/*}"
    package="${name#*/}"
    case "$name" in */*) ;; *) package="" ;; esac
    case "${scope#@}" in
      "" | [!A-Za-z0-9]* | *[!A-Za-z0-9._-]*) package="" ;;
    esac
    case "$package" in
      "" | [!A-Za-z0-9]* | *[!A-Za-z0-9._-]*)
        refuse "'$identifier' opens an npm scope it does not finish" \
          "spell a scoped name as @scope/name, exactly as npm serves it" ;;
    esac
    ;;
  "" | [!A-Za-z0-9]* | *[!A-Za-z0-9._-]*)
    refuse "'$identifier' has a name that is not one a registry serves" \
      "spell the name exactly as its registry does" ;;
esac

# One registry, one request, one field — each spelled exactly as
# config/registry-interfaces.toml pins it, which is what
# scripts/check-release-probe.sh reconciles them against.
encoded="$name"
case "$registry" in
  crate)
    url_template="https://crates.io/api/v1/crates/{name}"
    version_path="crate.max_stable_version"
    service="crates.io"
    # crates.io documents this field as nullable, and null there is the registry
    # itself saying it serves no stable version. Whether a null means that is per
    # registry rather than general: reading one that way where the registry does
    # not mean it would report a document this probe cannot understand as "nothing
    # published", which is the answer a consumer stops waiting on.
    null_is_no_release=1
    ;;
  pypi)
    url_template="https://pypi.org/pypi/{name}/json"
    version_path="info.version"
    service="PyPI"
    null_is_no_release=0
    ;;
  npm)
    url_template="https://registry.npmjs.org/{name}/latest"
    version_path="version"
    service="npm"
    null_is_no_release=0
    # A scoped name is one path segment, so its separator is percent-encoded.
    encoded="${name//\//%2F}"
    ;;
  *)
    refuse "'$identifier' names the registry '$registry', which this probe cannot read" \
      "add that registry to scripts/release-probe.sh and pin its interface in config/registry-interfaces.toml, or declare the target under a registry this probe reads" ;;
esac
url="${url_template/\{name\}/$encoded}"

# One registry read: the body, then its HTTP status. A transport failure is not an
# answer at all, so it never reaches the reading below.
body_file="$(mktemp)" || refuse \
  "could not create a temporary file to read the registry into" \
  "check the temporary directory's permissions and free space, then re-ask"
trap 'rm -f "$body_file"' EXIT
status=0
http_status="$(curl -q -sS --connect-timeout "$CONNECT_TIMEOUT" --max-time "$MAX_TIME" \
  -A "$USER_AGENT" -o "$body_file" -w '%{http_code}' -- "$url")" || status=$?
[ "$status" -eq 0 ] || refuse \
  "curl exited $status reading $url, so no registry answered (its diagnostic is above)" \
  "re-ask once the registry is reachable; nothing about a release has been established"
case "$http_status" in
  200) ;;
  # 404 is the one status that means "no release yet"; every other one is a
  # registry that did not say.
  404) exit 0 ;;
  *) refuse "$service answered $http_status for $identifier, so it cannot say what is released" \
    "re-ask once the registry answers; nothing about a release has been established" ;;
esac

version=""
status=0
version="$(python3 - "$version_path" "$body_file" "$null_is_no_release" <<'PY'
import json
import sys

path, body_file, null_is_no_release = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
try:
    with open(body_file, "rb") as handle:
        document = json.load(handle)
except (OSError, ValueError) as problem:
    print(f"the answer is not readable JSON: {problem}", file=sys.stderr)
    raise SystemExit(3)

value = document
for key in path.split("."):
    if not isinstance(value, dict) or key not in value:
        print(f"the answer carries no {path}", file=sys.stderr)
        raise SystemExit(3)
    value = value[key]

# A field the registry itself set to null, where that registry means by it that
# it serves no release: crates.io answers `"max_stable_version": null` for a crate
# whose every version is yanked or a prerelease. That is the empty answer, and it
# is the only way a 200 reaches it. Where the registry does not mean that — the
# other two never answer null — it is a document this cannot read, and reading it
# as "nothing published" would end a wait on evidence nobody gave.
if value is None:
    if null_is_no_release:
        raise SystemExit(4)
    print(f"{path} is null, which this registry does not use to mean it serves nothing", file=sys.stderr)
    raise SystemExit(3)
if not isinstance(value, str):
    print(f"{path} is {type(value).__name__} rather than a version string", file=sys.stderr)
    raise SystemExit(3)
print(value)
PY
)" || status=$?
case "$status" in
  0) ;;
  4) exit 0 ;;
  *) refuse "$service answered 200 for $identifier without a readable $version_path (its reason is above)" \
    "re-ask once the registry answers a document of the shape config/registry-interfaces.toml pins; nothing about a release has been established" ;;
esac

# The answer, held closed on its way out: what a caller reads on stdout is read as
# a released version, so a body that answered something which is not one is not
# answered at all.
case "$version" in
  "" | [!0-9]* | *[!0-9A-Za-z.+-]*)
    refuse "$service answered '$version' for $identifier, which is not a version" \
      "re-ask once the registry answers a version; nothing about a release has been established" ;;
esac
printf '%s\n' "$version"
