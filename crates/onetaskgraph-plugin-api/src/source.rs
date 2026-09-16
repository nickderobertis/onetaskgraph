//! The two traits a plugin implements, and the secret lookup it is handed.

use schemars::{JsonSchema, Schema};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{
    Capabilities, Comment, CommentBody, DependencyEdge, Direction, Document, DocumentQuery,
    ItemWrite, Label, Metering, NativeId, NewComment, Page, PageRequest, Project, ProjectQuery,
    SourceError, SourceName, Status, StatusCategory, Task, TaskQuery, TaskRef, WriteSupport,
    commentless, documentless, unwritable, unwritable_field,
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
// llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this restates at a new site the justification already recorded at `Capabilities.max_page_size` (capability.rs) and `PageRequest.limit` (query.rs), and a third time in this type's own doc comment above and in AGENTS.md's "Open contract question — `Health`": `Health`'s shape is approved contract text that `TaskSource::health` returns, so an enum here would change the serialized form and the trait six undispatched nodes implement. That is the contract owner's call, not this crate's.
pub struct Health {
    /// Whether the source answered.
    ///
    /// A bare `bool` beside an untyped `detail` cannot say that an unreachable source
    /// must explain itself, or keep "reachable with a warning" apart from "reachable";
    /// an enum carrying the detail in its unreachable variant would.
    // llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this
    // restates at this field the justification already recorded at
    // `Capabilities.max_page_size` (capability.rs), `PageRequest.limit` (query.rs), this
    // type's own doc comment above, and AGENTS.md's "Open contract question — `Health`":
    // `Health`'s shape is approved contract text that `TaskSource::health` returns, so an
    // enum here would change the serialized form and the trait six undispatched nodes
    // implement. That is the contract owner's call, not this crate's.
    // llmlint: ignore[boundary_inputs_validated] making "unreachable with no reason given" unrepresentable means an enum here, which changes the serialized form and the trait six undispatched nodes implement. Deferred to the contract's owner — AGENTS.md, "Open contract question — `Health`".
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
///    `Support`-typed *predicates* alone; a dependency read is always real, and so is a
///    document read — [`Capabilities::documents`] says whether this source has documents
///    at all, and a source that says it has none is never asked for one rather than
///    answering an empty page.
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

    /// Whether this source can be written through at all.
    ///
    /// Defaulted to [`WriteSupport::Unsupported`], which is what keeps this a read
    /// interface for every source that has nothing to write into: one that cannot be
    /// written needs no edit and keeps working. Read before a write is attempted, so a
    /// copy naming such a source as its destination is refused before anything is read.
    fn writes(&self) -> WriteSupport {
        WriteSupport::Unsupported
    }

    /// Create or update one task, answering with the native id the destination holds it
    /// under.
    ///
    /// A source declaring [`WriteSupport::Supported`] owes three things here. It refuses,
    /// naming the field, anything it cannot represent rather than dropping it — including
    /// a metadata key it cannot carry, which it names. It writes every other field it was
    /// given. And it never creates when [`ItemWrite::target`] names an item it does not
    /// hold.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no write side, when a field
    /// or a metadata key cannot be represented, or when `target` names nothing here; and
    /// whatever else the source could not do the write for.
    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        let _ = write;
        Err(unwritable(self.kind()))
    }

    /// Create or update one project, on exactly the terms of
    /// [`write_task`](Self::write_task).
    ///
    /// # Errors
    ///
    /// As [`write_task`](Self::write_task).
    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        let _ = write;
        Err(unwritable(self.kind()))
    }

    /// Set the status of one task this source holds, and change nothing else about it,
    /// answering with the status as this source now reads it — or `None` when this source
    /// holds no such task.
    ///
    /// The category lands where this source's own mapping sends it, exactly as a
    /// [`write_task`](Self::write_task) of a task in that category would: a category this
    /// source has disabled is refused in the words a write of it is refused with. Title,
    /// content, labels, metadata, dependencies, [`Task::delivers`], [`Task::delivered_by`],
    /// project and comments are left exactly as they are.
    ///
    /// Defaulted to [`unwritable_field`], which is what keeps this an addition rather than a
    /// break: a source that cannot write a status on its own needs no edit and refuses by
    /// saying so. A source declaring [`WriteSupport::Unsupported`] is never asked.
    ///
    /// [`Task::delivers`]: crate::Task::delivers
    /// [`Task::delivered_by`]: crate::Task::delivered_by
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot write a status, or cannot
    /// write this one; and whatever else the source could not do the write for.
    async fn set_task_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        let _ = (id, category);
        Err(unwritable_field(self.kind(), "status"))
    }

    /// Replace the [`Task::delivered_by`] of one task this source holds, and change nothing
    /// else about it — or answer `None` when this source holds no such task.
    ///
    /// Every entry is a qualified id, and the list is the whole of it: what the task held
    /// there before is replaced, not merged. It is the store's to keep in step — the engine
    /// calls this whenever it writes a task's [`Task::delivers`] — and nothing a person types
    /// reaches it directly.
    ///
    /// Defaulted to [`unwritable_field`] on exactly the terms of
    /// [`set_task_status`](Self::set_task_status).
    ///
    /// [`Task::delivers`]: crate::Task::delivers
    /// [`Task::delivered_by`]: crate::Task::delivered_by
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot hold the list, and whatever
    /// else it could not do the write for.
    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        let _ = (id, delivered_by);
        Err(unwritable_field(self.kind(), "delivered_by"))
    }

    /// Remove one task this destination holds, so a copy that could not finish can put
    /// the destination back the way it found it.
    ///
    /// This is not a verb of the product: nothing a user types deletes anything, and a
    /// copy never deletes an item it did not itself create in the run that is failing.
    /// It exists because a copy is either complete or it never happened — a half-written
    /// project has to be run again, and the re-run is the mutation burst that trips a
    /// hosted destination's rate limiter. Undoing this run's own creates is what removes
    /// that retry at source.
    ///
    /// A source declaring [`WriteSupport::Supported`] owes a real implementation, for the
    /// reason it owes [`write_task`](Self::write_task) one: the engine will create items
    /// there, so it has to be able to remove the ones it created. An `id` naming nothing
    /// is **not** an error — the item is already gone, which is the state this asks for.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no write side, and whatever
    /// else the source could not remove the item for.
    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        let _ = id;
        Err(unwritable(self.kind()))
    }

    /// Remove one project this destination holds, on exactly the terms of
    /// [`delete_task`](Self::delete_task).
    ///
    /// # Errors
    ///
    /// As [`delete_task`](Self::delete_task).
    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        let _ = id;
        Err(unwritable(self.kind()))
    }

    /// Fetch one document by its native id, or `None` when there is no such document.
    ///
    /// Defaulted to [`documentless`], which is what keeps documents an addition rather
    /// than a break: a source with none needs no edit, keeps working, and says so in the
    /// same words every other document-free source does. A source that has documents
    /// declares [`Support::Native`](crate::Support::Native) for
    /// [`Capabilities::documents`] and owes a real implementation here, because that
    /// declaration is what makes the engine ask.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no documents, and whatever
    /// else the source could not answer for.
    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        let _ = id;
        Err(documentless(self.kind()))
    }

    /// One page of the documents matching `query`.
    ///
    /// Defaulted on exactly the terms of [`get_document`](Self::get_document). A source
    /// with no documents refuses rather than answering an empty page: an empty page reads
    /// as a source that has documents and holds none matching, which is the one wrong
    /// answer this method can give.
    ///
    /// # Errors
    ///
    /// As [`get_document`](Self::get_document).
    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        let _ = (query, page);
        Err(documentless(self.kind()))
    }

    /// Create or update one document, on exactly the terms of
    /// [`write_task`](Self::write_task).
    ///
    /// # Errors
    ///
    /// As [`write_task`](Self::write_task).
    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        let _ = write;
        Err(unwritable(self.kind()))
    }

    /// Remove one document this destination holds, on exactly the terms of
    /// [`delete_task`](Self::delete_task).
    ///
    /// # Errors
    ///
    /// As [`delete_task`](Self::delete_task).
    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        let _ = id;
        Err(unwritable(self.kind()))
    }

    /// One page of the comments on `task`, oldest first, or `None` when this source holds
    /// no such task.
    ///
    /// Defaulted to [`commentless`], which is what keeps comments an addition rather than a
    /// break: a source with none needs no edit and keeps working. A source whose tasks have
    /// comments declares [`Support::Native`](crate::Support::Native) for
    /// [`Capabilities::comments`] and owes a real implementation of all four comment methods,
    /// because that declaration is what makes the engine ask.
    ///
    /// "No such task" is `None` rather than an error, exactly as it is for
    /// [`get_task`](Self::get_task); a task that exists and has no comments is an empty page.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no comments, and whatever else
    /// the source could not answer for.
    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        let _ = (task, page);
        Err(commentless(self.kind()))
    }

    /// Add one comment to `task`, answering with the comment as the source now holds it, or
    /// `None` when this source holds no such task.
    ///
    /// The body is stored byte for byte. A source that records the author itself refuses a
    /// [`NewComment::author`] rather than dropping it, naming why; a source that cannot
    /// represent the body refuses it, naming why, rather than escaping it into something
    /// else.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no comments or cannot be
    /// written, when it cannot record what it was given, and whatever else it could not do
    /// the write for.
    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        let _ = (task, comment);
        Err(commentless(self.kind()))
    }

    /// Replace the body of the comment `comment` on `task`, answering with the comment as the
    /// source now holds it, or `None` when this source holds no such task or that task has no
    /// such comment.
    ///
    /// Only the body and the time it last changed move: the id, the author and the time it
    /// was written are the comment's own.
    ///
    /// # Errors
    ///
    /// As [`add_comment`](Self::add_comment).
    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        let _ = (task, comment, body);
        Err(commentless(self.kind()))
    }

    /// Remove the comment `comment` from `task`, answering with the id it removed, or `None`
    /// when this source holds no such task or that task has no such comment.
    ///
    /// Unlike [`delete_task`](Self::delete_task), this *is* a verb of the product — a person
    /// removes a comment they posted — so a comment that is not there is reported as `None`
    /// for the engine to refuse by name, rather than treated as already gone.
    ///
    /// # Errors
    ///
    /// As [`add_comment`](Self::add_comment).
    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        let _ = (task, comment);
        Err(commentless(self.kind()))
    }

    /// What this source has sent to its backend since it was built and what that spent, or
    /// `None` when it does not meter its own requests.
    ///
    /// Defaulted to `None`, which is what keeps metering an addition rather than a break: a
    /// source that does not count its requests needs no edit, and is reported as not
    /// metering rather than as having spent nothing. A source that answers owes a running
    /// total — see [`Metering`] — because what one command spent is read as the difference
    /// between two readings.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the reading itself could not be taken. A caller
    /// reports such a source as not metering; what a command cost is never a reason for the
    /// command to fail.
    async fn metering(&self) -> Result<Option<Metering>, SourceError> {
        Ok(None)
    }
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

    /// The fields of this plugin's `config:` block that name a filesystem path, as dotted
    /// paths into that block.
    ///
    /// A relative value at one of these, **supplied by a configuration document**, is
    /// resolved against the directory holding that document before [`Self::build`] sees it;
    /// supplied through the environment or a flag it keeps resolving against the process
    /// working directory, because there is no document to rebase it on. A plugin is handed
    /// values and no origins, so this declaration is the only way it can say which of its
    /// own fields that rule reaches.
    ///
    /// Defaulted to none, which is what keeps this an addition rather than a break: a
    /// plugin whose block holds no path needs no edit, and a caller asks every plugin
    /// rather than keeping a table of which ones answer.
    // llmlint: ignore[invalid_states_unrepresentable] The identity of a configuration field
    // is a name, and no type can make a wrong one unrepresentable here: every string is a
    // syntactically valid dotted path, so a newtype would validate nothing and would only
    // move where a name that is not a field of *this* plugin is accepted. What decides that
    // is whether the name is a property of the schema `config_schema` publishes — a
    // per-plugin fact no shared type can hold — so the gate is per plugin and executable:
    // `document_relative_fields_are_fields_this_plugin_declares` in
    // `onetaskgraph-local-md/tests/plugin.rs`, which a plugin adding a declaration owes its
    // own copy of.
    fn document_relative_paths(&self) -> &'static [&'static str] {
        &[]
    }
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
