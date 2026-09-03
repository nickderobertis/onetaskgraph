<!-- llmlint: ignore-file[agents_md_durable_and_terse] every section this rule reads as
     non-terse is a required property of this repository rather than an authoring choice:
     the create-repo baseline checker fails when the composition is missing, and this
     repository's own acceptance criteria require it to record the enforcement mechanisms,
     the three affected selections and the journey inventory here. Tightening the wording
     is tracked as follow-up; removing the content is not available. -->
<!-- llmlint: ignore-file[instruction_layer_localized] the nested-`AGENTS.md` half of this
     rule is met — `crates/AGENTS.md` carries the crate-subtree rules and this file keeps
     the repo-wide ones — but its `CODEOWNERS` half asks for something this repository has
     decided against. "Commits, releases, and merging" below records zero required
     approvals *because nobody reviews these pull requests*, and GitHub does not request
     review from a pull request's own author, so a `CODEOWNERS` naming this repository's
     one owner would route nothing to nobody. Adding one would also fork the merge-path
     arrangement away from its stated source of truth, the create-repo skill's
     `setup_github_governance.py`. If review ever becomes something that happens here,
     delete this directive and add the file. -->
# AGENTS.md

Durable constraints for humans and agents working here. Terse on purpose — this is
always-loaded context, so it carries what the commands, the code and the gate cannot say
for themselves. `CLAUDE.md` is a symlink to this file; edit only this one.

## What this repo is

One interface over the ticketing systems the user's work lives in (Linear, GitHub
Projects, local Markdown), shipped three ways from one engine: a Rust CLI, a Python SDK,
and a TypeScript SDK.

## Stack and composition

- **Product shape:** `cli`.
- **Language(s):** Rust (the engine, the plugins and the binary), Python (the SDK and the
  maturin-wrapped CLI distribution), TypeScript (the SDK and the npm launcher).
- **References composed (11):** `base.md`, `shapes/cli.md`, `languages/rust.md`,
  `languages/python.md`, `languages/typescript.md`, `intersections/rust-cli.md`,
  `intersections/python-cli.md`, `ci.md`, `llmlint.md`, `releasing.md`, `monorepo.md`.
- **Excluded, and why:** web-app / React / Next.js (no web surface), asdf-plugin
  (distribution is crates.io, PyPI and npm), skills-repo (this is a product, not a skills
  collection). Nothing non-negotiable is excluded.
- **Nx is a deliberate inclusion**, and the reason is the user's own: *affected selection
  is the gate, so a change touching one plugin does not run the other plugins' tests.*
  With seven crates and two SDK packages over three toolchains that is the difference
  between a usable pull-request cycle and an unusable one. Putting Node on the critical
  path of the Rust gate is the accepted cost and is not reopened.
- **`onetaskgraph-plugin-api` exists for that same reason.** With the trait inside the
  engine crate, every plugin would depend on the engine, every engine change would mark
  every plugin affected, and affected selection would buy nothing for the six crates where
  it matters most.

## The plugin contract

It lives in **two** crates, and which one a type is in is the contract, not a detail.

This inventory is not prose alone: `scripts/check-contract-inventory.sh` reconciles it
against the api crate's real exports on every `check`, so the two cannot drift apart in
silence. See the note on `Health` below for the one difference it carries deliberately.

- **`onetaskgraph-plugin-api`** — exactly what a plugin author needs, and nothing else:
  the traits `TaskSource`, `SourcePlugin` and `SecretResolver`; the work types `Task`,
  `Project`, `Document`, `Location`, `Label`, `Status`, `StatusCategory`, `Repository`,
  `DependencyEdge`, `DependencyEndpoint`, `ItemKind`, `DependencyKind`,
  `Direction`, `NativeId`, `SourceName`; the query and paging types `TaskQuery`,
  `ProjectQuery`, `DocumentQuery`, `TextQuery`, `TextFields`, `LabelFilter`,
  `ProjectFilter`, `PageRequest`, `Page`, `Cursor`; the capability types `Capabilities`,
  `Support`, `DependencySupport`; the write types `ItemWrite` and `WriteSupport`; and
  `SourceError`.
  **It depends on no other crate of this workspace.**
- **`onetaskgraph-core`** — the engine, plus the reporting types `QueryResponse`,
  `QueryPlan`, `SourcePlan`, `Predicate`, `PageToken`, `SourceFailure` and `GlobalId`.

