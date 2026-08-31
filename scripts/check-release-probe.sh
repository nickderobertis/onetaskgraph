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

# ---------------------------------------------------------------------------
# Half one: the probe against the pin. Text only, so it runs on every platform.
# ---------------------------------------------------------------------------
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
registries = pin.get("registry", [])
if not registries:
    refuse([f"{pin_path} pins no registry at all, so nothing holds the probe to an interface"])

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
if not targets:
    refuse([f"{declaration_path} declares no target, so there is nothing to prove the probe answers"])

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

    served = json.loads(registry["served_body"])
    (directory / "served.json").write_text(json.dumps(served), encoding="utf-8")
    (directory / "absent.json").write_text(registry["absent_body"].strip() + "\n", encoding="utf-8")

    # The same document with the pinned field taken away and every decoy left
    # standing: a probe reading the neighbour instead of the field answers this
    # one, and a probe reading the pinned field cannot.
    def descend(document, dotted):
        keys = dotted.split(".")
        for key in keys[:-1]:
            document = document[key]
        return document, keys[-1]

    decoyed = json.loads(registry["served_body"])
    holder, leaf = descend(decoyed, registry["version_path"])
    del holder[leaf]
    (directory / "decoyed.json").write_text(json.dumps(decoyed), encoding="utf-8")

    nulled = ""
    if registry["null_means_no_release"]:
        document = json.loads(registry["served_body"])
        holder, leaf = descend(document, registry["version_path"])
        holder[leaf] = None
        (directory / "nulled.json").write_text(json.dumps(document), encoding="utf-8")
        nulled = str(directory / "nulled.json")

    plan.append(
        "\t".join([
            identifier,
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

# ---------------------------------------------------------------------------
# Half two: the three answers, driven through the real probe.
# ---------------------------------------------------------------------------
# The stub is an extensionless executable earlier on PATH than curl, which is a
# Unix shape — the same reason scripts/check-real-release-preparation.sh names for
# its own skip. Windows keeps the reconciliation above, which is where a drifted
# lookup or a drifted field is caught; the Linux and macOS lanes drive the answers.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-release-probe: pinned-interface reconciliation passed; the stood-in answers are skipped on Windows (they need an extensionless curl earlier on PATH, which is a Unix shape) — the Linux and macOS lanes drive all three" >&2
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
previous=""
for argument in "$@"; do
  case "$previous" in
    -o) out="$argument" ;;
    -A) agent="$argument" ;;
  esac
  case "$argument" in
    -*) ;;
    *) [ "$previous" = "--" ] && url="$argument" ;;
  esac
  previous="$argument"
done
printf '%s\n' "$url" > "$STUB_REQUEST"
printf '%s\n' "$agent" > "$STUB_AGENT"
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
  STUB_REQUEST="$work/request" STUB_AGENT="$work/agent" STUB_BODY="$3" \
    STUB_STATUS="$2" STUB_TRANSPORT_FAILS="$4" PATH="$work/bin:$PATH" \
    "$PROBE" "$1" > "$work/out" 2> "$work/err" || probe_status=$?
}

while IFS=$'\t' read -r identifier expected_url served_status served_version absent_status directory nulled; do
  [ -n "$identifier" ] || continue

  # 1. The registry serves a version.
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
  if [ ! -s "$work/agent" ]; then
    fail "$identifier was looked up without a user agent, which crates.io answers 403"
  fi

  # 2. The registry serves nothing yet.
  run_probe "$identifier" "$absent_status" "$directory/absent.json" 0
  empty="$probe_status|$(cat "$work/out")"
  if [ "$probe_status" -ne 0 ]; then
    fail "$identifier was refused where the registry answered $absent_status, which is the registry saying it has no release: $(cat "$work/err")"
  elif [ "$empty" != "0|" ]; then
    fail "$identifier answered '$(cat "$work/out")' where the registry answered $absent_status; nothing published is the empty answer"
  fi

  # 3. The lookup could not be made.
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

  # 4. The pinned field is gone and its version-shaped neighbours remain: not
  #    answered, because the probe reads the field the pin names and no other.
  run_probe "$identifier" "$served_status" "$directory/decoyed.json" 0
  if [ "$probe_status" -eq 0 ]; then
    fail "$identifier answered '$(cat "$work/out")' from a document with the pinned field removed; scripts/release-probe.sh is reading a neighbouring field config/registry-interfaces.toml names as a decoy"
  fi

  # 5. Where the pin records that the registry itself answers null, that is the
  #    registry saying it serves nothing — the empty answer, not a refusal.
  if [ -n "$nulled" ]; then
    run_probe "$identifier" "$served_status" "$nulled" 0
    if [ "$probe_status" -ne 0 ] || [ -s "$work/out" ]; then
      fail "$identifier did not read a null pinned field as the registry serving nothing; config/registry-interfaces.toml records that this registry answers null for an artifact it has no released version of"
    fi
  fi
done < "$work/plan.tsv"

[ "$failures" -eq 0 ] || fatal \
  "scripts/release-probe.sh answered $failures case(s) wrongly (each is named above)" \
  "fix the branch of the probe each failure names; the three answers — a version, nothing, and not answered — must stay distinct, because a consumer holds on the third and stops holding on the second"
