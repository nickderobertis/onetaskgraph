#!/usr/bin/env bash
# Drive the release workflow's npm publication against a registry this check stands up.
#
# A release is otherwise the only thing that runs `publish-npm`, so the path is unexercised
# until a version has already been cut. This runs the real `scripts/publish-npm.sh` — the
# script that job invokes — against a registry on loopback, and reads back the packages it
# would send: their names, their versions, and that each is this repository's own rather
# than somebody else's of a similar name. The carriers are built here out of the same
# `npm/platforms/*/package.json` manifests the workflow uses, because a publication whose
# operands do not exist proves nothing about the operands that will.
# llmlint: ignore-file[new_code_lands_in_a_project] scripts/ is deliberately outside the
# Nx project graph (AGENTS.md, Conventions): Nx maps no project to it, which is why the
# justfile invokes these from recipes of its own. Nothing here escapes the gate — it
# runs unconditionally from `just distribution-test` rather than by affected selection —
# so the graph's absence costs an optimisation rather than the coverage this rule
# protects.
set -euo pipefail

fatal() {
  echo "check-npm-publish: $1" >&2
  echo "check-npm-publish: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-test' does"
readonly ROOT
cd "$ROOT" || fatal "could not enter $ROOT" "check that directory's permissions, then rerun"

for tool in node npm python3; do
  command -v "$tool" >/dev/null 2>&1 || fatal \
    "$tool is not installed, and the npm publication cannot be driven without it" \
    "install it (scripts/provision-gate.sh names what this workspace needs), then rerun"
done

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check publishes into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
REGISTRY_PID=""
cleanup() {
  # Only the process this script started, by the pid it was given.
  [ -z "$REGISTRY_PID" ] || kill "$REGISTRY_PID" 2>/dev/null || true
  rm -rf "$scratch"
}
trap cleanup EXIT

readonly PUBLISHED="$scratch/published.jsonl"
readonly PORT_FILE="$scratch/port"
# What the registry stands in for at any moment, read per request so one server covers
# every branch the publication has: `record` accepts publications and remembers them,
# `refuse-reads` answers every query 403, and `refuse-writes` reports every package
# absent and then refuses the publication itself.
readonly MODE_FILE="$scratch/mode"

# The registry. Recording, it answers a read 404 until that exact version has been
# published to it and 200 afterwards — which is what makes the re-run of a partly finished
# publication readable here — and records each publication as one JSON line naming the
# package, the version and the tarball it carried.
cat > "$scratch/registry.py" <<'PY'
import json
import socketserver
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

published, port_file, mode_file = sys.argv[1], sys.argv[2], sys.argv[3]

# package name -> the versions this registry has been sent. A publication asks about a
# version before sending it, and a registry that forgets what it was given cannot tell
# the "already there, leave it alone" branch from the "absent, send it" one.
holdings = {}


def mode():
    """Which registry this is standing in for right now.

    Read per request rather than once at startup: the check moves this one server between
    modes, and a mode read once would answer every later case as the first one.
    """
    try:
        with open(mode_file, encoding="utf-8") as handle:
            return handle.read().strip() or "record"
    except FileNotFoundError:
        return "record"


class Registry(BaseHTTPRequestHandler):
    def _answer(self, status, body):
        encoded = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        # npm caches registry reads, and a cached 404 would answer the re-run below
        # instead of this server — reporting a package this registry holds as absent.
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self):
        current = mode()
        if current == "refuse-reads":
            self._answer(403, {"error": "Forbidden"})
            return
        name = urllib.parse.unquote(self.path.split("?", 1)[0].lstrip("/"))
        versions = [] if current == "refuse-writes" else holdings.get(name, [])
        if not versions:
            # Absent, which is the answer that tells the publication to send it.
            self._answer(404, {"error": "Not found"})
            return
        self._answer(
            200,
            {
                "name": name,
                "dist-tags": {"latest": versions[-1]},
                "versions": {
                    version: {
                        "name": name,
                        "version": version,
                        "dist": {
                            "tarball": f"http://127.0.0.1/{name}/-/{version}.tgz",
                            "shasum": "0" * 40,
                        },
                    }
                    for version in versions
                },
            },
        )

    def do_PUT(self):
        # The header is untrusted input like any other: a length that is not a
        # non-negative decimal is refused as a bad request rather than raising out of
        # the handler, which would answer the publication with a closed connection and
        # report a framing mistake as npm being unreachable.
        raw = self.headers.get("Content-Length", "0")
        if not raw.strip().isdigit():
            self._answer(400, {"error": f"Content-Length is not a length: {raw!r}"})
            return
        length = int(raw)
        # Read either way, and before answering: a refusal sent while the body is still
        # arriving closes the connection under npm, which reports it as the registry
        # being unreachable rather than as the refusal it is.
        body = self.rfile.read(length)
        if mode() == "refuse-writes":
            self._answer(403, {"error": "Forbidden"})
            return
        try:
            document = json.loads(body or b"{}")
        except json.JSONDecodeError as error:
            self._answer(400, {"error": str(error)})
            return
        versions = document.get("versions") or {}
        with open(published, "a", encoding="utf-8") as record:
            for version, manifest in versions.items():
                holdings.setdefault(document.get("name"), []).append(version)
                record.write(
                    json.dumps(
                        {
                            "path": self.path,
                            "name": document.get("name"),
                            "version": version,
                            "manifest_name": manifest.get("name"),
                            "attachments": sorted(document.get("_attachments") or {}),
                        }
                    )
                    + "\n"
                )
        self._answer(201, {"ok": True})

    def log_message(self, *_):
        """Quiet: this check's own output is the signal."""


