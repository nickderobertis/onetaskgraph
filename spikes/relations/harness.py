#!/usr/bin/env python3
"""One harness for the relations spike: real GitHub and real Linear, inside a stated sandbox.

    python3 spikes/relations/harness.py <stage>... [--stamp S] [--keep]

Stages are listed in `STAGES` at the end of this file; `all` runs every one and cleans up however
it ends. Evidence goes to `spikes/relations/evidence/<stamp>/`: `requests.jsonl` holds, per
request, its operation names and costs, and no credential, title or body; `<stage>.json` holds
the stage's derived answers, keyed by the question they answer, and the only bodies it holds are
the synthetic ones the report quotes. Every write passes `sandbox.Sandbox` before it is sent.
"""

# llmlint: ignore-file[async_typed_clients_at_boundaries] A single-threaded measurement script run once per spike on a branch kept for the record and never landing; a blocking client keeps every request's position in the requests.jsonl sequence the report cites.
# llmlint: ignore-file[changed_behavior_has_e2e] This file is the measurement: its proof is the live run whose evidence sits beside it, against the real trackers the task names, and sandbox.py carries the offline test. There is no production behaviour here for an e2e suite to prove.
# llmlint: ignore-file[modern_domain_modeling] The evidence is whatever each tracker answers on the day, recorded verbatim for one report; typed models of those answers would be a second schema of two external APIs for a script that runs once and is kept only for the record.
# llmlint: ignore-file[new_code_lands_in_a_project] Spike branch kept for the record, not merged; adding an Nx project here would widen every selection this repository's graph promises, for code that never lands on main.
# llmlint: ignore-file[boundary_inputs_validated] The stamp, stages and credentials are validated at the entry; the rest of the inputs are this script's own evidence files or the tracker's answer, which the stage reads back and writes down as it stands, because that answer is what is being measured.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] SLOT_OPEN restates the metadata comment the two plugins write, on purpose: the spike observes what is stored on the tracker, and if a plugin moved the delimiter the run would measure that as a change rather than follow it.

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from sandbox import (  # noqa: E402 -- sandbox.py sits beside this script, and sys.path is set above
    BOARD_TITLE_PREFIX,
    Enum,
    Mut,
    Refused,
    Sandbox,
    document,
    refuse_protected_board,
)

ROOT = pathlib.Path(__file__).resolve().parents[2]
EVIDENCE = pathlib.Path(__file__).parent / "evidence"
BINARY = ROOT / "target" / "debug" / "onetaskgraph"
OWNER = "nickderobertis"
REPO1 = "onetaskgraph-live-scratch"
REPO2 = "onetaskgraph-live-scratch-2"
GITHUB_URL = "https://api.github.com/graphql"
LINEAR_URL = "https://api.linear.app/graphql"
MAX_LEVELS = 10
MAX_CHILDREN = 105
STOP_GITHUB_POINTS = 1800  # stop short of the 2,000 that needs the manager, leaving cleanup room
STOP_LINEAR_REQUESTS = 900
PAUSE_FRACTION = 0.40
ISSUE_PACE_SECONDS = 3.0  # the manager's pacing after the first run tripped the secondary limit


class QuotaStop(Exception):
    """The spike reached the spend it may not pass without asking the manager."""


class RateRefused(Exception):
    """A tracker refused for a rate limit; never retried, because retrying extends it."""


def load_linear_key() -> str:
    """LINEAR_API_KEY alone from the secrets file; no other line is read into the process."""
    path = pathlib.Path.home() / ".config" / "onetaskgraph" / "secrets.env"
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("LINEAR_API_KEY="):
            key = line.split("=", 1)[1].strip().strip("'\"")
            if not key:
                raise SystemExit(f"LINEAR_API_KEY in {path} is empty; next: set it")
            return key
    raise SystemExit(f"LINEAR_API_KEY is not in {path}; next: add it, or run on the spike host")


def scrub(text: str) -> str:
    """Evidence may name a team key, never the workspace a Linear URL spells."""
    return re.sub(r"linear\.app/[^/\s\)\"]+/", "linear.app/<workspace>/", text)


def digest(text: str) -> str:
    return "sha256:" + hashlib.sha256(text.encode()).hexdigest()[:16]


class Run:
    """One stamp's state: evidence files, the ledger of what it created, and its spend."""

    def __init__(self, stamp: str) -> None:
        self.stamp = stamp
        self.dir = EVIDENCE / stamp
        self.dir.mkdir(parents=True, exist_ok=True)
        self.requests_path = self.dir / "requests.jsonl"
        self.ledger_path = self.dir / "ledger.jsonl"
        token = os.environ.get("GH_PROJECTS_TOKEN", "")
        if not token:
            raise SystemExit(
                "GH_PROJECTS_TOKEN is not set; next: run from the dispatch environment"
            )
        self.token = token
        self.linear_key = load_linear_key()
        self.seq = 0
        self.stage = "-"
        self.github_points = 0
        self.linear_requests = 0
        self.linear_complexity = 0
        self.last_used: int | None = None
        self.ledger: dict[str, dict[str, Any]] = {}
        self.cleaning = False
        self.github_off = False
        scratch = os.environ.get("ONEPIPELINE_NODE_SCRATCH_DIR") or tempfile.mkdtemp()
        self.scratch = pathlib.Path(scratch)
        self._replay()
        self.box = Sandbox(stamp, "", set(), set(), {k for k, v in self.ledger.items() if v})
        self.github_ids: dict[str, str] = {}
        self.linear_teams: dict[str, str] = {}

    def _replay(self) -> None:
        if self.requests_path.exists():
            for line in self.requests_path.read_text(encoding="utf-8").splitlines():
                rec = json.loads(line)
                self.seq = max(self.seq, rec["seq"])
                match rec:
                    case {"tracker": "github"}:
                        self.github_points += rec.get("cost") or 0
                    case {"tracker": "linear"}:
                        self.linear_requests += 1
                        self.linear_complexity += rec.get("complexity") or 0
        if self.ledger_path.exists():
            for line in self.ledger_path.read_text(encoding="utf-8").splitlines():
                ev = json.loads(line)
                if ev["event"] == "created":
                    self.ledger[ev["id"]] = ev
                else:
                    self.ledger[ev["id"]].update(ended=ev["event"], how=ev.get("how"))

    def record_created(self, id_: str, kind: str, tracker: str, **where: Any) -> None:
        ev = {"event": "created", "id": id_, "kind": kind, "tracker": tracker, **where}
        ev["stage"] = self.stage
        self.ledger[id_] = ev
        self.box.created.add(id_)
        with self.ledger_path.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(ev) + "\n")

    def record_ended(self, id_: str, event: str, how: str) -> None:
        self.ledger[id_].update(ended=event, how=how)
        with self.ledger_path.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps({"event": event, "id": id_, "how": how}) + "\n")

    def _post(self, url: str, auth: str, doc: str) -> tuple[Any, dict[str, str], float, int]:
        body = json.dumps({"query": doc}).encode()
        request = urllib.request.Request(
            url,
            body,
            {"authorization": auth, "content-type": "application/json", "user-agent": "spike"},
        )
        started = time.monotonic()
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                raw = response.read()
                headers = {k.lower(): v for k, v in response.headers.items()}
                status = response.status
        except urllib.error.HTTPError as err:
            raw = err.read()
            headers = {k.lower(): v for k, v in err.headers.items()}
            status = err.code
        elapsed = time.monotonic() - started
        try:
            return json.loads(raw), headers, elapsed, status
        except json.JSONDecodeError:
            return (
                {"errors": [{"message": raw[:300].decode(errors="replace")}]},
                headers,
                elapsed,
                status,
            )

    def github_rate_used(self) -> int:
        """x-ratelimit-used as of now, from GitHub's free REST read, bracketing a binary run."""
        request = urllib.request.Request(
            "https://api.github.com/rate_limit",
            headers={"authorization": f"bearer {self.token}", "user-agent": "spike"},
        )
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.load(response)["resources"]["graphql"]["used"]

    def _write_record(self, rec: dict[str, Any]) -> int:
        self.seq += 1
        rec = {"seq": self.seq, "stage": self.stage, **rec}
        with self.requests_path.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(rec) + "\n")
        return self.seq

    def github(self, doc: str, q: str, *, roots: list[str], aliases: int) -> tuple[dict, list, int]:
        if self.github_off:
            raise Refused("this run was started with --no-github")
        if not self.cleaning and self.github_points >= STOP_GITHUB_POINTS:
            raise QuotaStop(f"GitHub spend {self.github_points}")
        is_mutation = doc.startswith("mutation")
        if is_mutation:
            sent = doc
        else:
            sent = doc[:-1] + " spikeRateLimit:rateLimit{cost remaining limit resetAt}}"
        payload, headers, elapsed, status = self._post(GITHUB_URL, f"bearer {self.token}", sent)
        used = int(headers.get("x-ratelimit-used", "0") or 0)
        data = payload.get("data") or {}
        errors = payload.get("errors") or []
        limit = data.pop("spikeRateLimit", None) if isinstance(data, dict) else None
        if limit:
            cost = limit["cost"]
        else:
            delta = used - self.last_used if self.last_used is not None else 1
            cost = 1  # the quiet-window delta of one document, aliased or not; see `used_delta`
        self.last_used = used
        self.github_points += cost
        rec = {
            "tracker": "github",
            "q": q,
            "kind": "mutation" if is_mutation else "query",
            "roots": roots,
            "aliases": aliases,
            "cost": cost,
            "cost_source": "rateLimit.cost" if limit else "1 per mutation document",
            "used_delta": None if limit else delta,
            "http": status,
            "ms": round(elapsed * 1000),
            "ratelimit_used": used,
            "ratelimit_remaining": headers.get("x-ratelimit-remaining"),
            "errors": [e.get("message") for e in errors][:6]
            or ([str(payload.get("message"))[:300]] if payload.get("message") else []),
            "retry_after": headers.get("retry-after"),
        }
        seq = self._write_record(rec)
        text = json.dumps(errors).lower()
        if status in (403, 429) or "rate limit" in text or "abuse" in text:
            raise RateRefused(f"request {seq}: {rec['errors'] or status}")
        return data, errors, seq

    def linear(self, doc: str, q: str, *, roots: list[str], aliases: int) -> tuple[dict, list, int]:
        if not self.cleaning and self.linear_requests >= STOP_LINEAR_REQUESTS:
            raise QuotaStop(f"Linear spend {self.linear_requests}")
        payload, headers, elapsed, status = self._post(LINEAR_URL, self.linear_key, doc)
        self.linear_requests += 1
        data = payload.get("data") or {}
        errors = payload.get("errors") or []
        complexity = int(float(headers.get("x-complexity", "0") or 0))
        self.linear_complexity += complexity
        rec = {
            "tracker": "linear",
            "q": q,
            "kind": "mutation" if doc.startswith("mutation") else "query",
            "roots": roots,
            "aliases": aliases,
            "http": status,
            "complexity": complexity,
            "ms": round(elapsed * 1000),
            "requests_remaining": headers.get("x-ratelimit-requests-remaining"),
            "complexity_remaining": headers.get("x-ratelimit-complexity-remaining"),
            "errors": [
                (e.get("message"), (e.get("extensions") or {}).get("userPresentableMessage"))
                for e in errors
            ][:6],
        }
        seq = self._write_record(rec)
        if status == 429 or "ratelimit" in json.dumps(errors).lower():
            raise RateRefused(f"request {seq}: {rec['errors'] or status}")
        return data, errors, seq

    def gh_read(self, doc: str, q: str) -> tuple[dict, list, int]:
        if doc.lstrip().startswith("mutation"):
            raise Refused("gh_read only reads")
        return self.github(doc, q, roots=argument_fields(doc), aliases=doc.count("node("))

    def li_read(self, doc: str, q: str) -> tuple[dict, list, int]:
        if doc.lstrip().startswith("mutation"):
            raise Refused("li_read only reads")
        return self.linear(doc, q, roots=argument_fields(doc), aliases=doc.count("issue("))

    def gh_mut(self, muts: list[Mut], q: str) -> tuple[dict, list, int]:
        self.box.check_github(muts)
        return self.github(
            document(muts), q, roots=sorted({m.root for m in muts}), aliases=len(muts)
        )

    def li_mut(self, muts: list[Mut], q: str) -> tuple[dict, list, int]:
        self.box.check_linear(muts)
        return self.linear(
            document(muts), q, roots=sorted({m.root for m in muts}), aliases=len(muts)
        )

    def rate_gate(self) -> dict[str, Any]:
        """Read both allowances; wait while either is under 40% of its limit."""
        while True:
            gh = {"remaining": 1, "limit": 1, "resetAt": "2000-01-01T00:00:00Z"}
            if not self.github_off:
                data, _, _ = self.gh_read("{rateLimit{remaining limit resetAt}}", "rate-gate")
                gh = data["rateLimit"]
            data, _, _ = self.li_read(
                "{rateLimitStatus{limits{type remainingAmount allowedAmount reset}}}", "rate-gate"
            )
            limits = data["rateLimitStatus"]["limits"]
            low = gh["remaining"] < PAUSE_FRACTION * gh["limit"]
            waits = []
            if low:
                reset = dt.datetime.fromisoformat(gh["resetAt"].replace("Z", "+00:00"))
                waits.append((reset - dt.datetime.now(dt.UTC)).total_seconds())
            for limit in limits:
                if limit["remainingAmount"] < PAUSE_FRACTION * limit["allowedAmount"]:
                    waits.append(limit["reset"] / 1000 - time.time())
            if not waits:
                return {"github": gh, "linear": limits}
            wait = min(max(waits) + 5, 3700)
            print(f"[{self.stamp}] pausing {wait:.0f}s: allowance under 40%", flush=True)
            time.sleep(wait)

    def bootstrap(self) -> None:
        if not self.github_off:
            self._bootstrap_github()
        data, _, _ = self.li_read("{teams{nodes{id key}}}", "bootstrap")
        keys = {t["key"]: t["id"] for t in data["teams"]["nodes"]}
        self.linear_teams = {k: keys[k] for k in ("TES2", "TES")}
        self.box.teams = set(self.linear_teams.values())

    def _bootstrap_github(self) -> None:
        data, _, _ = self.gh_read(
            f'query{{viewer{{id login}} r1:repository(owner:"{OWNER}",name:"{REPO1}"){{id}}'
            f' r2:repository(owner:"{OWNER}",name:"{REPO2}"){{id}}}}',
            "bootstrap",
        )
        self.box.owner_id = data["viewer"]["id"]
        self.github_ids = {"r1": data["r1"]["id"], "r2": data["r2"]["id"]}
        self.box.repositories = set(self.github_ids.values())

    def title(self, slug: str) -> str:
        return f"{self.stamp} {slug}"

    def gh_issues(self, repo: str, slugs: list[str], q: str, body: str = "spike") -> dict:
        repo_id = self.github_ids[repo]
        out: dict[str, dict] = {}
        for start in range(0, len(slugs), 8):
            chunk = slugs[start : start + 8]
            time.sleep(ISSUE_PACE_SECONDS * len(chunk))
            muts = [
                Mut(
                    f"a{i}",
                    "createIssue",
                    {"input": {"repositoryId": repo_id, "title": self.title(s), "body": body}},
                    "issue{id number url}",
                )
                for i, s in enumerate(chunk)
            ]
            data, errors, seq = self.gh_mut(muts, q)
            for i, slug in enumerate(chunk):
                issue = (data.get(f"a{i}") or {}).get("issue")
                if not issue:
                    raise RuntimeError(f"createIssue {slug} failed at request {seq}: {errors[:2]}")
                self.record_created(
                    issue["id"], "issue", "github", repo=repo, number=issue["number"]
                )
                out[slug] = {**issue, "repo": repo}
        return out

    def gh_board(self, slug: str, q: str) -> dict:
        title = f"{BOARD_TITLE_PREFIX}{self.stamp}-{slug}"
        data, errors, seq = self.gh_mut(
            [
                Mut(
                    "a",
                    "createProjectV2",
                    {"input": {"ownerId": self.box.owner_id, "title": title}},
                    "projectV2{id number url}",
                )
            ],
            q,
        )
        board = data["a"]["projectV2"]
        refuse_protected_board(board["number"])
        self.record_created(board["id"], "board", "github", number=board["number"], title=title)
        return board

    def gh_add_item(self, board_id: str, issue_id: str, q: str) -> str:
        data, errors, seq = self.gh_mut(
            [
                Mut(
                    "a",
                    "addProjectV2ItemById",
                    {"input": {"projectId": board_id, "contentId": issue_id}},
                    "item{id}",
                )
            ],
            q,
        )
        if errors:
            raise RuntimeError(f"addProjectV2ItemById failed at request {seq}: {errors[:2]}")
        return data["a"]["item"]["id"]

    def li_issues(self, team: str, slugs: list[str], q: str, **extra: Any) -> dict:
        out: dict[str, dict] = {}
        team_id = self.linear_teams[team]
        for start in range(0, len(slugs), 20):
            chunk = slugs[start : start + 20]
            muts = [
                Mut(
                    f"a{i}",
                    "issueCreate",
                    {
                        "input": {
                            "teamId": team_id,
                            "title": self.title(s),
                            "description": "spike",
                            **extra,
                        }
                    },
                    "success issue{id identifier url}",
                )
                for i, s in enumerate(chunk)
            ]
            data, errors, seq = self.li_mut(muts, q)
            for i, slug in enumerate(chunk):
                payload = data.get(f"a{i}") or {}
                issue = payload.get("issue")
                if not issue:
                    raise RuntimeError(f"issueCreate {slug} failed at request {seq}: {errors[:2]}")
                self.record_created(
                    issue["id"], "issue", "linear", team=team, identifier=issue["identifier"]
                )
                out[slug] = issue
        return out

    def li_project(self, team: str, slug: str, q: str) -> str:
        data, errors, seq = self.li_mut(
            [
                Mut(
                    "a",
                    "projectCreate",
                    {"input": {"teamIds": [self.linear_teams[team]], "name": self.title(slug)}},
                    "success project{id}",
                )
            ],
            q,
        )
        project = data["a"]["project"]
        self.record_created(project["id"], "project", "linear", team=team)
        return project["id"]

    def write_stage(self, name: str, answers: dict[str, Any]) -> None:
        path = self.dir / f"{name}.json"
        text = scrub(json.dumps(answers, indent=1, ensure_ascii=False)) + "\n"
        path.write_text(text, encoding="utf-8")
        print(f"[{self.stamp}] stage {name} written to {path.name}", flush=True)


