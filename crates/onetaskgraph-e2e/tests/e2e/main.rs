//! Every journey of the engine and the command line, driven the way a user drives them.
//!
//! Each test spawns the compiled binary as a subprocess and asserts on its exit code,
//! stdout and stderr — never an in-process `run()` call, and nothing about the process is
//! mocked. `AGENTS.md` carries the list of journeys this repository owes; the modules
//! below are where the engine's and the CLI's own live, and each hosted plugin's own are in
//! the e2e suite named after it.
//!
//! This is a test-only workspace member of its own rather than the binary crate's `tests/`, so
//! a change the engine and the binary cannot see does not run it. It drives the binary
//! `onetaskgraph:build` made, found by `onetaskgraph_e2e_support::binary`, and
//! `onetaskgraph_e2e_support::fixtures` is the table that makes a journey written once run
//! against every source kind — `scripts/check-journey-matrix.sh` fails when a plugin the
//! registry knows has no row in it.

use onetaskgraph_e2e_support::{common, fixtures, linear_vocabulary};

// Image assets on tasks and documents: stored, listed, rendered and copied through the
// binary against folders of Markdown and the Python asset store over a real pipe.
mod assets;
// The simulated clock: its coordinator driven by real client processes over loopback, and the
// binary attaching to it. No source and no third party; nothing waits in real time but the
// deliberate few hundred milliseconds of computing a scenario needs to prove time holds still.
mod clock;
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
// narrower edge for it: every journey here drives the binary against folders of Markdown and a
// loopback fixture board, with no credential and no network, and the whole module finishes in
// under a second. What it proves is the engine's own rule — `task status set` and the
// delivered-task propagation live in `onetaskgraph-core` — so it cannot sit behind a plugin
// crate's edge, which AGENTS.md forbids depending on the engine at any depth, and AGENTS.md
// requires a journey to drive the compiled binary rather than the engine in process.
mod delivery;
mod document_store;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the engine the binary links against the
// loopback GitHub board and Linear workspace, a folder of Markdown and the stdio host, with no
// credential and no network, in about a second. `Engine::end_command` is the engine's, so it
// cannot sit behind a plugin crate's edge, which AGENTS.md forbids depending on the engine at
// any depth; the protocol's half is proven in `onetaskgraph-core`'s own `tests/subprocess.rs`.
mod end_command;
mod failures;
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
// seconds. Creating and regenerating a project is the engine's, so it cannot sit behind a
// plugin crate's edge, which AGENTS.md forbids depending on the engine at any depth.
mod project_render;
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
mod surface;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: the template journeys configure no source, reach no network and finish
// in under a second, and `template` is a verb of this binary, so this suite of the binary's own
// journeys is the narrowest project that can own a journey driving it as a subprocess — the
// library half is proven in `onetaskgraph-core`'s own `tests/templates.rs`, behind that crate's
// edge.
mod templates;
// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] Not expensive, and there is no
// narrower edge for it: every journey here drives the binary against the shared rows — folders
// of Markdown, in-memory sources, the loopback Linear workspace and GitHub board, and the stdio
// host — with no credential and no network, in about a second. The targeted update is the
// engine's (`Engine::update_task` decides what is reported and which delivered tasks move), so
// it cannot sit behind one plugin crate's edge, which AGENTS.md forbids depending on the engine
// at any depth; each plugin's own half is proven behind its own edge in its own tests.
mod update;
