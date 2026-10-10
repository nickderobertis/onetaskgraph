//! The public boundary: what may be written where, decided once for every writer.
//!
//! Every write this engine makes passes through here before its first mutation. Two facts
//! decide it, and the store owns only the first:
//!
//! - **Where it lands.** A source's configuration declares its `visibility`, and a source
//!   declared private is held to its backend's own live answer at every write
//!   ([`TaskSource::visibility`]). An item that may only be written somewhere private is
//!   written only there.
//! - **What it says.** A write reaching a destination that is not verified private is put to
//!   the caller's [`WritePolicy`], which decides whether its text names anything private.
//!   The store never derives a term itself, and never decides a repository's visibility
//!   itself: both are the policy's, so a linking caller supplies its own and the command line
//!   runs the commands `write_policy` names ([`CommandWritePolicy`]).
//!
//! **The boundary is active** when the configuration names a `write_policy`, when any source
//! declares a `visibility`, or when a linking caller supplies a [`WritePolicy`]
//! ([`Engine::with_write_policy`]). An inactive store still persists, copies and routes
//! classification, and still refuses an explicitly private item anywhere not declared private
//! — but it consults no repository's visibility and reads nothing extra before a narrow write,
//! so a store that has not opted in answers and spends exactly what it did before. A store that
//! wants repositories screened has to configure `write_policy` or declare a visibility.
//!
//! [`TaskSource::visibility`]: onetaskgraph_plugin_api::TaskSource::visibility

use std::collections::BTreeMap;
use std::io::Write;
use std::process::Stdio;
use std::sync::{Arc, LazyLock};

use onetaskgraph_plugin_api::{
    Classification, Document, Label, NativeId, Project, Repository, Task, Visibility, WriteTarget,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{PolicyCommand, SourceVisibility, WritePolicyConfig};
use crate::resolve::ResolvedSource;

use super::{Engine, EngineError};

/// Whether a repository is public, as a caller's policy answers.
///
/// Only `public` is public: `unknown` is what a policy answers when it cannot say, and the
/// store treats it exactly as `private`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RepositoryVisibility {
    /// Anybody can read it.
    Public,
    /// It is private, or internal to an organisation.
    Private,
    /// Nobody has said. Treated as private.
    Unknown,
}

impl RepositoryVisibility {
    /// Whether this is [`Public`](Self::Public), the one value that is.
    #[must_use]
    pub const fn is_public(self) -> bool {
        matches!(self, Self::Public)
    }
}

/// What one write is about to put somewhere, put to [`WritePolicy::check_public_write`].
///
/// Serialized exactly as onevcs's `BoundaryInput` reads it, and the reconciliation journey
/// holds the two to one schema: `scope` is left out when absent, which a check reads as every
/// private identity it knows of, and an empty list derives no term at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicWriteInput {
    /// Where it is going.
    pub destination: RepositoryVisibility,
    /// Prose and file contents.
    #[serde(default)]
    pub text: Vec<String>,
    /// Paths it names or writes.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Titles, labels, metadata and every other short field.
    #[serde(default)]
    pub metadata: Vec<String>,
    /// Which repositories' private terms the check derives, as the caller scoped it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<String>>,
}

/// What a check decided about one write.
///
/// Every reason is neutral — it says where the check stopped and never what it found — so a
/// refusal can be printed wherever the write was asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "verdict", rename_all = "kebab-case")]
pub enum WriteVerdict {
    /// Nothing private was found.
    Pass,
    /// Something private was found, and the write must not go ahead.
    Refuse {
        /// Where, in neutral words.
        reason: String,
    },
    /// The check could not decide, which is never a pass.
    Unavailable {
        /// Why, in neutral words.
        reason: String,
    },
}

/// A policy that could not be asked at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct PolicyError {
    /// What went wrong, in neutral words.
    pub message: String,
}