def argument_fields(doc: str) -> list[str]:
    """Up to six distinct argument-bearing field names of a read, for the request log."""
    return sorted(set(re.findall(r"(?:^|[{ ])(?:\w+:)?(\w+)\(", doc)))[:6]


def errs(errors: list) -> list[str]:
    return [str(e.get("message", e)) for e in errors]


class Cli:
    """The real onetaskgraph binary, on sources this run created and nothing else."""

    def __init__(self, run: Run, sources: dict[str, Any]) -> None:
        for spec in sources.values():
            if spec.get("plugin") == "github-projects":
                refuse_protected_board(spec["config"]["project_number"])
        self.run_ = run
        self.dir = run.scratch / f"cli-{run.stamp}"
        (self.dir / "xdg").mkdir(parents=True, exist_ok=True)
        config = {"default_sources": sorted(sources), "sources": sources}
        (self.dir / "onetaskgraph.yaml").write_text(json.dumps(config, indent=1), encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if not k.startswith("ONETASKGRAPH_")}
        env.update(
            HOME=str(self.dir),
            XDG_CONFIG_HOME=str(self.dir / "xdg"),
            XDG_CACHE_HOME=str(self.dir / "cache"),
            GH_PROJECTS_TOKEN=run.token,
            LINEAR_API_KEY=run.linear_key,
            ONETASKGRAPH_INTERACTIVE="false",
        )
        self.env = env

    def _used(self) -> int:
        return 0 if self.run_.github_off else self.run_.github_rate_used()

    def __call__(self, *args: str, q: str = "cli") -> dict[str, Any]:
        full = [str(BINARY), "--output", "json", *args]
        before = self._used()
        proc = subprocess.run(
            full, cwd=self.dir, env=self.env, capture_output=True, text=True, check=False
        )
        out: Any = proc.stdout
        try:
            out = json.loads(proc.stdout)
        except json.JSONDecodeError:
            pass
        self.run_._write_record(
            {
                "tracker": "cli",
                "q": q,
                "kind": "cli",
                "roots": list(args[:3]),
                "aliases": 0,
                "exit": proc.returncode,
                "github_points_delta": self._used() - before,
            }
        )
        return {"exit": proc.returncode, "out": out, "err": proc.stderr.strip()[:900]}


GH_RELS = (
    "number state stateReason parent{number} subIssuesSummary{total}"
    " subIssues(first:100){totalCount nodes{number}}"
    " relatesTo(first:50){totalCount nodes{number}}"
    " blockedBy(first:50){totalCount nodes{number}}"
    " blocking(first:50){totalCount nodes{number}}"
    " duplicateOf{number}"
    " trackedIssues(first:10){totalCount} trackedInIssues(first:10){totalCount}"
)
LI_RELS = (
    "identifier parent{identifier} children(first:120){nodes{identifier}}"
    " relations(first:50){nodes{id type relatedIssue{identifier}}}"
    " inverseRelations(first:50){nodes{id type issue{identifier}}}"
)


def gh_node(run: Run, issue: dict, q: str) -> dict:
    data, errors, seq = run.gh_read(
        'query{{node(id:"{}"){{... on Issue{{{}}}}}}}'.format(issue["id"], GH_RELS), q
    )
    return {"seq": seq, **(data.get("node") or {}), "errors": errs(errors)}


def li_node(run: Run, issue: dict, q: str, fields: str = LI_RELS) -> dict:
    data, errors, seq = run.li_read('query{{issue(id:"{}"){{{}}}}}'.format(issue["id"], fields), q)
    return {"seq": seq, **(data.get("issue") or {}), "errors": errs(errors)}


def gh_rel(run: Run, root: str, q: str, **ids: str) -> dict:
    data, errors, seq = run.gh_mut([Mut("a", root, {"input": ids}, "clientMutationId")], q)
    return {"seq": seq, "ok": not errors, "errors": errs(errors)}


def li_upd(run: Run, id_: str, q: str, **fields: Any) -> dict:
    data, errors, seq = run.li_mut(
        [Mut("a", "issueUpdate", {"id": id_, "input": fields}, "success issue{identifier}")], q
    )
    return {"seq": seq, "ok": not errors, "errors": errs(errors)}


def li_rel(run: Run, kind: str, a: dict, b: dict, q: str) -> dict:
    data, errors, seq = run.li_mut(
        [
            Mut(
                "a",
                "issueRelationCreate",
                {"input": {"issueId": a["id"], "relatedIssueId": b["id"], "type": Enum(kind)}},
                "success issueRelation{id type}",
            )
        ],
        q,
    )
    rel = ((data.get("a") or {}).get("issueRelation")) or {}
    if rel.get("id"):
        run.record_created(rel["id"], "relation", "linear")
    return {"seq": seq, "ok": bool(rel.get("id")), "id": rel.get("id"), "errors": errs(errors)}


def stage_introspect(run: Run) -> None:
    query = "{__schema{types{name kind fields{name} inputFields{name} enumValues{name}}}}"
    gh, _, gh_seq = run.gh_read(query, "introspect.github")
    li, _, li_seq = run.li_read(query, "introspect.linear")

    def names(items: list | None) -> list[str]:
        return [item["name"] for item in items or []]

    gt = {t["name"]: t for t in gh["__schema"]["types"]}
    lt = {t["name"]: t for t in li["__schema"]["types"]}
    pattern = re.compile(r"(?i)subissue|blockedby|relatesto|duplicate|trackedissue")
    keep = {
        "github_request": gh_seq,
        "linear_request": li_seq,
        "github_issue_relation_fields": [
            f for f in names(gt["Issue"]["fields"]) if pattern.search(f) or f == "parent"
        ],
        "github_relation_mutations": [
            f for f in names(gt["Mutation"]["fields"]) if pattern.search(f)
        ],
        "github_issue_close_reasons": names(gt["IssueClosedStateReason"]["enumValues"]),
        "github_add_relates_to_input": names(gt["AddRelatesToInput"]["inputFields"]),
        "github_add_blocked_by_input": names(gt["AddBlockedByInput"]["inputFields"]),
        "github_add_sub_issue_input": names(gt["AddSubIssueInput"]["inputFields"]),
        "github_field_types_hierarchy": [
            e
            for e in names(gt["ProjectV2FieldType"]["enumValues"])
            if e in {"PARENT_ISSUE", "SUB_ISSUES_PROGRESS", "TRACKS", "TRACKED_BY"}
        ],
        "linear_issue_relation_types": names(lt["IssueRelationType"]["enumValues"]),
        "linear_issue_relation_create_input": names(lt["IssueRelationCreateInput"]["inputFields"]),
        "linear_relation_mutations": [
            f
            for f in names(lt["Mutation"]["fields"])
            if re.match(r"issueRelation|projectRelation|issueBatch", f)
        ],
        "linear_issue_relation_fields": [
            f
            for f in names(lt["Issue"]["fields"])
            if f in {"parent", "children", "relations", "inverseRelations"}
        ],
    }
    run.write_stage("introspect", keep)


