# Recorded GitHub responses for the release tail

`../../release-tail.test.mjs` replays these through `ONEVCS_GH`, onevcs's seam for replaying
recorded responses, while it runs `publish`'s last step as release.yml writes it. Every file under
`recordings/` is a real GitHub REST response in onevcs's envelope — `{"request", "status",
"headers", "body"}` — recorded read-only from this repository with
`gh api --include --method GET <path>` on 2026-10-31 and 2026-11-01, byte for byte as gh printed it. Its
`<name>.source.json` sidecar names the API path and the release, pull request or run it was
fetched for.

The release is v0.12.13 (commit 3b6b228f): its own three reads, the runs at its commit and at
each of its four pull requests' heads, and the jobs of every run the figures select. One more is
the releases listing answered 401 under an invalid token, the error status the tail must exit
non-zero on.

`recordings-v0.13.0/` holds the same reads for v0.13.0 (commit c6adfa0d), whose release commit
carries a push-triggered "Visual docs" run that concluded skipped beside release-plz's run, the
Release run and the cache warmer's. The jobs of the skipped run and of the warmer are recorded too,
though the figures read neither: served, they make a script that reads them count the read rather
than fail on a missing recording, which is how v0.13.0's tail once read 7 release requests against
6. Nothing is hand-written or edited: the one change any response is served with — the
`publish` job of the Release run served in progress, because the tail runs inside it — is the
test's, applied when it serves, and named by that recording's sidecar under
`served_in_progress`.

`reconcile.mjs` re-fetches every path of one release's directory (`recordings` unless named) and diffs it against its recording, ignoring only what
GitHub rewrites (the date, etag, last-modified, rate-limit and request-id headers, and `download_count` and
`updated_at`). It reaches GitHub, so it runs by hand and never in a gate.
