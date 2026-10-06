//! The harness every binary end-to-end suite of this workspace drives the CLI through.
//!
//! The journeys are split by the crate each one exercises — the engine's and the CLI's own in
//! `onetaskgraph-e2e`, each hosted plugin's in a suite of its own — so that affected selection
//! and caching follow the code a journey proves. What they share lives here, once:
//!
//! - [`common`]: the sandbox a journey runs the binary in, and the runner that spawns it;
//! - [`binary`] and [`source_binary`]: where that runner finds the build `onetaskgraph:build`
//!   made;
//! - [`fixtures`]: the journey-matrix table and the loopback servers behind its rows;
//! - [`linear_vocabulary`]: the one Linear team's vocabulary more than one suite configures.
//!
//! Not published, and depended on only by path from the test-only e2e members.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub mod common;
pub mod fixtures;
pub mod linear_vocabulary;

/// The `onetaskgraph` executable every journey spawns.
///
/// Located rather than named by `CARGO_BIN_EXE_onetaskgraph`, which cargo sets only for the
/// binary's own package: see [`built`].
#[must_use]
pub fn binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| built("onetaskgraph"))
}

/// The `onetaskgraph-source` executable, the reference stdio plugin host a subprocess-wrapped
/// source is configured to run.
#[must_use]
pub fn source_binary() -> &'static Path {
    static SOURCE: OnceLock<PathBuf> = OnceLock::new();
    SOURCE.get_or_init(|| built("onetaskgraph-source"))
}

/// One executable of the `onetaskgraph` package, in the build directory this test executable
/// was itself built into.
///
/// A test executable is `<target>/<profile>/deps/<name>-<hash>`, and cargo puts a package's
/// binaries in `<target>/<profile>`, so the two are found from one another whatever the target
/// directory is: `.cargo/config.toml`'s one `target` for a plain run, and cargo-llvm-cov's
/// `target/llvm-cov-target` for an instrumented one. This crate never builds the binary — every
/// e2e suite's `test` target depends on `onetaskgraph:build`, which is what puts it there — so a
/// missing file is refused naming that build rather than spawned and reported as a journey
/// failing.
fn built(name: &str) -> PathBuf {
    let executable = std::env::current_exe().expect("the running test executable has a path");
    let mut directory = executable
        .parent()
        .expect("a test executable sits inside a build directory")
        .to_path_buf();
    if directory.file_name().is_some_and(|leaf| leaf == "deps") {
        directory.pop();
    }
    let path = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "{} is not there, so there is no binary for this journey to drive; run \
         `scripts/nx.sh run onetaskgraph:build` from the workspace root (every e2e suite's `test` \
         target depends on it), then re-run",
        path.display()
    );
    path
}
