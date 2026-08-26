//! What a person sees.
//!
//! Machine-readable output is the response types themselves, serialised — a stable
//! contract an SDK is generated from, and validated in the journeys against the schema
//! this binary emits. Everything in this file is the *other* output: minimal, aligned,
//! and deliberately not a second contract. Nothing here reshapes a response; it renders
//! one.
//!
//! Every vocabulary a line spells — a status category, a predicate, a dependency kind —
//! is taken from the type's own `Serialize`, never from a `match` written out again
//! here. A second spelling of `in-progress` in this file would be a second place for it
//! to drift from the one a filter compares against.

use onetaskgraph_core::{
    CopyReport, Predicate, Qualified, QualifiedEdge, QueryPlan, SearchHit, SourceListing,
    SourceState,
};
use onetaskgraph_plugin_api::{Capabilities, Label, Project, Support, Task};
use serde::Serialize;

/// One value as the wire spells it — `in-progress`, `search-title`, `blocks`.
///
/// Every caller passes a unit-like enum of the contract, which serialises to a quoted
/// string and cannot fail; stripping the quotes is the whole of the work. Taking the
/// spelling from `Serialize` rather than from a `match` written out again here is what
/// stops a second spelling of `in-progress` existing to drift from the one a filter
/// compares against.
fn wire(value: &impl Serialize) -> String {
    serde_json::to_string(value)
        .expect("a contract enum serialises")
        .trim_matches('"')
        .to_owned()
}

/// Lay `rows` out as aligned columns, one line each.
///
/// The last column is never padded, so nothing trails a line with blanks that a shell
/// pipeline would then have to strip.
fn columns(rows: &[Vec<String>]) -> String {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..width)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();

    let mut rendered = String::new();
    for row in rows {
        let last = row.len().saturating_sub(1);
        for (index, cell) in row.iter().enumerate() {
            if index == last {
                rendered.push_str(cell);
            } else {
                let pad = widths[index].saturating_sub(cell.chars().count());
                rendered.push_str(cell);
                rendered.push_str(&" ".repeat(pad));
                rendered.push_str("  ");
            }
        }
        rendered.push('\n');
    }
    rendered
}

