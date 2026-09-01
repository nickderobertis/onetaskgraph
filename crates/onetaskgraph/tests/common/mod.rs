//! A sandboxed host for the journeys that drive the binary.
//!
//! Everything the configuration layer reads is real here — a real directory tree, real
//! files, a real process environment — because that is the layer under test. What the
//! sandbox does is put all of it somewhere this run owns, so a journey cannot read the
//! developer's own `~/.config/onetaskgraph` and cannot be changed by it.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

/// The two boundaries every source-backed journey must cross.
#[derive(Clone, Copy, Debug)]
pub enum SourceBoundary {
    /// The engine builds the source in its own process.
    Direct,
    /// The engine reaches the same source through the shipped stdio host.
    Subprocess,
}

pub const SOURCE_BOUNDARIES: [SourceBoundary; 2] =
    [SourceBoundary::Direct, SourceBoundary::Subprocess];

impl SourceBoundary {
    /// One configured source, preserving the source's own plugin and configuration.
    pub fn source(self, plugin: &str, config: Value) -> Value {
        self.source_with_secrets(plugin, config, &[])
    }

    /// The same, for a source that needs a named credential to reach its backend.
    ///
    /// §3.1 forbids a plugin reading a credential out of its own environment, and the
    /// stdio host clears the child's, so a hosted source reaches only the variables its
    /// configuration *names*. An in-process source reads the engine's own resolver and
    /// needs no such list, which is why this is one method rather than two shapes of
    /// fixture: the journey says which credential the source behind the boundary needs,
    /// and each boundary does whatever getting it there takes.
    pub fn source_with_secrets(self, plugin: &str, config: Value, secrets: &[&str]) -> Value {
        match self {
            Self::Direct => json!({"plugin": plugin, "config": config}),
            Self::Subprocess => json!({
                "plugin": "subprocess",
                "config": {
                    "command": env!("CARGO_BIN_EXE_onetaskgraph-source"),
                    "secrets": secrets,
                    "settings": {"kind": plugin, "config": config},
                },
            }),
        }
    }

    /// The configuration path to a field in the source behind this boundary.
    pub fn config_path(self, field: &str) -> String {
        match self {
            Self::Direct => format!("sources.work.config.{field}"),
            Self::Subprocess => format!("sources.work.config.settings.config.{field}"),
        }
    }

    /// The environment spelling of [`Self::config_path`].
    pub fn config_variable(self, field: &str) -> String {
        format!(
            "ONETASKGRAPH_{}",
            self.config_path(field)
                .split('.')
                .map(|segment| segment.to_ascii_uppercase().replace('-', "_"))
                .collect::<Vec<_>>()
                .join("__")
        )
    }
}

/// A temporary host: a project tree to run in, and a configuration home over it.
pub struct Sandbox {
    /// Held for its drop, which removes the tree. Every path comes from `root` below.
    _directory: TempDir,
    /// The same tree, named the way the child process will name it.
    root: PathBuf,
}

impl Sandbox {
    /// An empty host — no documents, no credentials file.
    pub fn new() -> Self {
        let directory = TempDir::new().expect("a temporary directory");
        let root = resolved(directory.path());
        std::fs::create_dir_all(root.join("project")).expect("the project tree");
        std::fs::create_dir_all(root.join("home/onetaskgraph")).expect("the config home");
        Self {
            _directory: directory,
            root,
        }
    }

    /// The directory a command runs in.
    pub fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    /// The configuration home, as `XDG_CONFIG_HOME`.
    pub fn config_home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// Write `onetaskgraph.yaml` at the top of the project tree.
    pub fn project_document(&self, text: &str) -> PathBuf {
        write(self.project().join("onetaskgraph.yaml"), text)
    }

    /// Write the user-level document under the configuration home.
    pub fn user_document(&self, text: &str) -> PathBuf {
        write(self.config_home().join("onetaskgraph/config.yaml"), text)
    }

    /// Write the credentials file under the configuration home.
    pub fn secrets_file(&self, text: &str) -> PathBuf {
        write(self.config_home().join("onetaskgraph/secrets.env"), text)
    }

    /// Create a directory under the project tree and return it.
    pub fn subdirectory(&self, relative: &str) -> PathBuf {
        let path = self.project().join(relative);
        std::fs::create_dir_all(&path).expect("a directory under the project tree");
        path
    }

    /// The binary, running in the project tree with this sandbox's configuration home.
    pub fn command(&self) -> Command {
        self.command_in(&self.project())
    }

    /// The binary, running in `directory` with this sandbox's configuration home.
    ///
    /// The ambient `ONETASKGRAPH_` variables are removed rather than the whole
    /// environment cleared: a coverage run hands the child `LLVM_PROFILE_FILE`, and
    /// clearing it would silently stop attributing this binary's lines to its crate.
    pub fn command_in(&self, directory: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_onetaskgraph"));
        for (name, _) in std::env::vars() {
            if name.starts_with("ONETASKGRAPH_") {
                command.env_remove(name);
            }
        }
        command
            .current_dir(directory)
            .env("XDG_CONFIG_HOME", self.config_home())
            .env_remove("HOME");
        command
    }

    /// Put a directory where a file belongs, under the configuration home.
    ///
    /// A file this run cannot read, without needing a permission a test user may not
    /// be able to set and without a `chmod` that means nothing on Windows: a directory
    /// exists as far as a read is concerned, and reading one fails everywhere.
    pub fn unreadable(&self, relative: &str) -> PathBuf {
        let path = self.config_home().join(relative);
        std::fs::create_dir_all(&path).expect("a directory in the file's place");
        path
    }
}

/// The temporary root under the name the binary will report for it.
///
/// A journey that asserts on the document's path compares two names for one file: the
/// one this sandbox built and the one the child derived from its own `current_dir()`.
/// The operating system resolves symbolic links out of the second, so on a host whose
/// temporary directory is reached through one — macOS, where `/var` is a link to
/// `/private/var` — the two names differ and every such assertion fails there and
/// nowhere else. Resolving here makes the sandbox's name for the tree the child's name
/// for it, so the comparison is about the layer under test rather than about the host.
#[cfg(not(windows))]
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().expect("the temporary root resolves")
}

/// On Windows, the name `TempDir` returns is already the one the child reports.
///
/// There is no symlinked temporary root to resolve there, and `canonicalize` would
/// answer with a `\\?\` verbatim path — a third name for the tree, carried by neither
/// side of the comparison above.
#[cfg(windows)]
fn resolved(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Write `text` to `path`, creating what it needs, and return the path.
fn write(path: PathBuf, text: &str) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the directory holding the file");
    }
    std::fs::write(&path, text).expect("the file is written");
    path
}

/// A configuration with one `in-memory` source called `work`, on either boundary.
pub fn one_source(boundary: SourceBoundary) -> String {
    serde_json::to_string(&json!({
        "sources": {
            "work": boundary.source("in-memory", json!({
                "capabilities": {"max_page_size": 20}
            }))
        }
    }))
    .expect("a one-source document")
}

/// Standard output as text, failing the test rather than the assertion when it is not
/// UTF-8 — a binary that emitted bytes here would be a different bug entirely.
pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

/// Standard error as text.
pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}
