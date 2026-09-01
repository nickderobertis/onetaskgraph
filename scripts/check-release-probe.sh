#!/usr/bin/env bash
# Hold scripts/release-probe.sh to the registries it really consumes, and drive
# every answer it can give without reaching one.
#
# Two halves, and neither stands without the other.
#
# **The pin.** config/registry-interfaces.toml records each registry's published
# interface — the request the probe makes and the field it reads out of the
# answer — with the date and the URL it was observed from. This reconciles the
# probe against it in both directions: a lookup the pin does not describe, a field
# the pin does not name, a registry one side handles and the other does not, and a
# field the pin names as a decoy — a version-shaped neighbour of the one that
# means what the probe wants — all fail here. That reconciliation is what makes
# the stood-in answers below evidence: a probe that could not really query a
# registry cannot pass, because the request it makes is held to the pin rather
# than to whatever shape a stub found convenient.
#
# **The answers.** The probe's three answers are driven end to end through the
# real script, against a curl that answers as each pinned document says the
# registry does. A version, nothing, and a refusal have to come back different
# from each other — a lookup that could not be made must never read as nothing
# having been released — so this asserts on all three and on their being
# distinct, for every target release-targets.toml declares.
#
# The versions in the pinned bodies are invented, so a case that somehow reached
# the real registry answers this repository's real version and fails here instead
# of passing on the wrong evidence.
#
# Quiet on success. On failure it names the drift and what to do about it.
set -euo pipefail

