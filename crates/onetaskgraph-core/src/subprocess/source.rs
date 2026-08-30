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
use std::time::Duration;

use async_trait::async_trait;
use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, Direction, Health, ItemWrite, Label, NativeId, Page, PageRequest,
    Project, ProjectQuery, SourceError, SourceName, Task, TaskQuery, TaskSource, WriteSupport,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::connection::{Connection, Peer};
use super::wire::{
    DeleteParams, DependencyParams, EngineIdentity, IdParams, InitializeParams, InitializeResult,
    LabelParams, PROTOCOL_VERSION, ProjectQueryParams, ProjectResult, ProjectWriteParams, Request,
    TaskQueryParams, TaskResult, TaskWriteParams, WriteResult,
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
    pub fn connect_with_deadline(
        program: &str,
        args: &[String],
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
        deadline: RequestDeadline,
    ) -> Result<Self, SourceError> {
        Self::adopt(
            Peer::spawn(program, args, deadline.duration())?,
            name,
            config,
            secrets,
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
        )
    }

    /// Shake hands with `peer` and take the connection over.
    fn adopt(
        mut peer: Peer,
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
    ) -> Result<Self, SourceError> {
        let result = Self::handshake(&mut peer, name, config, secrets);
        let InitializeResult {
            protocol_version,
            kind,
            capabilities,
            writes,
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
            connection: Connection::adopt(peer),
        })
    }

    /// Send `initialize` and read what came back (§3).
    fn handshake(
        peer: &mut Peer,
        name: &SourceName,
        config: &Value,
        secrets: BTreeMap<String, String>,
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
        SourceError::RateLimited { .. } => error,
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
        self.ask(
            "query_tasks",
            params(&TaskQueryParams {
                query: query.clone(),
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
        self.ask(
            "query_projects",
            params(&ProjectQueryParams {
                query: query.clone(),
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
}

/// The empty object §4.10 answers with, decoded so that `ask` has a type to hand back.
///
/// A named type rather than `serde_json::Value` so a plugin answering with something other
/// than an object is still refused where every other method's answer is.
#[derive(serde::Deserialize)]
struct IgnoredResult {}

/// One method's parameters as the object the envelope carries.
///
/// Every parameter type in `wire` is built from contract types that all serialize, so
/// this cannot fail for a reason a caller could act on.
fn params<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("method parameters are plain data")
}
