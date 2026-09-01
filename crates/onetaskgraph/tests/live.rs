// llmlint: ignore-file[live_tier_compiles_and_requires_credential] The registries this
// lane reads — crates.io, PyPI and npm — are public, so there is no credential for it to
// require and none whose absence it could fail fast on. The half of that rule this lane
// does keep is the half it can: it stays compiled by `cargo test -p onetaskgraph`, it is
// never `#[cfg]`'d out, and a registry that does not answer fails it rather than passing
// green.
//! The live lane for this crate: what the public registries really serve.
//!
//! `release-targets.toml` declares what this repository publishes, and
//! `config/registry-interfaces.toml` pins the interface each registry answers
//! through. The deterministic gate holds the probe to that pin and drives its
//! three answers against documents built from it, so no required check waits on a
//! registry being up. This is the other half: it asks the real registries, so the
//! day one of them changes its published interface there is something that says
//! so and the pin can be re-observed.
//!
//! `#[ignore]`, like every live test here, which is what keeps a third party out
//! of a required check. `just test-live` is what runs it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root, from this crate's own location rather than from the
/// working directory a runner happened to choose.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the repository root is two directories above this crate")
}

/// Whether an id is the `<registry>:<name>` the probe can act on.
///
/// The registry is lowercase and hyphenated; the name is everything after the one
/// colon between them, non-empty and carrying neither whitespace nor a second
/// colon. Deliberately the *shape* rather than the set of registries: which
/// registries exist is the probe's own answer, and a declaration naming one it
/// does not carry earns the probe's refusal rather than this lane's.
fn is_registry_qualified(identifier: &str) -> bool {
    let Some((registry, name)) = identifier.split_once(':') else {
        return false;
    };
    !registry.is_empty()
        && registry.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !name.is_empty()
        && !name.contains(':')
        && !name.chars().any(char::is_whitespace)
}

/// Every `[[target]]` id the declaration carries.
///
/// A scan rather than a TOML parse — what the document *is* is held by
/// `scripts/check-release-targets.sh` and by the canonical reader it runs, and
/// what this lane needs is only the list of things a consumer waits on — but a
/// scan that is strict about the two things it reads. A key is `id` exactly, not
/// anything beginning with it, and its value is a quoted `<registry>:<name>`; a
/// line under `[[target]]` that spells `id` some other way, or carries none at
/// all, panics here rather than being passed over, because a declared target this
/// lane silently skipped is a target nothing asks a registry about.
///
/// The id's own syntax is checked here as well as in the deterministic gate, and
/// not because one spelling can drift from the other — the gate is authoritative
/// about what the document is. It is because this lane hands each id straight to
/// the probe, which refuses a malformed one as a usage error, and the lane would
/// then report a declaration this could have refused by name as a registry that
/// did not answer. A caller sent looking at crates.io for a typo in a TOML file
/// is a caller this lane misdirected.
fn declared_target_ids(root: &Path) -> Vec<String> {
    let declaration = root.join("release-targets.toml");
    let text = std::fs::read_to_string(&declaration)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", declaration.display()));
    let mut ids = Vec::new();
    let mut in_target = false;
    let mut targets = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // Every table this leaves has to have given up exactly one id. A
            // target with none would otherwise be skipped in silence, which is
            // the one outcome this scan may not have.
            assert_eq!(
                ids.len(),
                targets,
                "{} has a [[target]] with no id in it",
                declaration.display()
            );
            in_target = line.starts_with("[[target]]");
            if in_target {
                targets += 1;
            }
            continue;
        }
        if !in_target {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "id" {
            continue;
        }
        let value = value.trim();
        let quoted = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .filter(|value| !value.is_empty() && !value.contains('"'));
        let Some(identifier) = quoted.filter(|value| is_registry_qualified(value)) else {
            panic!(
                "{} gives a [[target]] the id {value}, which is not a quoted <registry>:<name>",
                declaration.display()
            )
        };
        assert_eq!(
            ids.len() + 1,
            targets,
            "{} gives one [[target]] two ids, the second being {identifier}",
            declaration.display()
        );
        ids.push(identifier.to_string());
    }
    assert_eq!(
        ids.len(),
        targets,
        "{} has a [[target]] with no id in it",
        declaration.display()
    );
    assert!(
        !ids.is_empty(),
        "{} declares no target, so there is nothing to ask a registry about",
        declaration.display()
    );
    ids
}

/// Every artifact this repository declares answers the version its own registry
/// serves right now, through the probe a consumer runs.
///
/// A target that does not answer lands here whether the registry was unreachable
/// or its published interface moved, and the failure says which to look at: the
/// probe's own reason distinguishes a transport failure from a document that no
/// longer carries the pinned field.
#[test]
#[ignore = "reaches the public registries; run it with `just test-live`"]
fn every_declared_target_answers_from_its_real_registry() {
    let root = repository_root();
    let probe = root.join("scripts").join("release-probe.sh");
    let mut failures = Vec::new();

    for identifier in declared_target_ids(&root) {
        let answer = Command::new("bash")
            .arg(&probe)
            .arg(&identifier)
            .current_dir(&root)
            .output()
            .unwrap_or_else(|error| panic!("could not run {}: {error}", probe.display()));
        let stdout = String::from_utf8_lossy(&answer.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&answer.stderr).trim().to_string();

        if !answer.status.success() {
            failures.push(format!(
                "{identifier}: not answered ({}). The probe said: {stderr}",
                answer.status
            ));
            continue;
        }
        if stdout.is_empty() {
            failures.push(format!(
                "{identifier}: its registry answered that it serves nothing. Every declared \
                 target here has been released, so this is the lookup reaching the wrong \
                 place — re-observe that registry's interface and bring \
                 config/registry-interfaces.toml and scripts/release-probe.sh to it. A target \
                 declared before its own first release is the one other way to land here."
            ));
            continue;
        }
        // What a version may look like is not restated here. The probe holds its
        // own answer closed — a body that is not a version is refused there, and
        // the three registries' grammars differ — so a second copy of that shape
        // in this lane would be one nothing reconciles, and it would refuse the
        // PEP 440 epoch the probe deliberately answers.
    }

    assert!(
        failures.is_empty(),
        "the public registries did not answer for every declared target:\n  {}",
        failures.join("\n  ")
    );
}
