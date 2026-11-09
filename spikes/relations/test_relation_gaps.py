"""`cleanup_relation_gaps`, driven over real HTTP against a loopback stand-in for both trackers.

Offline: `python3 -B -m unittest discover -s spikes/relations -p 'test_*.py'`.
"""

# llmlint: ignore-file[new_code_lands_in_a_project] Spike branch, never merged; see harness.py.
# llmlint: ignore-file[modern_domain_modeling] Answers raw tracker JSON; see harness.py.

import json
import os
import pathlib
import re
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from unittest import mock

import harness
from sandbox import Refused

STAMP = "20261108T000000"
GH_ISSUE = "I_gh"
LI_ISSUES = ("li-near", "li-far")
LI_PROJECT = "li-project"
LI_RELATION = "li-relation"
DELETE = re.compile(
    r'(\w+):(deleteIssue|issueDelete|projectDelete)\((?:input:\{issueId|id):"([^"]+)"'
)
READ = re.compile(r'(node|issue|project)\(id:"([^"]+)"\)')
CREATE_ISSUE = "createIssue("


class Trackers:
    """What the stand-in holds and how it has been told to misbehave."""

    def __init__(self) -> None:
        """Hold the objects one gap stage would have left behind."""
        self.live = {GH_ISSUE, *LI_ISSUES}
        self.trashed: set[str] = set()
        self.projects = {LI_PROJECT}
        self.refuse_delete: set[str] = set()
        self.accept_but_keep: set[str] = set()
        self.unreadable: set[str] = set()
        self.refuse_create = False
        self.sent: list[str] = []

    def mutate(self, doc: str) -> dict[str, Any]:
        """Answer one mutation document."""
        if CREATE_ISSUE in doc and self.refuse_create:
            return {"data": {"a0": None}, "errors": [{"message": "creation refused"}]}
        data: dict[str, Any] = {}
        errors: list[dict[str, str]] = []
        for alias, root, key in DELETE.findall(doc):
            if key in self.refuse_delete:
                data[alias] = None
                errors.append({"message": f"{root} refused"})
                continue
            if root == "projectDelete":
                self.trashed.add(key)
            elif key not in self.accept_but_keep:
                self.live.discard(key)
            data[alias] = {"clientMutationId": None, "success": True}
        return {"data": data, "errors": errors} if errors else {"data": data}

    def read(self, doc: str) -> dict[str, Any]:
        """Answer one read: bootstrap, the rate gate, or the verification of one object."""
        data: dict[str, Any] = {
            "spikeRateLimit": {"cost": 1, "remaining": 5000, "limit": 5000, "resetAt": "x"}
        }
        if "viewer{" in doc:
            data |= {
                "viewer": {"id": "OWNER", "login": "o"},
                "r1": {"id": "R1"},
                "r2": {"id": "R2"},
            }
        if "teams{" in doc:
            data |= {"teams": {"nodes": [{"id": "T2", "key": "TES2"}, {"id": "T1", "key": "TES"}]}}
        found = READ.search(doc)
        if not found:
            return {"data": data}
        field, key = found.groups()
        if key in self.unreadable:
            return {"data": None, "errors": [{"message": "Something went wrong"}]}
        if field == "project":
            return {"data": {**data, "project": {"trashed": key in self.trashed}}}
        if key in self.live:
            return {"data": {**data, field: {"id": key}}}
        message = (
            f"Could not resolve to a node with the global id of '{key}'."
            if field == "node"
            else "Entity not found: Issue"
        )
        return {"data": {**data, field: None}, "errors": [{"message": message}]}