fatal() {
  echo "check-release-probe: $1" >&2
  echo "check-release-probe: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run this from a checkout of this repository, as 'just distribution-check' does"
readonly ROOT
cd "$ROOT" || fatal "could not enter $ROOT" "check that directory's permissions, then rerun"

readonly PIN="config/registry-interfaces.toml"
readonly PROBE="scripts/release-probe.sh"
readonly DECLARATION="release-targets.toml"

work="$(mktemp -d)" || fatal \
  "could not create a working directory" \
  "check the temporary directory's permissions and free space, then rerun"
trap 'rm -rf "$work"' EXIT

# Text only, so this half runs on every platform.
python3 - "$PIN" "$PROBE" "$DECLARATION" "$work" <<'PY' || fatal \
  "scripts/release-probe.sh and config/registry-interfaces.toml disagree (the drift is above)" \
  "bring the probe back to the pinned interface, or re-observe that registry's interface and record the new observation in the pin"
import json
import sys
from pathlib import Path
from urllib.parse import quote

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python older than 3.11
    print("this python3 has no tomllib, so neither document can be read", file=sys.stderr)
    print("install Python 3.11 or newer and rerun", file=sys.stderr)
    raise SystemExit(1)

pin_path, probe_path, declaration_path, work = (Path(argument) for argument in sys.argv[1:5])


def refuse(problems):
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    raise SystemExit(1)


def load(path):
    try:
        with open(path, "rb") as handle:
            return tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as problem:
        refuse([f"{path} could not be read as TOML: {problem}"])


pin = load(pin_path)
declaration = load(declaration_path)

# Both documents are input to this check rather than something it wrote, so every
# field the pin declares is held to its type here — one named refusal a reader can
# act on, rather than a traceback from the first place a string turned out to be a
# list.
PINNED_FIELDS = {
    "registry": str,
    "service": str,
    # The provenance: what was read, where, and when. A pin carrying none is a
    # recollection rather than an observation, so it is required like the rest.
    "documentation": str,
    "observed": str,
    "observed_from": str,
    "method": str,
    "url": str,
    "name_in_url": str,
    "version_path": str,
    "served_status": int,
    "absent_status": int,
    "served_version": str,
    "served_body": str,
    "absent_body": str,
    "null_means_no_release": bool,
    "decoy_paths": list,
    "user_agent_required": bool,
}
registries = pin.get("registry", [])
if not isinstance(registries, list) or not registries:
    refuse([f"{pin_path} pins no [[registry]] table at all, so nothing holds the probe to an interface"])
for position, registry in enumerate(registries, start=1):
    if not isinstance(registry, dict):
        refuse([f"{pin_path} [[registry]] #{position} is not a table"])
    for field, kind in PINNED_FIELDS.items():
        if field not in registry:
            refuse([f"{pin_path} [[registry]] #{position} is missing '{field}'"])
        # bool is a subclass of int, so an integer field would accept `true`.
        if not isinstance(registry[field], kind) or (kind is int and isinstance(registry[field], bool)):
            refuse([
                f"{pin_path} [[registry]] #{position} holds '{field}' as "
                f"{type(registry[field]).__name__}, and the pin's shape gives it {kind.__name__}"
            ])
    if registry["name_in_url"] not in {"verbatim", "percent-encoded"}:
        refuse([
            f"{pin_path} [[registry]] #{position} spells its name in the URL as "
            f"'{registry['name_in_url']}', which is neither 'verbatim' nor 'percent-encoded'"
        ])
    for decoy in registry["decoy_paths"]:
        if not isinstance(decoy, str):
            refuse([f"{pin_path} [[registry]] #{position} has a decoy path that is not a string"])
seen = {}
for position, registry in enumerate(registries, start=1):
    # Two tables for one registry would collapse into whichever came last, and the
    # probe would be reconciled against half a pin without anything saying so.
    if registry["registry"] in seen:
        refuse([
            f"{pin_path} pins '{registry['registry']}' twice, in [[registry]] "
            f"#{seen[registry['registry']]} and #{position}"
        ])
    seen[registry["registry"]] = position

probe_source = probe_path.read_text(encoding="utf-8")
# Instructions, not prose: the header deliberately explains which field must not
# be read, and a scan that could not tell those apart would make the explanation
# impossible to write down.
code = "\n".join(
    line.split("#", 1)[0] for line in probe_source.splitlines() if not line.lstrip().startswith("#")
)

problems = []

# Every lookup the probe can make, and every lookup the pin describes: the two
# sets must be the same one. A URL carrying the {name} placeholder is a lookup;
# the probe's own user-agent URL carries none and is not one.
probe_urls = set()
for fragment in code.replace('"', " ").replace("'", " ").split():
    if fragment.startswith("https://") and "{name}" in fragment:
        probe_urls.add(fragment)
pinned_urls = {registry["url"] for registry in registries}
for url in sorted(probe_urls - pinned_urls):
    problems.append(
        f"{probe_path} looks up {url}, which {pin_path} does not describe — a request "
        "nothing pinned it to is a request nothing has checked"
    )
for url in sorted(pinned_urls - probe_urls):
    problems.append(
        f"{pin_path} pins the lookup {url}, which {probe_path} never makes"
    )

# Every registry word the probe branches on, against the pinned set.
probe_registries = set()
for line in code.splitlines():
    stripped = line.strip()
    if stripped.endswith(")") and stripped[:-1].isalnum() and stripped[:-1].islower():
        probe_registries.add(stripped[:-1])
pinned_registries = {registry["registry"] for registry in registries}
for registry in sorted(pinned_registries - probe_registries):
    problems.append(f"{probe_path} has no branch for the pinned registry '{registry}'")

for registry in registries:
    name = registry["registry"]
    path = registry["version_path"]
    if path not in code:
        problems.append(
            f"{probe_path} never reads {path}, which is the field {pin_path} pins as "
            f"{name}'s released version"
        )
    for decoy in registry.get("decoy_paths", []):
        if decoy in code:
            problems.append(
                f"{probe_path} reads {decoy}, which {pin_path} names as a {name} field that "
                "looks like the released version and is not"
            )
    if registry.get("user_agent_required") and " -A " not in code:
        problems.append(
            f"{pin_path} records that {registry['service']} refuses a request that does not "
            f"identify its caller, and {probe_path} sends no user agent"
        )

if problems:
    refuse(problems)

# The cases the second half drives, derived from the pin and the declaration
# together rather than transcribed: one per declared target, so a target added to
# the declaration is a target proven here.
targets = declaration.get("target", [])
if not isinstance(targets, list) or not targets:
    refuse([f"{declaration_path} declares no [[target]], so there is nothing to prove the probe answers"])
for position, target in enumerate(targets, start=1):
    if not isinstance(target, dict) or not isinstance(target.get("id"), str) or ":" not in target["id"]:
        refuse([
            f"{declaration_path} [[target]] #{position} has no id of the form <registry>:<name>; "
            "scripts/check-release-targets.sh is what says why in full"
        ])

by_registry = {registry["registry"]: registry for registry in registries}
plan = []
for target in targets:
    identifier = target["id"]
    registry_name, _, artifact = identifier.partition(":")
    registry = by_registry.get(registry_name)
    if registry is None:
        refuse([
            f"{declaration_path} declares {identifier}, whose registry '{registry_name}' "
            f"{pin_path} does not pin — a target nothing can look up"
        ])
    in_url = artifact if registry["name_in_url"] == "verbatim" else quote(artifact, safe="@")
    directory = work / identifier.replace(":", "_").replace("/", "_")
    directory.mkdir(parents=True, exist_ok=True)

    def pinned_document(where):
        """The pinned body, as a JSON object; refused by name if it is not one."""
        try:
            document = json.loads(registry["served_body"])
        except ValueError as problem:
            refuse([f"{pin_path}'s {where} served_body is not JSON: {problem}"])
        if not isinstance(document, dict):
            refuse([f"{pin_path}'s {where} served_body is not a JSON object"])
        return document

    # A path the pinned body does not carry means the pin describes a document it
    # does not hold, which is the one thing this half cannot work around: every
    # case below is built by reaching that path.
    def descend(document, dotted):
        keys = dotted.split(".")
        for key in keys[:-1]:
            if not isinstance(document, dict) or key not in document:
                refuse([
                    f"{pin_path}'s {registry_name} served_body has no {dotted}, which is the "
                    "field it pins as that registry's released version"
                ])
            document = document[key]
        if not isinstance(document, dict) or keys[-1] not in document:
            refuse([
                f"{pin_path}'s {registry_name} served_body has no {dotted}, which is the field "
                "it pins as that registry's released version"
            ])
        return document, keys[-1]

    served = pinned_document(registry_name)
    (directory / "served.json").write_text(json.dumps(served), encoding="utf-8")
    (directory / "absent.json").write_text(registry["absent_body"].strip() + "\n", encoding="utf-8")

    # The same document with the pinned field taken away and every decoy left
    # standing: a probe reading the neighbour instead of the field answers this
    # one, and a probe reading the pinned field cannot.
    decoyed = pinned_document(registry_name)
    holder, leaf = descend(decoyed, registry["version_path"])
    del holder[leaf]
    (directory / "decoyed.json").write_text(json.dumps(decoyed), encoding="utf-8")

    # A registry that answers null in the field the probe reads. Whether that
    # means "serves nothing" is per registry — crates.io means it, the other two
    # never answer it — so both sides are driven, and the pin is what says which
    # this one is.
    document = pinned_document(registry_name)
    holder, leaf = descend(document, registry["version_path"])
    holder[leaf] = None
    (directory / "nulled.json").write_text(json.dumps(document), encoding="utf-8")
    nulled = "empty" if registry["null_means_no_release"] else "refused"

    plan.append(
        "\t".join([
            identifier,
            registry["method"],
            registry["url"].replace("{name}", in_url),
            str(registry["served_status"]),
            registry["served_version"],
            str(registry["absent_status"]),
            str(directory),
            nulled,
        ])
    )

(work / "plan.tsv").write_text("\n".join(plan) + "\n", encoding="utf-8")
PY

# The stub is an extensionless executable earlier on PATH than curl, which is a
# Unix shape — the same reason scripts/check-real-release-preparation.sh names for
# its own skip. Windows keeps the reconciliation above, which is where a drifted
# lookup or a drifted field is caught; the Linux and macOS lanes drive the answers.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-release-probe: skipped the stood-in answers on Windows (they need an extensionless curl on PATH); the Linux and macOS lanes drive them" >&2
    exit 0
    ;;
esac

mkdir -p "$work/bin"
# A curl that answers however a case asks it to, and records what it was asked
# for. Its variables are read by this stub; the probe still reads none of them.
cat > "$work/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -u
out=""
url=""
agent=""
# curl's own default, and what the caller gets unless it asks for another.
method="GET"
previous=""
for argument in "$@"; do
  case "$previous" in
    -o) out="$argument" ;;
    -A) agent="$argument" ;;
    -X | --request) method="$argument" ;;
  esac
  case "$argument" in
    -*) ;;
    *) [ "$previous" = "--" ] && url="$argument" ;;
  esac
  previous="$argument"
