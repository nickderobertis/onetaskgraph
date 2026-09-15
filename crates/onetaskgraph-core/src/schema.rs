//! The JSON Schema bundle both SDKs are generated from.
//!
//! It is emitted by `onetaskgraph schema` rather than committed, so the schema a
//! consumer generates against can never drift from the types this binary actually
//! serialises: they are the same types.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    Capabilities, Comment, DependencyEdge, DependencyEndpoint, DependencyKind, Direction, Document,
    DocumentQuery, Health, ItemKind, Label, Location, NewComment, Page, PageRequest, Project,
    ProjectQuery, Repository, SourceError, Status, StatusCategory, Task, TaskQuery, TaskRef,
    TextFields,
};
use schemars::{Schema, schema_for};
use serde_json::{Value, json};

use crate::config::{EffectiveConfig, Origin, OutputFormat, Setting};
use crate::registry::registry;
use crate::secrets::{CredentialLayer, ResolvedCredential, SecretsReport};
use crate::{
    CommentList, CopyAction, CopyOutcome, CopyReport, DeletedComment, Delivered, DeliveryOutcome,
    Failure, FailureClass, FailureDocument, GlobalId, PageToken, Predicate, Qualified,
    QualifiedEdge, QualifiedEndpoint, QueryPlan, QueryResponse, SearchHit, SearchKind,
    SourceFailure, SourceListing, SourcePlan, TaskDetail, TaskStatusSet,
};

/// The bundle's own version, bumped whenever any root's schema changes — added, removed,
/// renamed, **or altered inside**.
///
/// Consumers generate code from this document, so the version is part of the
/// contract rather than a convenience: an SDK can refuse a bundle it was not
/// generated against instead of silently emitting the wrong models. A property added to an
/// existing root is a new field in both SDKs' generated models exactly as a new root is a
/// new model, which is why the reach is the whole document rather than the set of names.
///
/// What each version brought is what `git log` answers; what this number owes a reader is
/// that it moves whenever [`schema_bundle`] below emits a different document. The golden
/// that holds it to that is `PUBLISHED_BUNDLES` in `tests/engine.rs`, which records every
/// root's schema by digest from this version on.
pub const SCHEMA_BUNDLE_VERSION: u32 = 14;