impl PolicyError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// The caller's half of the public boundary: which repositories are public, and whether a
/// write may reach a public destination.
///
/// A linking caller implements it directly and hands it to [`Engine::with_write_policy`];
/// the command line and both SDKs run the commands a configuration's `write_policy` names
/// through [`CommandWritePolicy`]. Either way the store applies one rule over its answers,
/// and **nothing here is allowed to pass by failing**: an error answering a repository's
/// visibility makes it private, and an error checking a write makes the check unavailable.
#[async_trait::async_trait]
pub trait WritePolicy: Send + Sync {
    /// Whether `repository` is public.
    ///
    /// # Errors
    ///
    /// [`PolicyError`] when it cannot be asked; the store treats that as `unknown`.
    async fn repository_visibility(
        &self,
        repository: &Repository,
    ) -> Result<RepositoryVisibility, PolicyError>;

    /// Whether `input` may be written to its destination.
    ///
    /// # Errors
    ///
    /// [`PolicyError`] when it cannot be asked; the store treats that as unavailable.
    async fn check_public_write(
        &self,
        input: &PublicWriteInput,
    ) -> Result<WriteVerdict, PolicyError>;
}

/// The policy a store with no `write_policy` asks: every repository unknown, every check
/// unavailable — so an active store without one treats every repository's item as private,
/// which only a destination verified private takes, and writes nothing at all to any other.
#[derive(Debug, Clone, Copy, Default)]
pub struct MissingWritePolicy;

#[async_trait::async_trait]
impl WritePolicy for MissingWritePolicy {
    async fn repository_visibility(
        &self,
        _repository: &Repository,
    ) -> Result<RepositoryVisibility, PolicyError> {
        Ok(RepositoryVisibility::Unknown)
    }

    async fn check_public_write(
        &self,
        _input: &PublicWriteInput,
    ) -> Result<WriteVerdict, PolicyError> {
        Ok(WriteVerdict::Unavailable {
            reason: "no write_policy.check_command is configured".to_owned(),
        })
    }
}

/// The schemas onevcs's `boundary` commands exchange, as the released `onevcs boundary schema
/// --json` printed them. What this adapter sends and accepts is validated against them, and
/// the reconciliation journey fails the moment the released command prints anything else.
pub const ONEVCS_BOUNDARY_SCHEMA: &str = include_str!("boundary/onevcs-boundary-schema-v1.json");

/// The `schema_version` of [`ONEVCS_BOUNDARY_SCHEMA`], which an answer is held to.
pub const ONEVCS_BOUNDARY_SCHEMA_VERSION: u64 = 1;

/// One compiled validator per schema half, built once.
struct Validators {
    inspect_input: jsonschema::Validator,
    inspect_output: jsonschema::Validator,
    check_input: jsonschema::Validator,
    check_output: jsonschema::Validator,
}

static VALIDATORS: LazyLock<Validators> = LazyLock::new(|| {
    let schema: Value =
        serde_json::from_str(ONEVCS_BOUNDARY_SCHEMA).expect("the pinned boundary schema is JSON");
    assert_eq!(
        schema.get("schema_version").and_then(Value::as_u64),
        Some(ONEVCS_BOUNDARY_SCHEMA_VERSION),
        "the pinned boundary schema is the version this adapter speaks"
    );
    let compile = |pointer: &str| {
        jsonschema::validator_for(schema.pointer(pointer).expect("the pinned schema has it"))
            .expect("the pinned boundary schema compiles")
    };
    Validators {
        inspect_input: compile("/inspect/input"),
        inspect_output: compile("/inspect/output"),
        check_input: compile("/check/input"),
        check_output: compile("/check/output"),
    }
});

/// The policy behind `write_policy`: two generic commands, each handed one JSON document on
/// its standard input.
///
/// - `visibility_command` is handed `{"repository": "<host/owner/name>"}` and answers
///   `{"visibility": …}` on its standard output, exiting 0. Anything else — no command, a
///   non-zero exit, an answer the pinned schema refuses — is `unknown`, which is private.
/// - `check_command` is handed the whole [`PublicWriteInput`] and **passes a write only by
///   exiting 0 with `{"verdict": "pass"}`**. Exit 1 with a refusal is a refusal; every other
///   outcome, a missing command included, is unavailable.
///
/// Both the documents sent and the answers read are held to onevcs's versioned boundary
/// schema ([`ONEVCS_BOUNDARY_SCHEMA`]), so a command speaking anything else is not believed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandWritePolicy {
    visibility_command: Option<PolicyCommand>,
    check_command: Option<PolicyCommand>,
}