A type needed on both sides goes in the api crate, because the dependency only runs one
way. `NativeId`, `SourceName`, `Page`, `PageRequest`, `Cursor` and `SourceError` are there
although the engine handles all six: each appears in a trait signature. `GlobalId` is
**not**, deliberately — a plugin never sees a qualified id.

**Keep the api crate still.** Every change to it rebuilds and re-tests every plugin. When a
new type could sit on either side, put it in `onetaskgraph-core` unless a trait signature
names it.

> **Open contract question — `Health`.** The approved enumeration of the api crate's
> contents is exhaustive and does not name `Health`, but `TaskSource::health` returns it
> and that trait is in the api crate, so the enumeration and the trait as written cannot
> both stand. It sits in the api crate today because that is the only placement that
> compiles; see the note on the type. Resolving it — add it to the enumeration, or
> redesign `health` so no such type crosses the boundary — belongs to the contract's owner.
> Do not read the enumeration's silence as an answer.
>
> The disagreement is open, not unwatched. `scripts/check-contract-inventory.sh` — a target
> in `check` — reconciles the inventory above against what the api crate really exports and
> fails on any difference, carrying `Health` as one named exception with this reason. So a
> type added on either side without the other fails in seconds, and settling this question
> means deleting that exception and this note in the same change rather than quietly
> editing a bullet.

### The three capability rules

Every implementation of `TaskSource` is bound by all three, and the engine's compensation
is only correct while they hold:

1. A plugin **applies** every predicate it declares `Native`.
2. A plugin **ignores** every `Support`-typed predicate it declares `Unsupported` — it
   returns the *wider* result set, never a narrower one. Silently dropping rows for a
   predicate it did not declare is the one failure no test above the plugin can catch.
   This reaches the `Support`-typed predicates alone. A corollary the in-memory source
   proves: a predicate a source can only *half* apply (a `title-or-content` search where
   only titles are native) must be ignored outright, because half-applying it narrows.
3. The engine compensates transiently — at most one source page plus the caller's page,
   nothing written down. For a `ForwardOnly` source it answers `DependedOnBy` by a bounded
   page-by-page scan and reports it as emulated. A dependency read is never ignored and
   never silently empty.

`Capabilities.documents` is `Support`-typed and is nevertheless **not** one of the
predicates rule 2 reaches. It says whether the source has documents at all, in the shape
`projects` uses, so there is no wider result set to return and nothing to compensate: the
engine reads it once at the handshake, exactly as it reads `writes`, never asks a source
declaring `Unsupported` for a document, and reports such a source as holding none rather
than as having failed. A source with no documents therefore *refuses* a document read
rather than answering an empty page — a plugin says what it cannot do and is then not
asked, instead of degrading into an answer indistinguishable from a source that has
documents and holds none.

A `Document` carries **no status** and **no dependencies**, and both omissions are the
contract: a document is not work, so it has no place in a status filter and none in a
dependency graph. `ItemKind` therefore gains no document variant — that enum names what a
dependency endpoint points at, and nothing may point at a document. `Location` is where an
entity is, as a link (`{"url": …}`) or as an absolute path on the machine the source runs
on (`{"path": …}`); it sits on `Task`, `Project` and `Document`, defaults to absent, and
does not touch, replace or derive from the `url` field those types already carry.

`DependencySupport` has no unsupported variant on purpose: dependency traversal is a
guaranteed capability of this product, not one a source may opt out of. `Support` — which
governs the filters and the searches — is a different enum and keeps its `Unsupported`
variant, because in-memory compensation for those is sound. Do not conflate the two.

## Invariant: no work data outside a plugin

Nothing of a user's work is stored, cached, indexed or mirrored outside the plugin that
owns it. The engine compensates transiently and writes nothing down. Extend these three
mechanisms rather than rediscovering the rule; each says how it fails, not how it works.

**A destination write is not a cache, and the difference is stated rather than assumed.**
A destination write is at the user's explicit request, names its destination, goes through
that source's own write interface into that source's own store, and is never read back to
answer a query. A cache is a write nobody asked for that the engine reads back. The
invariant above does not move: `copy` writes *into a plugin*, which is the one place a
user's work is allowed to be. The same terms settle a cross-source dependency edge —
storing the far end's qualified id on the near item, inside the plugin that owns it, at
the user's explicit request, is not the state `DependencyEdge`'s own documentation
forbids, because the engine holds nothing between calls and reads nothing back to answer a
later query.