def stage_preflight(run: Run) -> None:
    """One create and one delete on each tracker, so an undeletable write is found first."""
    made = run.gh_issues("r1", ["preflight"], "preflight.create")
    data, errors, seq1 = run.gh_mut(
        [
            Mut(
                "a",
                "deleteIssue",
                {"input": {"issueId": made["preflight"]["id"]}},
                "clientMutationId",
            )
        ],
        "preflight.delete",
    )
    if errors:
        raise RuntimeError(f"GitHub deleteIssue refused at {seq1}: {errs(errors)}")
    run.record_ended(made["preflight"]["id"], "deleted", f"deleteIssue request {seq1}")
    lin = run.li_issues("TES2", ["preflight"], "preflight.create")
    data, errors, seq2 = run.li_mut(
        [
            Mut(
                "a",
                "issueDelete",
                {"id": lin["preflight"]["id"], "permanentlyDelete": True},
                "success",
            )
        ],
        "preflight.delete",
    )
    if errors:
        raise RuntimeError(
            f"Linear issueDelete(permanentlyDelete) refused at {seq2}: {errs(errors)}"
        )
    run.record_ended(lin["preflight"]["id"], "deleted", f"issueDelete permanently, request {seq2}")
    run.write_stage("preflight", {"github_delete": seq1, "linear_delete": seq2})


def drive_github_limit(
    run: Run, issues: list[dict], q: str, edge: Callable[[dict, dict], dict]
) -> dict[str, Any]:
    """Add edges one at a time until the tracker refuses, or the list ends."""
    for size, (parent, child) in enumerate(issues, 1):
        result = edge(parent, child)
        if not result["ok"]:
            return {"refused_at": size, "request": result["seq"], "error": result["errors"]}
    return {"accepted_up_to": len(issues)}


def drive_children(run: Run, parent: dict, ordered: list[dict]) -> dict[str, Any]:
    accepted = 0
    refusal: dict[str, Any] | None = None
    batch_log = []
    for start in range(0, MAX_CHILDREN, 25):
        chunk = ordered[start : start + 25]
        muts = [
            Mut(
                f"a{i}",
                "addSubIssue",
                {"input": {"issueId": parent["id"], "subIssueId": kid["id"]}},
                "clientMutationId",
            )
            for i, kid in enumerate(chunk)
        ]
        data, errors, seq = run.gh_mut(muts, "q1.gh.children.batch")
        done = sum(1 for i in range(len(chunk)) if data.get(f"a{i}"))
        accepted += done
        batch_log.append(
            {"seq": seq, "sent": len(chunk), "accepted": done, "errors": errs(errors)[:2]}
        )
        if done < len(chunk):
            refusal = {"accepted_total": accepted, "seq": seq, "errors": errs(errors)[:2]}
            break
    return {
        "probe_children": MAX_CHILDREN,
        "accepted": accepted,
        "refusal": refusal,
        "batches": batch_log,
        "read": gh_node(run, parent, "q1.gh.children.read"),
    }


def prior_limits(run: Run) -> dict[str, Any] | None:
    """Depth and children limits an earlier session of this stamp already drove to refusal.

    The limits are final once refused; re-driving them would spend another 200 content writes
    against the secondary limiter that stopped the first session.
    """
    rows = [json.loads(line) for line in run.requests_path.read_text(encoding="utf-8").splitlines()]
    depth = [r for r in rows if r["q"] == "q1.gh.depth"]
    batches = [r for r in rows if r["q"] == "q1.gh.children.batch"]
    if not depth or not depth[-1]["errors"] or len(batches) < 4:
        return None
    match = re.search(r"to parent #(\d+)", batches[-1]["errors"][0])
    if match is None:
        return None
    return {
        "depth": {
            "probe_levels": MAX_LEVELS,
            "refused_at": len(depth),
            "request": depth[-1]["seq"],
            "error": depth[-1]["errors"][:1],
            "deepest_chain_levels_accepted": len(depth),
            "refused_level": len(depth) + 1,
        },
        "children": {
            "probe_children": MAX_CHILDREN,
            "accepted": sum(r["aliases"] for r in batches[:-1]),
            "refusal": {
                "accepted_total": sum(r["aliases"] for r in batches[:-1]),
                "seq": batches[-1]["seq"],
                "errors": batches[-1]["errors"][:1],
            },
            "batches": [
                {
                    "seq": r["seq"],
                    "sent": r["aliases"],
                    "cost_delta": r.get("used_delta"),
                    "errors": r["errors"][:1],
                }
                for r in batches
            ],
        },
        "children_parent_number": int(match.group(1)),
    }


def stage_gh_parents(run: Run) -> None:
    answers: dict[str, Any] = {}
    made = run.gh_issues("r1", ["root", "mid", "leaf", "other"], "q1.gh.crud.create")
    root, mid, leaf, other = (made[k] for k in ("root", "mid", "leaf", "other"))
    answers["add"] = gh_rel(
        run, "addSubIssue", "q1.gh.add", issueId=root["id"], subIssueId=mid["id"]
    )
    answers["chain"] = gh_rel(
        run, "addSubIssue", "q1.gh.add", issueId=mid["id"], subIssueId=leaf["id"]
    )
    answers["read_root"] = gh_node(run, root, "q1.gh.read")
    answers["read_leaf"] = gh_node(run, leaf, "q1.gh.read")
    answers["move_without_replace"] = gh_rel(
        run, "addSubIssue", "q1.gh.move", issueId=other["id"], subIssueId=mid["id"]
    )
    answers["move_with_replace"] = gh_rel(
        run,
        "addSubIssue",
        "q1.gh.move",
        issueId=other["id"],
        subIssueId=mid["id"],
        replaceParent=True,
    )
    answers["read_moved_mid"] = gh_node(run, mid, "q1.gh.read")
    answers["cycle"] = gh_rel(
        run, "addSubIssue", "q1.gh.cycle", issueId=leaf["id"], subIssueId=other["id"]
    )
    answers["remove"] = gh_rel(
        run, "removeSubIssue", "q1.gh.remove", issueId=mid["id"], subIssueId=leaf["id"]
    )
    answers["read_removed_leaf"] = gh_node(run, leaf, "q1.gh.read")

    prior = prior_limits(run)
    if prior is None:
        chain = run.gh_issues("r1", [f"lvl{i}" for i in range(MAX_LEVELS)], "q1.gh.depth.create")
        pairs = [(chain[f"lvl{i - 1}"], chain[f"lvl{i}"]) for i in range(1, MAX_LEVELS)]
        outcome = drive_github_limit(
            run,
            pairs,
            "q1.gh.depth",
            lambda p, c: gh_rel(
                run, "addSubIssue", "q1.gh.depth", issueId=p["id"], subIssueId=c["id"]
            ),
        )
        answers["depth"] = {"probe_levels": MAX_LEVELS, **outcome}
        parent = run.gh_issues("r1", ["kids-parent"], "q1.gh.children.create")["kids-parent"]
        kids = run.gh_issues(
            "r1", [f"kid{i:03d}" for i in range(MAX_CHILDREN)], "q1.gh.children.create"
        )
        answers["children"] = drive_children(
            run, parent, [kids[f"kid{i:03d}"] for i in range(MAX_CHILDREN)]
        )
        answers["children_parent_number"] = parent["number"]
    else:
        answers["depth"] = prior["depth"]
        children = prior["children"]
        data, _, seq = run.gh_read(
            f'query{{repository(owner:"{OWNER}",name:"{REPO1}"){{issue(number:{prior["children_parent_number"]}){{subIssuesSummary{{total}} subIssues(first:100){{totalCount}}}}}}}}',
            "q1.gh.children.read",
        )
        children["read"] = {"seq": seq, **(data.get("repository") or {}).get("issue", {})}
        answers["children"] = children
        answers["children_parent_number"] = prior["children_parent_number"]
        answers["limits_measured_in_the_earlier_session_of_this_stamp"] = True

    across = run.gh_issues("r2", ["xrepo-child"], "q1.gh.cross.create")["xrepo-child"]
    host = run.gh_issues("r1", ["xrepo-parent"], "q1.gh.cross.create")["xrepo-parent"]
    answers["cross_repo_child_in_r2"] = gh_rel(
        run, "addSubIssue", "q1.gh.cross", issueId=host["id"], subIssueId=across["id"]
    )
    answers["cross_repo_read"] = gh_node(run, host, "q1.gh.cross.read")
    answers["cross_repo_child_read"] = gh_node(run, across, "q1.gh.cross.read")
    answers["objects"] = {
        "root": root["number"],
        "children_parent": answers["children_parent_number"],
        "cross_parent": host["number"],
        "cross_child": across["number"],
    }
    run.write_stage("gh_parents", answers)


BOARD_READ = (
    "fields(first:30){nodes{... on ProjectV2FieldCommon{name dataType}}}"
    " workflows(first:20){nodes{name enabled number}}"
    " items(first:100){totalCount nodes{id content{... on Issue{number parent{number}"
    " subIssuesSummary{total} title}}}}"
)


def board_view(run: Run, board: dict, q: str) -> dict:
    data, errors, seq = run.gh_read(
        'query{{node(id:"{}"){{... on ProjectV2{{{}}}}}}}'.format(board["id"], BOARD_READ), q
    )
    node = data.get("node") or {}
    items = (node.get("items") or {}).get("nodes") or []
    return {
        "seq": seq,
        "errors": errs(errors),
        "fields": [f.get("dataType") for f in (node.get("fields") or {}).get("nodes", []) if f],
        "workflows": (node.get("workflows") or {}).get("nodes"),
        "item_numbers": sorted((i.get("content") or {}).get("number") or 0 for i in items),
        "items": [
            {
                "number": (i.get("content") or {}).get("number"),
                "parent": ((i.get("content") or {}).get("parent") or {}).get("number"),
                "children": ((i.get("content") or {}).get("subIssuesSummary") or {}).get("total"),
            }
            for i in items
        ],
    }


def gh_source(number: int) -> dict[str, Any]:
    return {
        "plugin": "github-projects",
        "config": {
            "owner": OWNER,
            "project_number": number,
            "repository": f"{OWNER}/{REPO1}",
            "token_env": "GH_PROJECTS_TOKEN",
            "status_mapping": {"todo": "Todo", "in-progress": "In Progress", "done": "Done"},
        },
    }


def summary(result: dict[str, Any]) -> dict[str, Any]:
    """What a binary run answered, down to ids, numbers and kinds."""
    out = result["out"]
    items: Any = None
    if isinstance(out, dict):
        items = out.get("items") or out.get("results") or out.get("result") or out
    return {
        "exit": result["exit"],
        "err": result["err"] or None,
        "ids": sorted(
            (i.get("id") or i.get("qualified_id") or "") for i in items if isinstance(i, dict)
        )
        if isinstance(items, list)
        else None,
        "item": ({k: items.get(k) for k in ("id", "parent", "project", "version") if k in items})
        if isinstance(items, dict)
        else None,
    }


def list_ids(result: dict[str, Any]) -> list[str]:
    out = result["out"]
    items = out.get("items", []) if isinstance(out, dict) else []
    return [str(item.get("id")) for item in items]


def numbers_for(issues: dict[str, dict], ids: list[str]) -> dict[str, list[str]]:
    """Which named issues a binary listing held, by slug, from each issue's GitHub number."""
    by_id = {issue["id"]: slug for slug, issue in issues.items()}
    known: dict[str, list[str]] = {"listed": [], "unknown": []}
    for value in ids:
        native = value.split(":", 1)[-1]
        known["listed" if native in by_id else "unknown"].append(by_id.get(native, value))
    return known