/// Every contract root, keyed by name, plus each registered plugin's config schema.
#[must_use]
pub fn schema_bundle() -> Value {
    let mut roots: BTreeMap<&'static str, Schema> = BTreeMap::new();

    roots.insert("Task", schema_for!(Task));
    roots.insert("Project", schema_for!(Project));
    roots.insert("Document", schema_for!(Document));
    // A root of its own although both `Task` and `Project` reach it inside their own
    // definitions, for the reason `TextFields` is one: a consumer acts on a location by
    // asking which of the two keys is present, so the shape it switches on has to be
    // nameable rather than only reachable.
    roots.insert("Location", schema_for!(Location));
    roots.insert("Label", schema_for!(Label));
    roots.insert("Status", schema_for!(Status));
    roots.insert("StatusCategory", schema_for!(StatusCategory));
    roots.insert("DependencyEdge", schema_for!(DependencyEdge));
    roots.insert("DependencyEndpoint", schema_for!(DependencyEndpoint));
    roots.insert("QualifiedEndpoint", schema_for!(QualifiedEndpoint));
    roots.insert("ItemKind", schema_for!(ItemKind));
    roots.insert("Repository", schema_for!(Repository));
    roots.insert("DependencyKind", schema_for!(DependencyKind));
    roots.insert("Direction", schema_for!(Direction));
    // Roots of their own although both are reachable inside `TaskQuery`'s definitions,
    // which is enough for a generator and not enough for a reconciliation: the command
    // line spells both deliberately differently (`both` for `title-or-content`, `task`
    // for `tasks`), so a variant added to either would leave the command line quietly
    // unable to name it. A root apiece gives that gate one document to read.
    roots.insert("TextFields", schema_for!(TextFields));
    roots.insert("SearchKind", schema_for!(SearchKind));

    roots.insert("TaskQuery", schema_for!(TaskQuery));
    roots.insert("ProjectQuery", schema_for!(ProjectQuery));
    roots.insert("DocumentQuery", schema_for!(DocumentQuery));
    roots.insert("PageRequest", schema_for!(PageRequest));
    roots.insert("PageOfTask", schema_for!(Page<Task>));
    roots.insert("PageOfProject", schema_for!(Page<Project>));
    roots.insert("PageOfDocument", schema_for!(Page<Document>));
    roots.insert("PageOfLabel", schema_for!(Page<Label>));
    roots.insert("PageOfDependencyEdge", schema_for!(Page<DependencyEdge>));

    roots.insert("Capabilities", schema_for!(Capabilities));
    roots.insert("Health", schema_for!(Health));
    roots.insert("SourceError", schema_for!(SourceError));

    roots.insert("GlobalId", schema_for!(GlobalId));
    roots.insert("PageToken", schema_for!(PageToken));
    roots.insert("QueryPlan", schema_for!(QueryPlan));
    roots.insert("SourcePlan", schema_for!(SourcePlan));
    roots.insert("Predicate", schema_for!(Predicate));
    roots.insert("SourceFailure", schema_for!(SourceFailure));
    // What a command writes to standard output when it exits `1` under machine output,
    // and the two types inside it a caller branches on by name.
    roots.insert("FailureDocument", schema_for!(FailureDocument));
    roots.insert("Failure", schema_for!(Failure));
    roots.insert("FailureClass", schema_for!(FailureClass));
    roots.insert("QualifiedTask", schema_for!(Qualified<Task>));
    roots.insert("QualifiedProject", schema_for!(Qualified<Project>));
    roots.insert("QualifiedDocument", schema_for!(Qualified<Document>));
    roots.insert("QualifiedLabel", schema_for!(Qualified<Label>));
    roots.insert("QualifiedEdge", schema_for!(QualifiedEdge));
    roots.insert("SearchHit", schema_for!(SearchHit));
    roots.insert("SourceListing", schema_for!(SourceListing));
    roots.insert("SourceListings", schema_for!(Vec<SourceListing>));
    roots.insert(
        "QueryResponseOfQualifiedTask",
        schema_for!(QueryResponse<Qualified<Task>>),
    );
    roots.insert(
        "QueryResponseOfQualifiedProject",
        schema_for!(QueryResponse<Qualified<Project>>),
    );
    roots.insert(
        "QueryResponseOfQualifiedDocument",
        schema_for!(QueryResponse<Qualified<Document>>),
    );
    roots.insert(
        "QueryResponseOfQualifiedLabel",
        schema_for!(QueryResponse<Qualified<Label>>),
    );
    roots.insert(
        "QueryResponseOfQualifiedEdge",
        schema_for!(QueryResponse<QualifiedEdge>),
    );
    roots.insert(
        "QueryResponseOfSearchHit",
        schema_for!(QueryResponse<SearchHit>),
    );

    // The comment roots: the contract's own `Comment` and `NewComment`, which cross the
    // plugin protocol, and the three shapes the comment verbs and `task show` answer with.
    roots.insert("Comment", schema_for!(Comment));
    roots.insert("NewComment", schema_for!(NewComment));
    roots.insert("PageOfComment", schema_for!(Page<Comment>));
    roots.insert("CommentList", schema_for!(CommentList));
    roots.insert("DeletedComment", schema_for!(DeletedComment));
    roots.insert("TaskDetail", schema_for!(TaskDetail));

    // What `task status set` answers with, and the per-task entries it and a copy report for
    // the delivered tasks they kept in step — plus the entry type both of a task's lists hold.
    roots.insert("TaskRef", schema_for!(TaskRef));
    roots.insert("TaskStatusSet", schema_for!(TaskStatusSet));
    roots.insert("Delivered", schema_for!(Delivered));
    roots.insert("DeliveryOutcome", schema_for!(DeliveryOutcome));

    roots.insert("CopyReport", schema_for!(CopyReport));
    roots.insert("CopyOutcome", schema_for!(CopyOutcome));
    roots.insert("CopyAction", schema_for!(CopyAction));

    roots.insert("EffectiveConfig", schema_for!(EffectiveConfig));
    roots.insert("Setting", schema_for!(Setting));
    roots.insert("Origin", schema_for!(Origin));
    roots.insert("OutputFormat", schema_for!(OutputFormat));
    roots.insert("SecretsReport", schema_for!(SecretsReport));
    roots.insert("ResolvedCredential", schema_for!(ResolvedCredential));
    roots.insert("CredentialLayer", schema_for!(CredentialLayer));

    let plugins: BTreeMap<String, Schema> = registry()
        .iter()
        .map(|plugin| (plugin.kind().to_owned(), plugin.config_schema()))
        .collect();

    json!({
        "version": SCHEMA_BUNDLE_VERSION,
        "roots": roots,
        "plugin_config": plugins,
    })
}