**A copy is either complete or it never happened, and undoing one is not a delete verb.**
A copy that cannot finish removes the items *it created in that run* and writes back the
items it overwrote, through `TaskSource::delete_task` and `TaskSource::delete_project`.
Nothing a user types deletes anything, and no item this run did not write is ever touched:
what is undone is this engine's own writes, from a journal that lives for the length of one
`copy` call and is dropped with it. The reason is not tidiness — a half-written project has
to be run again, and the re-run is the mutation burst that trips a hosted destination's
secondary rate limiter, which then refuses even reads for the next fifty minutes. A
destination that will not take one of its items back is not papered over: the copy refuses
with `EngineError::CopyNotUndone`, naming why it failed, why it could not be undone, and
what is still there.

1. `deny.toml` refuses every embedded store, index and cache crate, and `deny` is a
   required check — so reaching for one cannot merge.
2. `crates/onetaskgraph/tests/e2e/no_persistence.rs` sandboxes `HOME`, every `XDG_*` and
   `TMPDIR` into one tree, plants sentinels in a source's work, drives every verb, and
   compares the tree with itself: it fails, naming the path, if any file was created or
   changed during the run, and says which sentinels a new file held. It asserts on the
   effect, so it catches caching by any technique. It drives `copy` too, which is the one
   verb that writes: the named destination's own store is the only place a file may
   change, and every other path in the tree is held to exactly the rule above.
3. `crates/onetaskgraph-core/tests/no_reuse.rs` catches the half a filesystem scan cannot
   see: one query asked twice must reach the source twice.

## Invariant: no plugin crate depends on the engine

Not as a dependency, not as a dev-dependency, at no depth — and the same for
`onetaskgraph-plugin-api` depending on anything of this workspace. Prose does not hold
this; the first worker who wants one helper out of the engine will reach for it. So
`deny.toml` permits the engine exactly one wrapper (the binary, failing the required
`deny` job) and `scripts/check-plugin-isolation.sh` reads the real `cargo metadata` graph
inside `just check`, failing in seconds instead of minutes.

Both are watched failing rather than trusted: `scripts/check-isolation-enforced.sh`
introduces the forbidden edge in a scratch clone — as a dependency, as a dev-dependency,
through an intermediate crate at depth two, and from the api crate — and asserts on both
the refusal and the diagnostic. Its fifth case is `cargo deny` refusing a dev edge to the
engine, because a *normal* edge is a Cargo cycle that cargo rejects before `deny` runs, so
the wrapper restriction is what actually stands between a dev-dependency and a merge. A guard nobody has
watched fail is a guard nobody knows works, and two of this one's cases exist because it
was wrong in exactly those ways. Keep its cases; extend them when the rule grows.

`scripts/check-hook-safety.sh` guards the guards: they clone, the gate runs from a hook,
and git exports `GIT_DIR` to hooks, where it overrides `git -C`. Both clone through
`scripts/scratch-clone.sh` for that reason.

## The three selections the project graph owes

The split buys exactly one thing and it is a build-graph thing, so the graph is not
correct until it *selects* this way. `scripts/check-affected-selection.sh` proves it
against real Nx; reasoning about the configuration does not count.

1. Editing `onetaskgraph-plugin-api` selects **every** plugin crate.
2. Editing `onetaskgraph-core` outside it selects **no** plugin crate. This is the return
   on the split and the one that fails silently — an over-broad `implicitDependencies`
   entry or a too-wide `namedInputs` glob makes every engine commit run every plugin's
   tests, and nothing complains.
3. Editing one plugin selects that crate and its dependents — never a sibling plugin.

Nx cannot read a Cargo manifest, so each Rust `project.json` mirrors its crate's Cargo
dependencies as `implicitDependencies`; `scripts/check-nx-graph.sh` fails when the two
disagree either way, because a missing edge under-runs the gate and an extra one over-runs
it.

## Tests

The suite is the only QA loop; realism and completeness are rules, not preferences.

- **Never mock the layer under test.** Every journey drives the compiled binary as a
  subprocess and asserts on exit code, stdout and stderr.
- **The copy verb is proven twice, deliberately.** The journeys drive the binary the way a
  user does, and `crates/onetaskgraph-core/tests/copy.rs` drives the engine's own
  `Engine::copy` as a library call. The second is not a duplicate: this product is exposed
  three ways from one engine, and the consumer a command-line-only copy would strand is
  the Rust caller that links the crate.
