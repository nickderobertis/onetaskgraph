# Linear live document lifecycle

Linear retains deleted documents in trash. Deletion retires test artifacts; it does
not restore the team's document quota. The nominated `LINEAR_WRITE_TEAM` is the only
team these journeys may mutate. Never replace that nomination by discovering an
arbitrary writable team.

Ordinary affected CI passes reuse standing document slots rather than creating
documents: filed, loose and follow-up probes in the plugin lane, and the asset
destination in the binary lane, plus the parent project the filed probe needs. The team
holds a **pool** of 16 such sets (`live_artifact::POOL_SIZE`), each titled with its
member (`retained-v1 ci <TEAM> <slot> pool-<n>`), and every CI run **leases** one member
per lane for its whole session, so any number of runs up to the pool's size execute the
lanes at once, each over documents no other run reads, writes or counts. No run waits on
another, and no workflow serialises them. Each invocation resets its member's mutable
contents to known run-specific values before asserting updates, reads, filtering,
metadata, Markdown, image integrity and unchanged recopy.

A lease is a Linear issue in the nominated team
(`lease-v1 <TEAM> <lane> pool-<n> gen-<g> <holder stamp>`), created under an id derived
from the member and its generation (`tests/document_lifecycle/lease.rs`). Linear refuses
a create under an id it already holds, trashed or not, so of every claimant of one
generation exactly one wins, and which one did is read back off the issue, never inferred
from the create's answer. Every claim in CI then asks Linear to create that id once more and
requires the refusal, and the lease to read back exactly as claimed, before anything is done
over the member, so a Linear that stopped refusing would fail the lane rather than let two
runs share a member. The next generation may be claimed once the newest is released
— every lane releases on its way out, failures included — or was claimed more than six
and a half hours ago: no GitHub-hosted job runs longer than six hours, so such a lease
belongs to a run that has ended. A claim that finds every member held is refused, naming
the pool; it is not queued. The claimant that supersedes a generation deletes it, and
that id is never asked for again because generations only rise. A CI lane records each lease it holds in
`ONETASKGRAPH_LINEAR_LEASE_DIR`, and a cancelled job's last step releases exactly those
(`onetaskgraph-live-janitor --linear --release-leases`), each only while it still reads back
as recorded, so a cancellation does not keep a member for six and a half hours.

The pool bounds the team's documents: 16 members of 3 plugin documents and 16 of 1 asset
document, 64 of the 1000 the team may hold with its trash, created once per member and
never again by an ordinary pass. An ordinary CI pass that finds its leased member absent
or incomplete initializes exactly that member's missing slots, and its journal files those
creates as a cold rebuild: reported as detail, never counted in the ordinary-pass total and
never read as zero-cost reuse. A warm ordinary pass creates none. The operator fills the
whole pool once with `just linear-document-pool initialize` (`dry-run` prints what it would
create and sends no mutation), which skips a member a live run holds and creates nothing on
a second run. Either way only missing slots are created; partial initialization discovers
existing objects by exact identity. An uncertain create acknowledgement requires discovery
before any retry. Unknown, duplicate or ambiguous ownership fails closed. A retained marker
alone never authorizes a write: exact recognized ownership and verified nominated-team
membership are both required.

Each lane's own Linear traffic passes its run's namespace guard
(`tests/document_lifecycle/namespace.rs`): it learns, from every answer it relays, which
records carry another run's identity — another member, another holder's lease, another
run's stamp — and refuses before sending any request that names one by id or writes one's
identity into a title. A lane whose guard refused anything fails.

The same workflow runs one scheduled fresh-creation pass each day. It exercises the
original filed and loose writes, follow-up metadata creation, and first document
asset copy, with the original deletion/absence assertions and failure cleanup.
This deliberately moves fresh-creation coverage to the daily schedule; ordinary
selected journeys still run their non-creation assertions. The assertion-by-assertion
mapping is in [the coverage inventory](linear-document-assertion-inventory.md).
Issue, label and state coverage remains in the live journeys.

The document mode environment variable is
`ONETASKGRAPH_LINEAR_DOCUMENT_MODE`: `ordinary`, `fresh`, or `initialize`.
No workflow holds a concurrency group over the live lanes, and
`.github/linear-documents-policy/policy.test.mjs` fails if one is reintroduced.
Machine-local Session exclusivity remains an additional guard on one machine.

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

Standing CI slots, their parents and pool leases survive normal cleanup, cancellation
residue cleanup, local orphan sweeping and the hourly janitor regardless of age; the
pre-pool standing set (`retained-v1 ci <TEAM> <slot>`) is still recognized and protected,
and no lane uses it. Fresh-mode
artifacts remain disposable with stamped ownership and expiry safeguards. The janitor
must continue protecting unrelated, fresh, malformed, ambiguous and unvouched records.
Never infer ownership from a similar title or treat trashing as quota recovery.

## Per-pass measurement and privacy

The ordinary budget measures one warm affected CI pass of both lanes: zero successful
`documentCreate` operations. The daily budget measures one fresh pass of both lanes:
three plugin documents and one binary asset document. Retries and pass frequency do
not weight either figure. A cold rebuild of the standing set, whether by an ordinary
CI pass or the explicit initialization, and failed-pass residue are separate diagnostic
detail, not part of either pass total.

Offline HTTP replay drives the actual live entry paths and journals successful creates
at the transport, including objects subsequently deleted or left by failure. The
plugin and binary test targets record their own lane telemetry. The budget report
reads those records without replaying journeys; missing or incomplete journals fail,
rather than substituting zero. Only onebudgetspec compares the registered thresholds.

Fixtures contain synthetic content and schema-validated recorded response shapes,
never captured account listings, credentials or private document bodies. Reports
contain operation counts and lifecycle outcomes, not workspace content. A contributor
run without credentials is a decline, not evidence of a successful real Linear pass.
