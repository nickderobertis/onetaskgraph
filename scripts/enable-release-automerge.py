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

ENABLE = """mutation($pullRequestId: ID!) {
  enablePullRequestAutoMerge(input: {pullRequestId: $pullRequestId, mergeMethod: SQUASH}) {
    pullRequest { number autoMergeRequest { mergeMethod } }
  }
}"""


class Refused(Exception):
    pass


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
        candidates = [
            pull for pull in pulls
            if isinstance(pull, dict)
            and str(pull.get("head", {}).get("ref", "")).startswith("release-plz-")
            and (pull.get("head", {}).get("repo") or {}).get("full_name") == repository
        ]
        if not candidates:
            print("enable-release-automerge: no release pull request is open; nothing to set", file=sys.stderr)
            return 0
        pull = max(candidates, key=lambda candidate: candidate["number"])
        answer = call(graphql, token, {"query": ENABLE, "variables": {"pullRequestId": pull["node_id"]}})
        errors = answer.get("errors") if isinstance(answer, dict) else ["no answer"]
        if errors:
            raise Refused(f"GitHub refused auto-merge for #{pull['number']}: {json.dumps(errors)[:500]}")
    except Refused as error:
        print(f"enable-release-automerge: {error}", file=sys.stderr)
        print("enable-release-automerge: next: the release pull request will not merge itself; fix what GitHub reported (auto-merge must be allowed in the repository settings) and re-run this workflow, or merge it by hand once its checks pass", file=sys.stderr)
        return 1
    print(f"enable-release-automerge: #{pull['number']} ({pull['head']['ref']}) will squash-merge once its required checks pass", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