- **A document copy is proven against a destination that outlives the invocation.** One
  command line is one process, so an `in-memory` destination cannot be read back by a later
  command and `local-md` declared, when this was written, that it holds no documents —
  which once left the document copy observable only through the report it printed.
  `onetaskgraph-document-store` closes it: a file-backed peer spawned over a real pipe, so
  what one invocation writes the next one reads. **It is Python, and that is not
  incidental.** A spawned plugin can never be measured — the engine clears the child's
  environment (§3.1), which takes `LLVM_PROFILE_FILE` with it, and
  `bin/onetaskgraph-source.rs` sits at 0% for exactly that reason — so a peer of any size
  written in Rust would be permanently uncovered lines in the binary crate. Being another
  language also makes the journeys test the seam's actual claim, that a plugin can be
  written against the protocol document alone, rather than restating the engine's own half
  back to itself. `python3` is already what every guard under `workspace:lint` runs on all
  three platforms, so it costs the gate nothing. **Do not replace that round trip with a
  destination pre-populated as the copy would leave it and an assertion that nothing
  changed.** That proves what a copy *would* write; it does not prove one landed, and the
  difference is the whole of what the journey is for.
- **Coverage: 95% lines, per project, and each project measures only its own crate.**
  A workspace average lets a weak crate hide behind a strong one — and, decisively, a
  workspace-wide pass runs every crate's tests on every change, which is what affected
  selection exists to avoid. The *measurement* is skipped on Windows with a printed
  notice (instrumentation there does not attribute subprocess coverage); the functional
  lanes still gate that platform.
- **The tests that reach a real API are ordinary tests, and that reverses an earlier
  decision.** They were a separate `test-live` target on every project, run by a workflow of
  their own on a schedule and on every pull request, deliberately outside the required set —
  because a required check a third party can turn red is a check that stops being trusted.
  **That arrangement is what let a query GitHub refuses outright merge:** the advisory lane
  failed on the pull request that introduced it, with GitHub's own node-limit error, and
  auto-merge squashed it anyway, because auto-merge waits only on *required* checks and
  nobody was there to read a red advisory one. So there is now no live workflow, no live
  job, no live target, no live recipe and no `#[ignore]`: change the GitHub Projects plugin
  and its GitHub tests run in the required check and can fail it, change the Linear plugin
  and its Linear tests do, change the engine or another plugin and neither runs, because
  affected selection does not select them. `scripts/check-live-lane.sh`, a target in
  `check`, asserts the whole of that shape so no part of it can be quietly undone.
  **A third party being down now blocks a merge, and that is the accepted cost** — the
  answer is to re-run once it answers, never to bypass the check, and never to reinstate a
  signal the merge does not wait on. It is a different outcome from a session that
  *declined*: an outage makes a run that happened fail, and a decline is a run that never
  happened, which says so on its face and is not read as a defect in the code.
- **One gate, because the next precondition has to govern every path.** A live test cannot
  hold its credential except from `onetaskgraph_live::Session::open`
  (`crates/onetaskgraph-live`), which runs every precondition first and hands the credential
  back only if they pass. That is where a precondition governing every path by which these
  tests reach a real API is added — one place rather than one per lane — and
  `check-live-lane.sh` fails when a journey stops opening it. Its three answers are not
  interchangeable: **run**; **skip**, when a credential or a nomination was not given and
  none was expected, which is a contributor with no keys and a pull request from a fork,
  where the host supplies no secrets; and **declined**, when the session could have run and
  a precondition refused it. A decline is not a pass — it fails the run, so the required
  check concludes neither success nor any conclusion (`neutral`, `skipped`) branch
  protection accepts in place of success — and its message leads with the tests not having
  run, so a refusal is not read as a code defect.
  `ONETASKGRAPH_LIVE_REQUIRED=1` turns a skip into a failure, which is what stops the
  required lane passing green merely because a credential went missing where one was
  expected; `.github/workflows/ci.yml` sets it everywhere but a fork pull request.
  `scripts/check-live-decline.sh`, a command in each hosted plugin's own `test` target,
  drives all three outcomes without a credential and without reaching the API.
