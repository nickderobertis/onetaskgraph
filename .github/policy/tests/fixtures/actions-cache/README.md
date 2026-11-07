# Recorded Actions cache answers

`listing.json` is a real answer of `GET /repos/{r}/actions/caches?sort=size_in_bytes&direction=desc&per_page=100`
for this repository, every page in order, as `recorded_at` says, and `events/` holds real pull requests,
each projected to the fields the cache guards read and wrapped in the `pull_request` event's shape,
fork and same-repository. All three are create-repo's own recordings
(`skills/bootstrap/create-repo/tests/` at dero-skills 270bea5d287f). None of them holds a secret:
a listing is cache keys, refs and sizes. `.github/policy/tests/rust-cache-warm.test.mjs` replays them
at GitHub's REST boundary and evaluates the workflows' guards under them.