class Loopback(HTTPServer):
    """Bound without the reverse DNS lookup `HTTPServer` does on its own account.

    `HTTPServer.server_bind` calls `socket.getfqdn(host)` to name itself, and on the
    macOS runner that lookup of 127.0.0.1 outlasted the start-up window the shell below
    waits out — so a registry that had in fact bound was reported as one that never
    reported a port, and the whole install-path lane failed on a name nothing reads.
    """

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = self.server_address[1]


server = Loopback(("127.0.0.1", 0), Registry)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
PY

: > "$PUBLISHED"
printf 'record\n' > "$MODE_FILE"
# Its own stderr, kept rather than let out: a registry that cannot start says why there,
# and the wait below is the only thing that would otherwise notice — reporting the silence
# and not the reason.
readonly REGISTRY_LOG="$scratch/registry.log"
python3 "$scratch/registry.py" "$PUBLISHED" "$PORT_FILE" "$MODE_FILE" 2>"$REGISTRY_LOG" &
REGISTRY_PID=$!

# Thirty seconds, which is what scripts/test-distribution.sh gives the three servers it
# stands up the same way. That script's servers bound on the macOS runner in the very job
# where this one was killed for reporting nothing, and the wait is the only thing that
# differed — so a start-up this host is slow at is a slow lane there and a failed lane here.
# The liveness break keeps that generosity off the one case it would only make slower: a
# registry that has already exited is never going to write the file.
registry_exit=""
for _ in $(seq 1 300); do
  [ -s "$PORT_FILE" ] && break
  if ! kill -0 "$REGISTRY_PID" 2>/dev/null; then
    if wait "$REGISTRY_PID"; then registry_exit=0; else registry_exit=$?; fi
    REGISTRY_PID=""
    break
  fi
  sleep 0.1
done
if [ ! -s "$PORT_FILE" ]; then
  if [ -s "$REGISTRY_LOG" ]; then
    sed 's/^/check-npm-publish:   /' "$REGISTRY_LOG" >&2
    fatal \
      "the stub registry never reported a port, and said above why it could not" \
      "fix what it reported there, then rerun"
  fi
  [ -z "$registry_exit" ] || fatal \
    "the stub registry exited with status $registry_exit before reporting a port, printing nothing on its way out" \
    "run 'python3 -V' — nothing was captured from the registry, so start with whether this interpreter runs at all — then rerun"
  # Silence is a different failure from a traceback, and it owes a different next step. An
  # interpreter without http.server raises on import and would have printed it above, so
  # naming that import here sent the reader to the one thing already ruled out — which is
  # what the macOS lane's whole install path was read as. What is left when nothing was
  # printed is the bind, so that is what the next step below reaches for.
  fatal \
    "the stub registry bound no port within 30s and printed nothing about why" \
    "run: python3 -c 'import socket; s = socket.socket(); s.bind((\"127.0.0.1\", 0)); print(s.getsockname())' — which is all this registry does before it writes the file — then rerun"
fi
port="$(cat "$PORT_FILE")"
# What the file holds is a port only because the registry above put it there, and reading
# it as one without saying so is how a truncated or half-written file becomes a URL that
# fails several steps later as npm being unreachable.
case $port in
  '' | *[!0-9]*) port=""; ;;
