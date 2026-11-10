//! `task attest`, `project attest` and `document attest`, and the `attestation check` verbs:
//! the flags, the renderings and the exit codes. Composing, recording and checking a record
//! are the engine's ([`onetaskgraph_core::Engine::attest`] and its siblings), so a Rust caller
//! and this binary cannot do them differently.

use std::io::Write;

use onetaskgraph_core::attestation::{
    AttestationCheck, AttestationKind, AttestationState, Attested, ProjectAttestationCheck, label,
};
use onetaskgraph_core::{AttestRequest, Failure, Loaded};
use onetaskgraph_plugin_api::{MetadataKey, MetadataRecord};

use crate::cli::{AttestArgs, AttestationCheckArgs, ProjectAttestationCheckArgs};
use crate::{EXIT_OK, emit, engine, qualified, rendering, write_failed};

/// The check ran and answered a record that is not valid — `void` or `absent` — so a script
/// gating on it can tell that from a check that could not run (exit `1`).
pub(crate) const EXIT_NOT_VALID: u8 = 3;

/// `<task|project|document> attest`: record the attestation, then write the record — under
/// `--json`, `{id, record}`.
pub(crate) async fn attest(
    out: &mut impl Write,
    loaded: &Loaded,
    record: MetadataRecord,
    args: &AttestArgs,
) -> Result<u8, Failure> {
    let request = AttestRequest {
        id: qualified(&args.id)?,
        kind: kind(&args.kind)?,
        by: label("--by", &args.by).map_err(refused)?,
        bar: label("--bar", &args.bar).map_err(refused)?,
        excludes: args
            .exclude
            .iter()
            .map(|key| {
                MetadataKey::new(key.as_str()).map_err(|message| {
                    Failure::decided(
                        "invalid-metadata-key",
                        format!("--exclude {key}: {message}"),
                    )
                })
            })
            .collect::<Result<_, _>>()?,
        note: args
            .note
            .as_deref()
            .map(|note| label("--note", note))
            .transpose()
            .map_err(refused)?,
        if_version: args.if_version.clone(),
    };
    let engine = engine(loaded);
    let attested = engine
        .attest(record, &request)
        .await
        .map_err(|error| write_failed(&engine, &error))?;
    let rendered = rendering(loaded, &attested, recorded, "the attestation")?;
    emit(out, rendered.trim_end(), "the attestation")?;
    Ok(EXIT_OK)
}

/// `task attestation check` and `document attestation check`.
pub(crate) async fn check(
    out: &mut impl Write,
    loaded: &Loaded,
    record: MetadataRecord,
    args: &AttestationCheckArgs,
) -> Result<u8, Failure> {
    let id = qualified(&args.id)?;
    let kind = kind(&args.kind)?;
    let checked = engine(loaded)
        .check_attestation(record, &id, &kind, args.bar.as_deref())
        .await
        .map_err(|error| Failure::from(&error))?;
    let rendered = rendering(loaded, &checked, checked_text, "the attestation check")?;
    emit(out, rendered.trim_end(), "the attestation check")?;
    Ok(exit(checked.is_valid()))
}

/// `project attestation check`: the project's own record, or with `--tasks` every task's.
pub(crate) async fn check_project(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &ProjectAttestationCheckArgs,
) -> Result<u8, Failure> {
    if !args.tasks {
        return check(out, loaded, MetadataRecord::Project, &args.check).await;
    }
    let id = qualified(&args.check.id)?;
    let kind = kind(&args.check.kind)?;
    let checked = engine(loaded)
        .check_project_attestations(&id, &kind, args.check.bar.as_deref())
        .await
        .map_err(|error| Failure::from(&error))?;
    let rendered = rendering(loaded, &checked, tasks_text, "the attestation check")?;
    emit(out, rendered.trim_end(), "the attestation check")?;
    Ok(exit(checked.is_valid()))
}

fn exit(valid: bool) -> u8 {
    if valid { EXIT_OK } else { EXIT_NOT_VALID }
}

fn kind(kind: &str) -> Result<AttestationKind, Failure> {
    AttestationKind::new(kind).map_err(refused)
}

fn refused(error: onetaskgraph_core::attestation::AttestationInputError) -> Failure {
    Failure::decided(
        "invalid-attestation",
        format!(
            "{error}\nnext: give {} a value of one line — a kind of letters, digits, `.`, `_` \
             and `-`.",
            error.field
        ),
    )
}

/// One record, as a person reads it.
fn recorded(attested: &Attested) -> String {
    let record = &attested.record;
    format!(
        "{}: {} recorded by {} at {} against bar {} (content {})",
        attested.id, record.kind, record.by, record.at, record.bar, record.content_digest
    )
}

fn state(check: &AttestationCheck) -> String {
    match (check.state, check.reason) {
        (AttestationState::Valid, _) => "valid".to_owned(),
        (AttestationState::Absent, _) => "absent".to_owned(),
        (AttestationState::Void, reason) => format!(
            "void ({})",
            reason.map_or("unknown", |reason| match reason {
                onetaskgraph_core::attestation::VoidReason::ContentMoved => "content-moved",
                onetaskgraph_core::attestation::VoidReason::BarMoved => "bar-moved",
            })
        ),
    }
}

fn checked_text(check: &AttestationCheck) -> String {
    format!("{}: {}", check.kind, state(check))
}

fn tasks_text(checked: &ProjectAttestationCheck) -> String {
    if checked.tasks.is_empty() {
        return format!("{}: no task to check", checked.project);
    }
    checked
        .tasks
        .iter()
        .map(|task| format!("{}\t{}", task.id, checked_text(&task.check)))
        .collect::<Vec<_>>()
        .join("\n")
}
