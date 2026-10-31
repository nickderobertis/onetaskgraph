The private `github-workflows` project owns the workflow graph, the four pull-request jobs'
contract and the CD journeys; `github-workflow-policy` (`policy/`) owns macOS routing, the
offline workflow policy proofs and the telemetry budgets, so a crate edit that selects the CD
journeys does not run the policy proofs. Run `just github-workflows-check` from the repository
root for both; `github-workflow-policy`'s uncached test target records
`.github/.telemetry/macos-policy.json`, and its budgets target reads that recording without
running scenarios again. Registration commands in `policy/budgets.yaml` are relative to
`.github/policy`, because onebudgetspec runs commands beside their budgets file; standalone
reporter commands retain the repository-root paths.

The classifier's interface is `node .github/macos-policy.mjs classify --event PATH --base
REF --head REF`. `GITHUB_EVENT_NAME` is authoritative in Actions; offline fixtures carry
`event_name`. Malformed/unreadable PRs return a successful conservative classification
requiring Mac. Non-PR automatic events never require Mac. A same-repository `release-plz-`
branch and semantic version/changelog-only diff authorize omission; an author's identity
or a title does not. The classifier prints JSON and writes the corresponding fields to
`GITHUB_OUTPUT` when supplied. Its exit codes are 0 for a decision and 1 for invalid
invocation or failed output IO. `lint` runs it on this repository's `release-plz-` branch,
where `test-macos` is skipped, and fails unless it authorizes the omission.

The root `budgets.yaml`'s seven `release-tail` budgets are checked by `publish`'s last step;
`tests/release-tail.test.mjs` runs that step and the install step before it as release.yml
writes them, over GitHub responses recorded from this repository under
`tests/fixtures/release-tail/` (its README says how they were taken and how to reconcile
them). `tests/ci-events.test.mjs` holds which workflows and jobs run on a pull request and
on the release commit, and `tests/gate-time.test.mjs` the timing wrapper `just gate` runs
inside; all three are `github-workflows:budget-journeys`.

`policy/macos-budget.mjs --metric jobs-per-run|runs-per-change --telemetry PATH` exits 0 after
validating and reporting current telemetry, and 1 for invalid invocation, missing/stale
recordings or report IO failure. Numeric limits live solely in budgets.yaml. The internal
Python semantic comparator exits 0 with a boolean result and 1 for unreadable refs,
malformed data or invalid invocation. `semantic-json.mjs` exits 0 with parsed Bun values
and 1 for malformed input; the authoritative product-version validator is in
scripts/product_versions.py. Restore readable refs/data before retrying those errors.

Reusable calls use the exact upstream source pinned in reusable-lock.json, with one
local fixture annotation. Unknown calls, runner OS expressions and unresolved matrices
fail explicitly. The annotation cannot alter upstream bytes unnoticed. Release payload
proofs remain in scripts/test-distribution.sh and scripts/check-release-packaging.sh,
behind their existing distribution targets. The local journey executes only host-native
products; Darwin compatibility is proved by tags, contents, hashes and the declared
native Apple Silicon installers. Those installers' real Mac execution is draft-run evidence,
which can be queued or skipped without being claimed as executed locally.