esac
if [ -z "$port" ] || [ "$port" -lt 1 ] || [ "$port" -gt 65535 ]; then
  fatal \
    "the stub registry reported '$(cat "$PORT_FILE")' where a port number belongs" \
    "report this — $PORT_FILE is written by this check's own registry and nothing else"
fi
readonly REGISTRY="http://127.0.0.1:$port/"

# The carriers, built the way .github/workflows/release.yml builds them: one directory per
# platform holding that platform's manifest and a binary, packed with `npm pack`. The
# binary is a stand-in — what is under test is which package is published, not what is
# inside it, and building five real cross-compiled binaries is the release's job.
carriers="$scratch/carriers"
mkdir -p "$carriers" || fatal "could not create $carriers" "check \$TMPDIR, then rerun"
for package in npm/platforms/*; do
  platform="${package##*/}"
  carrier="$scratch/carrier-$platform/bin"
  mkdir -p "$carrier" || fatal "could not create $carrier" "check \$TMPDIR, then rerun"
  cp "$package/package.json" "$scratch/carrier-$platform/package.json" || fatal \
    "could not copy $package/package.json" "check \$TMPDIR, then rerun"
  printf 'stand-in for the released binary\n' > "$carrier/onetaskgraph"
  npm pack "$scratch/carrier-$platform" --pack-destination "$carriers" >/dev/null 2>&1 || fatal \
    "npm pack refused the $platform carrier" \
    "run 'npm pack npm/platforms/$platform' and fix what it reports"
done

# The TypeScript SDK publishes what it built, and the manifest names `dist`. Nothing here
# reads it, so a stand-in is enough to make the pack succeed.
built_sdk=""
if [ ! -d sdks/typescript/dist ]; then
  mkdir -p sdks/typescript/dist || fatal \
    "could not create sdks/typescript/dist" "check the permissions of $ROOT, then rerun"
  printf 'export {};\n' > sdks/typescript/dist/index.js
  printf 'export {};\n' > sdks/typescript/dist/index.d.ts
  built_sdk=yes
fi
remove_stand_in() {
  [ -z "$built_sdk" ] || rm -rf sdks/typescript/dist
}
trap 'remove_stand_in; cleanup' EXIT

# Without a token, npm packs every tarball in full and then fails ENEEDAUTH as though it
# were logged out — so the publication refuses first, saying what to set. The token is read
# through a default before its length is taken, because under `set -u` an unset one would
# end the script on bash's own diagnostic instead of this message.
refusal="$scratch/no-token.log"
if NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$carriers" RUNNER_TEMP="$scratch" \
  scripts/publish-npm.sh > "$refusal" 2>&1; then
  cat "$refusal" >&2
  fatal "scripts/publish-npm.sh published with no NPM_TOKEN set" \
    "restore the token guard at the top of it"
fi
for term in "NPM_TOKEN is required" "received 0 characters" "gh-secrets.json"; do
  grep -qF -- "$term" "$refusal" || {
    cat "$refusal" >&2
    fatal "the missing-token refusal never mentions '$term'" \
      "restore the token guard's message in scripts/publish-npm.sh, which must say what to set"
  }
done
if [ -s "$PUBLISHED" ]; then
  fatal "scripts/publish-npm.sh reached the registry with no NPM_TOKEN set" \
    "the token guard must refuse before anything is sent"
fi

# How many packages have reached the registry so far. Every refusal below is a refusal to
# send anything, and a refusal that sends first is the one failure this whole check exists
# to catch — so each case reads this before and after.
sent_lines() {
  wc -l < "$PUBLISHED" | tr -d ' '
}

