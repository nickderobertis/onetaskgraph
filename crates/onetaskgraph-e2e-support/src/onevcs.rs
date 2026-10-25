//! The released onevcs boundary commands, for the journeys of every suite that configure a
//! store's `write_policy` with them.
//!
//! Nothing here stands in for onevcs: a journey runs the release this repository pins, against
//! an onevcs home this run owns, holding synthetic identities only — a git repository made in
//! the sandbox, registered under an invented owner and name, and declared private or public by
//! a rule so no host is ever asked about it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::{Value, json};

use crate::common::Sandbox;

/// The onevcs release whose boundary commands these journeys drive: the one place it is pinned.
///
/// Its `boundary schema --json` is what the store's command adapter embeds, so moving this pin
/// is moving the schema the reconciliation journey compares against.
pub const ONEVCS_CLI: &str = "onevcs-cli==0.44.0";

// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] uv provides the pinned release from its own cache, fetching it from PyPI at most once per host, and only journeys that configure a `write_policy` ask for it: the engine suite's `boundary` journeys, which are the engine's and the configuration's and so cannot sit behind a plugin crate's edge, and the GitHub Projects suite's screened plan copy and approval, which already sit behind that plugin's own edge.
/// The pinned `onevcs` executable, resolved once per test process through `uv`.
///
/// # Panics
///
/// When `uv` cannot provide the pinned release, naming how to install it.
#[must_use]
pub fn onevcs() -> &'static Path {
    static ONEVCS: OnceLock<PathBuf> = OnceLock::new();
    ONEVCS.get_or_init(|| {
        let output = Command::new("uv")
            .args([
                "tool",
                "run",
                "--from",
                ONEVCS_CLI,
                "python",
                "-c",
                "import shutil, sys; sys.stdout.write(shutil.which('onevcs') or '')",
            ])
            .output()
            .unwrap_or_else(|error| {
                panic!(
                    "uv could not be started to provide {ONEVCS_CLI} ({error}); install uv — \
                     https://docs.astral.sh/uv/ — which `just bootstrap` also needs"
                )
            });
        let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        assert!(
            output.status.success() && !path.is_empty(),
            "uv could not provide {ONEVCS_CLI}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        PathBuf::from(path)
    })
}
// llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]

/// What a registered identity's rule declares about who can read it.
#[derive(Clone, Copy)]
enum Declared {
    Public,
    Private,
}