- **A live session that the account cannot afford declines, and that is the second producer
  of the third outcome.** The share it may never touch is `RETAINED_BUFFER` in
  `onetaskgraph-live` — twenty per cent of each budget's **whole allowance**, one constant
  for every lane, and nothing lowers it — and `affordable` there is the arithmetic. What
  belongs to a lane is the *read*: the GitHub Projects journey makes exactly one,
  `GET /rate_limit`, which GitHub documents as not counting against the REST rate limit and
  which answers both of its budgets, before it does anything else. The estimate it decides
  on is derived offline from that plugin's own per-call record of the session and a cost
  model stated once in `tests/journey/budget.rs`, so it needs no credential and moves with
  the session rather than with an edit; node count is **not** that estimate and cannot stand
  in for it. `scripts/check-budget-decline.sh`, a command in that plugin's `test` target,
  follows such a decline through to the conclusion the required check reads, because a test
  that asserts a panic passes and the half worth proving is that the check goes red.
- **A test that reads and writes a shared external fixture must not run concurrently with
  another instance of itself.** That is a general property of this repository rather than an
  exemption for two plugins, and the reason is the self-healing half of these journeys: each
  sweeps residue by title before it starts, and that sweep recognises *any* run's artifacts,
  so two concurrent runs delete each other's in-flight items. Concurrency here is a
  correctness problem, not a cost one. Three things hold it, and **the second is the one a
  fold that stops at the test target gets wrong**: the session's own seat, which declines a
  second instance on the same machine; `scripts/rust-coverage.sh` clearing the credentials,
  because `just check` performs the affected `test` target **and** the affected `coverage`
  target, and coverage is `cargo llvm-cov --package <crate>`, which re-runs the very same
  integration tests — so a fold that stops at `test` opens a second session per lane; and
  `.github/workflows/ci.yml` handing the credentials to exactly one leg of its three-platform
  matrix, so the count is one session per run rather than six. A run of the required check
  opens at most one session per journey. If you are changing the matrix or the coverage
  target, that trio is what has to stay true, and the note in `rust-coverage.sh` says it
  where you will meet it.
- **A live lane that writes names what it writes to.** The GitHub Projects lane takes its
  board from `GH_PROJECTS_OWNER` and `GH_PROJECTS_NUMBER` and the repository it creates its
  issues in from `GH_PROJECTS_REPOSITORY` — a project there is an issue and a board has no
  repository of its own — all three being required inputs of it alongside
  `GH_PROJECTS_TOKEN`, and skips with a printed reason when any is absent —
  `ONETASKGRAPH_LIVE_REQUIRED=1` turning that skip into a failure, the same pairing the
  credential has. It never asks GitHub which project was updated most recently: that rule once
  retargeted the credentialed lane from the fixture board onto the board plans are authored on.
  Requiring the board to be named is what keeps the lane off a board nobody nominated. The
  lane's separate sweep of items titled the way it titles its own artifacts is self-healing
  after an interrupted run — it recovers residue a killed process left behind, and it is not
  what bounds where the lane may write. **The Linear lane names its scratch team the same
  way**, through `LINEAR_WRITE_TEAM`, which is the `LINEAR_WRITE_TEAM` repository variable
  on this repository — a variable rather than a secret, because a nomination has to be
  readable for anyone reviewing where a credentialed write may land, and it names the
  scratch team the operator set aside for it. A run that reaches that lane without the
  nomination fails naming the variable and how to set it, rather than skipping: a nomination
  is not a credential, and collapsing the two would rebuild, one level in, the silent skip
  this arrangement exists to remove.

### The journeys this repository owes

The Linear row is backed by a real local HTTP server in the shared e2e process. Its
responses and the crate fixtures under `crates/onetaskgraph-linear/tests/fixtures/`
follow Linear's published GraphQL schema and Relay connection documentation as recorded
on 2026-08-24, `projectDelete` as re-observed there on 2026-08-29, and the whole document
contract — `document`, `documents`, `Document`, its connection, filter, two inputs and both
payloads, and the three document mutations — as re-observed there on 2026-09-01. The checked-in `schema.graphql` there is the pinned source artifact, and
`pinned_schema_checks_selected_fields_arguments_and_fixture_keys` fails when a production
operation drifts from it. Each fixture records whether it was live-captured or
documentation-derived.

Each drives the real binary as a subprocess, and each runs against **every** configured
source kind through one shared fixture table — `crates/onetaskgraph/tests/e2e/fixtures.rs`
— so a plugin is never proven by a suite of its own writing. That coverage is not a habit:
`scripts/check-journey-matrix.sh`, a target in `check`, reconciles the table against the
registry both ways and fails naming the plugin, so a plugin that lands without a row
cannot merge. A plugin whose source has not landed carries a `Pending` row, which is a
journey of its own — it asserts that plugin refuses with its own message — rather than a
placeholder.

