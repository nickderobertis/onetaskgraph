The private `github-workflows` project owns macOS routing, offline workflow graph proofs,
and telemetry budgets. Run `just github-workflows-check` from the repository root;
its uncached Nx target records `.github/.telemetry/macos-policy.json`. The budgets target
reads that recording without running scenarios again. Registration commands are relative
to `.github`, because onebudgetspec runs commands beside their budgets file; standalone
reporter commands retain the repository-root paths.

The classifier's interface is `node .github/macos-policy.mjs classify --event PATH --base
REF --head REF`. `GITHUB_EVENT_NAME` is authoritative in Actions; offline fixtures carry
`event_name`. Malformed/unreadable PRs return a successful conservative classification
requiring Mac. Non-PR automatic events never require Mac. A same-repository `release-plz-`
branch and semantic version/changelog-only diff authorize omission; an author's identity
or a title does not. The classifier prints JSON and writes the corresponding fields to
`GITHUB_OUTPUT` when supplied. Its exit codes are 0 for a decision and 1 for invalid
invocation or failed output IO; a failed classifier cannot satisfy either required carrier.

`macos-result.mjs NEEDS_JSON` exits 0 only for successful required work or a successful
classification authorizing skipped work; explicit manual diagnostics require successful
work. It exits 1 for malformed inputs or failed/cancelled/missing prerequisites, prints
errors to stderr, and distinguishes omission from executed proof in stdout.

`macos-budget.mjs --metric jobs-per-run|runs-per-change --telemetry PATH` exits 0 after
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