impl Declared {
    /// The value onevcs's rules file spells it as.
    fn spelled(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

/// One identity this run registers: an invented `owner/name`, and what its rule declares.
struct Identity {
    owner: String,
    name: String,
    visibility: Declared,
}

/// An onevcs home inside one sandbox, and the synthetic identities registered in it.
pub struct OnevcsHome {
    home: PathBuf,
    checkouts: PathBuf,
    identities: Vec<Identity>,
}

impl OnevcsHome {
    /// An empty home under `sandbox`, holding no identity.
    #[must_use]
    pub fn new(sandbox: &Sandbox) -> Self {
        let home = sandbox.config_home().join("onevcs-home");
        let checkouts = sandbox.config_home().join("onevcs-checkouts");
        std::fs::create_dir_all(&home).expect("the onevcs home");
        std::fs::create_dir_all(&checkouts).expect("the checkouts directory");
        let created = Self {
            home,
            checkouts,
            identities: Vec::new(),
        };
        created.write_rules();
        created
    }

    /// Where `ONEVCS_HOME` points for every command this home answers.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.home
    }

    /// Register `owner/name` on `github.com` as a private identity.
    pub fn private(&mut self, owner: &str, name: &str) -> &mut Self {
        self.register(owner, name, Declared::Private, &[])
    }

    /// Register `owner/name` on `github.com` as a public identity.
    pub fn public(&mut self, owner: &str, name: &str) -> &mut Self {
        self.register(owner, name, Declared::Public, &[])
    }

    /// Register `owner/name` as a private identity whose committed `Cargo.toml` does not parse,
    /// which makes a check deriving terms from it unavailable.
    pub fn private_with_malformed_manifest(&mut self, owner: &str, name: &str) -> &mut Self {
        self.register(
            owner,
            name,
            Declared::Private,
            &[("Cargo.toml", "[package\nname = \"broken")],
        )
    }

    /// The `write_policy` block naming this release's two commands.
    #[must_use]
    pub fn write_policy(&self) -> Value {
        let onevcs = onevcs().to_string_lossy().into_owned();
        json!({
            "visibility_command": [onevcs, "boundary", "inspect", "--input", "-"],
            "check_command": [onevcs, "boundary", "check", "--destination", "public", "--input", "-"],
        })
    }

    fn register(
        &mut self,
        owner: &str,
        name: &str,
        visibility: Declared,
        files: &[(&str, &str)],
    ) -> &mut Self {
        self.identities.push(Identity {
            owner: owner.to_owned(),
            name: name.to_owned(),
            visibility,
        });
        self.write_rules();
        let checkout = self.checkouts.join(format!("{owner}-{name}"));
        std::fs::create_dir_all(&checkout).expect("the synthetic checkout");
        std::fs::write(checkout.join("README.md"), format!("# {name}\n")).expect("a file");
        for (path, text) in files {
            std::fs::write(checkout.join(path), text).expect("a committed file");
        }
        for arguments in [
            vec!["init", "--quiet", "--initial-branch=main"],
            vec!["add", "--all"],
            vec![
                "-c",
                "user.name=Synthetic",
                "-c",
                "user.email=synthetic@example.invalid",
                "commit",
                "--quiet",
                "--message",
                "synthetic identity",
            ],
        ] {
            let status = Command::new("git")
                .args(&arguments)
                .current_dir(&checkout)
                .status()
                .expect("git runs");
            assert!(
                status.success(),
                "git {arguments:?} in {}",
                checkout.display()
            );
        }
        let origin = format!("https://github.com/{owner}/{name}.git");
        let output = Command::new(onevcs())
            .arg("register")
            .arg(&checkout)
            .args(["--origin", &origin])
            .env("ONEVCS_HOME", &self.home)
            .output()
            .expect("onevcs runs");
        assert!(
            output.status.success(),
            "onevcs could not register {owner}/{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self
    }

    /// The rules file declaring every identity's visibility, so no host is asked.
    fn write_rules(&self) {
        let mut rules = String::from("version: 4\nrules:\n");
        for identity in &self.identities {
            rules.push_str(&format!(
                "  - match: {{host: github.com, owner: {}, name: {}}}\n    visibility: {}\n",
                identity.owner,
                identity.name,
                identity.visibility.spelled()
            ));
        }
        if self.identities.is_empty() {
            rules = String::from("version: 4\nrules: []\n");
        }
        rules.push_str("default: {publication: change-open, approvals: required}\n");
        std::fs::write(self.home.join("rules.yml"), rules).expect("the rules file");
    }
}

/// A store's `write_policy` naming this release's two commands through a recorder, and the
/// record it keeps: every boundary command the store ran, in the order it ran them, with the
/// document it was handed.
///
/// The recorder is test code a journey writes into its own sandbox — a few lines of Python
/// that append one line to the record and hand the input to the released command unchanged,
/// answering exactly what it answered — so what a journey reads back is what the store really
/// invoked, and the store gains no surface for it.
pub struct BoundaryRecord {
    log: PathBuf,
    policy: Value,
}

/// One boundary command a store ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryCall {
    /// `inspect` or `check`.
    pub command: String,
    /// The document the store handed it.
    pub input: Value,
    /// What it exited with.
    pub exit: Option<i64>,
}

const RECORDER: &str = r#"import json, subprocess, sys
log, command = sys.argv[1], sys.argv[2:]
document = sys.stdin.buffer.read()
ran = subprocess.run(command, input=document, capture_output=True)
with open(log, "a", encoding="utf-8", newline="\n") as record:
    record.write(json.dumps({"command": command[2], "input": json.loads(document),
                             "exit": ran.returncode}) + "\n")
sys.stdout.buffer.write(ran.stdout)
sys.stdout.flush()
sys.exit(ran.returncode)
"#;

/// The Python interpreter the recorder runs on, found on `PATH` — `python3` is what every
/// guard of this workspace already runs on all three platforms.
fn python() -> &'static Path {
    static PYTHON: OnceLock<PathBuf> = OnceLock::new();
    PYTHON.get_or_init(|| {
        let names: &[&str] = if cfg!(windows) {
            &["python3.exe", "python.exe"]
        } else {
            &["python3", "python"]
        };
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
            .find(|candidate| candidate.is_file())
            .expect(
                "no python3 on PATH to record the boundary commands with; install Python 3, \
                 which every guard under `workspace:lint` already runs",
            )
    })
}

impl OnevcsHome {
    /// [`Self::write_policy`], every command run through a recorder writing to `sandbox`.
    #[must_use]
    pub fn recorded(&self, sandbox: &Sandbox) -> BoundaryRecord {
        let directory = sandbox.subdirectory("boundary-record");
        let script = directory.join("recorder.py");
        std::fs::write(&script, RECORDER).expect("the recorder");
        let log = directory.join("calls.jsonl");
        let through = |command: &[&str]| {
            let mut argv = vec![
                python().to_string_lossy().into_owned(),
                script.to_string_lossy().into_owned(),
                log.to_string_lossy().into_owned(),
                onevcs().to_string_lossy().into_owned(),
            ];
            argv.extend(command.iter().map(|part| (*part).to_owned()));
            argv
        };
        BoundaryRecord {
            policy: json!({
                "visibility_command": through(&["boundary", "inspect", "--input", "-"]),
                "check_command": through(&["boundary", "check", "--destination", "public", "--input", "-"]),
            }),
            log,
        }
    }
}

impl BoundaryRecord {
    /// The `write_policy` block to configure.
    #[must_use]
    pub fn write_policy(&self) -> &Value {
        &self.policy
    }

    /// Every boundary command run so far, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<BoundaryCall> {
        let Ok(text) = std::fs::read_to_string(&self.log) else {
            return Vec::new();
        };
        text.lines()
            .map(|line| {
                let call: Value = serde_json::from_str(line).expect("a recorded call");
                BoundaryCall {
                    command: call["command"].as_str().expect("its command").to_owned(),
                    input: call["input"].clone(),
                    exit: call["exit"].as_i64(),
                }
            })
            .collect()
    }
}
