# Recorded GitHub responses for the release tail

`../../release-tail.test.mjs` replays these through `ONEVCS_GH`, onevcs's seam for replaying
recorded responses, while it runs `publish`'s last step as release.yml writes it. Every file under
`recordings/` is a real GitHub REST response in onevcs's envelope — `{"request", "status",
"headers", "body"}` — recorded read-only from this repository with
`gh api --include --method GET <path>` on 2026-10-31, byte for byte as gh printed it. Its
`<name>.source.json` sidecar names the API path and the release, pull request or run it was
fetched for.

The release is v0.12.13 (commit 3b6b228f): its own three reads, the runs at its commit and at
each of its four pull requests' heads, and the jobs of every run the figures select. One more is
the releases listing answered 401 under an invalid token, the error status the tail must exit
non-zero on. Nothing is hand-written or edited: the one change any response is served with — the
`publish` job of the Release run served in progress, because the tail runs inside it — is the
test's, applied when it serves, and named by that recording's sidecar under
`served_in_progress`.

`reconcile.mjs` re-fetches every path and diffs it against its recording, ignoring only what
GitHub rewrites (the date, etag, last-modified, rate-limit and request-id headers, and `download_count` and
`updated_at`). It reaches GitHub, so it runs by hand and never in a gate.