done
printf '%s\n' "$url" > "$STUB_REQUEST"
printf '%s\n' "$agent" > "$STUB_AGENT"
printf '%s\n' "$method" > "$STUB_METHOD"
if [ "${STUB_TRANSPORT_FAILS:-0}" = 1 ]; then
  echo "stub curl: could not resolve host" >&2
  exit 6
fi
[ -z "$out" ] || cp "$STUB_BODY" "$out"
printf '%s' "${STUB_STATUS:-200}"
STUB
chmod +x "$work/bin/curl"

failures=0
fail() {
  echo "check-release-probe: $1" >&2
  failures=$((failures + 1))
}

# run_probe <identifier> <status> <body-file> <transport-fails>
# Leaves the probe's stdout in $work/out, its stderr in $work/err, its exit status
# in $probe_status, and the URL the stub was asked for in $work/request.
probe_status=0
run_probe() {
  probe_status=0
  STUB_REQUEST="$work/request" STUB_AGENT="$work/agent" STUB_METHOD="$work/method" \
    STUB_BODY="$3" STUB_STATUS="$2" STUB_TRANSPORT_FAILS="$4" PATH="$work/bin:$PATH" \
    "$PROBE" "$1" > "$work/out" 2> "$work/err" || probe_status=$?
}

while IFS=$'\t' read -r identifier expected_method expected_url served_status served_version absent_status directory nulled; do
  [ -n "$identifier" ] || continue

  run_probe "$identifier" "$served_status" "$directory/served.json" 0
  answered="$probe_status|$(cat "$work/out")"
  if [ "$probe_status" -ne 0 ]; then
    fail "$identifier was not answered where the pinned document serves $served_version: $(cat "$work/err")"
  elif [ "$answered" != "0|$served_version" ]; then
    fail "$identifier answered '$answered' where config/registry-interfaces.toml's pinned document serves '$served_version'; scripts/release-probe.sh is reading something other than the pinned field"
  fi
  requested="$(cat "$work/request")"
  if [ "$requested" != "$expected_url" ]; then
    fail "$identifier was looked up at '$requested', and config/registry-interfaces.toml pins '$expected_url'; bring the URL scripts/release-probe.sh builds back to the pinned one"
  fi
  method="$(cat "$work/method")"
  if [ "$method" != "$expected_method" ]; then
    fail "$identifier was looked up with $method, and config/registry-interfaces.toml pins $expected_method as the method that registry answers this read on"
  fi
  if [ ! -s "$work/agent" ]; then
    fail "$identifier was looked up without a user agent, which crates.io answers 403"
  fi

  run_probe "$identifier" "$absent_status" "$directory/absent.json" 0
  empty="$probe_status|$(cat "$work/out")"
  if [ "$probe_status" -ne 0 ]; then
    fail "$identifier was refused where the registry answered $absent_status, which is the registry saying it has no release: $(cat "$work/err")"
  elif [ "$empty" != "0|" ]; then
    fail "$identifier answered '$(cat "$work/out")' where the registry answered $absent_status; nothing published is the empty answer"
  fi

  run_probe "$identifier" "$served_status" "$directory/served.json" 1
  refused="$probe_status|$(cat "$work/out")"
  if [ "$probe_status" -eq 0 ]; then
    fail "$identifier exited 0 where curl could not reach the registry; a lookup that could not be made must never read as nothing having been released"
  fi
  if [ -s "$work/out" ]; then
    fail "$identifier wrote '$(cat "$work/out")' to stdout while refusing; a refusal says nothing there, because an empty stdout is the answer 'nothing published'"
  fi
  if [ ! -s "$work/err" ]; then
    fail "$identifier refused without a reason on stderr, so a caller learns only that something went wrong"
  fi

  # The three, held apart. What a caller reads is the exit status and stdout
  # together, so that pair is what has to differ: a refusal that reads as the
  # empty answer, or an empty answer that reads as a version, would each end a
  # consumer's wait on evidence it does not have.
  if [ "$answered" = "$empty" ]; then
    fail "$identifier answers the same '$answered' (exit status and stdout) when the registry serves $served_version and when it serves nothing"
  fi
  if [ "$refused" = "$answered" ] || [ "$refused" = "$empty" ]; then
    fail "$identifier's refusal '$refused' (exit status and stdout) is indistinguishable from one of its two answers; a consumer reads a refusal as 'ask again' and an empty answer as 'not released yet'"
  fi

  # The pinned field gone and its version-shaped neighbours left standing: not
  # answered, because the probe reads the field the pin names and no other.
  run_probe "$identifier" "$served_status" "$directory/decoyed.json" 0
  if [ "$probe_status" -eq 0 ]; then
    fail "$identifier answered '$(cat "$work/out")' from a document with the pinned field removed; scripts/release-probe.sh is reading a neighbouring field config/registry-interfaces.toml names as a decoy"
  fi

  # What a null in that field means is per registry: crates.io says by it that
  # it serves no stable version, and the other two never answer it at all, so
  # reading one there as "nothing published" would end a wait on a document nobody
  # understood. Both sides are driven, from the pin's own `null_means_no_release`.
  run_probe "$identifier" "$served_status" "$directory/nulled.json" 0
  if [ "$nulled" = "empty" ]; then
    if [ "$probe_status" -ne 0 ] || [ -s "$work/out" ]; then
      fail "$identifier did not read a null pinned field as the registry serving nothing, and config/registry-interfaces.toml records that this registry answers null for an artifact it has no released version of"
    fi
  elif [ "$probe_status" -eq 0 ]; then
    fail "$identifier read a null pinned field as an answer, and config/registry-interfaces.toml records that this registry never means 'serves nothing' by null — so that document is one the probe could not read, not a release that has not happened"
  fi
