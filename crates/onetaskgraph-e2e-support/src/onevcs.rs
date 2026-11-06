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
pub const ONEVCS_CLI: &str = "onevcs-cli==0.59.0";

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

    /// [`Self::write_policy`], with this release's sanitise and terms commands, and
    /// `replacements` as the store's own replacement layer.
    #[must_use]
    pub fn sanitizing_write_policy(&self, replacements: Value) -> Value {
        let onevcs = onevcs().to_string_lossy().into_owned();
        let mut policy = self.write_policy();
        policy["sanitize_command"] = json!([onevcs, "boundary", "sanitize", "--input", "-"]);
        policy["terms_command"] = json!([onevcs, "boundary", "terms", "--input", "-"]);
        if replacements
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
        {
            policy["replacements"] = replacements;
        }
        policy
    }

    /// Commit `declaration`, a private-terms TOML document, as the repository-level
    /// declaration of the registered identity `owner/name`.
    pub fn declare_repository_terms(&self, owner: &str, name: &str, declaration: &str) {
        let checkout = self.checkouts.join(format!("{owner}-{name}"));
        std::fs::write(checkout.join("private-terms.toml"), declaration).expect("a declaration");
        for arguments in [
            vec!["add", "--all"],
            vec![
                "-c",
                "user.name=Synthetic",
                "-c",
                "user.email=synthetic@example.invalid",
                "commit",
                "--quiet",
                "--message",
                "declare terms",
            ],
        ] {
            let status = Command::new("git")
                .args(&arguments)
                .current_dir(&checkout)
                .status()
                .expect("git runs");
            assert!(status.success(), "git {arguments:?}");
        }
    }

    /// Write `declaration` as the directory level every registered checkout sits under.
    pub fn declare_directory_terms(&self, declaration: &str) {
        std::fs::write(self.checkouts.join("private-terms.toml"), declaration)
            .expect("a declaration");
    }

    /// Write `declaration` as the `$ONEVCS_HOME` fallback level.
    pub fn declare_fallback_terms(&self, declaration: &str) {
        std::fs::write(self.home.join("private-terms.toml"), declaration).expect("a declaration");
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

/// Which of the two boundary commands a store ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BoundaryCommand {
    /// `boundary inspect`: one repository's visibility.
    Inspect,
    /// `boundary check`: one write's exposure.
    Check,
    /// `boundary sanitize`: one write's exposure, rewritten.
    Sanitize,
    /// `boundary terms`: a scope's resolved terms.
    Terms,
}

/// One boundary command a store ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryCall {
    /// Which command it was.
    pub command: BoundaryCommand,
    /// The document the store handed it.
    pub input: Value,
    /// What it exited with.
    pub exit: Option<i64>,
    /// The wall-clock time it finished, since the Unix epoch.
    pub finished: std::time::Duration,
}

const RECORDER: &str = r#"import json, subprocess, sys, time
log, command = sys.argv[1], sys.argv[2:]
document = sys.stdin.buffer.read()
ran = subprocess.run(command, input=document, capture_output=True)
with open(log, "a", encoding="utf-8", newline="\n") as record:
    record.write(json.dumps({"command": command[2], "input": json.loads(document),
                             "exit": ran.returncode, "finished": time.time()}) + "\n")
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
    /// [`Self::write_policy`], every command naming this home itself, for an engine a test links:
    /// its commands inherit the test process's environment, which cannot name a home of this run.
    #[must_use]
    pub fn write_policy_naming_home(&self, sandbox: &Sandbox) -> Value {
        let script = sandbox.subdirectory("boundary-home").join("onevcs_home.py");
        std::fs::write(
            &script,
            "import os, subprocess, sys\n\
             os.environ['ONEVCS_HOME'] = sys.argv[1]\n\
             sys.exit(subprocess.call(sys.argv[2:]))\n",
        )
        .expect("the wrapper");
        let through = |command: &[&str]| {
            let mut argv = vec![
                python().to_string_lossy().into_owned(),
                script.to_string_lossy().into_owned(),
                self.path().to_string_lossy().into_owned(),
                onevcs().to_string_lossy().into_owned(),
            ];
            argv.extend(command.iter().map(|part| (*part).to_owned()));
            argv
        };
        json!({
            "visibility_command": through(&["boundary", "inspect", "--input", "-"]),
            "check_command": through(&["boundary", "check", "--destination", "public", "--input", "-"]),
        })
    }

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

impl OnevcsHome {
    /// [`Self::sanitizing_write_policy`], every command run through a recorder writing to
    /// `sandbox`.
    #[must_use]
    pub fn recorded_sanitizing(&self, sandbox: &Sandbox, replacements: Value) -> BoundaryRecord {
        let mut record = self.recorded(sandbox);
        let wrap = |words: &[&str]| -> Value {
            let base = record.policy["check_command"]
                .as_array()
                .expect("a recorded command")[..4]
                .to_vec();
            let mut argv = base;
            argv.extend(words.iter().map(|word| json!(word)));
            Value::Array(argv)
        };
        let sanitize = wrap(&["boundary", "sanitize", "--input", "-"]);
        let terms = wrap(&["boundary", "terms", "--input", "-"]);
        record.policy["sanitize_command"] = sanitize;
        record.policy["terms_command"] = terms;
        if replacements
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
        {
            record.policy["replacements"] = replacements;
        }
        record
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
                    command: match call["command"].as_str() {
                        Some("inspect") => BoundaryCommand::Inspect,
                        Some("check") => BoundaryCommand::Check,
                        Some("sanitize") => BoundaryCommand::Sanitize,
                        Some("terms") => BoundaryCommand::Terms,
                        other => panic!("the recorder ran a command it never runs: {other:?}"),
                    },
                    input: call["input"].clone(),
                    exit: call["exit"].as_i64(),
                    finished: std::time::Duration::from_secs_f64(
                        call["finished"].as_f64().expect("when it finished"),
                    ),
                }
            })
            .collect()
    }
}