def serve(trackers: Trackers) -> ThreadingHTTPServer:
    """Start the stand-in on an ephemeral loopback port."""

    class Handler(BaseHTTPRequestHandler):
        """One GraphQL endpoint standing in for both trackers."""

        def do_POST(self) -> None:
            """Answer one GraphQL document."""
            length = int(self.headers["content-length"])
            doc = json.loads(self.rfile.read(length))["query"]
            trackers.sent.append(doc)
            answer = trackers.mutate(doc) if doc.startswith("mutation") else trackers.read(doc)
            body = json.dumps(answer).encode()
            self.send_response(200)
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args: object) -> None:
            """Keep the test output to the test runner's."""

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class CleanupRelationGaps(unittest.TestCase):
    """The stage's own cleanup over a real `Run`, its sandbox and real HTTP."""

    def setUp(self) -> None:
        """Build a `Run` whose only doubled boundary is the two trackers' addresses."""
        self.trackers = Trackers()
        server = serve(self.trackers)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        url = f"http://127.0.0.1:{server.server_address[1]}/"
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        home = pathlib.Path(scratch.name)
        (home / ".config" / "onetaskgraph").mkdir(parents=True)
        (home / ".config" / "onetaskgraph" / "secrets.env").write_text("LINEAR_API_KEY=fake\n")
        for patcher in (
            mock.patch.dict(
                os.environ,
                {"HOME": scratch.name, "GH_PROJECTS_TOKEN": "fake", "USERPROFILE": scratch.name},
            ),
            mock.patch.object(harness, "EVIDENCE", home / "evidence"),
            mock.patch.object(harness, "GITHUB_URL", url),
            mock.patch.object(harness, "LINEAR_URL", url),
            mock.patch.object(harness, "ISSUE_PACE_SECONDS", 0),
        ):
            patcher.start()
            self.addCleanup(patcher.stop)
        self.run_ = harness.Run(STAMP)
        self.run_.bootstrap()
        self.run_.stage = "relation_gaps"
        self.run_.record_created(GH_ISSUE, "issue", "github", repo="r1", number=1)
        for key in LI_ISSUES:
            self.run_.record_created(key, "issue", "linear", team="TES2", identifier=key)
        self.run_.record_created(LI_PROJECT, "project", "linear", team="TES2")
        self.run_.record_created(LI_RELATION, "relation", "linear")

    def ended(self, key: str) -> str | None:
        """How the ledger says this object ended, or None while it is open."""
        return self.run_.ledger[key].get("ended")

    def evidence(self) -> dict[str, Any]:
        """The stage's written cleanup answers."""
        return json.loads((self.run_.dir / "relation_gaps_cleanup.json").read_text())

    def test_success_deletes_every_object_and_reads_it_back(self) -> None:
        """Every object goes, issues before their project, and the answer is clean."""
        self.assertTrue(harness.cleanup_relation_gaps(self.run_))
        self.assertEqual(self.trackers.live, set())
        self.assertEqual(self.trackers.trashed, {LI_PROJECT})
        for key in (GH_ISSUE, *LI_ISSUES, LI_PROJECT, LI_RELATION):
            self.assertEqual(self.ended(key), "deleted", key)
        deletes = [d for d in self.trackers.sent if d.startswith("mutation")]
        self.assertIn("projectDelete", deletes[-1])
        evidence = self.evidence()
        self.assertEqual(evidence["errors"], [])
        self.assertEqual(len(evidence["verified"]), 4)
        self.assertTrue(all(row["gone"] for row in evidence["verified"]))

    def test_only_this_stages_open_entries_are_touched(self) -> None:
        """Another stage's item and an entry already ended are never sent a delete."""
        self.run_.stage = "cleanup"
        self.run_.record_created("I_other", "issue", "github", repo="r1", number=2)
        self.run_.stage = "relation_gaps"
        self.run_.record_created("I_ended", "issue", "github", repo="r1", number=3)
        self.run_.record_ended("I_ended", "closed-not-deleted", "ruled")
        self.assertTrue(harness.cleanup_relation_gaps(self.run_))
        sent = "\n".join(self.trackers.sent)
        self.assertNotIn("I_other", sent)
        self.assertNotIn("I_ended", sent)
        self.assertFalse((self.run_.dir / "cleanup.json").exists())

    def test_a_refused_github_delete_fails_and_stays_open(self) -> None:
        """A delete GitHub refuses leaves the entry open and the cleanup failed."""
        self.trackers.refuse_delete.add(GH_ISSUE)
        self.assertFalse(harness.cleanup_relation_gaps(self.run_))
        self.assertIsNone(self.ended(GH_ISSUE))
        self.assertIn("deleteIssue refused", self.evidence()["errors"])

    def test_a_refused_linear_issue_delete_keeps_its_relation_open(self) -> None:
        """A relation goes with its issues, so a surviving issue means it was not removed."""
        self.trackers.refuse_delete.add(LI_ISSUES[0])
        self.assertFalse(harness.cleanup_relation_gaps(self.run_))
        self.assertIsNone(self.ended(LI_ISSUES[0]))
        self.assertIsNone(self.ended(LI_RELATION))

    def test_a_refused_project_delete_fails_and_stays_open(self) -> None:
        """A project Linear will not trash leaves the entry open and the cleanup failed."""
        self.trackers.refuse_delete.add(LI_PROJECT)
        self.assertFalse(harness.cleanup_relation_gaps(self.run_))
        self.assertIsNone(self.ended(LI_PROJECT))

    def test_a_delete_accepted_but_still_readable_fails(self) -> None:
        """The `#6094` shape: success is answered and the issue is still there."""
        self.trackers.accept_but_keep.add(GH_ISSUE)
        self.assertFalse(harness.cleanup_relation_gaps(self.run_))
        rows = {row["id"]: row["gone"] for row in self.evidence()["verified"]}
        self.assertFalse(rows[GH_ISSUE])

    def test_an_unreadable_verification_is_never_read_as_gone(self) -> None:
        """An error that is not "not found" says nothing about the object, on any kind."""
        for key in (GH_ISSUE, LI_ISSUES[0], LI_PROJECT):
            with self.subTest(key=key):
                self.trackers.unreadable = {key}
                self.trackers.live |= {GH_ISSUE, *LI_ISSUES}
                self.trackers.trashed.clear()
                self.reopen()
                self.assertFalse(harness.cleanup_relation_gaps(self.run_))
                rows = {row["id"]: row["gone"] for row in self.evidence()["verified"]}
                self.assertFalse(rows[key])

    def reopen(self) -> None:
        """Put every entry back to open, for a second pass in one test."""
        for value in self.run_.ledger.values():
            value.pop("ended", None)

    def test_the_sandbox_refuses_a_delete_this_run_did_not_create(self) -> None:
        """A ledger entry the sandbox cannot vouch for is refused before anything is sent."""
        self.run_.box.created.discard(GH_ISSUE)
        before = len(self.trackers.sent)
        with self.assertRaises(Refused):
            harness.cleanup_relation_gaps(self.run_)
        self.assertEqual(len(self.trackers.sent), before)
        self.assertIn(GH_ISSUE, self.trackers.live)

    def test_a_stage_that_fails_part_way_still_cleans_up(self) -> None:
        """The measurement's failure propagates, after what it left has been deleted."""
        self.trackers.refuse_create = True
        with self.assertRaises(RuntimeError):
            harness.stage_relation_gaps(self.run_)
        self.assertEqual(self.trackers.live, set())
        self.assertEqual(self.ended(LI_PROJECT), "deleted")


if __name__ == "__main__":
    unittest.main()
