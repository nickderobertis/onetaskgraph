#!/usr/bin/env python3
"""The npm registry scripts/check-npm-publish.sh stands up on loopback.

Called as `loopback-npm-registry.py <published jsonl> <port file> <mode file>`: it binds a
port the kernel picks, writes that port to the file, and answers the publication until it
is killed. Recording, it answers a read 404 until that exact version has been published to
it and 200 afterwards — which is what makes the re-run of a partly finished publication
readable — and records each publication as one JSON line.

It is a file rather than a heredoc inside that check so that
scripts/check-loopback-registries.sh can run this very launcher against the bind `Loopback`
below states — a requirement nothing about the code that meets it makes visible.
"""

import enum
import json
import socketserver
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

published, port_file, mode_file = sys.argv[1:]

# package name -> the versions this registry has been sent. A publication asks about a
# version before sending it, and a registry that forgets what it was given cannot tell
# the "already there, leave it alone" branch from the "absent, send it" one.
holdings = {}


class Mode(enum.Enum):
    """The registries this one server stands in for, a member per case the check drives."""

    RECORD = "record"
    REFUSE_READS = "refuse-reads"
    REFUSE_WRITES = "refuse-writes"


class UnknownMode(Exception):
    """The mode file holds a word this registry stands in for no registry as."""


def mode():
    """Which registry this is standing in for right now.

    Read per request rather than once at startup: the check moves this one server between
    modes, and a mode read once would answer every later case as the first one. A word that
    is no mode raises rather than recording, because recording is what the check reads as a
    publication having landed.
    """
    try:
        with open(mode_file, encoding="utf-8") as handle:
            written = handle.read().strip()
    except FileNotFoundError:
        written = ""
    if not written:
        return Mode.RECORD
    try:
        return Mode(written)
    except ValueError as error:
        raise UnknownMode(f"{mode_file} holds {written!r}, which is no mode of this registry") from error


def shape_of(document):
    """What is wrong with this publication document, or `None` when nothing is."""
    if not isinstance(document, dict):
        return f"the publication is not a JSON object: {type(document).__name__}"
    name = document.get("name")
    if not isinstance(name, str) or not name.strip():
        return f"the publication names no package: {name!r}"
    versions = document.get("versions") or {}
    if not isinstance(versions, dict):
        return f"'versions' is not an object: {versions!r}"
    for version, manifest in versions.items():
        # Non-empty, and no stricter: this registry records what it is sent, and a grammar
        # invented here would refuse a name or a version npm itself takes.
        if not isinstance(version, str) or not version.strip():
            return f"a version of {name} is not a version: {version!r}"
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

    def _mode_or_refuse(self):
        """This registry's mode, or `None` once it has answered that it was given no mode.

        Answered rather than raised, for the reason every other refusal here is: a handler
        that raises closes the connection, and npm reads that as the registry being
        unreachable rather than as the refusal it is.
        """
        try:
            return mode()
        except UnknownMode as unknown:
            self._answer(500, {"error": str(unknown)})
            return None

    def do_GET(self):
        current = self._mode_or_refuse()
        if current is None:
            return
        if current is Mode.REFUSE_READS:
            self._answer(403, {"error": "Forbidden"})
            return
        name = urllib.parse.unquote(self.path.split("?", 1)[0].lstrip("/"))
        versions = [] if current is Mode.REFUSE_WRITES else holdings.get(name, [])
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
        current = self._mode_or_refuse()
        if current is None:
            return
        if current is Mode.REFUSE_WRITES:
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
    """Bound without the reverse DNS lookup `HTTPServer.server_bind` does on its own account.

    That lookup of 127.0.0.1 outlasted the start-up window on the macOS release runner, so a
    registry that had in fact bound was reported as one that never reported a port and the
    whole install-path lane failed on a name nothing reads.
    """

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = "127.0.0.1"
        self.server_port = self.server_address[1]


server = Loopback(("127.0.0.1", 0), Registry)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
