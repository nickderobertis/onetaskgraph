//! The two traits a plugin implements, and the secret lookup it is handed.

use schemars::{JsonSchema, Schema};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{
    Capabilities, DependencyEdge, Direction, Label, NativeId, Page, PageRequest, Project,
    ProjectQuery, SourceError, SourceName, Task, TaskQuery,
};

/// Whether a source is answering right now.
///
/// # Placement is an open contract question
///
/// This type lives here because [`TaskSource::health`] returns it and the trait
/// lives here: placing it in `onetaskgraph-core` would make this crate depend on
/// the engine and invert the one direction the crate split exists to establish.
/// The approved contract enumerates this crate's contents exhaustively and does
/// not name `Health`, so the enumeration and the trait as written cannot both
/// stand. Compiling forces the placement below; the resolution — add it to the
/// enumeration, or redesign `health` so no such type crosses the boundary —
/// belongs to the contract's owner, not to this crate. See `AGENTS.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
// llmlint: ignore[invalid_states_unrepresentable] Health's shape is frozen by the plugin contract every source is written against; only the contract's owner may change it, and an enum over reachable/unreachable is post-build follow-up.
pub struct Health {
    /// Whether the source answered.
    pub reachable: bool,
    /// What the source said, when it said anything useful.
    pub detail: Option<String>,
}

/// One configured source, as the engine drives it.
///
/// Dyn-compatible through `async_trait` because the engine holds
/// `Vec<Box<dyn TaskSource>>` over heterogeneous plugins.
///
/// Three rules bind every implementation, and the engine's compensation is only
/// correct while all three hold:
///
/// 1. **Apply** every predicate you declare [`Support::Native`](crate::Support::Native).
/// 2. **Ignore** every [`Support`](crate::Support)-typed predicate you declare
///    `Unsupported` — return the *wider* result set, never a narrower one.
///    Silently dropping rows for a predicate you did not declare is the one
///    failure no test above the plugin can catch.
/// 3. Never return a silently empty dependency read. Rule 2 reaches the
///    `Support`-typed predicates alone; a dependency read is always real.
#[async_trait::async_trait]
pub trait TaskSource: Send + Sync {
    /// The plugin kind that built this source, for display and for plan output.
    fn kind(&self) -> &'static str;

    /// What this source applies itself. Read once per query by the engine.
    fn capabilities(&self) -> Capabilities;

    /// Whether the source is answering right now.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the check itself could not be made.
    async fn health(&self) -> Result<Health, SourceError>;

    /// Fetch one task by its native id, or `None` when there is no such task.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError>;

    /// Fetch one project by its native id, or `None` when there is no such project.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError>;

    /// One page of the tasks matching `query`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError>;

    /// One page of the projects matching `query`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError>;

    /// One page of every label this source knows.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError>;

    /// One page of the task dependency edges at `id`, in `direction`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError>;

    /// One page of the project dependency edges at `id`, in `direction`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError>;
}

/// The factory that turns one configuration block into a live [`TaskSource`].
///
/// Having the compile-time registry and the subprocess seam be the same shape is
/// the whole reason this is a trait rather than a free function.
pub trait SourcePlugin: Send + Sync + 'static {
    /// The name a configuration document's `plugin:` field names.
    fn kind(&self) -> &'static str;

    /// The JSON Schema for this plugin's own `config:` block.
    fn config_schema(&self) -> Schema;

    /// Build a live source from one configuration block.
    ///
    /// `name` is the configured source's name, for error messages only — a
    /// plugin never learns it for any other purpose.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Config`] when `config` is not valid for this
    /// plugin, or [`SourceError::Auth`] when a named credential is absent.
    fn build(
        &self,
        name: &SourceName,
        config: &serde_json::Value,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError>;
}

/// How a plugin reads the credential its configuration names.
///
/// A configuration document never carries a credential value, only the name of
/// the environment variable holding it.
pub trait SecretResolver: Send + Sync {
    /// The value of `var`, or `None` when nothing defines it.
    ///
    /// The returned value is never logged and never appears in `Debug` output.
    fn get(&self, var: &str) -> Option<SecretString>;
}
