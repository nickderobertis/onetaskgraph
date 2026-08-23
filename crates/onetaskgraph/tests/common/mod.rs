//! A sandboxed host for the journeys that drive the binary.
//!
//! Everything the configuration layer reads is real here — a real directory tree, real
//! files, a real process environment — because that is the layer under test. What the
//! sandbox does is put all of it somewhere this run owns, so a journey cannot read the
//! developer's own `~/.config/onetaskgraph` and cannot be changed by it.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

/// A temporary host: a project tree to run in, and a configuration home over it.
pub struct Sandbox {
    root: TempDir,
}

impl Sandbox {
    /// An empty host — no documents, no credentials file.
    pub fn new() -> Self {
        let root = TempDir::new().expect("a temporary directory");
        std::fs::create_dir_all(root.path().join("project")).expect("the project tree");
        std::fs::create_dir_all(root.path().join("home/onetaskgraph")).expect("the config home");
        Self { root }
    }

    /// The directory a command runs in.
    pub fn project(&self) -> PathBuf {
        self.root.path().join("project")
    }

    /// The configuration home, as `XDG_CONFIG_HOME`.
    pub fn config_home(&self) -> PathBuf {
        self.root.path().join("home")
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
        let mut command = Command::cargo_bin("onetaskgraph").expect("the binary is built");
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

/// Write `text` to `path`, creating what it needs, and return the path.
fn write(path: PathBuf, text: &str) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the directory holding the file");
    }
    std::fs::write(&path, text).expect("the file is written");
    path
}

/// A configuration with one `in-memory` source called `work`, for journeys that need
/// a source to address rather than a particular source's behaviour.
pub const ONE_SOURCE: &str = "\
sources:
  work:
    plugin: in-memory
    config:
      capabilities:
        max_page_size: 20
";

/// Standard output as text, failing the test rather than the assertion when it is not
/// UTF-8 — a binary that emitted bytes here would be a different bug entirely.
pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

/// Standard error as text.
pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}
