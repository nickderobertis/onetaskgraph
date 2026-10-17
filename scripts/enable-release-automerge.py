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
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass

ENABLE = """mutation($pullRequestId: ID!) {
  enablePullRequestAutoMerge(input: {pullRequestId: $pullRequestId, mergeMethod: SQUASH}) {
    pullRequest { number autoMergeRequest { mergeMethod } }
  }
}"""


class Refused(Exception):
    pass


@dataclass(frozen=True)
class PullRequest:
    number: int
    node_id: str
    head_ref: str
    head_repository: str | None

    @classmethod
    def parse(cls, value: object) -> "PullRequest":
        head = value.get("head") if isinstance(value, dict) else None
        repository = head.get("repo") if isinstance(head, dict) else None
        if not isinstance(value, dict) or not isinstance(value.get("number"), int) or not isinstance(value.get("node_id"), str) or not isinstance(head, dict) or not isinstance(head.get("ref"), str) or not (repository is None or isinstance(repository, dict)):
            raise Refused(f"GitHub listed something that is not a pull request: {value!r:.200}")
        full_name = repository.get("full_name") if repository else None
        return cls(value["number"], value["node_id"], head["ref"], full_name if isinstance(full_name, str) else None)

    def is_release_of(self, repository: str) -> bool:
        """release-plz's own pull request: a `release-plz-` branch of this repository, not a fork's."""
        return self.head_ref.startswith("release-plz-") and self.head_repository == repository


def call(url: str, token: str, body: dict | None = None) -> object:
    request = urllib.request.Request(
        url,
        data=None if body is None else json.dumps(body).encode(),
        method="GET" if body is None else "POST",
        headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json", "Content-Type": "application/json", "User-Agent": "onetaskgraph-release-plz"},
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.loads(response.read() or b"null")
    except urllib.error.HTTPError as error:
        raise Refused(f"GitHub answered HTTP {error.code} to {url}: {error.read()[:300].decode('utf-8', 'replace')}") from error
    except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError) as error:
        raise Refused(f"could not ask GitHub at {url}: {error}") from error


def main() -> int:
    token = os.environ.get("GH_TOKEN", "")
    repository = os.environ.get("GITHUB_REPOSITORY", "")
    api = os.environ.get("GITHUB_API_URL", "https://api.github.com").rstrip("/")
    graphql = os.environ.get("GITHUB_GRAPHQL_URL", f"{api}/graphql")
    if not token or repository.count("/") != 1:
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
        if not isinstance(enabled, dict) or not enabled.get("autoMergeRequest"):
            raise Refused(f"GitHub answered auto-merge for #{pull.number} without an auto-merge request: {json.dumps(answer)[:300]}")
    # A malformed GitHub answer is a refusal like any other: it says no auto-merge was set.
    except (Refused, AttributeError, TypeError) as error:
        print(f"enable-release-automerge: {error}", file=sys.stderr)
        print("enable-release-automerge: next: the release pull request will not merge itself; fix what GitHub reported (auto-merge must be allowed in the repository settings) and re-run this workflow, or merge it by hand once its checks pass", file=sys.stderr)
        return 1
    print(f"enable-release-automerge: #{pull.number} ({pull.head_ref}) will squash-merge once its required checks pass", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
