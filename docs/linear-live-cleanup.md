# Linear live document residue

Hosted run [40741930571, test-linux](https://github.com/nickderobertis/onetaskgraph/actions/runs/40741930571/job/123005634093)
is the reported failure: `linear_assets.rs:1194` in
`linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy`
was refused by Linear's maximum of 1000 documents per team. A refused create
cannot return an id for the journey's cleanup journal. This dispatch could not
re-fetch the hosted log: `gh run view 40741930571 --log-failed` reported that
the run was still in progress and its logs were unavailable.

There are two live document producers:

- `crates/onetaskgraph-linear/tests/live.rs`: `drive_documents` creates a filed and a
  loose document and explicitly deletes both. `drive_follow_ups` creates a third for
  metadata/Markdown probes. All three use `artifact_title`, and `run_then_cleanup`
  calls `remove_artifacts` for this run's stamped titles even when the journey returns
  an error. Updates and the rejected labelled write do not create additional documents.
- `crates/onetaskgraph-linear-e2e/tests/e2e/linear_assets.rs`: the live asset journey
  copies one document, then recopies it onto the existing destination. `live_observer`
  journals ids returned by `documentCreate`, and the journey catches assertion panics,
  deletes the journalled items, and rethrows. Previously its title was
  `onetaskgraph disposable asset document <timestamp>`, which no orphan sweep recognized.
  Its title now carries the same machine/process/time ownership stamp as the other lane.

Normal cleanup therefore exists; interrupted-host recovery was missing. A killed process
never reaches either cleanup. The Linear lane's local `sweep_orphans` additionally needs
positive evidence from the writing machine's registration locks; it cannot recover work
from a discarded hosted runner. The scheduled janitor enumerated GitHub artifacts only
and explicitly never read Linear. Thus documents from interrupted live journeys could
remain indefinitely, and the asset journey's old names were not sweepable even locally.
Code and the reported refusal establish those paths, not the exact composition of the team's
1000 documents. No credential or captured account listing was available to this dispatch.

The scheduled workflow now also invokes `onetaskgraph-live-janitor --linear` with only
its Linear credential and test-team nomination. It lists documents before writing and
removes only exact stamped test titles older than 24 hours, directly in that team or
under a project belonging solely to that team. The expiry deliberately survives the loss
of the CI machine; it is independent of the local lock sweep. Fresh documents, malformed
or unvouched stamps, legacy asset titles, unstamped documents, unknown/shared ownership,
and other teams are protected. Listing errors stop before deletion; deletion refusals
fail the invocation. Read and write caps and pacing bound each scheduled pass.

Offline subprocess journeys replay schema-derived recorded response fixtures through
HTTP, including stale machine/CI/project documents, fresh and unstamped documents, other
teams, pagination and delete refusals. Fixture provenance is explicit in
`crates/onetaskgraph-live-janitor/tests/fixtures/README.md`; these are not captured account
responses. Both GraphQL operations are checked against a pinned official SDK schema.
No live assertion was removed or loosened and no journey was skipped.

Legacy residue is deliberately not swept by guessing at its title. The manager is taking
one-time cleanup of the existing residue to the user separately. The live asset lane
remains refused until that cleanup restores document capacity; this code does not claim
to have emptied the team or verified a credentialed run.

## Verification of the finished change

On this branch, the following passed:

- The test, lint and formatting targets for the janitor, shared live support, Linear
  plugin, Linear e2e, scripts and workflow projects completed all 20 tasks
  successfully (two lint tasks used Nx cache). This includes
  146 Linear e2e tests, 95 workflow tests, the janitor HTTP journeys and schema checks,
  and the script guard journeys. No Linear credential was available; contributor
  declines are not evidence of a successful live API run.
- Type checking passed for the workflow, janitor, shared live support, Linear plugin
  and Linear e2e projects.
- The store-fixture and live-janitor guards and the whitespace check passed.
- The seven Linear janitor subprocess journeys also passed directly. Temporarily
  replacing the stale machine document's ownership title with an ordinary title made
  the removal assertion fail because that document was preserved; the fixture was
  restored before the full Nx run completed.

The previously reported publication assertion at `.github/tests/release-cd.test.mjs:814`
did not recur. In a separate detached checkout of `main` at
`057b6c30b35314d1c2a70daef1feec3f5b14ef67`, after installing the pinned `.github`
dependencies, the release-CD test file passed all 12 tests.
The full workflow suite on this branch also passed that publication journey. The
separate checkout was removed afterward; no release tooling was changed.

## Publication repair verification

The shared live-support crate no longer owns the Linear prefix. The Linear plugin
exports it, matching the existing GitHub plugin's ownership of its design prefix;
the janitor's Cargo and Nx dependencies agree. No GitHub lane depends on that plugin.

The repair also validates incoming fixture-server requests, represents advancing
pagination with a required cursor, and checks the actual listing filter and recorded
delete acknowledgement against the pinned upstream schema. Nine Linear subprocess
journeys passed, including direct-team precedence, empty project ownership and a zero
ownership timestamp, missing ownership fields and a failure after the first deletion. Temporarily removing the zero-timestamp guard made the new
journey fail on the unexpected deletion; restoring it made the full suite pass.

The main-branch release-CD recheck passed all 12 tests at
`057b6c30b35314d1c2a70daef1feec3f5b14ef67`. Its local HTTP double emitted connection-reset
tracebacks, but the test runner returned success. The earlier publication assertion
did not reproduce.

Two narrow lint exceptions retain established repository conventions: the scheduled
janitor already owns hosted sweeps, so its Cargo and Nx Linear dependency declarations
carry `new_command_or_client_gets_its_own_project` markers; the shared fixture guard's
central MANIFEST already inventories domain stores, so the Linear listing entry carries
`code_lands_in_the_domain_that_owns_it`. The non-store delete-acknowledgement
registration carries the same marker because that central table also requires every
fixture to be named. The manager confirmed those placements and a
follow-up records the rule-applicability question. No lint configuration changed.

The parallel verification run hit a packaging race: Maturin could not open its
`target/maturin/onetaskgraph` intermediate while script and workflow publication
journeys packaged the same tree. The manager confirmed the race is covered by a
separate build-lock task. These targets are being rerun serially, with two Rust test
threads and no committed configuration change. The busy host's load averages were
25.67, 37.44 and 31.81; the first Linear e2e run passed all 146 tests in 803.92 seconds.