done < "$work/plan.tsv"

# Each of these is a branch that ends without a version, and the one thing none
# of them may do is end at exit 0 with empty output — that is the registry's
# answer "no release yet", and a consumer stops waiting on it. So each is driven
# through the real script and held to a refusal a caller cannot mistake for it:
# non-zero, nothing on stdout, and a reason on stderr about the branch the case is
# actually testing.

# assert_refused <description> <reason fragment> <expected exit, or "" for any
# non-zero> <PATH> <probe> <status> <body> <transport-fails> [arguments...]
assert_refused() {
  local description=$1 expected=$2 wanted=$3 path=$4 script=$5 http=$6 body=$7 transport=$8
  shift 8
  local status=0
  STUB_REQUEST="$work/request" STUB_AGENT="$work/agent" STUB_METHOD="$work/method" \
    STUB_BODY="$body" STUB_STATUS="$http" STUB_TRANSPORT_FAILS="$transport" PATH="$path" \
    "$script" "$@" > "$work/out" 2> "$work/err" || status=$?
  if [ "$status" -eq 0 ]; then
    fail "$description was answered (exit 0) instead of refused; a caller cannot tell that from the registry saying it serves nothing"
    return
  fi
  # Where the probe's own contract names a status, that status is part of what a
  # caller reads: exit 2 says the caller asked wrongly rather than anything about
  # a release, and a refusal that reported it as 1 would say the opposite.
  if [ -n "$wanted" ] && [ "$status" -ne "$wanted" ]; then
    fail "$description exited $status, and scripts/release-probe.sh's documented contract gives that branch exit $wanted"
    return
  fi
  if [ -s "$work/out" ]; then
    fail "$description wrote '$(cat "$work/out")' to stdout while refusing; stdout carries the answer alone"
    return
  fi
  if [ ! -s "$work/err" ]; then
    fail "$description refused without a reason on stderr, so a caller learns only that something went wrong"
    return
  fi
  if ! grep -Fq "$expected" "$work/err"; then
    fail "$description was refused for some reason other than '$expected', so it exercised a branch it is not about: $(cat "$work/err")"
  fi
}

