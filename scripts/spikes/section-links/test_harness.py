#!/usr/bin/env python3
"""Tests of the section-links harness's Linear read-back, against a loopback GraphQL server.

    python3 scripts/spikes/section-links/test_harness.py

`harness.py linear-state` runs as a subprocess, as a person runs it, with its Linear endpoint
pointed (`SECTION_LINKS_LINEAR_URL`) at a local server that answers each target's editor state.
The first version of that step called itself after every pass and spent ~460 requests before
Linear refused it; the request count below is what would have caught it.
"""

from __future__ import annotations

import base64
import http.server
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
import unittest

HARNESS = pathlib.Path(__file__).resolve().parent / "harness.py"


def yjs_like(marker: str, node: bytes) -> str:
    """Bytes shaped like Linear's Yjs state: a length-prefixed node type, then its text run."""
    raw = (b"\x07heading\x07\x00\x04What\x86" + bytes([len(node)]) + node + b"\x07\x00\x0battribution\x04null\x84"
           + bytes([len(marker)]) + marker.encode() + b"\x86")
    return base64.b64encode(raw).decode()


STATES = {
    "issue-plain": yjs_like("no anchor here", b"paragraph"),
    "issue-anchor-id": yjs_like('<a id="what"></a>', b"paragraph"),
    "issue-anchor-name": yjs_like('<a name="what"></a>', b"paragraph"),
    "doc-plain": yjs_like("no anchor here", b"paragraph"),
    "doc-anchor-id": yjs_like('<a id="what"></a>', b"html"),
    "doc-anchor-name": yjs_like('<a name="what"></a>', b"paragraph"),
}


class Linear(http.server.BaseHTTPRequestHandler):
    requests: list[dict] = []

    def do_POST(self):  # noqa: N802 - the handler's name is http.server's
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        Linear.requests.append(body)
        kind = "issue" if "issue(" in body["query"] else "document"
        field = "descriptionState" if kind == "issue" else "contentState"
        answer = json.dumps({"data": {kind: {field: STATES[body["variables"]["id"]]}}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(answer)))
        self.end_headers()
        self.wfile.write(answer)

    def log_message(self, *args):
        pass


class LinearStateTest(unittest.TestCase):
    def setUp(self):
        Linear.requests = []
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Linear)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.out = pathlib.Path(tempfile.mkdtemp())
        targets = lambda prefix: {v: {"id": f"{prefix}-{v}", "url": f"https://linear.example/{prefix}-{v}"} for v in ("plain", "anchor-id", "anchor-name")}
        (self.out / "manifest.json").write_text(json.dumps({"stamp": "test", "linear": {
            "issues": {"targets": targets("issue"), "links": []},
            "documents": {"targets": targets("doc"), "links": []},
        }}))

    def tearDown(self):
        self.server.shutdown()

    def run_step(self) -> subprocess.CompletedProcess:
        env = dict(os.environ, LINEAR_API_KEY="test-key", SECTION_LINKS_LINEAR_URL=f"http://127.0.0.1:{self.server.server_port}/graphql")
        return subprocess.run([sys.executable, str(HARNESS), "linear-state", "--out", str(self.out)],
                              env=env, capture_output=True, text=True, timeout=60)

    def test_reads_each_target_once_and_records_how_its_anchor_is_held(self):
        ran = self.run_step()
        self.assertEqual(ran.returncode, 0, ran.stderr[-2000:])
        self.assertEqual(len(Linear.requests), 6, "one read of each of the six targets, and no more")
        held = json.loads((self.out / "manifest.json").read_text())["linear"]
        self.assertEqual(held["issues"]["targets"]["anchor-id"]["anchor_held_as"], "literal text in a `paragraph` node")
        self.assertEqual(held["documents"]["targets"]["anchor-id"]["anchor_held_as"], "a `html` node")
        self.assertIsNone(held["issues"]["targets"]["plain"]["anchor_held_as"])

    def test_a_state_linear_answers_as_null_fails_the_step(self):
        STATES["doc-plain"] = None  # answered as null: the harness must fail, not record a guess
        try:
            ran = self.run_step()
        finally:
            STATES["doc-plain"] = yjs_like("no anchor here", b"paragraph")
        self.assertNotEqual(ran.returncode, 0)
        self.assertLessEqual(len(Linear.requests), 6)


if __name__ == "__main__":
    unittest.main()