# A registry that is not an http(s) URL npm can reach. npm would read it its own way and
# the npmrc built from it authenticates nothing, so the publication refuses before it packs
# anything. A scheme is not a URL on its own, which is why the last two are here: each
# satisfies a prefix test and then fails inside npm, about a value this script chose.
for bad_registry in "registry.npmjs.org" "https://" "https://registry npmjs org/"; do
  refusal="$scratch/bad-registry.log"
  status=0
  NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$bad_registry" NPM_CARRIERS="$carriers" \
    RUNNER_TEMP="$scratch" scripts/publish-npm.sh > "$refusal" 2>&1 || status=$?
  [ "$status" -eq 64 ] || {
    cat "$refusal" >&2
    fatal "the registry '$bad_registry' was accepted (exit $status, expected 64)" \
      "restore the NPM_REGISTRY guard in scripts/publish-npm.sh"
  }
  for term in "NPM_REGISTRY must be an http or https URL" "$bad_registry" "next:"; do
    grep -qF -- "$term" "$refusal" || {
      cat "$refusal" >&2
      fatal "the refusal of '$bad_registry' never mentions '$term'" \
        "its message must name the value it refused and what to do about it"
    }
  done
  [ "$(sent_lines)" -eq 0 ] || fatal \
    "the publication reached the registry with NPM_REGISTRY set to '$bad_registry'" \
    "the guard must refuse before anything is sent"
done

# A carriers directory that is not there is a download step that did not run. Left to npm
# it surfaces one tarball at a time, after the first carrier has already landed.
refusal="$scratch/no-carriers.log"
status=0
NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$scratch/absent" \
  RUNNER_TEMP="$scratch" scripts/publish-npm.sh > "$refusal" 2>&1 || status=$?
[ "$status" -eq 64 ] || {
  cat "$refusal" >&2
  fatal "a missing carrier directory was accepted (exit $status, expected 64)" \
    "restore the NPM_CARRIERS guard in scripts/publish-npm.sh"
}
for term in "no carrier directory at $scratch/absent" "NPM_CARRIERS" "next:"; do
  grep -qF -- "$term" "$refusal" || {
    cat "$refusal" >&2
    fatal "the missing-carriers refusal never mentions '$term'" \
      "its message must name the directory it looked in and what to do about it"
  }
done
[ "$(sent_lines)" -eq 0 ] || fatal \
  "the publication reached the registry with no carriers to send" \
  "the guard must refuse before anything is sent"

# A scratch directory that is not there. It is where npm's own output is held and replayed
# when a publication fails, so left to a shell redirection it ends the publication on
# bash's `No such file or directory` about a path nobody named — after carriers have
# already gone to the registry. The npmrc is pointed elsewhere here because that step reads
# RUNNER_TEMP too, and would otherwise create the very directory this case is about.
refusal="$scratch/no-scratch.log"
status=0
NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$carriers" \
  ONETASKGRAPH_NPM_CONFIG_DIR="$scratch" \
  RUNNER_TEMP="$scratch/absent-scratch" scripts/publish-npm.sh > "$refusal" 2>&1 || status=$?
[ "$status" -eq 64 ] || {
  cat "$refusal" >&2
  fatal "a scratch directory that is not there was accepted (exit $status, expected 64)" \
    "restore the RUNNER_TEMP guard in scripts/publish-npm.sh"
}
for term in "no writable scratch directory at $scratch/absent-scratch" "RUNNER_TEMP" "next:"; do
  grep -qF -- "$term" "$refusal" || {
    cat "$refusal" >&2
    fatal "the missing-scratch refusal never mentions '$term'" \
      "its message must name the directory it looked in and what to do about it"
  }
done
[ "$(sent_lines)" -eq 0 ] || fatal \
  "the publication reached the registry with nowhere to hold what npm reported" \
  "the guard must refuse before anything is sent"