The list grows as features land, and the suite is what says which of
them do; this is the inventory of what is owed, not a status board.

1. List tasks from one source.
2. List tasks from several named sources at once, and from every configured source.
3. Show one task by its qualified id.
4. List projects; show one project.
5. A task belonging to no project is listed by default and selected on its own.
6. List labels across sources.
7. Filter tasks by one label, by several at once, and by exclusion.
8. Filter tasks by status category.
9. Search by title only, by content only, and by either — over tasks and projects.
10. Task dependencies forward and reverse — reverse against a `BothDirections` source, and
    again against a `ForwardOnly` one where the bounded scan answers and the plan says so.
11. Project dependencies, forward and reverse, both ways again.
12. A source declaring a predicate unsupported still returns the correct rows, and the plan
    names that predicate as applied locally.
13. A source declaring a predicate native has it pushed down and the plan says so — one
    query against two sources of differing capability, one correct answer, two plans.
14. One source failing leaves the others' results intact, names the failure, and exits
    non-zero unless partial results were allowed.
15. Paging: a limit smaller than the result set walks to exhaustion in a stable order.
16. Configuration precedence: file; environment over file; flag over environment; and the
    effective-configuration verb naming which layer each setting came from.
17. A field of one **named source** set at each of those three layers in turn.
18. The secrets file supplies a variable a config names, and does not override one the
    process environment already defines.
19. Every journey above, again, through a subprocess-wrapped source.
20. The no-persistence sentinel journey.
21. Machine-readable output validates against the schema the binary itself emits.
22. Failure and recovery: unknown source name, malformed configuration, unknown id, and an
    unreachable source each exit non-zero with the problem and a suggested next action on
    stderr.
23. Caller-defined metadata and repository origins come back out of every source kind with
    their JSON types intact, and the keys this product reserves are read while every other
    key is passed through untouched.
24. A dependency edge that leaves the source — across projects, across the task and project
    levels, and across sources — is reported by qualified id and item kind, is never
    reported in reverse, and is never followed to the far source.
25. One relationship reads the same from either end: `from` is the item that depends,
    whichever way a backend spells the relationship, so a source that stores it from the
    blocking side reports the same edge rather than its mirror.
26. A reserved-key far end the near item's own backend could have named is refused, naming
    the entry and what to record instead; so is one this interface cannot represent.
27. A task copies out of every source kind into a folder of Markdown with every field it
    read intact, and a second copy of the same item updates that one rather than
    duplicating it.
28. The round trip: a task is copied out of a destination, the Markdown is edited, and the
    copy back updates the item it came from — exactly one item where there was one before,
    the edited field changed, and every field the edit did not touch byte-for-byte what it
    was.
29. Every refusal a copy owes: a destination configured with no write side, one that
    cannot carry a metadata key, an origin naming an item the destination no longer holds,
    and an id or a destination nothing configures. Plus the escapes — `--recreate`,
    `--match-by` — and `--dry-run`, which reads everything and writes nothing.
30. A project copy carries the tasks in it, matches each independently on a second copy,
    and reports a destination item the source no longer holds as orphaned rather than
    deleting it.
31. A copy that cannot finish leaves the destination as it found it: an item's write is
    made to fail after another item has already landed, and the destination afterwards
    holds none of that copy's items. A destination that will not take one back is refused
    with both halves of the reason and the name of what is still there.
32. A copy resolves a dependency on an item it created earlier in the same run — including
    one in another project of the same command, and against a destination whose own read
    of itself is behind. No item a copy created is ever reported as not found.
33. Documents are listed, narrowed to a project, selected on their own when they are in
    none, filtered by label and by exclusion, and searched by title, by content and by
    either; one is shown by its qualified id, and a limit smaller than the result set walks
    to exhaustion in a stable order. `document list` has no status filter and `document`
    has no dependency verb: both are refused as invocations rather than accepted and
    ignored, because a document is not work.
34. A source declaring it has no documents is reported as holding none rather than as
    having failed — a document list spanning one exits zero with the plan naming the
    predicate unavailable, and a document copy naming it at either end is refused before
    anything is read, naming the source and its plugin.
35. Both renderings report where an entity is, for documents, tasks and projects alike: the
    human rendering says which kind of place it is, the machine rendering carries the
    contract type's own JSON so a consumer branches on which key is present, and a location
    the source did not give is absent rather than a third variant.
