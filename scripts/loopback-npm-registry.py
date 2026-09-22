#!/usr/bin/env python3
"""The npm registry scripts/check-npm-publish.sh stands up on loopback.

Called as `loopback-npm-registry.py <published jsonl> <port file> <mode file>`: it binds a
port the kernel picks, writes that port to the file, and answers the publication until it
is killed.

Recording, it answers a read 404 until that exact version has been published to it and 200
afterwards — which is what makes the re-run of a partly finished publication readable — and
records each publication as one JSON line naming the package, the version and the tarball
it carried.

It is a file rather than a heredoc inside that check because the bind below is a
requirement invisible in the code that meets it — a cleanup back to the stock bind passes
every Linux check and hangs the macOS release lane.
scripts/check-loopback-registries.sh runs this very launcher with `socket.getfqdn` replaced
by a function that blocks, which is what holds it.
"""

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


def shape_of(document):
    """What is wrong with this publication document, or `None` when nothing is."""
    if not isinstance(document, dict):
        return f"the publication is not a JSON object: {type(document).__name__}"
    if not isinstance(document.get("name"), str):
        return f"the publication names no package: {document.get('name')!r}"
    versions = document.get("versions") or {}
    if not isinstance(versions, dict):
        return f"'versions' is not an object: {versions!r}"
    for version, manifest in versions.items():
        if not isinstance(manifest, dict):
            return f"the manifest for version {version!r} is not an object: {manifest!r}"
    if not isinstance(document.get("_attachments") or {}, dict):
        return f"'_attachments' is not an object: {document.get('_attachments')!r}"
    return None


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
        # The body is untrusted for the same reason its length was: a publication that is
        # not shaped like one is refused as a bad request, because reaching `.items()` or
        # `.get()` on a member that is not a mapping raises out of the handler instead —
        # which answers with a closed connection, and npm reports a closed connection as
        # the registry being unreachable rather than as the malformed body it is. Every
        # member is checked before anything is written down, so a document that is wrong
        # part of the way through records none of itself.
        refusal = shape_of(document)
        if refusal is not None:
            self._answer(400, {"error": refusal})
            return
        name = document["name"]
        versions = document.get("versions") or {}
        attachments = sorted(document.get("_attachments") or {})
        with open(published, "a", encoding="utf-8") as record:
            for version, manifest in versions.items():
                holdings.setdefault(name, []).append(version)
                record.write(
                    json.dumps(
                        {
                            "path": self.path,
                            "name": name,
                            "version": version,
                            "manifest_name": manifest.get("name"),
                            "attachments": attachments,
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
    macOS runner that lookup of 127.0.0.1 outlasted the start-up window the check that
    launches this waits out — so a registry that had in fact bound was reported as one
    that never reported a port, and the whole install-path lane failed on a name nothing
    reads.
    """

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = self.server_address[1]


server = Loopback(("127.0.0.1", 0), Registry)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