impl CommandWritePolicy {
    /// The policy a configuration's `write_policy` names.
    #[must_use]
    pub fn new(config: &WritePolicyConfig) -> Self {
        Self {
            visibility_command: config.visibility_command.clone(),
            check_command: config.check_command.clone(),
        }
    }
}

/// One command run to completion over `input`: its exit code, if it exited, and its stdout.
///
/// Run on a thread of its own and awaited through a channel, as a hosted plugin's process is,
/// so the engine starts no runtime of its own and blocks none its caller started.
async fn run(
    command: &PolicyCommand,
    input: &Value,
) -> Result<(Option<i32>, Vec<u8>), PolicyError> {
    let program = command.program().to_owned();
    let arguments = command.arguments().to_vec();
    let document = serde_json::to_vec(input).expect("a JSON value serializes");
    let (answered, answer) = tokio::sync::oneshot::channel();
    let runner = std::thread::Builder::new().name("onetaskgraph-policy".to_owned());
    runner
        .spawn(move || {
            let ran = (|| {
                let mut child = std::process::Command::new(&program)
                    .args(&arguments)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| {
                        PolicyError::new(format!(
                            "the command {program} could not be started: {error}"
                        ))
                    })?;
                if let Some(mut stdin) = child.stdin.take() {
                    // A command that exits without reading its input closes the pipe; what it
                    // answered is still read below, and decides.
                    let _ = stdin.write_all(&document);
                }
                let output = child.wait_with_output().map_err(|error| {
                    PolicyError::new(format!("the command {program} could not be read: {error}"))
                })?;
                Ok((output.status.code(), output.stdout))
            })();
            let _ = answered.send(ran);
        })
        .map_err(|error| {
            PolicyError::new(format!(
                "the command {} could not be run: {error}",
                command.program()
            ))
        })?;
    answer
        .await
        .map_err(|_| PolicyError::new("the command's runner stopped without an answer"))?
}

/// `bytes` as one JSON document the validator accepts, or `None`.
fn validated(bytes: &[u8], validator: &jsonschema::Validator) -> Option<Value> {
    let answer: Value = serde_json::from_slice(bytes).ok()?;
    validator.is_valid(&answer).then_some(answer)
}

#[async_trait::async_trait]
impl WritePolicy for CommandWritePolicy {
    async fn repository_visibility(
        &self,
        repository: &Repository,
    ) -> Result<RepositoryVisibility, PolicyError> {
        let Some(command) = &self.visibility_command else {
            return Ok(RepositoryVisibility::Unknown);
        };
        let request = serde_json::json!({ "repository": repository.as_str() });
        debug_assert!(VALIDATORS.inspect_input.is_valid(&request));
        let (code, stdout) = run(command, &request).await?;
        if code != Some(0) {
            return Err(PolicyError::new(format!(
                "the visibility command {} did not answer",
                command.program()
            )));
        }
        let answer = validated(&stdout, &VALIDATORS.inspect_output).ok_or_else(|| {
            PolicyError::new(format!(
                "the visibility command {} answered something the boundary schema refuses",
                command.program()
            ))
        })?;
        serde_json::from_value(answer["visibility"].clone())
            .map_err(|error| PolicyError::new(error.to_string()))
    }

    async fn check_public_write(
        &self,
        input: &PublicWriteInput,
    ) -> Result<WriteVerdict, PolicyError> {
        let Some(command) = &self.check_command else {
            return MissingWritePolicy.check_public_write(input).await;
        };
        let document = serde_json::to_value(input).expect("a write input serializes");
        if !VALIDATORS.check_input.is_valid(&document) {
            return Ok(WriteVerdict::Unavailable {
                reason: "the write could not be put in the shape the boundary schema reads"
                    .to_owned(),
            });
        }
        let (code, stdout) = run(command, &document).await?;
        Ok(check_verdict(code, &stdout))
    }
}

