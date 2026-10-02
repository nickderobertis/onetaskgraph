//! Every journey this binary answers, driven the way a user drives it.
//!
//! Each test spawns the compiled binary as a subprocess and asserts on its exit code,
//! stdout and stderr — never an in-process `run()` call, and nothing about the process is
//! mocked. `AGENTS.md` carries the list of journeys this repository owes; the modules
//! below are where they live.
//!
//! [`fixtures`] is the table that makes a journey written once run against every source
//! kind, and `scripts/check-journey-matrix.sh` fails when a plugin the registry knows has
//! no row in it.

// The sandbox `tests/configuration.rs` uses, reached by path because a module of this
// test target would otherwise be looked for under `tests/e2e/`.
//
// `allow` rather than `expect`, and here rather than in the module itself: one support
// module serves two test targets and each uses the part it needs, so what is unused is a
// property of *this* target rather than of the helper. Marking it in the module would
// switch the check off for `tests/configuration.rs` too, where a helper nothing calls
// really is dead.
#[allow(
    dead_code,
    reason = "one shared sandbox, two test targets, a subset used by each"
)]
#[path = "../common/mod.rs"]
mod common;

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against two folders of Markdown,
// in process and over the stdio plugin protocol, with no credential and no network, in about a
// second. The flag and the engine's narrowing are the binary's and the engine's, so they cannot
// sit behind a plugin crate's edge, which AGENTS.md forbids depending on the engine at any
// depth.
mod commented_since;
mod comments;
mod copy;
mod copy_link;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: it drives the binary against a loopback fixture board with no
// credential and no network and finishes in under a second, and because a copy is the
// engine's it cannot sit behind `onetaskgraph-github-projects`' own edge — AGENTS.md forbids
// a plugin crate depending on the engine at any depth. Every other journey against that same
// fixture board already runs in this target.
mod copy_cost;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against folders of Markdown and a
// loopback fixture board, with no credential and no network, and the whole module finishes in
// under a second. What it proves is the engine's own rule — `task status set` and the
// delivered-task propagation live in `onetaskgraph-core` — so it cannot sit behind a plugin
// crate's edge, which AGENTS.md forbids depending on the engine at any depth, and AGENTS.md
// requires a journey to drive the compiled binary rather than the engine in process.
mod delivery;
mod document_store;
mod failures;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] This offline module drives
// the required real CLI boundary against a loopback board and completes in about a second,
// for the reason `status_options` below does: the binary project is the narrowest project that
// can own a binary subprocess journey, and the plugin-isolation contract forbids moving it
// into a plugin crate.
mod fields;
mod fixtures;
mod journeys;
mod machine;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against folders of Markdown and
// the Python stdio peer, with no credential and no network, in about a second. The verbs are
// the engine's and the binary's — the refusals it proves happen before any plugin is built —
// so they cannot sit behind a plugin crate's edge, which AGENTS.md forbids depending on the
// engine at any depth.
mod metadata;
mod multi_source;
mod no_persistence;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against the shared rows — folders
// of Markdown, in-memory sources, the loopback Linear workspace and GitHub board, and the
// Python stdio peer — with no credential and no network, in a few seconds. What it proves is
// the engine's own refusal and the contract every plugin shares, so it cannot sit behind one
// plugin crate's edge, which AGENTS.md forbids depending on the engine at any depth.
mod priority;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against folders of Markdown, an
// in-memory source and the loopback GitHub board, with no credential and no network, in a few
// seconds. Creating and regenerating an item is the engine's, so it cannot sit behind a plugin
// crate's edge, which AGENTS.md forbids depending on the engine at any depth.
mod rendered;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against folders of Markdown and
// the loopback Linear workspace and GitHub board, with no credential and no network, and the
// module's thirty-eight journeys finish in under two seconds. Routing and member projects are
// the engine's and the configuration's — a copy spanning two plugins is placed and undone by
// `onetaskgraph-core` — so they cannot sit behind one plugin crate's edge, which AGENTS.md
// forbids depending on the engine at any depth.
mod routes;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: it drives the binary against the shared fixture rows and a loopback
// fixture board, with no credential and no network, in a few seconds. `task show-many` is the
// engine's verb, so it cannot sit behind a plugin crate's edge, which AGENTS.md forbids
// depending on the engine at any depth.
mod show_many;
mod source_host;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] This offline module drives
// the required real CLI boundary against a loopback board and completes nine journeys in
// under one second. The binary project is the narrowest project that can own a binary
// subprocess journey; the plugin-isolation contract forbids moving it into a plugin crate.
mod status_options;
mod surface;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: the template journeys configure no source, reach no network and finish
// in under a second, and `template` is a verb of this binary, so the binary crate is the
// narrowest project that can own a journey driving it as a subprocess — the library half is
// proven in `onetaskgraph-core`'s own `tests/templates.rs`, behind that crate's edge.
mod templates;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against the shared rows — folders
// of Markdown, in-memory sources, the loopback Linear workspace and GitHub board, and the stdio
// host — with no credential and no network, in about a second. The targeted update is the
// engine's (`Engine::update_task` decides what is reported and which delivered tasks move), so
// it cannot sit behind one plugin crate's edge, which AGENTS.md forbids depending on the engine
// at any depth; each plugin's own half is proven behind its own edge in its own tests.
mod update;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against a folder of Markdown and
// the loopback fixture board, with no credential and no network, in a few seconds. The update
// and the copy it holds to their write order are the engine's verbs, so they cannot sit behind
// a plugin crate's edge, which AGENTS.md forbids depending on the engine at any depth.
mod write_order;