# A carriers directory that is there and is one carrier short. The publication reads every
# operand before it sends any, so this is found before the first carrier lands rather than
# at the fourth — a release half at npm is what the whole re-run path exists to survive.
short="$scratch/short-carriers"
mkdir -p "$short" || fatal "could not create $short" "check \$TMPDIR, then rerun"
cp "$carriers"/*.tgz "$short/" || fatal \
  "could not copy the carriers into $short" "check \$TMPDIR, then rerun"
withheld="$(ls "$short"/*.tgz | tail -1)"
rm "$withheld" || fatal "could not remove $withheld" "check \$TMPDIR, then rerun"
refusal="$scratch/short-carriers.log"
status=0
NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$short" \
  RUNNER_TEMP="$scratch" scripts/publish-npm.sh > "$refusal" 2>&1 || status=$?
[ "$status" -eq 64 ] || {
  cat "$refusal" >&2
  fatal "a carriers directory missing a tarball was accepted (exit $status, expected 64)" \
    "every package operand must be checked before the first one is published"
}
for term in "not there" "$withheld" "next:"; do
  grep -qF -- "$term" "$refusal" || {
    cat "$refusal" >&2
    fatal "the missing-tarball refusal never mentions '$term'" \
      "it must name the operand it could not find and what to do about it"
  }
done
[ "$(sent_lines)" -eq 0 ] || fatal \
  "the publication sent a carrier before noticing another was missing" \
  "read every operand before publishing any of them"

# A sandbox checkout of the operands, so the manifest cases below can hand this
# publication a file it cannot read without corrupting the tree the rest of this check
# publishes out of. The publication resolves its own root from BASH_SOURCE, so a copy of
# it under this tree reads that tree's manifests.
sandbox="$scratch/sandbox"
mkdir -p "$sandbox/scripts" "$sandbox/npm/cli" "$sandbox/sdks/typescript" || fatal \
  "could not create the sandbox checkout at $sandbox" "check \$TMPDIR, then rerun"
cp scripts/publish-npm.sh scripts/npm-registry-auth.sh "$sandbox/scripts/" || fatal \
  "could not copy the publication into $sandbox/scripts" "check \$TMPDIR, then rerun"
cp npm/cli/package.json "$sandbox/npm/cli/package.json" || fatal \
  "could not copy npm/cli/package.json into $sandbox" "check \$TMPDIR, then rerun"
cp sdks/typescript/package.json "$sandbox/sdks/typescript/package.json" || fatal \
  "could not copy sdks/typescript/package.json into $sandbox" "check \$TMPDIR, then rerun"
victim=""
for package in npm/platforms/*; do
  platform="${package##*/}"
  [ -n "$victim" ] || victim="$platform"
  mkdir -p "$sandbox/npm/platforms/$platform" || fatal \
    "could not create $sandbox/npm/platforms/$platform" "check \$TMPDIR, then rerun"
  cp "$package/package.json" "$sandbox/npm/platforms/$platform/package.json" || fatal \
    "could not copy $package/package.json into $sandbox" "check \$TMPDIR, then rerun"
done

# A carrier manifest this publication cannot read. node has already said on stderr why it
# could not read it, and what turns that stack trace into a diagnostic is the guard below
# naming the manifest and the field to set — which happens only because the value node
# could not produce is allowed to fall through instead of ending the script where it stood.
printf 'this is not JSON\n' > "$sandbox/npm/platforms/$victim/package.json" || fatal \
  "could not rewrite the $victim carrier manifest in $sandbox" "check \$TMPDIR, then rerun"
refusal="$scratch/unreadable-carrier.log"
status=0
NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$carriers" \
  RUNNER_TEMP="$scratch" "$sandbox/scripts/publish-npm.sh" > "$refusal" 2>&1 || status=$?
[ "$status" -eq 64 ] || {
  cat "$refusal" >&2
  fatal "a carrier manifest that is not JSON was accepted (exit $status, expected 64)" \
    "the value node could not read must reach the name guard, which names the file to fix"
}
for term in "SyntaxError" "invalid carrier name" "npm/platforms/$victim/package.json" "next:"; do
  grep -qF -- "$term" "$refusal" || {
    cat "$refusal" >&2
    fatal "the unreadable-carrier-manifest refusal never mentions '$term'" \
      "it must replay node's own cause and name the manifest and the field to set"
  }
done
[ "$(sent_lines)" -eq 0 ] || fatal \
  "the publication reached the registry with a carrier manifest it could not read" \
  "every manifest is read before anything is sent"
cp "npm/platforms/$victim/package.json" "$sandbox/npm/platforms/$victim/package.json" || fatal \
  "could not restore the $victim carrier manifest in $sandbox" "check \$TMPDIR, then rerun"

# The launcher's manifest and the SDK's, unreadable the same way and owed the same
# treatment: the version guards are what name the manifest and the command that sets it.
for unreadable in "npm/cli:invalid CLI version" "sdks/typescript:invalid TypeScript SDK version"; do
  directory="${unreadable%%:*}"
  expected="${unreadable#*:}"
  printf 'this is not JSON\n' > "$sandbox/$directory/package.json" || fatal \
    "could not rewrite $directory/package.json in $sandbox" "check \$TMPDIR, then rerun"
  refusal="$scratch/unreadable-${directory//\//-}.log"
  status=0
  NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$carriers" \
    RUNNER_TEMP="$scratch" "$sandbox/scripts/publish-npm.sh" > "$refusal" 2>&1 || status=$?
  [ "$status" -eq 64 ] || {
    cat "$refusal" >&2
    fatal "a $directory manifest that is not JSON was accepted (exit $status, expected 64)" \
      "the value node could not read must reach the version guard for $directory"
  }
  for term in "SyntaxError" "$expected" "$directory/package.json" "scripts/set-version.sh"; do
    grep -qF -- "$term" "$refusal" || {
      cat "$refusal" >&2
      fatal "the unreadable-$directory-manifest refusal never mentions '$term'" \
        "it must replay node's own cause and name the manifest and the command that sets it"
    }
  done
  [ "$(sent_lines)" -eq 0 ] || fatal \
    "the publication reached the registry with $directory/package.json unreadable" \
    "every manifest is read before anything is sent"
  cp "$directory/package.json" "$sandbox/$directory/package.json" || fatal \
    "could not restore $directory/package.json in $sandbox" "check \$TMPDIR, then rerun"