def stage_gh_boards(run: Run) -> None:
    answers: dict[str, Any] = {}
    board_x = run.gh_board("x", "q1.gh.board.create")
    board_y = run.gh_board("y", "q1.gh.board.create")
    answers["boards"] = {"x": board_x["number"], "y": board_y["number"]}
    answers["board_x_initial"] = board_view(run, board_x, "q1.gh.board.read")
    made = run.gh_issues("r1", ["jp", "jc", "jc2"], "q1.gh.join.create")
    jp, jc, jc2 = made["jp"], made["jc"], made["jc2"]
    run.gh_add_item(board_x["id"], jp["id"], "q1.gh.join.add")
    answers["join_sub_issue"] = gh_rel(
        run, "addSubIssue", "q1.gh.join.edge", issueId=jp["id"], subIssueId=jc["id"]
    )
    time.sleep(20)
    answers["board_after_sub_issue_added_to_a_board_parent"] = board_view(
        run, board_x, "q1.gh.join.read"
    )
    answers["join_grandchild_of_a_board_parent"] = gh_rel(
        run, "addSubIssue", "q1.gh.join.edge", issueId=jc["id"], subIssueId=jc2["id"]
    )
    time.sleep(20)
    answers["board_after_grandchild"] = board_view(run, board_x, "q1.gh.join.read")
    answers["numbers"] = {k: v["number"] for k, v in made.items()}

    lm_root = run.scratch / f"lm-{run.stamp}"
    (lm_root / "projects").mkdir(parents=True, exist_ok=True)
    (lm_root / "tasks").mkdir(parents=True, exist_ok=True)
    cli = Cli(
        run,
        {
            "xd": gh_source(board_x["number"]),
            "yd": gh_source(board_y["number"]),
            "lm": {"plugin": "local-md", "config": {"root": str(lm_root)}},
        },
    )
    one = run.gh_issues("r1", ["sym", "cl1", "cl2", "cause"], "q1.coexist.one.create")
    sym, cl1, cl2, cause = (one[k] for k in ("sym", "cl1", "cl2", "cause"))
    for issue in one.values():
        run.gh_add_item(board_x["id"], issue["id"], "q1.coexist.one.add")
    answers["one_before"] = {
        "projects": summary(cli("project", "list", "--source", "xd", q="coexist.one.list")),
        "tasks": summary(cli("task", "list", "--source", "xd", q="coexist.one.list")),
    }
    for child in (cl1, cl2):
        gh_rel(
            run, "addSubIssue", "q1.coexist.one.parent", issueId=sym["id"], subIssueId=child["id"]
        )
    gh_rel(run, "addSubIssue", "q1.coexist.one.parent", issueId=cause["id"], subIssueId=sym["id"])
    projects = cli("project", "list", "--source", "xd", q="coexist.one.list")
    tasks = cli("task", "list", "--source", "xd", q="coexist.one.list")
    answers["one_after_parent_write"] = {
        "default_projects": numbers_for(one, list_ids(projects)),
        "default_tasks": numbers_for(one, list_ids(tasks)),
        "exit": [projects["exit"], tasks["exit"]],
        "parent_reading": {
            "cause": gh_node(run, cause, "q1.coexist.one.read"),
            "sym": gh_node(run, sym, "q1.coexist.one.read"),
        },
    }
    answers["one_task_show_of_a_parent_ticket"] = summary(
        cli("task", "show", f"xd:{sym['id']}", q="coexist.one.show")
    )
    answers["one_task_show_of_top_ticket"] = summary(
        cli("task", "show", f"xd:{cause['id']}", q="coexist.one.show")
    )
    answers["one_project_show_of_top_ticket"] = summary(
        cli("project", "show", f"xd:{cause['id']}", q="coexist.one.show")
    )
    answers["one_project_status_write_on_a_parent_ticket"] = summary(
        cli("project", "status", "set", f"xd:{cause['id']}", "in-progress", q="coexist.one.write")
    )
    answers["one_remove_only_child_of_cause"] = gh_rel(
        run, "removeSubIssue", "q1.coexist.one.parent", issueId=cause["id"], subIssueId=sym["id"]
    )
    projects = cli("project", "list", "--source", "xd", q="coexist.one.list")
    answers["one_after_child_removed"] = numbers_for(one, list_ids(projects))
    (lm_root / "projects" / "p.md").write_text(
        f"---\ntitle: {run.title('plugin-project')}\nstatus: todo\n---\nproject\n", encoding="utf-8"
    )
    (lm_root / "tasks" / "t.md").write_text(
        f"---\ntitle: {run.title('plugin-task')}\nstatus: todo\nproject: p\n---\ntask\n",
        encoding="utf-8",
    )
    copied = cli("project", "copy", "lm:p", "--to", "xd", q="coexist.one.write")
    answers["one_plugin_project_copy"] = summary(copied)
    native_project = ""
    native_task = ""
    outcomes = copied["out"] if isinstance(copied["out"], dict) else {}
    for bucket in ("projects", "tasks", "results", "outcomes", "items"):
        for row in outcomes.get(bucket) or []:
            target = row.get("to") or row.get("destination") or row.get("result") or row
            ident = str(
                (target.get("id") if isinstance(target, dict) else target) or row.get("id") or ""
            )
            if not ident:
                continue
            if bucket == "projects" or row.get("kind") == "project":
                native_project = ident.split(":", 1)[-1]
            elif not native_task:
                native_task = ident.split(":", 1)[-1]
    answers["one_plugin_copy_report_keys"] = sorted(outcomes)
    if not native_project:
        for line in str(copied["out"]).splitlines():
            found = re.search(r"\bxd:(I_\w+)", line)
            if found and not native_project:
                native_project = found.group(1)
    for native in (native_project, native_task):
        if native:
            run.record_created(native, "issue", "github", repo="r1", via="binary")
    if native_project:
        answers["one_parent_reading_of_plugin_project"] = gh_node(
            run, {"id": native_project}, "q1.coexist.one.read"
        )
        answers["one_parent_write_onto_plugin_project_with_replace"] = gh_rel(
            run,
            "addSubIssue",
            "q1.coexist.one.parent",
            issueId=native_project,
            subIssueId=cl1["id"],
            replaceParent=True,
        )
        answers["one_after_parent_write_onto_plugin_project"] = {
            "tasks": numbers_for(
                one, list_ids(cli("task", "list", "--source", "xd", q="coexist.one.list"))
            ),
            "plugin_project_read": summary(
                cli("project", "show", f"xd:{native_project}", q="coexist.one.show")
            ),
        }
    two = run.gh_issues("r1", ["py", "ty", "tkt", "tkt2", "shared"], "q1.coexist.two.create")
    py, ty, tkt, tkt2, shared = (two[k] for k in ("py", "ty", "tkt", "tkt2", "shared"))
    for issue in (py, ty):
        run.gh_add_item(board_y["id"], issue["id"], "q1.coexist.two.add")
    gh_rel(run, "addSubIssue", "q1.coexist.two.project", issueId=py["id"], subIssueId=ty["id"])
    for issue in (tkt, tkt2, shared):
        run.gh_add_item(board_x["id"], issue["id"], "q1.coexist.two.add")
    gh_rel(run, "addSubIssue", "q1.coexist.two.parent", issueId=tkt["id"], subIssueId=tkt2["id"])
    gh_rel(run, "addSubIssue", "q1.coexist.two.parent", issueId=shared["id"], subIssueId=tkt["id"])
    answers["two_default_on_y"] = {
        "projects": numbers_for(
            two, list_ids(cli("project", "list", "--source", "yd", q="coexist.two.list"))
        ),
        "tasks": numbers_for(
            two, list_ids(cli("task", "list", "--source", "yd", q="coexist.two.list"))
        ),
    }
    run.gh_add_item(board_y["id"], shared["id"], "q1.coexist.two.shared")
    answers["two_default_on_y_immediately_after_parent_ticket_joins_y"] = {
        "projects": numbers_for(
            two, list_ids(cli("project", "list", "--source", "yd", q="coexist.two.list"))
        ),
        "tasks": numbers_for(
            two, list_ids(cli("task", "list", "--source", "yd", q="coexist.two.list"))
        ),
    }
    time.sleep(30)
    answers["two_default_on_y_30s_after_parent_ticket_joined_y"] = {
        "projects": numbers_for(
            two, list_ids(cli("project", "list", "--source", "yd", q="coexist.two.list"))
        ),
        "tasks": numbers_for(
            two, list_ids(cli("task", "list", "--source", "yd", q="coexist.two.list"))
        ),
    }
    answers["two_move_project_task_under_a_ticket_without_replace"] = gh_rel(
        run, "addSubIssue", "q1.coexist.two.parent", issueId=tkt["id"], subIssueId=ty["id"]
    )
    answers["two_move_project_task_under_a_ticket_with_replace"] = gh_rel(
        run,
        "addSubIssue",
        "q1.coexist.two.parent",
        issueId=tkt["id"],
        subIssueId=ty["id"],
        replaceParent=True,
    )
    answers["two_default_on_y_after_move"] = {
        "projects": numbers_for(
            two, list_ids(cli("project", "list", "--source", "yd", q="coexist.two.list"))
        ),
        "tasks": numbers_for(
            two, list_ids(cli("task", "list", "--source", "yd", q="coexist.two.list"))
        ),
    }
    run.write_stage("gh_boards", answers)


def gh_read_body(run: Run, issue: dict, q: str) -> tuple[str, int]:
    data, _, seq = run.gh_read(
        'query{{node(id:"{}"){{... on Issue{{body}}}}}}'.format(issue["id"]), q
    )
    return (data.get("node") or {}).get("body") or "", seq


def gh_set_body(run: Run, issue: dict, body: str, q: str) -> dict:
    data, errors, seq = run.gh_mut(
        [Mut("a", "updateIssue", {"input": {"id": issue["id"], "body": body}}, "issue{id}")], q
    )
    return {"seq": seq, "ok": not errors, "errors": errs(errors)}


def split_links(body: str) -> dict[str, Any]:
    """Where the metadata slot and the `## Links` section are, in a stored body."""
    slot = re.search(SLOT_OPEN_PATTERN, body)
    section = re.search(r"^#{1,6} Links\s*$", body, re.M)
    return {
        "section_present": bool(section),
        "metadata_slot_present": bool(slot),
        "chars": len(body),
        "digest": digest(body),
    }


SLOT_OPEN = "<!-- onetaskgraph.metadata"
SLOT_OPEN_PATTERN = re.escape(SLOT_OPEN)
LINKS_SECTION = "## Links\n\n- **caused-by** → [{title}]({url}) `{qualified}`\n\n"


def fallback_body(visible: str, links: str, stored: str) -> str:
    """The visible text and a Links section, followed by whatever slot the store holds."""
    slot = re.search(SLOT_OPEN_PATTERN + ".*", stored, re.S)
    return visible + "\n\n" + links + (slot.group(0) if slot else "")


def cli_task(cli: Cli, qualified: str, q: str) -> dict:
    result = cli("task", "show", qualified, q=q)
    out = result["out"]
    items = out.get("items", []) if isinstance(out, dict) else []
    task = (items[0].get("item") or {}) if items else {}
    return {
        "exit": result["exit"],
        "err": result["err"] or None,
        "metadata_keys": sorted(task.get("metadata") or {}),
        "links_metadata": (task.get("metadata") or {}).get("spike.links"),
        "content_has_links_section": "## Links" in (task.get("content") or ""),
        "content_digest": digest(task.get("content") or ""),
    }