/// One line per task: qualified id, normalised status, title.
///
/// The normalised category rather than the source's own wording, because this list
/// crosses sources and the category is the one vocabulary they share — and it is what
/// `--status` compares against. `task show` prints both.
pub fn tasks(items: &[Qualified<Task>]) -> String {
    columns(
        &items
            .iter()
            .map(|task| {
                vec![
                    task.id.to_string(),
                    wire(&task.item.status.category),
                    task.item.title.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    )
}

/// One line per project: qualified id, normalised status, title.
pub fn projects(items: &[Qualified<Project>]) -> String {
    columns(
        &items
            .iter()
            .map(|project| {
                vec![
                    project.id.to_string(),
                    wire(&project.item.status.category),
                    project.item.title.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    )
}

/// One line per item a copy considered: where it came from, where it went, what happened.
///
/// A dry run that would create has no destination id to print, because nothing was
/// created and inventing one would be a claim about an id the destination never issued.
pub fn copied(report: &CopyReport) -> String {
    columns(
        &report
            .items
            .iter()
            .map(|outcome| {
                vec![
                    outcome.source.to_string(),
                    outcome
                        .destination()
                        .map_or_else(|| "-".to_owned(), ToString::to_string),
                    outcome.action.name(),
                ]
            })
            .collect::<Vec<_>>(),
    )
}

/// One line per label: qualified id and the name a filter types.
pub fn labels(items: &[Qualified<Label>]) -> String {
    columns(
        &items
            .iter()
            .map(|label| vec![label.id.to_string(), label.item.name.clone()])
            .collect::<Vec<_>>(),
    )
}

/// One line per edge: where it starts, what it means, where it points.
pub fn edges(items: &[QualifiedEdge]) -> String {
    columns(
        &items
            .iter()
            .map(|edge| {
                vec![
                    format!("{} {}", wire(&edge.from.kind), edge.from.id),
                    wire(&edge.kind),
                    format!("{} {}", wire(&edge.to.kind), edge.to.id),
                ]
            })
            .collect::<Vec<_>>(),
    )
}

/// One line per hit, saying which entity matched.
pub fn hits(items: &[SearchHit]) -> String {
    columns(
        &items
            .iter()
            .map(|hit| match hit {
                SearchHit::Task(task) => vec![
                    "task".to_owned(),
                    task.id.to_string(),
                    task.item.title.clone(),
                ],
                SearchHit::Project(project) => vec![
                    "project".to_owned(),
                    project.id.to_string(),
                    project.item.title.clone(),
                ],
            })
            .collect::<Vec<_>>(),
    )
}

/// One task in full, body last.
pub fn task_detail(task: &Qualified<Task>) -> String {
    let item = &task.item;
    let mut fields = vec![
        ("id", task.id.to_string()),
        ("title", item.title.clone()),
        (
            "status",
            format!("{} ({})", wire(&item.status.category), item.status.name),
        ),
    ];
    fields.push((
        "project",
        match &item.project {
            Some(project) => format!("{}:{project}", task.id.source),
            None => "none".to_owned(),
        },
    ));
    detail(&mut fields, &item.labels, item.url.as_deref());
    body(&fields, item.content.as_deref())
}

/// One project in full, body last.
pub fn project_detail(project: &Qualified<Project>) -> String {
    let item = &project.item;
    let mut fields = vec![
        ("id", project.id.to_string()),
        ("title", item.title.clone()),
        (
            "status",
            format!("{} ({})", wire(&item.status.category), item.status.name),
        ),
    ];
    detail(&mut fields, &item.labels, item.url.as_deref());
    body(&fields, item.content.as_deref())
}

/// The fields a task and a project share below their own.
fn detail(fields: &mut Vec<(&'static str, String)>, item_labels: &[Label], url: Option<&str>) {
    if !item_labels.is_empty() {
        fields.push((
            "labels",
            item_labels
                .iter()
                .map(|label| label.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if let Some(url) = url {
        fields.push(("url", url.to_owned()));
    }
}

/// The field table, then the long-form body under a blank line.
fn body(fields: &[(&'static str, String)], content: Option<&str>) -> String {
    let mut rendered = columns(
        &fields
            .iter()
            .map(|(name, value)| vec![format!("{name}:"), value.clone()])
            .collect::<Vec<_>>(),
    );
    if let Some(content) = content.map(str::trim).filter(|body| !body.is_empty()) {
        rendered.push('\n');
        rendered.push_str(content);
        rendered.push('\n');
    }
    rendered
}

/// One line per configured source: what it is, and what it says it can do.
pub fn sources(listings: &[SourceListing]) -> String {
    columns(
        &listings
            .iter()
            .map(|listing| {
                vec![
                    listing.source.to_string(),
                    listing.kind.clone(),
                    match &listing.state {
                        SourceState::Available { capabilities } => declared(capabilities),
                        SourceState::Unavailable { error } => {
                            format!("unavailable — {error}")
                        }
                    },
                ]
            })
            .collect::<Vec<_>>(),
    )
}

/// What one source applies itself, in one line.
fn declared(capabilities: &Capabilities) -> String {
    let native: Vec<&str> = [
        ("label", capabilities.filter_by_label),
        ("status", capabilities.filter_by_status),
        ("search-title", capabilities.search_title),
        ("search-content", capabilities.search_content),
        ("project", capabilities.projects),
        ("orphan-tasks", capabilities.orphan_tasks),
    ]
    .into_iter()
    .filter(|(_, support)| *support == Support::Native)
    .map(|(name, _)| name)
    .collect();

    format!(
        "native: {}; deps: task {}, project {}; page <= {}",
        if native.is_empty() {
            "none".to_owned()
        } else {
            native.join(", ")
        },
        wire(&capabilities.task_dependencies),
        wire(&capabilities.project_dependencies),
        capabilities.max_page_size,
    )
}

/// The plan, per source, with only the lines that have something to say.
///
/// This is the whole reason capability declaration exists: two sources answer the same
/// query by two different plans and both answers are correct, and without this a caller
/// could only guess which of the two they got.
pub fn plan(plan: &QueryPlan) -> String {
    let mut rendered = String::from("plan:\n");
    if plan.per_source.is_empty() {
        rendered.push_str("  (no source was addressed)\n");
        return rendered;
    }
    for source in &plan.per_source {
        rendered.push_str(&format!(
            "  {} ({})  {} page(s)\n",
            source.source, source.kind, source.pages_fetched
        ));
        for (label, predicates) in [
            ("pushed down", &source.pushed_down),
            ("applied locally", &source.applied_locally),
            ("emulated", &source.emulated),
            ("unavailable", &source.unavailable),
        ] {
            if predicates.is_empty() {
                continue;
            }
            rendered.push_str(&format!("    {label}: {}\n", predicate_list(predicates)));
        }
    }
    rendered
}

/// Predicate names, in the wire spelling `--json` publishes.
fn predicate_list(predicates: &[Predicate]) -> String {
    predicates.iter().map(wire).collect::<Vec<_>>().join(", ")
}