done

# An npmrc this publication cannot write. Left to fall through, npm packs every tarball in
# full and then fails ENEEDAUTH exactly as though it were logged out — which reads as the
# registry refusing this package rather than as the setup problem it is. The directory is
# blocked by a plain file rather than by permissions, so this case runs the same for a
# root runner and on a platform with no POSIX mode bits.
blocker="$scratch/npmrc-blocker"
: > "$blocker" || fatal "could not create $blocker" "check \$TMPDIR, then rerun"
refusal="$scratch/unwritable-npmrc.log"
status=0
NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$carriers" \
  RUNNER_TEMP="$scratch" ONETASKGRAPH_NPM_CONFIG_DIR="$blocker/inside" \
  scripts/publish-npm.sh > "$refusal" 2>&1 || status=$?
[ "$status" -eq 70 ] || {
  cat "$refusal" >&2
  fatal "an npmrc that could not be written was accepted (exit $status, expected 70)" \
    "the publication must stop in its own words when it cannot configure authentication"
}
for term in "could not create the npm configuration directory" \
  "could not write the npmrc that authenticates to $REGISTRY" "ENEEDAUTH" "next:"; do
  grep -qF -- "$term" "$refusal" || {
    cat "$refusal" >&2
    fatal "the unwritable-npmrc refusal never mentions '$term'" \
      "it must replay what scripts/npm-registry-auth.sh reported and say what to do about it"
  }
done
[ "$(sent_lines)" -eq 0 ] || fatal \
  "the publication reached the registry with no npmrc to authenticate with" \
  "the npmrc is written before anything is sent"

log="$scratch/publish.log"
if ! NODE_AUTH_TOKEN=stub-token \
  NPM_REGISTRY="$REGISTRY" \
  NPM_CARRIERS="$carriers" \
  RUNNER_TEMP="$scratch" \
  scripts/publish-npm.sh > "$log" 2>&1; then
  cat "$log" >&2
  fatal "scripts/publish-npm.sh failed against a registry that accepts everything" \
    "fix what it reports above; the release workflow runs exactly this script"
fi

# What it sent, read back off the registry's own record.
NPM_PUBLISHED="$PUBLISHED" NPM_CHECKOUT="$ROOT" python3 <<'PY' || exit 1
import json
import os
import pathlib
import sys


def refuse(problem, action):
    print(f"check-npm-publish: {problem}", file=sys.stderr)
    print(f"check-npm-publish: next: {action}", file=sys.stderr)
    raise SystemExit(1)


root = pathlib.Path(os.environ["NPM_CHECKOUT"])
lines = [
    json.loads(line)
    for line in pathlib.Path(os.environ["NPM_PUBLISHED"]).read_text(encoding="utf-8").splitlines()
    if line.strip()
]
if not lines:
    refuse(
        "the publication sent nothing to the registry at all.",
        "run scripts/publish-npm.sh by hand and read its output — a publication that "
        "publishes nothing is why this repository is absent from npm.",
    )

# The version the release names is the one every manifest in this tree carries;
# scripts/set-version.sh keeps them in step and `just distribution-check` proves it.
expected_version = json.loads(
    (root / "npm/cli/package.json").read_text(encoding="utf-8")
)["version"]

expected = {"@onetaskgraph/cli", "@onetaskgraph/sdk"} | {
    f"@onetaskgraph/cli-{path.name}" for path in sorted((root / "npm/platforms").iterdir())
}

