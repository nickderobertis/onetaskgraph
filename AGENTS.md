<!-- llmlint: ignore-file[agents_md_durable_and_terse] every section this rule reads as
     non-terse is a required property of this repository rather than an authoring choice:
     the create-repo baseline checker fails when the composition is missing, and this
     repository's own acceptance criteria require it to record the enforcement mechanisms,
     the three affected selections and the journey inventory here. Tightening the wording
     is tracked as follow-up; removing the content is not available. -->
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

<!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] SECOND PERMITTED REASON —
     this restates at the inventory the justification already recorded ten lines below it,
     in "Open contract question — `Health`", and again at the type in
     onetaskgraph-plugin-api/src/source.rs: the enumeration omitting `Health` while
     `TaskSource::health` returns it is a known, deliberate contradiction in the APPROVED
     contract. Reconciling it — adding `Health` to this list, or redesigning `health` so no
     such type crosses the boundary — is expressly reserved to the contract's owner and
     forbidden to any node of this plan, so no change here can close this finding. -->
- **`onetaskgraph-plugin-api`** — exactly what a plugin author needs, and nothing else:
  the traits `TaskSource`, `SourcePlugin` and `SecretResolver`; the work types `Task`,
  `Project`, `Label`, `Status`, `StatusCategory`, `DependencyEdge`, `DependencyKind`,
  `Direction`, `NativeId`, `SourceName`; the query and paging types `TaskQuery`,
  `ProjectQuery`, `TextQuery`, `TextFields`, `LabelFilter`, `ProjectFilter`, `PageRequest`,
  `Page`, `Cursor`; the capability types `Capabilities`, `Support`, `DependencySupport`;
  and `SourceError`. **It depends on no other crate of this workspace.**
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

`DependencySupport` has no unsupported variant on purpose: dependency traversal is a
guaranteed capability of this product, not one a source may opt out of. `Support` — which
governs the filters and the searches — is a different enum and keeps its `Unsupported`
variant, because in-memory compensation for those is sound. Do not conflate the two.

## Invariant: no work data outside a plugin

Nothing of a user's work is stored, cached, indexed or mirrored outside the plugin that
owns it. The engine compensates transiently and writes nothing down. Extend these three
mechanisms rather than rediscovering the rule; each says how it fails, not how it works.

1. `deny.toml` refuses every embedded store, index and cache crate, and `deny` is a
   required check — so reaching for one cannot merge.
2. `crates/onetaskgraph/tests/e2e/no_persistence.rs` sandboxes `HOME`, every `XDG_*` and
   `TMPDIR`, plants sentinels, drives every verb, and fails if any file written during the
   run holds one. It asserts on the effect, so it catches caching by any technique.
3. `crates/onetaskgraph-core/tests/no_reuse.rs` catches the half a filesystem scan cannot
   see: one query asked twice must reach the source twice.

## Invariant: no plugin crate depends on the engine

Not as a dependency, not as a dev-dependency, at no depth — and the same for
`onetaskgraph-plugin-api` depending on anything of this workspace. Prose does not hold
this; the first worker who wants one helper out of the engine will reach for it. So
`deny.toml` permits the engine exactly one wrapper (the binary, failing the required
`deny` job) and `scripts/check-plugin-isolation.sh` reads the real `cargo metadata` graph
inside `just check`, failing in seconds instead of minutes.

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
- **Coverage: 95% lines, per project, and each project measures only its own crate.**
  A workspace average lets a weak crate hide behind a strong one — and, decisively, a
  workspace-wide pass runs every crate's tests on every change, which is what affected
  selection exists to avoid. The *measurement* is skipped on Windows with a printed
  notice (instrumentation there does not attribute subprocess coverage); the functional
  lanes still gate that platform.
- **The live lane** is a uniform `test-live` target on every project, empty ones included.
  It is **not** a required check, and that is a decision: a required check a third party
  can turn red is a check that stops being trusted, and a Linear or GitHub outage must not
  block an unrelated merge.

### The journeys this repository owes

Each drives the real binary as a subprocess, and each runs against **every** configured
source kind through one shared fixture table — so a plugin is never proven by a suite of
its own writing. The list grows as features land, and the suite is what says which of
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

## Conventions

- **Scripts are context.** Quiet on success; on failure print the exact problem and a
  concrete next action.
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
