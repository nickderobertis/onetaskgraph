//! The engine's half of the protocol: a [`TaskSource`] that is another process.
//!
//! Every method here is one line out and one line back. What it deliberately does *not*
//! do is decide anything: a `forward-only` plugin is never asked for
//! [`Direction::DependedOnBy`] because the layer above reads that off the capabilities
//! this handshake returned and emulates the reverse scan itself, and a predicate a plugin
//! declared unsupported is removed from the query before it ever reaches here. Putting
//! either decision in this file would give the product a second compensation layer that
//! only subprocess-hosted sources went through.

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use onetaskgraph_plugin_api::{
    Capabilities, Comment, CommentBody, DependencyEdge, Direction, Document, DocumentQuery, Health,
    ItemWrite, Label, MetadataKey, MetadataRecord, Metering, NativeId, NewComment, Page,
    PageRequest, Priority, Project, ProjectQuery, SourceError, SourceName, Status, StatusCategory,
    Task, TaskQuery, TaskRef, TaskSource, WriteSupport, unwritable_field, unwritable_metadata,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::connection::{Connection, Peer};
use super::wire::{
    AddCommentParams, CommentResult, CommentsParams, CommentsResult, ContentParams, ContentResult,
    DeleteCommentParams, DeleteParams, DeletedCommentResult, DeliveredByParams, DeliveredByResult,
    DependencyParams, DocumentDir, DocumentQueryParams, DocumentResult, DocumentWriteParams,
    EditCommentParams, EngineIdentity, IdParams, InitializeParams, InitializeResult, LabelParams,
    MetadataParams, MeteringResult, PROTOCOL_VERSION, PriorityParams, PriorityResult,
    ProjectQueryParams, ProjectResult, ProjectWriteParams, Request, StatusParams, StatusResult,
    TaskQueryParams, TaskResult, TaskWriteParams, WriteResult, after_the_first_vocabulary,
    knows_every_category, spelled, vocabulary,
};

/// The id the handshake is sent under. §3 makes it the first request on a connection, so
/// nothing else can have been sent under it, and an answer addressed elsewhere is a
/// violation rather than an ordering the engine could accommodate.
const HANDSHAKE_ID: &str = "0";

/// A positive per-request deadline, measured in milliseconds at the configuration edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestDeadline(NonZeroU64);

impl RequestDeadline {
    /// The protocol's default deadline.
    pub const DEFAULT: Self = Self(NonZeroU64::new(30_000).expect("non-zero default"));

    /// Validate a millisecond value from a configuration or another public boundary.
    #[must_use]
    pub const fn from_millis(milliseconds: NonZeroU64) -> Self {
        Self(milliseconds)
    }

    /// The positive millisecond count used by configuration and diagnostics.
    #[must_use]
    pub const fn milliseconds(self) -> NonZeroU64 {
        self.0
    }

    fn duration(self) -> Duration {
        Duration::from_millis(self.0.get())
    }
}

/// The two bounds a spawned plugin's exchanges are held to, carried together so that the
/// one constructor every other reaches takes a pair rather than two arguments of one type
/// a caller could transpose.
#[derive(Debug, Clone, Copy)]
struct Deadlines {
    /// Bounds the `initialize` exchange, and so the child's own start-up with it.
    handshake: RequestDeadline,
    /// Bounds each exchange after the handshake, against an already running child.
    requests: RequestDeadline,
}

/// A source served by a spawned program speaking `docs/plugin-protocol.md`.
pub struct SubprocessSource {
    /// What the plugin called itself in the handshake.
    ///
    /// Leaked once per connection because [`TaskSource::kind`] returns `&'static str` for
    /// the compiled-in plugins, whose kinds really are static, and a subprocess-hosted
    /// plugin's kind is not known until it answers. One small allocation per configured
    /// source, for the life of a process that was going to hold that source anyway, is
    /// the cheapest way to keep the trait honest for both.
    kind: &'static str,
    /// Read once at the handshake; §3 says the engine does not ask again.
    capabilities: Capabilities,
    /// Whether the plugin said it can be written through, read at the same handshake.
    ///
    /// A plugin that said nothing is read as read-only, which is what §3.3 makes an
    /// absent member mean and what every version-1 plugin written before there was a
    /// write side is.
    writes: WriteSupport,
    /// Whether the plugin said it answers `metering`, read at the same handshake.
    ///
    /// A plugin that said nothing is never sent the method and is reported as not metering,
    /// which is what §3.4 makes an absent member mean.
    meters: bool,
    /// Whether the plugin's handshake listed every status category this build knows (§3.5).
    ///
    /// A plugin that listed none was written against the first vocabulary, and is never
    /// handed a category added after it: not in a query, not in a write.
    knows_every_category: bool,
    /// Whether the plugin said it answers the two narrow task writes and holds a task's two
    /// lists (§3.6), read at the same handshake.
    task_updates: bool,
    /// Whether the plugin said it answers the three narrow metadata writes (§3.7), read at the
    /// same handshake.
    metadata_updates: bool,
    /// Whether the plugin said it answers the narrow content write (§3.9), read at the same
    /// handshake.
    content_updates: bool,
    /// The live process.
    connection: Connection,
}

impl std::fmt::Debug for SubprocessSource {
    /// Named without its connection, which holds a live child and a credential the
    /// handshake forwarded — neither belongs in a diagnostic.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubprocessSource")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl SubprocessSource {
    /// Spawn `program`, complete the handshake, and adopt the connection.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Unavailable`] when the program cannot be run or stops
    /// answering, the plugin's own error when it refuses the handshake, and
    /// [`SourceError::Config`] when the two sides do not speak the same protocol version
    /// — refused by name, never guessed at (§6.1).
    pub fn connect(
        program: &str,
        args: &[String],
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
    ) -> Result<Self, SourceError> {
        Self::connect_with_deadline(
            program,
            args,
            name,
            config,
            secrets,
            RequestDeadline::DEFAULT,
        )
    }

    /// Spawn a plugin with a deadline applying independently to every exchange.
    ///
    /// # Errors
    ///
    /// What [`connect`](Self::connect) returns.
    pub fn connect_with_deadline(
        program: &str,
        args: &[String],
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
        deadline: RequestDeadline,
    ) -> Result<Self, SourceError> {
        Self::connect_with_deadlines(program, args, name, config, secrets, deadline, deadline)
    }

    /// Spawn a plugin, bounding the initialization handshake and every later request
    /// separately.
    ///
    /// `handshake` bounds the one `initialize` exchange of §3, and a newly spawned child
    /// does its own starting up inside that exchange: whatever a runtime loads before it
    /// can read its first line is counted against this bound. `requests` bounds every
    /// exchange after the handshake succeeded, each on its own, against a child that is by
    /// then already running. They are the same bound in
    /// [`connect_with_deadline`](Self::connect_with_deadline) and in every configured
    /// source, because one `deadline_ms` is what `docs/plugin-protocol.md` §1 gives a user
    /// to set. Separating them is for a caller that means to hold a *request* to a span
    /// shorter than a program takes to start — the engine's own probe of what a silent
    /// child's expired request reports — where one bound would fail the handshake on a
    /// loaded host instead of reaching the behaviour it was after.
    ///
    /// # Errors
    ///
    /// What [`connect`](Self::connect) returns.
    pub fn connect_with_deadlines(
        program: &str,
        args: &[String],
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
        handshake: RequestDeadline,
        requests: RequestDeadline,
    ) -> Result<Self, SourceError> {
        Self::connect_bounded(
            program,
            args,
            name,
            config,
            secrets,
            Deadlines {
                handshake,
                requests,
            },
            None,
        )
    }

    /// Spawn a plugin, telling it which document's directory its settings are measured
    /// from — `document_dir` in `docs/plugin-protocol.md` §3, sent only when there is one.
    ///
    /// # Errors
    ///
    /// What [`connect`](Self::connect) returns, and [`SourceError::Config`] when
    /// `document_dir` is not an absolute directory whose name is valid UTF-8, and so
    /// cannot be written into the handshake — refused rather than dropped, because dropping
    /// it would silently measure the child's paths from its working directory instead.
    pub fn connect_from_document(
        program: &str,
        args: &[String],
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
        deadline: RequestDeadline,
        document_dir: Option<&Path>,
    ) -> Result<Self, SourceError> {
        Self::connect_bounded(
            program,
            args,
            name,
            config,
            secrets,
            Deadlines {
                handshake: deadline,
                requests: deadline,
            },
            document_dir,
        )
    }

    fn connect_bounded(
        program: &str,
        args: &[String],
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
        deadlines: Deadlines,
        document_dir: Option<&Path>,
    ) -> Result<Self, SourceError> {
        let document_dir = document_dir
            .map(|directory| {
                DocumentDir::new(directory).map_err(|problem| SourceError::Config {
                    message: format!(
                        "source {name}: its settings are measured from the directory holding \
                         the configuration document that set them, and {problem}; give the \
                         settings absolute paths, or move the document under a directory \
                         whose name is valid UTF-8"
                    ),
                })
            })
            .transpose()?;
        Self::adopt(
            Peer::spawn(
                program,
                args,
                deadlines.handshake.duration(),
                deadlines.requests.duration(),
            )?,
            name,
            config,
            secrets,
            document_dir,
        )
    }

    /// Connect to a plugin that is already running, over streams somebody else owns.
    ///
    /// The handshake, the framing and every refusal are the same as [`connect`]'s, because
    /// they are the protocol's rather than the process's. What this constructor adds is
    /// the ability to hold the *other* end: it is how the engine's own tests drive this
    /// half against [`serve`](super::serve) over a real pipe, including the answers a
    /// well-behaved program would never give.
    ///
    /// # Errors
    ///
    /// Returns what [`connect`](Self::connect) returns, minus the failures that belong to
    /// spawning a program.
    ///
    /// [`connect`]: Self::connect
    pub fn over(
        to_plugin: impl std::io::Write + Send + 'static,
        from_plugin: impl std::io::Read + Send + 'static,
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
    ) -> Result<Self, SourceError> {
        Self::over_with_request_deadline(
            to_plugin,
            from_plugin,
            name,
            config,
            secrets,
            RequestDeadline::DEFAULT,
        )
    }

    /// Connect over existing streams with a deadline for requests after initialization.
    ///
    /// Unlike [`connect_with_deadline`](Self::connect_with_deadline), this engine does
    /// not own a process it can interrupt while the synchronous handshake is blocked.
    /// The supplied deadline therefore begins only after initialization succeeds.
    pub fn over_with_request_deadline(
        to_plugin: impl std::io::Write + Send + 'static,
        from_plugin: impl std::io::Read + Send + 'static,
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
        deadline: RequestDeadline,
    ) -> Result<Self, SourceError> {
        Self::adopt(
            Peer::over(to_plugin, from_plugin, deadline.duration()),
            name,
            config,
            secrets,
            None,
        )
    }

    /// Shake hands with `peer` and take the connection over.
    fn adopt(
        mut peer: Peer,
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
        document_dir: Option<DocumentDir>,
    ) -> Result<Self, SourceError> {
        let result = Self::handshake(&mut peer, name, config, secrets, document_dir);
        let InitializeResult {
            protocol_version,
            kind,
            capabilities,
            writes,
            meters,
            statuses,
            task_updates,
            metadata_updates,
            content_updates,
        } = match result {
            Ok(result) => result,
            Err(error) => return Err(with_diagnostics(error, &mut peer)),
        };
        let kind = kind.into_string();
        if protocol_version != Some(PROTOCOL_VERSION) {
            return Err(SourceError::Config {
                message: match protocol_version {
                    Some(spoken) => format!(
                        "the {kind:?} plugin was asked for protocol version \
                         {PROTOCOL_VERSION} and answered in version {spoken}; the two are \
                         incompatible and this engine does not guess between them"
                    ),
                    None => format!(
                        "the {kind:?} plugin did not say which protocol version it \
                         answered in; this engine speaks version {PROTOCOL_VERSION} and \
                         does not guess"
                    ),
                },
            });
        }
        Ok(Self {
            kind: String::leak(kind),
            capabilities,
            writes: writes.unwrap_or(WriteSupport::Unsupported),
            meters,
            knows_every_category: knows_every_category(statuses.as_deref()),
            task_updates,
            metadata_updates,
            content_updates,
            connection: Connection::adopt(peer),
        })
    }

    /// Send `initialize` and read what came back (§3).
    fn handshake(
        peer: &mut Peer,
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
        document_dir: Option<DocumentDir>,
    ) -> Result<InitializeResult, SourceError> {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION,
            engine: EngineIdentity {
                name: "onetaskgraph".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            source_name: name.as_str().to_owned(),
            config: config.clone(),
            secrets,
            statuses: Some(vocabulary()),
            document_dir,
        };
        let request = Request {
            id: HANDSHAKE_ID.to_owned(),
            method: "initialize".to_owned(),
            // Plain data throughout: a `BTreeMap<String, String>` and a `Value` the
            // configuration layer already parsed.
            params: serde_json::to_value(&params).expect("a handshake is plain data"),
        };
        let line = peer.exchange(
            &serde_json::to_string(&request).expect("a handshake request is plain data"),
        )?;
        let response: super::wire::Response =
            serde_json::from_str(&line).map_err(|error| SourceError::Malformed {
                message: format!(
                    "the plugin's handshake answer is not a response envelope: {error}"
                ),
            })?;
        // §6.3: an envelope addressed to an id this side never sent is a violation, and it
        // is one here for the same reason it is later — a plugin whose first line answers
        // something else has not answered the handshake, and reading it as one would build
        // a source out of a message that was about something different.
        if response.id != HANDSHAKE_ID {
            return Err(SourceError::Malformed {
                message: format!(
                    "the plugin answered the handshake with an envelope addressed to {:?} \
                     rather than to {HANDSHAKE_ID:?}",
                    response.id
                ),
            });
        }
        let outcome = response.outcome().ok_or_else(|| SourceError::Malformed {
            message: "the plugin's handshake answer carried both a result and an error, or \
                      neither"
                .to_owned(),
        })?;
        let result = outcome?;
        serde_json::from_value(result).map_err(|error| SourceError::Malformed {
            message: format!("the plugin's handshake answer is not an initialize result: {error}"),
        })
    }

    /// The statuses a query may hand this plugin, or `None` when every one asked for is a
    /// category it was not written against — which no row it holds can be in.
    ///
    /// Dropping those categories narrows nothing: a plugin that does not know a category
    /// cannot report a row in it, so the rows the rest of the list matches are every row the
    /// whole list matches.
    fn statuses_for(&self, statuses: &[StatusCategory]) -> Option<Vec<StatusCategory>> {
        if self.knows_every_category {
            return Some(statuses.to_vec());
        }
        let known: Vec<StatusCategory> = statuses
            .iter()
            .copied()
            .filter(|category| !after_the_first_vocabulary(*category))
            .collect();
        (known.len() == statuses.len() || !known.is_empty()).then_some(known)
    }

    /// Refuse a status this plugin's handshake says it was not written against, before it is
    /// sent (§3.5).
    fn knows(&self, category: StatusCategory) -> Result<(), SourceError> {
        if self.knows_every_category || !after_the_first_vocabulary(category) {
            return Ok(());
        }
        Err(SourceError::Refused {
            message: format!(
                "the {:?} plugin's handshake does not list the status category {}, so this \
                 engine does not hand it one (docs/plugin-protocol.md §3.5); next: upgrade the \
                 plugin to one whose handshake lists it, or use a category it knows",
                self.kind,
                spelled(category)
            ),
        })
    }

    /// Refuse what only a plugin declaring `task_updates` is handed, before it is sent (§3.6).
    fn updates(&self, what: &str) -> Result<(), SourceError> {
        if self.task_updates {
            return Ok(());
        }
        Err(SourceError::Refused {
            message: format!(
                "the {:?} plugin's handshake does not say it answers the narrow task writes, so \
                 this engine does not send it {what} (docs/plugin-protocol.md §3.6); next: \
                 upgrade the plugin to one whose handshake sets task_updates",
                self.kind
            ),
        })
    }

    /// Refuse a narrow metadata write of `record` to a plugin whose handshake did not declare
    /// `metadata_updates`, before it is sent (§3.7), in the contract's own words.
    fn metadata_updates(&self, record: MetadataRecord) -> Result<(), SourceError> {
        if self.metadata_updates {
            return Ok(());
        }
        Err(unwritable_metadata(self.kind, record))
    }

    /// Refuse a task write this plugin could only drop part of in silence.
    fn writable_task(&self, task: &Task) -> Result<(), SourceError> {
        self.knows(task.status.category)?;
        // A plugin whose handshake declares no priority is one written before there were
        // any, and it would drop one in silence (§6). The engine refuses such a write before
        // it reaches here; this keeps the seam itself from ever sending one.
        if task.priority != Priority::None && !self.capabilities.priority.is_native() {
            return Err(unwritable_field(self.kind, "priority"));
        }
        if !task.delivers.is_empty() || !task.delivered_by.is_empty() {
            self.updates("a task carrying delivers or delivered_by")?;
        }
        Ok(())
    }

    /// One call, with its result parsed into the shape the method promises.
    async fn ask<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, SourceError> {
        let result = self.connection.call(method, params).await?;
        serde_json::from_value(result).map_err(|error| SourceError::Malformed {
            message: format!(
                "the plugin's answer to {method} is not the shape it promises: {error}"
            ),
        })
    }
}

/// Append whatever the plugin said on standard error to a handshake failure.
///
/// A plugin that refuses the handshake and exits has usually said why there and nowhere
/// else, and a bare "could not read the plugin's answer" would throw that away.
fn with_diagnostics(error: SourceError, peer: &mut Peer) -> SourceError {
    let said = peer.said();
    if said.is_empty() {
        return error;
    }
    let message = format!("{error}; the plugin wrote: {said}");
    match error {
        // The wait it asked for is preserved: what the plugin wrote is extra reason, not a
        // replacement for the one piece of this refusal the engine acts on.
        SourceError::RateLimited {
            retry_after_seconds,
            ..
        } => SourceError::RateLimited {
            retry_after_seconds,
            message: Some(message),
        },
        SourceError::Config { .. } => SourceError::Config { message },
        SourceError::Auth { .. } => SourceError::Auth { message },
        SourceError::Refused { .. } => SourceError::Refused { message },
        SourceError::Malformed { .. } => SourceError::Malformed { message },
        SourceError::Unavailable { .. } => SourceError::Unavailable { message },
    }
}

#[async_trait]
impl TaskSource for SubprocessSource {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }

    async fn health(&self) -> Result<Health, SourceError> {
        self.ask("health", json!({})).await
    }

    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        let result: TaskResult = self
            .ask("get_task", params(&IdParams { id: id.clone() }))
            .await?;
        Ok(result.task)
    }

    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        let result: ProjectResult = self
            .ask("get_project", params(&IdParams { id: id.clone() }))
            .await?;
        Ok(result.project)
    }

    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        let Some(statuses) = self.statuses_for(&query.statuses) else {
            return Ok(Page::last(Vec::new()));
        };
        self.ask(
            "query_tasks",
            params(&TaskQueryParams {
                query: TaskQuery {
                    statuses,
                    ..query.clone()
                },
                page: page.clone(),
            }),
        )
        .await
    }

    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        let Some(statuses) = self.statuses_for(&query.statuses) else {
            return Ok(Page::last(Vec::new()));
        };
        self.ask(
            "query_projects",
            params(&ProjectQueryParams {
                query: ProjectQuery {
                    statuses,
                    ..query.clone()
                },
                page: page.clone(),
            }),
        )
        .await
    }

    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        self.ask("labels", params(&LabelParams { page: page.clone() }))
            .await
    }

    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.ask(
            "task_dependencies",
            params(&DependencyParams {
                id: id.clone(),
                direction,
                page: page.clone(),
            }),
        )
        .await
    }

    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.ask(
            "project_dependencies",
            params(&DependencyParams {
                id: id.clone(),
                direction,
                page: page.clone(),
            }),
        )
        .await
    }

    fn writes(&self) -> WriteSupport {
        self.writes
    }

    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.writable_task(&write.item)?;
        let result: WriteResult = self
            .ask(
                "write_task",
                params(&TaskWriteParams {
                    write: write.clone(),
                }),
            )
            .await?;
        Ok(result.id)
    }

    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        self.knows(write.item.status.category)?;
        let result: WriteResult = self
            .ask(
                "write_project",
                params(&ProjectWriteParams {
                    write: write.clone(),
                }),
            )
            .await?;
        Ok(result.id)
    }

    async fn set_task_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        self.updates("set_task_status")?;
        self.knows(category)?;
        let result: StatusResult = self
            .ask(
                "set_task_status",
                params(&StatusParams {
                    id: id.clone(),
                    category,
                }),
            )
            .await?;
        Ok(result.status)
    }

    async fn set_task_priority(
        &self,
        id: &NativeId,
        priority: Priority,
    ) -> Result<Option<Priority>, SourceError> {
        // §4.19: sent only to a plugin whose handshake declares it holds a priority.
        if !self.capabilities.priority.is_native() {
            return Err(unwritable_field(self.kind, "priority"));
        }
        let result: PriorityResult = self
            .ask(
                "set_task_priority",
                params(&PriorityParams {
                    id: id.clone(),
                    priority,
                }),
            )
            .await?;
        Ok(result.priority)
    }

    async fn set_task_content(
        &self,
        id: &NativeId,
        content: &str,
    ) -> Result<Option<()>, SourceError> {
        // §3.9: sent only to a plugin whose handshake sets content_updates.
        if !self.content_updates {
            return Err(unwritable_field(self.kind, "content"));
        }
        let result: ContentResult = self
            .ask(
                "set_task_content",
                params(&ContentParams {
                    id: id.clone(),
                    content: content.to_owned(),
                }),
            )
            .await?;
        Ok(result.id.map(|_| ()))
    }

    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        self.updates("set_delivered_by")?;
        let result: DeliveredByResult = self
            .ask(
                "set_delivered_by",
                params(&DeliveredByParams {
                    id: id.clone(),
                    delivered_by: delivered_by.to_vec(),
                }),
            )
            .await?;
        Ok(result.delivered_by.map(|_| ()))
    }

    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Task>, SourceError> {
        self.metadata_updates(MetadataRecord::Task)?;
        let result: TaskResult = self
            .ask("set_task_metadata", metadata_params(id, key, value))
            .await?;
        Ok(result.task)
    }

    async fn set_project_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Project>, SourceError> {
        self.metadata_updates(MetadataRecord::Project)?;
        let result: ProjectResult = self
            .ask("set_project_metadata", metadata_params(id, key, value))
            .await?;
        Ok(result.project)
    }

    async fn set_document_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Document>, SourceError> {
        self.metadata_updates(MetadataRecord::Document)?;
        let result: DocumentResult = self
            .ask("set_document_metadata", metadata_params(id, key, value))
            .await?;
        Ok(result.document)
    }

    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        let _: IgnoredResult = self
            .ask("delete_task", params(&DeleteParams { id: id.clone() }))
            .await?;
        Ok(())
    }

    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        let _: IgnoredResult = self
            .ask("delete_project", params(&DeleteParams { id: id.clone() }))
            .await?;
        Ok(())
    }

    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        let result: DocumentResult = self
            .ask("get_document", params(&IdParams { id: id.clone() }))
            .await?;
        Ok(result.document)
    }

    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        self.ask(
            "query_documents",
            params(&DocumentQueryParams {
                query: query.clone(),
                page: page.clone(),
            }),
        )
        .await
    }

    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        let result: WriteResult = self
            .ask(
                "write_document",
                params(&DocumentWriteParams {
                    write: write.clone(),
                }),
            )
            .await?;
        Ok(result.id)
    }

    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        let _: IgnoredResult = self
            .ask("delete_document", params(&DeleteParams { id: id.clone() }))
            .await?;
        Ok(())
    }

    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        let result: CommentsResult = self
            .ask(
                "task_comments",
                params(&CommentsParams {
                    task: task.clone(),
                    page: page.clone(),
                }),
            )
            .await?;
        Ok(result.page)
    }

    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        let result: CommentResult = self
            .ask(
                "add_comment",
                params(&AddCommentParams {
                    task: task.clone(),
                    comment: comment.clone(),
                }),
            )
            .await?;
        Ok(result.comment)
    }

    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        let result: CommentResult = self
            .ask(
                "edit_comment",
                params(&EditCommentParams {
                    task: task.clone(),
                    comment: comment.clone(),
                    body: body.clone(),
                }),
            )
            .await?;
        Ok(result.comment)
    }

    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        let result: DeletedCommentResult = self
            .ask(
                "delete_comment",
                params(&DeleteCommentParams {
                    task: task.clone(),
                    comment: comment.clone(),
                }),
            )
            .await?;
        Ok(result.deleted)
    }

    async fn metering(&self) -> Result<Option<Metering>, SourceError> {
        // Never sent to a plugin that did not declare it (§3.4), which is what lets a
        // plugin written before there was metering go on working without an edit.
        if !self.meters {
            return Ok(None);
        }
        let result: MeteringResult = self.ask("metering", json!({})).await?;
        Ok(result.metering)
    }
}

/// The object §4.10 answers with, decoded so that `ask` has a type to hand back.
///
/// A named type rather than `serde_json::Value` so a plugin answering with something other
/// than an object is still refused where every other method's answer is. It does **not**
/// require that object to be empty, and no `deny_unknown_fields` belongs here: §2.1 is that
/// a reader ignores members it does not know, at every level, which is what lets a later
/// version add an optional one without a version bump. Refusing an unknown member here
/// would refuse that plugin outright, and would be the only type of this boundary that did.
#[derive(serde::Deserialize)]
struct IgnoredResult {}

/// One method's parameters as the object the envelope carries.
///
/// Every parameter type in `wire` is built from contract types that all serialize, so
/// this cannot fail for a reason a caller could act on.
fn params<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("method parameters are plain data")
}

/// The parameters of any of the three narrow metadata writes (§4.18).
fn metadata_params(id: &NativeId, key: &MetadataKey, value: &Value) -> Value {
    params(&MetadataParams {
        id: id.clone(),
        key: key.clone(),
        value: value.clone(),
    })
}