sent = {}
for line in lines:
    name = line["name"] or line["manifest_name"]
    if name != line["manifest_name"]:
        refuse(
            f"the registry was sent {name!r} whose own manifest says "
            f"{line['manifest_name']!r}.",
            "the two must agree, or the package that lands is not the one this "
            "repository built.",
        )
    sent[name] = line["version"]

missing = sorted(expected - set(sent))
if missing:
    refuse(
        "the publication never sent: " + ", ".join(missing) + ".",
        "restore those operands in scripts/publish-npm.sh — a package it does not send is "
        "a package this repository stays absent from npm for.",
    )
unexpected = sorted(set(sent) - expected)
if unexpected:
    refuse(
        "the publication sent packages this repository does not own: "
        + ", ".join(unexpected)
        + ".",
        "correct the operands in scripts/publish-npm.sh; a bare `npm/cli` operand is npm's "
        "shorthand for a GitHub repository, not this checkout's directory.",
    )
wrong = sorted(name for name, version in sent.items() if version != expected_version)
if wrong:
    refuse(
        f"these packages were published at a version other than {expected_version}, which "
        "is the version this release names: " + ", ".join(wrong) + ".",
        "run 'scripts/set-version.sh <VERSION>' so every manifest agrees, then rerun.",
    )
PY

# The recovery this script's own documentation promises: a release that partly published
# is re-run, and a package already at the registry at that exact version is left alone
# rather than republished. The registry now holds everything the run above sent, so a
# second run must reach it and send nothing.
before="$(sent_lines)"
rerun="$scratch/rerun.log"
if ! NODE_AUTH_TOKEN=stub-token \
  NPM_REGISTRY="$REGISTRY" \
  NPM_CARRIERS="$carriers" \
  RUNNER_TEMP="$scratch" \
  scripts/publish-npm.sh > "$rerun" 2>&1; then
  cat "$rerun" >&2
  fatal "re-running the publication against a registry that already holds it failed" \
    "a version already published must be left alone, so a partly published release can be re-run"
fi
[ "$(sent_lines)" -eq "$before" ] || {
  cat "$rerun" >&2
  fatal "the re-run republished packages the registry already held" \
    "restore the 'npm view' check in publish_if_absent, which is what makes a re-run safe"
}

# A registry that answers the query with something other than a 404 is not saying the
# package is absent, and publishing over that answer is how a release sends a package
# nobody could see. It stops, distinguishably: exit 69, and the registry it could not ask.
printf 'refuse-reads\n' > "$MODE_FILE"
refusal="$scratch/unreadable.log"
status=0
NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$carriers" \
  RUNNER_TEMP="$scratch" scripts/publish-npm.sh > "$refusal" 2>&1 || status=$?
[ "$status" -eq 69 ] || {
  cat "$refusal" >&2
  fatal "a registry that refused the query was published to anyway (exit $status, expected 69)" \
    "publish_if_absent must only publish on a 404 — every other answer is a stop"
}
for term in "could not query npm for" "$REGISTRY" "next:"; do
  grep -qF -- "$term" "$refusal" || {
    cat "$refusal" >&2
    fatal "the unreadable-registry refusal never mentions '$term'" \
      "its message must name the registry it could not ask and what to do about it"
  }
done

# And a registry that refuses the publication itself: npm reports it in a dozen lines this
# script holds until it matters, and the failure replays them and names what was refused.
printf 'refuse-writes\n' > "$MODE_FILE"
before="$(sent_lines)"
refusal="$scratch/refused.log"
status=0
NODE_AUTH_TOKEN=stub-token NPM_REGISTRY="$REGISTRY" NPM_CARRIERS="$carriers" \
  RUNNER_TEMP="$scratch" scripts/publish-npm.sh > "$refusal" 2>&1 || status=$?
[ "$status" -eq 1 ] || {
  cat "$refusal" >&2
  fatal "a refused publication reported success (exit $status, expected 1)" \
    "publish_if_absent must exit non-zero when 'npm publish' does"
}
for term in "npm refused to publish" "@onetaskgraph/cli-" "next:"; do
  grep -qF -- "$term" "$refusal" || {
    cat "$refusal" >&2
    fatal "the refused-publication message never mentions '$term'" \
      "it must name the package npm refused and replay what npm said"
  }
done
[ "$(sent_lines)" -eq "$before" ] || fatal \
  "the registry recorded a publication it refused" \
  "this is the stub registry disagreeing with itself; check its refuse-writes branch"
printf 'record\n' > "$MODE_FILE"
