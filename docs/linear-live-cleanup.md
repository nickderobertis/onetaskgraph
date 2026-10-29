# Linear live document lifecycle

Linear retains deleted documents in trash. Deletion retires test artifacts; it does
not restore the team's document quota. The nominated `LINEAR_WRITE_TEAM` is the only
team these journeys may mutate. Never replace that nomination by discovering an
arbitrary writable team.

Ordinary affected CI passes reuse exactly four standing document slots: filed,
loose and follow-up probes in the plugin lane, and the asset destination in the
binary lane. Their identity is stable within the nominated team, without a process
timestamp. Required parent projects persist too. Each invocation resets mutable
contents to known run-specific values before asserting updates, reads, filtering,
metadata, Markdown, image integrity and unchanged recopy. An absent or incomplete
set refuses with a setup requirement before any `documentCreate`.

Initialize or recover the standing set explicitly by manually dispatching
`.github/workflows/linear-documents.yml`. Initialization creates only missing slots;
partial initialization discovers existing objects by exact identity. An uncertain
create acknowledgement requires discovery before any retry. Unknown, duplicate or
ambiguous ownership fails closed. A retained marker alone never authorizes a write:
exact recognized ownership and verified nominated-team membership are both required.

The same workflow runs one scheduled fresh-creation pass each day. It exercises the
original filed and loose writes, follow-up metadata creation, and first document
asset copy, with the original deletion/absence assertions and failure cleanup.
This deliberately moves fresh-creation coverage to the daily schedule; ordinary
selected journeys still run their non-creation assertions. The assertion-by-assertion
mapping is in [the coverage inventory](linear-document-assertion-inventory.md).
Issue, label and state coverage remains in the live journeys.

The document mode environment variable is
`ONETASKGRAPH_LINEAR_DOCUMENT_MODE`: `ordinary`, `fresh`, or `initialize`.
CI's Linux affected check and the daily/initialization job share the
`linear-live-documents` concurrency group with `cancel-in-progress: false` and
`queue: max`. This preserves up to 100 pending writers under
[GitHub's queue semantics](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency).
The lock covers the affected gate because its live journeys are embedded in that
gate. Other platform checks do not share the Linear lock. Machine-local Session
exclusivity remains an additional guard.

Local invocations use a separate reusable set containing the existing machine/process
ownership stamp and cannot resolve CI slots. Default local `ordinary` refuses before
writing when its set is absent. Opt in with `ONETASKGRAPH_LINEAR_DOCUMENT_MODE=initialize`:
that invocation initializes its isolated slots, runs the ordinary reuse assertions, and
retires the set in the same process. It prints and journals this as an explicit local
creation pass, outside typical CI sizing. Normal invocation cleanup
retires local slots. A failure or panic at any point after the first slot create,
partway through initialization included, retires every slot the invocation created —
by the id Linear acknowledged, or for a create acknowledged without one by its exact
identity — before the lane fails; those creations stay counted in its journal. After a killed invocation, the existing local registration/orphan
sweep may retire them only with positive evidence that their registration is dead;
still-active registrations remain protected. There is no global coordination for
local invocations.

Standing CI slots and their parents survive normal cleanup, cancellation residue
cleanup, local orphan sweeping and the hourly janitor regardless of age. Fresh-mode
artifacts remain disposable with stamped ownership and expiry safeguards. The janitor
must continue protecting unrelated, fresh, malformed, ambiguous and unvouched records.
Never infer ownership from a similar title or treat trashing as quota recovery.

## Per-pass measurement and privacy

The ordinary budget measures one warm affected CI pass of both lanes: zero successful
`documentCreate` operations. The daily budget measures one fresh pass of both lanes:
three plugin documents and one binary asset document. Retries and pass frequency do
not weight either figure. Cold standing-set initialization and failed-pass residue
are separate diagnostic detail, not part of either pass total.

Offline HTTP replay drives the actual live entry paths and journals successful creates
at the transport, including objects subsequently deleted or left by failure. The
plugin and binary test targets record their own lane telemetry. The budget report
reads those records without replaying journeys; missing or incomplete journals fail,
rather than substituting zero. Only onebudgetspec compares the registered thresholds.

Fixtures contain synthetic content and schema-validated recorded response shapes,
never captured account listings, credentials or private document bodies. Reports
contain operation counts and lifecycle outcomes, not workspace content. A contributor
run without credentials is a decline, not evidence of a successful real Linear pass.