def stage_gh_links(run: Run) -> None:
    answers: dict[str, Any] = {}
    made = run.gh_issues("r1", ["a", "b", "c", "d", "e", "f"], "q2.gh.links.create")
    cross = run.gh_issues("r2", ["x"], "q2.gh.links.create")["x"]
    a, b, c, d, e, f = (made[k] for k in "abcdef")
    relates = {
        "add": gh_rel(
            run, "addRelatesTo", "q2.gh.relates", issueId=a["id"], relatedIssueId=b["id"]
        ),
        "read_a": gh_node(run, a, "q2.gh.relates.read"),
        "read_b": gh_node(run, b, "q2.gh.relates.read"),
        "duplicate_add": gh_rel(
            run, "addRelatesTo", "q2.gh.relates", issueId=a["id"], relatedIssueId=b["id"]
        ),
        "remove_from_other_end": gh_rel(
            run, "removeRelatesTo", "q2.gh.relates", issueId=b["id"], relatedIssueId=a["id"]
        ),
        "read_a_after": gh_node(run, a, "q2.gh.relates.read"),
        "cross_repo": gh_rel(
            run, "addRelatesTo", "q2.gh.relates", issueId=a["id"], relatedIssueId=cross["id"]
        ),
        "cross_read": gh_node(run, cross, "q2.gh.relates.read"),
        "self": gh_rel(
            run, "addRelatesTo", "q2.gh.relates", issueId=a["id"], relatedIssueId=a["id"]
        ),
    }
    answers["relates_to"] = relates
    blocked = {
        "add": gh_rel(
            run, "addBlockedBy", "q2.gh.blocked", issueId=c["id"], blockingIssueId=d["id"]
        ),
        "read_blocked": gh_node(run, c, "q2.gh.blocked.read"),
        "read_blocker": gh_node(run, d, "q2.gh.blocked.read"),
        "cycle": gh_rel(
            run, "addBlockedBy", "q2.gh.blocked", issueId=d["id"], blockingIssueId=c["id"]
        ),
        "cross_repo": gh_rel(
            run, "addBlockedBy", "q2.gh.blocked", issueId=c["id"], blockingIssueId=cross["id"]
        ),
        "read_cross": gh_node(run, cross, "q2.gh.blocked.read"),
        "remove": gh_rel(
            run, "removeBlockedBy", "q2.gh.blocked", issueId=c["id"], blockingIssueId=d["id"]
        ),
    }
    answers["blocked_by"] = blocked
    # duplicate: closing with duplicateIssueId is the API's only way to mark one.
    close, errors, seq = run.gh_mut(
        [
            Mut(
                "a",
                "closeIssue",
                {
                    "input": {
                        "issueId": e["id"],
                        "stateReason": Enum("DUPLICATE"),
                        "duplicateIssueId": f["id"],
                    }
                },
                "issue{state stateReason}",
            )
        ],
        "q2.gh.duplicate",
    )
    duplicate = {
        "close": {"seq": seq, "ok": not errors, "errors": errs(errors), "payload": close.get("a")},
        "read_duplicate": gh_node(run, e, "q2.gh.duplicate.read"),
        "read_canonical": gh_node(run, f, "q2.gh.duplicate.read"),
        "unmark": gh_rel(
            run,
            "unmarkIssueAsDuplicate",
            "q2.gh.duplicate",
            duplicateId=e["id"],
            canonicalId=f["id"],
        ),
        "read_after_unmark": gh_node(run, e, "q2.gh.duplicate.read"),
    }
    data, errors, seq = run.gh_mut(
        [
            Mut(
                "a",
                "closeIssue",
                {
                    "input": {
                        "issueId": e["id"],
                        "stateReason": Enum("DUPLICATE"),
                        "duplicateIssueId": cross["id"],
                    }
                },
                "issue{state stateReason}",
            )
        ],
        "q2.gh.duplicate.cross",
    )
    duplicate["cross_repo_close"] = {"seq": seq, "ok": not errors, "errors": errs(errors)}
    duplicate["cross_repo_read"] = gh_node(run, e, "q2.gh.duplicate.read")
    answers["duplicate"] = duplicate
    # tracked: read-only fields driven by the deprecated task list; prove the API offers no write.
    answers["tracked"] = {
        "set_body_with_task_list": gh_set_body(run, a, f"- [ ] #{b['number']}\n", "q2.gh.tracked"),
        "read": gh_node(run, a, "q2.gh.tracked.read"),
        "note": "no trackedIssues/tracksIssues mutation exists in the introspected schema",
    }
    gh_set_body(run, a, "spike", "q2.gh.tracked.restore")

    board = run.gh_board("l", "q2.gh.fallback.board")
    answers["fallback_board"] = board["number"]
    pair = run.gh_issues(
        "r1", ["sym", "root"], "q2.gh.fallback.create", body="Visible symptom text."
    )
    sym, root = pair["sym"], pair["root"]
    for issue in (sym, root):
        run.gh_add_item(board["id"], issue["id"], "q2.gh.fallback.add")
    cli = Cli(run, {"ld": gh_source(board["number"])})
    edges = [
        (sym, [{"type": "caused-by", "direction": "out", "other": f"ld:{root['id']}"}]),
        (root, [{"type": "caused-by", "direction": "in", "other": f"ld:{sym['id']}"}]),
    ]
    writes = []
    for issue, value in edges:
        writes.append(
            summary(
                cli(
                    "task",
                    "metadata",
                    "set",
                    f"ld:{issue['id']}",
                    "spike.links",
                    json.dumps(value),
                    q="fallback.set",
                )
            )
        )
    answers["fallback_metadata_writes"] = writes
    stored, read_seq = gh_read_body(run, sym, "q2.gh.fallback.read")
    answers["fallback_after_metadata"] = {"seq": read_seq, **split_links(stored)}
    section = LINKS_SECTION.format(title="root", url=root["url"], qualified=f"ld:{root['id']}")
    body = fallback_body("Visible symptom text.", section, stored)
    answers["fallback_with_section"] = gh_set_body(run, sym, body, "q2.gh.fallback.section")
    answers["fallback_read_with_section"] = cli_task(cli, f"ld:{sym['id']}", "fallback.show")
    answers["fallback_page"] = {"url": sym["url"]}
    stored, _ = gh_read_body(run, sym, "q2.gh.fallback.read")
    edited = stored.replace("Visible symptom text.", "Visible symptom text, edited by a person.")
    answers["edit_keeps_section"] = {
        "write": gh_set_body(run, sym, edited, "q2.gh.fallback.edit1"),
        "read": cli_task(cli, f"ld:{sym['id']}", "fallback.show"),
        "stored": split_links(gh_read_body(run, sym, "q2.gh.fallback.read")[0]),
    }
    # A person deletes only the section; the comment slot remains.
    stored, _ = gh_read_body(run, sym, "q2.gh.fallback.read")
    without_section = re.sub(r"## Links\s.*?(?=" + SLOT_OPEN_PATTERN + ")", "", stored, flags=re.S)
    answers["edit_deletes_section_keeps_slot"] = {
        "write": gh_set_body(run, sym, without_section, "q2.gh.fallback.edit2"),
        "read": cli_task(cli, f"ld:{sym['id']}", "fallback.show"),
        "stored": split_links(gh_read_body(run, sym, "q2.gh.fallback.read")[0]),
    }
    # A person rewrites the body whole; the comment slot goes with it.
    answers["edit_rewrites_whole_body"] = {
        "write": gh_set_body(
            run, sym, "A person replaced the whole description.", "q2.gh.fallback.edit3"
        ),
        "read": cli_task(cli, f"ld:{sym['id']}", "fallback.show"),
        "stored": split_links(gh_read_body(run, sym, "q2.gh.fallback.read")[0]),
        "other_end_read": cli_task(cli, f"ld:{root['id']}", "fallback.show"),
    }
    answers["objects"] = {k: v["number"] for k, v in made.items()} | {"cross": cross["number"]}
    run.write_stage("gh_links", answers)


def stage_gh_duplicate_cross(run: Run) -> None:
    """A duplicate naming an issue of another repository: closing the open issue names it."""
    near = run.gh_issues("r1", ["dup-near"], "q2.gh.duplicate.cross.create")["dup-near"]
    far = run.gh_issues("r2", ["dup-far"], "q2.gh.duplicate.cross.create")["dup-far"]
    data, errors, seq = run.gh_mut(
        [
            Mut(
                "a",
                "closeIssue",
                {
                    "input": {
                        "issueId": near["id"],
                        "stateReason": Enum("DUPLICATE"),
                        "duplicateIssueId": far["id"],
                    }
                },
                "issue{state stateReason}",
            )
        ],
        "q2.gh.duplicate.cross",
    )
    stage = json.loads((run.dir / "gh_links.json").read_text())
    stage["duplicate"]["cross_repo_close"] = {
        "seq": seq,
        "ok": not errors,
        "errors": errs(errors),
        "payload": data.get("a"),
    }
    stage["duplicate"]["cross_repo_read"] = gh_node(run, near, "q2.gh.duplicate.read")
    stage["duplicate"]["cross_repo_note"] = (
        "first attempt in gh_links closed an already-closed issue, a no-op"
    )
    run.write_stage("gh_links", stage)


def stage_gh_plugin_project(run: Run) -> None:
    """What the project meaning writes through today's plugin, read as a parent reading."""
    boards = json.loads((run.dir / "gh_boards.json").read_text())
    number = boards["boards"]["x"]
    board_id = next(
        k for k, v in run.ledger.items() if v["kind"] == "board" and v.get("number") == number
    )
    data, errors, seq = run.gh_mut(
        [
            Mut(
                "a",
                "createProjectV2Field",
                {
                    "input": {
                        "projectId": board_id,
                        "dataType": Enum("TEXT"),
                        "name": "onetaskgraph.origin",
                    }
                },
                "projectV2Field{... on ProjectV2Field{id}}",
            )
        ],
        "q1.coexist.one.origin-field",
    )
    lm_root = run.scratch / f"lm-{run.stamp}"
    cli = Cli(
        run,
        {"xd": gh_source(number), "lm": {"plugin": "local-md", "config": {"root": str(lm_root)}}},
    )
    copied = cli("project", "copy", "lm:p", "--to", "xd", q="coexist.one.write")
    answers: dict[str, Any] = {
        "field": {"seq": seq, "errors": errs(errors)},
        "copy_exit": copied["exit"],
        "copy_err": copied["err"],
    }
    out = copied["out"] if isinstance(copied["out"], dict) else {}
    native_project = ""
    natives: list[str] = []
    for row in out.get("items") or out.get("outcomes") or out.get("results") or []:
        result = row.get("result") or {}
        ident = str(result.get("id") or row.get("id") or "")
        if ident:
            natives.append(ident.split(":", 1)[-1])
    answers["copy_report_keys"] = sorted(out)
    answers["copy_outcomes"] = [
        {k: row.get(k) for k in ("kind", "action", "status", "reason") if k in row}
        for row in (out.get("items") or out.get("outcomes") or out.get("results") or [])
    ]
    if not natives:
        natives = re.findall(r"I_kw\w+", json.dumps(out))
    natives = list(dict.fromkeys(natives))
    for native in natives:
        run.record_created(native, "issue", "github", repo="r1", via="binary")
    if natives:
        reads = [gh_node(run, {"id": native}, "q1.coexist.one.read") for native in natives]
        answers["reads"] = [
            {
                "number": r.get("number"),
                "parent": r.get("parent"),
                "children": r.get("subIssuesSummary"),
            }
            for r in reads
        ]
        parent_candidates = [r for r in reads if (r.get("subIssuesSummary") or {}).get("total")]
        if parent_candidates:
            native_project = next(
                n for n, r in zip(natives, reads, strict=True) if r is parent_candidates[0]
            )
    answers["project_list"] = summary(
        cli("project", "list", "--source", "xd", q="coexist.one.list")
    )
    fresh = run.gh_issues("r1", ["into-project"], "q1.coexist.one.create")["into-project"]
    run.gh_add_item(board_id, fresh["id"], "q1.coexist.one.add")
    answers["fresh_listed_before"] = fresh["number"]
    if native_project:
        answers["parent_write_onto_plugin_project"] = gh_rel(
            run,
            "addSubIssue",
            "q1.coexist.one.parent",
            issueId=native_project,
            subIssueId=fresh["id"],
        )
        answers["project_show_after_parent_write"] = summary(
            cli("project", "show", f"xd:{native_project}", q="coexist.one.show")
        )
        answers["tasks_after_parent_write"] = numbers_for(
            {"fresh": fresh},
            list_ids(cli("task", "list", "--source", "xd", q="coexist.one.list")),
        )
    run.write_stage("gh_plugin_project", answers)


def stage_gh_landed60(run: Run) -> None:
    """How much of the 60-alias mutation GitHub answered 504 to had landed, read after it."""
    budget = [
        v for v in run.ledger.values() if v.get("stage") == "budget_github" and v["kind"] == "issue"
    ]
    ids = [v["id"] for v in budget][:20]
    doc = (
        "query{"
        + " ".join(
            f'i{n}:node(id:"{i}"){{... on Issue{{relatesTo(first:100){{totalCount}}}}}}'
            for n, i in enumerate(ids)
        )
        + "}"
    )
    data, errors, seq = run.gh_read(doc, "aliases.read-all")
    total = sum(
        ((data.get(f"i{n}") or {}).get("relatesTo") or {}).get("totalCount", 0)
        for n in range(len(ids))
    )
    run.write_stage(
        "gh_landed60",
        {
            "seq": seq,
            "tickets": len(ids),
            "sum_of_both_ends": total,
            "edges_visible": total / 2,
            "expected_if_every_alias_landed": 30 + 60,
            "expected_if_none_landed": 30,
        },
    )


def stage_gh_fallback_page(run: Run) -> None:
    """How a `## Links` section and the hidden slot render on a public issue page."""
    root = run.gh_issues("r1", ["links-target"], "q2.gh.fallback.page.create")["links-target"]
    body = (
        "Visible symptom text.\n\n"
        + LINKS_SECTION.format(title="target", url=root["url"], qualified=f"ld:{root['id']}")
        + SLOT_OPEN
        + ' {"spike.links":[{"type":"caused-by"}]} -->'
    )
    issue = run.gh_issues("r1", ["links-page"], "q2.gh.fallback.page.create", body=body)[
        "links-page"
    ]
    script = run.scratch / f"capture-{run.stamp}.cjs"
    script.write_text(CAPTURE_JS, encoding="utf-8")
    shots = run.dir / "pages"
    shots.mkdir(exist_ok=True)
    proc = subprocess.run(
        ["node", str(script), json.dumps([["fallback-rendered", issue["url"]]]), str(shots)],
        capture_output=True,
        text=True,
        check=False,
        timeout=300,
    )
    script.unlink(missing_ok=True)
    run.write_stage(
        "gh_fallback_page",
        {
            "issue": issue["number"],
            "capture_exit": proc.returncode,
            "stderr": proc.stderr.strip()[:300],
        },
    )


