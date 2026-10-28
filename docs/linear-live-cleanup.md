# Linear live document residue

Hosted run [40741930571, test-linux](https://github.com/nickderobertis/onetaskgraph/actions/runs/40741930571/job/123005634093)
failed on 2026-10-28 at 18:21:14 UTC. Its log names
`linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy`,
`linear_assets.rs:1194`, and Linear's `QUOTA_EXCEEDED` response with
`quota: max-documents-per-team`: “Your team can have a maximum of 1000 documents.”
The task's image reads returned HTTP 200, and the log subsequently confirms
`live disposable issue … removed=true`. No document was created by that refused request.
The main Linear live journey passed earlier in the same job, at 18:19:49 UTC.

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
Code and this failure log establish those paths, not the exact composition of the team's
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