36. A document copies into another document-bearing source with every field and every
    caller-defined metadata key it was read with, its JSON types intact, and a second copy
    of the same document updates the one already there rather than adding a duplicate.

## Recorded decisions

- **The SDKs drive the real binary as a subprocess.** They do not reimplement the engine
  and do not link it. One implementation of the query semantics means the CLI, a script
  and a TypeScript application cannot answer the same question differently — which is the
  whole reason this repository exists. The cost is a process spawn per call.
- **The stdio plugin protocol is the named upgrade path.** If a longer-lived session is
  ever wanted — to amortise the spawn, or to hold a connection open — the answer is the
  JSON-over-stdio seam the contract is already shaped for (`SourceError` carries owned data
  only for exactly this reason), not an in-process binding that would fork the engine.

## Commits, releases, and merging

- **Squash-merge only, via pull request, with auto-merge.** One PR is one commit whose
  subject is the PR title and whose body is the PR description. Queue it with
  `gh pr merge --auto --squash`.
- **Auto-merge proceeds on a required check that concludes success — and on any conclusion
  the required check accepts *in place of* success, which is the same hole under a gentler
  name.** `neutral` and `skipped` read as "did not run" and satisfy branch protection
  anyway. Everything about how a signal that never ran must conclude rests on this, and so
  does why an advisory check cannot substitute for a required one: a query GitHub refuses
  outright reached the default branch because auto-merge waited only on the required set
  while the advisory lane that caught it sat red.
- **Six required checks — `check` on all three platforms, `deny`, `llmlint`, `pr-title` —
  and they are the only thing that can refuse a merge here.** Re-apply or verify the whole
  arrangement with the create-repo skill's `setup_github_governance.py`, which is its
  source of truth. Two values in it are load-bearing rather than conventional: **zero
  required approvals**, because nobody reviews these pull requests and any other value
  makes auto-merge wait forever; and **`strict`**, so a branch is up to date before it
  merges. Adding a required check costs a change in the orchestration repository too,
  which checks its inventory of this merge path against GitHub on every gate run.
- **Bump policy (pre-1.0):** `feat` → minor; `feat!` / `BREAKING CHANGE` → minor (a
  breaking change before 1.0 is *not* a major); `fix` / `perf` / `refactor` / `build` →
  patch; `chore` / `docs` / `ci` / `test` / `style` → no release. Post-1.0 a breaking
  change becomes a major. The PR title is what the release tool parses, which is why
  `pr-title` is required.
- **Release driver: `release-plz`,** in its release-PR shape: the bot accumulates
  unreleased commits into a PR, merging it cuts the release, and the tag fires build and
  publish. It must authenticate with `RELEASE_PLZ_TOKEN` — a tag pushed by the default
  `GITHUB_TOKEN` fires no workflow, so the release would silently ship nothing.
- **The release PR is prepared by `scripts/prepare-release-pr.sh`, never by
  `release-plz release-pr` alone.** release-plz writes the Cargo manifests and nothing
  else — no `package.json`, no `pyproject.toml`, not even the version under
  `[workspace.package]` — and it has no hook that could: the pinned release-plz refuses
  `pre_release_hook` as an unknown field and its own `generate-schema` lists no other. So a
  pull request it opens by itself fails `distribution-check`, one of the required checks its
  own merge waits on, and no release is cut at all. The script bumps with `release-plz
  update`, brings every other manifest to the version it chose with `scripts/set-version.sh`,
  and hands the whole tree to `release-pr --allow-dirty`, whose uncommitted changes become
  the release commit. `scripts/check-release-pr-sync.sh` drives that end to end on every
  `check` and refuses a workflow that goes around it. It stands in for release-plz rather
  than installing it — a required check must not depend on crates.io — so the workflow pins
  the release-plz version and that check fails when the pin and the version the stand-in was
  recorded from part. Moving the pin means re-observing the real tool.
- **Registry lag alone never proposes a release.** `scripts/select-release-version.sh`
  recovers a partly failed publish only when `release-plz.toml`'s own `release_commits`
  policy — the single declared one, read rather than restated — accepts a commit since the
  release boundary. The lag itself says nothing: no registry can hold a version this
  pipeline tagged seconds earlier, so every push that merges a release pull request looks
  exactly like a publish to recover. Acting on the lag alone released v0.2.4 and v0.2.5
  from no source change at all, and auto-merge fed each proposal back in as the next push.
  `scripts/check-release-tooling-selection.sh` and `scripts/check-real-release-preparation.sh`
  drive both decisions, the second against the real pinned tool over a real checkout.