def stage_li_duplicate_cross(run: Run) -> None:
    """Whether `duplicate` crosses teams, the one relation `li_links` did not cross."""
    near = run.li_issues("TES2", ["dup-near"], "q2.li.duplicate.cross.create")["dup-near"]
    far = run.li_issues("TES", ["dup-far"], "q2.li.duplicate.cross.create")["dup-far"]
    result = li_rel(run, "duplicate", near, far, "q2.li.duplicate.cross-team")
    read = li_node(run, near, "q2.li.duplicate.cross-team.read")
    state = li_node(
        run, near, "q2.li.duplicate.cross-team.state", "identifier state{name type} canceledAt"
    )
    run.write_stage("li_duplicate_cross", {"add": result, "read": read, "state": state})


def li_body(run: Run, issue: dict, q: str) -> tuple[str, int]:
    data, _, seq = run.li_read(
        'query{{issue(id:"{}"){{description updatedAt}}}}'.format(issue["id"]), q
    )
    return (data.get("issue") or {}).get("description") or "", seq


def stage_li_parents(run: Run) -> None:
    answers: dict[str, Any] = {}
    made = run.li_issues("TES2", ["root", "mid", "leaf", "other"], "q1.li.crud.create")
    root, mid, leaf, other = (made[k] for k in ("root", "mid", "leaf", "other"))
    answers["add"] = li_upd(run, mid["id"], "q1.li.add", parentId=root["id"])
    answers["chain"] = li_upd(run, leaf["id"], "q1.li.add", parentId=mid["id"])
    answers["read_root"] = li_node(run, root, "q1.li.read")
    answers["read_leaf"] = li_node(run, leaf, "q1.li.read")
    answers["move"] = li_upd(run, mid["id"], "q1.li.move", parentId=other["id"])
    answers["read_moved"] = li_node(run, mid, "q1.li.read")
    answers["cycle"] = li_upd(run, other["id"], "q1.li.cycle", parentId=leaf["id"])
    answers["remove"] = li_upd(run, leaf["id"], "q1.li.remove", parentId=None)
    answers["read_removed"] = li_node(run, leaf, "q1.li.read")

    top = run.li_issues("TES2", ["lvl0"], "q1.li.depth.create")["lvl0"]
    previous = top
    depth: dict[str, Any] = {"probe_levels": MAX_LEVELS, "accepted_levels": 1}
    for level in range(1, MAX_LEVELS):
        data, errors, seq = run.li_mut(
            [
                Mut(
                    "a",
                    "issueCreate",
                    {
                        "input": {
                            "teamId": run.linear_teams["TES2"],
                            "title": run.title(f"lvl{level}"),
                            "parentId": previous["id"],
                        }
                    },
                    "success issue{id identifier}",
                )
            ],
            "q1.li.depth",
        )
        issue = (data.get("a") or {}).get("issue")
        if issue is None:
            depth["refused_level"] = level + 1
            depth["request"] = seq
            depth["error"] = errs(errors)
            break
        run.record_created(
            issue["id"], "issue", "linear", team="TES2", identifier=issue["identifier"]
        )
        previous = issue
        depth["accepted_levels"] = level + 1
    answers["depth"] = depth

    parent = run.li_issues("TES2", ["kids-parent"], "q1.li.children.create")["kids-parent"]
    accepted = 0
    refusal = None
    batch_log = []
    for start in range(0, MAX_CHILDREN, 20):
        count = min(20, MAX_CHILDREN - start)
        slugs = [f"kid{start + i:03d}" for i in range(count)]
        muts = [
            Mut(
                f"a{i}",
                "issueCreate",
                {
                    "input": {
                        "teamId": run.linear_teams["TES2"],
                        "title": run.title(s),
                        "parentId": parent["id"],
                    }
                },
                "success issue{id identifier}",
            )
            for i, s in enumerate(slugs)
        ]
        data, errors, seq = run.li_mut(muts, "q1.li.children.batch")
        done = 0
        for i in range(count):
            issue = (data.get(f"a{i}") or {}).get("issue")
            if issue:
                run.record_created(
                    issue["id"], "issue", "linear", team="TES2", identifier=issue["identifier"]
                )
                done += 1
        accepted += done
        batch_log.append({"seq": seq, "sent": count, "accepted": done, "errors": errs(errors)[:2]})
        if done < count:
            refusal = {"accepted_total": accepted, "seq": seq, "errors": errs(errors)[:2]}
            break
    read = li_node(run, parent, "q1.li.children.read", "children(first:120){nodes{identifier}}")
    answers["children"] = {
        "probe_children": MAX_CHILDREN,
        "accepted": accepted,
        "refusal": refusal,
        "batches": batch_log,
        "children_read": len((read.get("children") or {}).get("nodes") or []),
        "read_seq": read["seq"],
    }

    other_team = run.li_issues("TES", ["cross"], "q1.li.cross.create")["cross"]
    answers["cross_team_parent_in_TES2"] = li_upd(
        run, other_team["id"], "q1.li.cross", parentId=root["id"]
    )
    answers["cross_team_read_child"] = li_node(run, other_team, "q1.li.cross.read")
    answers["cross_team_read_parent"] = li_node(run, root, "q1.li.cross.read")
    answers["read_cost_probe"] = li_node(run, root, "q1.li.cost")
    answers["objects"] = {
        "root": root["identifier"],
        "top": top["identifier"],
        "children_parent": parent["identifier"],
        "cross": other_team["identifier"],
    }
    run.write_stage("li_parents", answers)


def stage_li_links(run: Run) -> None:
    answers: dict[str, Any] = {}
    made = run.li_issues("TES2", list("abcdefgh"), "q2.li.links.create")
    project_a = run.li_project("TES2", "proj-a", "q2.li.links.project")
    project_b = run.li_project("TES2", "proj-b", "q2.li.links.project")
    other = run.li_issues("TES", ["other"], "q2.li.links.create")["other"]
    in_a = run.li_issues("TES2", ["pa"], "q2.li.links.create", projectId=project_a)["pa"]
    in_b = run.li_issues("TES2", ["pb"], "q2.li.links.create", projectId=project_b)["pb"]
    a, b, c, d, e, f, g, h = (made[k] for k in "abcdefgh")
    for kind, x, y in (("related", a, b), ("similar", c, d), ("blocks", e, f), ("duplicate", g, h)):
        add = li_rel(run, kind, x, y, f"q2.li.{kind}")
        read_x = li_node(run, x, f"q2.li.{kind}.read")
        read_y = li_node(run, y, f"q2.li.{kind}.read")
        cross_team = (
            li_rel(run, kind, x, other, f"q2.li.{kind}.cross-team") if kind != "duplicate" else None
        )
        answers[kind] = {
            "add": add,
            "read_from": read_x,
            "read_to": read_y,
            "cross_team": cross_team,
        }
    answers["related_again"] = li_rel(run, "related", a, b, "q2.li.related.again")
    answers["related_reverse_again"] = li_rel(run, "related", b, a, "q2.li.related.reverse")
    answers["duplicate_marks_state"] = li_node(
        run, g, "q2.li.duplicate.state", "identifier state{name type} canceledAt"
    )
    answers["cross_project_related"] = {
        "add": li_rel(run, "related", in_a, in_b, "q2.li.related.cross-project"),
        "read": li_node(run, in_b, "q2.li.related.read"),
    }
    rel = answers["related"]["add"]
    if rel.get("id"):
        data, errors, seq = run.li_mut(
            [
                Mut(
                    "a",
                    "issueRelationUpdate",
                    {"id": rel["id"], "input": {"type": "similar"}},
                    "success issueRelation{id type}",
                )
            ],
            "q2.li.update",
        )
        answers["relation_update_to_similar"] = {
            "seq": seq,
            "ok": not errors,
            "errors": errs(errors),
            "payload": data.get("a"),
        }
    cli = Cli(
        run,
        {"tes2": {"plugin": "linear", "config": {"api_key_env": "LINEAR_API_KEY", "team": "TES2"}}},
    )
    pair = run.li_issues("TES2", ["sym", "root"], "q2.li.fallback.create")
    sym, root = pair["sym"], pair["root"]
    set_results = []
    for issue, value in (
        (sym, [{"type": "caused-by", "direction": "out", "other": f"tes2:{root['id']}"}]),
        (root, [{"type": "caused-by", "direction": "in", "other": f"tes2:{sym['id']}"}]),
    ):
        set_results.append(
            summary(
                cli(
                    "task",
                    "metadata",
                    "set",
                    f"tes2:{issue['id']}",
                    "spike.links",
                    json.dumps(value),
                    q="fallback.set",
                )
            )
        )
    answers["fallback_metadata_writes"] = set_results
    stored, _ = li_body(run, sym, "q2.li.fallback.read")
    answers["fallback_after_metadata"] = split_links(stored)
    answers["fallback_after_metadata_quote"] = stored
    sent = (
        "Visible symptom text.\n\n## Links\n\n"
        f"- **caused-by** → [{root['identifier']}]({root['url']}) `tes2:{root['identifier']}`\n"
        f"* second bullet\n\n{re.search(SLOT_OPEN_PATTERN + '.*', stored, re.S).group(0) if SLOT_OPEN in stored else stored}"
    )
    answers["fallback_sent_quote"] = sent
    answers["fallback_write"] = li_upd(run, sym["id"], "q2.li.fallback.section", description=sent)
    back, _ = li_body(run, sym, "q2.li.fallback.read")
    answers["fallback_stored_quote"] = back
    answers["fallback_stored"] = split_links(back)
    answers["fallback_normalized"] = back != sent
    answers["fallback_read"] = cli_task(cli, f"tes2:{sym['id']}", "fallback.show")
    answers["edit_keeps_section"] = {
        "write": li_upd(
            run,
            sym["id"],
            "q2.li.fallback.edit1",
            description=back.replace(
                "Visible symptom text.", "Visible symptom text, edited by a person."
            ),
        ),
        "read": cli_task(cli, f"tes2:{sym['id']}", "fallback.show"),
        "stored": split_links(li_body(run, sym, "q2.li.fallback.read")[0]),
    }
    current, _ = li_body(run, sym, "q2.li.fallback.read")
    without = re.sub(r"#+ Links\s.*?(?=" + SLOT_OPEN_PATTERN + ")", "", current, flags=re.S)
    answers["edit_deletes_section_keeps_slot"] = {
        "write": li_upd(run, sym["id"], "q2.li.fallback.edit2", description=without),
        "read": cli_task(cli, f"tes2:{sym['id']}", "fallback.show"),
        "stored": split_links(li_body(run, sym, "q2.li.fallback.read")[0]),
    }
    answers["edit_rewrites_whole_body"] = {
        "write": li_upd(
            run,
            sym["id"],
            "q2.li.fallback.edit3",
            description="A person replaced the whole description.",
        ),
        "read": cli_task(cli, f"tes2:{sym['id']}", "fallback.show"),
        "other_end_read": cli_task(cli, f"tes2:{root['id']}", "fallback.show"),
    }
    answers["objects"] = {k: v["identifier"] for k, v in made.items()}
    run.write_stage("li_links", answers)


