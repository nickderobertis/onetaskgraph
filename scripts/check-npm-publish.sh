#!/usr/bin/env bash
# Drive the release workflow's npm publication against a registry this check stands up.
#
# This repository is absent from npm although its release workflow has carried a
# `publish-npm` job for several versions, and the reason that could sit there unnoticed is
# that a release is the only thing which runs it: the path was unexercised until a version
# had already been cut. So this runs the real `scripts/publish-npm.sh` — the same script
# the job invokes — against a registry on loopback, and reads back the packages it would
# send: their names, their versions, and the fact that each is this repository's own
# package rather than somebody else's of a similar name.
#
# The carriers are built here the way the workflow builds them, out of the same
# `npm/platforms/*/package.json` manifests, because a publication whose operands do not
# exist proves nothing about the operands that will.
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

# The registry. It answers every read 404 — nothing has been published to it — and records
# each publication as one JSON line naming the package, the version and the tarball it
# carried, which is what this check reads back.
cat > "$scratch/registry.py" <<'PY'
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

published, port_file = sys.argv[1], sys.argv[2]


class Registry(BaseHTTPRequestHandler):
    def _answer(self, status, body):
        encoded = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self):
        # Nothing is published here, so every read is a 404 — which is the answer that
        # tells the publication this version is absent and must be sent.
        self._answer(404, {"error": "Not found"})

    def do_PUT(self):
        length = int(self.headers.get("Content-Length", "0"))
        try:
            document = json.loads(self.rfile.read(length) or b"{}")
        except json.JSONDecodeError as error:
            self._answer(400, {"error": str(error)})
            return
        versions = document.get("versions") or {}
        with open(published, "a", encoding="utf-8") as record:
            for version, manifest in versions.items():
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


server = HTTPServer(("127.0.0.1", 0), Registry)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
PY

: > "$PUBLISHED"
python3 "$scratch/registry.py" "$PUBLISHED" "$PORT_FILE" &
REGISTRY_PID=$!

for _ in $(seq 1 100); do
  [ -s "$PORT_FILE" ] && break
  sleep 0.1
done
[ -s "$PORT_FILE" ] || fatal \
  "the stub registry never reported a port" \
  "run 'python3 -c \"import http.server\"' to check the interpreter, then rerun"
port="$(cat "$PORT_FILE")"
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
