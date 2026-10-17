#!/usr/bin/env python3
"""Set release-plz's release pull request to merge itself once its required checks pass.

The release-plz job of .github/workflows/release-plz.yml runs this after it opens or updates
that pull request. It asks GitHub for the open pull requests, picks the newest whose head is a
`release-plz-` branch of this repository — never a fork's, and never anything else — and asks
GraphQL's `enablePullRequestAutoMerge` to squash-merge it. That is auto-merge, not a merge:
GitHub merges only once every required check concludes success, which is the condition the
release pull request has to meet like any other.

Inputs: GH_TOKEN (required), GITHUB_REPOSITORY (owner/name), and GITHUB_API_URL and
GITHUB_GRAPHQL_URL, which GitHub Actions sets and the journey points at a double. Exit codes:
0 set, or no release pull request open; 1 GitHub refused or could not be asked.
"""

# llmlint: ignore-file[async_typed_clients_at_boundaries] Two sequential calls, the second needing the first's answer, in a short CI step with nothing to overlap.

from __future__ import annotations

import json
import os
import re
import urllib.parse
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import NewType

PullNumber = NewType("PullNumber", int)
NodeId = NewType("NodeId", str)
BranchRef = NewType("BranchRef", str)
RepositoryName = NewType("RepositoryName", str)
Token = NewType("Token", str)

ENABLE = """mutation($pullRequestId: ID!) {
  enablePullRequestAutoMerge(input: {pullRequestId: $pullRequestId, mergeMethod: SQUASH}) {
    pullRequest { number autoMergeRequest { mergeMethod } }
  }
}"""


class Refused(Exception):
    pass


class SameOriginRedirects(urllib.request.HTTPRedirectHandler):
    """Follow a redirect only within the origin a token was sent to.

    urllib copies every header but the body's onto the redirected request, Authorization
    included, so a redirect to another host would hand it the token. Such a redirect is
    refused instead, as the HTTP error it arrived as.
    """

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        before, after = urllib.parse.urlsplit(req.full_url), urllib.parse.urlsplit(newurl)
        if (before.scheme, before.netloc) != (after.scheme, after.netloc):
            raise urllib.error.HTTPError(req.full_url, code, f"refusing to follow a redirect to {after.scheme}://{after.netloc} with credentials", headers, fp)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


AUTHENTICATED = urllib.request.build_opener(SameOriginRedirects)


@dataclass(frozen=True)
class PullRequest:
    number: PullNumber
    node_id: NodeId
    head_ref: BranchRef
    head_repository: RepositoryName | None

    @classmethod
    def parse(cls, value: object) -> "PullRequest":
        head = value.get("head") if isinstance(value, dict) else None
        repository = head.get("repo") if isinstance(head, dict) else None
        if not isinstance(value, dict) or not isinstance(value.get("number"), int) or not isinstance(value.get("node_id"), str) or not isinstance(head, dict) or not isinstance(head.get("ref"), str) or not (repository is None or isinstance(repository, dict)):
            raise Refused(f"GitHub listed something that is not a pull request: {value!r:.200}")
        full_name = repository.get("full_name") if repository else None
        return cls(PullNumber(value["number"]), NodeId(value["node_id"]), BranchRef(head["ref"]), RepositoryName(full_name) if isinstance(full_name, str) else None)

    def is_release_of(self, repository: RepositoryName) -> bool:
        """release-plz's own pull request: a `release-plz-` branch of this repository, not a fork's."""
        return self.head_ref.startswith("release-plz-") and self.head_repository == repository


def call(url: str, token: Token, body: dict | None = None) -> object:
    request = urllib.request.Request(
        url,
        data=None if body is None else json.dumps(body).encode(),
        method="GET" if body is None else "POST",
        headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json", "Content-Type": "application/json", "User-Agent": "onetaskgraph-release-plz"},
    )
    try:
        with AUTHENTICATED.open(request, timeout=60) as response:
            return json.loads(response.read() or b"null")
    except urllib.error.HTTPError as error:
        raise Refused(f"GitHub answered HTTP {error.code} to {url}: {error.msg}: {(error.read() if error.fp else b'')[:300].decode('utf-8', 'replace')}") from error
    except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as error:
        raise Refused(f"could not ask GitHub at {url}: {error}") from error


def main() -> int:
    token = Token(os.environ.get("GH_TOKEN", ""))
    repository = RepositoryName(os.environ.get("GITHUB_REPOSITORY", ""))
    api = os.environ.get("GITHUB_API_URL", "https://api.github.com").rstrip("/")
    graphql = os.environ.get("GITHUB_GRAPHQL_URL", f"{api}/graphql")
    for name, url in (("GITHUB_API_URL", api), ("GITHUB_GRAPHQL_URL", graphql)):
        try:
            parts = urllib.parse.urlsplit(url)
            usable = parts.scheme in ("http", "https") and bool(parts.hostname) and "@" not in parts.netloc and not parts.query and not parts.fragment
            parts.port  # noqa: B018 - raises ValueError on a port that is not 0-65535
        except ValueError:
            usable = False
        if not usable:
            print(f"enable-release-automerge: {name} is {url!r}, not an http(s) URL naming a host", file=sys.stderr)
            print(f"enable-release-automerge: next: unset {name}; GitHub Actions sets it to GitHub's own", file=sys.stderr)
            return 1
    if not token or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        print("enable-release-automerge: GH_TOKEN and an owner/name GITHUB_REPOSITORY are required", file=sys.stderr)
        print("enable-release-automerge: next: set GH_TOKEN from RELEASE_PLZ_TOKEN, as .github/workflows/release-plz.yml does", file=sys.stderr)
        return 1
    try:
        pulls = call(f"{api}/repos/{repository}/pulls?state=open&per_page=100", token)
        if not isinstance(pulls, list):
            raise Refused(f"GitHub's list of open pull requests is not a list: {pulls!r:.200}")
        candidates = [pull for pull in map(PullRequest.parse, pulls) if pull.is_release_of(repository)]
        if not candidates:
            print("enable-release-automerge: no release pull request is open; nothing to set", file=sys.stderr)
            return 0
        pull = max(candidates, key=lambda candidate: candidate.number)
        answer = call(graphql, token, {"query": ENABLE, "variables": {"pullRequestId": pull.node_id}})
        errors = answer.get("errors") if isinstance(answer, dict) else ["no answer"]
        if errors:
            raise Refused(f"GitHub refused auto-merge for #{pull.number}: {json.dumps(errors)[:500]}")
        enabled = ((answer.get("data") or {}).get("enablePullRequestAutoMerge") or {}).get("pullRequest") or {}
        request = enabled.get("autoMergeRequest") if isinstance(enabled, dict) else None
        if not isinstance(request, dict) or request.get("mergeMethod") != "SQUASH" or enabled.get("number") != pull.number:
            raise Refused(f"GitHub answered auto-merge for #{pull.number} without a squash auto-merge request on it: {json.dumps(answer)[:300]}")
    # A malformed GitHub answer is a refusal like any other: it says no auto-merge was set.
    except (Refused, AttributeError, TypeError) as error:
        print(f"enable-release-automerge: {error}", file=sys.stderr)
        print("enable-release-automerge: next: the release pull request will not merge itself; fix what GitHub reported (auto-merge must be allowed in the repository settings) and re-run this workflow, or merge it by hand once its checks pass", file=sys.stderr)
        return 1
    print(f"enable-release-automerge: #{pull.number} ({pull.head_ref}) will squash-merge once its required checks pass", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