def stage_li_walkthrough(run: Run) -> None:
    """The design's symptoms-and-root-causes walk-through, run live on Linear."""
    cli = Cli(
        run,
        {"eng": {"plugin": "linear", "config": {"api_key_env": "LINEAR_API_KEY", "team": "TES2"}}},
    )
    slugs = [
        "stale-cache-lease",
        "TIMEOUT-42",
        "PAY-7",
        "TIMEOUT-42-acme",
        "TIMEOUT-42-globex",
        "PAY-7-acme",
        "THIRD-9",
    ]
    made = run.li_issues("TES2", slugs, "q4.walk.create")
    root = made["stale-cache-lease"]
    parents = {
        "TIMEOUT-42-acme": "TIMEOUT-42",
        "TIMEOUT-42-globex": "TIMEOUT-42",
        "PAY-7-acme": "PAY-7",
    }
    answers: dict[str, Any] = {
        "identifiers": {slug: issue["identifier"] for slug, issue in made.items()}
    }
    answers["parent_writes"] = {
        child: li_upd(run, made[child]["id"], "q4.walk.parent", parentId=made[parent]["id"])
        for child, parent in parents.items()
    }
    symptoms = ["TIMEOUT-42", "PAY-7", "THIRD-9"]
    answers["mirrors"] = {
        s: li_rel(run, "related", made[s], root, "q4.walk.related") for s in symptoms
    }
    metadata = {}
    in_edges = []
    for symptom in symptoms:
        value = [{"type": "caused-by", "direction": "out", "other": f"eng:{root['id']}"}]
        metadata[symptom] = summary(
            cli(
                "task",
                "metadata",
                "set",
                f"eng:{made[symptom]['id']}",
                "spike.links",
                json.dumps(value),
                q="walk.metadata",
            )
        )
        in_edges.append(
            {"type": "caused-by", "direction": "in", "other": f"eng:{made[symptom]['id']}"}
        )
    metadata["root"] = summary(
        cli(
            "task",
            "metadata",
            "set",
            f"eng:{root['id']}",
            "spike.links",
            json.dumps(in_edges),
            q="walk.metadata",
        )
    )
    answers["metadata_writes"] = metadata
    answers["read_root"] = li_node(run, root, "q4.walk.read")
    answers["read_root_metadata"] = cli_task(cli, f"eng:{root['id']}", "walk.show")
    answers["read_symptom"] = {s: li_node(run, made[s], "q4.walk.read") for s in symptoms}
    answers["read_symptom_metadata"] = {
        s: cli_task(cli, f"eng:{made[s]['id']}", "walk.show") for s in symptoms
    }
    answers["read_clients"] = {c: li_node(run, made[c], "q4.walk.read") for c in parents}
    answers["widespread_route_input_count"] = len(
        answers["read_root_metadata"].get("links_metadata") or []
    )
    run.write_stage("li_walkthrough", answers)


def batch_counts(run: Run, start: int) -> dict[str, Any]:
    """What the requests since `start` spent, from the request log."""
    per: dict[str, Any] = {}
    for line in run.requests_path.read_text(encoding="utf-8").splitlines():
        rec = json.loads(line)
        if rec["seq"] > start and rec["stage"] == run.stage and rec["q"].startswith("budget."):
            call = rec["q"]
            slot = per.setdefault(
                call,
                {
                    "requests": 0,
                    "cost": 0,
                    "complexity": 0,
                    "aliases": [],
                    "seqs": [],
                    "errors": [],
                },
            )
            slot["requests"] += 1
            slot["cost"] += rec.get("cost") or 0
            slot["complexity"] += rec.get("complexity") or 0
            slot["aliases"].append(rec["aliases"])
            slot["seqs"].append(rec["seq"])
            slot["errors"] += rec["errors"]
    return per