/// What a check command's exit `code` and standard output amount to — the one reading of a
/// `boundary check` answer, held to the pinned schema.
///
/// **Only an explicit pass passes**: exit 0 with `{"verdict": "pass"}`. Exit 1 with a refusal
/// is a refusal naming where the check stopped and never what it found; an `unavailable`
/// answer, any other exit, a missing exit and anything the schema refuses are unavailable.
#[must_use]
pub fn check_verdict(code: Option<i32>, stdout: &[u8]) -> WriteVerdict {
    let answer = validated(stdout, &VALIDATORS.check_output);
    let field = |name: &str| {
        answer
            .as_ref()
            .and_then(|answer| answer.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match (code, field("verdict").as_deref()) {
        (Some(0), Some("pass")) => WriteVerdict::Pass,
        (Some(1), Some("refuse")) => WriteVerdict::Refuse {
            reason: format!(
                "it carries a term of a private repository in its {}",
                field("surface").unwrap_or_else(|| "content".to_owned())
            ),
        },
        (_, Some("unavailable")) => WriteVerdict::Unavailable {
            reason: format!(
                "the check could not decide ({})",
                field("reason").unwrap_or_else(|| "no reason given".to_owned())
            ),
        },
        (code, _) => WriteVerdict::Unavailable {
            reason: format!(
                "the check command {} without a verdict this store believes",
                code.map_or_else(|| "was stopped".to_owned(), |code| format!("exited {code}"))
            ),
        },
    }
}

/// How the boundary stands for one engine: active or not, the policy it asks, and the term
/// scope its checks carry.
#[derive(Clone)]
pub(crate) struct Boundary {
    active: bool,
    policy: Arc<dyn WritePolicy>,
    scope: Option<Vec<String>>,
}

impl Default for Boundary {
    fn default() -> Self {
        Self {
            active: false,
            policy: Arc::new(MissingWritePolicy),
            scope: None,
        }
    }
}

impl Boundary {
    /// The boundary a configuration asks for.
    pub(crate) fn configured(
        write_policy: Option<&WritePolicyConfig>,
        visibilities: &BTreeMap<onetaskgraph_plugin_api::SourceName, SourceVisibility>,
    ) -> Self {
        Self {
            active: write_policy.is_some()
                || visibilities
                    .values()
                    .any(|declared| *declared != SourceVisibility::Unknown),
            policy: match write_policy {
                Some(config) => Arc::new(CommandWritePolicy::new(config)),
                None => Arc::new(MissingWritePolicy),
            },
            scope: None,
        }
    }
}

/// What one write puts in front of a reader, by the surfaces a check reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Exposure {
    text: Vec<String>,
    paths: Vec<String>,
    metadata: Vec<String>,
}

impl Exposure {
    /// Prose a write carries — a comment's body, a rendering.
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            text: vec![text.into()],
            ..Self::default()
        }
    }

    /// A short field a write carries — a key, a value, a status name.
    pub(crate) fn metadata(values: impl IntoIterator<Item = String>) -> Self {
        Self {
            metadata: values.into_iter().collect(),
            ..Self::default()
        }
    }

    pub(crate) fn and(mut self, other: Self) -> Self {
        self.text.extend(other.text);
        self.paths.extend(other.paths);
        self.metadata.extend(other.metadata);
        self
    }

    /// Paths a write names — an image asset's name.
    pub(crate) fn paths(mut self, paths: impl IntoIterator<Item = String>) -> Self {
        self.paths.extend(paths);
        self
    }

    /// The names of the image assets a write stores, which are paths.
    pub(crate) fn assets(self, assets: &[onetaskgraph_plugin_api::AssetPayload]) -> Self {
        self.paths(assets.iter().map(|asset| asset.name.as_str().to_owned()))
    }

    /// The answers a rendering stores beside its content, where its source keeps them.
    pub(crate) fn answers(mut self, answers: Option<&BTreeMap<String, Value>>) -> Self {
        for (key, value) in answers.into_iter().flatten() {
            self.metadata.push(key.clone());
            strings(value, &mut self);
        }
        self
    }

    /// One metadata entry a write sets: its key and every string in its value.
    pub(crate) fn of_entry(key: &str, value: &Value) -> Self {
        let mut exposure = Self::metadata([key.to_owned()]);
        strings(value, &mut exposure);
        exposure
    }

    /// Everything a targeted update writes.
    pub(crate) fn of_update(update: &onetaskgraph_plugin_api::TaskUpdate) -> Self {
        let mut exposure = Self {
            text: update.content.iter().cloned().collect(),
            ..Self::default()
        };
        exposure.metadata.extend(update.title.iter().cloned());
        exposure
            .metadata
            .extend(update.status.iter().map(|status| status.name.clone()));
        exposure.metadata.extend(
            update
                .priority
                .iter()
                .map(|priority| priority.as_str().to_owned()),
        );
        for (key, value) in &update.metadata_set {
            exposure = exposure.and(Self::of_entry(key.as_str(), value));
        }
        exposure
            .metadata
            .extend(update.delivers.iter().flatten().map(ToString::to_string));
        exposure.metadata.extend(
            update
                .depends_on
                .iter()
                .flatten()
                .map(|edge| edge.to.id().to_owned()),
        );
        exposure
    }

    /// Everything a task carries that a reader of it would read.
    pub(crate) fn of_task(task: &Task) -> Self {
        let mut exposure = Self::of_record(
            &task.title,
            task.content.as_deref(),
            &task.labels,
            &task.metadata,
            &task.repositories,
        );
        exposure.metadata.push(task.status.name.clone());
        exposure
            .metadata
            .extend(task.project.iter().map(|id| id.0.clone()));
        exposure.metadata.extend(
            task.delivers
                .iter()
                .chain(&task.delivered_by)
                .map(ToString::to_string),
        );
        exposure
    }

    /// Everything a project carries that a reader of it would read.
    pub(crate) fn of_project(project: &Project) -> Self {
        let mut exposure = Self::of_record(
            &project.title,
            project.content.as_deref(),
            &project.labels,
            &project.metadata,
            &project.repositories,
        );
        exposure.metadata.push(project.status.name.clone());
        exposure
    }

    /// Everything a document carries that a reader of it would read.
    pub(crate) fn of_document(document: &Document) -> Self {
        let mut exposure = Self::of_record(
            &document.title,
            document.content.as_deref(),
            &document.labels,
            &document.metadata,
            &document.repositories,
        );
        exposure
            .metadata
            .extend(document.project.iter().map(|id| id.0.clone()));
        exposure
    }

    fn of_record(
        title: &str,
        content: Option<&str>,
        labels: &[Label],
        metadata: &BTreeMap<String, Value>,
        repositories: &[Repository],
    ) -> Self {
        let mut exposure = Self {
            text: content.map(str::to_owned).into_iter().collect(),
            ..Self::default()
        };
        exposure.metadata.push(title.to_owned());
        exposure
            .metadata
            .extend(labels.iter().map(|label| label.name.clone()));
        exposure.metadata.extend(
            repositories
                .iter()
                .map(|repository| repository.as_str().to_owned()),
        );
        for (key, value) in metadata {
            exposure.metadata.push(key.clone());
            strings(value, &mut exposure);
        }
        exposure
    }
}

