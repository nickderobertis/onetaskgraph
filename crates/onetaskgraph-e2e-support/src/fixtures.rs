//! The one table every journey is written against.
//!
//! A journey is written once and run against **every** source kind, so no plugin is ever
//! proven by a suite of its own writing. A row says which registry plugin it stands for,
//! how to configure one over the shared dataset below, and what that configuration
//! *declares* — which is what lets one journey assert both the rows and the plan against
//! a source that filters natively and one that does not.
//!
//! `scripts/check-journey-matrix.sh` fails, naming the plugin, when a plugin the registry
//! knows has no row here. Every registered plugin is implemented, so every row carries a
//! working source fixture.

use onetaskgraph_plugin_api::{Capabilities, DependencySupport, Support};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use crate::common::{Sandbox, SourceBoundary};
use crate::loopback;

/// One row: a source kind, in one configuration, over the shared dataset.
pub struct Row {
    /// The registry plugin kind this row stands for.
    pub plugin: &'static str,
    /// This row's own name, unique across rows, used in failure messages.
    pub name: &'static str,
    /// How to build it.
    pub fixture: Ready,
}

/// Everything a journey needs in order to drive one configured source.
pub struct Ready {
    /// The `config:` block, given a sandbox to write into if the source needs files.
    pub block: fn(&Sandbox) -> Value,
    /// What this configuration declares it applies itself.
    pub declared: Declared,
    /// Whether this source can represent the complete cross-plugin dataset.
    pub complete_dataset: bool,
    /// Whether this source's *documents* carry the shared dataset's labels.
    ///
    /// Read only where the row holds documents at all, and separate from
    /// [`Declared::filter_by_label`] because the two say different things: that one is
    /// whether the source applies a label predicate itself, this one is whether its
    /// documents have labels for such a predicate to keep. Linear is why the table needs
    /// the difference — its published `Document` type has no `labels` field, where its
    /// `Issue` and `Project` do — so a document read of it reports none, a document write
    /// into it refuses a label by name, and the shared journeys below drive that answer
    /// rather than skipping the row.
    pub labels_its_documents: bool,
    /// Where this source says one of its entities is, or that it does not say.
    pub place: Place,
    /// The short handle this row's backend shows people for one of its tasks, or that it
    /// has none.
    pub handle: Handle,
}

/// Where a row's source says one entity is: its verb, its native id, this run's sandbox.
///
/// A function rather than a table of constants because a location is not always knowable
/// before the run: a source over a folder of files reports the path of the file behind each
/// item, which exists only once the sandbox does. `None` is the source not saying where an
/// entity is, which is not the same as saying it is nowhere.
pub type Place = fn(&Sandbox, &str, &str) -> Option<Placed>;

/// One location, split into the JSON key a consumer branches on and what it holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Placed {
    /// The `Location` variant's own key: `url` or `path`.
    pub key: &'static str,
    /// What that key holds.
    pub value: String,
}

impl Placed {
    /// This location as the human rendering says it: which kind of place, then the place.
    #[must_use]
    pub fn rendered(&self) -> String {
        format!("{} {}", self.key, self.value)
    }
}

/// One entity's location as the shared dataset states it, for every row serving it.
///
/// Read out of [`dataset`] rather than restated, so a row and the fixture it is built from
/// cannot disagree about where an item is.
fn dataset_place(_sandbox: &Sandbox, verb: &str, id: &str) -> Option<Placed> {
    let dataset = dataset();
    let held = dataset[format!("{verb}s")]
        .as_array()
        .expect("the shared dataset holds this kind")
        .iter()
        .find(|item| item["id"] == json!(id))
        .expect("the shared dataset holds this id");
    let location = &held["location"];
    for key in ["url", "path"] {
        if let Some(value) = location[key].as_str() {
            return Some(Placed {
                key,
                value: value.to_owned(),
            });
        }
    }
    None
}

/// The short handle a row's source reports for one task, by that task's native id.
///
/// A function for the reason [`Place`] is one: a handle is the *backend's* own name for an
/// item, so what it is belongs to the row rather than to the shared dataset. It is
/// deliberately not a field of [`dataset`] — a source with no handle of its own must report
/// none, and a dataset carrying one would hand every row the same answer and prove nothing.
///
/// `None` is this backend having no separate handle for its tasks, which is the contract's
/// case for a folder of Markdown and for an in-memory source alike, and is not the same as
/// a handle that happens to be empty.
pub type Handle = fn(&str) -> Option<String>;

fn no_handle(_id: &str) -> Option<String> {
    None
}

/// What one row's source declares, so a journey can assert the plan as well as the rows.
///
/// One field per field of [`Capabilities`], in that type's own order and spelled with that
/// type's own values, so a capability no journey happens to drive is still represented
/// here — a predicate a plugin declares and then ignores narrows the answer silently, and
/// a field this table has no room for is a field nothing above the plugin can catch.
///
/// A struct of its own rather than a `Capabilities`, because a row's entry is a *claim
/// about* what the plugin reports rather than a copy of it. [`Declared::claimed`] is where
/// the two are made comparable, and `every_row_declares_exactly_what_its_plugin_reports`
/// in `journeys.rs` fails, naming the row and the field, when they disagree.
pub struct Declared {
    /// Whether the source filters tasks to a named project itself.
    pub projects: Support,
    /// Whether the source has documents at all.
    ///
    /// Not a predicate, and so unlike every other `Support` here: it says what the source
    /// *holds*, the engine reads it once at the handshake rather than compensating for it,
    /// and a source declaring it unsupported is never asked for a document. The table
    /// carries rows of both kinds on purpose — the `in-memory`, `local-md` and GitHub
    /// Projects rows hold documents and `linear` holds none yet — so the shared document
    /// journeys drive the answer and the honest refusal against real difference rather than
    /// against a mock.
    pub documents: Support,
    /// Whether the source's tasks have comments at all.
    ///
    /// Not a predicate either, on exactly the terms [`Self::documents`] is not: it says what
    /// the source holds, and a source declaring it unsupported is never sent a comment call.
    pub comments: Support,
    /// Whether the source stores the image assets its records' content references.
    ///
    /// Not a predicate either, on the terms [`Self::documents`] is not: it says what the
    /// source holds, and a source declaring it unsupported is never handed an asset.
    pub assets: Support,
    /// Whether the source's tasks hold a priority at all.
    ///
    /// Not a predicate either, on the terms [`Self::documents`] is not: it says what the
    /// source holds, and a source declaring it unsupported reports every task at `none` and
    /// is never handed another priority.
    pub priority: Support,
    /// Whether the source keeps only the tasks whose priority a query lists, itself.
    pub filter_by_priority: Support,
    /// Whether the source keeps only the tasks with a comment created or edited since an
    /// instant, itself.
    pub filter_by_comment_activity: Support,
    /// Whether the source keeps only the tasks holding every metadata value a query names,
    /// itself.
    pub filter_by_metadata: Support,
    /// Whether the source keeps only the tasks copied from the origin a query names, itself.
    pub filter_by_origin: Support,
    /// Whether the source answers which of its items contradict their issue's closure.
    ///
    /// Not a predicate, on the terms [`Self::documents`] is not: a source whose status is its
    /// closure declares it unsupported and is never asked.
    pub reconcile_status: Support,
    /// Whether the source answers which of its items an issue reference names.
    ///
    /// Not a predicate, on the same terms.
    pub complete_by_issue: Support,
    /// Whether the source can select tasks belonging to no project.
    pub orphan_tasks: Support,
    /// Whether the source filters by label itself.
    pub filter_by_label: Support,
    /// Whether the source filters by status itself.
    pub filter_by_status: Support,
    /// Whether the source searches titles itself.
    pub search_title: Support,
    /// Whether the source searches bodies itself.
    pub search_content: Support,
    /// How far the source walks task dependencies itself.
    pub task_dependencies: DependencySupport,
    /// How far the source walks project dependencies itself.
    pub project_dependencies: DependencySupport,
    /// The largest page the source will serve.
    pub max_page_size: u32,
}

impl Declared {
    /// This declaration as the capability value a plugin would report.
    ///
    /// Spelled out field by field over a type with no `Default`, so a field added to the
    /// contract fails to compile here rather than going unreconciled.
    #[must_use]
    pub fn claimed(&self) -> Capabilities {
        Capabilities {
            projects: self.projects,
            documents: self.documents,
            comments: self.comments,
            assets: self.assets,
            priority: self.priority,
            filter_by_priority: self.filter_by_priority,
            filter_by_comment_activity: self.filter_by_comment_activity,
            filter_by_metadata: self.filter_by_metadata,
            filter_by_origin: self.filter_by_origin,
            reconcile_status: self.reconcile_status,
            complete_by_issue: self.complete_by_issue,
            orphan_tasks: self.orphan_tasks,
            filter_by_label: self.filter_by_label,
            filter_by_status: self.filter_by_status,
            search_title: self.search_title,
            search_content: self.search_content,
            task_dependencies: self.task_dependencies,
            project_dependencies: self.project_dependencies,
            max_page_size: self.max_page_size,
        }
    }

    /// Every field where this declaration and `reported` differ, named.
    ///
    /// One entry per disagreeing field rather than one whole-value comparison, because
    /// the failure a reader needs is *which capability* the table is lying about.
    #[must_use]
    pub fn disagreements(&self, reported: &Capabilities) -> Vec<String> {
        let claimed = self.claimed();
        let support = |field: &'static str, claimed: Support, reported: Support| {
            (claimed != reported).then(|| {
                format!("{field}: the table declares {claimed:?}, the plugin reports {reported:?}")
            })
        };
        let dependencies = |field: &'static str,
                            claimed: DependencySupport,
                            reported: DependencySupport| {
            (claimed != reported).then(|| {
                format!("{field}: the table declares {claimed:?}, the plugin reports {reported:?}")
            })
        };
        [
            support("projects", claimed.projects, reported.projects),
            support("documents", claimed.documents, reported.documents),
            support("comments", claimed.comments, reported.comments),
            support("priority", claimed.priority, reported.priority),
            support(
                "filter_by_priority",
                claimed.filter_by_priority,
                reported.filter_by_priority,
            ),
            support(
                "filter_by_comment_activity",
                claimed.filter_by_comment_activity,
                reported.filter_by_comment_activity,
            ),
            support(
                "filter_by_metadata",
                claimed.filter_by_metadata,
                reported.filter_by_metadata,
            ),
            support(
                "filter_by_origin",
                claimed.filter_by_origin,
                reported.filter_by_origin,
            ),
            support(
                "reconcile_status",
                claimed.reconcile_status,
                reported.reconcile_status,
            ),
            support(
                "complete_by_issue",
                claimed.complete_by_issue,
                reported.complete_by_issue,
            ),
            support("orphan_tasks", claimed.orphan_tasks, reported.orphan_tasks),
            support(
                "filter_by_label",
                claimed.filter_by_label,
                reported.filter_by_label,
            ),
            support(
                "filter_by_status",
                claimed.filter_by_status,
                reported.filter_by_status,
            ),
            support("search_title", claimed.search_title, reported.search_title),
            support(
                "search_content",
                claimed.search_content,
                reported.search_content,
            ),
            dependencies(
                "task_dependencies",
                claimed.task_dependencies,
                reported.task_dependencies,
            ),
            dependencies(
                "project_dependencies",
                claimed.project_dependencies,
                reported.project_dependencies,
            ),
            (claimed.max_page_size != reported.max_page_size).then(|| {
                format!(
                    "max_page_size: the table declares {}, the plugin reports {}",
                    claimed.max_page_size, reported.max_page_size
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

impl Row {
    /// This row as a configuration document naming one source, `work`.
    ///
    /// Written as JSON, which the YAML reader accepts, so a fixture is a value rather
    /// than a string a test has to indent correctly.
    pub fn document(&self, sandbox: &Sandbox) -> String {
        let block = (self.fixture.block)(sandbox);
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block}
        }))
    }

    /// This row as a document naming `work` and one writable Markdown folder beside it.
    ///
    /// The copy journeys need a destination, and a folder of Markdown is the one every
    /// row can be copied into: it is the source this plan makes writable, and it is what
    /// the user's own flow writes into and edits.
    pub fn document_with_folder(&self, sandbox: &Sandbox, folder: &str) -> String {
        let block = (self.fixture.block)(sandbox);
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block},
            folder: {"plugin": "local-md", "config": empty_folder(sandbox, folder)},
        }))
    }

    /// This row as a document naming `work` and one empty document-bearing source beside
    /// it.
    ///
    /// A second `in-memory` source rather than a folder of Markdown, unlike
    /// [`document_with_folder`](Self::document_with_folder): a destination that dies with
    /// the process is all a refusal needs, and it keeps a refusal journey from depending on
    /// whichever plugin happens to hold documents today.
    pub fn document_with_store(&self, sandbox: &Sandbox, store: &str) -> String {
        let block = (self.fixture.block)(sandbox);
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block},
            store: {"plugin": "in-memory", "config": empty_document_store()},
        }))
    }

    /// This row as a document naming `work` and one source that declares it has no
    /// documents beside it.
    ///
    /// The destination a document copy must be refused by. An `in-memory` source without
    /// the `documents` key declares exactly that — and it is one rather than a folder of
    /// Markdown because `local-md` now holds documents, which is what a destination naming
    /// none has to *not* be.
    pub fn document_with_documentless(&self, sandbox: &Sandbox, store: &str) -> String {
        let block = (self.fixture.block)(sandbox);
        document(&json!({
            SOURCE: {"plugin": self.plugin, "config": block},
            store: {"plugin": "in-memory", "config": json!({})},
        }))
    }

    /// What this row declares.
    pub fn declared(&self) -> &Declared {
        &self.fixture.declared
    }
}

/// An empty in-memory source that has documents, ready to be copied into.
pub fn empty_document_store() -> Value {
    json!({"capabilities": {"documents": "native"}})
}

/// An empty Markdown folder, ready to be copied into.
///
/// The status mapping covers every status name the shared dataset spells, because a
/// destination that would read a written status back as something else refuses the write
/// rather than narrowing it — which is right, and is not what these journeys are about.
pub fn empty_folder(sandbox: &Sandbox, relative: &str) -> Value {
    json!({
        "root": sandbox.subdirectory(relative),
        "status_mapping": {"todo": "todo", "doing": "in-progress", "shipped": "done"},
    })
}

pub fn document(sources: &Value) -> String {
    serde_json::to_string_pretty(&json!({ "sources": sources })).expect("a fixture renders")
}

/// The name every single-source journey configures its source under.
pub const SOURCE: &str = "work";

/// The name the two-source journeys give the row that applies everything itself.
pub const NATIVE: &str = "native";

/// The name they give the row that applies none of it and walks forwards only.
pub const SCANNED: &str = "scanned";

/// The capability pair built on either side of the process boundary.
pub fn pair_at(sandbox: &Sandbox, boundary: SourceBoundary) -> String {
    let mut sources = serde_json::Map::new();
    for (name, row) in [(NATIVE, &ROWS[0]), (SCANNED, &ROWS[1])] {
        sources.insert(
            name.to_owned(),
            boundary.source(row.plugin, (row.fixture.block)(sandbox)),
        );
    }
    document(&Value::Object(sources))
}

/// `<source>:<native>`, the form a user types.
///
/// Spelled here rather than inline so a journey asserting on an id is asserting on the
/// rendering under test rather than on its own `format!`.
pub fn qualified(source: &str, native: &str) -> String {
    format!("{source}:{native}")
}

/// Every row a journey runs against.
///
/// The two `in-memory` rows are the pair that proves pushdown and compensation return
/// one correct answer by two different plans: they hold the same dataset and the same
/// dependency graph, and differ only in what they declare. One answers reverse
/// dependencies itself and one does not, which is what makes the engine's emulated
/// reverse scan exercised deliberately here rather than incidentally by whichever plugin
/// happens to be poor at it.
pub const ROWS: &[Row] = &[
    Row {
        plugin: "in-memory",
        name: "in-memory (declares everything native)",
        fixture: Ready {
            block: native_block,
            complete_dataset: true,
            labels_its_documents: true,
            place: dataset_place,
            handle: no_handle,
            declared: Declared {
                documents: Support::Native,
                comments: Support::Native,
                priority: Support::Native,
                ..EVERY_PREDICATE_NATIVE
            },
        },
    },
    Row {
        plugin: "in-memory",
        // `projects` is the one field this row cannot drop, and the name says so rather
        // than reading as an oversight somebody would helpfully "fix": in the contract
        // that field means *this source has projects at all*, so a source declaring it
        // unsupported contributes no project rows and the engine reports the predicate
        // unreachable instead of compensating. Every field where compensation is sound is
        // unsupported here on purpose — this row is the engine's compensation path's only
        // coverage, not a plugin someone forgot to finish.
        name: "in-memory (compensated: nothing native but its project table, forward-only)",
        fixture: Ready {
            block: compensated_block,
            complete_dataset: true,
            labels_its_documents: true,
            place: dataset_place,
            handle: no_handle,
            declared: Declared {
                projects: Support::Native,
                // Native here for the same reason `projects` is: it says this source
                // *holds* documents, and a row declaring it unsupported would contribute
                // no document rows at all — which would leave the engine's document
                // compensation with no coverage, not less of it.
                documents: Support::Native,
                comments: Support::Native,
                assets: Support::Unsupported,
                // Held for the reason `documents` is: a task's priority is what the priority
                // filter this row leaves to the engine compares against.
                priority: Support::Native,
                filter_by_priority: Support::Unsupported,
                filter_by_comment_activity: Support::Unsupported,
                filter_by_metadata: Support::Unsupported,
                filter_by_origin: Support::Unsupported,
                reconcile_status: Support::Unsupported,
                complete_by_issue: Support::Unsupported,
                orphan_tasks: Support::Unsupported,
                filter_by_label: Support::Unsupported,
                filter_by_status: Support::Unsupported,
                search_title: Support::Unsupported,
                search_content: Support::Unsupported,
                task_dependencies: DependencySupport::ForwardOnly,
                project_dependencies: DependencySupport::ForwardOnly,
                max_page_size: 2,
            },
        },
    },
    Row {
        plugin: "subprocess",
        name: "subprocess (the in-memory source over a real pipe)",
        fixture: Ready {
            block: hosted_block,
            complete_dataset: true,
            labels_its_documents: true,
            place: dataset_place,
            handle: no_handle,
            declared: Declared {
                documents: Support::Native,
                comments: Support::Native,
                priority: Support::Native,
                ..EVERY_PREDICATE_NATIVE
            },
        },
    },
    Row {
        plugin: "local-md",
        name: "local-md",
        fixture: Ready {
            block: local_md_block,
            complete_dataset: true,
            // A `labels:` key in the front matter of a document, exactly as in a task's.
            labels_its_documents: true,
            // A folder of files, so every entity is a file and this source says where each
            // one is — the only row of this table whose locations are not knowable until
            // the sandbox holding them exists.
            place: local_md_place,
            handle: no_handle,
            declared: Declared {
                // A third folder beside `tasks/` and `projects/`: this source holds
                // documents, and reads, filters and writes them on the same terms.
                documents: Support::Native,
                // A trailing section of each task's own file.
                comments: Support::Native,
                // A `priority:` key in a task's front matter.
                priority: Support::Native,
                // A folder of its own beside each record's file.
                assets: Support::Native,
                max_page_size: 200,
                ..EVERY_PREDICATE_NATIVE
            },
        },
    },
    Row {
        plugin: "linear",
        name: "linear",
        fixture: Ready {
            block: linear_block,
            // Every issue, project and document of a Linear workspace has a page a person
            // can open, so this row's source says where all three are — as a link, which
            // is the shape a folder of Markdown's path is not.
            place: linear_place,
            handle: linear_handle,
            // Linear models the whole table: two projects, an orphan, and dependencies in
            // both directions, so it drives the shared complete-dataset journeys.
            complete_dataset: true,
            // The one row of this table whose documents carry no labels, and Linear's
            // published schema is why: `Document` has no `labels` field where `Issue` and
            // `Project` do. So the shared document journeys assert the answer that follows
            // from it — a label demanded of a document keeps nothing, a label excluded
            // keeps everything — against a real remote protocol rather than against a mock.
            labels_its_documents: false,
            // Every predicate is sent to Linear and confirmed in process, the follow-up
            // searches included — see the verdict table in `onetaskgraph-linear`'s own module
            // documentation — so this row proves pushdown against a remote protocol, and the
            // `scanned` row is what proves the engine's compensation.
            declared: Declared {
                // Linear's own first-class `Document`, read through `documents(…)` and
                // written through `documentCreate`/`documentUpdate`.
                documents: Support::Native,
                assets: Support::Native,
                // A Linear issue's own comments.
                comments: Support::Native,
                // Linear's own issue priority, 0 to 4.
                priority: Support::Native,
                max_page_size: onetaskgraph_linear::MAX_PAGE_SIZE,
                ..EVERY_PREDICATE_NATIVE
            },
        },
    },
    Row {
        plugin: "github-projects",
        name: "github-projects",
        fixture: Ready {
            block: github_projects_block,
            // Every entity here is an issue and every issue has a web address, so this row
            // is the one whose places are all links.
            place: github_place,
            handle: github_handle,
            // A document here is an issue, and an issue carries labels, so this row's
            // documents are labelled exactly as its tasks are.
            labels_its_documents: true,
            // A board is a container of projects, not a project: a project is an issue and
            // its tasks are that issue's sub-issues, which is what this source's own module
            // documentation records and what the fixture board below is built as. So one
            // GitHub source represents the whole shared dataset — two projects with tasks
            // filed under each, and a task filed under neither — and drives every shared
            // journey rather than a subset chosen for it.
            complete_dataset: true,
            // The source walks the whole board before it answers anything, so it applies
            // every predicate a query carries itself; `GitHubProjectsSource`'s own module
            // documentation records why that is what `Native` means here.
            //
            // `documents` included: a board has no document type, so this source spells
            // one as an issue whose title begins its own design prefix — which is what the
            // three `D-*` items of the fixture board above are.
            declared: Declared {
                documents: Support::Native,
                assets: Support::Native,
                // An issue's own comments; a draft has none, and says so.
                comments: Support::Native,
                // The board's `Priority` field, which this row's configuration maps.
                priority: Support::Native,
                // A card's `Status` option beside an issue that closes on its own: the one
                // source whose card can contradict its closure, and whose items an issue
                // reference names.
                reconcile_status: Support::Native,
                complete_by_issue: Support::Native,
                max_page_size: 100,
                ..EVERY_PREDICATE_NATIVE
            },
        },
    },
];

/// Every entity a GitHub Projects board reports is an issue, and every issue has a web
/// address, so every one of them is a link and none of them is absent.
///
/// This is the contrast the location contract exists for, and it is real difference rather
/// than a second copy of `local-md`'s: one reader is handed something to open and another
/// a path to read out, and neither has to know which plugin answered. The address is the
/// one the fixture board's own GraphQL responses carry, so the row and the board it is
/// built from cannot disagree about where an item is.
fn github_place(_sandbox: &Sandbox, _verb: &str, id: &str) -> Option<Placed> {
    Some(Placed {
        key: "url",
        value: format!("https://example.invalid/{id}"),
    })
}

/// The declaration a source that applies every predicate itself carries.
///
/// *Predicate* is the whole of the name, and `documents` below is why: that field is not
/// one, so a source can apply every predicate natively and still hold no documents. A name
/// saying `EVERYTHING` would contradict the value.
///
/// A named constant because five of the six rows differ from it in at most two fields,
/// and a row spelled out in full is a row whose one interesting difference is buried.
/// `max_page_size` is the in-memory default; a row whose plugin picks its own overrides it.
const EVERY_PREDICATE_NATIVE: Declared = Declared {
    projects: Support::Native,
    // Unsupported even here, because `documents` is not a predicate: "native" would claim
    // this source *holds* documents rather than that it filters them itself, which is what
    // keeps this constant's name true. The rows whose source really does hold documents
    // override it; docs/follow-ups.md tracks the two plugins that do not.
    documents: Support::Unsupported,
    // Unsupported here for the reason `documents` is: not a predicate, so "every predicate
    // native" says nothing about whether a source's tasks have comments. The rows whose
    // source really does hold them override it.
    comments: Support::Unsupported,
    // Unsupported here for the reason `documents` is: whether a source stores image assets is
    // not a predicate. The row whose source really does keep them overrides it.
    assets: Support::Unsupported,
    // Unsupported here for the reason `documents` is: whether a source's tasks hold a
    // priority is not a predicate. Every row whose source holds one overrides it.
    priority: Support::Unsupported,
    filter_by_priority: Support::Native,
    filter_by_comment_activity: Support::Native,
    filter_by_metadata: Support::Native,
    filter_by_origin: Support::Native,
    // Unsupported here for the reason `documents` is: neither is a predicate. The row whose
    // source answers both overrides it.
    reconcile_status: Support::Unsupported,
    complete_by_issue: Support::Unsupported,
    orphan_tasks: Support::Native,
    filter_by_label: Support::Native,
    filter_by_status: Support::Native,
    search_title: Support::Native,
    search_content: Support::Native,
    task_dependencies: DependencySupport::BothDirections,
    project_dependencies: DependencySupport::BothDirections,
    max_page_size: 50,
};
fn github_projects_block(sandbox: &Sandbox) -> Value {
    github_projects_server(sandbox, None)
}

/// The same board, with `T-1` recording `recorded` under the reserved dependency key.
///
/// The journeys that drive a key holding something it must not need a board that holds
/// it, and the shared row cannot be that board — it is the one every other journey reads.
pub fn github_projects_recording(sandbox: &Sandbox, recorded: Value) -> Value {
    github_projects_server(sandbox, Some(recorded))
}

/// The issue number this fixture board gives one item, shared by the fake and the journey
/// that asserts on it so the two cannot drift.
pub fn github_number(id: &str) -> u64 {
    let digits: String = id.chars().filter(char::is_ascii_digit).collect();
    let tail: u64 = digits.parse().expect("a fixture id ends in digits");
    let level = match id.chars().next() {
        Some('P') => 20,
        Some('D') => 40,
        // `ISSUE-<n>`, an issue a run of a journey created. Its own band, so a created
        // issue never wears a number the shared dataset already gave something else.
        Some('I') => 60,
        // `N-<n>`, an issue a journey names by its number: a band past every other, so a
        // reference `owner/name#<number>` names exactly one issue of the board.
        Some('N') => 99_000,
        _ => 0,
    };
    1000 + level + tail
}

fn github_handle(id: &str) -> Option<String> {
    Some(github_number(id).to_string())
}

/// One item on the fixture board, in the shape the fixture keeps it between requests.
///
/// A board is a container of projects: `T-1`..`T-4` are task issues, and `P-1` and `P-2`
/// are project issues, readable as projects because they carry this source's own kind
/// marker rather than because the board is one.
///
/// [`Placement::parent`] is what makes that structure real rather than asserted, and the
/// board below sets it: the shared dataset's two projects hold their own tasks and one
/// task is filed under neither. Before it carried parents, every task on this board was an
/// orphan and no filter scoped to a project could have separated anything.
fn github_item(
    id: &str,
    title: &str,
    body: &str,
    at: Placement,
    labels: Value,
    slot: Value,
) -> Value {
    // The board field is where an origin lives; the slot's copy is the mirror a write keeps.
    let origin = slot
        .get("onetaskgraph.origin")
        .cloned()
        .unwrap_or_else(|| json!(""));
    let body = if slot.as_object().is_some_and(serde_json::Map::is_empty) {
        body.to_owned()
    } else {
        format!("{body}\n\n<!-- onetaskgraph.metadata\n{slot}\n-->")
    };
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,"body":body,
           "state":at.state.0,"reason":at.state.1,
           "parent":at.parent.map_or(Value::Null, |id| json!(id)),
           "repo":"nickderobertis/onetaskgraph","status":at.status,"origin":origin,
           "priority":at.priority,"labels":labels,
           "updatedAt":GITHUB_BOARD_EPOCH,"itemUpdatedAt":GITHUB_BOARD_EPOCH})
}

/// Where one fixture item sits on the board.
///
/// The three facts GitHub keeps about an issue's *position* rather than its content, in
/// one value: which `Status` option the board gives it, whether the issue is open or
/// closed and why, and which issue it is a sub-issue of.
struct Placement<'a> {
    /// The name of the board `Status` option on this item.
    status: &'a str,
    /// `IssueState`, and the `IssueStateReason` behind a closed one.
    state: (&'a str, Option<&'a str>),
    /// The issue this one is filed under, which is what project membership is here.
    parent: Option<&'a str>,
    /// The name of the board `Priority` option on this item, when it has one.
    priority: Option<&'a str>,
}

fn github_dataset(recorded: Option<&Value>) -> Vec<Value> {
    let marked = |extra: Value| {
        let mut slot = json!({"onetaskgraph.item_kind":"project"});
        for (key, value) in extra.as_object().expect("an object") {
            slot[key] = value.clone();
        }
        slot
    };
    vec![
        github_item(
            "T-1",
            "Alpha engine",
            "the engine core",
            Placement {
                status: "Todo",
                state: ("OPEN", None),
                parent: Some("P-1"),
                priority: Some("High"),
            },
            json!([["L-1", "bug"], ["L-3", "core"]]),
            json!({"onepipeline.turn_budget":12,"caller.flags":[true,null],
                   "onetaskgraph.depends_on":recorded.cloned().unwrap_or_else(||
                       Value::Array(recorded_far_ends("task_dependencies", &json!("T-1"))))}),
        ),
        github_item(
            "T-2",
            "Beta",
            "alpha in the body",
            Placement {
                status: "Shipped",
                state: ("CLOSED", Some("COMPLETED")),
                parent: Some("P-1"),
                priority: None,
            },
            json!([["L-2", "chore"]]),
            json!({}),
        ),
        github_item(
            "T-3",
            "Gamma",
            "unrelated",
            Placement {
                status: "Todo",
                state: ("OPEN", None),
                parent: None,
                priority: Some("Urgent"),
            },
            json!([["L-1", "bug"]]),
            json!({}),
        ),
        github_item(
            "T-4",
            "Delta docs",
            "documentation",
            Placement {
                status: "Doing",
                state: ("OPEN", None),
                parent: Some("P-2"),
                priority: Some("Low"),
            },
            json!([["L-3", "core"]]),
            // A copy's origin, in the board field every release reads it from and mirrored in
            // the slot the way this release writes it.
            json!({"onetaskgraph.origin":"elsewhere:ORIG-4",
                   "orchestrator.follow-up":{"root_cause":"stale-cache"}}),
        ),
        github_item(
            "P-1",
            "Engine",
            "the engine",
            Placement {
                status: "Doing",
                state: ("OPEN", None),
                parent: None,
                priority: None,
            },
            json!([["L-3", "core"]]),
            marked(json!({"onepipeline.publication":{"mode":"review"},
                          "onetaskgraph.depends_on":recorded_far_ends("project_dependencies", &json!("P-1"))})),
        ),
        github_item(
            "P-2",
            "Docs",
            "alpha docs",
            Placement {
                status: "Todo",
                state: ("OPEN", None),
                parent: None,
                priority: None,
            },
            json!([]),
            marked(json!({})),
        ),
        // The shared dataset's three documents. A board has no document type, so each is an
        // ordinary issue whose title begins with this source's own design prefix — taken
        // from the plugin rather than spelled again here, so this fixture cannot drift from
        // what the source reads. The prefix is what makes them documents: `D-1` carries the
        // *project* marker and sub-issues of its own, `D-3` has neither and would otherwise
        // be an empty project, and `D-2` is a sub-issue, so each of the three arms that
        // separate a project from a task is present on one of them and each must lose.
        github_document(
            "D-1",
            "Alpha design",
            "the engine core, reviewed",
            Some("P-1"),
            json!([["L-1", "bug"]]),
            json!({"onepipeline.turn_budget":12,"caller.flags":[true,null]}),
        ),
        github_document(
            "D-2",
            "Runbook",
            "how to read the alpha design",
            Some("P-2"),
            json!([["L-3", "core"]]),
            json!({}),
        ),
        github_document(
            "D-3",
            "Loose note",
            "filed nowhere",
            None,
            json!([]),
            json!({}),
        ),
    ]
}

/// One document on the fixture board: an issue titled the way this source spells one.
///
/// A document has no status, so this takes none — the issue sits open in whatever column
/// the board gives it, and nothing this source reports about a document reads it.
fn github_document(
    id: &str,
    title: &str,
    body: &str,
    parent: Option<&str>,
    labels: Value,
    slot: Value,
) -> Value {
    github_item(
        id,
        &format!(
            "{}{title}",
            onetaskgraph_github_projects::DESIGN_TITLE_PREFIX
        ),
        body,
        Placement {
            status: "Todo",
            state: ("OPEN", None),
            parent,
            priority: None,
        },
        labels,
        slot,
    )
}

/// The board's own `blockedBy` graph: which item waits on which.
///
/// The dataset's fourth task edge is `related` rather than `blocks`, and GitHub has no
/// such relation — `blockedBy` is the only native one an issue has — so this board spells
/// it the only way it can. The shared journeys assert the edge's *ends*, which is the
/// property every backend owes; the kind a backend cannot express is not one of them.
fn github_blockers() -> Vec<(String, Vec<String>)> {
    [
        ("T-1", vec!["T-2"]),
        ("T-3", vec!["T-2"]),
        ("T-4", vec!["T-2"]),
        ("P-1", vec!["P-2"]),
    ]
    .into_iter()
    .map(|(id, blockers)| {
        (
            id.to_owned(),
            blockers.into_iter().map(str::to_owned).collect(),
        )
    })
    .collect()
}

/// Whether a relationship between two issues was made or taken away.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] Which of a relationship's two timeline events the loopback board records, so it is part of `TimelineEvent`'s vocabulary and lives beside it, for the reason recorded on `GitHubBoard` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationshipChange {
    Added,
    Removed,
}

impl RelationshipChange {
    /// The change a mutation makes: `added` names the mutation that makes the relationship.
    const fn made_by(added: bool) -> Self {
        if added { Self::Added } else { Self::Removed }
    }
}

/// One relationship event GitHub records on an issue's timeline: every type this board models,
/// each spelled as `IssueTimelineItemsItemType` spells it.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] The loopback board's vocabulary of GitHub timeline events, which only `GitHubBoard` records and counts; the board lives in this shared harness for the reason recorded on `GitHubBoard` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimelineEvent {
    BlockedByAdded,
    BlockedByRemoved,
    BlockingAdded,
    BlockingRemoved,
    ParentIssueAdded,
    ParentIssueRemoved,
    SubIssueAdded,
    SubIssueRemoved,
}

impl TimelineEvent {
    const ALL: [Self; 8] = [
        Self::BlockedByAdded,
        Self::BlockedByRemoved,
        Self::BlockingAdded,
        Self::BlockingRemoved,
        Self::ParentIssueAdded,
        Self::ParentIssueRemoved,
        Self::SubIssueAdded,
        Self::SubIssueRemoved,
    ];

    /// The event a dependency `change` records on the blocked issue, and the one it records on
    /// the blocking issue.
    const fn dependency(change: RelationshipChange) -> (Self, Self) {
        match change {
            RelationshipChange::Added => (Self::BlockedByAdded, Self::BlockingAdded),
            RelationshipChange::Removed => (Self::BlockedByRemoved, Self::BlockingRemoved),
        }
    }

    /// The event a project membership `change` records on the sub-issue, and the one it
    /// records on its parent.
    const fn membership(change: RelationshipChange) -> (Self, Self) {
        match change {
            RelationshipChange::Added => (Self::ParentIssueAdded, Self::SubIssueAdded),
            RelationshipChange::Removed => (Self::ParentIssueRemoved, Self::SubIssueRemoved),
        }
    }

    /// GitHub's spelling of this type.
    const fn name(self) -> &'static str {
        match self {
            Self::BlockedByAdded => "BLOCKED_BY_ADDED_EVENT",
            Self::BlockedByRemoved => "BLOCKED_BY_REMOVED_EVENT",
            Self::BlockingAdded => "BLOCKING_ADDED_EVENT",
            Self::BlockingRemoved => "BLOCKING_REMOVED_EVENT",
            Self::ParentIssueAdded => "PARENT_ISSUE_ADDED_EVENT",
            Self::ParentIssueRemoved => "PARENT_ISSUE_REMOVED_EVENT",
            Self::SubIssueAdded => "SUB_ISSUE_ADDED_EVENT",
            Self::SubIssueRemoved => "SUB_ISSUE_REMOVED_EVENT",
        }
    }

    /// The timeline event types `query` counts: the default list of the
    /// `[IssueTimelineItemsItemType!]` variable it declares, or none. A type this board does
    /// not model is a document this board cannot answer truthfully, so it refuses it by name.
    fn counted_by(query: &str) -> Vec<Self> {
        query
            .split_once("[IssueTimelineItemsItemType!]=[")
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(list, _)| {
                list.split(',')
                    .map(|spelled| {
                        let spelled = spelled.trim();
                        Self::ALL
                            .into_iter()
                            .find(|kind| kind.name() == spelled)
                            .unwrap_or_else(|| {
                                panic!("the loopback board models no timeline event {spelled}")
                            })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Where a page of the comments on issue `issue` ended: after `offset` of them, the last being
/// comment `last`, at `position - 1` of the board's comments.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] Private state of `GitHubBoard` below, which no suite can reach but through that board's handle; it lives beside the board for the reason the board's own directive gives.
#[derive(Debug)]
struct CommentResume {
    issue: String,
    offset: usize,
    position: usize,
    last: String,
}

/// How many comments issue `issue` held, counted when the board held `comments` of them all,
/// the last of those being comment `last` — `None` when it held none.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] Private state of `GitHubBoard` below, which no suite can reach but through that board's handle; it lives beside the board for the reason the board's own directive gives.
#[derive(Debug)]
struct CommentCount {
    issue: String,
    comments: usize,
    last: Option<String>,
    count: usize,
}

// llmlint: ignore[code_lands_in_the_domain_that_owns_it] The measured delays the shared loopback board's placement model is driven with, beside that model: AGENTS.md puts every loopback server and what configures it in this shared harness, once.
/// How long after its create was answered GitHub placed each child issue a create filed on a
/// board with `projectV2Ids`, in milliseconds: the feasibility spike's ten samples of eight
/// children each (`board-copy-gaps-2026-10-24-spike-feasibility-report`, its `probe-placement`
/// runs), in the order they were measured. None of those creates' answers carried the item.
pub const MEASURED_PLACEMENT_MILLIS: [u64; 80] = [
    453, 453, 453, 453, 453, 453, 453, 1423, 482, 482, 482, 482, 482, 482, 482, 1622, 509, 509,
    509, 509, 509, 509, 509, 1555, 548, 548, 548, 548, 548, 548, 548, 1553, 577, 577, 577, 577,
    577, 577, 577, 1526, 537, 537, 537, 537, 537, 537, 537, 537, 523, 523, 523, 523, 523, 523, 523,
    1837, 614, 614, 614, 614, 614, 614, 614, 614, 626, 626, 626, 626, 626, 626, 626, 1692, 568,
    568, 568, 568, 568, 568, 568, 568,
];

// llmlint: ignore[code_lands_in_the_domain_that_owns_it] The loopback GitHub board's own placement state, which every read of it answers from: AGENTS.md puts every loopback server in this shared harness, once, so the plan-copy journeys of the GitHub suite and any other suite driving this board read one model of GitHub's delayed placement rather than copies of it.
/// GitHub's delayed placement of an issue `createIssue` files on a board through
/// `projectV2Ids`: the create is answered with no board item, and the item appears on the board
/// some time later — between 0.45 and 1.84 seconds after the answer in the feasibility spike
/// (`board-copy-gaps-2026-10-24-spike-feasibility-report`), and never in the create's own answer.
struct BoardPlacement {
    /// The time on whatever clock the journey runs on.
    now: Arc<dyn Fn() -> Duration + Send + Sync>,
    /// The delay of each create in turn, cycled.
    delays: Vec<Duration>,
    /// How many delays were handed out.
    next: usize,
    /// Each issue waiting to be placed: its id, its delay, and when it is placed — `None` until
    /// the create that made it has been answered.
    waiting: Vec<(Value, Duration, Option<Duration>)>,
}

/// The board this fixture keeps, and everything a request may change on it.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] The shared e2e harness is where the agreed e2e layout puts every loopback server, so no suite copies one: the journey-matrix table in this file builds each of its rows through both hosted plugins' loopbacks, and the engine's own suite drives both, so neither can live in one plugin's suite. Editing it re-selects every suite, as editing any shared harness does.
struct GitHubBoard {
    items: Vec<Value>,
    /// Every comment on every issue this board holds, oldest first.
    ///
    /// Held here rather than per request because the server outlives one invocation of the
    /// binary, so a comment one command adds is what the next command lists, edits or removes.
    comments: Vec<Value>,
    /// Where the last page of comments ended — see [`CommentResume`] — so the next page of the
    /// same walk starts there rather than counting the issue's comments again from the first.
    comment_resume: std::sync::Mutex<Option<CommentResume>>,
    /// The last count [`Self::comment_count`] made, which answers `totalCount` on every page of
    /// a walk without counting the issue's comments again while none was added or removed.
    comment_counted: std::sync::Mutex<Option<CommentCount>>,
    /// How many comment timestamps this board has handed out, which makes each one later than
    /// the last and gives every comment an id no other comment has had.
    comment_ticks: u64,
    /// The second every change is stamped with while a journey holds this board's clock — so
    /// two changes share one, as two made inside one second on GitHub do — and `None` while
    /// each change takes the second after the last.
    held_second: Option<u64>,
    /// How long after its latest change this board is read: the `Date` every answer carries is
    /// that many seconds past it. An hour unless a journey reads it in the very second of a
    /// change — and `None` for an answer carrying no `Date` at all.
    read_lag: Option<u64>,
    /// Issues `createIssue` made which `addProjectV2ItemById` has not filed yet.
    pending: Vec<Value>,
    /// An alias of one batched field write this board fails, once, after performing every
    /// alias before it — the way GitHub runs a document's mutation fields in order and answers
    /// one that failed with `null` and an error, without undoing the ones before it.
    failing_alias: Option<&'static str>,
    /// An alias of one batched write this board performs and then answers malformed, once.
    bent_alias: Option<(&'static str, AliasAnswer)>,
    /// Whether the batched origin lookups answer the repository with a blank node id.
    blank_repository_id: bool,
    /// The id of a board field whose write, in whichever alias of a batched field write carries
    /// it, this board fails once in the same way.
    failing_field: Option<String>,
    /// The errors the request being answered owes beside its data.
    owed_errors: Vec<String>,
    /// Operations this board refuses, each once, named after it was built — so a journey
    /// can set a board up through the same operations and only then have one refused.
    refusing: Vec<&'static str>,
    /// Operations this board refuses once after answering that many requests carrying them —
    /// so a journey can let a read's first pages land and refuse a later one.
    refusing_after: Vec<(&'static str, usize)>,
    /// The `retry-after` the next request is refused `429 Too Many Requests` with, once.
    rate_limit_next: Option<u64>,
    /// Items this board replaces after answering that many requests naming them by id — with
    /// another item, or with nothing — so a journey can let a command read one and then find it
    /// changed or gone.
    replacing: Vec<(String, usize, Option<Value>)>,
    blocked_by: Vec<(String, Vec<String>)>,
    /// Every relationship event recorded since the board was built: the issue whose timeline
    /// holds it, the issue it names, and its type — so deleting an issue can take the events
    /// naming it off every other timeline, as GitHub does.
    relationship_log: Vec<(String, String, TimelineEvent)>,
    /// The timeline event types the request being answered counts: the default of the
    /// `[IssueTimelineItemsItemType!]` variable its document declares, which is the only way
    /// this source names them.
    counted_types: Vec<TimelineEvent>,
    created: usize,
    /// How many of the most recently filed items a board read leaves out.
    ///
    /// GitHub's `projectV2.items` is eventually consistent: an issue put on a board with
    /// `addProjectV2ItemById` is routinely absent from the very next read of that board.
    /// The board still holds it — every mutation below finds it — and only the *read* is
    /// behind, which is exactly what this models.
    lagging_reads: usize,
    /// Every GraphQL document this board has received, in order.
    documents: Vec<String>,
    /// The bearer token each of those requests carried, at the same index — which of the
    /// sources configured against this one server sent it.
    tokens: Vec<String>,
    /// When each of those requests arrived, at the same index.
    arrivals: Vec<std::time::Instant>,
    /// How many of an issue's board memberships the request being answered asks for, from its
    /// `$boardItems` or `$memberships` — every one when it names neither.
    membership_first: usize,
    /// Errors the request being answered owes beside its data, each a whole GraphQL error
    /// object — a `NOT_FOUND` naming the alias it is about.
    owed_error_objects: Vec<Value>,
    /// Whether the board's `is:closed` item filter also lets open items through, so a
    /// journey can hold a reader to confirming each item it is handed.
    closed_filter_leaks: bool,
    /// Whether the board's closed-items read answers its second page with the cursor it was
    /// asked from, as a connection that does not advance would.
    closed_cursor_stalls: bool,
    /// Errors this board answers beside the data of the next request carrying each operation,
    /// once — an error GitHub reports about one part of a read it otherwise answered.
    erring_beside: Vec<(&'static str, String)>,
    /// How the next read by issue reference bends its first slot's answer, once.
    reference_fault: Option<ReferenceFault>,
    /// The variables each of those documents was sent with, at the same index.
    variables: Vec<Value>,
    /// The Status field's whole option set, replaced exactly as GitHub replaces it.
    status_options: Vec<Value>,
    /// The `Priority` field's whole option set, or `None` for a board without the field.
    priority_options: Option<Vec<Value>>,
    /// Whether the next whole-list update of the `Priority` field deliberately changes an
    /// item's value, as `drift_after_status_update` does for `Status`.
    drift_after_priority_update: bool,
    /// Whether a `Priority` value write is answered as landed and not kept.
    drops_priority_writes: bool,
    priority_response: Option<Value>,
    /// Whether the board goes out of the token's sight after the next option-list update.
    hides_after_update: bool,
    /// Whether the next `Priority` option-list update re-mints a pre-existing option's id.
    remints_after_priority_update: bool,
    /// Whether the next `Priority` option-list update drops the option it was asked to add.
    omits_added_priority_option: bool,
    /// Fields of a person's own beside the ones this product sets up, each with one item's
    /// value of it.
    persons_fields: Vec<Value>,
    /// Text fields beyond the origin's — what a source's `metadata_fields` projects onto —
    /// each as the board's field list answers it. An item's value of one is under its
    /// `texts`, by field id.
    text_fields: Vec<Value>,
    /// Whether the next text-field create is answered as landed without the board keeping it.
    omits_created_text_field: bool,
    /// Whether the `Priority` field answers its `options` as something other than a list.
    malformed_priority_options: bool,
    /// A field whose value the board snapshot answers twice on `T-1`, the second time naming
    /// another of that field's options.
    repeated_snapshot_value: Option<&'static str>,
    /// Whether the next whole-list update deliberately changes an assignment.
    drift_after_status_update: bool,
    remint_after_status_update: bool,
    omit_added_status_option: bool,
    /// Whether an issue created here is filed with no `Status`, as on a board left with
    /// GitHub's default workflows, none of which sets one on an item added to it; `false`
    /// files it at `Todo`, as a board with that workflow turned on does.
    files_without_status: bool,
    /// Whether a project's members listing answers without what blocks each member, as
    /// ai-orchestrator's GitHub double answers it, though the read selects it.
    members_without_blocked_by: bool,
    /// How many more reads of created issues' board items answer that GitHub has not filed
    /// them yet, as it may not have in the moments after a create naming the board.
    unfiled_board_item_reads: usize,
    /// When set, an issue a create files on this board with `projectV2Ids` is placed on it only
    /// once its delay has passed since the create was answered, as GitHub places one: until
    /// then it is an issue on no board, and `addProjectV2ItemById` files it at once.
    placement: Option<BoardPlacement>,
    /// Each member a page of a project's members beside a batched origin lookup answers short of
    /// what the board holds — its id, the field and the value that page gives it — while every
    /// other read of it answers the board's own record.
    members_page_misanswers: Vec<(String, MembersPageField, Value)>,
    /// The repository, `owner/name`, this board's issues are created in and its source is
    /// configured with.
    repository: String,
    /// Whether the next read of a project's sub-issues beside a copy's batch answers them with
    /// no list at all.
    omits_sub_issue_list: bool,
    status_snapshot_page_size: Option<usize>,
    status_field_present: bool,
    status_board_accessible: bool,
    /// The board's **own** title, description and readme — a person's, not this
    /// product's. `updateProjectV2` is answered here rather than refused so that a
    /// journey asserting these are byte-identical after a copy fails when something
    /// writes them, instead of passing because nothing could have.
    own: Value,
    /// Whether the board's Project answers `public: true`. Private unless a journey says so.
    project_public: bool,
    /// How many reads of the Project's `public` field answer before a person makes it public,
    /// so a change in the middle of one command can be driven; `None` for never.
    project_public_after_reads: Option<usize>,
    /// The reads of the Project's `public` field answered so far.
    project_visibility_reads: usize,
    /// What every read of the Project's `public` field is answered with as `projectV2`, in
    /// place of the board's own answer; `None` for the board's own.
    project_visibility_answer: Option<Value>,
    /// The GraphQL error every read of the Project's `public` field is refused with, in place
    /// of an answer; `None` to answer it.
    project_visibility_error: Option<String>,
    /// Whether the token lacks `read:project`, so a read of the Project's `public` field is
    /// refused the way GitHub refuses it: `INSUFFICIENT_SCOPES`, naming the scope.
    project_scope_withheld: bool,
    /// How `GET /repos/{owner}/{name}` answers for each repository by `owner/name` —
    /// [`RepositoryAnswer::Private`] for one no journey named.
    repository_visibility: std::collections::BTreeMap<String, RepositoryAnswer>,
    /// Changes a journey has this board make once it has answered a request matching each,
    /// before it reads the next — a person editing an item while a command is reading it.
    hooks: Vec<BoardHook>,
    /// The most rows one page of an issue search answers with, fewer than it was asked for —
    /// as GitHub may — or `None` to answer as many as asked.
    search_page_rows: Option<usize>,
    /// Whether a mutation selecting the whole issue it wrote is answered with only what it
    /// wrote and the parts of the issue's change token — the response a store's cache must
    /// not take for the whole issue.
    partial_writes: bool,
    /// Operations whose next request is applied and then answered with nothing at all: the
    /// connection is closed with no response, as a lost answer leaves a caller.
    dropping: Vec<&'static str>,
    /// Operations whose next request is refused as naming a node id that does not resolve.
    refusing_stale: Vec<&'static str>,
    /// Members of the issue a whole-issue document's answer leaves out.
    omitting: Vec<&'static str>,
    /// Connections of the issue, by pointer, a whole-issue document's answer says go on.
    truncating: Vec<&'static str>,
}

/// Whether one request — its document and its variables — is one a [`BoardHook`] counts.
type RequestMatch = Box<dyn Fn(&str, &Value) -> bool + Send>;

/// A rewrite of one answer, given the request's variables and the `data` it would have carried.
type AnswerBend = Box<dyn FnMut(&Value, &mut Value) + Send>;

/// One change a journey has the board make part way through a command: after the
/// `skip + 1`th request `matches` holds of, and before the board reads another.
struct BoardHook {
    /// Whether one request is one this hook counts.
    matches: RequestMatch,
    /// How many matching requests to answer before making the change.
    skip: usize,
    /// The change, made through the same handle a journey makes one with.
    change: Box<dyn FnOnce(&GitHubBoardFields) + Send>,
}

/// The moment every item this board starts with was last changed, before any tick.
const GITHUB_BOARD_EPOCH: &str = "2026-09-01T00:00:00Z";

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] How the loopback board answers its one REST read, which the board's own request loop serves from its private state; it lives beside `GitHubBoard` for the reason that type records, and a GitHub suite could not add an answer from outside it.
/// How the loopback GitHub answers `GET /repos/{owner}/{name}`, the read a write to a board
/// declared private sends for the repository an issue lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryAnswer {
    /// `"visibility": "public"`, beside the `private: false` GitHub sends with it.
    Public,
    /// `"visibility": "private"`, beside `private: true`.
    Private,
    /// `"visibility": "internal"` — readable across an enterprise, so never public — beside
    /// `private: true`.
    Internal,
    /// The older answer: the `private` flag alone, with no `visibility` field.
    FlagOnly {
        /// The flag's value.
        private: bool,
    },
    /// A `visibility` field present but not a string.
    MalformedVisibility,
    /// A `visibility` string naming none of `public`, `private` and `internal`.
    UnnamedVisibility,
    /// A `200` whose body is not JSON.
    NotJson,
    /// A repository the token cannot see: `404`.
    Unseen,
}

impl RepositoryAnswer {
    /// The status line and body this answer is served as, for the repository `slug`.
    fn served(self, slug: &str) -> (&'static str, String) {
        let named = |visibility: &str| {
            json!({"id": 123_456, "full_name": slug,
                   "private": visibility != "public", "visibility": visibility})
            .to_string()
        };
        match self {
            Self::Public => ("200 OK", named("public")),
            Self::Private => ("200 OK", named("private")),
            Self::Internal => ("200 OK", named("internal")),
            Self::FlagOnly { private } => (
                "200 OK",
                json!({"id": 123_456, "full_name": slug, "private": private}).to_string(),
            ),
            Self::MalformedVisibility => (
                "200 OK",
                json!({"id": 123_456, "full_name": slug, "private": true, "visibility": 7})
                    .to_string(),
            ),
            Self::UnnamedVisibility => ("200 OK", named("restricted")),
            Self::NotJson => ("200 OK", "<html>not a repository</html>".to_owned()),
            Self::Unseen => ("404 Not Found", json!({"message": "Not Found"}).to_string()),
        }
    }
}
// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

/// A read-only handle on one fixture board's own fields.
#[derive(Clone)]
pub struct GitHubBoardFields {
    endpoint: String,
    board: Arc<Mutex<GitHubBoard>>,
    connections: Arc<AtomicUsize>,
    /// The one connection [`Self::graphql`] and [`Self::own`] keep to the board, as the
    /// product's own client keeps one, rather than a connection closed after every request.
    client: Arc<Mutex<loopback::Upstream>>,
}

impl GitHubBoardFields {
    /// How many TCP connections this board has accepted — what a journey holding the binary's
    /// requests to the connections its client keeps alive counts.
    #[must_use]
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// Every GraphQL document this board has received, in order.
    ///
    /// What a journey needs to say that a read never *asked* for something, which is a
    /// claim its answer cannot carry: a read scoped to one project and a read of the whole
    /// board can agree about the tasks in that project and disagree entirely about what
    /// they cost.
    #[must_use]
    pub fn documents(&self) -> Vec<String> {
        self.board.lock().unwrap().documents.clone()
    }

    /// Every GraphQL request this board has served, in order: the document and the
    /// variables it was sent with.
    ///
    /// The variables are what name the node a per-issue read reached, which is what a
    /// journey needs to say that a copy read *these* issues and no others.
    #[must_use]
    pub fn served(&self) -> Vec<(String, Value)> {
        let board = self.board.lock().unwrap();
        board
            .documents
            .iter()
            .cloned()
            .zip(board.variables.iter().cloned())
            .collect()
    }

    /// Every request this server answered for the source whose credential is `token`, as
    /// [`Self::served`] reports them — one board's requests out of the one log two boards
    /// configured against this server share.
    #[must_use]
    pub fn served_to(&self, token: &str) -> Vec<(String, Value)> {
        let board = self.board.lock().unwrap();
        board
            .documents
            .iter()
            .zip(&board.variables)
            .zip(&board.tokens)
            .filter(|(_, sent)| *sent == token)
            .map(|((document, variables), _)| (document.clone(), variables.clone()))
            .collect()
    }

    /// Every request this server answered, as [`Self::served`] reports it, with the moment it
    /// arrived — what a journey holding a source to its write pacing reads the gaps from.
    #[must_use]
    pub fn served_when(&self) -> Vec<(String, Value, std::time::Instant)> {
        let board = self.board.lock().unwrap();
        board
            .documents
            .iter()
            .zip(&board.variables)
            .zip(&board.arrivals)
            .map(|((document, variables), at)| (document.clone(), variables.clone(), *at))
            .collect()
    }

    /// A source configuration for the **second** board this server holds, project
    /// [`GITHUB_SECOND_BOARD`], over the very issues the first board's items are — so an
    /// issue's state is one whichever board it is read through, and each board holds cards of
    /// its own. Its requests carry [`GITHUB_SECOND_BOARD_TOKEN`].
    #[must_use]
    pub fn second_board(&self) -> Value {
        let mut block = github_projects_block_at(&self.endpoint);
        block["project_number"] = json!(GITHUB_SECOND_BOARD);
        block["token_env"] = json!("GITHUB_PROJECTS_FIXTURE_TOKEN_B");
        block
    }

    /// Put one issue on the second board too, its card at `status` — or at no option.
    pub fn put_on_second_board(&self, id: &str, status: Option<&str>) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        let card = json!({"number":GITHUB_SECOND_BOARD,
                          "item":format!("ITEM{GITHUB_SECOND_BOARD}-{id}"),
                          "status":status,"itemUpdatedAt":GITHUB_BOARD_EPOCH});
        match item["boards"].as_array_mut() {
            Some(cards) => cards.push(card),
            None => item["boards"] = json!([card]),
        }
    }

    /// Put one issue on each board of `numbers` too, its cards at no option, ahead of every card
    /// it already holds beside the first board's — so a card a journey put it on before is that
    /// many memberships further down its list.
    pub fn put_on_other_boards(&self, id: &str, numbers: &[u64]) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        let mut cards: Vec<Value> = numbers
            .iter()
            .map(|number| {
                json!({"number":number,"item":format!("ITEM{number}-{id}"),"status":null,
                       "itemUpdatedAt":GITHUB_BOARD_EPOCH})
            })
            .collect();
        cards.extend(item["boards"].as_array().cloned().unwrap_or_default());
        item["boards"] = json!(cards);
    }

    /// Take one issue's card off this server's first board, leaving the issue itself — open or
    /// closed, in its repository — and any card it has on the second board.
    pub fn take_off_the_board(&self, id: &str) {
        let mut board = self.board.lock().unwrap();
        board.find(&json!(id))["off_board"] = json!(true);
    }

    /// The `Status` option one issue's card on the second board sits at, or `None`.
    #[must_use]
    pub fn second_board_status(&self, id: &str) -> Option<String> {
        let board = self.board.lock().unwrap();
        board
            .items
            .iter()
            .find(|item| item["id"] == json!(id))
            .and_then(|item| item["boards"].as_array())
            .and_then(|cards| {
                cards
                    .iter()
                    .find(|card| card["number"] == json!(GITHUB_SECOND_BOARD))
            })
            .and_then(|card| card["status"].as_str().map(str::to_owned))
    }

    /// Whether this server holds the issue `id` at all — on a board, or created and filed on
    /// none — as its repository would.
    #[must_use]
    pub fn holds_issue(&self, id: &str) -> bool {
        let board = self.board.lock().unwrap();
        board
            .items
            .iter()
            .chain(&board.pending)
            .any(|item| item["id"] == json!(id))
    }

    /// Bend the first slot of the next read by issue reference as `fault` says, once.
    pub fn bend_the_next_reference_read(&self, fault: ReferenceFault) {
        self.board.lock().unwrap().reference_fault = Some(fault);
    }

    /// Have the board's closed-items read answer its second page with the very cursor it was
    /// asked from, saying there is more — a connection that does not advance.
    pub fn stall_the_closed_items_cursor(&self) {
        self.board.lock().unwrap().closed_cursor_stalls = true;
    }

    /// Answer the next request carrying `operation` with `message` as an error beside its data,
    /// once.
    pub fn error_beside_once(&self, operation: &'static str, message: &str) {
        self.board
            .lock()
            .unwrap()
            .erring_beside
            .push((operation, message.to_owned()));
    }

    /// Have the board's `is:closed` item filter let every open item through as well, as a
    /// filter GitHub did not honour would.
    pub fn leak_open_items_through_the_closed_filter(&self) {
        self.board.lock().unwrap().closed_filter_leaks = true;
    }

    // llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] Controls of the loopback board's visibility answers, which its request loop reads from its private state: like `refuse_once` below, they can only be methods of this handle, and a suite of one plugin could not add them from outside.
    /// Make the board's Project public, or private again.
    pub fn set_project_public(&self, public: bool) {
        self.board.lock().unwrap().project_public = public;
    }

    /// Answer the next `reads` reads of the Project's `public` field as they stand, and every
    /// read after them as public — a person making the board public part way through a command.
    pub fn make_public_after_visibility_reads(&self, reads: usize) {
        self.board.lock().unwrap().project_public_after_reads = Some(reads);
    }

    /// Withhold `read:project` from the token, so the Project's `public` field is refused.
    pub fn withhold_project_scope(&self) {
        self.board.lock().unwrap().project_scope_withheld = true;
    }

    /// Answer `GET /repos/{owner}/{name}` for `owner/name` as `answer`.
    pub fn set_repository_visibility(&self, slug: &str, answer: RepositoryAnswer) {
        self.board
            .lock()
            .unwrap()
            .repository_visibility
            .insert(slug.to_owned(), answer);
    }

    /// Answer every read of the Project's `public` field with `project` as `projectV2`, in
    /// place of the board's own answer — a response GitHub's schema does not describe.
    pub fn malform_project_visibility(&self, project: Value) {
        self.board.lock().unwrap().project_visibility_answer = Some(project);
    }

    /// Refuse every read of the Project's `public` field with the GraphQL error `message`.
    pub fn refuse_project_visibility(&self, message: &str) {
        self.board.lock().unwrap().project_visibility_error = Some(message.to_owned());
    }
    // llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

    /// Refuse the next request carrying `operation`, once, from here on.
    pub fn refuse_once(&self, operation: &'static str) {
        self.board.lock().unwrap().refusing.push(operation);
    }

    /// Answer the next `answered` requests carrying `operation`, then refuse the one after
    /// them, once — the second page of a read, say, after its first landed.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: what it refuses is read by the board's own request loop and held in its private state, so it can only be a method of this handle, beside `refuse_once`, and a suite of one plugin could not add it from outside.
    pub fn refuse_after(&self, operation: &'static str, answered: usize) {
        self.board
            .lock()
            .unwrap()
            .refusing_after
            .push((operation, answered));
    }

    /// Answer the next `answered` requests naming the item `id`, then hold `replacement` in its
    /// place — or nothing at all — so every later read finds it as a person changed or deleted it
    /// part way through a command.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: the items it replaces are the board's private state, read by its own request loop, so it can only be a method of this handle, beside `refuse_after`, and a suite could not add it from outside.
    pub fn replace_after(&self, id: &str, answered: usize, replacement: Option<Value>) {
        self.board
            .lock()
            .unwrap()
            .replacing
            .push((id.to_owned(), answered, replacement));
    }

    /// Refuse the next request `429 Too Many Requests`, naming a reset `retry_after` seconds
    /// ahead, as GitHub's primary limit does; every request after it is answered as before.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own answer to its next request, which only its server loop here answers: that state is private to the loopback, so a suite can set it only through this handle, as `block`, `edit_body` and `refuse_once` set the rest; the GitHub Projects and Linear suites are what drive it.
    pub fn rate_limit_next(&self, retry_after: u64) {
        self.board.lock().unwrap().rate_limit_next = Some(retry_after);
    }

    /// Record that the issue `id` is blocked by each of `blockers`, beside whatever already
    /// blocks it.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own `blockedBy` graph, which every read of it answers from: that graph is the board's private state, so a suite naming who blocks whom can only do it through this handle, as `edit_body` and `move_card` change what the board holds.
    pub fn block(&self, id: &str, blockers: &[String]) {
        let mut board = self.board.lock().unwrap();
        match board.blocked_by.iter_mut().find(|(near, _)| near == id) {
            Some((_, held)) => held.extend(blockers.iter().cloned()),
            None => board.blocked_by.push((id.to_owned(), blockers.to_vec())),
        }
        for blocker in blockers {
            board.record_relationship_event(
                (id, blocker),
                TimelineEvent::dependency(RelationshipChange::Added),
            );
        }
    }

    /// File the issue `id` under the project `parent`, or under none, as a person moving it
    /// between projects on GitHub would: a sub-issue removed from its old parent and added to
    /// the new one, each a relationship event on both of the issues it relates.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own project memberships, which every read of it answers from: that state is the board's private state, so a suite moving a task between projects can only do it through this handle, as `block` changes who blocks whom.
    pub fn file_under(&self, id: &str, parent: Option<&str>) {
        let mut board = self.board.lock().unwrap();
        let old = board.find(&json!(id))["parent"].as_str().map(str::to_owned);
        if let Some(old) = &old {
            board.record_relationship_event(
                (id, old),
                TimelineEvent::membership(RelationshipChange::Removed),
            );
        }
        if let Some(parent) = parent {
            board.record_relationship_event(
                (id, parent),
                TimelineEvent::membership(RelationshipChange::Added),
            );
        }
        board.find(&json!(id))["parent"] = parent.map_or(Value::Null, |parent| json!(parent));
    }

    /// Move the issue `id` to the repository `repository`, as a person transferring it on
    /// GitHub would — the issue keeps its node id and its board item, and the one thing this
    /// board says moved is its `repository`.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own record of each issue's repository, which every read of it answers from: that state is the board's private state, so a suite transferring an issue can only do it through this handle, beside `file_under`.
    pub fn transfer(&self, id: &str, repository: &str) {
        self.board.lock().unwrap().find(&json!(id))["repo"] = json!(repository);
    }

    /// Take `blocker` off what blocks the issue `id`, as a person removing the relationship on
    /// GitHub would.
    pub fn unblock(&self, id: &str, blocker: &str) {
        let mut board = self.board.lock().unwrap();
        if let Some((_, held)) = board.blocked_by.iter_mut().find(|(near, _)| near == id) {
            held.retain(|held| held != blocker);
        }
        board.record_relationship_event(
            (id, blocker),
            TimelineEvent::dependency(RelationshipChange::Removed),
        );
    }

    /// Perform the batched write aliased `alias` in the next document carrying it, and answer
    /// it as `answer` says, once.
    pub fn bend_batch_alias(&self, alias: &'static str, answer: AliasAnswer) {
        self.board.lock().unwrap().bent_alias = Some((alias, answer));
    }

    /// Answer every batched origin lookup with the repository's node id blank.
    pub fn blank_repository_id(&self) {
        self.board.lock().unwrap().blank_repository_id = true;
    }

    /// Fail the field write aliased `alias` in the next batched field write, once, after
    /// every alias before it in that document has landed.
    pub fn fail_field_alias(&self, alias: &'static str) {
        self.board.lock().unwrap().failing_alias = Some(alias);
    }

    /// Fail the next batched field write's alias that writes the board field `field` — its id,
    /// such as `FIELD-status` — once, after every alias before it landed and before any after.
    pub fn fail_field_write(&self, field: &str) {
        self.board.lock().unwrap().failing_field = Some(field.to_owned());
    }

    /// Take one `Status` option off this board, as a person deleting a column would.
    pub fn without_option(&self, name: &str) {
        self.board
            .lock()
            .unwrap()
            .status_options
            .retain(|option| option["name"] != json!(name));
    }

    /// Answer every mutation selecting the whole issue it wrote with only what it wrote and
    /// the parts of the issue's change token — its facets' sources included — or, with
    /// `false`, whole again.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: what it changes is the board's private state, read by its own request loop, so it can only be a method of this handle, beside `refuse_once` and `rename_option`, and a suite of one plugin could not add it from outside.
    pub fn answer_writes_partially(&self, partially: bool) {
        self.board.lock().unwrap().partial_writes = partially;
    }

    /// Answer every whole-issue document with `members` of the issue left out, as a response
    /// that did not select them would.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: what it changes is the board's private state, read by its own request loop, so it can only be a method of this handle, beside `refuse_once` and `rename_option`, and a suite of one plugin could not add it from outside.
    pub fn omit_from_writes(&self, members: &[&'static str]) {
        self.board.lock().unwrap().omitting = members.to_vec();
    }

    /// Answer every whole-issue document with each connection `connections` points at —
    /// `/labels`, `/comments`, `/projectItems/nodes/0/fieldValues` — saying it has another
    /// page, as a response holding only the first page of a longer one would.
    pub fn truncate_in_writes(&self, connections: &[&'static str]) {
        self.board.lock().unwrap().truncating = connections.to_vec();
    }

    /// Refuse the next request carrying `operation` as GitHub refuses one naming a node id that
    /// no longer resolves — an id a cache kept going stale — once, writing nothing.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: what it changes is the board's private state, read by its own request loop, so it can only be a method of this handle, beside `refuse_once` and `rename_option`, and a suite of one plugin could not add it from outside.
    pub fn refuse_as_stale_once(&self, operation: &'static str) {
        self.board.lock().unwrap().refusing_stale.push(operation);
    }

    /// Apply the next request carrying `operation`, and close its connection without an
    /// answer — once.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: what it changes is the board's private state, read by its own request loop, so it can only be a method of this handle, beside `refuse_once` and `rename_option`, and a suite of one plugin could not add it from outside.
    pub fn drop_response_once(&self, operation: &'static str) {
        self.board.lock().unwrap().dropping.push(operation);
    }

    /// Give the `Status` option `name` a new id, as GitHub does to an option a whole-list
    /// update renames: every item in it stays, and the old id no longer resolves.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: what it changes is the board's private state, read by its own request loop, so it can only be a method of this handle, beside `refuse_once` and `rename_option`, and a suite of one plugin could not add it from outside.
    pub fn remint_status_option(&self, name: &str) {
        let mut board = self.board.lock().unwrap();
        let option = board
            .status_options
            .iter_mut()
            .find(|option| option["name"] == json!(name))
            .expect("the option to re-mint exists");
        option["id"] = json!(format!("{}-reminted", option["id"].as_str().unwrap()));
    }

    /// Give the `Priority` option `name` a new id, as [`Self::remint_status_option`] does.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: what it changes is the board's private state, read by its own request loop, so it can only be a method of this handle, beside `refuse_once` and `rename_option`, and a suite of one plugin could not add it from outside.
    pub fn remint_priority_option(&self, name: &str) {
        let mut board = self.board.lock().unwrap();
        let option = board
            .priority_options
            .as_mut()
            .and_then(|options| {
                options
                    .iter_mut()
                    .find(|option| option["name"] == json!(name))
            })
            .expect("the option to re-mint exists");
        option["id"] = json!(format!("{}-reminted", option["id"].as_str().unwrap()));
    }

    /// Change an option's display casing without changing its identity.
    pub fn rename_option(&self, from: &str, to: &str) {
        let mut board = self.board.lock().unwrap();
        let option = board
            .status_options
            .iter_mut()
            .find(|option| option["name"] == json!(from))
            .expect("the option to rename exists");
        option["name"] = json!(to);
    }

    /// Rename the `Status` option `from` to `to` as a person renaming it on the board would: the
    /// option keeps its id, every item sitting in it now reads under the new name, and neither
    /// an issue's `updatedAt` nor a board item's moves.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own field options and the values naming them, which every read of it answers from: that state is the board's private state, so a suite renaming an option as a person would can only do it through this handle, beside `rename_option`.
    pub fn rename_status_option(&self, from: &str, to: &str) {
        self.rename_option(from, to);
        let mut board = self.board.lock().unwrap();
        for item in &mut board.items {
            if item["status"] == json!(from) {
                item["status"] = json!(to);
            }
        }
    }

    /// Make the fixture report assignment drift after the next Status option update.
    pub fn drift_after_status_update(&self) {
        self.board.lock().unwrap().drift_after_status_update = true;
    }

    /// Make the fixture change one pre-existing option id after the next update.
    pub fn remint_after_status_update(&self) {
        self.board.lock().unwrap().remint_after_status_update = true;
    }

    /// Make the fixture omit the newly requested option from the post-write state.
    pub fn omit_added_status_option(&self) {
        self.board.lock().unwrap().omit_added_status_option = true;
    }

    /// File every issue created from here on with no `Status`, as a board left with GitHub's
    /// default workflows does — those set `Done` on a closed or merged item and nothing on an
    /// added one — or, with `false`, at `Todo`, as a board with that workflow turned on does.
    pub fn file_new_items_without_status(&self, without: bool) {
        self.board.lock().unwrap().files_without_status = without;
    }

    /// Answer every listing of a project's members without what blocks each member, as
    /// ai-orchestrator's GitHub double answers it — or, with `false`, with it, as GitHub does.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's members listing, which its request loop reads from its private state: like `file_new_items_without_status` above, it can only be a method of this handle, and the board lives in this shared harness for the reason `GitHubBoard` records.
    pub fn list_members_without_blocked_by(&self, without: bool) {
        self.board.lock().unwrap().members_without_blocked_by = without;
    }

    /// Answer the next `reads` reads of created issues' board items as GitHub answers before it
    /// has filed them: each issue on no board yet.
    pub fn file_created_items_after(&self, reads: usize) {
        self.board.lock().unwrap().unfiled_board_item_reads = reads;
    }

    /// Answer the member `id` in every page of a project's members
    /// [`ORIGIN_LOOKUPS_WITH_MEMBERS`](onetaskgraph_github_projects::graphql::ORIGIN_LOOKUPS_WITH_MEMBERS)
    /// carries with its `field` as `value`, while every full read of it answers what the board
    /// holds: a trimmed page that says less about a member, or something malformed, than a read
    /// of the same member does.
    pub fn misanswer_in_members_page(&self, id: &str, field: MembersPageField, value: Value) {
        self.board
            .lock()
            .unwrap()
            .members_page_misanswers
            .push((id.to_owned(), field, value));
    }

    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own placement state, which every read of it answers from, beside `file_created_items_after`: the board is this shared harness's, so a suite can only set it through this handle.
    /// Place every issue a create files on the board with `projectV2Ids` only once a delay of
    /// `delays` — each create's in turn, cycled — has passed on `now` since the create was
    /// answered ([`Self::answered_at`]), as GitHub places one; until then the issue is on no
    /// board, as a read of its board items answers it.
    pub fn place_filed_issues_after(
        &self,
        delays: Vec<Duration>,
        now: impl Fn() -> Duration + Send + Sync + 'static,
    ) {
        assert!(!delays.is_empty(), "a placement delay to hand out");
        self.board.lock().unwrap().placement = Some(BoardPlacement {
            now: Arc::new(now),
            delays,
            next: 0,
            waiting: Vec::new(),
        });
    }

    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] As `place_filed_issues_after` above: the time a proxy in front of this board released a create's answer, which the board's placement keeps time by.
    /// Every create this board has answered so far was answered at `now`: each issue it is
    /// waiting to place is placed once its delay has passed since then.
    pub fn answered_at(&self, now: Duration) {
        if let Some(placement) = &mut self.board.lock().unwrap().placement {
            for (_, delay, due) in &mut placement.waiting {
                due.get_or_insert(now + *delay);
            }
        }
    }

    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own record of the repository its issues live in, which every read of it answers from, beside `transfer`: the board is this shared harness's.
    /// Hold every issue of this board in `repository`, `owner/name`, and create every new one
    /// there: what a journey configuring its source with that repository sets, so no answer
    /// names another.
    pub fn use_repository(&self, repository: &str) {
        let mut guard = self.board.lock().unwrap();
        let board = &mut *guard;
        let before = std::mem::replace(&mut board.repository, repository.to_owned());
        for item in board.items.iter_mut().chain(board.pending.iter_mut()) {
            if item["repo"] == json!(before) {
                item["repo"] = json!(repository);
            }
        }
    }

    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] One request to this shared board, as the product sends one, through the harness's own `graphql_over_http`: what a journey replaying a recorded trace drives the board with.
    /// Send this board one GraphQL request, as the product sends one, and answer its `data`:
    /// what a journey replaying a recorded request sequence drives the board with.
    pub fn graphql(&self, query: &str, variables: &Value) -> Value {
        self.graphql_over_http(query, variables)
    }

    /// Answer the next read of a project's sub-issues beside a copy's batch with no list of
    /// them, once.
    pub fn omit_sub_issue_list_once(&self) {
        self.board.lock().unwrap().omits_sub_issue_list = true;
    }

    /// Force Status snapshots to span pages even though the client requests 100 items.
    pub fn paginate_status_snapshots(&self) {
        self.board.lock().unwrap().status_snapshot_page_size = Some(2);
    }

    /// Make the fixture change one item's `Priority` value after the next update of that
    /// field's option list.
    pub fn drift_after_priority_update(&self) {
        self.board.lock().unwrap().drift_after_priority_update = true;
    }

    /// Give the board a text field called `name`, which a source's `metadata_fields` may
    /// project a value onto, holding nothing on any item.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board, which lives here for the reason `GitHubBoard` records: its field list and every item's values are the board's private state, read by its own request loop, so a handle on them can only be a method beside `with_persons_field` and `refuse_once`, and a suite of one plugin could not add it from outside.
    pub fn with_text_field(&self, name: &str) {
        self.board.lock().unwrap().text_fields.push(
            json!({"__typename":"ProjectV2Field","id":format!("FIELD-text-{name}"),
                   "name":name,"dataType":"TEXT"}),
        );
    }

    /// What the text field called `name` holds on the board item whose content is `id`, or
    /// `None` when it holds nothing — or the board has no such field.
    // llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] A read of the loopback board's private item state, beside `status_options` and `priority_options`, for the reason `with_text_field` above states.
    #[must_use]
    pub fn text_of(&self, id: &str, name: &str) -> Option<String> {
        let board = self.board.lock().unwrap();
        let field = board
            .text_fields
            .iter()
            .find(|field| field["name"] == name)?;
        let item = board.items.iter().find(|item| item["id"] == id)?;
        item["texts"][field["id"].as_str()?]
            .as_str()
            .map(str::to_owned)
    }
    // llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

    /// The board's fields as its own field list answers them.
    // llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] A read of the loopback board's private field list, for the reason `with_text_field` above states.
    #[must_use]
    pub fn field_list(&self) -> Vec<Value> {
        self.board.lock().unwrap().fields()["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    // llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

    /// Answer the next text-field create as landed without keeping the field, as a response
    /// the setup's verification read has to catch would.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's private field list, for the reason `with_text_field` above states.
    pub fn omit_created_text_field(&self) {
        self.board.lock().unwrap().omits_created_text_field = true;
    }

    /// Give the board a single-select field of a person's own, `field`, holding the first of
    /// its options on `T-1`.
    pub fn with_persons_field(&self, field: Value) {
        self.board.lock().unwrap().persons_fields.push(field);
    }

    /// Make the board answer its `Priority` field's `options` as a string rather than a list,
    /// as a response this product cannot read would — in the board's field list and in every
    /// item value that carries the field's definition alike.
    pub fn malform_priority_options(&self) {
        self.board.lock().unwrap().malformed_priority_options = true;
    }

    /// Make the board snapshot answer `field`'s value on `T-1` twice, the second time naming
    /// that field's last option, as a response that cannot say which one the item holds.
    pub fn repeat_snapshot_value(&self, field: &'static str) {
        self.board.lock().unwrap().repeated_snapshot_value = Some(field);
    }

    /// Make the next `Priority` option-list update change one pre-existing option's id.
    pub fn remint_after_priority_update(&self) {
        self.board.lock().unwrap().remints_after_priority_update = true;
    }

    /// Make the next `Priority` option-list update leave out the option it was asked to add.
    pub fn omit_added_priority_option(&self) {
        self.board.lock().unwrap().omits_added_priority_option = true;
    }

    /// Make the board go out of the token's sight once the next `Priority` option-list update
    /// has landed, so the read that would verify it fails.
    pub fn hide_after_update(&self) {
        self.board.lock().unwrap().hides_after_update = true;
    }

    /// Give one `Priority` option a colour GitHub's vocabulary has no member for, as a board
    /// answering with something this product cannot read would.
    pub fn recolor_priority_option(&self, name: &str, color: &str) {
        let mut board = self.board.lock().unwrap();
        let option = board
            .priority_options
            .as_mut()
            .expect("the board has a Priority field")
            .iter_mut()
            .find(|option| option["name"] == json!(name))
            .expect("the option to recolour exists");
        option["color"] = json!(color);
    }

    /// Make every `Priority` value write answer as landed without the board keeping it.
    pub fn drop_priority_writes(&self) {
        self.board.lock().unwrap().drops_priority_writes = true;
    }

    /// Remove the board's `Priority` field entirely, and every item's value of it.
    pub fn without_priority_field(&self) {
        let mut board = self.board.lock().unwrap();
        board.priority_options = None;
        for item in &mut board.items {
            item["priority"] = Value::Null;
        }
    }

    /// Take one `Priority` option off this board, and every item's value of it.
    pub fn without_priority_option(&self, name: &str) {
        let mut board = self.board.lock().unwrap();
        if let Some(options) = &mut board.priority_options {
            options.retain(|option| option["name"] != json!(name));
        }
        for item in &mut board.items {
            if item["priority"] == json!(name) {
                item["priority"] = Value::Null;
            }
        }
    }

    /// Give the `Priority` field one more option, as a person adding a column would.
    pub fn with_priority_option(&self, id: &str, name: &str) {
        let mut board = self.board.lock().unwrap();
        board
            .priority_options
            .as_mut()
            .expect("the board has a Priority field")
            .push(json!({"id":id,"name":name,"color":"PINK","description":"a person's"}));
    }

    /// Replace the next priority mutation's item payload with a malformed host response.
    pub fn malform_priority_response(&self, value: Value) {
        self.board.lock().unwrap().priority_response = Some(value);
    }

    /// Put one item in the `Priority` option named `name`, or in none.
    pub fn assign_priority(&self, id: &str, name: Option<&str>) {
        let mut board = self.board.lock().unwrap();
        let held = board
            .items
            .iter_mut()
            .find(|item| item["id"] == json!(id))
            .expect("the item to assign exists");
        held["priority"] = name.map_or(Value::Null, |name| json!(name));
        let item = held["item"].clone();
        board.touch_item(&item);
    }

    /// Stamp every change from here on with one second, until [`Self::release_clock`]: two
    /// changes a person makes inside one second on GitHub, which GitHub stamps alike.
    pub fn hold_clock(&self) {
        let mut board = self.board.lock().unwrap();
        board.held_second = Some(board.comment_ticks + 1);
    }

    /// Let each change take the second after the last again.
    pub fn release_clock(&self) {
        self.board.lock().unwrap().held_second = None;
    }

    /// Answer every read with a `Date` in the very second of the board's latest change — a
    /// board read while a person is still editing it — or, with `false`, an hour after it.
    pub fn read_as_it_changes(&self, at_once: bool) {
        self.board.lock().unwrap().read_lag = Some(if at_once { 0 } else { 3600 });
    }

    /// Answer every read without a `Date` header, or, with `false`, with one again.
    pub fn serve_without_date(&self, without: bool) {
        self.board.lock().unwrap().read_lag = (!without).then_some(3600);
    }

    /// Rewrite one issue's body, as a person editing it on GitHub would.
    pub fn edit_body(&self, id: &str, body: &str) {
        let mut board = self.board.lock().unwrap();
        board.find(&json!(id))["body"] = json!(body);
        board.touch_issue(&json!(id));
    }

    /// Give one issue the creation time GitHub reports for it, which no edit moves; an issue
    /// given none reports `null`.
    pub fn set_created_at(&self, id: &str, at: &str) {
        self.board.lock().unwrap().find(&json!(id))["createdAt"] = json!(at);
    }

    /// Reopen one closed issue, as a person reopening it on GitHub would.
    pub fn reopen(&self, id: &str) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        item["state"] = json!("OPEN");
        item["reason"] = Value::Null;
        board.touch_issue(&json!(id));
    }

    /// Close one issue with `reason` — GitHub's `IssueStateReason`, or `None` for an issue
    /// closed before GitHub recorded one — and leave its card where it is.
    ///
    /// Whichever way an issue closes on GitHub — a person closing it, a change request merged
    /// with a closing line, a landing's issue-closing line — what any reader of it then sees
    /// is exactly this: its state and its reason, with the board's `Status` untouched.
    pub fn close(&self, id: &str, reason: Option<&str>) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        item["state"] = json!("CLOSED");
        item["reason"] = reason.map_or(Value::Null, |reason| json!(reason));
        board.touch_issue(&json!(id));
    }

    /// One issue's `state` and the reason it was closed with, as GitHub holds them.
    #[must_use]
    pub fn state(&self, id: &str) -> (String, Option<String>) {
        let board = self.board.lock().unwrap();
        let item = board
            .items
            .iter()
            .find(|item| item["id"] == json!(id))
            .unwrap_or_else(|| panic!("the board holds {id}"));
        (
            item["state"].as_str().unwrap_or_default().to_owned(),
            item["reason"].as_str().map(str::to_owned),
        )
    }

    /// Add one `Status` option to this board, as a person adding a column would.
    pub fn with_status_option(&self, id: &str, name: &str) {
        self.board
            .lock()
            .unwrap()
            .status_options
            .push(json!({"id":id,"name":name,"color":"GRAY","description":""}));
    }

    /// Rewrite one issue's title, as a person editing it on GitHub would.
    pub fn edit_title(&self, id: &str, title: &str) {
        let mut board = self.board.lock().unwrap();
        board.find(&json!(id))["title"] = json!(title);
        board.touch_issue(&json!(id));
    }

    /// Give one issue exactly the labels named, as a person labelling it on GitHub would.
    pub fn set_labels(&self, id: &str, labels: &[&str]) {
        let mut board = self.board.lock().unwrap();
        board.find(&json!(id))["labels"] = Value::Array(
            labels
                .iter()
                .map(|label| json!([format!("LABEL-{label}"), label]))
                .collect(),
        );
        board.touch_issue(&json!(id));
    }

    /// Rename the repository label `label` — every issue carrying it now carries the new name —
    /// as a person renaming it on GitHub would: no issue's `updatedAt` moves, and no board
    /// item's.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's labels, which every read of it answers from: that state is the board's private state, so a suite renaming a label as a person would can only do it through this handle, beside `set_labels`.
    pub fn rename_label(&self, label: &str, name: &str) {
        let mut board = self.board.lock().unwrap();
        for item in &mut board.items {
            for pair in item["labels"].as_array_mut().into_iter().flatten() {
                if pair[0] == json!(label) {
                    pair[1] = json!(name);
                }
            }
        }
    }

    /// Give the repository label `label` the colour `color` on every issue carrying it, as a
    /// person recolouring it on GitHub would: no issue's `updatedAt` moves, and no board item's.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's labels, beside `rename_label`, for the same reason.
    pub fn recolour_label(&self, label: &str, color: &str) {
        let mut board = self.board.lock().unwrap();
        for item in &mut board.items {
            for pair in item["labels"].as_array_mut().into_iter().flatten() {
                if pair[0] == json!(label)
                    && let Some(pair) = pair.as_array_mut()
                {
                    pair.truncate(2);
                    pair.push(json!(color));
                }
            }
        }
    }

    /// Answer every read of `id` with its labels saying another page follows, as a read of an
    /// issue carrying more labels than one page holds would.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own answers, beside `rename_label`, for the same reason.
    pub fn report_more_labels(&self, id: &str, more: bool) {
        self.board.lock().unwrap().find(&json!(id))["more_labels"] = json!(more);
    }

    /// Delete the issue `id`, as a person deleting it on GitHub would: it leaves the board, every
    /// dependency and project membership naming it goes, and so does every relationship event
    /// naming it on another issue's timeline — which is how a far end's deletion *lowers* the
    /// relationship count of an issue that depended on it — while no other issue's `updatedAt`
    /// moves.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's issues and their timelines, which every read of it answers from: that state is the board's private state, so a suite deleting an issue as a person would can only do it through this handle, beside `block` and `file_under`.
    pub fn delete_issue(&self, id: &str) {
        let mut board = self.board.lock().unwrap();
        board.items.retain(|item| item["id"] != json!(id));
        board.blocked_by.retain(|(near, _)| near != id);
        for (_, blockers) in &mut board.blocked_by {
            blockers.retain(|blocker| blocker != id);
        }
        for item in &mut board.items {
            if item["parent"] == json!(id) {
                item["parent"] = Value::Null;
            }
        }
        let naming: Vec<(String, TimelineEvent)> = board
            .relationship_log
            .iter()
            .filter(|(end, other, _)| other == id && end != id)
            .map(|(end, _, kind)| (end.clone(), *kind))
            .collect();
        board
            .relationship_log
            .retain(|(end, other, _)| end != id && other != id);
        for (end, kind) in naming {
            if let Some(item) = board.items.iter_mut().find(|item| item["id"] == json!(end)) {
                let held = item["timeline"][kind.name()].as_u64().unwrap_or(0);
                item["timeline"][kind.name()] = json!(held.saturating_sub(1));
            }
        }
    }

    /// Set one caller key in the metadata slot at the end of one issue's body, adding the slot
    /// when it holds none, as a person editing the body on GitHub would.
    pub fn set_metadata(&self, id: &str, key: &str, value: Value) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        let body = item["body"].as_str().unwrap_or_default().to_owned();
        let open = "<!-- onetaskgraph.metadata\n";
        let rewritten = match body.rfind(open) {
            Some(at) => {
                let start = at + open.len();
                let end = start + body[start..].find("\n-->").expect("a closed slot");
                let mut slot: serde_json::Map<String, Value> =
                    serde_json::from_str(&body[start..end]).expect("the slot is JSON");
                slot.insert(key.to_owned(), value);
                format!("{}{}{}", &body[..start], Value::Object(slot), &body[end..])
            }
            None => format!("{body}\n\n{open}{}\n-->", json!({ key: value })),
        };
        item["body"] = json!(rewritten);
        board.touch_issue(&json!(id));
    }

    /// Set a person's own text field on one item's board card — a board field this product
    /// writes nothing to, and reads only as one of the values its change token's facets hold —
    /// as a person editing the card on the board would. GitHub moves the board item's
    /// `updatedAt` for it and leaves the issue's alone, and every read selecting a text value
    /// carries it.
    pub fn set_card_field(&self, id: &str, field: &str, value: &str) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        item["own_fields"][field] = json!(value);
        let card = item["item"].clone();
        board.touch_item(&card);
    }

    /// Write `value` into one item's `onetaskgraph.origin` text field, as a person editing that
    /// board field would — which moves the board item's `updatedAt` and not the issue's.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback board's own text field values, which every read of it answers from: that state is the board's private state, so a suite editing a board field as a person would can only do it through this handle, as `set_card_field` does.
    pub fn set_origin_field(&self, id: &str, value: &str) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        item["origin"] = json!(value);
        let card = item["item"].clone();
        board.touch_item(&card);
    }

    /// Add a comment to one issue, as the account the token belongs to posting it on GitHub
    /// would, answering with the comment's id.
    pub fn add_comment(&self, issue: &str, body: &str) -> String {
        let added = self.board.lock().unwrap().comment(issue, body);
        added["id"].as_str().expect("a comment id").to_owned()
    }

    /// Replace one comment's body, as its author editing it on GitHub would.
    pub fn edit_comment(&self, comment: &str, body: &str) {
        let mut board = self.board.lock().unwrap();
        let at = board.tick();
        let held = board
            .comments
            .iter_mut()
            .find(|held| held["id"] == json!(comment))
            .expect("the comment to edit");
        held["body"] = json!(body);
        held["updatedAt"] = json!(at);
        let issue = held["issue"].clone();
        board.touch_issue(&issue);
    }

    /// Delete one comment, as its author deleting it on GitHub would — which moves the
    /// issue's comment count and neither of its times.
    pub fn delete_comment(&self, comment: &str) {
        let mut board = self.board.lock().unwrap();
        assert!(
            board
                .comments
                .iter()
                .any(|held| held["id"] == json!(comment)),
            "the comment to delete: {comment}"
        );
        board.comments.retain(|held| held["id"] != json!(comment));
    }

    /// Every comment the issue `issue` holds, oldest first, as `{id, body}`.
    #[must_use]
    pub fn comments_on(&self, issue: &str) -> Vec<Value> {
        self.board
            .lock()
            .unwrap()
            .comments
            .iter()
            .filter(|held| held["issue"] == json!(issue))
            .map(|held| json!({"id": held["id"], "body": held["body"]}))
            .collect()
    }

    /// Answer every page of an issue search with at most `rows` rows, fewer than it was asked
    /// for when it asked for more, as GitHub's search may.
    pub fn answer_search_pages_of(&self, rows: usize) {
        self.board.lock().unwrap().search_page_rows = Some(rows);
    }

    /// Have this board make `change` once it has answered the `skip + 1`th request that
    /// `matches` holds of — its GraphQL document and its variables — and before it reads the
    /// next: a person editing an item part way through a command.
    pub fn after_request(
        &self,
        matches: impl Fn(&str, &Value) -> bool + Send + 'static,
        skip: usize,
        change: impl FnOnce(&GitHubBoardFields) + Send + 'static,
    ) {
        self.board.lock().unwrap().hooks.push(BoardHook {
            matches: Box::new(matches),
            skip,
            change: Box::new(change),
        });
    }

    /// Move one item's card to the `Status` option named `name`, as a person dragging it on
    /// the board would.
    pub fn move_card(&self, id: &str, name: &str) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        item["status"] = json!(name);
        let card = item["item"].clone();
        board.touch_item(&card);
    }

    /// The name of the `Status` option one item sits in, or `None`.
    #[must_use]
    pub fn status(&self, id: &str) -> Option<String> {
        self.board
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|item| item["id"] == json!(id))
            .and_then(|item| item["status"].as_str().map(str::to_owned))
    }

    /// The name of the `Priority` option one item sits in, or `None`.
    #[must_use]
    pub fn priority(&self, id: &str) -> Option<String> {
        self.board
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|item| item["id"] == json!(id))
            .and_then(|item| item["priority"].as_str().map(str::to_owned))
    }

    /// The `Priority` field's options, or `None` when the board has no such field.
    #[must_use]
    pub fn priority_options(&self) -> Option<Vec<Value>> {
        self.board.lock().unwrap().priority_options.clone()
    }

    /// The Status field's options.
    #[must_use]
    pub fn status_options(&self) -> Vec<Value> {
        self.board.lock().unwrap().status_options.clone()
    }

    /// Remove the board's Status field entirely.
    pub fn without_status_field(&self) {
        self.board.lock().unwrap().status_field_present = false;
    }

    /// Make the configured board absent from GitHub's response, as an inaccessible board is.
    pub fn without_accessible_status_board(&self) {
        self.board.lock().unwrap().status_board_accessible = false;
    }

    /// Take the origin out of one issue's metadata slot and leave it in the board's origin
    /// field alone — which is how the release before the slot mirrored it wrote every copy.
    pub fn written_before_the_origin_mirror(&self, id: &str) {
        let mut board = self.board.lock().unwrap();
        let item = board.find(&json!(id));
        let body = item["body"].as_str().expect("a body").to_owned();
        let open = "<!-- onetaskgraph.metadata\n";
        let start = body.rfind(open).expect("a metadata slot") + open.len();
        let end = start + body[start..].find("\n-->").expect("a closed slot");
        let mut slot: serde_json::Map<String, Value> =
            serde_json::from_str(&body[start..end]).expect("the slot is JSON");
        assert!(
            slot.remove("onetaskgraph.origin").is_some(),
            "this release mirrors the origin into the slot: {body}"
        );
        let rewritten = format!("{}{}{}", &body[..start], Value::Object(slot), &body[end..]);
        item["body"] = json!(rewritten);
        board.touch_issue(&json!(id));
    }

    /// The origin field this board holds for one issue.
    #[must_use]
    pub fn origin(&self, id: &str) -> Value {
        self.board
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|item| item["id"] == json!(id))
            .map_or(Value::Null, |item| item["origin"].clone())
    }

    /// The body this board holds for one issue, byte for byte.
    #[must_use]
    pub fn body(&self, id: &str) -> Value {
        self.board
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|item| item["id"] == json!(id))
            .map_or(Value::Null, |item| item["body"].clone())
    }

    /// One item exactly as this board holds it — its title, body, state, parent, `Status`
    /// option, origin field and the rest — or `None` when it holds no such item.
    ///
    /// What a journey compares a copy's destination against field by field, rather than one
    /// accessor per field.
    #[must_use]
    pub fn item(&self, id: &str) -> Option<Value> {
        self.board
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|item| item["id"] == json!(id))
            .cloned()
    }

    // llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] A read of the loopback board's private state beside `item` and `blockers`, which only this handle can make; the board lives in this shared harness for the reason `GitHubBoard` records.
    /// The id of every item this board holds whose title is `title`, in the order it holds
    /// them: what says a copy updated an item rather than adding a second beside it.
    #[must_use]
    pub fn items_titled(&self, title: &str) -> Vec<String> {
        self.board
            .lock()
            .unwrap()
            .items
            .iter()
            .filter(|item| item["title"] == json!(title))
            .filter_map(|item| item["id"].as_str().map(str::to_owned))
            .collect()
    }
    // llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

    /// The ids of the issues this board records as blocking `id`, in the order they were
    /// recorded.
    #[must_use]
    pub fn blockers(&self, id: &str) -> Vec<String> {
        self.board
            .lock()
            .unwrap()
            .blocked_by
            .iter()
            .find(|(near, _)| near == id)
            .map(|(_, blockers)| blockers.clone())
            .unwrap_or_default()
    }

    /// Which of the documents this board received selected the board's own item
    /// connection — the read whose cost is the whole board.
    #[must_use]
    pub fn board_item_reads(&self) -> Vec<String> {
        self.documents()
            .into_iter()
            .filter(|document| {
                document.contains("projectV2(number:$number)")
                    && document.contains("items(first:$first,after:$after)")
            })
            .collect()
    }

    /// The board's own `title`, `shortDescription` and `readme`, asked of the GitHub
    /// endpoint the way any client of it asks.
    ///
    /// It crosses the same HTTP boundary the product crosses rather than reading the
    /// fixture's own memory, so a journey asserting the board is untouched is asserting
    /// what a person opening that board would see.
    #[must_use]
    pub fn own(&self) -> Value {
        self.graphql_over_http(
            "query($id:ID!){node(id:$id){... on ProjectV2{title shortDescription readme}}}",
            &json!({"id":"PVT-board"}),
        )["node"]
            .clone()
    }

    /// One GraphQL request to this board, sent as the product itself sends one, over the
    /// connection this handle keeps to it, and the `data` it is answered with.
    ///
    /// A connection that ends or is reset before the answer arrives — which a Windows host
    /// reports as `ConnectionReset` (`os error 10054`) — is answered here rather than
    /// panicking the journey: the request is sent again on a fresh connection when the
    /// board never recorded it, or when it is a read, which a second sending cannot change.
    /// A mutation the board recorded and whose answer was lost is the one case that cannot
    /// be retried without being applied twice, and it fails naming that.
    fn graphql_over_http(&self, query: &str, variables: &Value) -> Value {
        const ATTEMPTS: usize = 3;
        let address = self
            .endpoint
            .strip_prefix("http://")
            .and_then(|rest| rest.split('/').next())
            .expect("a fixture endpoint spelled http://host:port/graphql");
        let body = json!({"query":query,"variables":variables}).to_string();
        let request = format!(
            "POST /graphql HTTP/1.1\r\nHost: {address}\r\nauthorization: Bearer test-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mutation = query.trim_start().starts_with("mutation");
        for _ in 0..ATTEMPTS {
            let recorded = self.board.lock().unwrap().documents.len();
            let response = self
                .client
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .forward(request.as_bytes());
            if response.is_empty() {
                // `forward` dropped the connection that failed, and has said why on standard
                // error; the next attempt opens another.
                let reached = self.board.lock().unwrap().documents.len() > recorded;
                assert!(
                    !(reached && mutation),
                    "the board recorded a mutation and its answer was lost: sending it again \
                     would apply it twice ({query})"
                );
                continue;
            }
            let at = response
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .expect("HTTP header terminator")
                + 4;
            let answered: Value =
                serde_json::from_slice(&response[at..]).expect("fixture response JSON");
            return answered["data"].clone();
        }
        panic!("the board answered none of {ATTEMPTS} sendings of {query}")
    }
}

/// The connection a [`GitHubBoardFields`] keeps to the board at `endpoint`, opened by its
/// first request.
fn board_client(endpoint: &str) -> Arc<Mutex<loopback::Upstream>> {
    let address = endpoint
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .and_then(|address| address.parse().ok())
        .expect("a fixture endpoint spelled http://host:port/graphql");
    Arc::new(Mutex::new(loopback::Upstream::new(address)))
}

impl GitHubBoard {
    fn options(&self) -> Value {
        Value::Array(self.status_options.clone())
    }

    /// The `Priority` field's options as every read that defines the field answers them, or
    /// `None` for a board without the field.
    fn priority_field_options(&self) -> Option<Value> {
        let options = self.priority_options.as_ref()?;
        Some(if self.malformed_priority_options {
            json!("Urgent, High, Medium, Low")
        } else {
            json!(options)
        })
    }

    fn fields(&self) -> Value {
        let mut nodes = vec![json!({"__typename":"ProjectV2Field","id":"FIELD-origin",
            "name":"onetaskgraph.origin","dataType":"TEXT"})];
        if self.status_field_present {
            nodes.insert(
                0,
                json!({"__typename":"ProjectV2SingleSelectField",
                "id":"FIELD-status","name":"Status","options":self.options()}),
            );
        }
        if let Some(options) = self.priority_field_options() {
            nodes.push(json!({"__typename":"ProjectV2SingleSelectField",
                "id":"FIELD-priority","name":"Priority","options":options}));
        }
        nodes.extend(self.persons_fields.iter().cloned());
        nodes.extend(self.text_fields.iter().cloned());
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}})
    }

    fn subs(&self, id: &str) -> usize {
        self.items
            .iter()
            .filter(|item| item["parent"] == json!(id))
            .count()
    }

    fn find(&mut self, id: &Value) -> &mut Value {
        self.items
            .iter_mut()
            .find(|item| item["id"] == *id)
            .expect("the fixture holds the item being written")
    }

    /// Whether this issue has a card on this server's first board — every issue does but one
    /// a journey took off it with [`GitHubBoardFields::take_off_the_board`], which is an issue
    /// of the repository on no board at all, or on the second board alone.
    fn on_board(item: &Value) -> bool {
        item["off_board"] != json!(true)
    }

    /// Whether this item is a board draft rather than an issue.
    fn is_draft(item: &Value) -> bool {
        item["type"] == "DraftIssue"
    }

    /// The login a request naming this board's repository by owner and name names it by: what
    /// the board holds each such request to.
    fn repository_owner(&self) -> &str {
        self.repository
            .split_once('/')
            .map_or(self.repository.as_str(), |(owner, _)| owner)
    }

    /// Place every issue whose placement delay has passed: from here on it is on the board.
    fn settle_placements(&mut self) {
        let Some(placement) = &mut self.placement else {
            return;
        };
        let now = (placement.now)();
        let due: Vec<Value> = placement
            .waiting
            .iter()
            .filter(|(_, _, due)| due.is_some_and(|due| due <= now))
            .map(|(id, ..)| id.clone())
            .collect();
        placement.waiting.retain(|(id, ..)| !due.contains(id));
        for id in due {
            if let Some(at) = self.pending.iter().position(|item| item["id"] == id) {
                let mut item = self.pending.remove(at);
                item["itemUpdatedAt"] = json!(self.tick());
                self.items.push(item);
            }
        }
    }

    /// The next moment this board stamps anything with — a comment, an issue's `updatedAt`, a
    /// board item's — a second after the one before, so no two changes share one, unless a
    /// journey holds the clock.
    fn tick(&mut self) -> String {
        self.comment_ticks += 1;
        let ticks = self.held_second.unwrap_or(self.comment_ticks);
        assert!(ticks < 86_400, "this board's clock runs one day");
        format!(
            "2026-09-01T{:02}:{:02}:{:02}Z",
            ticks / 3600,
            (ticks / 60) % 60,
            ticks % 60
        )
    }

    /// The server's clock on an answer, as GitHub's `Date` header spells it: `read_lag`
    /// seconds past this board's latest change, or `None` when it answers without one.
    fn served_at(&self) -> Option<String> {
        let lag = self.read_lag?;
        // While the clock is held every change carries the held second, however many it made.
        let latest = self.held_second.unwrap_or(self.comment_ticks);
        let epoch: chrono::DateTime<chrono::Utc> =
            GITHUB_BOARD_EPOCH.parse().expect("the board's epoch");
        let seconds = i64::try_from(latest + lag).expect("a day's seconds");
        Some(
            (epoch + chrono::TimeDelta::seconds(seconds))
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string(),
        )
    }

    /// Move the issue `id`'s own `updatedAt`, as GitHub does for an edit of its title, body,
    /// labels or state, and for a comment added or edited on it — never for a dependency or a
    /// project membership, which is [`GitHubBoard::record_relationship_event`].
    fn touch_issue(&mut self, id: &Value) {
        let at = self.tick();
        if let Some(item) = self
            .items
            .iter_mut()
            .chain(self.pending.iter_mut())
            .find(|item| item["id"] == *id)
        {
            item["updatedAt"] = json!(at);
        }
    }

    /// Move the board item `item`'s own `updatedAt`, as GitHub does for a change to one of its
    /// field values — the issue's own does not move.
    fn touch_item(&mut self, item: &Value) {
        let at = self.tick();
        if let Some(held) = self.items.iter_mut().find(|held| held["item"] == *item) {
            held["itemUpdatedAt"] = json!(at);
        }
    }

    /// Record a relationship — a dependency, or a project membership — added between the
    /// issues `id` and `other` or removed, as GitHub does: one event on each end's timeline,
    /// which a `timelineItems` count of the relationship event types answers — and **neither
    /// issue's `updatedAt` nor either board item's**, because the credentialed lane added a
    /// dependency to a real issue and saw all three stay put, and a project membership is held
    /// to the same worst case.
    ///
    /// Each end's event is kept under its own [`TimelineEvent`] — `BLOCKED_BY_ADDED_EVENT` on
    /// the blocked issue and `BLOCKING_ADDED_EVENT` on the blocking one,
    /// `PARENT_ISSUE_ADDED_EVENT` on a sub-issue and `SUB_ISSUE_ADDED_EVENT` on its parent — so a
    /// read counts only the types its document names, as GitHub's `itemTypes` filter does.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] The loopback board's model of GitHub's relationship timeline, a method of `GitHubBoard` because it changes that board's private state; the board lives in this shared harness for the reason recorded on `GitHubBoard` itself.
    fn record_relationship_event(
        &mut self,
        (near, far): (&str, &str),
        (near_kind, far_kind): (TimelineEvent, TimelineEvent),
    ) {
        for (end, other, kind) in [(near, far, near_kind), (far, near, far_kind)] {
            if let Some(item) = self
                .items
                .iter_mut()
                .chain(self.pending.iter_mut())
                .find(|item| item["id"] == end)
            {
                let held = item["timeline"][kind.name()].as_u64().unwrap_or(0);
                item["timeline"][kind.name()] = json!(held + 1);
                self.relationship_log
                    .push((end.to_owned(), other.to_owned(), kind));
            }
        }
    }

    /// How many of the issue `item`'s timeline events are of a type the request being answered
    /// counts, as `timelineItems(itemTypes:) { totalCount }` answers.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] How the loopback board answers a filtered timeline count, a method of `GitHubBoard` because it reads that board's private state; the board lives in this shared harness for the reason recorded on `GitHubBoard` itself.
    fn timeline_count(&self, item: &Value) -> u64 {
        self.counted_types
            .iter()
            .filter_map(|kind| item["timeline"][kind.name()].as_u64())
            .sum()
    }

    /// How many comments the issue `id` holds, as `comments { totalCount }` answers.
    ///
    /// Counted again only when the board's comments may have changed since the last count of
    /// that issue: comments are only ever added at the end, under an id no comment has had, so
    /// none was added or removed while their number and the last one's id stand.
    fn comment_count(&self, id: &Value) -> usize {
        let last = self
            .comments
            .last()
            .and_then(|held| held["id"].as_str())
            .map(str::to_owned);
        let mut counted = self.comment_counted.lock().unwrap();
        if let Some(held) = counted.as_ref()
            && id.as_str() == Some(held.issue.as_str())
            && held.comments == self.comments.len()
            && held.last == last
        {
            return held.count;
        }
        let count = self
            .comments
            .iter()
            .filter(|held| held["issue"] == *id)
            .count();
        *counted = id.as_str().map(|issue| CommentCount {
            issue: issue.to_owned(),
            comments: self.comments.len(),
            last,
            count,
        });
        count
    }

    /// Put a comment on `issue`, signed as [`GITHUB_COMMENTER`] — the account this board's
    /// token belongs to, which is who GitHub records as the author of every comment.
    fn comment(&mut self, issue: &str, body: &str) -> Value {
        let at = self.tick();
        let held = json!({"id":format!("COMMENT-{}", self.comment_ticks),"issue":issue,
                          "author":GITHUB_COMMENTER,"body":body,"createdAt":at,"updatedAt":at});
        self.comments.push(held.clone());
        self.touch_issue(&json!(issue));
        held
    }

    /// One page of an issue's comments, oldest first, resumed from `after`.
    ///
    /// `after` counts the issue's comments already listed. The page starts where the page that
    /// answered that cursor ended when nothing at or before its last comment has gone since —
    /// comments are only ever added at the end — and otherwise counts them again from the first,
    /// so either way the answer is the same: a walk of many pages costs each page its own length
    /// rather than every comment the board holds.
    fn comment_connection(&self, id: &Value, after: &Value, first: &Value) -> Value {
        let offset = match after {
            Value::Null => 0,
            Value::String(cursor) => cursor.parse::<usize>().expect("numeric after cursor"),
            other => panic!("GraphQL after must be null or a numeric string: {other}"),
        };
        let first = usize::try_from(
            first
                .as_u64()
                .expect("GraphQL first must be an unsigned integer"),
        )
        .expect("GraphQL first fits usize");
        assert!((1..=100).contains(&first), "comments first is out of range");
        let mut resume = self.comment_resume.lock().unwrap();
        let start = resume
            .as_ref()
            .filter(|held| {
                id.as_str() == Some(held.issue.as_str())
                    && held.offset == offset
                    && held.position > 0
                    && self
                        .comments
                        .get(held.position - 1)
                        .is_some_and(|last| last["id"].as_str() == Some(held.last.as_str()))
            })
            .map(|held| held.position);
        let (from, skip) = start.map_or((0, offset), |position| (position, 0));
        let mut remaining = self
            .comments
            .iter()
            .enumerate()
            .skip(from)
            .filter(|(_, held)| held["issue"] == *id)
            .skip(skip);
        let page: Vec<(usize, &Value)> = remaining.by_ref().take(first).collect();
        let more = remaining.next().is_some();
        let end = offset + page.len();
        *resume = page.last().and_then(|(position, last)| {
            Some(CommentResume {
                issue: id.as_str()?.to_owned(),
                offset: end,
                position: position + 1,
                last: last["id"].as_str()?.to_owned(),
            })
        });
        let nodes = page
            .iter()
            .map(|(_, held)| GitHubBoard::comment_node(held))
            .collect::<Vec<_>>();
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":more,
                                          "endCursor":(!page.is_empty()).then(|| end.to_string())}})
    }

    /// One node a detail read reaches: the issue as a node read answers it, with a page of
    /// its comments when asked — a draft answered by its type alone, as GitHub answers a
    /// fragment on `Issue` about something that is not one.
    fn detail_node(&self, id: &Value, after: &Value, first: &Value, comments: bool) -> Value {
        let Some(item) = self.items.iter().find(|item| item["id"] == *id) else {
            return Value::Null;
        };
        if GitHubBoard::is_draft(item) {
            return json!({"__typename":"DraftIssue"});
        }
        let mut node = self.as_issue(item);
        if comments {
            node["comments"] = self.comment_connection(id, after, first);
        }
        node
    }
    /// One held comment as every comment document selects it.
    fn comment_node(held: &Value) -> Value {
        json!({"id":held["id"],"author":{"login":held["author"]},"createdAt":held["createdAt"],
               "updatedAt":held["updatedAt"],"body":held["body"],
               "url":format!("https://example.invalid/{}#{}",
                             held["issue"].as_str().expect("a comment's issue"),
                             held["id"].as_str().expect("a comment's id"))})
    }

    fn content(&self, item: &Value) -> Value {
        if Self::is_draft(item) {
            // A draft has no url, state, repository, parent, sub-issues or labels on GitHub,
            // which is what a board read of one selects nothing of.
            return json!({"__typename":"DraftIssue","id":item["id"],"title":item["title"],
                          "body":item["body"],"createdAt":null,"updatedAt":null});
        }
        json!({"__typename":"Issue","id":item["id"],"title":item["title"],"body":item["body"],
               "number":github_number(item["id"].as_str().unwrap()),
               "url":format!("https://example.invalid/{}", item["id"].as_str().unwrap()),
               "createdAt":item["createdAt"],"updatedAt":item["updatedAt"],
               "commentCount":{"totalCount":self.comment_count(&item["id"])},
               "relationshipEvents":{"totalCount":self.timeline_count(item)},
               "state":item["state"],
               "stateReason":item["reason"],
               "repository":item["repo"].as_str().map(|repo| json!({"nameWithOwner":repo})),
               "parent":item["parent"].as_str().map(|id| json!({"id":id})),
               "subIssuesSummary":{"total":self.subs(item["id"].as_str().unwrap())},
               "labels":{"nodes":item["labels"].as_array().unwrap().iter()
                   .map(|pair| json!({"id":pair[0],"name":pair[1],
                                      "color":pair.get(2).cloned().unwrap_or(Value::Null)}))
                   .collect::<Vec<_>>(),
                   "pageInfo":{"hasNextPage":item["more_labels"] == json!(true)}}})
    }

    /// The two halves of an origin lookup, answered the way GitHub answers each: the board's
    /// own item connection under its field filter — an exact match on the origin field, over
    /// what a board read can see — and the board-scoped issue search for the value as a
    /// phrase in the body, by token, over what the search index can see.
    fn origin_lookup(&self, variables: &Value) -> Value {
        assert_eq!(variables["type"], "ISSUE");
        let filter = variables["filter"].as_str().expect("a field filter");
        let quoted = filter
            .strip_prefix("onetaskgraph.origin:\"")
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_else(|| panic!("an origin field filter: {filter}"));
        let mut wanted = String::new();
        let mut chars = quoted.chars();
        while let Some(character) = chars.next() {
            match character {
                '\\' => wanted.extend(chars.next()),
                other => wanted.push(other),
            }
        }
        let search = variables["search"].as_str().expect("a search query");
        let phrase = IssueSearch::parse(
            search
                .strip_prefix("project:fixture-owner/7 is:issue")
                .unwrap_or_else(|| panic!("a search scoped to the configured board: {search}")),
        );
        let first = usize::try_from(variables["originFirst"].as_u64().expect("originFirst"))
            .expect("originFirst fits usize");
        let visible = self.items.len().saturating_sub(self.lagging_reads);
        let carriers = self.items[..visible]
            .iter()
            .filter(|item| Self::on_board(item) && item["origin"] == json!(wanted))
            .map(|item| self.rendered(item))
            .collect();
        let found = self.items[..visible]
            .iter()
            .filter(|item| !Self::is_draft(item) && Self::on_board(item))
            .filter(|item| {
                phrase.admits(
                    item["title"].as_str().unwrap_or_default(),
                    item["body"].as_str().unwrap_or_default(),
                )
            })
            .map(|item| self.as_issue(item))
            .collect();
        let page = |rows: Vec<Value>, after: &Value| {
            let offset = match after {
                Value::Null => 0,
                Value::String(cursor) => cursor.parse::<usize>().expect("numeric after cursor"),
                other => panic!("GraphQL after must be null or a numeric string: {other}"),
            };
            let end = (offset + first).min(rows.len());
            json!({"totalCount":rows.len(),"nodes":rows[offset.min(end)..end].to_vec(),
                   "pageInfo":{"hasNextPage":end < rows.len(),
                               "endCursor":(end > 0).then(|| end.to_string())}})
        };
        json!({"originItems":{"projectV2":{"items":page(carriers, &variables["itemsAfter"])}},
               "search":page(found, &variables["searchAfter"])})
    }

    /// One issue as a board-scoped search, a node read or a sub-issue read returns it:
    /// the issue's own fields with the board half riding along on `projectItems`.
    ///
    /// The board half is `rendered`'s, read back out of it rather than spelled twice, so
    /// an item reached through the board and the same item reached through its own id
    /// cannot disagree about its status or its origin.
    fn as_issue(&self, item: &Value) -> Value {
        let mut issue = self.content(item);
        issue["projectItems"] =
            Self::membership_page(self.memberships(item), 0, self.membership_first);
        issue
    }

    /// Every board membership of one issue, this board's own first and then each card a
    /// journey put it on beside it, as `Issue.projectItems` answers one.
    ///
    /// The board half is `rendered`'s, read back out of it rather than spelled twice, so an
    /// item reached through the board and the same item reached through its own id cannot
    /// disagree about its status or its origin.
    fn memberships(&self, item: &Value) -> Vec<Value> {
        let board_item = self.rendered(item);
        let mut nodes = Vec::new();
        if Self::on_board(item) {
            nodes.push(json!({"id":board_item["id"],
                              "updatedAt":board_item["updatedAt"],
                              "project":{"id":"PVT-board","number":7},
                              "fieldValues":board_item["fieldValues"]}));
        }
        for card in item["boards"].as_array().into_iter().flatten() {
            let number = card["number"].as_u64().expect("a card's board number");
            let values: Vec<Value> = card["status"]
                .as_str()
                .map(|name| {
                    json!({"name":name,"field":{"id":"FIELD-status","name":"Status",
                                                "options":self.options()}})
                })
                .into_iter()
                .collect();
            nodes.push(json!({"id":card["item"],"updatedAt":card["itemUpdatedAt"],
                              "project":{"id":format!("PVT-board{number}"),"number":number},
                              "fieldValues":{"nodes":values,"pageInfo":{"hasNextPage":false}}}));
        }
        nodes
    }

    /// One page of an issue's memberships, from `offset`, at most `first` of them.
    fn membership_page(nodes: Vec<Value>, offset: usize, first: usize) -> Value {
        let end = offset.saturating_add(first).min(nodes.len());
        let has_next = end < nodes.len();
        json!({"nodes":nodes[offset.min(end)..end].to_vec(),
               "pageInfo":{"hasNextPage":has_next,
                           "endCursor":(end > offset).then(|| end.to_string())}})
    }

    /// The card one issue holds on the board `number`, as a journey put it there.
    fn card_on(&mut self, number: u64, card: &Value) -> Option<&mut Value> {
        self.items.iter_mut().find_map(|item| {
            item["boards"].as_array_mut().and_then(|cards| {
                cards
                    .iter_mut()
                    .find(|held| held["number"] == json!(number) && held["item"] == *card)
            })
        })
    }

    /// The closed items of this board, as its own `is:closed` filter answers — every item,
    /// open ones included, while [`GitHubBoard::closed_filter_leaks`] says the filter leaks.
    fn closed_items(&self, variables: &Value) -> Value {
        assert_eq!(variables["owner"], "fixture-owner");
        assert_eq!(variables["number"], 7);
        assert_eq!(
            variables["filter"], "is:closed",
            "a reconcile reads the board through its closed filter alone"
        );
        let offset = match &variables["after"] {
            Value::Null => 0,
            Value::String(cursor) => cursor.parse::<usize>().expect("numeric after cursor"),
            other => panic!("GraphQL after must be null or a numeric string: {other}"),
        };
        let first = usize::try_from(variables["first"].as_u64().expect("GraphQL first"))
            .expect("GraphQL first fits usize");
        let visible = self.items.len().saturating_sub(self.lagging_reads);
        let closed: Vec<&Value> = self.items[..visible]
            .iter()
            .filter(|item| !Self::is_draft(item) && Self::on_board(item))
            .filter(|item| self.closed_filter_leaks || item["state"] == "CLOSED")
            .collect();
        let end = (offset + first).min(closed.len());
        let nodes: Vec<Value> = closed[offset.min(end)..end]
            .iter()
            .map(|item| self.rendered(item))
            .collect();
        let (more, cursor) = if self.closed_cursor_stalls && offset > 0 {
            (true, offset.to_string())
        } else {
            (end < closed.len(), end.to_string())
        };
        json!({"closedItems":{"projectV2":{"id":"PVT-board","fields":self.fields(),
            "items":{"nodes":nodes,"pageInfo":{"hasNextPage":more,"endCursor":cursor}}}}})
    }

    /// Each reference of a batch read by reference, answered as GitHub answers one: the issue
    /// that repository holds under that number, or `null` beside a `NOT_FOUND` error naming the
    /// alias — its first slot bent once as [`ReferenceFault`] says, when one is armed.
    fn issues_by_reference(&mut self, variables: &Value) -> Value {
        let mut answer = self.issues_by_reference_as_github_answers(variables);
        match self.reference_fault.take() {
            Some(ReferenceFault::NoAlias) => {
                answer.as_object_mut().expect("aliases").remove("r0");
            }
            Some(ReferenceFault::NoIssue) => {
                answer["r0"] = json!({});
            }
            Some(ReferenceFault::NotAnIssue) => {
                answer["r0"]["issue"]["__typename"] = json!("PullRequest");
            }
            Some(ReferenceFault::MembershipNotAnObject) => {
                answer["r0"]["issue"]["projectItems"]["nodes"] = json!(["ITEM"]);
            }
            Some(ReferenceFault::StatusNamingNoOption) => {
                answer["r0"]["issue"]["projectItems"]["nodes"][0]["status"] = json!({"name": 7});
            }
            None => {}
        }
        answer
    }

    fn issues_by_reference_as_github_answers(&mut self, variables: &Value) -> Value {
        let mut answer = serde_json::Map::new();
        for slot in 0..onetaskgraph_github_projects::REFERENCE_BATCH {
            let owner = variables[format!("owner{slot}")]
                .as_str()
                .expect("a reference's owner");
            let name = variables[format!("name{slot}")]
                .as_str()
                .expect("a reference's name");
            let number = variables[format!("number{slot}")]
                .as_u64()
                .expect("a reference's number");
            let repository = format!("{owner}/{name}");
            let found = self.items.iter().find(|item| {
                !Self::is_draft(item)
                    && item["repo"]
                        .as_str()
                        .is_some_and(|repo| repo.eq_ignore_ascii_case(&repository))
                    && item["id"].as_str().map(github_number) == Some(number)
            });
            match found {
                Some(item) => {
                    let mut issue = self.as_issue(item);
                    // Each membership's `Status` value read by name, as `fieldValueByName`
                    // answers it, in place of its field values.
                    for node in issue["projectItems"]["nodes"]
                        .as_array_mut()
                        .into_iter()
                        .flatten()
                    {
                        let status = node["fieldValues"]["nodes"]
                            .as_array()
                            .and_then(|values| {
                                values
                                    .iter()
                                    .find(|value| value["field"]["name"] == "Status")
                            })
                            .cloned()
                            .unwrap_or(Value::Null);
                        node["status"] = status;
                        node.as_object_mut()
                            .expect("a membership")
                            .remove("fieldValues");
                    }
                    answer.insert(format!("r{slot}"), json!({ "issue": issue }));
                }
                None => {
                    answer.insert(format!("r{slot}"), json!({ "issue": null }));
                    self.owed_error_objects.push(json!({
                        "type": "NOT_FOUND",
                        "path": [format!("r{slot}"), "issue"],
                        "message": format!(
                            "Could not resolve to an issue or pull request with the number of \
                             {number}."
                        )
                    }));
                }
            }
        }
        Value::Object(answer)
    }

    fn rendered(&self, item: &Value) -> Value {
        // No board field value carries labels: no document this source sends selects the
        // board's built-in `Labels` field, so an item's labels are its content's alone.
        // A card with no `Status` carries no node for the field, as GitHub leaves an empty
        // single-select out of `fieldValues`.
        let mut values: Vec<Value> = item["status"]
            .is_string()
            .then(|| {
                json!({"name":item["status"],"field":{"id":"FIELD-status","name":"Status",
                       "options":self.options()}})
            })
            .into_iter()
            .collect();
        // An origin field holding nothing — an item a copy out of a source declared private
        // wrote — carries no node, as GitHub leaves an empty text field out.
        if item["origin"]
            .as_str()
            .is_none_or(|origin| !origin.is_empty())
        {
            values.push(
                json!({"text":item["origin"],"field":{"id":"FIELD-origin","name":"onetaskgraph.origin"}}),
            );
        }
        // An item with no `Priority` value carries no node for the field, as GitHub leaves
        // an empty single-select out of `fieldValues`.
        if let (Some(name), Some(options)) =
            (item["priority"].as_str(), self.priority_field_options())
        {
            values.push(
                json!({"name":name,"field":{"id":"FIELD-priority","name":"Priority",
                               "options":options}}),
            );
        }
        // A text field holding nothing carries no node, as GitHub leaves it out.
        for field in &self.text_fields {
            if let Some(text) = item["texts"][field["id"].as_str().unwrap_or_default()].as_str() {
                values.push(json!({"text":text,"field":{"id":field["id"],"name":field["name"]}}));
            }
        }
        // A person's own text fields, which every read selecting a text value carries too.
        if let Some(own) = item["own_fields"].as_object() {
            for (name, text) in own {
                values.push(
                    json!({"text":text,"field":{"id":format!("FIELD-own-{name}"),
                                                         "name":name}}),
                );
            }
        }
        json!({"id":item["item"],"updatedAt":item["itemUpdatedAt"],
               "fieldValues":{"nodes":values,"pageInfo":{"hasNextPage":false}},
               "content":self.content(item)})
    }

    /// Every far end one issue is related to, in the direction asked for.
    fn related(&self, id: &str, blocking: bool) -> Value {
        let ids: Vec<String> = if blocking {
            self.blocked_by
                .iter()
                .filter(|(_, blockers)| blockers.iter().any(|blocker| blocker == id))
                .map(|(near, _)| near.clone())
                .collect()
        } else {
            self.blocked_by
                .iter()
                .find(|(near, _)| near == id)
                .map(|(_, blockers)| blockers.clone())
                .unwrap_or_default()
        };
        Value::Array(
            ids.into_iter()
                .map(|id| {
                    let far = self.items.iter().find(|item| item["id"] == json!(id));
                    json!({"id":id,
                           "title":far.map(|item| item["title"].clone()),
                           "body":far.map(|item| item["body"].clone()),
                           "parent":far.and_then(|item| item["parent"].as_str())
                               .map(|parent| json!({"id":parent})),
                           "subIssuesSummary":{"total":self.subs(&id)}})
                })
                .collect(),
        )
    }
}

fn github_projects_server(sandbox: &Sandbox, recorded: Option<Value>) -> Value {
    github_projects_board(sandbox, recorded, &[]).0
}

/// The login every comment added through a GitHub fixture board is signed with.
///
/// GitHub records the account a token belongs to as the author of every comment made with
/// it, and this board's token is its owner's.
pub const GITHUB_COMMENTER: &str = "fixture-owner";

/// The shared board, refusing each operation `fail_first` names once, with its handle.
pub fn github_projects_with_board_failing(
    sandbox: &Sandbox,
    fail_first: &'static [&'static str],
) -> (Value, GitHubBoardFields) {
    github_projects_board(sandbox, None, fail_first)
}

/// The id of the one draft item [`github_projects_with_draft`] adds to the shared board.
pub const GITHUB_DRAFT_TASK: &str = "DRAFT-1";

/// The shared board plus one **draft** item, [`GITHUB_DRAFT_TASK`], filed under no project.
///
/// A draft is a task of a board and not an issue, so `task show` reports it and every comment
/// verb on it is refused — GitHub keeps comments on issues alone. It is a board of its own
/// rather than a ninth item of the shared one because the shared journeys count what that
/// board holds, and a draft is a task every one of those counts would then have to know
/// about.
pub fn github_projects_with_draft(sandbox: &Sandbox) -> Value {
    let draft = json!({"item":format!("ITEM-{GITHUB_DRAFT_TASK}"),"id":GITHUB_DRAFT_TASK,
        "type":"DraftIssue","title":"Sketch the next plan","body":"a draft nobody has converted",
        "state":"OPEN","reason":null,"parent":null,"repo":null,"status":"Todo","origin":"",
        "labels":[]});
    github_projects_board_at(sandbox, None, &[], 0, vec![draft]).0
}

/// The id of the `n`th task [`github_projects_with_tasks`] adds to the shared board.
#[must_use]
pub fn github_extra_task(n: usize) -> String {
    format!("X-{n}")
}

/// The shared board plus `count` more task issues, [`github_extra_task`] `0..count`, filed
/// under no project — enough of them that a read of them all is more than one batch.
pub fn github_projects_with_tasks(sandbox: &Sandbox, count: usize) -> (Value, GitHubBoardFields) {
    let extra = (0..count)
        .map(|n| {
            let id = github_extra_task(n);
            json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue",
                "title":format!("Extra task {n}"),"body":format!("extra {n}"),"state":"OPEN",
                "reason":null,"parent":null,"repo":"nickderobertis/onetaskgraph",
                "status":"Todo","origin":"","labels":[]})
        })
        .collect();
    github_projects_board_at(sandbox, None, &[], 0, extra)
}

/// The shared board plus `items`, each spelled as the dataset spells a board item — `id`,
/// `type`, `title`, `body`, `state`, `parent` and the rest — and put on the board after it, so
/// every id the dataset holds is where it always was.
///
/// What a suite of one plugin builds a board of its own shape from: the item shape is the
/// board's, so it is spelled here, and the shape of the plan it adds is the suite's.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] The loopback board's only constructor, `github_projects_board_at`, and its private state live here for the reason `GitHubBoard` records, so a board of a shape one suite needs can only be opened through an entry point beside it; the shape itself — the plan a GitHub Projects journey draws — is built in that suite, `crates/onetaskgraph-github-projects-e2e/tests/e2e/graph.rs`, as `github_projects_with_tasks` serves the engine's.
pub fn github_projects_with_items(
    sandbox: &Sandbox,
    items: Vec<Value>,
) -> (Value, GitHubBoardFields) {
    github_projects_board_at(sandbox, None, &[], 0, items)
}

/// The same board, with a handle on the fields this source must never write.
pub fn github_projects_with_board(sandbox: &Sandbox) -> (Value, GitHubBoardFields) {
    github_projects_board(sandbox, None, &[])
}

/// The read of where a created issue was filed — the board item GitHub adds once a copy's
/// `createIssue` names the board in `projectV2Ids` — as the failures below name it.
const READ_OF_FILED_ITEMS: &str = "status:fieldValueByName(name:$statusName)";

/// The same board again, failing the first read of the board item a created issue was filed
/// as.
///
/// Creating an item there is several calls — `createIssue` filing it on the board, then a
/// read of the board item GitHub adds after answering, then the board's own fields — so
/// GitHub can fail part way through, and what the product does with an issue that exists but
/// has no board item it can write is a journey rather than a reading of the code. The failure
/// is spent once, so the same board answers the retry.
pub fn github_projects_failing_to_read_a_filing_once(sandbox: &Sandbox) -> Value {
    github_projects_board(sandbox, None, &[READ_OF_FILED_ITEMS]).0
}

/// The same board, failing the first field write onto an item it has already filed.
///
/// The later half of that sequence: the issue exists and is on the board, and the copy
/// origin and status that make it findable and readable are what did not land.
pub fn github_projects_failing_a_field_write_once(sandbox: &Sandbox) -> Value {
    // Whichever document carries the write: one item's own, or a copy's aliased batch.
    github_projects_board(sandbox, None, &["updateProjectV2ItemFieldValue(input:$"]).0
}

/// The same board, failing the read of where a created issue was filed *and* the tidy-up that
/// would remove it.
///
/// The earlier half of the same sequence: the issue exists in the repository and nothing of it
/// past its create landed, so the source deletes it — and this board refuses that too. The
/// caller is owed the failure that stopped the copy, for the reason the sibling below gives.
pub fn github_projects_failing_to_read_a_filing_and_its_cleanup(sandbox: &Sandbox) -> Value {
    github_projects_board(sandbox, None, &[READ_OF_FILED_ITEMS, "deleteIssue(input:$"]).0
}

/// The same board, failing a field write *and* the tidy-up that would take the issue back.
///
/// The source removes an issue it created when the rest of the write fails, so a caller is
/// not left an item nobody asked for. GitHub can refuse that removal too — a permission,
/// a rate limiter — and what the caller is owed then is the failure that made the write
/// fail, not the one that made the tidy-up fail: the second is about an item they never
/// asked to exist, and reporting it would hide why the copy stopped.
pub fn github_projects_failing_a_field_write_and_its_cleanup(sandbox: &Sandbox) -> Value {
    github_projects_board(
        sandbox,
        None,
        &[
            "updateProjectV2ItemFieldValue(input:$",
            "deleteIssue(input:$",
        ],
    )
    .0
}

/// The same board, whose reads never show the item most recently filed on it.
///
/// GitHub's `projectV2.items` is eventually consistent, and a copy that created a task and
/// then wrote the task depending on it looked the far end up, did not find it, and refused
/// naming an item that same run had just created. The board here holds every item — every
/// mutation finds them — and only its read is behind, which is the whole of the hazard.
pub fn github_projects_reading_one_item_behind(sandbox: &Sandbox) -> Value {
    github_projects_board_lagging(sandbox, 1)
}

fn github_projects_board(
    sandbox: &Sandbox,
    recorded: Option<Value>,
    fail_first: &'static [&'static str],
) -> (Value, GitHubBoardFields) {
    github_projects_board_at(sandbox, recorded, fail_first, 0, Vec::new())
}

fn github_projects_board_lagging(sandbox: &Sandbox, lagging_reads: usize) -> Value {
    github_projects_board_at(sandbox, None, &[], lagging_reads, Vec::new()).0
}

/// The credentials every GitHub fixture board accepts: one per source a journey configures
/// against it, so which source sent a request is read off the token it carried.
const GITHUB_FIXTURE_SECRETS: &str =
    "GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nGITHUB_PROJECTS_FIXTURE_TOKEN_B=test-token-b\n";

/// Which of a member's two fields a page of a project's members carries beside a batched origin
/// lookup is answered short of what the board holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembersPageField {
    Body,
    Repository,
}

impl MembersPageField {
    /// The field's key in the page's node.
    const fn key(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Repository => "repository",
        }
    }
}

/// How one alias of a batched write that landed is answered malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasAnswer {
    /// With no member for the alias at all, and no error beside it.
    Missing,
    /// With another item's id than the one its input named.
    NamingAnother,
    /// With another issue than the one its input named as blocking, the blocked one right.
    NamingAnotherBlocker,
}

/// How a read by issue reference can come back malformed in its first slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceFault {
    /// With no alias for the slot at all.
    NoAlias,
    /// With the slot's repository answered and no `issue` member in it.
    NoIssue,
    /// With the slot's issue answered as a node that is not an issue.
    NotAnIssue,
    /// With the slot's issue holding a board membership that is not an object.
    MembershipNotAnObject,
    /// With the slot's issue's first membership answering a status that names no option.
    StatusNamingNoOption,
}

/// The token a source configured by [`GitHubBoardFields::second_board`] sends, which tells its
/// requests from the first board's in this server's one log.
pub const GITHUB_SECOND_BOARD_TOKEN: &str = "test-token-b";

/// The project number of the second board one fixture server holds.
pub const GITHUB_SECOND_BOARD: u64 = 8;

/// `fail_first` names the operations this board refuses, each once — so a retry, and the
/// tidy-up that follows a refusal, meet the board answering normally again. `extra` is put on
/// the board after the shared dataset, so every id that dataset holds is where it always was.
fn github_projects_board_at(
    sandbox: &Sandbox,
    recorded: Option<Value>,
    fail_first: &'static [&'static str],
    lagging_reads: usize,
    extra: Vec<Value>,
) -> (Value, GitHubBoardFields) {
    sandbox.secrets_file(GITHUB_FIXTURE_SECRETS);
    let listener = TcpListener::bind("127.0.0.1:0").expect("GitHub fixture listener");
    let endpoint = format!(
        "http://{}/graphql",
        listener.local_addr().expect("fixture address")
    );
    let mut items = github_dataset(recorded.as_ref());
    items.extend(extra);
    // Every issue the board starts with was last changed at its epoch, as far as either of its
    // times says, so each carries a change token from the first read.
    for item in &mut items {
        if !GitHubBoard::is_draft(item) {
            for time in ["updatedAt", "itemUpdatedAt"] {
                if item.get(time).is_none_or(Value::is_null) {
                    item[time] = json!(GITHUB_BOARD_EPOCH);
                }
            }
        }
    }
    let board = Arc::new(Mutex::new(GitHubBoard {
        items,
        comments: Vec::new(),
        comment_ticks: 0,
        held_second: None,
        read_lag: Some(3600),
        pending: Vec::new(),
        failing_alias: None,
        bent_alias: None,
        blank_repository_id: false,
        failing_field: None,
        owed_errors: Vec::new(),
        refusing: Vec::new(),
        refusing_after: Vec::new(),
        rate_limit_next: None,
        replacing: Vec::new(),
        blocked_by: github_blockers(),
        counted_types: Vec::new(),
        relationship_log: Vec::new(),
        created: 0,
        lagging_reads,
        documents: Vec::new(),
        tokens: Vec::new(),
        arrivals: Vec::new(),
        membership_first: usize::MAX,
        owed_error_objects: Vec::new(),
        closed_filter_leaks: false,
        closed_cursor_stalls: false,
        erring_beside: Vec::new(),
        reference_fault: None,
        variables: Vec::new(),
        status_options: json!([
            {"id":"OPT-backlog","name":"Backlog","color":"GRAY","description":""},
            {"id":"OPT-todo","name":"Todo","color":"BLUE","description":"ready"},
            {"id":"OPT-queued","name":"Queued","color":"YELLOW","description":""},
            {"id":"OPT-doing","name":"Doing","color":"GREEN","description":"active"},
            {"id":"OPT-shipped","name":"Shipped","color":"PURPLE","description":"custom"},
            {"id":"OPT-done","name":"Done","color":"GREEN","description":""},
            {"id":"OPT-cancelled","name":"Cancelled","color":"RED","description":""}
        ])
        .as_array()
        .unwrap()
        .clone(),
        priority_options: Some(
            json!([
                {"id":"OPT-p-urgent","name":"Urgent","color":"RED","description":""},
                {"id":"OPT-p-high","name":"High","color":"ORANGE","description":""},
                {"id":"OPT-p-medium","name":"Medium","color":"YELLOW","description":""},
                {"id":"OPT-p-low","name":"Low","color":"GRAY","description":""}
            ])
            .as_array()
            .unwrap()
            .clone(),
        ),
        comment_resume: std::sync::Mutex::new(None),
        comment_counted: std::sync::Mutex::new(None),
        drift_after_priority_update: false,
        drops_priority_writes: false,
        priority_response: None,
        hides_after_update: false,
        remints_after_priority_update: false,
        omits_added_priority_option: false,
        persons_fields: Vec::new(),
        text_fields: Vec::new(),
        omits_created_text_field: false,
        malformed_priority_options: false,
        repeated_snapshot_value: None,
        drift_after_status_update: false,
        remint_after_status_update: false,
        omit_added_status_option: false,
        files_without_status: false,
        members_without_blocked_by: false,
        unfiled_board_item_reads: 0,
        placement: None,
        members_page_misanswers: Vec::new(),
        repository: "nickderobertis/onetaskgraph".to_owned(),
        omits_sub_issue_list: false,
        status_snapshot_page_size: None,
        status_field_present: true,
        status_board_accessible: true,
        own: json!({"title":"Fixture board",
                    "shortDescription":"the board a person set up",
                    "readme":"# Fixture board\n\nA person wrote this."}),
        project_public: false,
        project_public_after_reads: None,
        project_visibility_reads: 0,
        project_visibility_answer: None,
        project_visibility_error: None,
        project_scope_withheld: false,
        repository_visibility: std::collections::BTreeMap::new(),
        hooks: Vec::new(),
        search_page_rows: None,
        partial_writes: false,
        dropping: Vec::new(),
        refusing_stale: Vec::new(),
        omitting: Vec::new(),
        truncating: Vec::new(),
    }));
    let mut owed_failures: Vec<&'static str> = fail_first.to_vec();
    let watched = Arc::clone(&board);
    let connections = Arc::<AtomicUsize>::default();
    let hooked = GitHubBoardFields {
        client: board_client(&endpoint),
        endpoint: endpoint.clone(),
        board: Arc::clone(&board),
        connections: Arc::clone(&connections),
    };
    loopback::serve(listener, Arc::clone(&connections), move |sent| {
        let (line, request, token) = http_request_parts(sent);
        // The one REST read a write to a source declared private sends: the visibility of
        // the repository an issue lives in. Recorded beside the GraphQL documents, so a
        // journey counting what a write spent counts it too.
        if let Some(slug) = line
            .strip_prefix("GET /repos/")
            .and_then(|rest| rest.split_whitespace().next())
        {
            let answer = {
                let mut served = board.lock().unwrap();
                served.documents.push(format!("GET /repos/{slug}"));
                served.variables.push(Value::Null);
                served.tokens.push(token.clone());
                served.arrivals.push(std::time::Instant::now());
                served
                    .repository_visibility
                    .get(slug)
                    .copied()
                    .unwrap_or(RepositoryAnswer::Private)
            };
            let (status, body) = answer.served(slug);
            return Some(format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ));
        }
        let request = request.unwrap_or_else(|| panic!("a {line} carries no JSON body"));
        // Every one of these refuses a request the plugin should not have sent, and
        // each names the request it refused: a shape assumed silently here surfaces as
        // a journey failing about something else entirely.
        let query = request["query"]
            .as_str()
            .unwrap_or_else(|| panic!("GraphQL request carries no query string: {request}"));
        graphql_parser::parse_query::<String>(query)
            .unwrap_or_else(|problem| panic!("invalid GraphQL document ({problem}): {query}"));
        let variables = request["variables"]
            .as_object()
            .unwrap_or_else(|| panic!("GraphQL request carries no variables object: {request}"));
        let variables = Value::Object(variables.clone());
        {
            let mut served = board.lock().unwrap();
            served.documents.push(query.to_owned());
            served.variables.push(variables.clone());
            served.tokens.push(token.clone());
            served.arrivals.push(std::time::Instant::now());
            served.membership_first = ["memberships", "boardItems"]
                .iter()
                .find_map(|name| variables[name].as_u64())
                .map_or(usize::MAX, |first| {
                    usize::try_from(first).expect("a membership page fits usize")
                });
            let named = variables["id"].as_str().unwrap_or_default().to_owned();
            if let Some(at) = served.replacing.iter().position(|(id, ..)| *id == named) {
                if served.replacing[at].1 == 0 {
                    let (_, _, replacement) = served.replacing.remove(at);
                    served.items.retain(|item| item["id"] != named.as_str());
                    served.items.extend(replacement);
                } else {
                    served.replacing[at].1 -= 1;
                }
            }
        }
        if let Some(wait) = board.lock().unwrap().rate_limit_next.take() {
            let body = r#"{"message":"API rate limit exceeded for this fixture"}"#;
            return Some(format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nretry-after: {wait}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ));
        }
        {
            // Refusals named after the board was built join the ones it was built with.
            let mut held = board.lock().unwrap();
            owed_failures.append(&mut held.refusing);
        }
        let late = {
            let mut held = board.lock().unwrap();
            let due = held
                .refusing_after
                .iter()
                .position(|(operation, _)| query.contains(operation));
            match due {
                Some(at) if held.refusing_after[at].1 == 0 => {
                    Some(held.refusing_after.remove(at).0)
                }
                Some(at) => {
                    held.refusing_after[at].1 -= 1;
                    None
                }
                None => None,
            }
        };
        let owed = late.or_else(|| {
            owed_failures
                .iter()
                .position(|operation| query.contains(operation))
                .map(|at| owed_failures.remove(at))
        });
        let stale = {
            let mut held = board.lock().unwrap();
            held.refusing_stale
                .iter()
                .position(|operation| query.contains(operation))
                .map(|at| held.refusing_stale.remove(at))
        };
        let body = if let Some(operation) = stale {
            json!({"data":Value::Null,
                       "errors":[{"message":format!(
                           "Could not resolve to a node with the global id of '{operation}'")}]})
            .to_string()
        } else if let Some(operation) = owed {
            // The operation is named in the message so a journey can tell *which*
            // failure reached the caller — a copy whose write failed and whose tidy-up
            // then failed too has two, and which one it reports is the behaviour.
            json!({"data":Value::Null,
                       "errors":[{"message":format!(
                           "Something went wrong while executing your query: {operation}")}]})
            .to_string()
        } else {
            // A document selecting the whole issue it writes is answered as its plain twin
            // is, and then with that issue as a read of it by its own id shows it.
            let plain = github_plain_of(query);
            let data = github_answer(&board, plain.unwrap_or(query), &variables);
            let data = match plain {
                Some(_) => board.lock().unwrap().whole(query, data, &variables),
                None => data,
            };
            {
                let mut held = board.lock().unwrap();
                if let Some(at) = held
                    .erring_beside
                    .iter()
                    .position(|(operation, _)| query.contains(operation))
                {
                    let (_, message) = held.erring_beside.remove(at);
                    held.owed_errors.push(message);
                }
            }
            let owed = std::mem::take(&mut board.lock().unwrap().owed_errors);
            let objects = std::mem::take(&mut board.lock().unwrap().owed_error_objects);
            if owed.is_empty() && objects.is_empty() {
                json!({ "data": data }).to_string()
            } else {
                json!({"data": data,
                           "errors": owed.iter().map(|message| json!({"message": message}))
                               .chain(objects)
                               .collect::<Vec<_>>()})
                .to_string()
            }
        };
        // Answered, and not yet sent: a change made here is one the next request sees and
        // this one did not.
        let due: Vec<BoardHook> = {
            let mut held = board.lock().unwrap();
            let mut due = Vec::new();
            let mut kept = Vec::new();
            for mut hook in std::mem::take(&mut held.hooks) {
                if !(hook.matches)(query, &variables) {
                    kept.push(hook);
                } else if hook.skip == 0 {
                    due.push(hook);
                } else {
                    hook.skip -= 1;
                    kept.push(hook);
                }
            }
            held.hooks = kept;
            due
        };
        for hook in due {
            (hook.change)(&hooked);
        }
        let dropped = {
            let mut held = board.lock().unwrap();
            match held
                .dropping
                .iter()
                .position(|operation| query.contains(operation))
            {
                Some(at) => {
                    held.dropping.remove(at);
                    true
                }
                None => false,
            }
        };
        if dropped {
            // Applied, and never answered: the caller cannot know which.
            return None;
        }
        let date = board
            .lock()
            .unwrap()
            .served_at()
            .map(|at| format!("Date: {at}\r\n"))
            .unwrap_or_default();
        Some(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{date}Content-Length: {}\r\n\r\n{body}",
            body.len()
        ))
    });
    (
        github_projects_block_at(&endpoint),
        GitHubBoardFields {
            client: board_client(&endpoint),
            endpoint,
            board: watched,
            connections,
        },
    )
}

/// One block shared by the working, rate-limited and unreachable boards, so a failure
/// journey's board differs from the working one only in what that journey sets: the class
/// it asserts then comes from the endpoint's answer, never from a configuration drifting.
fn github_projects_block_at(endpoint: &str) -> Value {
    json!({
        "owner": "fixture-owner",
        "project_number": 7,
        "repository": "nickderobertis/onetaskgraph",
        "token_env": "GITHUB_PROJECTS_FIXTURE_TOKEN",
        "endpoint": endpoint,
        // `done` and `cancelled` keep their shipped defaults, which select the board's
        // `Done` and `Cancelled` options as well as closing the issue:
        // GitHub derives a project's `Sub-issues progress` from closed sub-issues, so a
        // plan whose finished tasks were only moved to a column reads 0% complete forever.
        "status_mapping": {"todo":"Todo","in-progress":"Doing"},
        // Every level keeps its shipped option name, which is what the board's own
        // `Priority` field below holds.
        "priority_mapping": {},
        // This board is a socket on loopback, not github.com, and it has no rate
        // limiter to be paced for. The shipped default spaces a content-creating
        // mutation every 750 ms so a copy cannot trip GitHub's secondary limit; left
        // on here it would buy nothing and would spend that per mutation on every
        // journey that writes. The pacing itself is proven where it is the subject, in
        // `crates/onetaskgraph-github-projects/tests/plugin.rs`.
        "pacing": {"min_mutation_interval_ms": 0}
    })
}

/// A board whose rate limiter refuses every request, asking for `retry_after` seconds
/// when it is given.
///
/// GitHub's primary limit answers `429 Too Many Requests`, with a `retry-after` when it
/// names a wait. The source is given no retry budget, so it reports the first refusal
/// rather than waiting it out: what is under test is what a caller is told, not how long
/// the source is prepared to wait before telling it.
pub fn github_projects_rate_limited(sandbox: &Sandbox, retry_after: Option<u64>) -> Value {
    github_projects_rate_limited_counted(sandbox, retry_after).0
}

/// [`github_projects_rate_limited`], and a count of the requests it was sent.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] This is `github_projects_rate_limited` above with its request count exposed, and that helper now delegates here: AGENTS.md puts every loopback server and its constructors in this shared harness, once, so no suite copies one.
pub fn github_projects_rate_limited_counted(
    sandbox: &Sandbox,
    retry_after: Option<u64>,
) -> (Value, Arc<Mutex<usize>>) {
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("GitHub fixture listener");
    let endpoint = format!(
        "http://{}/graphql",
        listener.local_addr().expect("fixture address")
    );
    let sent = Arc::new(Mutex::new(0_usize));
    let counted = Arc::clone(&sent);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("GitHub fixture connection");
            read_http_json(&mut stream);
            *counted.lock().unwrap() += 1;
            let body = json!({"message": "API rate limit exceeded for this fixture"}).to_string();
            let wait = retry_after
                .map(|seconds| format!("retry-after: {seconds}\r\n"))
                .unwrap_or_default();
            let response = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\n{wait}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("GitHub fixture response");
        }
    });
    let mut block = github_projects_block_at(&endpoint);
    block["pacing"]["retry_budget_ms"] = json!(0);
    (block, sent)
}

/// A board whose GraphQL budget for the hour is already spent, answering every request the
/// way GitHub does then: an HTTP 200 whose one error reads "API rate limit already exceeded
/// for user ID …", with no `type`, and the spent budget's reset `reset_in` seconds away.
pub fn github_projects_budget_spent(sandbox: &Sandbox, reset_in: u64) -> Value {
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("GitHub fixture listener");
    let endpoint = format!(
        "http://{}/graphql",
        listener.local_addr().expect("fixture address")
    );
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("GitHub fixture connection");
            read_http_json(&mut stream);
            let body = json!({"errors": [{
                "message": "API rate limit already exceeded for user ID 19440155."
            }]})
            .to_string();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock past the epoch")
                .as_secs();
            let reset = now + reset_in;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nx-ratelimit-remaining: 0\r\nx-ratelimit-reset: {reset}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("GitHub fixture response");
        }
    });
    let mut block = github_projects_block_at(&endpoint);
    block["pacing"]["retry_budget_ms"] = json!(0);
    block
}

/// A board at an address nothing listens on.
///
/// The port is bound and released again, so it is one this host just handed out and
/// nothing has had reason to take since: a connection to it is refused outright rather
/// than left waiting on a host that might answer.
pub fn github_projects_unreachable(sandbox: &Sandbox) -> Value {
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    let address = TcpListener::bind("127.0.0.1:0")
        .expect("GitHub fixture listener")
        .local_addr()
        .expect("fixture address");
    github_projects_block_at(&format!("http://{address}/graphql"))
}

/// The option list a whole-list `singleSelectOptions` input leaves a field holding: each
/// option sent with its id keeps it, and one sent without is minted a new one.
fn github_replaced_options(_old: &[Value], input: &Value, minted: &str) -> Vec<Value> {
    input["singleSelectOptions"]
        .as_array()
        .expect("the whole option list")
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let id = option["id"]
                .as_str()
                .map_or_else(|| format!("{minted}-{index}"), str::to_owned);
            json!({"id":id,"name":option["name"],"color":option["color"],
                   "description":option["description"]})
        })
        .collect()
}

/// The plain document a whole-issue twin is answered as, or `None` for any other document.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] Part of the loopback board's own request loop, which lives here for the reason `GitHubBoard` records: the shared journey table and the engine's suite drive this board too, so the board — and how it answers each document — cannot live in one plugin's suite.
fn github_plain_of(query: &str) -> Option<&'static str> {
    use onetaskgraph_github_projects::graphql;
    // Read off the plugin's own pairing, so a twin it adds is answered here without a second
    // list to keep in step.
    graphql::DOCUMENTS
        .iter()
        .map(|(document, _)| *document)
        .find(|plain| graphql::whole_twin(plain) == Some(query))
}

// llmlint: ignore[code_lands_in_the_domain_that_owns_it] How the loopback board answers a whole-issue document, which belongs beside the rest of its answers for the reason `GitHubBoard` records.
impl GitHubBoard {
    /// `data`, the answer a whole-issue twin's plain document gave, with the issue it wrote
    /// shown as a read of that issue by its own id shows it — or, while
    /// [`GitHubBoardFields::answer_writes_partially`] holds, only the parts of its change token.
    fn whole(&self, query: &str, mut data: Value, variables: &Value) -> Value {
        use onetaskgraph_github_projects::graphql;
        let first = json!(100);
        let (at, issue): (&str, Value) = if query == graphql::ADD_COMMENT_WHOLE {
            (
                "/addComment/subject",
                variables["input"]["subjectId"].clone(),
            )
        } else if query == graphql::UPDATE_COMMENT_WHOLE {
            let comment = variables["input"]["id"].clone();
            let issue = self
                .comments
                .iter()
                .find(|held| held["id"] == comment)
                .map_or(Value::Null, |held| held["issue"].clone());
            ("/updateIssueComment/issueComment/issue", issue)
        } else if query == graphql::UPDATE_ISSUE_WHOLE {
            ("/updateIssue/issue", variables["input"]["id"].clone())
        } else {
            let key = if query == graphql::UPDATE_FIELD_WHOLE {
                "updateProjectV2ItemFieldValue"
            } else {
                "clearProjectV2ItemFieldValue"
            };
            let item = variables["input"]["itemId"].clone();
            let issue = self
                .items
                .iter()
                .find(|held| held["item"] == item)
                .map_or(Value::Null, |held| held["id"].clone());
            let at: &'static str = if key == "updateProjectV2ItemFieldValue" {
                "/updateProjectV2ItemFieldValue/projectV2Item/content"
            } else {
                "/clearProjectV2ItemFieldValue/projectV2Item/content"
            };
            (at, issue)
        };
        let mut node = self.detail_node(&issue, &Value::Null, &first, true);
        if self.partial_writes && node.is_object() {
            let token = |key: &str| node.get(key).cloned().unwrap_or(Value::Null);
            let mut kept = json!({
                "__typename": token("__typename"),
                "id": token("id"),
                "updatedAt": token("updatedAt"),
                "commentCount": token("commentCount"),
                "relationshipEvents": token("relationshipEvents"),
                "repository": token("repository"),
                // What the token's facets are read from: the item's own kind — its title, its
                // parent, its sub-issues and the marker its body's slot may carry — and every
                // label, beside the board item's values below.
                "title": token("title"),
                "parent": token("parent"),
                "subIssuesSummary": token("subIssuesSummary"),
                "body": token("body"),
                "labels": token("labels"),
                "projectItems": {"nodes": node["projectItems"]["nodes"].as_array().map(|nodes| {
                    nodes.iter().map(|membership| json!({
                        "id": membership["id"],
                        "updatedAt": membership["updatedAt"],
                        "project": membership["project"],
                        // The values on the board item, which a field write is answered with.
                        "fieldValues": membership["fieldValues"],
                    })).collect::<Vec<_>>()
                })},
            });
            for written in ["title", "body", "state"] {
                if variables["input"].get(written).is_some()
                    || (written == "state" && variables["input"].get("stateInput").is_some())
                {
                    kept[written] = token(written);
                }
            }
            node = kept;
        }
        if let Some(members) = node.as_object_mut() {
            for member in &self.omitting {
                members.remove(*member);
            }
        }
        for connection in &self.truncating {
            if let Some(info) = node.pointer_mut(&format!("{connection}/pageInfo")) {
                info["hasNextPage"] = json!(true);
            }
        }
        let Some((parent, leaf)) = at.rsplit_once('/') else {
            return data;
        };
        if let Some(holder) = data.pointer_mut(parent).filter(|holder| holder.is_object()) {
            match holder.get_mut(leaf) {
                Some(existing) if existing.is_object() => {
                    if let (Some(existing), Some(node)) =
                        (existing.as_object_mut(), node.as_object())
                    {
                        for (key, value) in node {
                            existing.insert(key.clone(), value.clone());
                        }
                    }
                }
                _ => holder[leaf] = node,
            }
        }
        data
    }
}

fn github_answer(board: &Arc<Mutex<GitHubBoard>>, query: &str, variables: &Value) -> Value {
    board.lock().unwrap().settle_placements();
    // One document with its project and the project's members: the document answered exactly
    // as a read of it by its id is, and its parent exactly as a listing of that project's
    // sub-issues carries it.
    if query == onetaskgraph_github_projects::graphql::DOCUMENT_IN_PROJECT {
        let mut answer = github_answer(
            board,
            onetaskgraph_github_projects::graphql::ISSUE,
            variables,
        );
        let parent = answer
            .pointer("/node/parent/id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(parent) = parent {
            let mut asked = variables.clone();
            asked["id"] = json!(parent);
            asked["after"] = Value::Null;
            // The project and a page of its members, as that listing answers them, but with no
            // page of what blocks any of them: this read selects none.
            let listed = github_answer(
                board,
                "node(id:$id){__typename ...BoardIssue subIssues(first:$memberFirst)",
                &asked,
            );
            answer["node"]["project"] = listed["node"].clone();
        } else if answer["node"].is_object() {
            answer["node"]["project"] = Value::Null;
        }
        return answer;
    }
    // The batched origin lookup with a page of one project's members beside it: the lookup
    // answered exactly as on its own, and the members as GitHub lists a project's sub-issues,
    // trimmed to what the read selects of each.
    if query == onetaskgraph_github_projects::graphql::ORIGIN_LOOKUPS_WITH_MEMBERS {
        let mut answer = github_answer(
            board,
            onetaskgraph_github_projects::graphql::ORIGIN_LOOKUPS,
            variables,
        );
        let held = board.lock().unwrap();
        let project = &variables["project"];
        let first = usize::try_from(variables["memberFirst"].as_u64().expect("memberFirst"))
            .expect("memberFirst fits usize");
        let children: Vec<&Value> = held
            .items
            .iter()
            .chain(&held.pending)
            .filter(|item| item["parent"] == *project)
            .collect();
        let nodes: Vec<Value> = children
            .iter()
            .take(first)
            .map(|item| {
                let mut node = json!({"id": item["id"], "body": item["body"], "parent": {"id": project},
                       "repository": item["repo"].as_str().map(|repo| json!({"nameWithOwner": repo}))});
                for (id, field, value) in &held.members_page_misanswers {
                    if item["id"] == json!(id) {
                        node[field.key()] = value.clone();
                    }
                }
                node
            })
            .collect();
        answer["members"] = if held.items.iter().any(|item| item["id"] == *project) {
            json!({"subIssues": {"nodes": nodes,
                "pageInfo": {"hasNextPage": children.len() > first,
                             "endCursor": first.min(children.len()).to_string()}}})
        } else {
            Value::Null
        };
        return answer;
    }
    board.lock().unwrap().counted_types = TimelineEvent::counted_by(query);
    if query == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY {
        let mut held = board.lock().unwrap();
        if held.project_scope_withheld {
            held.owed_errors.push(
                "Your token has not been granted the required scopes to execute this query. \
                 The 'public' field requires one of the following scopes: ['read:project'], \
                 but your token has only been granted the: ['repo'] scopes."
                    .to_owned(),
            );
            return Value::Null;
        }
        held.project_visibility_reads += 1;
        if let Some(error) = held.project_visibility_error.clone() {
            held.owed_errors.push(error);
            return Value::Null;
        }
        if let Some(project) = &held.project_visibility_answer {
            return json!({"visibility": {"projectV2": project}});
        }
        let public = held.project_public
            || held
                .project_public_after_reads
                .is_some_and(|reads| held.project_visibility_reads > reads);
        return json!({"visibility": {"projectV2": {"public": public}}});
    }
    // A copy's batched reads and writes: each slot answered exactly as the single document it
    // aliases, so the board ends exactly as it would after those sent one at a time.
    {
        use onetaskgraph_github_projects::graphql as documents;
        if query == documents::ORIGIN_LOOKUPS {
            let mut result = serde_json::Map::new();
            {
                let held = board.lock().unwrap();
                assert_eq!(variables["owner"], "fixture-owner");
                assert_eq!(variables["number"], 7);
                assert_eq!(variables["repositoryOwner"], held.repository_owner());
                result.insert(
                    "boardFields".into(),
                    json!({"projectV2":{"id":"PVT-board","fields":held.fields()}}),
                );
                let id = if held.blank_repository_id {
                    ""
                } else {
                    "REPO-1"
                };
                result.insert(
                    "repository".into(),
                    json!({"id":id,"databaseId":123_456,
                           "nameWithOwner":held.repository}),
                );
            }
            for slot in 0..onetaskgraph_github_projects::ORIGIN_BATCH {
                if variables[format!("ask{slot}")] != true {
                    continue;
                }
                let one = json!({"type": variables["type"],
                    "filter": variables[format!("filter{slot}")],
                    "search": variables[format!("search{slot}")],
                    "originFirst": variables["lookupFirst"],
                    "itemsAfter": null, "searchAfter": null});
                let answer = board.lock().unwrap().origin_lookup(&one);
                result.insert(format!("o{slot}"), answer["originItems"].clone());
                result.insert(format!("s{slot}"), answer["search"].clone());
            }
            return Value::Object(result);
        }
        if query == documents::CREATED_BOARD_ITEMS {
            let mut held = board.lock().unwrap();
            let unfiled = held.unfiled_board_item_reads > 0;
            held.unfiled_board_item_reads = held.unfiled_board_item_reads.saturating_sub(1);
            let mut result = serde_json::Map::new();
            for slot in 0..onetaskgraph_github_projects::COPY_DETAIL_BATCH {
                if variables[format!("read{slot}")] != true {
                    continue;
                }
                let id = &variables[format!("id{slot}")];
                let answer = match held.items.iter().find(|item| item["id"] == *id) {
                    Some(_) if unfiled => json!({"id":id,"projectItems":{"nodes":[]}}),
                    Some(item) => json!({"id":id,"projectItems":{"nodes":[{
                        "id":item["item"],"project":{"id":"PVT-board"},
                        "status": item["status"].as_str().map(|name| json!({"name":name}))}]}}),
                    None if held.pending.iter().any(|item| item["id"] == *id) => {
                        json!({"id":id,"projectItems":{"nodes":[]}})
                    }
                    None => Value::Null,
                };
                result.insert(format!("i{slot}"), answer);
            }
            return Value::Object(result);
        }
        use onetaskgraph_github_projects::{DELETE_BATCH, MUTATION_BATCH, WRITE_BATCH};
        type Kind = (&'static str, &'static str, &'static str, &'static str);
        let create: Kind = ("create", "send", documents::CREATE_ISSUE, "createIssue");
        let field: Kind = (
            "field",
            "send",
            documents::UPDATE_FIELD,
            "updateProjectV2ItemFieldValue",
        );
        let close: Kind = ("close", "send", documents::UPDATE_ISSUE, "updateIssue");
        // llmlint: ignore[code_lands_in_the_domain_that_owns_it] The kind of write the aliased update batch carries, beside the close batch's it mirrors, in the loopback board's own table; the board lives in this shared harness for the reason `GitHubBoard` records.
        let update: Kind = ("update", "send", documents::UPDATE_ISSUE, "updateIssue");
        let block: Kind = ("block", "send", documents::ADD_BLOCKED_BY, "addBlockedBy");
        let delete: Kind = ("delete", "send", documents::DELETE_ISSUE, "deleteIssue");
        let batches: [(&str, Vec<Kind>, usize); 7] = [
            (documents::CREATE_ISSUES, vec![create], MUTATION_BATCH),
            (documents::WRITE_FIELD_VALUES, vec![field], WRITE_BATCH),
            (documents::CLOSE_ISSUES, vec![close], MUTATION_BATCH),
            // llmlint: ignore[code_lands_in_the_domain_that_owns_it] The row answering the aliased update batch, beside the close batch it mirrors, in the loopback board's own table; the board lives in this shared harness for the reason `GitHubBoard` records.
            (documents::UPDATE_ISSUES, vec![update], MUTATION_BATCH),
            (documents::ADD_BLOCKED_BYS, vec![block], WRITE_BATCH),
            (
                documents::WRITE_FIELDS_AND_EDGES,
                vec![
                    ("field", "sendField", field.2, field.3),
                    ("block", "sendBlock", block.2, block.3),
                ],
                WRITE_BATCH,
            ),
            (documents::DELETE_ISSUES, vec![delete], DELETE_BATCH),
        ];
        if let Some((_, kinds, slots)) = batches.iter().find(|(batched, ..)| *batched == query) {
            let mut result = serde_json::Map::new();
            for &(prefix, send, single, root) in kinds {
                for slot in 0..*slots {
                    if variables[format!("{send}{slot}")] != true {
                        continue;
                    }
                    let alias = format!("{prefix}{slot}");
                    let failing = {
                        let mut held = board.lock().unwrap();
                        let field = variables[&alias]["fieldId"].as_str();
                        let by_field = field.is_some() && held.failing_field.as_deref() == field;
                        let failing = held.failing_alias == Some(alias.as_str()) || by_field;
                        if by_field {
                            held.failing_field = None;
                        }
                        if failing {
                            held.failing_alias = None;
                            held.owed_errors.push(format!(
                                "Something went wrong while executing your query: {alias} failed"
                            ));
                        }
                        failing
                    };
                    if failing {
                        result.insert(alias, Value::Null);
                        continue;
                    }
                    let mut one = json!({"input": variables[&alias]});
                    if single == documents::UPDATE_FIELD {
                        one["readPriority"] = json!(false);
                        one["priorityName"] = json!("Priority");
                    }
                    let answer = github_answer(board, single, &one);
                    let bent = {
                        let mut held = board.lock().unwrap();
                        match held.bent_alias {
                            Some((bent, answer)) if bent == alias => {
                                held.bent_alias = None;
                                Some(answer)
                            }
                            _ => None,
                        }
                    };
                    let mut answered = answer[root].clone();
                    match bent {
                        Some(AliasAnswer::Missing) => continue,
                        Some(AliasAnswer::NamingAnotherBlocker) => {
                            answered["blockingIssue"]["id"] = json!("ANOTHER-ITEM");
                        }
                        Some(AliasAnswer::NamingAnother) => {
                            for member in answered
                                .as_object_mut()
                                .into_iter()
                                .flat_map(|m| m.values_mut())
                            {
                                if let Some(id) = member.get_mut("id") {
                                    *id = json!("ANOTHER-ITEM");
                                }
                            }
                        }
                        None => {}
                    }
                    result.insert(alias, answered);
                }
            }
            return Value::Object(result);
        }
    }
    if query == onetaskgraph_github_projects::graphql::UPDATE_FIELDS {
        let mut result = serde_json::Map::new();
        let writes = onetaskgraph_github_projects::FIELD_WRITE_SLOTS.map(|slot| {
            (
                slot,
                onetaskgraph_github_projects::graphql::UPDATE_FIELD,
                "updateProjectV2ItemFieldValue",
            )
        });
        let clears = onetaskgraph_github_projects::FIELD_CLEAR_SLOTS.map(|slot| {
            (
                slot,
                onetaskgraph_github_projects::graphql::CLEAR_FIELD,
                "clearProjectV2ItemFieldValue",
            )
        });
        for (slot, document, key) in writes.into_iter().chain(clears) {
            let (alias, variable, enabled) =
                (slot.alias, slot.variable, variables[slot.include] == true);
            if enabled {
                let failing = {
                    let mut held = board.lock().unwrap();
                    let field = variables[variable]["fieldId"].as_str();
                    let by_field = field.is_some() && held.failing_field.as_deref() == field;
                    let failing = held.failing_alias == Some(alias) || by_field;
                    if by_field {
                        held.failing_field = None;
                    }
                    if failing && !by_field {
                        held.failing_alias = None;
                    }
                    if failing {
                        held.owed_errors.push(format!(
                            "Something went wrong while executing your query: {alias} failed"
                        ));
                    }
                    failing
                };
                if failing {
                    result.insert(alias.to_owned(), Value::Null);
                    continue;
                }
                let answer = github_answer(board, document, &json!({"input":variables[variable]}));
                result.insert(alias.to_owned(), answer[key].clone());
            }
        }
        return Value::Object(result);
    }
    let mut board = board.lock().unwrap();
    let input = variables.get("input").cloned().unwrap_or(Value::Null);
    if query.contains("updateProjectV2Field(input:$input)") && input["fieldId"] == "FIELD-priority"
    {
        assert_eq!(input["projectId"], "PVT-board");
        let old = board
            .priority_options
            .clone()
            .expect("an update of the Priority field names a field this board has");
        let mut next = github_replaced_options(&old, &input, "OPT-p-new");
        if board.omits_added_priority_option {
            board.omits_added_priority_option = false;
            next.retain(|option| old.iter().any(|held| held["id"] == option["id"]));
        }
        if board.remints_after_priority_update {
            board.remints_after_priority_update = false;
            next[0]["id"] = json!("OPT-p-reminted");
        }
        // As for Status below: an item assigned an option id the update did not send back
        // loses its value.
        for item in &mut board.items {
            let Some(name) = item["priority"].as_str() else {
                continue;
            };
            let old_id = old
                .iter()
                .find(|option| option["name"] == name)
                .map(|option| option["id"].clone());
            if old_id.is_some_and(|id| !next.iter().any(|option| option["id"] == id)) {
                item["priority"] = Value::Null;
            }
        }
        board.priority_options = Some(next.clone());
        if board.hides_after_update {
            board.status_board_accessible = false;
        }
        if board.drift_after_priority_update {
            board.drift_after_priority_update = false;
            if let Some(item) = board
                .items
                .iter_mut()
                .find(|item| item["priority"].is_string())
            {
                item["priority"] = Value::Null;
            }
        }
        return json!({"updateProjectV2Field":{"projectV2Field":{"id":"FIELD-priority",
            "options":next}}});
    }
    if query.contains("createProjectV2Field(input:$input)") && input["dataType"] == "TEXT" {
        assert_eq!(input["projectId"], "PVT-board");
        let name = input["name"].as_str().expect("a field name");
        assert!(
            !board.fields()["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field["name"] == name),
            "createProjectV2Field names a field this board already has"
        );
        let field = json!({"__typename":"ProjectV2Field","id":format!("FIELD-text-{name}"),
            "name":name,"dataType":"TEXT"});
        if std::mem::take(&mut board.omits_created_text_field) {
            return json!({"createProjectV2Field":{"projectV2Field":{}}});
        }
        board.text_fields.push(field);
        return json!({"createProjectV2Field":{"projectV2Field":{}}});
    }
    if query.contains("createProjectV2Field(input:$input)") {
        assert_eq!(input["projectId"], "PVT-board");
        assert_eq!(input["dataType"], "SINGLE_SELECT");
        assert_eq!(
            input["name"], "Priority",
            "only a Priority field is ever created"
        );
        assert!(
            board.priority_options.is_none(),
            "createProjectV2Field names a field this board already has"
        );
        let created = github_replaced_options(&[], &input, "OPT-p-new");
        board.priority_options = Some(created.clone());
        return json!({"createProjectV2Field":{"projectV2Field":{"id":"FIELD-priority",
            "name":"Priority","options":created}}});
    }
    if query.contains("clearProjectV2ItemFieldValue(input:$input)")
        && board
            .text_fields
            .iter()
            .any(|field| field["id"] == input["fieldId"])
    {
        assert_eq!(input["projectId"], "PVT-board");
        let item_id = input["itemId"].clone();
        let field = input["fieldId"].as_str().unwrap().to_owned();
        let held = board
            .items
            .iter_mut()
            .find(|item| item["item"] == item_id)
            .expect("a field clear names a board item");
        if let Some(texts) = held["texts"].as_object_mut() {
            texts.remove(&field);
        }
        board.touch_item(&item_id);
        return json!({"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id,"fieldValueByName":null}}});
    }
    if query.contains("clearProjectV2ItemFieldValue(input:$input)") {
        assert_eq!(input["projectId"], "PVT-board");
        assert_eq!(
            input["fieldId"], "FIELD-priority",
            "only a priority is ever cleared"
        );
        let item_id = input["itemId"].clone();
        let held = board
            .items
            .iter_mut()
            .find(|item| item["item"] == item_id)
            .expect("a field clear names a board item");
        held["priority"] = Value::Null;
        board.touch_item(&item_id);
        return json!({"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id,"fieldValueByName":null}}});
    }
    if query.contains("updateProjectV2Field(input:$input)") {
        assert_eq!(input["projectId"], "PVT-board");
        assert_eq!(input["fieldId"], "FIELD-status");
        let old = board.status_options.clone();
        let sent = input["singleSelectOptions"]
            .as_array()
            .expect("the whole option list");
        let mut next = Vec::new();
        for (index, option) in sent.iter().enumerate() {
            if board.omit_added_status_option && option["id"].is_null() {
                continue;
            }
            let id = option["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("OPT-new-{index}"));
            next.push(
                json!({"id":id,"name":option["name"],"color":option["color"],
                "description":option["description"]}),
            );
        }
        // GitHub re-mints an option sent without its id; items assigned to the old id lose
        // their status. This is the dangerous whole-list behavior the real mutation has.
        for item in &mut board.items {
            let Some(name) = item["status"].as_str() else {
                continue;
            };
            let old_id = old
                .iter()
                .find(|option| option["name"] == name)
                .map(|option| option["id"].clone());
            if old_id.is_some_and(|id| !next.iter().any(|option| option["id"] == id)) {
                item["status"] = Value::Null;
            }
        }
        board.status_options = next;
        board.omit_added_status_option = false;
        if board.remint_after_status_update {
            board.remint_after_status_update = false;
            board.status_options[0]["id"] = json!("OPT-reminted");
        }
        if board.drift_after_status_update {
            board.drift_after_status_update = false;
            if let Some(item) = board
                .items
                .iter_mut()
                .find(|item| item["status"].is_string())
            {
                item["status"] = Value::Null;
            }
        }
        return json!({"updateProjectV2Field":{"projectV2Field":{"id":"FIELD-status",
            "options":board.options()}}});
    }
    if query.contains("optionId field{") && query.contains("fields(first:$nestedFirst)") {
        assert_eq!(variables["owner"], "fixture-owner");
        assert_eq!(variables["number"], 7);
        if !board.status_board_accessible {
            return json!({"owner":{"projectV2":null}});
        }
        let offset = variables["after"]
            .as_str()
            .map_or(0, |cursor| cursor.parse().unwrap());
        let page_size = board.status_snapshot_page_size.unwrap_or(board.items.len());
        let end = (offset + page_size).min(board.items.len());
        let nodes = board.items[offset..end]
            .iter()
            .map(|item| {
                let mut values = item["status"].as_str().map_or_else(Vec::new, |name| {
                    let option = board
                        .status_options
                        .iter()
                        .find(|option| option["name"] == name)
                        .expect("an assigned option exists");
                    vec![json!({"name":name,"optionId":option["id"],
                    "field":{"id":"FIELD-status","name":"Status"}})]
                });
                // A person's own field's value on the first item, as GitHub answers it — the
                // setup reads past it.
                if let Some(own) = board.persons_fields.first()
                    && own["__typename"] == "ProjectV2SingleSelectField"
                    && item["id"] == "T-1"
                {
                    values.push(json!({"name":own["options"][0]["name"],
                        "optionId":own["options"][0]["id"],
                        "field":{"id":own["id"],"name":own["name"]}}));
                }
                if let (Some(name), Some(options)) =
                    (item["priority"].as_str(), &board.priority_options)
                {
                    let option = options
                        .iter()
                        .find(|option| option["name"] == name)
                        .expect("an assigned priority option exists");
                    values.push(json!({"name":name,"optionId":option["id"],
                    "field":{"id":"FIELD-priority","name":"Priority"}}));
                }
                if let Some(field) = board.repeated_snapshot_value
                    && item["id"] == "T-1"
                {
                    let (id, options) = if field == "Status" {
                        ("FIELD-status", Some(&board.status_options))
                    } else {
                        ("FIELD-priority", board.priority_options.as_ref())
                    };
                    let last = options
                        .and_then(|options| options.last())
                        .expect("the repeated field has an option");
                    values.push(json!({"name":last["name"],"optionId":last["id"],
                    "field":{"id":id,"name":field}}));
                }
                json!({"id":item["item"],"fieldValues":{"nodes":values,
                "pageInfo":{"hasNextPage":false}}})
            })
            .collect::<Vec<_>>();
        // The snapshot selects single-select fields alone, so GitHub answers every other field
        // as an empty node.
        let mut fields = board.fields();
        for field in fields["nodes"].as_array_mut().expect("field nodes") {
            if field["__typename"] != "ProjectV2SingleSelectField" {
                *field = json!({});
            }
        }
        return json!({"owner":{"projectV2":{"id":"PVT-board","fields":fields,
            "items":{"nodes":nodes,"pageInfo":{"hasNextPage":end < board.items.len(),
                "endCursor":end.to_string()}}}}});
    }
    if query.contains("addComment(input:$input)") {
        let subject = input["subjectId"]
            .as_str()
            .expect("addComment names its subject")
            .to_owned();
        assert!(
            board
                .items
                .iter()
                .any(|item| item["id"] == json!(subject) && !GitHubBoard::is_draft(item)),
            "addComment names {subject}, which is no issue on this board"
        );
        let body = input["body"].as_str().expect("addComment carries a body");
        assert!(
            !body.is_empty(),
            "GitHub refuses a comment with an empty body"
        );
        let added = board.comment(&subject, body);
        return json!({"addComment":{"subject":{"id":subject},
                                    "commentEdge":{"node":GitHubBoard::comment_node(&added)}}});
    }
    if query.contains("updateIssueComment(input:$input)") {
        let at = board.tick();
        let held = board
            .comments
            .iter_mut()
            .find(|held| held["id"] == input["id"])
            .unwrap_or_else(|| {
                panic!("updateIssueComment names no comment this board holds: {input}")
            });
        held["body"] = input["body"].clone();
        held["updatedAt"] = json!(at);
        let (issue, edited) = (held["issue"].clone(), GitHubBoard::comment_node(held));
        board.touch_issue(&issue);
        return json!({"updateIssueComment":{"issueComment":edited}});
    }
    if query.contains("deleteIssueComment(input:$input)") {
        assert!(
            board.comments.iter().any(|held| held["id"] == input["id"]),
            "deleteIssueComment names no comment this board holds: {input}"
        );
        board.comments.retain(|held| held["id"] != input["id"]);
        return json!({"deleteIssueComment":{"clientMutationId":null}});
    }
    if query.contains("on IssueComment{id body issue{id}}") {
        let id = variables["id"].clone();
        if let Some(held) = board.comments.iter().find(|held| held["id"] == id) {
            return json!({"node":{"__typename":"IssueComment","id":id,"body":held["body"],
                                  "issue":{"id":held["issue"]}}});
        }
        return match board.items.iter().find(|item| item["id"] == id) {
            Some(item) => json!({"node":{"__typename":board.content(item)["__typename"]}}),
            None => json!({ "node": null }),
        };
    }
    // A batch of items by their own ids, each answered as the one-item detail read answers
    // it — its comments riding along only when the batch asked for them.
    if query == onetaskgraph_github_projects::graphql::ISSUE_DETAILS {
        let mut answered = serde_json::Map::new();
        for slot in 0..onetaskgraph_github_projects::DETAIL_BATCH {
            let id = variables[format!("id{slot}")].clone();
            assert!(
                id.is_string(),
                "every slot of a batch names an id: {variables}"
            );
            answered.insert(
                format!("i{slot}"),
                board.detail_node(
                    &id,
                    &Value::Null,
                    &variables["first"],
                    variables["comments"] == json!(true),
                ),
            );
        }
        return Value::Object(answered);
    }
    // A copy's batch: each item with the first page of its comments and its `blockedBy`.
    if query == onetaskgraph_github_projects::graphql::ISSUE_COPY_DETAILS {
        let mut answered = serde_json::Map::new();
        for slot in 0..onetaskgraph_github_projects::COPY_DETAIL_BATCH {
            let id = variables[format!("id{slot}")].clone();
            assert!(
                id.is_string(),
                "every slot of a batch names an id: {variables}"
            );
            let mut node = board.detail_node(&id, &Value::Null, &variables["first"], true);
            if node.get("__typename") == Some(&json!("Issue")) {
                node["blockedBy"] = json!({
                    "nodes": board.related(id.as_str().expect("an id"), false),
                    "pageInfo": {"hasNextPage": false, "endCursor": null},
                });
            }
            answered.insert(format!("i{slot}"), node);
        }
        // The sub-issues of the issue `$project` names, each its id and title, when asked.
        if variables["children"] == true {
            let project = variables["project"].clone();
            let visible = board.items.len().saturating_sub(board.lagging_reads);
            let children: Vec<Value> = board.items[..visible]
                .iter()
                .filter(|item| item["parent"] == project && !GitHubBoard::is_draft(item))
                .map(|item| json!({"id": item["id"], "title": item["title"]}))
                .collect();
            let held = board.items.iter().any(|item| item["id"] == project);
            let mut sub_issues = json!({"nodes": children, "pageInfo": {"hasNextPage": false}});
            if std::mem::take(&mut board.omits_sub_issue_list) {
                sub_issues
                    .as_object_mut()
                    .expect("an object")
                    .remove("nodes");
            }
            answered.insert(
                "children".into(),
                if held {
                    json!({"id": project, "subIssues": sub_issues})
                } else {
                    Value::Null
                },
            );
        }
        return Value::Object(answered);
    }
    // One item by its own id with a page of its comments, in one request.
    if query == onetaskgraph_github_projects::graphql::ISSUE_DETAIL {
        let id = variables["id"].clone();
        return json!({"node": board.detail_node(&id, &variables["after"], &variables["first"], true)});
    }
    if query.contains("comments(first:$first,after:$after)") {
        let id = variables["id"].clone();
        let Some(item) = board.items.iter().find(|item| item["id"] == id) else {
            return json!({ "node": null });
        };
        if GitHubBoard::is_draft(item) {
            return json!({"node":{"__typename":"DraftIssue"}});
        }
        let comments = board.comment_connection(&id, &variables["after"], &variables["first"]);
        return json!({"node":{"__typename":"Issue","comments":comments}});
    }
    if query.contains("repository(owner:$owner,name:$name)") {
        assert_eq!(variables["owner"], board.repository_owner());
        return json!({"repository":{"id":"REPO-1","nameWithOwner":board.repository}});
    }
    if query.contains("createIssue(input:$input)") {
        assert_eq!(input["repositoryId"], "REPO-1");
        assert!(input["title"].as_str().is_some_and(|t| !t.is_empty()));
        assert!(input["body"].is_string() || input["body"].is_null());
        board.created += 1;
        let id = format!("ISSUE-{}", board.created);
        let at = board.tick();
        let mut created = json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue",
            "title":input["title"],"body":input["body"],"state":"OPEN","reason":null,
            "parent":Value::Null,"repo":board.repository,"status":"Todo",
            "origin":"","labels":[],"updatedAt":at});
        if board.files_without_status {
            created["status"] = Value::Null;
        }
        // `parentIssueId` files the new issue under its parent, as `addSubIssue` would.
        if let Some(parent) = input.get("parentIssueId").and_then(Value::as_str) {
            created["parent"] = json!(parent);
            board.record_relationship_event(
                (id.as_str(), parent),
                TimelineEvent::membership(RelationshipChange::made_by(true)),
            );
        }
        // `projectV2Ids` naming this board files the issue on it: GitHub adds the board item
        // after answering the create, so the answer names none, and a later read finds it —
        // filed with the `Status` this board gives a new item, if any.
        match input.get("projectV2Ids") {
            Some(boards) => {
                assert_eq!(
                    boards,
                    &json!(["PVT-board"]),
                    "a create naming another board"
                );
                if let Some(placement) = &mut board.placement {
                    let delay = placement.delays[placement.next % placement.delays.len()];
                    placement.next += 1;
                    placement.waiting.push((json!(id), delay, None));
                    board.pending.push(created);
                } else {
                    created["itemUpdatedAt"] = json!(at);
                    board.items.push(created);
                }
            }
            None => board.pending.push(created),
        }
        // GitHub answers the creating mutation with the issue's own web address and its
        // number, which is the only place a run learns where an item it just created is,
        // and what that item's short handle is, before this board's read catches up.
        return json!({"createIssue":{"issue":{"id":id,"number":github_number(&id),
            "url":format!("https://example.invalid/{id}")}}});
    }
    if query.contains("addProjectV2ItemById(input:$input)") {
        assert_eq!(input["projectId"], "PVT-board");
        let content = input["contentId"].clone();
        // GitHub files an issue once: one its create has placed already is refused rather than
        // answered with the item it holds.
        if board.items.iter().any(|item| item["id"] == content) {
            board
                .owed_errors
                .push("Content already exists in this project".to_owned());
            return json!({"addProjectV2ItemById": null});
        }
        if let Some(placement) = &mut board.placement {
            placement.waiting.retain(|(id, ..)| *id != content);
        }
        let at = board
            .pending
            .iter()
            .position(|item| item["id"] == content)
            .expect("the issue was created before it was filed");
        let mut item = board.pending.remove(at);
        let id = item["item"].clone();
        item["itemUpdatedAt"] = json!(board.tick());
        board.items.push(item);
        return json!({"addProjectV2ItemById":{"item":{"id":id}}});
    }
    if query.contains("updateIssue(input:$input)") {
        let held = board.find(&input["id"]);
        if let Some(title) = input["title"].as_str() {
            held["title"] = json!(title);
        }
        // A body arrives without a title when only the metadata slot changes, and it is the
        // whole new body either way.
        if input.get("body").is_some_and(|body| !body.is_null()) {
            held["body"] = input["body"].clone();
        }
        let state = input["stateInput"].clone();
        if !state.is_null() {
            held["state"] = state["value"].clone();
            held["reason"] = state["stateReason"].clone();
        }
        board.touch_issue(&input["id"]);
        return json!({"updateIssue":{"issue":{"id":input["id"]}}});
    }
    if query.contains("updateProjectV2ItemFieldValue(input:$input)")
        && input["projectId"] == json!(format!("PVT-board{GITHUB_SECOND_BOARD}"))
    {
        // A card on the second board: only its `Status` is ever written there.
        assert_eq!(input["fieldId"], "FIELD-status");
        let id = input["value"]["singleSelectOptionId"]
            .as_str()
            .expect("a Status option id");
        let Some(option) = board
            .status_options
            .iter()
            .find(|option| option["id"] == id)
            .map(|option| option["name"].clone())
        else {
            board.owed_errors.push(format!(
                "The single select option Id does not belong to the field: {id}"
            ));
            return json!({ "updateProjectV2ItemFieldValue": null });
        };
        let at = board.tick();
        let card = input["itemId"].clone();
        let held = board
            .card_on(GITHUB_SECOND_BOARD, &card)
            .expect("a field update names a card on the second board");
        held["status"] = option;
        held["itemUpdatedAt"] = json!(at);
        return json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":card,
            "fieldValueByName":null}}});
    }
    if query.contains("updateProjectV2ItemFieldValue(input:$input)") {
        assert_eq!(input["projectId"], "PVT-board");
        assert!(
            input["value"]
                .as_object()
                .is_some_and(|value| value.len() == 1)
        );
        let item_id = input["itemId"].clone();
        let priority_field = input["fieldId"] == "FIELD-priority";
        let drops_priority_writes = board.drops_priority_writes;
        let options = if priority_field {
            Value::Array(
                board
                    .priority_options
                    .clone()
                    .expect("a Priority write names a field this board has"),
            )
        } else {
            board.options()
        };
        let option = match input["value"]["singleSelectOptionId"].as_str() {
            Some(id) => match options
                .as_array()
                .unwrap()
                .iter()
                .find(|option| option["id"] == id)
            {
                Some(option) => Some(option["name"].clone()),
                // An option id the field no longer has — deleted, or re-minted by a rename —
                // is refused as GitHub refuses it, and nothing is written.
                None => {
                    board.owed_errors.push(format!(
                        "The single select option Id does not belong to the field: {id}"
                    ));
                    return json!({ "updateProjectV2ItemFieldValue": null });
                }
            },
            None => None,
        };
        let text = input["value"]["text"].clone();
        let projected = board
            .text_fields
            .iter()
            .any(|field| field["id"] == input["fieldId"]);
        let held = board
            .items
            .iter_mut()
            .find(|item| item["item"] == item_id)
            .expect("a field update names a board item");
        if projected {
            if !held["texts"].is_object() {
                held["texts"] = json!({});
            }
            held["texts"][input["fieldId"].as_str().unwrap()] = text;
            board.touch_item(&item_id);
            return json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id,"fieldValueByName":null}}});
        }
        match option {
            Some(_) if priority_field && drops_priority_writes => {}
            Some(option) if priority_field => held["priority"] = option,
            Some(option) => held["status"] = option,
            None => {}
        }
        if text.is_string() {
            held["origin"] = text;
        }
        board.touch_item(&item_id);
        let held = board
            .items
            .iter()
            .find(|item| item["item"] == item_id)
            .expect("the item just written")
            .clone();
        if variables["readPriority"] == true
            && let Some(response) = board.priority_response.take()
        {
            return json!({"updateProjectV2ItemFieldValue":{"projectV2Item":response}});
        }
        let rendered = board.rendered(&held);
        let priority = rendered["fieldValues"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["field"]["name"] == "Priority")
            .cloned()
            .unwrap_or(Value::Null);
        return json!({"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":item_id,"fieldValueByName":priority}}});
    }
    if query.contains("addSubIssue(input:$input)") || query.contains("removeSubIssue(input:$input)")
    {
        let adding = query.contains("addSubIssue(input:$input)");
        let parent = input["issueId"].clone();
        let child = input["subIssueId"].clone();
        board.find(&child)["parent"] = if adding { parent.clone() } else { Value::Null };
        board.record_relationship_event(
            (
                child.as_str().expect("a sub-issue id"),
                parent.as_str().expect("a parent id"),
            ),
            TimelineEvent::membership(RelationshipChange::made_by(adding)),
        );
        let root = if adding {
            "addSubIssue"
        } else {
            "removeSubIssue"
        };
        return json!({root:{"issue":{"id":parent},"subIssue":{"id":child}}});
    }
    if query.contains("deleteIssue(input:$input)") {
        let id = input["issueId"].clone();
        board.items.retain(|item| item["id"] != id);
        board.pending.retain(|item| item["id"] != id);
        // An issue's comments go with it on GitHub.
        board.comments.retain(|held| held["issue"] != id);
        let id = id.as_str().expect("an issue id").to_owned();
        board.blocked_by.retain(|(near, _)| *near != id);
        for (_, blockers) in &mut board.blocked_by {
            blockers.retain(|blocker| blocker != &id);
        }
        return json!({"deleteIssue":{"repository":{"id":"REPO-1"}}});
    }
    if query.contains("addBlockedBy(input:$input)")
        || query.contains("removeBlockedBy(input:$input)")
    {
        let adding = query.contains("addBlockedBy(input:$input)");
        let issue = input["issueId"].as_str().expect("an issue id").to_owned();
        let blocker = input["blockingIssueId"]
            .as_str()
            .expect("a blocking issue id")
            .to_owned();
        let entry = match board.blocked_by.iter().position(|(near, _)| *near == issue) {
            Some(at) => &mut board.blocked_by[at].1,
            None => {
                board.blocked_by.push((issue.clone(), Vec::new()));
                &mut board.blocked_by.last_mut().unwrap().1
            }
        };
        if adding {
            entry.push(blocker.clone());
        } else {
            entry.retain(|held| held != &blocker);
        }
        // The relationship is the dependent issue's own: GitHub lists it on that issue, and
        // records it on both ends' timelines without moving either's time.
        board.record_relationship_event(
            (&issue, &blocker),
            TimelineEvent::dependency(RelationshipChange::made_by(adding)),
        );
        let root = if adding {
            "addBlockedBy"
        } else {
            "removeBlockedBy"
        };
        return json!({root:{"issue":{"id":issue},"blockingIssue":{"id":blocker}}});
    }
    if query.contains("updateProjectV2(input:$input)") {
        for field in ["title", "shortDescription", "readme"] {
            if let Some(value) = input.get(field) {
                board.own[field] = value.clone();
            }
        }
        return json!({"updateProjectV2":{"projectV2":{"id":"PVT-board"}}});
    }
    if query.contains("... on ProjectV2{title shortDescription readme}") {
        assert_eq!(variables["id"], "PVT-board");
        return json!({ "node": board.own.clone() });
    }
    if query.contains("originItems:repositoryOwner(") {
        return board.origin_lookup(variables);
    }
    if query.contains("search(query:$search") {
        assert_eq!(variables["type"], "ISSUE");
        let search = variables["search"].as_str().expect("a search query");
        let wanted = search
            .strip_prefix("project:fixture-owner/7 is:issue")
            .unwrap_or_else(|| panic!("a search scoped to the configured board: {search}"));
        // The server side of every qualifier the source sends — `in:title "..."`, which is what
        // a project named by name is discovered through, and the phrases a text, metadata or
        // origin read asks for — matched by token, the way GitHub matches them.
        let wanted = IssueSearch::parse(wanted);
        let offset = match &variables["after"] {
            Value::Null => 0,
            Value::String(cursor) => cursor.parse::<usize>().expect("numeric after cursor"),
            other => panic!("GraphQL after must be null or a numeric string: {other}"),
        };
        let first = usize::try_from(
            variables["first"]
                .as_u64()
                .expect("GraphQL first must be an unsigned integer"),
        )
        .expect("GraphQL first fits usize");
        assert!(first > 0, "GraphQL first must be positive");
        let first = board.search_page_rows.map_or(first, |rows| first.min(rows));
        // GitHub's issue search is an index and is behind what the board really holds, the
        // same way its board read is; `lagging_reads` is how far.
        let visible = board.items.len().saturating_sub(board.lagging_reads);
        let matched = board.items[..visible]
            .iter()
            // An issue search returns issues, and a draft is not one; a search scoped to this
            // board returns only the issues it holds a card for.
            .filter(|item| !GitHubBoard::is_draft(item) && GitHubBoard::on_board(item))
            .filter(|item| {
                wanted.admits(
                    item["title"].as_str().unwrap_or_default(),
                    item["body"].as_str().unwrap_or_default(),
                ) && wanted.recent(&item["updatedAt"])
                    && wanted.state.is_none_or(|state| item["state"] == state)
            })
            .cloned()
            .collect::<Vec<_>>();
        let end = (offset + first).min(matched.len());
        let nodes = matched[offset.min(end)..end]
            .iter()
            .map(|item| board.as_issue(item))
            .collect::<Vec<_>>();
        return json!({"search":{"nodes":nodes,
            "pageInfo":{"hasNextPage":end < matched.len(),"endCursor":end.to_string()}}});
    }
    if query.contains("subIssues(first:$memberFirst") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = board.items.iter().find(|item| item["id"] == json!(id)) else {
            return json!({ "node": null });
        };
        // A draft has no sub-issues to select, so the `... on Issue` arm selects nothing.
        if GitHubBoard::is_draft(item) {
            return json!({"node":{"__typename":"DraftIssue"}});
        }
        let offset = match &variables["after"] {
            Value::Null => 0,
            Value::String(cursor) => cursor.parse::<usize>().expect("numeric after cursor"),
            other => panic!("GraphQL after must be null or a numeric string: {other}"),
        };
        let first = usize::try_from(
            variables["memberFirst"]
                .as_u64()
                .expect("GraphQL memberFirst must be an unsigned integer"),
        )
        .expect("GraphQL memberFirst fits usize");
        let parent = json!(id);
        let children = board
            .items
            .iter()
            .filter(|item| item["parent"] == parent)
            .collect::<Vec<_>>();
        let end = (offset + first).min(children.len());
        // What blocks each sub-issue, when the read asks for it beside the issue: a page of
        // `$nestedFirst`, saying whether more follow, as GitHub pages a nested connection.
        let blocked_first = query.contains("blockedBy(first:$nestedFirst)").then(|| {
            usize::try_from(variables["nestedFirst"].as_u64().expect("nestedFirst"))
                .expect("nestedFirst fits usize")
        });
        let nodes = children[offset.min(end)..end]
            .iter()
            .map(|item| {
                let mut issue = board.as_issue(item);
                if let Some(page) = blocked_first.filter(|_| !board.members_without_blocked_by) {
                    let all = board.related(item["id"].as_str().expect("an id"), false);
                    let all = all.as_array().expect("related issues");
                    issue["blockedBy"] = json!({
                        "nodes": all.iter().take(page).collect::<Vec<_>>(),
                        "pageInfo": {"hasNextPage": all.len() > page,
                                     "endCursor": page.min(all.len()).to_string()}});
                }
                issue
            })
            .collect::<Vec<_>>();
        // The project issue itself, with what blocks it, when the read selects it beside its
        // sub-issues.
        let mut node = json!({"__typename":"Issue"});
        if query.contains("node(id:$id){__typename ...BoardIssue") {
            let parent = board
                .items
                .iter()
                .find(|item| item["id"] == json!(id))
                .cloned()
                .expect("the project read");
            node = board.as_issue(&parent);
            if let Some(page) = blocked_first {
                let all = board.related(&id, false);
                let all = all.as_array().expect("related issues");
                node["blockedBy"] = json!({
                    "nodes": all.iter().take(page).collect::<Vec<_>>(),
                    "pageInfo": {"hasNextPage": all.len() > page,
                                 "endCursor": page.min(all.len()).to_string()}});
            }
        }
        node["subIssues"] = json!({"nodes":nodes,
            "pageInfo":{"hasNextPage":end < children.len(),"endCursor":end.to_string()}});
        return json!({ "node": node });
    }
    // The board's own id and fields with none of its items: what a write reads when the
    // item it writes does not carry them.
    if query == onetaskgraph_github_projects::graphql::CREATION_CONTEXT {
        assert_eq!(variables["owner"], "fixture-owner");
        assert_eq!(variables["number"], 7);
        assert_eq!(variables["repositoryOwner"], board.repository_owner());
        return json!({"boardFields":{"projectV2":{"id":"PVT-board","fields":board.fields()}},
                      "repository":{"id":"REPO-1","nameWithOwner":board.repository}});
    }
    if query.contains("boardFields:repositoryOwner") {
        assert_eq!(variables["owner"], "fixture-owner");
        let id = match variables["number"].as_u64() {
            Some(7) => "PVT-board".to_owned(),
            Some(GITHUB_SECOND_BOARD) => format!("PVT-board{GITHUB_SECOND_BOARD}"),
            other => panic!("this server holds boards 7 and {GITHUB_SECOND_BOARD}, not {other:?}"),
        };
        return json!({"boardFields":{"projectV2":{"id":id,"fields":board.fields()}}});
    }
    if query == onetaskgraph_github_projects::graphql::CLOSED_ITEMS {
        return board.closed_items(variables);
    }
    if query == onetaskgraph_github_projects::graphql::ISSUES_BY_REFERENCE {
        return board.issues_by_reference(variables);
    }
    if query == onetaskgraph_github_projects::graphql::ISSUE_BOARD_ITEMS {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = board.items.iter().find(|item| item["id"] == json!(id)) else {
            return json!({ "node": null });
        };
        let offset = variables["after"].as_str().map_or(0, |cursor| {
            cursor.parse::<usize>().expect("numeric after cursor")
        });
        let first = usize::try_from(variables["first"].as_u64().expect("GraphQL first"))
            .expect("GraphQL first fits usize");
        let page = GitHubBoard::membership_page(board.memberships(item), offset, first);
        return json!({"node":{"projectItems":page}});
    }
    // llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] How the loopback board answers the two one-direction dependency documents, which its request loop serves from its private state; it lives beside `GitHubBoard` for the reason that type records, and a GitHub suite could not add an answer from outside it.
    // A dependency read of one direction, answering exactly what its document selects: what
    // an item is blocked by, with its body — a draft's body with the one board item it is,
    // beside this board's own id — or what an issue blocks.
    {
        use onetaskgraph_github_projects::graphql::{ISSUE_DEPENDED_ON_BY, ISSUE_DEPENDS_ON};
        if query == ISSUE_DEPENDS_ON || query == ISSUE_DEPENDED_ON_BY {
            let forward = query == ISSUE_DEPENDS_ON;
            let id = variables["id"].as_str().expect("dependency id").to_owned();
            let first = variables["first"]
                .as_u64()
                .expect("dependency first must be an unsigned integer");
            assert!(
                (1..=100).contains(&first),
                "dependency first is out of range"
            );
            let board_root = json!({"projectV2":{"id":"PVT-board"}});
            let Some(near) = board.items.iter().find(|item| item["id"] == json!(id)) else {
                return if forward {
                    json!({"node":null,"board":board_root})
                } else {
                    json!({ "node": null })
                };
            };
            if GitHubBoard::is_draft(near) {
                if !forward {
                    return json!({"node":{"__typename":"DraftIssue"}});
                }
                return json!({"node":{"__typename":"DraftIssue","id":id,"body":near["body"]},
                    "draft":{"projectV2Items":{"nodes":[{"project":{"id":"PVT-board","number":7}}],
                                               "pageInfo":{"hasNextPage":false}}},
                    "board":board_root});
            }
            let (connection, blocking) = if forward {
                ("blockedBy", false)
            } else {
                ("blocking", true)
            };
            let mut node = json!({"__typename":"Issue"});
            node[connection] = json!({"nodes":board.related(&id, blocking),
                                      "pageInfo":{"hasNextPage":false,"endCursor":null}});
            if forward {
                node["body"] = near["body"].clone();
                return json!({"node":node,"board":board_root});
            }
            return json!({ "node": node });
        }
    }
    // llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]
    // One draft by its own id, with the one board item GitHub links it to.
    if query.contains("on DraftIssue{") && query.contains("projectV2Items(") {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = board.items.iter().find(|item| item["id"] == json!(id)) else {
            return json!({ "node": null });
        };
        if !GitHubBoard::is_draft(item) {
            return json!({"node":{"__typename":"Issue"}});
        }
        let board_item = board.rendered(item);
        let mut draft = board.content(item);
        draft["projectV2Items"] = json!({"nodes":[{"id":board_item["id"],
                                                   "project":{"id":"PVT-board","number":7},
                                                   "fieldValues":board_item["fieldValues"]}],
                                         "pageInfo":{"hasNextPage":false,"endCursor":null}});
        return json!({ "node": draft });
    }
    if query == onetaskgraph_github_projects::graphql::ISSUE {
        let id = variables["id"].as_str().expect("a node id").to_owned();
        let Some(item) = board.items.iter().find(|item| item["id"] == json!(id)) else {
            return json!({ "node": null });
        };
        // A draft resolves as a node of its own type, and the fragment on `Issue` selects
        // nothing of it: what it holds is read by its own draft read.
        if GitHubBoard::is_draft(item) {
            return json!({"node":{"__typename":"DraftIssue"}});
        }
        let mut node = board.as_issue(item);
        // The board it sits on with that board's fields, and what blocks it, as the read of an
        // item by its own id asks for beside the issue.
        node["boards"] = json!({"nodes":[{"project":{"id":"PVT-board","number":7,
                                                     "fields":board.fields()}}]});
        node["blockedBy"] = json!({"nodes":board.related(&id, false),
                                   "pageInfo":{"hasNextPage":false,"endCursor":null}});
        return json!({ "node": node });
    }
    if query.contains("node(id:$id)") {
        let id = variables["id"].as_str().expect("dependency id").to_owned();
        let first = variables["first"]
            .as_u64()
            .expect("dependency first must be an unsigned integer");
        assert!(
            (1..=100).contains(&first),
            "dependency first is out of range"
        );
        assert!(
            variables["after"].is_null() || variables["after"].is_string(),
            "dependency after must be null or a string"
        );
        let Some(near) = board.items.iter().find(|item| item["id"] == json!(id)) else {
            return json!({ "node": null });
        };
        // A draft has neither `blockedBy` nor `blocking`.
        if GitHubBoard::is_draft(near) {
            return json!({"node":{"__typename":"DraftIssue"}});
        }
        return json!({"node":{"__typename":"Issue","body":near["body"].clone(),
            "blockedBy":{"nodes":board.related(&id, false),
                         "pageInfo":{"hasNextPage":false,"endCursor":null}},
            "blocking":{"nodes":board.related(&id, true),
                        "pageInfo":{"hasNextPage":false,"endCursor":null}}}});
    }
    assert!(
        query.contains("owner:repositoryOwner"),
        "fixture received an unknown GraphQL operation"
    );
    assert_eq!(variables["owner"], "fixture-owner");
    assert_eq!(variables["number"], 7);
    assert_eq!(variables["nestedFirst"], 50);
    assert_eq!(variables["duplicates"], json!(true));
    let offset = match &variables["after"] {
        Value::Null => 0,
        Value::String(cursor) => cursor.parse::<usize>().expect("numeric after cursor"),
        other => panic!("GraphQL after must be null or a numeric string: {other}"),
    };
    let first = usize::try_from(
        variables["first"]
            .as_u64()
            .expect("GraphQL first must be an unsigned integer"),
    )
    .expect("GraphQL first fits usize");
    assert!(first > 0, "GraphQL first must be positive");
    assert!(
        offset <= board.items.len(),
        "GraphQL after cursor is out of range"
    );
    // The rows a read behind the board's own state can see. `lagging_reads` is how many of
    // the most recently filed ones it cannot yet; see the field's own documentation.
    let visible = board.items.len().saturating_sub(board.lagging_reads);
    let end = (offset + first).min(visible);
    let nodes = board.items[offset.min(visible)..end]
        .iter()
        .filter(|item| GitHubBoard::on_board(item))
        .map(|item| board.rendered(item))
        .collect::<Vec<_>>();
    let title = board.own["title"].clone();
    json!({"owner":{"projectV2":{"id":"PVT-board","title":title,
        "fields":board.fields(),
        "items":{"nodes":nodes,"pageInfo":{"hasNextPage":end < visible,
                                           "endCursor":end.to_string()}}}}})
}

/// GitHub's issue search, as far as this board models it: the qualifiers the source sends,
/// and the phrases it searches for, matched **by token** the way GitHub matches them — every
/// run of letters and digits a word, case-insensitively, the whole body indexed, its metadata
/// comment included.
struct IssueSearch {
    /// Whether a phrase may hold of the title.
    in_title: bool,
    /// Whether a phrase may hold of the body.
    in_body: bool,
    /// Every phrase, as its words; each must hold of one of the fields above.
    phrases: Vec<Vec<String>>,
    /// The instant an `updated:>=` qualifier names: an issue last updated before it is not
    /// in the answer.
    updated_since: Option<chrono::DateTime<chrono::Utc>>,
    /// The state an `is:open` or `is:closed` qualifier names, as the issue's `state` spells
    /// it: an issue in another state is not in the answer.
    state: Option<&'static str>,
}

impl IssueSearch {
    /// The search after its `project:… is:issue` scope.
    fn parse(wanted: &str) -> Self {
        let mut parsed = Self {
            in_title: true,
            in_body: true,
            phrases: Vec::new(),
            updated_since: None,
            state: None,
        };
        let mut chars = wanted.chars().peekable();
        while let Some(&next) = chars.peek() {
            if next.is_whitespace() {
                chars.next();
                continue;
            }
            if next == '"' {
                chars.next();
                let mut phrase = String::new();
                loop {
                    match chars.next() {
                        Some('\\') => phrase.extend(chars.next()),
                        Some('"') | None => break,
                        Some(other) => phrase.push(other),
                    }
                }
                parsed.phrases.push(search_words(&phrase));
                continue;
            }
            let mut word = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                word.push(next);
                chars.next();
            }
            if let Some(state) = match word.as_str() {
                "is:open" => Some("OPEN"),
                "is:closed" => Some("CLOSED"),
                _ => None,
            } {
                parsed.state = Some(state);
                continue;
            }
            match word.strip_prefix("in:") {
                Some(fields) => {
                    let fields: Vec<&str> = fields.split(',').collect();
                    parsed.in_title = fields.contains(&"title");
                    parsed.in_body = fields.contains(&"body");
                }
                None => match word.strip_prefix("updated:>=") {
                    Some(instant) => {
                        parsed.updated_since = Some(
                            chrono::DateTime::parse_from_rfc3339(instant)
                                .unwrap_or_else(|_| panic!("an RFC 3339 instant: {wanted}"))
                                .with_timezone(&chrono::Utc),
                        );
                    }
                    None => {
                        assert!(
                            !word.starts_with("updated:"),
                            "this board models `updated:>=` alone: {wanted}"
                        );
                        parsed.phrases.push(search_words(&word));
                    }
                },
            }
        }
        parsed
    }

    /// Whether an issue last updated at `updated` is recent enough for an `updated:>=`
    /// qualifier — every issue when there is none, and none whose time this board never set.
    fn recent(&self, updated: &Value) -> bool {
        self.updated_since.is_none_or(|since| {
            updated
                .as_str()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| at >= since)
        })
    }

    /// Whether an issue holding this title and body is in the answer.
    ///
    /// A field is split into words only when it could hold one of the phrases at all — each
    /// phrase may be held by either field — so a search of a board of thousands of items costs
    /// this loopback what it costs GitHub, nothing a copy's simulated time does not already
    /// charge, rather than a word list of every item's body.
    fn admits(&self, title: &str, body: &str) -> bool {
        let words = |field: &str, searched: bool| {
            (searched
                && (self.phrases.is_empty()
                    || self.phrases.iter().any(|phrase| may_hold(field, phrase))))
            .then(|| search_words(field))
        };
        let (title, body) = (words(title, self.in_title), words(body, self.in_body));
        let holds = |field: &Option<Vec<String>>, phrase: &[String]| {
            field.as_ref().is_some_and(|field| {
                phrase.is_empty() || field.windows(phrase.len()).any(|window| window == phrase)
            })
        };
        self.phrases
            .iter()
            .all(|phrase| holds(&title, phrase) || holds(&body, phrase))
    }
}

/// Whether `text` could hold every word of `phrase` as [`search_words`] splits it: always for
/// text that is not ASCII, whose lower-casing can turn another character into an ASCII one, and
/// otherwise only when each word occurs in it, any case.
fn may_hold(text: &str, phrase: &[String]) -> bool {
    if !text.is_ascii() {
        return true;
    }
    let lowered = text.to_ascii_lowercase();
    phrase.iter().all(|word| lowered.contains(word.as_str()))
}

/// The words GitHub's index holds of `text`: every run of letters and digits, lower-cased.
fn search_words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn read_http_json(stream: &mut impl Read) -> Value {
    let (line, body, _) = read_http_request(stream);
    body.unwrap_or_else(|| panic!("a {line} carries no JSON body"))
}

/// One request's line and, when it carries one, its JSON body.
fn read_http_request(stream: &mut impl Read) -> (String, Option<Value>, String) {
    let request = loopback::read_request(&mut std::io::BufReader::new(stream))
        .expect("fixture request ended before its HTTP headers or its declared body");
    http_request_parts(&request)
}

/// One request's line, its JSON body when it carries one, and the bearer token it was sent with.
fn http_request_parts(request: &loopback::Request) -> (String, Option<Value>, String) {
    let token = request
        .header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned();
    assert!(
        token.starts_with("test-token"),
        "a {} carries no fixture credential",
        request.line()
    );
    let line = request.line().to_owned();
    if request.header("content-length").is_none() {
        assert!(
            line.starts_with("GET "),
            "a {line} carries no Content-Length"
        );
        return (line, None, token);
    }
    // Named rather than asserted away: this fixture's only client is the binary under
    // test, so a body that is not JSON is that binary's defect, and the bytes it sent are
    // what says which one.
    let body = serde_json::from_slice(request.body()).unwrap_or_else(|problem| {
        panic!(
            "fixture request body is not JSON ({problem}): {}",
            String::from_utf8_lossy(request.body())
        )
    });
    (line, Some(body), token)
}

/// A socket-level Linear GraphQL fixture used by the shared binary journeys.
pub fn linear_block(sandbox: &Sandbox) -> Value {
    linear_server(sandbox, None, &[])
}

/// The same workspace, with the item a dependency read asks about recording `recorded`
/// under the reserved dependency key.
///
/// The counterpart of [`github_projects_recording`], and it exists for the same reason:
/// the shared row is the one every other journey reads, so a workspace holding a key it
/// must not cannot be that row.
pub fn linear_recording(sandbox: &Sandbox, recorded: Value) -> Value {
    linear_server(sandbox, Some(recorded), &[])
}

/// What the workspace below says when it refuses a write it has been told to fail.
///
/// The journey asserts the caller is told this, so the message the fixture sends and the
/// message the assertion reads are one string rather than two that can part.
pub const LINEAR_REFUSED_WRITE: &str = "Linear could not complete that mutation";

/// The same workspace, failing the first native relation it is asked to create.
///
/// The last write of a project copy: the project and both of its tasks have landed, and
/// the edge between the two tasks is what does not — so the copy has real items of its own
/// making to take back, of both kinds. Linear reports a refused mutation as an `errors`
/// entry on an otherwise successful response, which is the shape this sends. The failure
/// is spent once, so the same workspace answers the retry.
pub fn linear_failing_a_relation_write_once(sandbox: &Sandbox) -> Value {
    linear_server(
        sandbox,
        None,
        &[onetaskgraph_linear::graphql::ISSUE_RELATION_CREATE],
    )
}

/// A Linear workspace whose rate limiter refuses every request with `429 Too Many Requests`,
/// and a count of the requests it was sent — so a journey can say a source that would answer
/// rate limited was never asked at all.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] As `linear_workspace` below: the shared e2e harness is where the agreed e2e layout puts every loopback server, and the engine's own suite drives this one.
pub fn linear_rate_limited(sandbox: &Sandbox) -> (Value, Arc<Mutex<usize>>) {
    linear_rate_limited_for(sandbox, None)
}

/// [`linear_rate_limited`], naming its reset `retry_after` seconds ahead on every refusal when
/// it is given — what a source that remembers the reset is held to.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] This is `linear_rate_limited` above naming its reset, and that helper now delegates here: AGENTS.md puts every loopback server and its constructors in this shared harness, once, so no suite copies one.
pub fn linear_rate_limited_for(
    sandbox: &Sandbox,
    retry_after: Option<u64>,
) -> (Value, Arc<Mutex<usize>>) {
    sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let sent = Arc::new(Mutex::new(0_usize));
    let counted = Arc::clone(&sent);
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            if loopback::read_request(&mut std::io::BufReader::new(&mut stream)).is_none() {
                continue;
            }
            *counted.lock().unwrap() += 1;
            let body =
                r#"{"errors":[{"message":"rate limited","extensions":{"code":"RATELIMITED"}}]}"#;
            let wait = retry_after
                .map(|seconds| format!("retry-after: {seconds}\r\n"))
                .unwrap_or_default();
            let _ = write!(
                stream,
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\n{wait}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (
        json!({"endpoint":endpoint,"team":"FIX","status_mapping":linear_default_mapping()}),
        sent,
    )
}

/// An empty Linear workspace, ready to be copied into.
///
/// The same responder over an empty table rather than a second implementation, so what a
/// document written into it has to satisfy is exactly what a document read out of the
/// shared workspace was read through. It outlives one invocation because the responder
/// holds its table in this process — which is what lets a copy be read back by a *later*
/// command, the way a user reads one back.
pub fn linear_empty_workspace(sandbox: &Sandbox) -> Value {
    linear_server_over(sandbox, empty_dataset(), None, &[])
}

/// An empty Linear workspace refusing each operation `failing` names, once.
pub fn linear_empty_workspace_failing(sandbox: &Sandbox, failing: &[&str]) -> Value {
    linear_server_over(sandbox, empty_dataset(), None, failing)
}

/// The shared dataset's shape with nothing in it.
///
/// Spelled from [`dataset`]'s own keys rather than restated, so a collection added there
/// cannot go missing here and leave the responder unwrapping a key that is not present.
fn empty_dataset() -> Value {
    let mut empty = dataset();
    for (_, held) in empty.as_object_mut().expect("the dataset is an object") {
        *held = json!([]);
    }
    empty
}

fn linear_server(sandbox: &Sandbox, recorded: Option<Value>, failing: &[&str]) -> Value {
    linear_server_over(sandbox, dataset(), recorded, failing)
}

/// A Linear workspace over `held`, and a handle that reads what it holds and every request it
/// was sent.
///
/// The handle is what lets a journey assert on the store rather than on what the binary said
/// about it: an issue's raw description byte for byte, and which mutations a command sent.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] The shared e2e harness is where the agreed e2e layout puts every loopback server, so no suite copies one: the journey-matrix table in this file builds each of its rows through both hosted plugins' loopbacks, and the engine's own suite drives both, so neither can live in one plugin's suite. Editing it re-selects every suite, as editing any shared harness does.
pub fn linear_workspace(sandbox: &Sandbox, held: Value) -> (Value, LinearWorkspace) {
    linear_workspace_failing(sandbox, held, &[])
}

/// [`linear_workspace`], refusing each operation `failing` names once, as
/// [`linear_failing_a_relation_write_once`] refuses one: a copy whose last write is refused,
/// and which takes back what it wrote.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] As `linear_workspace` above: the shared e2e harness is where the agreed e2e layout puts every loopback server and its constructors, so no suite copies one, and the Linear suite's rollback journey needs this one's refusal beside the handle the engine's own suite drives too.
pub fn linear_workspace_failing(
    sandbox: &Sandbox,
    held: Value,
    failing: &[&str],
) -> (Value, LinearWorkspace) {
    let workspace = LinearWorkspace {
        state: Arc::new(Mutex::new(linear_with_comments(held))),
        ledger: Arc::new(Mutex::new(Vec::new())),
        answers: Arc::new(Mutex::new(Vec::new())),
        hooks: Arc::new(Mutex::new(Vec::new())),
        bends: Arc::new(Mutex::new(Vec::new())),
        limits: Arc::new(Mutex::new(std::collections::VecDeque::new())),
    };
    let config = linear_serve(sandbox, workspace.clone(), None, failing);
    (config, workspace)
}

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] The loopback Linear lives in the shared e2e harness because more than one suite drives it, and its write handlers here are what apply this rewrite.
/// What a Linear workspace does to the Markdown of a long form it is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearMarkdown {
    /// Stores it byte for byte, as every workspace does until told otherwise.
    Verbatim,
    /// Stores it as spike `spike-linear-stored-content` measured real Linear storing it —
    /// [`crate::linear_markdown::stored`] — and a follow-up write of what it stored as it was
    /// sent.
    Measured,
    /// Stores it as [`LinearMarkdown::Measured`] does, and then turns every `* ` bullet opening a
    /// line into `+ ` and every `+ ` into `* `: a backend that does not hold its own stored
    /// content stable, so a follow-up write of what it stored is stored otherwise again.
    Unstable,
}

/// `written` as the workspace `data` stores a long form, by what it was told with
/// [`LinearWorkspace::rewrite_markdown_on_write`].
fn linear_stored(data: &Value, written: Option<&Value>) -> Value {
    let Some(text) = written.and_then(Value::as_str) else {
        return written.cloned().unwrap_or(Value::Null);
    };
    match data["_linear_markdown"].as_str() {
        Some("measured") => json!(crate::linear_markdown::stored(text)),
        Some("unstable") => {
            let stored = crate::linear_markdown::stored(text)
                .split('\n')
                .map(|line| {
                    if let Some(rest) = line.strip_prefix("* ") {
                        format!("+ {rest}")
                    } else if let Some(rest) = line.strip_prefix("+ ") {
                        format!("* {rest}")
                    } else {
                        line.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            json!(stored)
        }
        _ => json!(text),
    }
}
// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

/// A rewrite of what a Linear workspace answers, applied to the next `times` requests
/// `matches` holds of.
struct LinearBend {
    /// Whether one request is one this bend rewrites.
    matches: RequestMatch,
    /// How many more matching requests it rewrites.
    times: usize,
    /// The rewrite, given the request's variables and the `data` it would have answered.
    bend: AnswerBend,
}

/// One change a journey has a Linear workspace make part way through a command: after the
/// `skip + 1`th request `matches` holds of, and before the workspace reads another.
struct LinearHook {
    /// Whether one request is one this hook counts.
    matches: RequestMatch,
    /// How many matching requests to answer before making the change.
    skip: usize,
    /// The change, made through the same handle a journey makes one with.
    change: Box<dyn FnOnce(&LinearWorkspace) + Send>,
}

/// A Linear workspace over `held` whose team holds exactly `states` — id, name and type — and
/// whose workspace holds exactly the project `statuses` — name and type — in that order.
pub fn linear_workspace_with(
    sandbox: &Sandbox,
    mut held: Value,
    states: &[(&str, &str, &str)],
    statuses: &[(&str, &str)],
) -> (Value, LinearWorkspace) {
    held["_linear_states"] = linear_states(states);
    held["_linear_project_statuses"] = linear_statuses(statuses);
    linear_workspace(sandbox, held)
}

/// What a journey reads of one Linear workspace it configured.
#[derive(Clone)]
pub struct LinearWorkspace {
    state: Arc<Mutex<Value>>,
    ledger: Arc<Mutex<Vec<(String, Value)>>>,
    /// What this workspace answered each of those requests with, at the same index.
    answers: Arc<Mutex<Vec<Value>>>,
    hooks: Arc<Mutex<Vec<LinearHook>>>,
    bends: Arc<Mutex<Vec<LinearBend>>>,
    /// How each of the next requests is answered, in order: `Some(wait)` refused `429` naming
    /// a reset `wait` seconds ahead, `None` answered as usual.
    limits: Arc<Mutex<std::collections::VecDeque<Option<u64>>>>,
}

impl LinearWorkspace {
    /// Answer each of the next requests by `script`, in order: `Some(wait)` is refused
    /// `429 Too Many Requests` with `retry-after: wait`, as Linear's limiter answers, and `None`
    /// is answered as it would have been. Every request after the script is answered as usual.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback Linear's own answers to its next requests, which only its server loop here answers: that state is private to the loopback, so a suite can set it only through this handle, as `block`, `edit_body` and `refuse_once` set the rest; the GitHub Projects and Linear suites are what drive it.
    pub fn rate_limit(&self, script: &[Option<u64>]) {
        self.limits.lock().unwrap().extend(script.iter().copied());
    }

    /// Have this workspace rewrite its answer to each of the next `times` requests `matches`
    /// holds of through `bend`, given that request's variables and the `data` it would have
    /// answered — a Linear whose connections answer as no correct one does.
    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the loopback Linear's answers, which its request loop builds from its private state: like `after_request` above it can only be a method of this handle, and a suite of one plugin could not add it from outside.
    pub fn bend_answers(
        &self,
        matches: impl Fn(&str, &Value) -> bool + Send + 'static,
        times: usize,
        bend: impl FnMut(&Value, &mut Value) + Send + 'static,
    ) {
        self.bends.lock().unwrap().push(LinearBend {
            matches: Box::new(matches),
            times,
            bend: Box::new(bend),
        });
    }

    /// Have this workspace make `change` once it has answered the `skip + 1`th request that
    /// `matches` holds of — its GraphQL document and its variables — and before it reads the
    /// next: a person editing an issue part way through a command.
    pub fn after_request(
        &self,
        matches: impl Fn(&str, &Value) -> bool + Send + 'static,
        skip: usize,
        change: impl FnOnce(&LinearWorkspace) + Send + 'static,
    ) {
        self.hooks.lock().unwrap().push(LinearHook {
            matches: Box::new(matches),
            skip,
            change: Box::new(change),
        });
    }

    /// Rewrite one issue's description, as a person editing it in Linear would.
    pub fn edit_description(&self, issue: &str, content: &str) {
        self.change("tasks", issue, true, |row| {
            row["content"] = json!(content);
            row.as_object_mut()
                .map(|row| row.remove("_linear_description"));
        });
    }

    // llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] The shared e2e harness is where the agreed e2e layout puts every loopback server, `linear_workspace` above among them, so no suite copies one: the engine's own suite and the Linear suite both drive this loopback Linear, so its controls — a person changing what the workspace holds — can only be methods of its handle, as `edit_description` and `add_comment` already are.
    /// Change the held `collection` row `id` through `change`, as a person editing it in
    /// Linear would — moving its `updatedAt` when `moves` says Linear moves it for that change.
    fn change(&self, collection: &str, id: &str, moves: bool, change: impl FnOnce(&mut Value)) {
        let mut data = self.state.lock().unwrap();
        let (_, stamp) = linear_tick(&mut data);
        let row = data[collection]
            .as_array_mut()
            .and_then(|rows| rows.iter_mut().find(|row| row["id"] == id))
            .unwrap_or_else(|| panic!("{collection} holds {id}"));
        change(row);
        if moves {
            row["_updated"] = json!(stamp);
        }
    }

    /// Every held issue and project row carrying the label `label`, changed by `change` — a
    /// label is the workspace's, so renaming or recolouring it changes every item carrying it,
    /// and Linear moves no item's `updatedAt` for it.
    fn each_label(&self, label: &str, change: impl Fn(&mut Value)) {
        let mut data = self.state.lock().unwrap();
        for collection in ["tasks", "projects"] {
            for row in data[collection].as_array_mut().into_iter().flatten() {
                for held in row["labels"].as_array_mut().into_iter().flatten() {
                    if held["id"] == label {
                        change(held);
                    }
                }
            }
        }
        for held in data["labels"].as_array_mut().into_iter().flatten() {
            if held["id"] == label {
                change(held);
            }
        }
    }

    /// Rename the workspace label `label`, as a person would in Linear.
    pub fn rename_label(&self, label: &str, name: &str) {
        self.each_label(label, |held| held["name"] = json!(name));
    }

    /// Give the workspace label `label` the colour `color`, as a person would in Linear.
    pub fn recolour_label(&self, label: &str, color: &str) {
        self.each_label(label, |held| held["color"] = json!(color));
    }

    /// Set the labels — `(id, name)` — the issue or project `id` of `collection` carries, as a
    /// person adding or removing one would: its `updatedAt` moves.
    pub fn set_labels(&self, collection: &str, id: &str, labels: &[(&str, &str)]) {
        self.change(collection, id, true, |row| {
            row["labels"] = Value::Array(
                labels
                    .iter()
                    .map(|(id, name)| json!({"id": id, "name": name}))
                    .collect(),
            );
        });
    }

    /// Retitle the `collection` row `id`: an issue's or a project's `updatedAt` moves, and a
    /// document's — whose title its token's facets carry — is left where it was.
    pub fn retitle(&self, collection: &str, id: &str, title: &str) {
        self.change(collection, id, collection != "documents", |row| {
            row["title"] = json!(title);
        });
    }

    /// Set one issue's priority, by the shared dataset's name for it.
    pub fn set_priority(&self, issue: &str, priority: &str) {
        self.change("tasks", issue, true, |row| {
            row["priority"] = json!(priority)
        });
    }

    /// Move one issue to the workflow state `name` of type `kind`.
    pub fn move_to_state(&self, issue: &str, name: &str, kind: &str) {
        self.change("tasks", issue, true, |row| {
            linear_put_state(row, (name.to_owned(), kind.to_owned()));
        });
    }

    /// Rename the workflow state `from` of the team to `to`, as a person would: every issue in it
    /// reads under the new name, and none moves its `updatedAt`.
    pub fn rename_state(&self, from: &str, to: &str) {
        let mut data = self.state.lock().unwrap();
        for row in data["tasks"].as_array_mut().into_iter().flatten() {
            if row["_linear_state"]["name"] == from {
                row["_linear_state"]["name"] = json!(to);
                row["status"]["name"] = json!(to);
            }
        }
        for state in data["_linear_states"].as_array_mut().into_iter().flatten() {
            if state["name"] == from {
                state["name"] = json!(to);
            }
        }
    }

    /// Move one project to the project status `name` of type `kind`.
    pub fn set_project_status(&self, project: &str, name: &str, kind: &str) {
        self.change("projects", project, true, |row| {
            row["status"]["_linear_status"] = json!({"name": name, "type": kind});
        });
    }

    /// Rename the project status `from` to `to`, as a person would: every project at it reads
    /// under the new name, and none moves its `updatedAt`.
    pub fn rename_project_status(&self, from: &str, to: &str) {
        let mut data = self.state.lock().unwrap();
        for row in data["projects"].as_array_mut().into_iter().flatten() {
            if linear_project_status(&row["status"])["name"] == from {
                let held = linear_project_status(&row["status"]);
                row["status"]["_linear_status"] = json!({"name": to, "type": held["type"]});
            }
        }
        for status in data["_linear_project_statuses"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            if status["name"] == from {
                status["name"] = json!(to);
            }
        }
    }

    /// Rewrite one project's content, as a person editing it in Linear would.
    pub fn edit_project_content(&self, project: &str, content: &str) {
        self.change("projects", project, true, |row| {
            row["_linear_description"] = json!(content);
        });
    }

    /// Rewrite one document's content — its metadata slot included — as a person would: Linear
    /// moves no `updatedAt` for it, as `spike-linear-coverage` measured (4 of 4).
    pub fn edit_document_content(&self, document: &str, content: &str) {
        self.change("documents", document, false, |row| {
            row["_linear_description"] = json!(content);
        });
    }

    /// Refuse, from now on, a document filed under a project this workspace does not hold, as
    /// Linear does — `Entity not found: Project`.
    pub fn refuse_unknown_projects(&self) {
        self.state.lock().unwrap()["_linear_refuse_unknown_projects"] = json!(true);
    }

    /// Put `text` before everything one held item's long-form field holds — an issue's or a
    /// project's `description`, a document's `content`, its metadata slot kept after it — as a
    /// person editing the item's body in Linear would.
    pub fn prepend_long_form(&self, collection: &str, id: &str, text: &str) {
        let held = self.long_form(collection, id).unwrap_or_default();
        self.change(collection, id, collection != "documents", |row| {
            row["_linear_description"] = json!(format!("{text}{held}"));
        });
    }

    /// File one document under `project`, or under none.
    pub fn refile_document(&self, document: &str, project: Option<&str>) {
        self.change("documents", document, false, |row| {
            row["project"] = project.map_or(Value::Null, |project| json!(project));
        });
    }

    /// File one issue under `project`, or under none.
    pub fn file_issue(&self, issue: &str, project: Option<&str>) {
        self.change("tasks", issue, true, |row| match project {
            Some(project) => row["project"] = json!(project),
            None => {
                row.as_object_mut().map(|row| row.remove("project"));
            }
        });
    }

    /// Move one issue to another team, which gives it another identifier and another address.
    pub fn move_team(&self, issue: &str, identifier: &str) {
        self.change("tasks", issue, true, |row| {
            row["_identifier"] = json!(identifier);
            row["url"] = json!(format!("https://linear.app/fixture/issue/{identifier}"));
        });
    }

    /// Relate `from` to `to` — `blocks` or `related` — as a person would: no issue's
    /// `updatedAt` moves for it, as `spike-linear-coverage` measured (3 of 3).
    pub fn relate(&self, from: &str, to: &str, kind: &str) {
        let mut data = self.state.lock().unwrap();
        data["task_dependencies"]
            .as_array_mut()
            .expect("the relations")
            .push(json!({"from": from, "to": to, "kind": kind}));
    }

    /// Remove every relation from `from` to `to`, as a person would.
    pub fn unrelate(&self, from: &str, to: &str) {
        let mut data = self.state.lock().unwrap();
        data["task_dependencies"]
            .as_array_mut()
            .expect("the relations")
            .retain(|edge| !(edge["from"] == from && edge["to"] == to));
    }

    /// Trash one issue, as a person would: it is read as archived and listed nowhere, and the
    /// relations naming it stay where they were.
    pub fn trash_issue(&self, issue: &str) {
        let mut data = self.state.lock().unwrap();
        let (_, stamp) = linear_tick(&mut data);
        let row = data["tasks"]
            .as_array_mut()
            .and_then(|rows| rows.iter_mut().find(|row| row["id"] == issue))
            .expect("the issue to trash");
        row["_trashed"] = json!(stamp);
    }

    /// Give the `collection` row `id` `count` labels — as many as Linear's default page holds or
    /// more, so a read of them may be a first page.
    pub fn crowd_labels(&self, collection: &str, id: &str, count: usize) {
        self.change(collection, id, true, |row| {
            row["labels"] = Value::Array(
                (0..count)
                    .map(|n| json!({"id": format!("L-crowd-{n}"), "name": format!("crowd {n}")}))
                    .collect(),
            );
        });
    }
    // llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

    /// Add one comment to an issue, as a person writing it in Linear would, answering with its
    /// id.
    pub fn add_comment(&self, issue: &str, body: &str) -> String {
        let mut data = self.state.lock().unwrap();
        let (tick, stamp) = linear_tick(&mut data);
        let id = format!("C-P{tick}");
        let row = json!({"id": id, "issue": issue, "body": body, "createdAt": stamp,
                         "updatedAt": stamp, "user": {"displayName": "a person"}});
        data["_linear_comments"]
            .as_array_mut()
            .expect("comments")
            .push(row);
        id
    }

    /// Replace one comment's body, as its author editing it in Linear would.
    pub fn edit_comment(&self, comment: &str, body: &str) {
        let mut data = self.state.lock().unwrap();
        let (_, stamp) = linear_tick(&mut data);
        let row = data["_linear_comments"]
            .as_array_mut()
            .and_then(|rows| rows.iter_mut().find(|row| row["id"] == comment))
            .expect("the comment to edit");
        row["body"] = json!(body);
        row["updatedAt"] = json!(stamp);
    }

    // llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] The shared e2e harness is where the agreed e2e layout puts every loopback server, `linear_workspace` above among them, so no suite copies one: the engine's own suite and the Linear suite both drive this loopback Linear, so its controls — a person changing what the workspace holds — can only be methods of its handle, as `edit_description` and `add_comment` already are.
    /// Delete one comment, as its author would in Linear: no issue's `updatedAt` moves for it, as
    /// `spike-linear-coverage` measured.
    pub fn delete_comment(&self, comment: &str) {
        let mut data = self.state.lock().unwrap();
        let rows = data["_linear_comments"].as_array_mut().expect("comments");
        let before = rows.len();
        rows.retain(|row| row["id"] != comment);
        assert_eq!(rows.len() + 1, before, "the comment to delete");
    }

    /// Rename the person every comment `from` wrote names, as a person changing their display
    /// name in Linear would: their comments read under the new name, and neither a comment's nor
    /// an issue's `updatedAt` moves for it.
    pub fn rename_comment_author(&self, from: &str, to: &str) {
        let mut data = self.state.lock().unwrap();
        for row in data["_linear_comments"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            if row["user"]["displayName"] == from {
                row["user"]["displayName"] = json!(to);
            }
        }
    }

    /// Relate the project `from` to depend on the project `to`, as a person would: no project's
    /// `updatedAt` moves for it, as `spike-linear-coverage` measured (0 of 2).
    pub fn relate_projects(&self, from: &str, to: &str) {
        let mut data = self.state.lock().unwrap();
        data["project_dependencies"]
            .as_array_mut()
            .expect("the project relations")
            .push(json!({"from": from, "to": to, "kind": "blocks"}));
    }

    /// Remove every relation from the project `from` to the project `to`, as a person would.
    pub fn unrelate_projects(&self, from: &str, to: &str) {
        let mut data = self.state.lock().unwrap();
        data["project_dependencies"]
            .as_array_mut()
            .expect("the project relations")
            .retain(|edge| !(edge["from"] == from && edge["to"] == to));
    }

    /// Delete the project `id`, as a person would: it is read nowhere, and the relations naming
    /// it stay where they were.
    pub fn remove_project(&self, id: &str) {
        let mut data = self.state.lock().unwrap();
        data["projects"]
            .as_array_mut()
            .expect("the projects")
            .retain(|row| row["id"] != id);
    }
    // llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

    /// Every request this workspace answered, as its GraphQL document and its variables, in
    /// the order they arrived.
    pub fn served(&self) -> Vec<(String, Value)> {
        self.ledger.lock().unwrap().clone()
    }

    // llmlint: ignore[code_lands_in_the_domain_that_owns_it] A control of the shared loopback Linear's handle, which more than one suite drives.
    /// Have this workspace store every long form it is written — an issue's `description`, a
    /// project's and a document's `content` — as `rewrite` says Linear does, from the next
    /// write on.
    pub fn rewrite_markdown_on_write(&self, rewrite: LinearMarkdown) {
        self.state.lock().unwrap()["_linear_markdown"] = json!(match rewrite {
            LinearMarkdown::Verbatim => "verbatim",
            LinearMarkdown::Measured => "measured",
            LinearMarkdown::Unstable => "unstable",
        });
    }

    /// What this workspace answered each request [`served`](Self::served) names with, at the
    /// same index: the response body, `data` or `errors`.
    pub fn answered(&self) -> Vec<Value> {
        self.answers.lock().unwrap().clone()
    }

    /// The raw long-form field one held item carries — an issue's or a project's
    /// `description`, a document's `content` — exactly as this workspace would hand it back.
    pub fn long_form(&self, kind: &str, id: &str) -> Option<String> {
        let data = self.state.lock().unwrap();
        let row = data[kind].as_array()?.iter().find(|row| row["id"] == id)?;
        Some(match kind {
            "documents" => linear_long_form(row, Vec::new()),
            "projects" => linear_description(row, "project_dependencies", &data),
            _ => linear_description(row, "task_dependencies", &data),
        })
    }

    /// The name of the workflow state one held issue is at.
    pub fn state_of(&self, id: &str) -> Option<String> {
        let data = self.state.lock().unwrap();
        let row = data["tasks"]
            .as_array()?
            .iter()
            .find(|row| row["id"] == id)?;
        linear_state(row)["name"].as_str().map(str::to_owned)
    }

    /// The name of the project status one held project is at.
    pub fn status_of(&self, id: &str) -> Option<String> {
        let data = self.state.lock().unwrap();
        let row = data["projects"]
            .as_array()?
            .iter()
            .find(|row| row["id"] == id)?;
        linear_project_status(&row["status"])["name"]
            .as_str()
            .map(str::to_owned)
    }

    /// Every workflow state of the team, as `(name, type)`, in the order Linear lists them.
    pub fn states(&self) -> Vec<(String, String)> {
        Self::named(&self.state.lock().unwrap()["_linear_states"])
    }

    /// Every project status of the workspace, as `(name, type)`, in the order Linear lists them.
    pub fn project_statuses(&self) -> Vec<(String, String)> {
        Self::named(&self.state.lock().unwrap()["_linear_project_statuses"])
    }

    fn named(held: &Value) -> Vec<(String, String)> {
        held.as_array()
            .map(|held| {
                held.iter()
                    .map(|row| {
                        (
                            row["name"].as_str().unwrap_or_default().to_owned(),
                            row["type"].as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Refuse every create of a workflow state or project status named `name`, from now on,
    /// as Linear refuses one the key may not make; [`Self::allow_create`] lifts it.
    pub fn refuse_create(&self, name: &str) {
        let mut data = self.state.lock().unwrap();
        let refused = data["_linear_refuse_names"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut refused = refused;
        refused.push(json!(name));
        data["_linear_refuse_names"] = Value::Array(refused);
    }

    /// Answer the create of a workflow state or project status named `name` other than as
    /// asked, from now on: `"other-type"` holds it, and answers it, under a type its category
    /// does not derive; `"other-name"` holds it, and answers it, under another name; and
    /// `"no-payload"` answers success with no state or status at all.
    pub fn misanswer_create(&self, name: &str, how: &str) {
        self.state.lock().unwrap()["_linear_misanswer_names"][name] = json!(how);
    }

    /// Lift every refusal [`Self::refuse_create`] set.
    pub fn allow_create(&self) {
        self.state.lock().unwrap()["_linear_refuse_names"] = json!([]);
    }

    /// The id of the held issue titled `title`, for one a command created.
    pub fn issue_titled(&self, title: &str) -> Option<String> {
        let data = self.state.lock().unwrap();
        data["tasks"]
            .as_array()?
            .iter()
            .find(|row| row["title"] == title)
            .and_then(|row| row["id"].as_str().map(str::to_owned))
    }

    /// The project one held issue is filed under.
    pub fn project_of(&self, id: &str) -> Option<String> {
        self.filed_under("tasks", id)
    }

    /// The project one held document is filed under.
    pub fn document_project(&self, id: &str) -> Option<String> {
        self.filed_under("documents", id)
    }

    /// The id of the held document titled `title`, for one a command created.
    pub fn document_titled(&self, title: &str) -> Option<String> {
        let data = self.state.lock().unwrap();
        data["documents"]
            .as_array()?
            .iter()
            .find(|row| row["title"] == title)
            .and_then(|row| row["id"].as_str().map(str::to_owned))
    }

    fn filed_under(&self, kind: &str, id: &str) -> Option<String> {
        let data = self.state.lock().unwrap();
        data[kind]
            .as_array()?
            .iter()
            .find(|row| row["id"] == id)
            .and_then(|row| row["project"].as_str().map(str::to_owned))
    }
}

fn linear_server_over(
    sandbox: &Sandbox,
    held: Value,
    recorded: Option<Value>,
    failing: &[&str],
) -> Value {
    let workspace = LinearWorkspace {
        state: Arc::new(Mutex::new(linear_with_comments(held))),
        ledger: Arc::new(Mutex::new(Vec::new())),
        answers: Arc::new(Mutex::new(Vec::new())),
        hooks: Arc::new(Mutex::new(Vec::new())),
        bends: Arc::new(Mutex::new(Vec::new())),
        limits: Arc::new(Mutex::new(std::collections::VecDeque::new())),
    };
    linear_serve(sandbox, workspace, recorded, failing)
}

/// The largest request this Linear workspace reads before refusing it as too large.
///
/// The stand-in's own bound, not Linear's: it is there so a runaway client cannot make the
/// fixture buffer without end. It is well above what a real write sends — a project carrying
/// a plan's budget answers is about ten kilobytes of JSON in one metadata key — so a journey
/// proving such a write lands is not refused by the stand-in in Linear's place.
const LINEAR_FIXTURE_REQUEST_LIMIT: usize = 256 * 1024;

fn linear_serve(
    sandbox: &Sandbox,
    workspace: LinearWorkspace,
    recorded: Option<Value>,
    failing: &[&str],
) -> Value {
    let (state, ledger, answers) = (
        workspace.state.clone(),
        workspace.ledger.clone(),
        workspace.answers.clone(),
    );
    sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let failing: Vec<String> = failing
        .iter()
        .map(|operation| (*operation).into())
        .collect();
    let failing = Arc::new(Mutex::new(failing));
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.len() > LINEAR_FIXTURE_REQUEST_LIMIT {
                    break;
                }
                if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .and_then(|value| value.parse::<usize>().ok());
                    if length.is_some_and(|length| bytes.len() >= split + 4 + length) {
                        break;
                    }
                }
            }
            if bytes.len() > LINEAR_FIXTURE_REQUEST_LIMIT {
                let text = r#"{"errors":[{"message":"fixture request too large"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 413 Content Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let split = bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|n| n + 4)
                .unwrap_or(bytes.len());
            let request_head = String::from_utf8_lossy(&bytes[..split]);
            let declared_length = request_head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .and_then(|value| value.parse::<usize>().ok());
            if declared_length.is_none_or(|length| bytes.len() != split + length) {
                let text = r#"{"errors":[{"message":"invalid content length"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let request_line = request_head
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches('\r');
            if request_line != "POST /graphql HTTP/1.1" {
                let text = r#"{"errors":[{"message":"expected POST /graphql"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let Ok(request) = serde_json::from_slice::<Value>(&bytes[split..]) else {
                let text = r#"{"errors":[{"message":"invalid fixture request"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            };
            if request.get("query").and_then(Value::as_str).is_none() {
                let text = r#"{"errors":[{"message":"missing GraphQL query"}]}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            ledger.lock().unwrap().push((
                request["query"].as_str().unwrap_or_default().to_owned(),
                request.get("variables").cloned().unwrap_or(Value::Null),
            ));
            let refused = {
                let mut pending = failing.lock().unwrap();
                let operation = request["query"].as_str().unwrap_or_default();
                pending
                    .iter()
                    .position(|failing| failing == operation)
                    .map(|at| pending.remove(at))
                    .is_some()
            };
            let limited = workspace.limits.lock().unwrap().pop_front().flatten();
            if let Some(wait) = limited {
                let body = json!({"errors":[{"message":"rate limited",
                                             "extensions":{"code":"RATELIMITED"}}]});
                let text = body.to_string();
                answers.lock().unwrap().push(body);
                let _ = write!(
                    stream,
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nretry-after: {wait}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            if refused {
                let text = json!({"errors":[{"message":LINEAR_REFUSED_WRITE}]}).to_string();
                answers
                    .lock()
                    .unwrap()
                    .push(json!({"errors":[{"message":LINEAR_REFUSED_WRITE}]}));
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                continue;
            }
            let (status, response) =
                match linear_response(&request, recorded.as_ref(), &mut state.lock().unwrap()) {
                    Ok(mut body) => {
                        let operation = request["query"].as_str().unwrap_or_default();
                        let variables = request.get("variables").cloned().unwrap_or(Value::Null);
                        let mut bends = workspace.bends.lock().unwrap();
                        if let Some(bend) = bends
                            .iter_mut()
                            .find(|bend| bend.times > 0 && (bend.matches)(operation, &variables))
                        {
                            bend.times -= 1;
                            (bend.bend)(&variables, &mut body);
                        }
                        ("200 OK", json!({"data":body}))
                    }
                    Err(message) => ("400 Bad Request", json!({"errors":[{"message":message}]})),
                };
            answers.lock().unwrap().push(response.clone());
            // Answered, and not yet sent: a change made here is one the next request sees and
            // this one did not.
            let due: Vec<LinearHook> = {
                let operation = request["query"].as_str().unwrap_or_default();
                let variables = request.get("variables").cloned().unwrap_or(Value::Null);
                let mut held = workspace.hooks.lock().unwrap();
                let mut due = Vec::new();
                let mut kept = Vec::new();
                for mut hook in std::mem::take(&mut *held) {
                    if !(hook.matches)(operation, &variables) {
                        kept.push(hook);
                    } else if hook.skip == 0 {
                        due.push(hook);
                    } else {
                        hook.skip -= 1;
                        kept.push(hook);
                    }
                }
                *held = kept;
                due
            };
            for hook in due {
                (hook.change)(&workspace);
            }
            let text = serde_json::to_string(&response).unwrap();
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    json!({"endpoint":endpoint,"team":"FIX","status_mapping":linear_default_mapping()})
}

/// The `status_mapping` every Linear workspace here is configured with unless a journey gives
/// its own: a name for each category the shared dataset and the shared journeys write, for
/// both kinds, each one a name the workspace's vocabulary holds for both.
///
/// Linear has no built-in names, so without one a source reads every item as `unknown` and
/// refuses every status write. `Doing` and `Shipped` are the dataset's own names for
/// `in-progress` and `done`, so what the shared rows read back is what the dataset says.
pub fn linear_default_mapping() -> Value {
    json!({"backlog":"Backlog","todo":"Todo","queued":"Queued","in-progress":"Doing",
           "done":"Shipped","cancelled":"Canceled"})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinearRequest {
    query: String,
    variables: serde_json::Map<String, Value>,
}

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] These typed fixture-boundary variables mirror the accepted 2026-08-24 Linear documents; the authoritative variable/nullability contract is available only from Linear's authenticated unversioned explorer, while focused TCP tests prove malformed local requests are rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoVariables {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemVariables {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageVariables {
    first: usize,
    #[serde(default)]
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryVariables {
    first: usize,
    #[serde(default)]
    after: Option<String>,
    filter: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelationVariables {
    id: String,
    first: usize,
    #[serde(default)]
    after: Option<String>,
}

/// An issue's comments are walked backwards, so their page is `last`/`before` rather than
/// the `first`/`after` of every other connection here.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentPageVariables {
    id: String,
    last: usize,
    #[serde(default)]
    before: Option<String>,
}

fn validate_linear_variables(operation: &str, variables: &Value) -> Result<(), &'static str> {
    use onetaskgraph_linear::graphql;
    let valid = match operation {
        graphql::VIEWER | graphql::ORGANIZATION => {
            serde_json::from_value::<NoVariables>(variables.clone()).is_ok()
        }
        graphql::ISSUE | graphql::PROJECT | graphql::DOCUMENT | graphql::COMMENT => {
            serde_json::from_value::<ItemVariables>(variables.clone())
                .is_ok_and(|variables| !variables.id.is_empty())
        }
        graphql::ISSUE_COMMENTS | graphql::ISSUE_DETAIL => serde_json::from_value::<
            CommentPageVariables,
        >(variables.clone())
        .is_ok_and(|variables| {
            !variables.id.is_empty()
                && variables.last > 0
                && variables.before.as_deref() != Some("")
        }),
        // An issue and a body, and nothing else — in particular no `createAsUser`, which Linear
        // takes only from an OAuth application, so a plugin that sent one would be refused.
        graphql::COMMENT_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(variables.get("input"), &["issueId", "body"], &[])
        }
        graphql::COMMENT_UPDATE => {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && valid_linear_write_input(variables.get("input"), &["body"], &[])
        }
        graphql::LABELS => serde_json::from_value::<PageVariables>(variables.clone())
            .is_ok_and(|variables| variables.first > 0 && variables.after.as_deref() != Some("")),
        graphql::ISSUES | graphql::PROJECTS | graphql::DOCUMENTS => {
            serde_json::from_value::<QueryVariables>(variables.clone()).is_ok_and(|variables| {
                variables.first > 0
                    && variables.after.as_deref() != Some("")
                    && valid_linear_filter(&Value::Object(variables.filter))
            })
        }
        graphql::ISSUE_RELATIONS | graphql::PROJECT_RELATIONS => {
            serde_json::from_value::<RelationVariables>(variables.clone()).is_ok_and(|variables| {
                !variables.id.is_empty()
                    && variables.first > 0
                    && variables.after.as_deref() != Some("")
            })
        }
        graphql::RESOLUTION => {
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(variables.clone())
                .is_ok_and(|values| {
                    values.len() == 1 && values.get("key").is_some_and(|value| !value.is_empty())
                })
        }
        // A created name carries a type of its own vocabulary and every member Linear
        // requires of that create: a colour, and for a project status a place in the flow.
        graphql::WORKFLOW_STATE_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["teamId", "name", "type", "color"],
                    &[],
                )
                && variables
                    .pointer("/input/type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        ["backlog", "unstarted", "started", "completed", "canceled"].contains(&kind)
                    })
        }
        graphql::PROJECT_STATUS_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["name", "type", "color", "position"],
                    &[],
                )
                && variables
                    .pointer("/input/type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        [
                            "backlog",
                            "planned",
                            "started",
                            "paused",
                            "completed",
                            "canceled",
                        ]
                        .contains(&kind)
                    })
        }
        // A whole rewrite, beside the page size its relations are read back at.
        graphql::ISSUE_REWRITE | graphql::PROJECT_REWRITE => {
            let mut whole = variables.clone();
            let first = whole
                .as_object_mut()
                .and_then(|variables| variables.remove("first"));
            first
                .as_ref()
                .and_then(Value::as_u64)
                .is_some_and(|first| first > 0)
                && validate_linear_variables(
                    if operation == graphql::ISSUE_REWRITE {
                        graphql::ISSUE_UPDATE
                    } else {
                        graphql::PROJECT_UPDATE
                    },
                    &whole,
                )
                .is_ok()
        }
        // A narrow or a targeted issue write, which reads the issue back in the same request:
        // no `labelIds`, which only a whole write sends, and at least one of the members a
        // targeted update may send, and nothing else — each one it is not sent is one Linear
        // leaves as it was.
        graphql::ISSUE_UPDATE_READ
            if variables
                .pointer("/input")
                .and_then(Value::as_object)
                .is_some_and(|input| !input.contains_key("labelIds")) =>
        {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && variables
                    .pointer("/input")
                    .and_then(Value::as_object)
                    .is_some_and(|input| !input.is_empty())
                && valid_linear_write_input(
                    variables.get("input"),
                    &[],
                    &["title", "description", "stateId", "priority"],
                )
        }
        graphql::ISSUE_LABEL | graphql::PROJECT_LABEL => {
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(variables.clone())
                .is_ok_and(|values| {
                    values.len() == 1 && values.get("name").is_some_and(|value| !value.is_empty())
                })
        }
        graphql::ISSUE_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["teamId", "title", "stateId", "labelIds", "priority"],
                    &["description", "projectId"],
                )
        }
        // The two narrow issue writes: exactly one member, and nothing beside it.
        graphql::ISSUE_PRIORITY_UPDATE => {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && valid_linear_write_input(variables.get("input"), &["priority"], &[])
        }
        graphql::PROJECT_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["teamIds", "name", "statusId", "labelIds"],
                    &["content"],
                )
        }
        graphql::ISSUE_RELATION_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["issueId", "relatedIssueId", "type"],
                    &[],
                )
        }
        graphql::PROJECT_RELATION_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &[
                        "projectId",
                        "relatedProjectId",
                        "type",
                        "anchorType",
                        "relatedAnchorType",
                    ],
                    &[],
                )
        }
        graphql::DOCUMENT_CREATE => {
            exact_linear_variable_keys(variables, &["input"])
                && valid_linear_write_input(
                    variables.get("input"),
                    &["title"],
                    &["content", "projectId", "teamId"],
                )
                // Exactly one home, counted by the key being *present*. Linear refuses this
                // input with `Exactly one of initiativeId, teamId, issueId, releaseId,
                // cycleId or projectId must be defined.` and a `projectId: null` beside a
                // `teamId` is two as far as that validator is concerned — observed
                // 2026-09-04, and it is what the live document write failed on. A fake that
                // took the null would put this straight back on a credentialed run.
                && variables
                    .get("input")
                    .and_then(Value::as_object)
                    .is_some_and(|input| {
                        ["projectId", "teamId"]
                            .iter()
                            .filter(|key| input.contains_key(**key))
                            .count()
                            == 1
                    })
        }
        // A project's status alone: `statusId`, and nothing beside it.
        graphql::PROJECT_STATUS_UPDATE => {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && valid_linear_write_input(variables.get("input"), &["statusId"], &[])
        }
        // The narrow long-form writes of a project and a document: the one field the metadata
        // slot lives in — `content`, for both — and nothing beside it.
        graphql::PROJECT_UPDATE | graphql::DOCUMENT_UPDATE
            if variables
                .pointer("/input")
                .and_then(Value::as_object)
                .is_some_and(|input| input.len() == 1 && input.contains_key("content")) =>
        {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && valid_linear_write_input(variables.get("input"), &[], &["content"])
        }
        graphql::DOCUMENT_UPDATE => {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && valid_linear_write_input(
                    variables.get("input"),
                    &["title"],
                    &["content", "projectId"],
                )
        }
        graphql::ISSUE_RELATION_DELETE
        | graphql::PROJECT_RELATION_DELETE
        | graphql::ISSUE_DELETE
        | graphql::PROJECT_DELETE
        | graphql::DOCUMENT_DELETE
        | graphql::COMMENT_DELETE => {
            exact_linear_variable_keys(variables, &["id"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
        }
        graphql::ISSUE_UPDATE | graphql::PROJECT_UPDATE => {
            exact_linear_variable_keys(variables, &["id", "input"])
                && variables
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && valid_linear_write_input(
                    variables.get("input"),
                    if operation == graphql::ISSUE_UPDATE {
                        &["title", "stateId", "labelIds", "priority"]
                    } else {
                        &["name", "statusId", "labelIds"]
                    },
                    if operation == graphql::ISSUE_UPDATE {
                        &["description", "projectId"]
                    } else {
                        &["content"]
                    },
                )
        }
        _ => false,
    };
    valid.then_some(()).ok_or("invalid operation variables")
}

fn exact_linear_variable_keys(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|fields| {
        fields.len() == expected.len() && expected.iter().all(|key| fields.contains_key(*key))
    })
}

fn valid_linear_write_input(value: Option<&Value>, required: &[&str], optional: &[&str]) -> bool {
    let Some(fields) = value.and_then(Value::as_object) else {
        return false;
    };
    if fields
        .keys()
        .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
        || required.iter().any(|key| !fields.contains_key(*key))
    {
        return false;
    }
    fields.iter().all(|(key, value)| match key.as_str() {
        "labelIds" | "teamIds" => value.as_array().is_some_and(|values| {
            values
                .iter()
                .all(|value| value.as_str().is_some_and(|id| !id.is_empty()))
        }),
        "description" | "content" => value.is_null() || value.is_string(),
        "projectId" => value.is_null() || value.as_str().is_some_and(|id| !id.is_empty()),
        // Linear's `priority` input is an `Int` on its own 0–4 scale.
        "priority" => value.as_u64().is_some_and(|number| number <= 4),
        // A project status's place in the workspace's flow, a `Float`.
        "position" => value.is_number(),
        _ => value.as_str().is_some_and(|text| !text.is_empty()),
    })
}

fn valid_linear_filter(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().all(|(key, value)| match key.as_str() {
            // `or` is here and `inIgnoreCase` is gone for the same reason: this server
            // once accepted `labels:{some:{name:{inIgnoreCase:[…]}}}`, which real Linear
            // refuses outright — `StringComparator` has no such member — so the source
            // spells "at least one of these labels" as an `or` of `eqIgnoreCase` now. A
            // fake that keeps taking what the real API refuses is how that reached a
            // credentialed run in the first place.
            "and" | "or" => value.as_array().is_some_and(|values| {
                values
                    .iter()
                    .all(|value| value.is_object() && valid_linear_filter(value))
            }),
            // `accessibleTeams` and `status` are here and `team` and the issue's `state`
            // shape are gone from the project side for the same reason `or` replaced
            // `inIgnoreCase` above: `ProjectFilter` is not `IssueFilter`, has no `team` at
            // all, and reaches a project's status through `status` — Linear refused the
            // other spelling with `Field "team" is not defined by type "ProjectFilter"`.
            //
            // `title`, `description`, `priority` and `comments` are the follow-up searches'
            // narrowings, and `createdAt`/`updatedAt` the comment members one of them reaches —
            // each one the plugin's pinned schema carries and the real API was observed taking.
            "team" | "accessibleTeams" | "key" | "labels" | "some" | "name" | "every" | "state"
            | "status" | "type" | "project" | "id" | "title" | "description" | "priority"
            | "comments" | "createdAt" | "updatedAt" => {
                value.is_object() && valid_linear_filter(value)
            }
            "eqIgnoreCase" | "neqIgnoreCase" | "eq" | "contains" | "containsIgnoreCase" | "gte" => {
                value.is_string()
            }
            // A list of names, or — under `priority` alone — of Linear's priority numbers.
            "in" | "nin" => value.as_array().is_some_and(|values| {
                values.iter().all(Value::is_string) || values.iter().all(Value::is_number)
            }),
            "null" => value.is_boolean(),
            _ => false,
        }),
        _ => false,
    }
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] The loopback Linear lives in the shared e2e harness because more than one suite drives it, and how it answers a document the Linear plugin sends is part of answering a request, beside every other answer it gives here.
impl LinearWorkspace {
    /// Answer each connection of a copy's batched origin read — the Linear plugin's
    /// `graphql::COUNTERPART_LOOKUPS` — with at most `rows` rows, whatever it asked for, so a page of
    /// it has a next page; every other read pages as it asks.
    pub fn cap_origin_lookup_page(&self, rows: usize) {
        self.state.lock().unwrap()["_linear_origin_page_cap"] = json!(rows);
    }

    /// Hold one more issue, `row` in the shape the dataset's tasks take, after every issue held
    /// already — as a person creating one in Linear would.
    pub fn add_issue(&self, row: Value) {
        let mut data = self.state.lock().unwrap();
        data["tasks"]
            .as_array_mut()
            .expect("the workspace holds issues")
            .push(row);
    }
}

/// A copy's batched origin read: each aliased connection answered as the `issues` and
/// `projects` listings answer the same `first` and `filter` — the cache's listings for the
/// document the cache sends — and `organization` as its own read answers it, side by side
/// under the aliases the document gives them.
fn linear_origin_lookups(
    operation: &str,
    vars: &Value,
    recorded: Option<&Value>,
    data: &mut Value,
) -> Result<Value, &'static str> {
    use onetaskgraph_linear::graphql;
    let (issues_document, projects_document) = if operation == graphql::COUNTERPART_LOOKUPS_CACHED {
        (graphql::ISSUES_CACHED, graphql::PROJECTS_CACHED)
    } else {
        (graphql::ISSUES, graphql::PROJECTS)
    };
    let fields = vars.as_object().ok_or("origin lookups take variables")?;
    let expected = [
        "issuesFirst",
        "issuesFilter",
        "projectsFirst",
        "projectsFilter",
        "key",
    ];
    if fields.len() != expected.len() || !expected.iter().all(|key| fields.contains_key(*key)) {
        return Err("origin lookups take exactly their five variables");
    }
    let cap = data["_linear_origin_page_cap"].as_u64();
    let first = |key: &str| -> Result<u64, &'static str> {
        let asked = vars[key]
            .as_u64()
            .filter(|first| *first > 0)
            .ok_or("an origin lookup's page asks for at least one row")?;
        if asked > u64::from(onetaskgraph_linear::MAX_PAGE_SIZE) {
            return Err("an origin lookup's page asks for more than the plugin's largest page");
        }
        Ok(cap.map_or(asked, |cap| asked.min(cap)))
    };
    let issues = linear_response(
        &json!({"query": issues_document,
                "variables": {"first": first("issuesFirst")?, "filter": vars["issuesFilter"]}}),
        recorded,
        data,
    )?;
    let projects = linear_response(
        &json!({"query": projects_document,
                "variables": {"first": first("projectsFirst")?, "filter": vars["projectsFilter"]}}),
        recorded,
        data,
    )?;
    // The organization, the team's workflow states and the workspace's project statuses, as the
    // resolution read on its own answers them.
    let resolution = linear_response(
        &json!({"query": graphql::RESOLUTION, "variables": {"key": vars["key"]}}),
        recorded,
        data,
    )?;
    Ok(json!({
        "originIssues": issues["issues"],
        "originProjects": projects["projects"],
        "organization": resolution["organization"],
        "teams": resolution["teams"],
        "projectStatuses": resolution["projectStatuses"],
    }))
}
// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

fn linear_response(
    request: &Value,
    recorded: Option<&Value>,
    data: &mut Value,
) -> Result<Value, &'static str> {
    let request: LinearRequest =
        serde_json::from_value(request.clone()).map_err(|_| "invalid GraphQL request")?;
    use onetaskgraph_linear::graphql;
    let operation = request.query.as_str();
    if operation == graphql::COUNTERPART_LOOKUPS || operation == graphql::COUNTERPART_LOOKUPS_CACHED
    {
        return linear_origin_lookups(operation, &Value::Object(request.variables), recorded, data);
    }
    if ![
        graphql::VIEWER,
        graphql::ORGANIZATION,
        graphql::ISSUE,
        graphql::PROJECT,
        graphql::ISSUES,
        graphql::PROJECTS,
        graphql::LABELS,
        graphql::ISSUE_RELATIONS,
        graphql::PROJECT_RELATIONS,
        graphql::RESOLUTION,
        graphql::WORKFLOW_STATE_CREATE,
        graphql::PROJECT_STATUS_CREATE,
        graphql::ISSUE_UPDATE_READ,
        graphql::ISSUE_REWRITE,
        graphql::PROJECT_REWRITE,
        graphql::ISSUE_LABEL,
        graphql::PROJECT_LABEL,
        graphql::ISSUE_CREATE,
        graphql::ISSUE_UPDATE,
        graphql::ISSUE_PRIORITY_UPDATE,
        graphql::PROJECT_CREATE,
        graphql::PROJECT_UPDATE,
        graphql::PROJECT_STATUS_UPDATE,
        graphql::ISSUE_RELATION_CREATE,
        graphql::PROJECT_RELATION_CREATE,
        graphql::ISSUE_RELATION_DELETE,
        graphql::PROJECT_RELATION_DELETE,
        graphql::ISSUE_DELETE,
        graphql::PROJECT_DELETE,
        graphql::DOCUMENT,
        graphql::DOCUMENTS,
        graphql::DOCUMENT_CREATE,
        graphql::DOCUMENT_UPDATE,
        graphql::DOCUMENT_DELETE,
        graphql::ISSUE_COMMENTS,
        graphql::ISSUE_DETAIL,
        graphql::COMMENT,
        graphql::COMMENT_CREATE,
        graphql::COMMENT_UPDATE,
        graphql::COMMENT_DELETE,
    ]
    .contains(&operation)
        // The documents an attached cache sends are the ones `linear_uncached` answers, and a
        // dependency read's are the ones `linear_narrowed` does.
        && linear_uncached(operation).is_none()
        && linear_narrowed(operation).is_none()
        && operation != graphql::ISSUE_RESOLVED
    {
        return Err("unknown GraphQL operation");
    }
    let vars = Value::Object(request.variables);
    // The workspace's own id, which a fixture names under `organization` — two fixtures naming
    // one are two loopback servers over the same workspace.
    if operation == graphql::ORGANIZATION {
        return Ok(json!({"organization":{
            "id": data.get("organization").and_then(Value::as_str).unwrap_or("fixture-organization")
        }}));
    }
    // What the store's cache sends in place of seven documents: the plain one's answer, with
    // exactly what the cache's document adds to it.
    if let Some(plain) = linear_uncached(operation) {
        let mut answer =
            linear_response(&json!({"query": plain, "variables": vars}), recorded, data)?;
        linear_widen(operation, &mut answer, data, &vars);
        return Ok(answer);
    }
    // llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] How the loopback Linear answers a document Linear's own plugin sends, which this harness's one request loop serves from the fixture's data; every loopback server lives here by the agreed e2e layout, as `GitHubBoard` records, so a Linear suite could not add an answer from outside it.
    // An issue read beside the resolution a status write needs: each answered as it is
    // answered alone, side by side under their own roots.
    if operation == graphql::ISSUE_RESOLVED {
        let mut answer = linear_response(
            &json!({"query": graphql::ISSUE, "variables": {"id": vars["id"]}}),
            recorded,
            data,
        )?;
        let resolution = linear_response(
            &json!({"query": graphql::RESOLUTION, "variables": {"key": vars["key"]}}),
            recorded,
            data,
        )?;
        for (root, value) in resolution.as_object().into_iter().flatten() {
            answer[root] = value.clone();
        }
        return Ok(answer);
    }
    // llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]
    // A dependency read of one direction: the two-direction read's answer, holding only the
    // members that document selects, as Linear answers exactly what is selected.
    if let Some((wide, root)) = linear_narrowed(operation) {
        let selected = selected_members(operation, root);
        let mut answer =
            linear_response(&json!({"query": wide, "variables": vars}), recorded, data)?;
        if let Some(item) = answer.get_mut(root).and_then(Value::as_object_mut) {
            item.retain(|member, _| selected.iter().any(|kept| kept == member));
        }
        return Ok(answer);
    }
    validate_linear_variables(operation, &vars)?;
    // The issue and a page of its comments in one answer: exactly what the issue read and the
    // comment page read answer apart, on one `issue` field.
    if operation == graphql::ISSUE_DETAIL {
        let issue = linear_response(
            &json!({"query": graphql::ISSUE, "variables": {"id": vars["id"]}}),
            recorded,
            data,
        )?;
        if issue["issue"].is_null() {
            return Ok(issue);
        }
        let comments = linear_response(
            &json!({"query": graphql::ISSUE_COMMENTS, "variables": vars}),
            recorded,
            data,
        )?;
        let mut answer = issue;
        answer["issue"]["comments"] = comments["issue"]["comments"].clone();
        return Ok(answer);
    }
    if let Some(answer) = linear_comment_response(data, &vars, operation) {
        return answer;
    }
    if operation == graphql::RESOLUTION {
        // The team, its states and the workspace's project statuses, as this workspace holds
        // them now — so a name `sources fields --apply` created is in the next answer.
        let team = vars["key"].as_str().unwrap_or_default();
        let teams = if team.eq_ignore_ascii_case(LINEAR_TEAM) {
            json!([{"id":"TEAM-1","states":{"nodes":data["_linear_states"],
                "pageInfo":{"hasNextPage":false}}}])
        } else {
            json!([])
        };
        // The workspace the credential belongs to, which a source declared private is verified
        // against; a journey withholding it sets `_linear_organization` to null.
        let organization = match data.get("_linear_organization") {
            None => json!({"id":"ORG-1"}),
            Some(held) => held.clone(),
        };
        return Ok(json!({"organization":organization,"teams":{"nodes":teams},
            "projectStatuses":{"nodes":data["_linear_project_statuses"],
                "pageInfo":{"hasNextPage":false}}}));
    }
    if matches!(
        operation,
        graphql::WORKFLOW_STATE_CREATE | graphql::PROJECT_STATUS_CREATE
    ) {
        return linear_create_status_name(data, &vars, operation == graphql::PROJECT_STATUS_CREATE);
    }
    // A whole rewrite of an item, answering with the first page of the relations it holds.
    if matches!(operation, graphql::ISSUE_REWRITE | graphql::PROJECT_REWRITE) {
        let project = operation == graphql::PROJECT_REWRITE;
        let mut answer = linear_write_item(data, &vars, false, project)?;
        let id = vars["id"].as_str().unwrap_or_default();
        let (key, suffix, root, payload) = if project {
            (
                "project_dependencies",
                "Project",
                "projectUpdate",
                "project",
            )
        } else {
            ("task_dependencies", "Issue", "issueUpdate", "issue")
        };
        answer[root][payload]["relations"] =
            linear_relations(data, key, id, suffix, recorded)["relations"].clone();
        return Ok(answer);
    }
    // A project's status alone, which the project then reads back at — answered, as Linear
    // answers one addressing a project it does not hold, with `Entity not found: Project`.
    if operation == graphql::PROJECT_STATUS_UPDATE {
        let id = vars["id"].as_str().ok_or("update id must be a string")?;
        let held = linear_held_status(data, vars["input"]["statusId"].as_str().unwrap_or_default())
            .ok_or("statusId names no project status of the workspace")?;
        let category = linear_status_category(held["type"].as_str().unwrap_or_default());
        let (_, stamp) = linear_tick(data);
        let Some(row) = data["projects"]
            .as_array_mut()
            .ok_or("fixture collection is not an array")?
            .iter_mut()
            .find(|row| row["id"] == id)
        else {
            return Err(graphql::PROJECT_NOT_FOUND_MESSAGE);
        };
        row["status"] = json!({"name":held["name"],"category":category,"_linear_status":held});
        row["_updated"] = json!(stamp);
        let status = linear_project_status(&row["status"]);
        return Ok(json!({"projectUpdate":{"success":true,
                         "project":{"id":id,"status":status}}}));
    }
    if operation == graphql::ISSUE_UPDATE_READ {
        let id = vars["id"].as_str().unwrap_or_default();
        let held = data["tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|task| task["id"] == id));
        if !held {
            return Err(LINEAR_ENTITY_MISSING);
        }
        linear_narrow_issue_update(data, &vars, operation)?;
        let row = data["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|task| task["id"] == id))
            .cloned()
            .ok_or("update target does not exist")?;
        return Ok(json!({"issueUpdate":{"success":true,"issue":linear_task(&row, data)}}));
    }
    if operation == graphql::ISSUE_LABEL {
        return Ok(json!({"issueLabels":{"nodes":[{"id":vars["name"]}]}}));
    }
    if operation == graphql::PROJECT_LABEL {
        return Ok(json!({"projectLabels":{"nodes":[{"id":vars["name"]}]}}));
    }
    // A narrow or a targeted write sends only the members it changes, and Linear leaves every
    // input member it was not sent as it was.
    if matches!(
        operation,
        graphql::ISSUE_UPDATE | graphql::ISSUE_PRIORITY_UPDATE
    ) && vars["input"].get("labelIds").is_none()
    {
        return linear_narrow_issue_update(data, &vars, operation);
    }
    if matches!(operation, graphql::ISSUE_CREATE | graphql::ISSUE_UPDATE) {
        return linear_write_item(data, &vars, operation == graphql::ISSUE_CREATE, false);
    }
    // A project's content alone, or a document's content alone: the long-form field moves
    // and nothing else about the item does.
    if matches!(
        operation,
        graphql::PROJECT_UPDATE | graphql::DOCUMENT_UPDATE
    ) && vars["input"]
        .as_object()
        .is_some_and(|input| input.len() == 1)
        && vars["input"].get("content").is_some()
    {
        let (collection, field, root, payload) = if operation == graphql::PROJECT_UPDATE {
            ("projects", "content", "projectUpdate", "project")
        } else {
            ("documents", "content", "documentUpdate", "document")
        };
        let id = vars["id"].as_str().ok_or("update id must be a string")?;
        let stored = linear_stored(data, vars["input"].get(field));
        let (_, stamp) = linear_tick(data);
        let row = data[collection]
            .as_array_mut()
            .ok_or("fixture collection is not an array")?
            .iter_mut()
            .find(|row| row["id"] == id)
            .ok_or("update target does not exist")?;
        row["_linear_description"] = stored;
        // A project's `updatedAt` moves for its content; a document's does not, which is what
        // `spike-linear-coverage` measured Linear doing (4 of 4).
        if collection == "projects" {
            row["_updated"] = json!(stamp);
        }
        let written = row.clone();
        let item = if collection == "projects" {
            linear_project(&written, data)
        } else {
            linear_document(&written)
        };
        let _ = field;
        return Ok(json!({ (root): {"success":true,(payload):item} }));
    }
    if matches!(operation, graphql::PROJECT_CREATE | graphql::PROJECT_UPDATE) {
        return linear_write_item(data, &vars, operation == graphql::PROJECT_CREATE, true);
    }
    if matches!(
        operation,
        graphql::DOCUMENT_CREATE | graphql::DOCUMENT_UPDATE
    ) {
        return linear_write_document(data, &vars, operation == graphql::DOCUMENT_CREATE);
    }
    if operation == graphql::DOCUMENT_DELETE {
        let id = vars["id"].as_str().ok_or("delete id must be a string")?;
        data["documents"]
            .as_array_mut()
            .ok_or("fixture collection is not an array")?
            .retain(|row| row["id"] != json!(id));
        return Ok(json!({"documentDelete":{"success":true}}));
    }
    if operation == graphql::DOCUMENTS {
        let rows = data["documents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| linear_holds(v, &vars["filter"], LinearRow::Document, data))
            .map(linear_document)
            .collect();
        return Ok(json!({"documents":linear_connection(rows,&vars)}));
    }
    if operation == graphql::DOCUMENT {
        let id = vars["id"].as_str().unwrap_or("");
        let item = data["documents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id);
        return Ok(json!({"document":item.map(linear_document)}));
    }
    if matches!(
        operation,
        graphql::ISSUE_RELATION_CREATE | graphql::PROJECT_RELATION_CREATE
    ) {
        return linear_write_relation(data, &vars, operation == graphql::PROJECT_RELATION_CREATE);
    }
    if matches!(operation, graphql::ISSUE_DELETE | graphql::PROJECT_DELETE) {
        let project = operation == graphql::PROJECT_DELETE;
        let id = vars["id"].as_str().ok_or("delete id must be a string")?;
        let rows = data[if project { "projects" } else { "tasks" }]
            .as_array_mut()
            .ok_or("fixture collection is not an array")?;
        rows.retain(|row| row["id"] != json!(id));
        return Ok(if project {
            json!({"projectDelete":{"success":true}})
        } else {
            json!({"issueDelete":{"success":true}})
        });
    }
    if matches!(
        operation,
        graphql::ISSUE_RELATION_DELETE | graphql::PROJECT_RELATION_DELETE
    ) {
        let project = operation == graphql::PROJECT_RELATION_DELETE;
        let index = vars["id"]
            .as_str()
            .and_then(|id| id.rsplit(':').next())
            .and_then(|id| id.parse::<usize>().ok())
            .ok_or("invalid relation fixture id")?;
        let edges = data[if project {
            "project_dependencies"
        } else {
            "task_dependencies"
        }]
        .as_array_mut()
        .ok_or("fixture edges are not an array")?;
        if index < edges.len() {
            edges.remove(index);
        }
        return Ok(if project {
            json!({"projectRelationDelete":{"success":true}})
        } else {
            json!({"issueRelationDelete":{"success":true}})
        });
    }
    if operation == graphql::LABELS {
        return Ok(
            json!({"issueLabels":linear_connection(data["labels"].as_array().unwrap().iter().map(linear_label).collect(),&vars)}),
        );
    }
    if operation == graphql::ISSUES {
        let mut rows: Vec<Value> = data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| linear_holds(v, &vars["filter"], LinearRow::Issue, data))
            // A trashed issue is listed nowhere, as Linear's own listing leaves it out.
            .filter(|v| v.get("_trashed").is_none_or(Value::is_null))
            .map(|v| linear_task(v, data))
            .collect();
        return Ok(json!({"issues":linear_connection(std::mem::take(&mut rows),&vars)}));
    }
    if operation == graphql::PROJECTS {
        let rows = data["projects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| linear_holds(v, &vars["filter"], LinearRow::Project, data))
            .map(|v| linear_project(v, data))
            .collect();
        return Ok(json!({"projects":linear_connection(rows,&vars)}));
    }
    if matches!(operation, graphql::ISSUE | graphql::ISSUE_RELATIONS) {
        let id = vars["id"].as_str().unwrap_or("");
        let item = data["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id);
        if operation == graphql::ISSUE_RELATIONS {
            return Ok(json!({"issue":linear_relation_page(
                linear_relations(data, "task_dependencies", id, "Issue", recorded),
                &vars,
            )}));
        }
        return Ok(json!({"issue":item.map(|v|linear_task(v,data))}));
    }
    if matches!(operation, graphql::PROJECT | graphql::PROJECT_RELATIONS) {
        let id = vars["id"].as_str().unwrap_or("");
        let item = data["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id);
        if operation == graphql::PROJECT_RELATIONS {
            return Ok(json!({"project":linear_relation_page(
                linear_relations(data, "project_dependencies", id, "Project", recorded),
                &vars,
            )}));
        }
        return Ok(json!({"project":item.map(|v|linear_project(v,data))}));
    }
    Ok(json!({"viewer":{"id":"fixture-user"}}))
}

/// `item`'s relations and inverse relations, each cut to the page a dependency read's `first` and
/// `after` ask for, as Linear pages a connection: at most `first` rows after the cursor `after`,
/// with `hasNextPage` saying whether more follow and `endCursor` where they resume.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] The shared e2e harness is where the agreed e2e layout puts every loopback server, `linear_workspace` above among them, so no suite copies one: the engine's own suite and the Linear suite both drive this loopback Linear, so its answers are its own: how it pages a relations connection is part of answering a request, beside every other answer it gives here.
fn linear_relation_page(mut item: Value, vars: &Value) -> Value {
    let first = vars["first"]
        .as_u64()
        .and_then(|first| usize::try_from(first).ok())
        .unwrap_or(50);
    let after = vars["after"]
        .as_str()
        .and_then(|after| after.parse::<usize>().ok())
        .unwrap_or(0);
    for connection in ["relations", "inverseRelations"] {
        let nodes = item[connection]["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let end = after.saturating_add(first).min(nodes.len());
        let page: Vec<Value> = nodes.get(after.min(end)..end).unwrap_or_default().to_vec();
        let more = end < nodes.len();
        item[connection] = json!({"nodes": page, "pageInfo": {
            "hasNextPage": more,
            "endCursor": more.then(|| end.to_string()),
        }});
    }
    item
}

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] The shared e2e harness is where the agreed e2e layout puts every loopback server, `linear_workspace` above among them, so no suite copies one: the engine's own suite and the Linear suite both drive this loopback Linear, so its answers are its own: recognising the documents an attached cache sends, and answering them, is part of answering a request, beside every other answer it gives here.
/// The document the store's cache sends `operation` in place of, when it sends it in place of one.
fn linear_uncached(operation: &str) -> Option<&'static str> {
    use onetaskgraph_linear::graphql;
    match operation {
        graphql::ISSUES_CACHED => Some(graphql::ISSUES),
        graphql::ISSUE_CACHED => Some(graphql::ISSUE),
        graphql::ISSUE_DETAIL_CACHED => Some(graphql::ISSUE_DETAIL),
        graphql::ISSUE_RELATIONS_CACHED => Some(graphql::ISSUE_RELATIONS),
        graphql::PROJECTS_CACHED => Some(graphql::PROJECTS),
        graphql::PROJECT_CACHED => Some(graphql::PROJECT),
        graphql::PROJECT_RELATIONS_CACHED => Some(graphql::PROJECT_RELATIONS),
        _ => None,
    }
}

/// The two-direction relations read a one-direction dependency read narrows, and the root it
/// reads; the members it selects of that root are read off the document itself, by
/// [`selected_members`], so they cannot drift from what the plugin sends.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] As `linear_uncached` below: the engine's own suite and the Linear suite both drive this loopback Linear, so recognising the one-direction dependency reads it is sent, and answering them, is part of answering a request here.
fn linear_narrowed(operation: &str) -> Option<(&'static str, &'static str)> {
    use onetaskgraph_linear::graphql;
    match operation {
        graphql::ISSUE_DEPENDS_ON | graphql::ISSUE_DEPENDED_ON_BY => {
            Some((graphql::ISSUE_RELATIONS, "issue"))
        }
        graphql::PROJECT_DEPENDS_ON | graphql::PROJECT_DEPENDED_ON_BY => {
            Some((graphql::PROJECT_RELATIONS, "project"))
        }
        _ => None,
    }
}

/// The members `document` selects directly under its root field `root`.
fn selected_members(document: &str, root: &str) -> Vec<String> {
    use graphql_parser::query::{Definition, OperationDefinition, Selection, SelectionSet};
    let parsed = graphql_parser::parse_query::<String>(document).expect("a plugin document parses");
    let fields = |set: &SelectionSet<'_, String>| -> Vec<String> {
        set.items
            .iter()
            .filter_map(|item| match item {
                Selection::Field(field) => {
                    Some(field.alias.clone().unwrap_or_else(|| field.name.clone()))
                }
                _ => None,
            })
            .collect()
    };
    parsed
        .definitions
        .iter()
        .find_map(|definition| match definition {
            Definition::Operation(OperationDefinition::Query(query)) => query
                .selection_set
                .items
                .iter()
                .find_map(|item| match item {
                    Selection::Field(field) if field.name == root => {
                        Some(fields(&field.selection_set))
                    }
                    _ => None,
                }),
            _ => None,
        })
        .expect("the document selects its root")
}

/// `nodes` as the first page of a `first:` connection the cache's documents select answers it:
/// at most that page's rows, and whether more follow.
fn linear_first_page(nodes: Vec<Value>) -> Value {
    // The page the cache's documents select, which the plugin's own test holds them to.
    let first = onetaskgraph_linear::change_token::PAGE;
    let more = nodes.len() > first;
    json!({"nodes": nodes.into_iter().take(first).collect::<Vec<_>>(),
           "pageInfo": {"hasNextPage": more}})
}

/// Add to `answer`, the plain document's, what the cache's `operation` selects beside it: each
/// issue's first page of comments, relations and inverse relations, each project's first page
/// of relations and inverse relations, or — for a dependency read — every other part of its
/// token.
fn linear_widen(operation: &str, answer: &mut Value, data: &Value, vars: &Value) {
    use onetaskgraph_linear::graphql;
    let relations = |key: &str, suffix: &str, id: &str, connection: &str| {
        let held = linear_relations(data, key, id, suffix, None);
        linear_first_page(
            held[connection]["nodes"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        )
    };
    // Newest first, as the comment page read lists them.
    let comments = |id: &str| {
        linear_first_page(
            data["_linear_comments"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
                .filter(|row| row["issue"] == id)
                .map(|row| json!({"id": row["id"], "updatedAt": row["updatedAt"]}))
                .collect(),
        )
    };
    let widen_relations = |issue: &mut Value| {
        if let Some(id) = issue.get("id").and_then(Value::as_str).map(str::to_owned) {
            issue["relations"] = relations("task_dependencies", "Issue", &id, "relations");
            issue["inverseRelations"] =
                relations("task_dependencies", "Issue", &id, "inverseRelations");
        }
    };
    // And the comments page, which a widened read with no comments page of its own adds.
    let widen_issue = |issue: &mut Value| {
        widen_relations(issue);
        if let Some(id) = issue.get("id").and_then(Value::as_str).map(str::to_owned) {
            issue["comments"] = comments(&id);
        }
    };
    let widen_project = |project: &mut Value| {
        if let Some(id) = project.get("id").and_then(Value::as_str).map(str::to_owned) {
            project["relations"] = relations("project_dependencies", "Project", &id, "relations");
            project["inverseRelations"] =
                relations("project_dependencies", "Project", &id, "inverseRelations");
        }
    };
    let row = |collection: &str| {
        data[collection]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"] == vars["id"]))
    };
    match operation {
        graphql::ISSUES_CACHED => {
            for issue in answer
                .pointer_mut("/issues/nodes")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                widen_issue(issue);
            }
        }
        graphql::ISSUE_CACHED => {
            if let Some(issue) = answer.get_mut("issue").filter(|issue| !issue.is_null()) {
                widen_issue(issue);
            }
        }
        graphql::ISSUE_DETAIL_CACHED => {
            if let Some(issue) = answer.get_mut("issue").filter(|issue| !issue.is_null()) {
                widen_relations(issue);
            }
        }
        graphql::PROJECTS_CACHED => {
            for project in answer
                .pointer_mut("/projects/nodes")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                widen_project(project);
            }
        }
        graphql::PROJECT_CACHED => {
            if let Some(project) = answer
                .get_mut("project")
                .filter(|project| !project.is_null())
            {
                widen_project(project);
            }
        }
        graphql::PROJECT_RELATIONS_CACHED => {
            let (Some(row), Some(project)) = (
                row("projects"),
                answer
                    .get_mut("project")
                    .filter(|project| project.is_object()),
            ) else {
                return;
            };
            let read = linear_project(row, data);
            for part in ["updatedAt", "status", "labels"] {
                project[part] = read[part].clone();
            }
        }
        // A dependency read: the issue's token parts beside the relations pages it reads.
        _ => {
            let (Some(row), Some(issue)) = (
                row("tasks"),
                answer.get_mut("issue").filter(|issue| issue.is_object()),
            ) else {
                return;
            };
            let read = linear_task(row, data);
            for part in [
                "updatedAt",
                "identifier",
                "url",
                "state",
                "labels",
                "project",
            ] {
                issue[part] = read[part].clone();
            }
            issue["comments"] = comments(row["id"].as_str().unwrap_or_default());
        }
    }
}
// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

fn linear_write_item(
    data: &mut Value,
    vars: &Value,
    create: bool,
    project: bool,
) -> Result<Value, &'static str> {
    let input = vars["input"]
        .as_object()
        .ok_or("write input must be an object")?;
    let collection = if project { "projects" } else { "tasks" };
    // A whole write moves the item's `updatedAt`, as Linear moves it for every field sent.
    let (_, stamp) = linear_tick(data);
    let vocabulary = json!({"_linear_states": data["_linear_states"].clone(),
        "_linear_project_statuses": data["_linear_project_statuses"].clone()});
    let vocabulary_markdown = json!({"_linear_markdown": data["_linear_markdown"].clone()});
    let rows = data[collection]
        .as_array_mut()
        .ok_or("fixture collection is not an array")?;
    let id = if create {
        format!("{}-W{}", if project { "P" } else { "T" }, rows.len() + 1)
    } else {
        vars["id"]
            .as_str()
            .ok_or("update id must be a string")?
            .to_owned()
    };
    let existing = rows.iter().position(|row| row["id"] == id);
    if !create && existing.is_none() {
        return Err("update target does not exist");
    }
    let title_key = if project { "name" } else { "title" };
    let status_key = if project { "statusId" } else { "stateId" };
    let labels = input
        .get("labelIds")
        .and_then(Value::as_array)
        .ok_or("labelIds must be an array")?
        .iter()
        .map(|id| json!({"id":id,"name":id}))
        .collect::<Vec<_>>();
    let status_id = input
        .get(status_key)
        .and_then(Value::as_str)
        .ok_or("status id must be a string")?;
    // A whole write names a state or a project status this workspace holds, by its id, and the
    // item then reads back at it — as Linear refuses an id naming neither.
    let (state, status) = if project {
        let held = linear_held_status(&vocabulary, status_id)
            .ok_or("statusId names no project status of the workspace")?;
        let category = linear_status_category(held["type"].as_str().unwrap_or_default());
        (
            None,
            json!({"name":held["name"],"category":category,"_linear_status":held}),
        )
    } else {
        let held = linear_held_state(&vocabulary, status_id)
            .ok_or("stateId names no state of the team")?;
        (Some(held), Value::Null)
    };
    // An issue's long form is its `description`, a project's its `content`.
    let long_form_key = if project { "content" } else { "description" };
    let stored = linear_stored(&vocabulary_markdown, input.get(long_form_key));
    let mut row = json!({
        "id": id,
        "title": input.get(title_key).and_then(Value::as_str).ok_or("title must be a string")?,
        "content": "",
        "status": status,
        "labels": labels,
        "_linear_description": stored,
    });
    if let Some(state) = state {
        linear_put_state(&mut row, state);
    }
    if !project && let Some(project_id) = input.get("projectId").filter(|v| !v.is_null()) {
        row["project"] = project_id.clone();
    }
    // What an update does not send it does not move: the team an issue is in.
    if let Some(team) = existing.and_then(|index| rows[index].get("team").cloned()) {
        row["team"] = team;
    }
    if !project && let Some(priority) = input.get("priority") {
        row["priority"] = linear_priority_name(priority);
    }
    row["_updated"] = json!(stamp);
    if let Some(index) = existing {
        rows[index] = row;
    } else {
        rows.push(row);
    }
    // The write's own answer carries the whole item as it is now stored — exactly what a read
    // of it answers, as spike `spike-linear-stored-content` measured of Linear — and so the long
    // form as it is now stored, as the plugin asks.
    let _ = (long_form_key, stored);
    let written = data[collection]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == json!(id)))
        .cloned()
        .ok_or("the written item is held")?;
    let payload = if project {
        linear_project(&written, data)
    } else {
        linear_task(&written, data)
    };
    Ok(if project {
        if create {
            json!({"projectCreate":{"success":true,"project":payload}})
        } else {
            json!({"projectUpdate":{"success":true,"project":payload}})
        }
    } else if create {
        json!({"issueCreate":{"success":true,"issue":payload}})
    } else {
        json!({"issueUpdate":{"success":true,"issue":payload}})
    })
}

/// A document created or updated in this workspace, on the terms an issue is.
fn linear_write_document(
    data: &mut Value,
    vars: &Value,
    create: bool,
) -> Result<Value, &'static str> {
    let input = vars["input"]
        .as_object()
        .ok_or("write input must be an object")?;
    // A document filed under a project the workspace does not hold is refused, as Linear refuses
    // a reference to an entity it does not hold — once told to, since most journeys file under a
    // project id they never created here.
    if data["_linear_refuse_unknown_projects"] == true
        && let Some(project) = input.get("projectId").and_then(Value::as_str)
        && !data["projects"]
            .as_array()
            .is_some_and(|projects| projects.iter().any(|row| row["id"] == project))
    {
        return Err(onetaskgraph_linear::graphql::PROJECT_NOT_FOUND_MESSAGE);
    }
    let stored = linear_stored(data, input.get("content"));
    let rows = data["documents"]
        .as_array_mut()
        .ok_or("fixture collection is not an array")?;
    let id = if create {
        format!("D-W{}", rows.len() + 1)
    } else {
        vars["id"]
            .as_str()
            .ok_or("update id must be a string")?
            .to_owned()
    };
    let existing = rows.iter().position(|row| row["id"] == id);
    if !create && existing.is_none() {
        return Err("update target does not exist");
    }
    let mut row = json!({
        "id": id,
        "title": input.get("title").and_then(Value::as_str).ok_or("title must be a string")?,
        "content": "",
        "labels": [],
        "_linear_description": stored,
    });
    if let Some(project) = input.get("projectId").filter(|value| !value.is_null()) {
        row["project"] = project.clone();
    }
    // A document's `updatedAt` stays where it was for a content write, as Linear's did.
    if let Some(updated) = existing.and_then(|index| rows[index].get("_updated").cloned()) {
        row["_updated"] = updated;
    }
    if let Some(index) = existing {
        rows[index] = row;
    } else {
        rows.push(row);
    }
    let _ = stored;
    let written = data["documents"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == json!(id)))
        .cloned()
        .ok_or("the written document is held")?;
    let payload = json!({"success":true,"document":linear_document(&written)});
    Ok(if create {
        json!({ "documentCreate": payload })
    } else {
        json!({ "documentUpdate": payload })
    })
}

/// The workspace, with its comments held the way Linear holds them.
///
/// Seeded from the shared dataset's `comments` when it carries any, in the order they were
/// written, so a comment the dataset gives is one this workspace answers with too; a
/// workspace whose dataset carries none starts with none. Held in the responder's own state,
/// which outlives one invocation of the binary, so a comment one command adds is one a later
/// command lists, edits and removes.
fn linear_with_comments(mut held: Value) -> Value {
    let seeded = held
        .get("comments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|seeded| {
            let comment = &seeded["comment"];
            json!({
                "id": comment["id"],
                "issue": seeded["task"],
                "body": comment["body"],
                "createdAt": comment.get("created_at").cloned().unwrap_or(Value::Null),
                "updatedAt": comment.get("updated_at").cloned().unwrap_or(Value::Null),
                "user": comment
                    .get("author")
                    .and_then(Value::as_str)
                    .map(|name| json!({"displayName": name})),
            })
        })
        .collect::<Vec<_>>();
    held["_linear_comments"] = Value::Array(seeded);
    held["_linear_clock"] = json!(0);
    if held.get("_linear_states").is_none() {
        held["_linear_states"] = Value::Array(
            LINEAR_TEAM_STATES
                .iter()
                .chain(LINEAR_SHARED_STATES)
                .map(|(id, name, kind)| json!({"id":id,"name":name,"type":kind}))
                .collect(),
        );
    }
    if held.get("_linear_project_statuses").is_none() {
        held["_linear_project_statuses"] = linear_statuses(LINEAR_SHARED_PROJECT_STATUSES);
    }
    held
}

/// `statuses` — name and `ProjectStatusType`, in the workspace's order — as this workspace
/// holds its project statuses: each one's id is its name, and its position its place.
pub fn linear_statuses(statuses: &[(&str, &str)]) -> Value {
    Value::Array(
        statuses
            .iter()
            .enumerate()
            .map(|(at, (name, kind))| {
                json!({"id":name,"name":name,"type":kind,"position":(at + 1) as f64})
            })
            .collect(),
    )
}

/// `states` — id, name and `WorkflowState.type` — as this workspace holds its team's states.
pub fn linear_states(states: &[(&str, &str, &str)]) -> Value {
    Value::Array(
        states
            .iter()
            .map(|(id, name, kind)| json!({"id":id,"name":name,"type":kind}))
            .collect(),
    )
}

/// The next instant of this workspace's clock, as Linear spells a `DateTime`.
///
/// A counter rather than the wall clock, so two writes in the same second still read back
/// in the order they were made, and an edit's `updatedAt` always moves.
fn linear_tick(data: &mut Value) -> (u64, String) {
    let tick = data["_linear_clock"].as_u64().unwrap_or(0) + 1;
    data["_linear_clock"] = json!(tick);
    let stamp = format!(
        "2026-09-01T{:02}:{:02}:{:02}Z",
        tick / 3600 % 24,
        tick / 60 % 60,
        tick % 60
    );
    (tick, stamp)
}

/// One held comment as Linear's `Comment` selection answers it.
fn linear_comment_node(row: &Value, data: &Value) -> Value {
    let issue = row["issue"].as_str().unwrap_or_default();
    let page = data["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["id"] == issue))
        .map(|task| linear_web_address(task, "issue"))
        .and_then(|url| url.as_str().map(str::to_owned))
        .unwrap_or_else(|| linear_address("issue", issue));
    json!({
        "id": row["id"],
        "body": row["body"],
        "url": format!("{page}#comment-{}", row["id"].as_str().unwrap_or_default()),
        "createdAt": row["createdAt"],
        "updatedAt": row["updatedAt"],
        "user": row["user"],
    })
}

/// The answer to one comment document, or `None` for any other operation.
fn linear_comment_response(
    data: &mut Value,
    vars: &Value,
    operation: &str,
) -> Option<Result<Value, &'static str>> {
    use onetaskgraph_linear::graphql;
    let holds_issue = |data: &Value, id: &str| {
        data["tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|task| task["id"] == id))
    };
    Some(match operation {
        graphql::ISSUE_COMMENTS => (|| {
            let id = vars["id"]
                .as_str()
                .ok_or("comment page id must be a string")?;
            if !holds_issue(data, id) {
                return Ok(json!({ "issue": null }));
            }
            // Newest first, because that is the order Linear lists a connection in — so a
            // plugin that walked it forwards would report the newest comment first here, as
            // it would against Linear, rather than passing against a fake in the other order.
            let newest_first = data["_linear_comments"]
                .as_array()
                .ok_or("fixture comments are not an array")?
                .iter()
                .rev()
                .filter(|row| row["issue"] == id)
                .map(|row| linear_comment_node(row, data))
                .collect::<Vec<_>>();
            // A cursor is a comment's own id rather than a position, as Linear's are opaque
            // and stable: a comment added between two pages must not shift the second one.
            let end = match vars["before"].as_str() {
                Some(cursor) => newest_first
                    .iter()
                    .position(|node| node["id"] == cursor)
                    .ok_or("unknown comment cursor")?,
                None => newest_first.len(),
            };
            let last = vars["last"].as_u64().ok_or("last must be a number")? as usize;
            let start = end.saturating_sub(last);
            let nodes = newest_first[start..end].to_vec();
            let start_cursor = nodes.first().map(|node| node["id"].clone());
            Ok(json!({"issue":{"archivedAt":null,"comments":{"nodes":nodes,
                "pageInfo":{"hasPreviousPage":start > 0,"startCursor":start_cursor}}}}))
        })(),
        graphql::COMMENT => (|| {
            let id = vars["id"].as_str().ok_or("comment id must be a string")?;
            let placed = data["_linear_comments"]
                .as_array()
                .ok_or("fixture comments are not an array")?
                .iter()
                .find(|row| row["id"] == id)
                .map(|row| {
                    json!({"id":row["id"],"body":row["body"],"archivedAt":null,
                                  "issue":{"id":row["issue"]}})
                });
            Ok(json!({ "comment": placed }))
        })(),
        graphql::COMMENT_CREATE => (|| {
            let input = &vars["input"];
            let issue = input["issueId"]
                .as_str()
                .ok_or("issueId must be a string")?;
            if !holds_issue(data, issue) {
                return Err("comment issue does not exist");
            }
            let (tick, stamp) = linear_tick(data);
            // Linear records the user whose key made the request, which is this workspace's
            // viewer.
            let row = json!({"id":format!("C-W{tick}"),"issue":issue,"body":input["body"],
                "createdAt":stamp,"updatedAt":stamp,"user":{"displayName":"fixture-user"}});
            let node = linear_comment_node(&row, data);
            data["_linear_comments"]
                .as_array_mut()
                .ok_or("fixture comments are not an array")?
                .push(row);
            Ok(json!({"commentCreate":{"success":true,"comment":node}}))
        })(),
        graphql::COMMENT_UPDATE => (|| {
            let id = vars["id"].as_str().ok_or("update id must be a string")?;
            let (_, stamp) = linear_tick(data);
            let rows = data["_linear_comments"]
                .as_array_mut()
                .ok_or("fixture comments are not an array")?;
            let row = rows
                .iter_mut()
                .find(|row| row["id"] == id)
                .ok_or("update target does not exist")?;
            row["body"] = vars["input"]["body"].clone();
            row["updatedAt"] = json!(stamp);
            let row = row.clone();
            Ok(json!({"commentUpdate":{"success":true,"comment":linear_comment_node(&row, data)}}))
        })(),
        graphql::COMMENT_DELETE => (|| {
            let id = vars["id"].as_str().ok_or("delete id must be a string")?;
            let rows = data["_linear_comments"]
                .as_array_mut()
                .ok_or("fixture comments are not an array")?;
            let at = rows
                .iter()
                .position(|row| row["id"] == id)
                .ok_or("delete target does not exist")?;
            rows.remove(at);
            Ok(json!({"commentDelete":{"success":true}}))
        })(),
        _ => return None,
    })
}

fn linear_write_relation(
    data: &mut Value,
    vars: &Value,
    project: bool,
) -> Result<Value, &'static str> {
    let input = vars["input"]
        .as_object()
        .ok_or("relation input must be an object")?;
    let near_key = if project { "projectId" } else { "issueId" };
    let far_key = if project {
        "relatedProjectId"
    } else {
        "relatedIssueId"
    };
    let wire = input
        .get("type")
        .and_then(Value::as_str)
        .ok_or("relation type must be a string")?;
    // Linear does not spell a project dependency the way it spells an issue's: a project
    // relation's ordering type is `dependency`, an issue relation's is `blocks`. This
    // responder refuses the other root's word for exactly that reason, so a write that
    // sent one would fail here rather than round-trip through a fixture that took it.
    //
    // And `related` is the issue vocabulary alone. Asked for a project relation typed
    // `related` on 2026-09-04 the real API refused it with `Argument Validation Error` and
    // `type must be one of the following values: dependency`, so this responder refuses it
    // too — the plugin now says so itself before the write, and a fixture that took the
    // value anyway would be the one place that never noticed if it stopped.
    let kind = match (project, wire) {
        (true, "dependency") | (false, "blocks") => "blocks",
        (false, "related") => "related",
        _ => return Err("undocumented relation type"),
    };
    // Linear anchors a project relation at both ends and requires both, so this responder
    // refuses an anchor it has no documented value for exactly as the real API refuses a
    // missing one.
    if project {
        for key in ["anchorType", "relatedAnchorType"] {
            let anchor = input
                .get(key)
                .and_then(Value::as_str)
                .ok_or("relation anchor must be a string")?;
            if !matches!(anchor, "start" | "end") {
                return Err("undocumented relation anchor");
            }
        }
    }
    let edge = json!({"from":input.get(near_key).ok_or("missing near id")?,"to":input.get(far_key).ok_or("missing far id")?,"kind":kind});
    data[if project {
        "project_dependencies"
    } else {
        "task_dependencies"
    }]
    .as_array_mut()
    .ok_or("fixture edges are not an array")?
    .push(edge);
    Ok(if project {
        json!({"projectRelationCreate":{"success":true,"projectRelation":{"id":"PR-W"}}})
    } else {
        json!({"issueRelationCreate":{"success":true,"issueRelation":{"id":"IR-W"}}})
    })
}

#[test]
// llmlint: ignore[tests_mirror_real_usage] This is a failure test for the fixture server's own untrusted HTTP boundary, which product CLI requests cannot malformedly exercise because the Linear client always emits valid typed requests; it intentionally sends raw TCP requests through the real socket rather than calling response logic directly.
fn linear_fixture_rejects_invalid_variables_and_unknown_operations() {
    let sandbox = Sandbox::new();
    let config = linear_block(&sandbox);
    let endpoint = config["endpoint"].as_str().unwrap();
    let address = endpoint
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/graphql")
        .unwrap();
    for body in [
        json!({"query":onetaskgraph_linear::graphql::VIEWER}),
        json!({"query":onetaskgraph_linear::graphql::VIEWER,"variables":[]}),
        json!({"query":"query { invented { id } }","variables":{}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{"id":7}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE,"variables":{"id":"i1","extra":true}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":0,"after":null,"filter":{}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":[]}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"invented":true}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"state":{"type":{"in":7}}}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"and":"invalid"}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUES,"variables":{"first":2,"after":null,"filter":{"project":{"null":"true"}}}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE_COMMENTS,"variables":{"id":"T-1","last":0,"before":null}}),
        json!({"query":onetaskgraph_linear::graphql::ISSUE_COMMENTS,"variables":{"id":"T-1","first":2,"after":null}}),
        json!({"query":onetaskgraph_linear::graphql::COMMENT_CREATE,"variables":{"input":{"issueId":"T-1","body":"text","createAsUser":"grace"}}}),
        json!({"query":onetaskgraph_linear::graphql::COMMENT_UPDATE,"variables":{"id":"C-1","input":{"body":""}}}),
        json!({"query":onetaskgraph_linear::graphql::COMMENT_DELETE,"variables":{"id":""}}),
    ] {
        let body = serde_json::to_string(&body).unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        write!(
            stream,
            "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{response}"
        );
    }
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(stream, "GET /graphql HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close the invalid-method fixture request before reading its response");
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));

    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(
        stream,
        "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: 9\r\n\r\n{{}}"
    )
    .unwrap();
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close the short-body fixture request before reading its response");
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));

    let mut stream = std::net::TcpStream::connect(address).unwrap();
    let oversized = "x".repeat(LINEAR_FIXTURE_REQUEST_LIMIT + 1);
    let _ = write!(
        stream,
        "POST /graphql HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n\r\n{oversized}",
        oversized.len()
    );
    // The fixture can reject and close this oversized request before the client reaches
    // shutdown, so a half-close here would race with the expected server response.
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    assert!(response.starts_with("HTTP/1.1 413 Content Too Large"));
}
/// The far ends `id` records under the reserved key, for a source with no native way to
/// name one: every qualified endpoint the dataset gives that item at `key`.
fn recorded_far_ends(key: &str, id: &Value) -> Vec<Value> {
    dataset()[key]
        .as_array()
        .expect("the dataset lists edges")
        .iter()
        .filter(|edge| edge["from"].get("id") == Some(id))
        .map(|edge| edge["to"].clone())
        .collect()
}
fn linear_label(v: &Value) -> Value {
    json!({"id":v["id"],"name":v["name"],"color":v.get("color").cloned().unwrap_or(Value::Null)})
}
/// Where the Linear row holds one entity: its own page there, as a link.
fn linear_place(_sandbox: &Sandbox, verb: &str, id: &str) -> Option<Placed> {
    let dataset = dataset();
    let held = dataset[format!("{verb}s")]
        .as_array()
        .expect("the shared dataset holds this kind")
        .iter()
        .find(|item| item["id"] == json!(id))
        .expect("the shared dataset holds this id");
    // Linear spells a task an issue, and the address says so.
    let kind = if verb == "task" { "issue" } else { verb };
    Some(Placed {
        key: "url",
        value: linear_web_address(held, kind)
            .as_str()
            .expect("a Linear web address is a string")
            .to_owned(),
    })
}

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This fixture-only mapping and matcher implement the finite shared journey dataset against the accepted 2026-08-24 contract; production parsing and real CLI row assertions independently verify the observable behavior without requiring live credentials.
fn linear_state(v: &Value) -> Value {
    if let Some(state) = v.get("_linear_state").filter(|state| state.is_object()) {
        return state.clone();
    }
    let v = &v["status"];
    let category = v["category"].as_str().unwrap_or("");
    json!({"name":v["name"],"type":match category{"todo"=>"unstarted","in-progress"=>"started","done"=>"completed","cancelled"=>"canceled",_=>"backlog"}})
}
/// The workflow states of this workspace's one team, in the order Linear lists them: id,
/// name, `WorkflowState.type`.
///
/// A real team's own names and types, as observed on 2026-10-01 — two states of type
/// `backlog`, two of type `unstarted`, review states typed `started`, `canceled` and
/// `duplicate`, and `Triage` — so a state written by name and one written as the first of its
/// type land in different places, and a journey can tell which a source did. The first state
/// of each type is the one this workspace has always answered a type lookup with, and keeps
/// the id it always had.
pub const LINEAR_TEAM_STATES: &[(&str, &str, &str)] = &[
    ("STATE-backlog", "Backlog", "backlog"),
    ("STATE-proposed", "Proposed", "backlog"),
    ("STATE-unstarted", "Todo", "unstarted"),
    ("STATE-queued", "Queued", "unstarted"),
    ("STATE-started", "In Progress", "started"),
    ("STATE-needs-attention", "Needs Attention", "started"),
    ("STATE-ready-for-review", "Ready for Review", "started"),
    ("STATE-in-review", "In Review", "started"),
    ("STATE-reviewed", "Reviewed", "started"),
    ("STATE-ready-to-merge", "Ready To Merge", "started"),
    ("STATE-blocked", "Blocked", "started"),
    ("STATE-completed", "Done", "completed"),
    ("STATE-canceled", "Canceled", "canceled"),
    ("STATE-cannot-reproduce", "Cannot Reproduce", "canceled"),
    ("STATE-duplicate", "Duplicate", "duplicate"),
    ("STATE-triage", "Triage", "triage"),
];

/// The two states the shared dataset's own names stand for, beside the team's: `Doing` and
/// `Shipped` are what [`linear_default_mapping`] writes `in-progress` and `done` as.
const LINEAR_SHARED_STATES: &[(&str, &str, &str)] = &[
    ("STATE-doing", "Doing", "started"),
    ("STATE-shipped", "Shipped", "completed"),
];

/// The workspace's project statuses, in Linear's order: name and `ProjectStatusType`. A name
/// for every category [`linear_default_mapping`] gives, so a project of any of them can be
/// written, and the example team's `In Progress` and `Done` beside them.
const LINEAR_SHARED_PROJECT_STATUSES: &[(&str, &str)] = &[
    ("Backlog", "backlog"),
    ("Todo", "planned"),
    ("Queued", "planned"),
    ("Doing", "started"),
    ("In Progress", "started"),
    ("Shipped", "completed"),
    ("Done", "completed"),
    ("Canceled", "canceled"),
];

/// The state of this workspace's team whose id is `id`: its name and its type.
fn linear_held_state(data: &Value, id: &str) -> Option<(String, String)> {
    data["_linear_states"].as_array()?.iter().find_map(|state| {
        (state["id"] == id).then(|| {
            (
                state["name"].as_str().unwrap_or_default().to_owned(),
                state["type"].as_str().unwrap_or_default().to_owned(),
            )
        })
    })
}

/// One issue row put at the team's workflow state `held`, which it then reads back as.
fn linear_put_state(row: &mut Value, (name, kind): (String, String)) {
    row["status"] = json!({"name": name, "category": linear_type_category(&kind)});
    row["_linear_state"] = json!({"name": name, "type": kind});
}

/// The project status of this workspace whose id is `id`, as a project row holds it.
fn linear_held_status(data: &Value, id: &str) -> Option<Value> {
    data["_linear_project_statuses"]
        .as_array()?
        .iter()
        .find(|status| status["id"] == id)
        .map(|status| json!({"name":status["name"],"type":status["type"]}))
}

/// What Linear answers a mutation addressing an id it does not hold with: an errored
/// response, `Entity not found: Issue`, rather than a null payload.
pub const LINEAR_ENTITY_MISSING: &str = "Entity not found: Issue";

/// The shared dataset's category for one `ProjectStatusType`, for a project row's own record.
fn linear_status_category(kind: &str) -> &'static str {
    match kind {
        "planned" => "todo",
        "started" | "paused" => "in-progress",
        "completed" => "done",
        "canceled" => "cancelled",
        "backlog" => "backlog",
        _ => "unknown",
    }
}

/// A workflow state of the team, or a project status of the workspace, created by
/// `sources fields --apply` — refused, as Linear refuses one, when the workspace has been
/// told to refuse that name, and when it already holds one so named.
fn linear_create_status_name(
    data: &mut Value,
    vars: &Value,
    project: bool,
) -> Result<Value, &'static str> {
    let input = &vars["input"];
    let name = input["name"].as_str().ok_or("name must be a string")?;
    if data["_linear_refuse_names"]
        .as_array()
        .is_some_and(|names| names.iter().any(|refused| refused == name))
    {
        return Err(LINEAR_REFUSED_CREATE);
    }
    let misanswer = data["_linear_misanswer_names"][name]
        .as_str()
        .map(str::to_owned);
    let root = if project {
        ("projectStatusCreate", "status")
    } else {
        ("workflowStateCreate", "workflowState")
    };
    if misanswer.as_deref() == Some("no-payload") {
        return Ok(json!({(root.0): {"success":true,(root.1): null}}));
    }
    let collection = if project {
        "_linear_project_statuses"
    } else {
        "_linear_states"
    };
    let held = data[collection]
        .as_array_mut()
        .ok_or("fixture vocabulary is not an array")?;
    if held.iter().any(|existing| {
        existing["name"]
            .as_str()
            .is_some_and(|existing| existing.eq_ignore_ascii_case(name))
    }) {
        return Err("a name the vocabulary already holds");
    }
    let kind = if misanswer.as_deref() == Some("other-type") {
        json!(if project { "paused" } else { "triage" })
    } else {
        input["type"].clone()
    };
    let name = if misanswer.as_deref() == Some("other-name") {
        format!("{name} (renamed)")
    } else {
        name.to_owned()
    };
    let created = if project {
        json!({"id":format!("STATUS-W{}", held.len() + 1),"name":name,"type":kind,
               "position":input["position"]})
    } else {
        json!({"id":format!("STATE-W{}", held.len() + 1),"name":name,"type":kind})
    };
    held.push(created.clone());
    Ok(if project {
        json!({"projectStatusCreate":{"success":true,"status":created}})
    } else {
        json!({"workflowStateCreate":{"success":true,"workflowState":created}})
    })
}

/// What the workspace says when it refuses to create a name it has been told to refuse.
pub const LINEAR_REFUSED_CREATE: &str = "You do not have permission to create that status";

/// The shared dataset's category for one workflow-state type.
fn linear_type_category(kind: &str) -> &'static str {
    match kind {
        "unstarted" => "todo",
        "started" => "in-progress",
        "completed" => "done",
        "canceled" => "cancelled",
        "backlog" => "backlog",
        _ => "unknown",
    }
}
/// A project's status, whose `type` is the `ProjectStatusType` enum and **not** the
/// workflow-state vocabulary above: `planned` is where an issue says `unstarted`, and
/// `paused` has no issue counterpart. A fake that answered a project with `unstarted`
/// would let a filter spelled in the wrong vocabulary pass here and fail against Linear,
/// which is exactly how `inIgnoreCase` reached a credentialed run.
fn linear_project_status(v: &Value) -> Value {
    if let Some(held) = v.get("_linear_status").filter(|held| held.is_object()) {
        return held.clone();
    }
    let category = v["category"].as_str().unwrap_or("");
    json!({"name":v["name"],"type":match category{"todo"=>"planned","in-progress"=>"started","done"=>"completed","cancelled"=>"canceled",_=>"backlog"}})
}
/// The `identifier` this workspace gives one issue, shared by the fake and the journey that
/// asserts on it so the two cannot drift. It only has to be distinct per issue: the plugin
/// never parses one.
pub fn linear_identifier(id: &str) -> String {
    format!("ENG-{}", id.replace('-', ""))
}

fn linear_handle(id: &str) -> Option<String> {
    Some(linear_identifier(id))
}

/// Linear's own number for one of the shared dataset's priorities: `0` none, `1` urgent, `2`
/// high, `3` medium — Linear's "normal" — and `4` low, sent as the `Float` its schema
/// declares.
fn linear_priority(priority: &Value) -> Value {
    json!(match priority.as_str().unwrap_or("none") {
        "urgent" => 1.0,
        "high" => 2.0,
        "medium" => 3.0,
        "low" => 4.0,
        _ => 0.0,
    })
}

/// The shared dataset's spelling of one of Linear's priority numbers.
fn linear_priority_name(number: &Value) -> Value {
    json!(match number.as_f64() {
        Some(1.0) => "urgent",
        Some(2.0) => "high",
        Some(3.0) => "medium",
        Some(4.0) => "low",
        _ => "none",
    })
}

/// One narrow `issueUpdate`: the `priority` alone or the `description` alone of an issue this
/// workspace holds, answered with what the issue holds afterwards.
fn linear_narrow_issue_update(
    data: &mut Value,
    vars: &Value,
    operation: &str,
) -> Result<Value, &'static str> {
    let id = vars["id"].as_str().ok_or("update id must be a string")?;
    let states = json!({"_linear_states": data["_linear_states"].clone()});
    let stored = vars["input"]
        .get("description")
        .map(|description| linear_stored(data, Some(description)));
    let (_, stamp) = linear_tick(data);
    let row = data["tasks"]
        .as_array_mut()
        .ok_or("fixture collection is not an array")?
        .iter_mut()
        .find(|row| row["id"] == id)
        .ok_or("update target does not exist")?;
    if let Some(priority) = vars["input"].get("priority") {
        row["priority"] = linear_priority_name(priority);
    }
    if let Some(description) = &stored {
        row["_linear_description"] = description.clone();
    }
    if let Some(title) = vars["input"].get("title") {
        row["title"] = title.clone();
    }
    if let Some(state) = vars["input"].get("stateId").and_then(Value::as_str) {
        // A narrow write names a state of the team, which the issue then reads back as.
        let held = linear_held_state(&states, state).ok_or("stateId names no state of the team")?;
        linear_put_state(row, held);
    }
    row["_updated"] = json!(stamp);
    Ok(
        if operation == onetaskgraph_linear::graphql::ISSUE_PRIORITY_UPDATE {
            json!({"issueUpdate":{"success":true,
                   "issue":{"id":id,"priority":linear_priority(&row["priority"])}}})
        } else {
            let mut issue = json!({"id":id});
            if let Some(description) = stored {
                issue["description"] = description;
            }
            json!({"issueUpdate":{"success":true,"issue":issue}})
        },
    )
}

fn linear_task(v: &Value, data: &Value) -> Value {
    let identifier = v
        .get("_identifier")
        .filter(|identifier| identifier.is_string())
        .cloned()
        .unwrap_or_else(|| json!(linear_identifier(v["id"].as_str().expect("an issue id"))));
    json!({"id":v["id"],"priority":linear_priority(&v["priority"]),"identifier":identifier,"title":v["title"],"description":linear_description(v,"task_dependencies",data),"state":linear_state(v),"labels":{"nodes":v["labels"].as_array().unwrap().iter().map(linear_label).collect::<Vec<_>>()},"project":v.get("project").map(|id|json!({"id":id})),"url":linear_web_address(v,"issue"),"createdAt":null,"updatedAt":linear_updated(v),"archivedAt":v.get("_trashed").cloned().unwrap_or(Value::Null)})
}
fn linear_project(v: &Value, data: &Value) -> Value {
    json!({"id":v["id"],"name":v["title"],"content":linear_description(v,"project_dependencies",data),"status":linear_project_status(&v["status"]),"labels":{"nodes":v["labels"].as_array().unwrap().iter().map(linear_label).collect::<Vec<_>>()},"url":linear_web_address(v,"project"),"createdAt":null,"updatedAt":linear_updated(v),"archivedAt":null})
}

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] The shared e2e harness is where the agreed e2e layout puts every loopback server, `linear_workspace` above among them, so no suite copies one: the engine's own suite and the Linear suite both drive this loopback Linear, so its answers are its own: the `updatedAt` every answer above carries is part of answering a request.
/// When this workspace last changed one item, as Linear's `updatedAt` answers it: the
/// workspace's epoch until something Linear moves that time for changes it.
fn linear_updated(v: &Value) -> Value {
    v.get("_updated")
        .filter(|updated| updated.is_string())
        .cloned()
        .unwrap_or_else(|| json!(LINEAR_EPOCH))
}

/// When every item of a loopback Linear workspace last changed before anything changed it —
/// the day its fixtures were recorded.
const LINEAR_EPOCH: &str = "2026-08-24T00:00:00Z";
// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

/// One Linear document. It has no `labels` field and no `state`, because Linear's own
/// published `Document` has neither — which is what the `linear` row of the table above
/// declares and what the shared document journeys drive.
fn linear_document(v: &Value) -> Value {
    json!({"id":v["id"],"title":v["title"],"content":linear_long_form(v,Vec::new()),"project":v.get("project").filter(|project|project.is_string()).map(|id|json!({"id":id})),"url":linear_web_address(v,"document"),"createdAt":null,"updatedAt":linear_updated(v),"archivedAt":null})
}

/// The Linear web address this workspace holds one item at.
///
/// Every item of a real Linear workspace has a page, so every item of this one does too —
/// which is what makes this row report a location for all three kinds. Where the shared
/// dataset states a url, that url *is* the address: a journey asserting on the one the
/// dataset gives must not find an invented one in its place.
fn linear_web_address(v: &Value, kind: &str) -> Value {
    v.get("url")
        .filter(|url| url.is_string())
        .cloned()
        .unwrap_or_else(|| json!(linear_address(kind, v["id"].as_str().unwrap_or_default())))
}

fn linear_address(kind: &str, id: &str) -> String {
    format!("https://linear.app/fixture/{kind}/{id}")
}

fn linear_description(v: &Value, edges: &str, data: &Value) -> String {
    if let Some(description) = v.get("_linear_description").and_then(Value::as_str) {
        return description.to_owned();
    }
    // No Linear relation can name an item of another source, so this is the one slot a
    // far end like that can be in.
    let far = data[edges]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| {
            edge["from"].get("id") == Some(&v["id"])
                && edge["to"]["id"].as_str().is_some_and(|id| id.contains(':'))
        })
        .map(|edge| edge["to"].clone())
        .collect::<Vec<_>>();
    linear_long_form(v, far)
}

/// The one long-form field a Linear item has, with this source's own slot at the end.
///
/// Shared by every kind this workspace serves, because the source reads all three out of
/// exactly the same slot. A document is handed no far ends: it is not work, so nothing
/// may depend on one and it has no dependency graph to record.
fn linear_long_form(v: &Value, far: Vec<Value>) -> String {
    if let Some(held) = v.get("_linear_description").and_then(Value::as_str) {
        return held.to_owned();
    }
    let mut metadata = v
        .get("metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(repositories) = v.get("repositories") {
        metadata.insert("onetaskgraph.repositories".into(), repositories.clone());
    }
    if !far.is_empty() {
        metadata.insert("onetaskgraph.depends_on".into(), Value::Array(far));
    }
    let content = v.get("content").and_then(Value::as_str).unwrap_or_default();
    if metadata.is_empty() {
        content.into()
    } else {
        linear_metadata_slot(content, &Value::Object(metadata))
    }
}

/// The one slot a Linear item keeps caller-defined metadata in: an HTML comment appended
/// to the description, which is what the source reads and what a person never sees.
fn linear_metadata_slot(content: &str, metadata: &Value) -> String {
    format!(
        "{content}\n\n<!-- onetaskgraph.metadata\n{}\n-->",
        serde_json::to_string(metadata).unwrap()
    )
}
fn linear_connection(rows: Vec<Value>, vars: &Value) -> Value {
    let start = vars["after"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let limit = vars["first"].as_u64().unwrap_or(50) as usize;
    let nodes = rows
        .iter()
        .skip(start)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let end = start + nodes.len();
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":end<rows.len(),"endCursor":if end<rows.len(){Some(end.to_string())}else{None}}})
}
/// Which of Linear's three filter inputs a filter object is: `IssueFilter`, `ProjectFilter` or
/// `DocumentFilter`, whose members differ.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LinearRow {
    Issue,
    Project,
    Document,
}

/// The team this workspace's rows are in unless one says otherwise: the one its sources are
/// configured with.
const LINEAR_TEAM: &str = "FIX";

/// Whether one held row satisfies one filter object, member by member, as Linear evaluates it.
///
/// Every member a source sends is evaluated rather than recognised by its spelling, so a
/// predicate a plugin sends and this workspace ignored could not pass a journey: the plugin
/// declares each of them native, and a row returned that the filter excludes, or one dropped
/// that it keeps, is what the journeys assert against. A member this does not know matches
/// nothing — [`valid_linear_filter`] has already refused it.
fn linear_holds(row: &Value, filter: &Value, kind: LinearRow, data: &Value) -> bool {
    let Some(members) = filter.as_object() else {
        return false;
    };
    members
        .iter()
        .all(|(member, wanted)| match member.as_str() {
            "and" => wanted
                .as_array()
                .is_some_and(|parts| parts.iter().all(|part| linear_holds(row, part, kind, data))),
            "or" => wanted
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| linear_holds(row, part, kind, data))),
            "team" => linear_compares(
                Some(row["team"].as_str().unwrap_or(LINEAR_TEAM)),
                &wanted["key"],
            ),
            "accessibleTeams" => linear_compares(
                Some(row["team"].as_str().unwrap_or(LINEAR_TEAM)),
                &wanted["some"]["key"],
            ),
            "labels" => {
                let names = row["labels"]
                    .as_array()
                    .map(|labels| {
                        labels
                            .iter()
                            .filter_map(|label| label["name"].as_str())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let some = wanted.get("some").is_none_or(|some| {
                    names
                        .iter()
                        .any(|name| linear_compares(Some(name), &some["name"]))
                });
                let every = wanted.get("every").is_none_or(|every| {
                    names
                        .iter()
                        .all(|name| linear_compares(Some(name), &every["name"]))
                });
                some && every
            }
            "state" => {
                let state = linear_state(row);
                wanted.as_object().is_some_and(|parts| {
                    parts
                        .iter()
                        .all(|(part, cmp)| linear_compares(state[part].as_str(), cmp))
                })
            }
            "status" => {
                let status = linear_project_status(&row["status"]);
                wanted.as_object().is_some_and(|parts| {
                    parts
                        .iter()
                        .all(|(part, cmp)| linear_compares(status[part].as_str(), cmp))
                })
            }
            "project" => {
                let held = row["project"].as_str();
                wanted.as_object().is_some_and(|parts| {
                    parts.iter().all(|(part, cmp)| match part.as_str() {
                        "null" => cmp.as_bool() == Some(held.is_none()),
                        "id" => linear_compares(held, cmp),
                        _ => false,
                    })
                })
            }
            "id" => linear_compares(row["id"].as_str(), wanted),
            "title" => linear_compares(row["title"].as_str(), wanted),
            "description" => {
                let held = match kind {
                    LinearRow::Issue => linear_description(row, "task_dependencies", data),
                    LinearRow::Project => linear_description(row, "project_dependencies", data),
                    LinearRow::Document => linear_long_form(row, Vec::new()),
                };
                linear_compares(Some(&held), wanted)
            }
            "priority" => {
                let held = linear_priority(&row["priority"]);
                wanted["in"].as_array().is_some_and(|levels| {
                    levels.iter().any(|level| level.as_f64() == held.as_f64())
                })
            }
            "comments" => {
                let issue = &row["id"];
                data["_linear_comments"].as_array().is_some_and(|comments| {
                    comments
                        .iter()
                        .filter(|comment| comment["issue"] == *issue)
                        .any(|comment| linear_comment_holds(comment, &wanted["some"]))
                })
            }
            _ => false,
        })
}

/// Whether one held comment satisfies one `CommentFilter`.
fn linear_comment_holds(comment: &Value, filter: &Value) -> bool {
    let Some(members) = filter.as_object() else {
        return false;
    };
    members
        .iter()
        .all(|(member, wanted)| match member.as_str() {
            "or" => wanted
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| linear_comment_holds(comment, part))),
            "createdAt" | "updatedAt" => {
                let at = |value: &Value| {
                    value
                        .as_str()
                        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
                };
                match (at(&comment[member]), at(&wanted["gte"])) {
                    (Some(held), Some(since)) => held >= since,
                    _ => false,
                }
            }
            _ => false,
        })
}

/// Whether `held` satisfies one of Linear's string comparators, every operator it carries.
fn linear_compares(held: Option<&str>, comparator: &Value) -> bool {
    let Some(operators) = comparator.as_object() else {
        return false;
    };
    operators.iter().all(|(operator, wanted)| {
        let text = wanted.as_str().unwrap_or_default();
        match (operator.as_str(), held) {
            ("eq", Some(held)) => held == text,
            ("eqIgnoreCase", Some(held)) => held.to_lowercase() == text.to_lowercase(),
            ("neqIgnoreCase", Some(held)) => held.to_lowercase() != text.to_lowercase(),
            ("neqIgnoreCase", None) => true,
            ("in", Some(held)) => wanted
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == held)),
            ("nin", Some(held)) => wanted
                .as_array()
                .is_some_and(|values| values.iter().all(|value| value != held)),
            ("contains", Some(held)) => held.contains(text),
            ("containsIgnoreCase", Some(held)) => {
                held.to_lowercase().contains(&text.to_lowercase())
            }
            _ => false,
        }
    })
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
fn linear_relations(
    data: &Value,
    key: &str,
    id: &str,
    suffix: &str,
    recorded: Option<&Value>,
) -> Value {
    let edges = data[key].as_array().unwrap();
    // A Linear relation names a Linear item, so only the edges whose ends are both plain
    // native ids are here. The rest are in the item's own description slot, which this
    // operation selects for exactly that reason.
    // The workspace holds one ordering kind and Linear has two words for it, so a
    // project's relations say `dependency` where an issue's say `blocks`.
    let wire = |kind: &Value| -> Value {
        if suffix == "Project" && kind == "blocks" {
            json!("dependency")
        } else {
            kind.clone()
        }
    };
    let forward = edges
        .iter()
        .enumerate()
        .filter(|(_,e)| e["from"] == id && e["to"].is_string())
        .map(|(index,e)| json!({"id":format!("relation:{index}"),"type":wire(&e["kind"]),(format!("related{suffix}")):{"id":e["to"]}}))
        .collect::<Vec<_>>();
    let inverse = edges
        .iter()
        .enumerate()
        .filter(|(_,e)| e["to"] == id && e["from"].is_string())
        .map(|(index,e)| json!({"id":format!("relation:{index}"),"type":wire(&e["kind"]),(suffix.to_ascii_lowercase()):{"id":e["from"]}}))
        .collect::<Vec<_>>();
    let items = if suffix == "Issue" {
        "tasks"
    } else {
        "projects"
    };
    let item = data[items]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id);
    // The description slot is where this source reads a recorded far end from, so a
    // workspace built to hold one puts it here and leaves every other operation alone.
    let description = match recorded {
        Some(recorded) => Some(linear_metadata_slot(
            "",
            &json!({"onetaskgraph.depends_on": recorded}),
        )),
        None => item.map(|item| linear_description(item, key, data)),
    };
    let long_form = if suffix == "Issue" {
        "description"
    } else {
        "content"
    };
    json!({(long_form):description,"relations":{"nodes":forward,"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":inverse,"pageInfo":{"hasNextPage":false,"endCursor":null}}})
}

/// The Markdown row's own configuration, for a journey that configures it itself.
///
/// Exported because the document round trip drives this source at both boundaries and as
/// both ends of a copy, which the table's one-source rows have no shape for — and planting
/// the fixture a second time would be a second dataset to keep in step with this one.
pub fn local_md_config(sandbox: &Sandbox) -> Value {
    local_md_block(sandbox)
}

fn local_md_block(sandbox: &Sandbox) -> Value {
    let root = sandbox.subdirectory("local-md");
    for (kind, id, front, body) in [
        (
            "tasks",
            "T-1",
            "title: Alpha engine\nstatus: Todo\npriority: high\nlabels: [{id: L-1, name: bug}, {id: L-3, name: core}]\nproject: P-1\nurl: https://example.invalid/T-1\nmetadata: {onepipeline.turn_budget: 12, caller.flags: [true, null]}\nrepositories: [github.com/nickderobertis/onetaskgraph]\ndepends_on: [T-2, {id: \"elsewhere:P-9\", item: project}]",
            "the engine core",
        ),
        (
            "tasks",
            "T-2",
            "title: Beta\nstatus: Shipped\nlabels: [{id: L-2, name: chore}]\nproject: P-1",
            "alpha in the body",
        ),
        (
            "tasks",
            "T-3",
            "title: Gamma\nstatus: Todo\npriority: urgent\nlabels: [{id: L-1, name: bug}]\ndepends_on: [T-2]",
            "unrelated",
        ),
        (
            "tasks",
            "T-4",
            "title: Delta docs\nstatus: Doing\npriority: low\nlabels: [{id: L-3, name: core}]\nproject: P-2\nmetadata: {onetaskgraph.origin: \"elsewhere:ORIG-4\", orchestrator.follow-up: {root_cause: stale-cache}}\ndepends_on:\n  - id: T-2\n    kind: related",
            "documentation",
        ),
        (
            "projects",
            "P-1",
            "title: Engine\nstatus: Doing\nlabels: [{id: L-3, name: core}]\nurl: https://example.invalid/P-1\nmetadata: {onepipeline.publication: {mode: review}}\nrepositories: [github.com/nickderobertis/onetaskgraph]\ndepends_on: [P-2, {id: \"elsewhere:T-9\", item: task}]",
            "the engine",
        ),
        ("projects", "P-2", "title: Docs\nstatus: Todo", "alpha docs"),
        // `documents/`, the third folder: no `status:` and no `depends_on:`, because a
        // document is not work and this source refuses either key under here.
        (
            "documents",
            "D-1",
            "title: Alpha design\nlabels: [{id: L-1, name: bug}]\nproject: P-1\nmetadata: {onepipeline.turn_budget: 12, caller.flags: [true, null]}\nrepositories: [github.com/nickderobertis/onetaskgraph]",
            "the engine core, reviewed",
        ),
        (
            "documents",
            "D-2",
            "title: Runbook\nlabels: [{id: L-3, name: core}]\nproject: P-2",
            "how to read the alpha design",
        ),
        ("documents", "D-3", "title: Loose note", "filed nowhere"),
    ] {
        let path = root.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
        std::fs::write(path, format!("---\n{front}\n---\n{body}\n")).expect("Markdown fixture");
    }
    json!({ "root": root, "status_mapping": {"todo":"todo", "doing":"in-progress", "shipped":"done"} })
}

/// Where the Markdown row holds one entity: the file behind it, by absolute path.
///
/// Canonicalized here as well as in the plugin, because the two have to be the same string
/// on a host whose temporary tree is reached through a link — which is every macOS runner,
/// and is why comparing against the sandbox path unresolved would pass on Linux alone.
fn local_md_place(sandbox: &Sandbox, verb: &str, id: &str) -> Option<Placed> {
    let path = sandbox
        .subdirectory("local-md")
        .join(format!("{verb}s"))
        .join(format!("{id}.md"));
    Some(Placed {
        key: "path",
        value: std::fs::canonicalize(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            .to_string_lossy()
            .into_owned(),
    })
}

/// The `in-memory` row that applies every predicate itself.
fn native_block(_sandbox: &Sandbox) -> Value {
    let mut block = dataset();
    block["capabilities"] = native_capabilities();
    block
}

/// The capability block the row that applies everything itself declares.
///
/// Spelled once and exported because three journey modules configure this same source
/// outside the table, and `documents` is a key a caller has to remember: the shared
/// dataset holds documents, and a source declaring it has none while holding some is
/// refused where its configuration is read.
pub fn native_capabilities() -> Value {
    json!({"documents": "native", "comments": "native", "max_page_size": 50})
}

/// The row that runs the same dataset in a second process, over the stdio protocol.
///
/// This is journey 19 — every journey again, through a subprocess-wrapped source — and it
/// is a row rather than a suite of its own for the reason the whole table exists: a
/// transport proven by tests written for it is proven against its author's expectations,
/// and this one has to answer the same assertions every in-process source answers. It
/// declares everything native because the source behind the pipe does, which is the claim
/// worth making: what a source can do must not change because it is a process away.
fn hosted_block(_sandbox: &Sandbox) -> Value {
    let mut settings = dataset();
    settings["capabilities"] = native_capabilities();
    json!({
        "command": crate::binary(),
        "args": ["plugin-serve", "in-memory"],
        "settings": settings,
    })
}

/// The `in-memory` row that applies none of them, and pages two rows at a time.
///
/// A small page ceiling on purpose: compensation has to walk more than one page to find
/// the rows a filter keeps, and a ceiling of two is what makes a journey notice when it
/// stops doing so.
fn compensated_block(_sandbox: &Sandbox) -> Value {
    let mut block = dataset();
    block["capabilities"] = json!({
        // `documents` is not a predicate: it says this source *holds* documents, which it
        // does. Declaring it unsupported here would contribute no document rows at all and
        // leave the engine's document compensation with no coverage rather than less of it.
        "documents": "native",
        // Not a predicate either, for the same reason: it says this source's tasks hold
        // comments, and the comment journeys drive this row as well as the native one.
        "comments": "native",
        "filter_by_label": "unsupported",
        "filter_by_status": "unsupported",
        "filter_by_priority": "unsupported",
        "filter_by_comment_activity": "unsupported",
        "filter_by_metadata": "unsupported",
        "filter_by_origin": "unsupported",
        "search_title": "unsupported",
        "search_content": "unsupported",
        "orphan_tasks": "unsupported",
        "task_dependencies": "forward-only",
        "project_dependencies": "forward-only",
        "max_page_size": 2
    });
    block
}

/// The work every row serves: four tasks, three of which are in a project, three labels,
/// two projects, three documents, and a dependency graph with a reverse answer worth
/// checking.
///
/// The documents are what only a source that *has* documents can serve, so they are here
/// rather than in a fixture of their own: the rows that hold them and the rows that
/// declare they hold none read the same table, and the shared journeys drive both against
/// it. One document carries a link and one a path, so a journey can assert that a consumer
/// tells the two shapes apart; the third carries neither, because "the source did not say"
/// is its own case. `T-1` and `P-1` carry a location apiece for the same reason — location
/// was added to all three entities, and a fixture with it on documents alone would leave
/// the other two renderings unproven.
///
/// Exactly one of the two projects carries a label, and the two sit in different status
/// categories, so every project filter has something to keep and something to drop —
/// a filter both projects satisfied would pass against a source that ignored it.
///
/// The dependency edges are listed in the order their `from` items are, which is what
/// makes the engine's emulated reverse scan — item by item, each item's forward edges in
/// order — produce the *same sequence* a source answering natively does, rather than the
/// same set in another order. A fixture that shuffled them would make the two answers
/// compare unequal for a reason that has nothing to do with the engine.
///
/// Two of the edges leave this source altogether — one from a task to a project, one from
/// a project to a task, both in a source called `elsewhere` that is not configured at all.
/// They are here rather than in a journey of their own because *where* such an edge is
/// held is each source's own business: a native relation that can name the far end, and
/// the reserved key on the near item where none can. Every row below encodes these two in
/// its own way, and one journey asserts that all of them report the same edge.
pub fn dataset() -> Value {
    json!({
        "tasks": [
            {"id": "T-1", "title": "Alpha engine", "content": "the engine core",
             "status": {"category": "todo", "name": "Todo"}, "priority": "high",
             "labels": [{"id": "L-1", "name": "bug"}, {"id": "L-3", "name": "core"}],
            "project": "P-1", "url": "https://example.invalid/T-1",
            "location": {"url": "https://example.invalid/T-1"},
            "metadata": {"onepipeline.turn_budget": 12, "caller.flags": [true, null]},
            "repositories": ["github.com/nickderobertis/onetaskgraph"]},
            {"id": "T-2", "title": "Beta", "content": "alpha in the body",
             "status": {"category": "done", "name": "Shipped"},
             "labels": [{"id": "L-2", "name": "chore"}], "project": "P-1"},
            {"id": "T-3", "title": "Gamma", "content": "unrelated",
             "status": {"category": "todo", "name": "Todo"}, "priority": "urgent",
             "labels": [{"id": "L-1", "name": "bug"}]},
            {"id": "T-4", "title": "Delta docs", "content": "documentation",
             "status": {"category": "in-progress", "name": "Doing"}, "priority": "low",
             "labels": [{"id": "L-3", "name": "core"}], "project": "P-2",
             "metadata": {"onetaskgraph.origin": "elsewhere:ORIG-4",
                          "orchestrator.follow-up": {"root_cause": "stale-cache"}}}
        ],
        "projects": [
            {"id": "P-1", "title": "Engine", "content": "the engine",
             "status": {"category": "in-progress", "name": "Doing"},
             "labels": [{"id": "L-3", "name": "core"}],
             "url": "https://example.invalid/P-1",
             "location": {"path": "/srv/engine"},
             "metadata": {"onepipeline.publication": {"mode": "review"}},
             "repositories": ["github.com/nickderobertis/onetaskgraph"]},
            {"id": "P-2", "title": "Docs", "content": "alpha docs",
             "status": {"category": "todo", "name": "Todo"}, "labels": []}
        ],
        "documents": [
            {"id": "D-1", "title": "Alpha design", "content": "the engine core, reviewed",
             "project": "P-1", "labels": [{"id": "L-1", "name": "bug"}],
             "location": {"url": "https://example.invalid/D-1"},
             "metadata": {"onepipeline.turn_budget": 12, "caller.flags": [true, null]},
             "repositories": ["github.com/nickderobertis/onetaskgraph"]},
            {"id": "D-2", "title": "Runbook", "content": "how to read the alpha design",
             "project": "P-2", "labels": [{"id": "L-3", "name": "core"}],
             "location": {"path": "/srv/notes/D-2.md"}},
            {"id": "D-3", "title": "Loose note", "content": "filed nowhere",
             "project": null, "labels": []}
        ],
        "labels": [
            {"id": "L-1", "name": "bug"},
            {"id": "L-2", "name": "chore"},
            {"id": "L-3", "name": "core"}
        ],
        "task_dependencies": [
            {"from": "T-1", "to": "T-2", "kind": "blocks"},
            {"from": {"id": "T-1", "kind": "task"},
             "to": {"id": "elsewhere:P-9", "kind": "project"}, "kind": "blocks"},
            {"from": "T-3", "to": "T-2", "kind": "blocks"},
            {"from": "T-4", "to": "T-2", "kind": "related"}
        ],
        "project_dependencies": [
            {"from": "P-1", "to": "P-2", "kind": "blocks"},
            {"from": {"id": "P-1", "kind": "project"},
             "to": {"id": "elsewhere:T-9", "kind": "task"}, "kind": "blocks"}
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::IssueSearch;

    #[test]
    fn each_phrase_of_a_search_may_be_held_by_either_field() {
        // GitHub admits an issue when every phrase is in a field searched, whichever field
        // holds it; one phrase in the title and another in the body is such an issue.
        let search = IssueSearch::parse(r#"in:title,body "alpha plan" beta"#);
        assert!(search.admits("The alpha plan", "beta, in the body"));
        assert!(search.admits("Nothing", "Beta follows the ALPHA PLAN."));
        assert!(!search.admits("The alpha plan", "nothing else"));
        assert!(!search.admits("alpha and plan apart", "beta"));
        let titles = IssueSearch::parse(r#"in:title "alpha plan""#);
        assert!(!titles.admits("Nothing", "the alpha plan"));
        assert!(titles.admits("The Ålpha plan or the alpha plan", ""));
    }
}