stub_path="$work/bin:$PATH"
first_target="$(head -n 1 "$work/plan.tsv" | cut -f 1)"
served_body="$(head -n 1 "$work/plan.tsv" | cut -f 7)/served.json"
served_http="$(head -n 1 "$work/plan.tsv" | cut -f 4)"

# The identifier: none, several, and one this repository does not release. The
# last is not answered rather than answered emptily, because an artifact nobody
# publishes says nothing about whether anything was released.
assert_refused "an invocation with no identifier" "takes exactly one" 2 \
  "$stub_path" "$PROBE" "$served_http" "$served_body" 0
assert_refused "an invocation with two identifiers" "takes exactly one" 2 \
  "$stub_path" "$PROBE" "$served_http" "$served_body" 0 "$first_target" "$first_target"
assert_refused "an identifier this repository does not declare" "is not one this repository declares" 1 \
  "$stub_path" "$PROBE" "$served_http" "$served_body" 0 "crate:not-a-thing-here"

# A declaration this probe cannot use. It resolves one from its own location, so
# each case is a copy of the script beside the declaration it is about.
scratch_probe() {
  local case_name=$1 declaration=$2
  mkdir -p "$work/$case_name/scripts"
  cp "$PROBE" "$work/$case_name/scripts/release-probe.sh"
  if [ -n "$declaration" ]; then
    printf '%s\n' "$declaration" > "$work/$case_name/release-targets.toml"
  fi
  printf '%s' "$work/$case_name/scripts/release-probe.sh"
}