/// Every string inside `value`, keys included. A string that reads as a path — the template
/// reference a rendering records, an asset's place — is a path as well, so a check reading
/// paths reads it there too.
fn strings(value: &Value, exposure: &mut Exposure) {
    match value {
        Value::String(text) => {
            if text.contains('/') || text.contains('\\') {
                exposure.paths.push(text.clone());
            }
            exposure.metadata.push(text.clone());
        }
        Value::Array(values) => values.iter().for_each(|value| strings(value, exposure)),
        Value::Object(fields) => {
            for (key, value) in fields {
                exposure.metadata.push(key.clone());
                strings(value, exposure);
            }
        }
        Value::Number(number) => exposure.metadata.push(number.to_string()),
        Value::Bool(_) | Value::Null => {}
    }
}

/// One write, as the boundary is asked about it.
pub(crate) struct Outbound<'a> {
    /// How the write names the item, for a refusal: a qualified id, or "the new task".
    pub(crate) item: String,
    /// The item's classification, already tightened by its repositories and its project.
    pub(crate) classification: Classification,
    /// Where in the destination it lands.
    pub(crate) target: WriteTarget<'a>,
    /// What it puts there.
    pub(crate) exposure: &'a Exposure,
}

impl Engine {
    /// The same engine, asking `policy` about the public boundary — which makes the boundary
    /// active, whatever the configuration says.
    #[must_use]
    pub fn with_write_policy(mut self, policy: Arc<dyn WritePolicy>) -> Self {
        self.boundary.active = true;
        self.boundary.policy = policy;
        self
    }

