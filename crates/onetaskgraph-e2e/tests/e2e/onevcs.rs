//! The released onevcs boundary commands, for the journeys that configure a store's
//! `write_policy` with them.
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
pub(crate) const ONEVCS_CLI: &str = "onevcs-cli==0.44.0";

// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] uv provides the pinned release from its own cache, fetching it from PyPI at most once per host, and only the `boundary` journeys ask for it. Those journeys are the engine's and the configuration's — classification, routing by it and the write gate — so they cannot sit behind a plugin crate's edge, which AGENTS.md forbids depending on the engine; the engine's suite is the narrowest edge they have.
/// The pinned `onevcs` executable, resolved once per test process through `uv`.
///
/// # Panics
///
/// When `uv` cannot provide the pinned release, naming how to install it.
#[must_use]
pub(crate) fn onevcs() -> &'static Path {
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
pub(crate) struct OnevcsHome {
    home: PathBuf,
    checkouts: PathBuf,
    identities: Vec<Identity>,
}

impl OnevcsHome {
    /// An empty home under `sandbox`, holding no identity.
    #[must_use]
    pub(crate) fn new(sandbox: &Sandbox) -> Self {
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
    pub(crate) fn path(&self) -> &Path {
        &self.home
    }

    /// Register `owner/name` on `github.com` as a private identity.
    pub(crate) fn private(&mut self, owner: &str, name: &str) -> &mut Self {
        self.register(owner, name, Declared::Private, &[])
    }

    /// Register `owner/name` on `github.com` as a public identity.
    pub(crate) fn public(&mut self, owner: &str, name: &str) -> &mut Self {
        self.register(owner, name, Declared::Public, &[])
    }

    /// Register `owner/name` as a private identity whose committed `Cargo.toml` does not parse,
    /// which makes a check deriving terms from it unavailable.
    pub(crate) fn private_with_malformed_manifest(&mut self, owner: &str, name: &str) -> &mut Self {
        self.register(
            owner,
            name,
            Declared::Private,
            &[("Cargo.toml", "[package\nname = \"broken")],
        )
    }

    /// The `write_policy` block naming this release's two commands.
    #[must_use]
    pub(crate) fn write_policy(&self) -> Value {
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