assert_refused "a checkout carrying no declaration" "is missing or unreadable" "" \
  "$stub_path" "$(scratch_probe undeclared "")" "$served_http" "$served_body" 0 "$first_target"
assert_refused "a declaration that is not TOML" "could not be read" "" \
  "$stub_path" "$(scratch_probe malformed 'this is not = = toml')" "$served_http" "$served_body" 0 "$first_target"
assert_refused "a declaration with no target in it" "declares no release target at all" "" \
  "$stub_path" "$(scratch_probe empty 'schema_version = 2')" "$served_http" "$served_body" 0 "$first_target"
assert_refused "a target on a registry this probe cannot read" "cannot read" "" \
  "$stub_path" \
  "$(scratch_probe unknown_registry 'schema_version = 2
[[target]]
id = "gem:onetaskgraph"
name = "gem"
what = "A registry this probe has no branch for."
published_by = "nothing; this is a fixture."')" \
  "$served_http" "$served_body" 0 "gem:onetaskgraph"

# A host missing one of the two tools it runs. Not answered rather than answered
# emptily, and it says which tool: that is the whole of what such a host can be
# told.
mkdir -p "$work/minbin"
for tool in bash dirname mktemp rm; do
  # `type -P` resolves the executable FILE. `command -v` would answer with a
  # shell function where a developer's profile defines one, and the link made
  # from that answer would point at itself.
  tool_path="$(type -P "$tool")" || fatal \
    "no $tool on this host, so the restricted-PATH cases cannot be built" \
    "install $tool (it is in coreutils on Linux and macOS) and rerun"
  ln -sf "$tool_path" "$work/minbin/$tool"
done
ln -sf "$(type -P python3)" "$work/minbin/python3"
assert_refused "a host with no curl" "curl is not on PATH" "" \
  "$work/minbin" "$PROBE" "$served_http" "$served_body" 0 "$first_target"