- **`release-targets.toml` is what a consumer waits on, and its short names are frozen.**
  It declares every artifact this repository publishes in the canonical schema `onevcs`
  defines and reads, because a dependent that has to guess a release happened either
  launches too early or waits by hand. The set is frozen at five — `crate`, `pypi`,
  `sdk-pypi`, `npm`, `sdk-npm` — because that is how another repository names one of these
  and it cannot see this file to notice one moved or a sixth appeared. A name is one of
  the five when a dependent writes it down, which is why both SDKs are targets and neither
  is folded into the binary's: `sdk-npm` is to `npm` what `sdk-pypi` is to `pypi`.
  Everything else this repository publishes — the five per-platform npm carriers, the six
  sibling crates — is covered by the target whose release carries it. Both rules go
  through `scripts/check-release-targets.sh`, the drift gate that makes the second
  spelling safe and that reconciles the declared set against the real release workflow
  both ways.
- **The probe's three answers are not interchangeable.** `scripts/release-probe.sh` prints
  a version, prints nothing when that registry serves none, or exits non-zero because the
  lookup could not be made — and a consumer holds indefinitely on the third and stops
  holding on the second, so reporting a failed lookup as "nothing published" is the one
  thing it must never do. It is a probe rather than a gate: it rules on no change. Each
  registry's request and the field read out of the answer are pinned with their provenance
  in `config/registry-interfaces.toml`; `scripts/check-release-probe.sh` holds the probe to
  that pin and drives all three answers against documents built from it, so what makes
  those answers evidence is the pin rather than a registry being reachable, and
  `crates/onetaskgraph/tests/live.rs` re-observes the real registries as an ordinary test of
  the binary crate — inside the required check, like every other test that reaches something
  real.

## Conventions

- **Scripts are context.** Quiet on success; on failure print the exact problem and a
  concrete next action.
- **They run on bash 3.2.** macos-latest ships it, and every script here declares
  `#!/usr/bin/env bash`, so on that runner each one IS a 3.2 script — which is why
  `mapfile` and `readarray`, one bash 4 builtin under two names, are refused by
  `scripts/check-bash4-array-builtins.sh`: reaching one there aborts the script with
  `command not found`, which reads as whatever it was proving having gone wrong rather than
  as a portability failure. Write `read_lines` from `scripts/read-lines.sh` instead.
  `scripts/check-bash4-array-builtins-enforced.sh` watches that guard refuse both spellings
  in every command position, and pass the name in a comment. Every path that guard names
  goes through one spelling, forward slashes, on all three platforms — python renders a path
  with the running platform's separator, so before it was normalised the same guard reported
  `scripts\check-distribution-contract.sh` on the Windows runner and failed every assertion
  written against the other two. `scripts/check-guard-path-spelling.sh` drives the guard and
  that enforcement again through a python whose paths spell with a backslash, so the lane
  that can fail on this is not the only lane that can catch it.
  The other 3.2 difference that bites here: a `source` whose file is missing ends that
  shell outright, so the handler after `||` never runs and the script says nothing about
  what to restore. Test the file, then source it. `scripts/check-line-reads.sh` drives every
  such load twice, the second under `set -o posix` — 3.2's behaviour in a bash the Linux and
  Windows lanes have, so a defect only macOS could otherwise report fails all three.
  `just script-check` runs them outside Nx, because Nx maps no project to `scripts/` and so
  selects nothing for a change that only edits one.
- **Suppress narrowly.** A diagnostic is an error or a suppression at that one site with a
  stated reason. `notignored` posts every suppression a PR adds, so they are read.
- **`gh-secrets.json` is tracked and load-bearing.** It declares the repository secrets
  this build needs and its only destination is the GitHub repository — it writes no local
  `.env` and is unrelated to `~/.config/onetaskgraph/secrets.env`, which is hand-managed.
  Its `RELEASE_PLZ_TOKEN` is sourced from the Bitwarden item named `GH_TOKEN` on purpose.
  Do not rewrite, regenerate or reformat it.
- **Keep the allowlist current**, and keep it to the command surface. `.claude/settings.json`
  grants the `just` recipes, read-only introspection, and `git add -A` / `git commit -m` —
  local and reversible. Every irreversible git operation (push, `reset --hard`, `clean`,
  force-checkout) is deliberately withheld, which is where the privilege boundary is.