    /// The same engine, deriving every check's terms from `scope`: `None` for every private
    /// identity the policy knows of, an empty list for none.
    #[must_use]
    pub fn with_term_scope(mut self, scope: Option<Vec<String>>) -> Self {
        self.boundary.scope = scope;
        self
    }

    /// Whether the boundary is active for this engine (see the module documentation).
    #[must_use]
    pub fn boundary_active(&self) -> bool {
        self.boundary.active
    }

    /// Refuse a write to `destination` that names, at the caller's own request, an item of a
    /// source declared private — a `--depends-on` or a `--delivers` of a create or an update —
    /// when `destination` is not itself declared private.
    ///
    /// Such a reference is a qualified id, `<source>:<id>`, and the source's name is in no
    /// term list unless it happens to match a private repository: so it is refused here,
    /// whatever the check would say, rather than written where it names somewhere private.
    /// A copy and a delivered task's back-reference withhold the same ids instead (see
    /// [`Engine::withholds`]), because there the id is what the engine carries along rather
    /// than what the caller asked to be written. `references` are `(source, native id)` pairs;
    /// one naming `destination` itself is that source's own item and names nothing elsewhere.
    /// While no source is declared private nothing can be refused, which keeps an inactive
    /// store exactly as it was.
    pub(crate) fn refuse_private_references<'a>(
        &self,
        destination: &onetaskgraph_plugin_api::SourceName,
        item: &str,
        references: impl IntoIterator<Item = &'a crate::GlobalId>,
    ) -> Result<(), EngineError> {
        if self.declared(destination) == SourceVisibility::Private {
            return Ok(());
        }
        for reference in references {
            if &reference.source != destination
                && self.declared(&reference.source) == SourceVisibility::Private
            {
                return Err(EngineError::PrivateReference {
                    item: item.to_owned(),
                    destination: destination.to_string(),
                    named: reference.source.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Whether a write to `destination` withholds an id naming `far`: an item of another source
    /// declared private, or one of the items in `private` — those this command knows are
    /// classified private — while `destination` is not itself declared private.
    ///
    /// A withheld id is left out of what that write carries — a dependency edge's far end, a
    /// `delivers` entry a copy carries along, a delivered task's `delivered_by` — and stays
    /// in the private store's own record, exactly as an `onetaskgraph.origin` or an
    /// `onetaskgraph.copies` link naming a private source does. Nothing is withheld while no
    /// source is declared private and no item is classified private.
    pub(crate) fn withholds(
        &self,
        destination: &onetaskgraph_plugin_api::SourceName,
        far: &crate::GlobalId,
        private: &[crate::GlobalId],
    ) -> bool {
        &far.source != destination
            && self.declared(destination) != SourceVisibility::Private
            && (self.declared(&far.source) == SourceVisibility::Private || private.contains(far))
    }

    /// Who `source` is declared readable by.
    pub(crate) fn declared(
        &self,
        source: &onetaskgraph_plugin_api::SourceName,
    ) -> SourceVisibility {
        self.routes.visibility(source)
    }

    /// `declared`, tightened by `repositories` — consulted only while the boundary is active,
    /// and any repository the policy does not answer `public` for makes it private.
    pub(crate) async fn classify(
        &self,
        declared: Classification,
        repositories: &[Repository],
    ) -> Classification {
        if !self.boundary.active || declared == Classification::Private {
            return declared;
        }
        for repository in repositories {
            let visibility = self
                .boundary
                .policy
                .repository_visibility(repository)
                .await
                .unwrap_or(RepositoryVisibility::Unknown);
            if !visibility.is_public() {
                return Classification::Private;
            }
        }
        declared
    }

    /// Admit one write to `destination`, or refuse it before anything is mutated:
    /// [`preflight`](Self::preflight), then [`at_write`](Self::at_write).
    pub(crate) async fn admit(
        &self,
        destination: &ResolvedSource,
        outbound: &Outbound<'_>,
    ) -> Result<(), EngineError> {
        self.preflight(destination, outbound).await?;
        self.at_write(destination, &outbound.item, &outbound.target)
            .await
    }

    /// Everything about one write that can be settled before anything is written: a private
    /// item refused where it is not declared private, and — while the boundary is active — a
    /// write to a destination not declared private put to the caller's check.
    ///
    /// A destination declared private is not checked for terms: its reality is held to its
    /// declaration at the write itself ([`at_write`](Self::at_write)), and a write there that
    /// finds it not private is refused there.
    pub(crate) async fn preflight(
        &self,
        destination: &ResolvedSource,
        outbound: &Outbound<'_>,
    ) -> Result<(), EngineError> {
        let name = destination.name();
        let declared = self.declared(name);
        if outbound.classification == Classification::Private
            && declared != SourceVisibility::Private
        {
            return Err(EngineError::NotPrivateDestination {
                item: outbound.item.clone(),
                destination: name.to_string(),
                declared,
            });
        }
        if declared == SourceVisibility::Private || !self.boundary.active {
            return Ok(());
        }
        self.check(destination, &outbound.item, outbound.exposure)
            .await
    }

    /// The check every write makes immediately before its first mutation: a destination
    /// declared private is read live, every time, because nothing read for an earlier write
    /// says what the backend is now.
    pub(crate) async fn at_write(
        &self,
        destination: &ResolvedSource,
        item: &str,
        target: &WriteTarget<'_>,
    ) -> Result<(), EngineError> {
        if self.declared(destination.name()) != SourceVisibility::Private {
            return Ok(());
        }
        self.verify_private(destination, item, target).await
    }

    /// Hold a source declared private to its backend's live answer.
    async fn verify_private(
        &self,
        destination: &ResolvedSource,
        item: &str,
        target: &WriteTarget<'_>,
    ) -> Result<(), EngineError> {
        match destination.source().visibility(target).await {
            Ok(Visibility::Private) => Ok(()),
            Ok(reality) => Err(EngineError::DestinationNotPrivate {
                item: item.to_owned(),
                destination: destination.name().to_string(),
                reality,
            }),
            Err(error) => Err(EngineError::VisibilityUnreadable {
                item: item.to_owned(),
                destination: destination.name().to_string(),
                error,
            }),
        }
    }

    /// Put one write to a destination not verified private to the caller's check.
    pub(crate) async fn check(
        &self,
        destination: &ResolvedSource,
        item: &str,
        exposure: &Exposure,
    ) -> Result<(), EngineError> {
        let input = PublicWriteInput {
            destination: RepositoryVisibility::Public,
            text: exposure.text.clone(),
            paths: exposure.paths.clone(),
            metadata: exposure.metadata.clone(),
            scope: self.boundary.scope.clone(),
        };
        let verdict = self
            .boundary
            .policy
            .check_public_write(&input)
            .await
            .unwrap_or_else(|error| WriteVerdict::Unavailable {
                reason: error.message,
            });
        match verdict {
            WriteVerdict::Pass => Ok(()),
            WriteVerdict::Refuse { reason } => Err(EngineError::BoundaryRefused {
                item: item.to_owned(),
                destination: destination.name().to_string(),
                reason,
            }),
            WriteVerdict::Unavailable { reason } => Err(EngineError::BoundaryUnavailable {
                item: item.to_owned(),
                destination: destination.name().to_string(),
                reason,
            }),
        }
    }

    /// `classification`, tightened by the project `project` of `source` it is filed under —
    /// refused when it is private and the project is held public, because a public project
    /// never holds a private member.
    ///
    /// The project is read always while the boundary is active, and for a private item while it
    /// is not, which is refused under a public project either way. A public item in an inactive
    /// store inherits nothing, although its project may be private by hand: reading the project
    /// would be a request a store that has not opted in never made.
    pub(crate) async fn filed_under(
        &self,
        source: &ResolvedSource,
        project: &NativeId,
        item: &str,
        classification: Classification,
    ) -> Result<Classification, EngineError> {
        if !self.boundary.active && classification.is_public() {
            return Ok(classification);
        }
        let Some(held) = source
            .source()
            .get_project(project)
            .await
            .map_err(|error| EngineError::SourceRefused {
                name: source.name().to_string(),
                error,
            })?
        else {
            return Ok(classification);
        };
        let filed = self.classify(held.classification, &held.repositories).await;
        if classification == Classification::Private && filed.is_public() {
            return Err(EngineError::PrivateMemberOfPublicProject {
                item: item.to_owned(),
                project: format!("{}:{}", source.name(), project),
            });
        }
        Ok(classification.strictest(filed))
    }

    /// Admit one narrow write to an item `destination` already holds.
    ///
    /// A destination verified private takes it with no read beyond its reality. Any other —
    /// while the boundary is active — has the item read once for its classification, refuses
    /// a private one, and puts what the write adds to the check. An inactive boundary reads
    /// nothing and admits it.
    pub(crate) async fn admit_existing(
        &self,
        destination: &ResolvedSource,
        item: &str,
        id: &NativeId,
        held: Held,
        exposure: &Exposure,
    ) -> Result<(), EngineError> {
        let target = WriteTarget::Existing(id);
        if self.declared(destination.name()) == SourceVisibility::Private {
            return self.verify_private(destination, item, &target).await;
        }
        if !self.boundary.active {
            return Ok(());
        }
        let stored = match held {
            Held::Task => destination
                .source()
                .get_task(id)
                .await
                .map(|task| task.map(|task| (task.classification, task.repositories))),
            Held::Project => destination.source().get_project(id).await.map(|project| {
                project.map(|project| (project.classification, project.repositories))
            }),
            Held::Document => destination.source().get_document(id).await.map(|document| {
                document.map(|document| (document.classification, document.repositories))
            }),
        }
        .map_err(|error| EngineError::SourceRefused {
            name: destination.name().to_string(),
            error,
        })?;
        // An item the source does not hold is the write's own refusal to make, in its own
        // words; nothing of it can be private.
        let Some((declared, repositories)) = stored else {
            return Ok(());
        };
        self.admit_known(destination, item, id, declared, &repositories, exposure)
            .await
    }

    /// Admit one write to an item `destination` holds, whose classification and repositories
    /// the caller has already read — so nothing more is read here but a private destination's
    /// reality.
    pub(crate) async fn admit_known(
        &self,
        destination: &ResolvedSource,
        item: &str,
        id: &NativeId,
        declared: Classification,
        repositories: &[Repository],
        exposure: &Exposure,
    ) -> Result<(), EngineError> {
        let classification = self.classify(declared, repositories).await;
        self.admit(
            destination,
            &Outbound {
                item: item.to_owned(),
                classification,
                target: WriteTarget::Existing(id),
                exposure,
            },
        )
        .await
    }
}

/// Which kind of record a narrow write is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Held {
    Task,
    Project,
    Document,
}