rm -f "$work/minbin/python3"
ln -sf "$work/bin/curl" "$work/minbin/curl"
assert_refused "a host with no python3" "python3 is not on PATH" "" \
  "$work/minbin" "$PROBE" "$served_http" "$served_body" 0 "$first_target"

# A declared identifier whose name is not one a registry serves. It becomes a path
# segment of a URL, so it is refused before it is spelled into one — and refused
# rather than answered emptily, because a name nobody can look up says nothing
# about what has been released.
assert_refused "a declared name that opens an npm scope it does not finish" "opens an npm scope" "" \
  "$stub_path" \
  "$(scratch_probe unfinished_scope 'schema_version = 2
[[target]]
id = "npm:@onetaskgraph/"
name = "npm"
what = "A scoped name with no package after the scope."
published_by = "nothing; this is a fixture."')" \
  "$served_http" "$served_body" 0 "npm:@onetaskgraph/"
assert_refused "a declared name outside the alphabet a registry serves" "is not one a registry serves" "" \
  "$stub_path" \
  "$(scratch_probe unserved_name 'schema_version = 2
[[target]]
id = "crate:not a name"
name = "crate"
what = "A name with a space in it."
published_by = "nothing; this is a fixture."')" \
  "$served_http" "$served_body" 0 "crate:not a name"

# A host that cannot give the probe a temporary file to read the answer into. It
# is the registry read that has not happened, so it is not answered.
TMPDIR="$work/no-such-directory" assert_refused "a host with no writable temporary directory" \
  "could not create a temporary file" "" \
  "$stub_path" "$PROBE" "$served_http" "$served_body" 0 "$first_target"

# A version its own registry's grammar allows and another's does not: PyPI serves
# PEP 440, whose epoch has no semver spelling. It is answered rather than refused,
# because a refusal is what a consumer waits on forever.
printf '{"info":{"version":"1!2.0"}}\n' > "$work/epoch-version"
epoch_status=0
STUB_REQUEST="$work/request" STUB_AGENT="$work/agent" STUB_METHOD="$work/method" \
  STUB_BODY="$work/epoch-version" STUB_STATUS=200 STUB_TRANSPORT_FAILS=0 PATH="$stub_path" \
  "$PROBE" "pypi:onetaskgraph-cli" > "$work/out" 2> "$work/err" || epoch_status=$?
if [ "$epoch_status" -ne 0 ] || [ "$(cat "$work/out")" != "1!2.0" ]; then
  fail "a PEP 440 epoch version was not answered as PyPI served it; the outgoing guard in scripts/release-probe.sh is a shape check rather than one registry's grammar, because refusing a version a registry really serves is a wait that never ends"
fi

# An answer that is not one. Every case here is a registry that replied and could
# not be understood, which is the state most easily mistaken for "nothing
# published" and never is one.
printf 'not json at all\n' > "$work/not-json"
printf '{"crate":{"max_stable_version":9}}\n' > "$work/not-a-string"
printf '{"crate":{"max_stable_version":"not a version"}}\n' > "$work/not-a-version"
assert_refused "a registry answering 500" "cannot say what is released" "" \
  "$stub_path" "$PROBE" 500 "$served_body" 0 "$first_target"
assert_refused "a body that is not JSON" "without a readable" "" \
  "$stub_path" "$PROBE" "$served_http" "$work/not-json" 0 "$first_target"
assert_refused "a version field that is not a string" "without a readable" "" \
  "$stub_path" "$PROBE" "$served_http" "$work/not-a-string" 0 "$first_target"
assert_refused "a version field that is not a version" "which is not a version" "" \
  "$stub_path" "$PROBE" "$served_http" "$work/not-a-version" 0 "$first_target"

[ "$failures" -eq 0 ] || fatal \
  "scripts/release-probe.sh answered $failures case(s) wrongly (each is named above)" \
  "fix the branch of the probe each failure names; the three answers — a version, nothing, and not answered — must stay distinct, because a consumer holds on the third and stops holding on the second"