def stage_budget_linear(run: Run) -> None:
    made = run.li_issues("TES2", [f"t{i:02d}" for i in range(20)], "budget-setup.create")
    tickets = [made[f"t{i:02d}"] for i in range(20)]
    start = run.seq
    read_doc = (
        "query{"
        + " ".join(
            'i{}:issue(id:"{}"){{id identifier description updatedAt parent{{id}} children(first:50){{nodes{{id}}}} relations(first:50){{nodes{{id type relatedIssue{{id}}}}}} inverseRelations(first:50){{nodes{{id type issue{{id}}}}}}}}'.format(
                n, t["id"]
            )
            for n, t in enumerate(tickets)
        )
        + "}"
    )
    state: dict[str, Any] = {}

    def read(call: str) -> dict:
        data, errors, seq = run.li_read(read_doc, call)
        return {"errors": errs(errors), "seq": seq, "data": data}

    edges = [(tickets[i + 8], tickets[i % 8]) for i in range(12)]
    links = [(tickets[i % 20], tickets[(i + 3 + i // 20) % 20]) for i in range(30)]
    state["pass1_relations"] = read("budget.pass1.relations")
    state["pass1_parent_read"] = read("budget.pass1.parent")
    muts = [
        Mut(
            f"a{i}",
            "issueUpdate",
            {"id": c["id"], "input": {"parentId": p["id"]}},
            "success issue{id updatedAt}",
        )
        for i, (c, p) in enumerate(edges)
    ]
    data, errors, seq = run.li_mut(muts, "budget.pass1.parent")
    state["pass1_parent_write"] = {
        "seq": seq,
        "errors": errs(errors),
        "accepted": sum(1 for i in range(len(muts)) if data.get(f"a{i}")),
    }
    state["pass1_links_read"] = read("budget.pass1.links")
    muts = [
        Mut(
            f"a{i}",
            "issueRelationCreate",
            {"input": {"issueId": x["id"], "relatedIssueId": y["id"], "type": Enum("related")}},
            "success issueRelation{id}",
        )
        for i, (x, y) in enumerate(links)
    ]
    data, errors, seq = run.li_mut(muts, "budget.pass1.links")
    accepted = 0
    for i in range(len(muts)):
        rel = ((data.get(f"a{i}") or {}).get("issueRelation")) or {}
        if rel.get("id"):
            run.record_created(rel["id"], "relation", "linear")
            accepted += 1
    state["pass1_mirror_write"] = {
        "seq": seq,
        "accepted": accepted,
        "sent": len(muts),
        "errors": errs(errors)[:3],
    }
    endpoints = sorted({t["id"] for pair in links for t in pair})
    muts = [
        Mut(
            f"a{i}",
            "issueUpdate",
            {
                "id": tid,
                "input": {"description": "spike\n\n" + SLOT_OPEN + ' `{"spike.links":[]}` -->'},
            },
            "success issue{updatedAt}",
        )
        for i, tid in enumerate(endpoints)
    ]
    data, errors, seq = run.li_mut(muts, "budget.pass1.links")
    state["pass1_metadata_write"] = {
        "seq": seq,
        "sent": len(muts),
        "accepted": sum(1 for i in range(len(muts)) if data.get(f"a{i}")),
        "errors": errs(errors)[:3],
    }
    state["pass2_relations"] = read("budget.pass2.relations")
    state["pass2_parent_read"] = read("budget.pass2.parent")
    state["pass2_links_read"] = read("budget.pass2.links")
    sizes = {}
    for size in (50, 100):
        muts = [
            Mut(
                f"a{i}",
                "issueUpdate",
                {"id": tickets[i % 20]["id"], "input": {"title": run.title(f"t{i % 20:02d}")}},
                "success",
            )
            for i in range(size)
        ]
        data, errors, seq = run.li_mut(muts, f"aliases.size{size}")
        sizes[size] = {
            "seq": seq,
            "accepted": sum(1 for i in range(size) if data.get(f"a{i}")),
            "errors": errs(errors)[:2],
        }
    state["larger_alias_sizes"] = sizes
    state["counts"] = batch_counts(run, start)
    state["read_results"] = {
        k: {"seq": v["seq"], "errors": v["errors"]}
        for k, v in state.items()
        if k.endswith(("relations", "parent_read", "links_read"))
    }
    for key in (
        "pass1_relations",
        "pass1_parent_read",
        "pass1_links_read",
        "pass2_relations",
        "pass2_parent_read",
        "pass2_links_read",
    ):
        state.pop(key)
    run.write_stage("budget_linear", state)


def stage_budget_github(run: Run) -> None:
    made = run.gh_issues("r1", [f"t{i:02d}" for i in range(20)], "budget-setup.create")
    tickets = [made[f"t{i:02d}"] for i in range(20)]
    start = run.seq
    read_doc = (
        "query{"
        + " ".join(
            'i{}:node(id:"{}"){{... on Issue{{id number body updatedAt parent{{id}} subIssues(first:50){{nodes{{id}}}} relatesTo(first:50){{nodes{{id}}}}}}}}'.format(
                n, t["id"]
            )
            for n, t in enumerate(tickets)
        )
        + "}"
    )
    state: dict[str, Any] = {}

    def read(call: str) -> dict:
        data, errors, seq = run.gh_read(read_doc, call)
        return {
            "errors": errs(errors),
            "seq": seq,
            "bodies": {k: (v or {}).get("body") for k, v in data.items()},
        }

    edges = [(tickets[i + 8], tickets[i % 8]) for i in range(12)]
    links = [(tickets[i % 20], tickets[(i + 3 + i // 20) % 20]) for i in range(30)]
    state["pass1_relations"] = read("budget.pass1.relations")
    state["pass1_parent_read"] = read("budget.pass1.parent")
    time.sleep(2)
    muts = [
        Mut(
            f"a{i}",
            "addSubIssue",
            {"input": {"issueId": p["id"], "subIssueId": c["id"]}},
            "clientMutationId",
        )
        for i, (c, p) in enumerate(edges)
    ]
    data, errors, seq = run.gh_mut(muts, "budget.pass1.parent")
    state["pass1_parent_write"] = {
        "seq": seq,
        "accepted": sum(1 for i in range(len(muts)) if data.get(f"a{i}")),
        "errors": errs(errors)[:3],
    }
    state["pass1_links_read"] = read("budget.pass1.links")
    time.sleep(2)
    muts = [
        Mut(
            f"a{i}",
            "addRelatesTo",
            {"input": {"issueId": x["id"], "relatedIssueId": y["id"]}},
            "clientMutationId",
        )
        for i, (x, y) in enumerate(links)
    ]
    data, errors, seq = run.gh_mut(muts, "budget.pass1.links")
    state["pass1_mirror_write"] = {
        "seq": seq,
        "accepted": sum(1 for i in range(len(muts)) if data.get(f"a{i}")),
        "sent": len(muts),
        "errors": errs(errors)[:3],
    }
    time.sleep(2)
    endpoints = sorted({t["id"] for pair in links for t in pair})
    muts = [
        Mut(
            f"a{i}",
            "updateIssue",
            {
                "input": {
                    "id": tid,
                    "body": "spike\n\n" + SLOT_OPEN + ' {"spike.links":[]} -->',
                }
            },
            "issue{id}",
        )
        for i, tid in enumerate(endpoints)
    ]
    data, errors, seq = run.gh_mut(muts, "budget.pass1.links")
    state["pass1_metadata_write"] = {
        "seq": seq,
        "sent": len(muts),
        "accepted": sum(1 for i in range(len(muts)) if data.get(f"a{i}")),
        "errors": errs(errors)[:3],
    }
    state["pass2_relations"] = read("budget.pass2.relations")
    state["pass2_parent_read"] = read("budget.pass2.parent")
    state["pass2_links_read"] = read("budget.pass2.links")
    sizes = {}
    used = {(x["id"], y["id"]) for x, y in links}
    fresh = [
        (tickets[i], tickets[j])
        for i in range(20)
        for j in range(i + 5, 20)
        if (tickets[i]["id"], tickets[j]["id"]) not in used
        and (tickets[j]["id"], tickets[i]["id"]) not in used
    ][:60]
    time.sleep(6)
    muts = [
        Mut(
            f"a{i}",
            "addRelatesTo",
            {"input": {"issueId": x["id"], "relatedIssueId": y["id"]}},
            "clientMutationId",
        )
        for i, (x, y) in enumerate(fresh)
    ]
    data, errors, seq = run.gh_mut(muts, "aliases.size60")
    sizes["size60"] = {
        "seq": seq,
        "sent": len(muts),
        "accepted": sum(1 for i in range(len(muts)) if data.get(f"a{i}")),
        "errors": errs(errors)[:2],
    }
    data, errors, seq = run.gh_read(
        "query{"
        + 'n:node(id:"{}"){{... on Issue{{relatesTo(first:100){{totalCount}}}}}}'.format(
            tickets[0]["id"]
        )
        + "}",
        "aliases.read",
    )
    sizes["relates_to_total_on_one_issue"] = (data.get("n") or {}).get("relatesTo")
    state["larger_alias_sizes"] = sizes
    state["counts"] = batch_counts(run, start)
    for key in (
        "pass1_relations",
        "pass1_parent_read",
        "pass1_links_read",
        "pass2_relations",
        "pass2_parent_read",
        "pass2_links_read",
    ):
        state[key] = {k: state[key][k] for k in ("seq", "errors")}
    run.write_stage("budget_github", state)


CAPTURE_JS = """
const { chromium } = require('/home/nick.guest/ai-orchestrator/node_modules/playwright');
(async () => {
  const browser = await chromium.launch();
  const out = [];
  for (const [name, url] of JSON.parse(process.argv[2])) {
    const page = await browser.newPage({ viewport: { width: 1400, height: 1100 } });
    const response = await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60000 });
    await page.waitForTimeout(5000);
    await page.screenshot({ path: process.argv[3] + '/' + name + '.png', fullPage: true });
    const text = await page.evaluate(() => document.body.innerText.slice(0, 4000));
    out.push({ name, url, status: response.status(), bytes: text.length, sample: text });
    await page.close();
  }
  await browser.close();
  console.log(JSON.stringify(out));
})();
"""


def stage_capture(run: Run) -> None:
    """GitHub's public pages of the hierarchy and links, kept on the branch."""
    answers: dict[str, Any] = {}
    gh_parents = json.loads((run.dir / "gh_parents.json").read_text())
    boards_stage = json.loads((run.dir / "gh_boards.json").read_text())
    links_stage = json.loads((run.dir / "gh_links.json").read_text())
    obj = gh_parents["objects"]
    repo = f"https://github.com/{OWNER}/{REPO1}/issues"
    board_number = boards_stage["boards"]["x"]
    board_id = next(
        k
        for k, v in run.ledger.items()
        if v.get("kind") == "board" and v.get("number") == board_number
    )
    links_board = links_stage["fallback_board"]
    links_board_id = next(
        k
        for k, v in run.ledger.items()
        if v.get("kind") == "board" and v.get("number") == links_board
    )
    for id_ in (board_id, links_board_id):
        data, errors, seq = run.gh_mut(
            [
                Mut(
                    "a",
                    "updateProjectV2",
                    {"input": {"projectId": id_, "public": True}},
                    "projectV2{public}",
                )
            ],
            "capture.public",
        )
        answers.setdefault("made_public", []).append({"seq": seq, "errors": errs(errors)})
    pages = [
        ("hierarchy-parent", f"{repo}/{obj['root']}"),
        ("children-parent", f"{repo}/{obj['children_parent']}"),
        ("links-relates-to", f"{repo}/{links_stage['objects']['a']}"),
        ("links-blocked-by", f"{repo}/{links_stage['objects']['c']}"),
        ("links-duplicate", f"{repo}/{links_stage['objects']['e']}"),
        ("board-table", f"https://github.com/users/{OWNER}/projects/{board_number}/views/1"),
        ("fallback-section", links_stage["fallback_page"]["url"]),
    ]
    script = run.scratch / f"capture-{run.stamp}.cjs"
    script.write_text(CAPTURE_JS, encoding="utf-8")
    shots = run.dir / "pages"
    shots.mkdir(exist_ok=True)
    proc = subprocess.run(
        ["node", str(script), json.dumps(pages), str(shots)],
        capture_output=True,
        text=True,
        check=False,
        timeout=600,
    )
    script.unlink(missing_ok=True)
    answers["capture_exit"] = proc.returncode
    answers["stderr"] = proc.stderr.strip()[:500]
    if proc.returncode == 0:
        answers["pages"] = json.loads(proc.stdout)
    run.write_stage("capture", answers)


def stage_cleanup(run: Run) -> bool:
    """Delete everything this stamp created; answer whether every object is verified gone."""
    run.cleaning = True
    run.stage = "cleanup"
    answers: dict[str, Any] = {"swept": []}
    for repo in (REPO1, REPO2):
        page = None
        while True:
            after = f',after:"{page}"' if page else ""
            data, errors, seq = run.gh_read(
                f'query{{search(type:ISSUE,first:100{after},query:"repo:{OWNER}/{repo} in:title {run.stamp}"){{nodes{{... on Issue{{id number}}}} pageInfo{{hasNextPage endCursor}}}}}}',
                "cleanup.sweep",
            )
            for node in data["search"]["nodes"]:
                if node.get("id") and node["id"] not in run.ledger:
                    run.record_created(
                        node["id"], "issue", "github", repo=repo, number=node["number"], via="sweep"
                    )
                    answers["swept"].append(node["number"])
            if not data["search"]["pageInfo"]["hasNextPage"]:
                break
            page = data["search"]["pageInfo"]["endCursor"]
    data, errors, seq = run.gh_read(
        f'query{{user(login:"{OWNER}"){{projectsV2(first:50,query:"{run.stamp}"){{nodes{{id number title}}}}}}}}',
        "cleanup.sweep",
    )
    for node in data["user"]["projectsV2"]["nodes"]:
        if (
            node["title"].startswith(BOARD_TITLE_PREFIX + run.stamp)
            and node["id"] not in run.ledger
        ):
            run.record_created(
                node["id"],
                "board",
                "github",
                number=node["number"],
                title=node["title"],
                via="sweep",
            )
    data, errors, seq = run.li_read(
        f'query{{issues(first:250,includeArchived:true,filter:{{title:{{contains:"{run.stamp}"}}}}){{nodes{{id identifier team{{key}}}}}}}}',
        "cleanup.sweep",
    )
    for node in data["issues"]["nodes"]:
        if node["id"] not in run.ledger:
            run.record_created(
                node["id"],
                "issue",
                "linear",
                team=node["team"]["key"],
                identifier=node["identifier"],
                via="sweep",
            )
            answers["swept"].append(node["identifier"])
    data, errors, seq = run.li_read(
        f'query{{projects(first:50,includeArchived:true,filter:{{name:{{contains:"{run.stamp}"}}}}){{nodes{{id name}}}}}}',
        "cleanup.sweep",
    )
    for node in data["projects"]["nodes"]:
        if node["id"] not in run.ledger:
            run.record_created(node["id"], "project", "linear", via="sweep")

    def live(kind: str, tracker: str) -> list[str]:
        return [
            k
            for k, v in run.ledger.items()
            if v["kind"] == kind and v["tracker"] == tracker and v.get("ended") != "deleted"
        ]

    def gh_delete(ids: list[str], root: str, key: str, label: str, size: int) -> None:
        for start in range(0, len(ids), size):
            chunk = ids[start : start + size]
            data, errors, seq = run.gh_mut(
                [
                    Mut(f"a{i}", root, {"input": {key: id_}}, "clientMutationId")
                    for i, id_ in enumerate(chunk)
                ],
                label,
            )
            absent = {
                (e.get("path") or [""])[0]: e
                for e in errors
                if "Could not resolve to a node" in str(e.get("message"))
            }
            for i, id_ in enumerate(chunk):
                if f"a{i}" in data and data[f"a{i}"] is not None:
                    run.record_ended(id_, "deleted", f"{root} request {seq}")
                elif f"a{i}" in absent:
                    run.record_ended(
                        id_, "deleted", f"already gone: the 502 batch landed (read back at {seq})"
                    )
            for error in errors:
                answers.setdefault("errors", []).append(str(error.get("message"))[:200])
            time.sleep(3)

    gh_delete(live("issue", "github"), "deleteIssue", "issueId", "cleanup.gh.issues", 5)
    gh_delete(live("board", "github"), "deleteProjectV2", "projectId", "cleanup.gh.boards", 1)

    def li_delete(
        ids: list[str], make: Callable[[int, str], Mut], label: str, size: int, how: str
    ) -> None:
        for start in range(0, len(ids), size):
            chunk = ids[start : start + size]
            muts = [make(i, id_) for i, id_ in enumerate(chunk)]
            data, errors, seq = run.li_mut(muts, label)
            for i, id_ in enumerate(chunk):
                if data.get(f"a{i}"):
                    run.record_ended(id_, "deleted", f"{how}, request {seq}")
            for error in errors:
                answers.setdefault("errors", []).append(str(error.get("message"))[:200])

    li_delete(
        live("issue", "linear"),
        lambda i, id_: Mut(
            f"a{i}", "issueDelete", {"id": id_, "permanentlyDelete": True}, "success"
        ),
        "cleanup.li.issues",
        25,
        "issueDelete permanently",
    )
    li_delete(
        live("project", "linear"),
        lambda i, id_: Mut(f"a{i}", "projectDelete", {"id": id_}, "success"),
        "cleanup.li.projects",
        25,
        "projectDelete",
    )
    for key, value in run.ledger.items():
        if value["kind"] == "relation" and not value.get("ended"):
            run.record_ended(key, "deleted", "removed with its issues by the permanent issueDelete")
    answers["remaining_after"] = {
        "github_issues": live("issue", "github"),
        "github_boards": live("board", "github"),
        "linear_issues": live("issue", "linear"),
        "linear_projects": live("project", "linear"),
    }
    verified: dict[str, Any] = {}
    for repo in (REPO1, REPO2):
        data, _, _ = run.gh_read(
            f'query{{search(type:ISSUE,first:5,query:"repo:{OWNER}/{repo} in:title {run.stamp}"){{issueCount}}}}',
            "cleanup.verify",
        )
        verified[repo] = data["search"]["issueCount"]
    data, _, _ = run.gh_read(
        f'query{{user(login:"{OWNER}"){{projectsV2(first:5,query:"{run.stamp}"){{totalCount}}}}}}',
        "cleanup.verify",
    )
    verified["boards"] = data["user"]["projectsV2"]["totalCount"]
    data, _, _ = run.li_read(
        f'query{{issues(first:5,includeArchived:true,filter:{{title:{{contains:"{run.stamp}"}}}}){{nodes{{id}}}}}}',
        "cleanup.verify",
    )
    verified["linear_issues"] = len(data["issues"]["nodes"])
    data, _, _ = run.li_read(
        f'query{{projects(first:5,includeArchived:true,filter:{{name:{{contains:"{run.stamp}"}}}}){{nodes{{id trashed}}}}}}',
        "cleanup.verify",
    )
    verified["linear_projects_including_archived"] = data["projects"]["nodes"]
    answers["verified_after"] = verified
    inventory: dict[str, Any] = {}
    for key, value in run.ledger.items():
        bucket = inventory.setdefault(
            f"{value['tracker']}:{value['kind']}", {"total": 0, "deleted": 0, "open": []}
        )
        bucket["total"] += 1
        if value.get("ended") == "deleted":
            bucket["deleted"] += 1
        else:
            bucket["open"].append(value.get("number") or value.get("identifier") or key)
    answers["inventory"] = inventory
    run.write_stage("cleanup", answers)
    return not any(bucket["open"] for bucket in inventory.values())


STAGES = {
    "introspect": stage_introspect,
    "preflight": stage_preflight,
    "gh_parents": stage_gh_parents,
    "gh_boards": stage_gh_boards,
    "gh_links": stage_gh_links,
    "gh_duplicate_cross": stage_gh_duplicate_cross,
    "gh_plugin_project": stage_gh_plugin_project,
    "li_parents": stage_li_parents,
    "li_links": stage_li_links,
    "li_walkthrough": stage_li_walkthrough,
    "budget_linear": stage_budget_linear,
    "budget_github": stage_budget_github,
    "gh_landed60": stage_gh_landed60,
    "li_duplicate_cross": stage_li_duplicate_cross,
    "capture": stage_capture,
    "gh_fallback_page": stage_gh_fallback_page,
    "cleanup": stage_cleanup,
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stages", nargs="+", help=f"all, or any of {', '.join(STAGES)}")
    parser.add_argument("--stamp", help="continue an earlier run's stamp")
    parser.add_argument("--keep", action="store_true", help="leave objects for a later stage")
    parser.add_argument(
        "--no-github", action="store_true", help="send GitHub nothing; implies --keep"
    )
    args = parser.parse_args()
    stamp = args.stamp or dt.datetime.now(dt.UTC).strftime("%Y%m%dT%H%M%S")
    if not re.fullmatch(r"\d{8}T\d{6}", stamp):
        parser.error(f"--stamp {stamp!r} is not YYYYmmddTHHMMSS")
    unknown = sorted(set(args.stages) - set(STAGES) - {"all"})
    if unknown:
        parser.error(f"unknown stages {unknown}; choose from {', '.join(STAGES)}")
    run = Run(stamp)
    run.github_off = args.no_github
    run.bootstrap()
    names = [s for s in STAGES if s != "cleanup"] if args.stages == ["all"] else args.stages
    failed = False
    try:
        for name in names:
            run.stage = name
            gate = run.rate_gate()
            (run.dir / "gates.jsonl").open("a", encoding="utf-8").write(
                json.dumps({"stage": name, "at": dt.datetime.now(dt.UTC).isoformat(), **gate})
                + "\n"
            )
            print(
                f"[{stamp}] stage {name}: GitHub spent {run.github_points}, Linear requests {run.linear_requests}",
                flush=True,
            )
            if STAGES[name](run) is False:
                failed = True
    except (
        QuotaStop,
        RateRefused,
        Refused,
        RuntimeError,
        KeyError,
        TypeError,
        urllib.error.URLError,
    ) as err:
        failed = True
        print(
            f"[{stamp}] stage {run.stage} stopped: {type(err).__name__}: {err}",
            file=sys.stderr,
            flush=True,
        )
    finally:
        if not args.keep and not args.no_github and "cleanup" not in names:
            run.stage = "cleanup"
            failed = not stage_cleanup(run) or failed
    print(
        f"[{stamp}] totals: GitHub points {run.github_points}, Linear requests {run.linear_requests}, complexity {run.linear_complexity}",
        flush=True,
    )
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
