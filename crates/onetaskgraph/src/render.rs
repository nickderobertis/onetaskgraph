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

use onetaskgraph_core::config::SourceRoute;
use onetaskgraph_core::{
    CommentList, CopyReport, DeletedComment, Delivered, DeliveryOutcome, GlobalId, MetadataSet,
    Predicate, Qualified, QualifiedEdge, QueryPlan, Regenerated, SearchHit, SourceListing,
    SourceState, TaskContentSet, TaskDetails, TaskPrioritySet, TaskStatusSet, TaskUpdated,
    TemplateVariables,
};
use onetaskgraph_plugin_api::{
    Asset, Capabilities, Comment, Document, Label, Location, MetadataKey, Priority, Project,
    Support, Task, TaskRef,
};
use onetaskgraph_status_options::{FieldOutcome, FieldsReport, StatusOptionsReport};
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

/// A concise read-only plan or verified apply result for board Status options.
pub fn status_options(report: &StatusOptionsReport) -> String {
    let missing = if report.missing.is_empty() {
        "none".to_owned()
    } else {
        report.missing.join(", ")
    };
    match report.outcome {
        onetaskgraph_status_options::StatusOptionsOutcome::Applied => {
            format!("{}: added and verified: {missing}\n", report.source)
        }
        onetaskgraph_status_options::StatusOptionsOutcome::Planned
        | onetaskgraph_status_options::StatusOptionsOutcome::Unchanged => format!(
            "{}: missing configured Status options: {missing}\n",
            report.source
        ),
    }
}

/// A concise read-only plan or verified apply result for every field a board setup names:
/// one line per field.
pub fn fields(report: &FieldsReport) -> String {
    let mut rendered = String::new();
    for field in &report.fields {
        // A Status name says which kind it is configured for — `task: Queued; project:
        // Shipped` — because one field holds both kinds' options.
        let missing = if field.missing.is_empty() {
            "none".to_owned()
        } else if field.kinds.is_empty() {
            field.missing.join(", ")
        } else {
            field
                .kinds
                .iter()
                .map(|kind| format!("{}: {}", wire(&kind.kind), kind.missing.join(", ")))
                .collect::<Vec<_>>()
                .join("; ")
        };
        let name = field.field.name();
        let line = match (field.outcome, field.exists) {
            (FieldOutcome::Created, _) => {
                format!("created the {name} field with: {missing}")
            }
            (FieldOutcome::Applied, _) => format!("added and verified {name} options: {missing}"),
            (FieldOutcome::Planned, false) => {
                format!("no {name} field; would create it with: {missing}")
            }
            (FieldOutcome::Planned | FieldOutcome::Unchanged, _) => {
                format!("missing configured {name} options: {missing}")
            }
        };
        rendered.push_str(&format!("{}: {line}\n", report.source));
    }
    rendered
}

/// What `sources fields` reports for a `linear` source: one line per name its `status_mapping`
/// gives a kind — a task's against the team's workflow states, a project's against the
/// workspace's project statuses — present with its type, created by this run, or missing. A
/// create Linear refused is the command's failure, which it reports on stderr.
pub fn status_names(report: &onetaskgraph_linear::StatusNamesReport) -> String {
    let word = |value: serde_json::Value| value.as_str().map(str::to_owned).unwrap_or_default();
    if report.names.is_empty() {
        return format!("{}: status_mapping names no status\n", report.source);
    }
    let mut rendered = String::new();
    for mapped in &report.names {
        let kind = word(serde_json::to_value(mapped.kind()).unwrap_or_default());
        let category = word(serde_json::to_value(mapped.category()).unwrap_or_default());
        let held_by = match mapped.kind() {
            onetaskgraph_plugin_api::ItemKind::Task => {
                format!("workflow state of team {}", report.team())
            }
            onetaskgraph_plugin_api::ItemKind::Project => {
                "project status of this workspace".to_owned()
            }
        };
        let found = match mapped.found() {
            onetaskgraph_linear::Found::Created(found) => {
                format!("created as a {held_by} ({found})")
            }
            onetaskgraph_linear::Found::Present(found) if found == mapped.expected_type() => {
                format!("present as a {held_by} ({found})")
            }
            onetaskgraph_linear::Found::Present(found) => format!(
                "present as a {held_by} ({found}; {category} is created as {}, and this one is left as it is)",
                mapped.expected_type()
            ),
            onetaskgraph_linear::Found::Missing => {
                format!("missing: no {held_by} has that name")
            }
        };
        rendered.push_str(&format!(
            "{}: {kind} {category} -> {}: {found}\n",
            report.source,
            mapped.name()
        ));
    }
    rendered
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

/// One line per task: qualified id, normalised status, title — and, for a task that delivers
/// or is delivered by anything, one more cell naming each entry qualified.
///
/// The normalised category rather than the source's own wording, because this list
/// crosses sources and the category is the one vocabulary they share — and it is what
/// `--status` compares against. `task show` prints both.
pub fn tasks(items: &[Qualified<Task>]) -> String {
    columns(
        &items
            .iter()
            .map(|task| {
                let mut row = vec![
                    task.id.to_string(),
                    wire(&task.item.status.category),
                    task.item.title.clone(),
                ];
                let related = relation(&task.item);
                if !related.is_empty() {
                    row.push(related);
                }
                row
            })
            .collect::<Vec<_>>(),
    )
}

/// What a task delivers and what delivers it, in one cell: empty when it does neither.
fn relation(task: &Task) -> String {
    let mut said = Vec::new();
    if !task.delivers.is_empty() {
        said.push(format!("delivers {}", listed(&task.delivers)));
    }
    if !task.delivered_by.is_empty() {
        said.push(format!("delivered by {}", listed(&task.delivered_by)));
    }
    said.join("; ")
}

/// A task list's entries, comma-separated, exactly as the verb qualified them.
fn listed(entries: &[TaskRef]) -> String {
    entries
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// What `task status set` did: the task, the status it now reads as, and each task it
/// delivers that was kept in step.
pub fn status_set(set: &TaskStatusSet) -> String {
    let mut rendered = columns(&[
        vec!["id:".to_owned(), set.id.to_string()],
        vec![
            "status:".to_owned(),
            format!("{} ({})", wire(&set.status.category), set.status.name),
        ],
    ]);
    if set.delivered.is_empty() {
        rendered.push_str("delivered: none\n");
    } else {
        rendered.push_str(&delivered(&set.delivered));
    }
    rendered
}

/// What `task update` did: the task, which of its fields were written — or that none needed
/// to be — the status it now reads as, and each task it delivers that was kept in step.
pub fn task_updated(updated: &TaskUpdated) -> String {
    let written = if updated.written.is_empty() {
        "nothing: every field named already held that value".to_owned()
    } else {
        updated
            .written
            .iter()
            .map(wire)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut rendered = columns(&[
        vec!["id:".to_owned(), updated.id.to_string()],
        vec!["written:".to_owned(), written],
        vec![
            "status:".to_owned(),
            format!(
                "{} ({})",
                wire(&updated.task.status.category),
                updated.task.status.name
            ),
        ],
    ]);
    if !updated.delivered.is_empty() {
        rendered.push_str(&delivered(&updated.delivered));
    }
    rendered
}

/// What `task priority set` did: the task, and the priority its source now reads it as.
pub fn priority_set(set: &TaskPrioritySet) -> String {
    columns(&[
        vec!["id:".to_owned(), set.id.to_string()],
        vec!["priority:".to_owned(), wire(&set.priority)],
    ])
}

/// What `task content set` did: the task whose content was replaced.
pub fn content_set(set: &TaskContentSet) -> String {
    columns(&[vec!["id:".to_owned(), set.id.to_string()]])
}

/// What a `task render` or `document render` did: the item, whether it changed — or would
/// have, for a dry run — and the two digests its provenance now records.
pub fn regenerated(regenerated: &Regenerated, dry_run: bool) -> String {
    let changed = match (regenerated.changed, dry_run) {
        (true, false) => "yes",
        (true, true) => "yes (dry run: nothing written)",
        (false, _) => "no",
    };
    columns(&[
        vec!["id:".to_owned(), regenerated.id.to_string()],
        vec!["changed:".to_owned(), changed.to_owned()],
        vec!["digest:".to_owned(), regenerated.digest.to_string()],
        vec![
            "body digest:".to_owned(),
            regenerated.body_digest.to_string(),
        ],
    ])
}

/// What a `metadata set` verb did: the record, the key, the value its source now holds there
/// as compact JSON, and where the record is when its source says.
pub fn metadata_set(set: &MetadataSet) -> String {
    let mut rows = vec![
        vec!["id:".to_owned(), set.id.to_string()],
        vec!["key:".to_owned(), set.key.to_string()],
        vec!["value:".to_owned(), set.value.to_string()],
    ];
    if let Some(location) = &set.location {
        rows.push(vec!["location:".to_owned(), located(location)]);
    }
    columns(&rows)
}

/// A template's declared set: its name and digest, then one row per variable — its name, its
/// type, whether it is required or what it defaults to, the file declaring it, and what it
/// is for.
pub fn template_variables(described: &TemplateVariables) -> String {
    let mut rendered = columns(&[
        vec!["template:".to_owned(), described.template.clone()],
        vec!["digest:".to_owned(), described.digest.clone()],
    ]);
    if described.variables.is_empty() {
        rendered.push_str("\n(it declares no variables)\n");
        return rendered;
    }
    let rows: Vec<Vec<String>> = described
        .variables
        .iter()
        .map(|variable| {
            vec![
                variable.name().to_owned(),
                match variable.items() {
                    Some(items) => format!("{}<{}>", variable.kind(), items.as_str()),
                    None => variable.kind().to_string(),
                },
                match (variable.default(), variable.required()) {
                    (Some(default), _) => format!("default {default}"),
                    (None, true) => "required".to_owned(),
                    (None, false) => "optional".to_owned(),
                },
                variable.declared_in().to_owned(),
                variable.description().to_owned(),
            ]
        })
        .collect();
    rendered.push('\n');
    rendered.push_str(&columns(&rows));
    rendered
}

/// One line per delivered task a write kept in step with a deliverer, saying in words what
/// happened to it: the same entries the machine output carries under `delivered`.
///
/// A failure is its first line alone, so each task stays one line; standard error carries the
/// whole of it, next action included.
pub fn delivered(entries: &[Delivered]) -> String {
    let mut rendered = String::new();
    for entry in entries {
        let what = match &entry.outcome {
            DeliveryOutcome::Written { from, to } => {
                format!("written from {} to {}", wire(from), wire(to))
            }
            DeliveryOutcome::Unchanged { from } => format!("unchanged at {}", wire(from)),
            DeliveryOutcome::Left { from } => format!("left at {}", wire(from)),
            DeliveryOutcome::Failed { from, failure } => format!(
                "failed{}: {}",
                from.as_ref()
                    .map(|from| format!(" at {}", wire(from)))
                    .unwrap_or_default(),
                failure.message().lines().next().unwrap_or_default()
            ),
        };
        rendered.push_str(&format!(
            "delivered {} by {}: {what}",
            entry.ticket, entry.deliverer
        ));
        if !entry.pruned.is_empty() {
            let pruned = entry
                .pruned
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            rendered.push_str(&format!("; pruned {pruned}"));
        }
        rendered.push('\n');
    }
    rendered
}

/// Where an entity is, as one cell: which kind of place, then the place itself.
///
/// The key is the kind, read off the type's own `Serialize` rather than written out again
/// in a `match`, for the reason [`wire`] gives: `Location` is externally tagged with
/// exactly two variants, so what a consumer branches on in JSON is what a reader sees here.
fn located(location: &Location) -> String {
    let rendered = serde_json::to_value(location).expect("a contract enum serialises");
    let (kind, place) = rendered
        .as_object()
        .and_then(|object| object.iter().next())
        .expect("an externally tagged enum is an object of exactly one member");
    format!("{kind} {}", place.as_str().unwrap_or_default())
}

/// One line per document: qualified id, where it is, title.
///
/// Where a task list prints the normalised status, this prints the location: a document
/// has no status, and where it is is what a reader does something with — open the link, or
/// read the file out. A document whose source did not say prints `-`, which is not the
/// same as saying it is nowhere.
pub fn documents(items: &[Qualified<Document>]) -> String {
    columns(
        &items
            .iter()
            .map(|document| {
                vec![
                    document.id.to_string(),
                    document
                        .item
                        .location
                        .as_ref()
                        .map_or_else(|| "-".to_owned(), located),
                    document.item.title.clone(),
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

/// One line per item a copy considered: where it came from, where it went, what happened —
/// and one final line for what it did to the references its documents hold.
///
/// A dry run that would create has no destination id to print, because nothing was
/// created and inventing one would be a claim about an id the destination never issued.
///
/// The reference line is **one** line and always printed, not a second report and not a
/// section a reader has to know to ask for: a silent bound is indistinguishable from a bug,
/// so a copy that recognised nothing says so with zeroes rather than by omission.
pub fn copied(report: &CopyReport) -> String {
    let mut rendered = columns(
        &report
            .items
            .iter()
            .map(|outcome| {
                let mut row = vec![
                    outcome.source.to_string(),
                    outcome
                        .destination()
                        .map_or_else(|| "-".to_owned(), ToString::to_string),
                    outcome.action.name(),
                ];
                // Only a routed copy places anything, so only its rows say where and why.
                if let Some(placed) = &outcome.placed {
                    row.push(match placed.route {
                        Some(index) => format!("in {} by route {index}", placed.destination),
                        None => format!("in {} by no route", placed.destination),
                    });
                }
                row
            })
            .collect::<Vec<_>>(),
    );
    if report.delivers_rewritten > 0 {
        rendered.push_str(&format!(
            "delivers: {} rewritten\n",
            report.delivers_rewritten
        ));
    }
    rendered.push_str(&references(report));
    rendered.push_str(&delivered(&report.delivered));
    rendered.push_str(&spent(report));
    rendered
}

/// The one line a copy says about what it spent, when a source in it metered its requests.
///
/// No line at all when none did, for the reason the machine output leaves the member out:
/// a source that does not count what it sends has not sent nothing.
fn spent(report: &CopyReport) -> String {
    let Some(spent) = &report.spent else {
        return String::new();
    };
    let budgets = spent
        .budgets
        .iter()
        .map(|budget| {
            format!(
                "{} {} {}{}",
                budget.budget,
                budget.amount,
                budget.unit,
                if budget.lower_bound { " at least" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("spent: {} requests; {budgets}\n", spent.requests)
}

/// The one line a copy says about the references its documents hold.
///
/// The ambiguous figure is spelled as what it is — part of the unresolved one — because
/// the two mean different things to a reader: an unresolved reference is ordinary under
/// the bound the copy works to, while an ambiguous one says the destination holds
/// duplicate records for one work item, or the source reports one location for two, and
/// re-running the copy will never clear it.
fn references(report: &CopyReport) -> String {
    format!(
        "references: {} rewritten, {} unresolved ({} ambiguous)\n",
        report.references_rewritten, report.references_unresolved, report.references_ambiguous
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
///
/// The `key` line sits directly under `id` and only when the source gives one: it is the
/// short handle that backend shows people — `ENG-123`, `1043` — and a reader looking for
/// the thing they say out loud looks next to the id they were given. A source with no
/// separate handle prints no line at all, rather than printing its id twice.
pub fn task_detail(task: &Qualified<Task>) -> String {
    let item = &task.item;
    let mut fields = vec![("id", task.id.to_string())];
    if let Some(key) = &item.key {
        fields.push(("key", key.clone()));
    }
    fields.extend([
        ("title", item.title.clone()),
        (
            "status",
            format!("{} ({})", wire(&item.status.category), item.status.name),
        ),
    ]);
    // Only when one is set: `none` is what every task of a source without priorities reads
    // as, and a line saying so on each of them would say nothing.
    if item.priority != Priority::None {
        fields.push(("priority", wire(&item.priority)));
    }
    fields.push((
        "project",
        match &item.project {
            Some(project) => format!("{}:{project}", task.id.source),
            None => "none".to_owned(),
        },
    ));
    if !item.delivers.is_empty() {
        fields.push(("delivers", listed(&item.delivers)));
    }
    if !item.delivered_by.is_empty() {
        fields.push(("delivered by", listed(&item.delivered_by)));
    }
    detail(
        &mut fields,
        &item.labels,
        item.url.as_deref(),
        item.location.as_ref(),
    );
    body(&fields, item.content.as_deref())
}

/// One task in full, body last, and then its comments when its source has them.
///
/// `None` is a source whose tasks have no comments, and says nothing about them: a line
/// reading "no comments" there would claim the source holds comments and this task has none.
pub fn task_with_comments(
    task: &Qualified<Task>,
    comments: Option<&[Comment]>,
    assets: Option<&[Asset]>,
) -> String {
    let mut rendered = task_detail(task);
    rendered.push_str(&assets_held(assets));
    let Some(comments) = comments else {
        return rendered;
    };
    rendered.push('\n');
    if comments.is_empty() {
        rendered.push_str("comments: none\n");
        return rendered;
    }
    rendered.push_str(&format!("comments: {}\n", comments.len()));
    for held in comments {
        rendered.push('\n');
        rendered.push_str(&comment(held));
    }
    rendered
}

/// Several tasks as `task show-many` reports them: each in full, as `task show` renders it,
/// in the order asked for, or the id and why it could not be shown.
pub fn task_details(ids: &[GlobalId], details: &TaskDetails) -> String {
    let mut rendered = Vec::new();
    for (id, detail) in ids.iter().zip(&details.details) {
        let mut shown = match detail.response.items.first() {
            Some(task) => {
                task_with_comments(task, detail.comments.as_deref(), detail.assets.as_deref())
            }
            None => format!("{id}\n"),
        };
        for failure in &detail.response.errors {
            shown.push_str(&format!(
                "error: source {} could not answer: {}\n",
                failure.source, failure.error
            ));
        }
        rendered.push(shown);
    }
    rendered.join("\n---\n\n")
}

/// One comment in full: the fields a person reads it by, then what it says, unaltered.
///
/// A field the source did not give is left out rather than printed empty, as a task's are.
pub fn comment(comment: &Comment) -> String {
    let mut fields = vec![("comment", comment.id.to_string())];
    if let Some(author) = &comment.author {
        fields.push(("author", author.clone()));
    }
    if let Some(created) = &comment.created_at {
        fields.push(("created", wire(created)));
    }
    if let Some(updated) = &comment.updated_at {
        fields.push(("updated", wire(updated)));
    }
    if let Some(url) = &comment.url {
        fields.push(("url", url.clone()));
    }
    let mut rendered = columns(
        &fields
            .iter()
            .map(|(name, value)| vec![format!("{name}:"), value.clone()])
            .collect::<Vec<_>>(),
    );
    rendered.push('\n');
    rendered.push_str(&comment.body);
    if !comment.body.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

/// Every comment on a task, oldest first, each in full with a blank line between.
pub fn comments(list: &CommentList) -> String {
    if list.comments.is_empty() {
        return "no comments\n".to_owned();
    }
    list.comments
        .iter()
        .map(comment)
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a delete removed.
pub fn deleted(deleted: &DeletedComment) -> String {
    format!("deleted comment {}\n", deleted.deleted)
}

/// One document in full, body last.
///
/// No status line, because a document has none — and no dependency line, because it is in
/// no graph. What it has that a task does not print above its body is the same `location`
/// every entity now carries.
pub fn document_detail(document: &Qualified<Document>) -> String {
    let item = &document.item;
    let mut fields = vec![
        ("id", document.id.to_string()),
        ("title", item.title.clone()),
        (
            "project",
            match &item.project {
                Some(project) => format!("{}:{project}", document.id.source),
                None => "none".to_owned(),
            },
        ),
    ];
    detail(
        &mut fields,
        &item.labels,
        item.url.as_deref(),
        item.location.as_ref(),
    );
    body(&fields, item.content.as_deref())
}

/// One document in full, body last, and then the image assets it holds.
pub fn document_with_assets(document: &Qualified<Document>, assets: Option<&[Asset]>) -> String {
    let mut rendered = document_detail(document);
    rendered.push_str(&assets_held(assets));
    rendered
}

/// The image assets a task or a document holds, each on a line of its own — name, content
/// type, SHA-256 and where its bytes are — after a blank line; nothing at all for an item
/// holding none, whose rendering is then exactly what it was before there were assets.
fn assets_held(assets: Option<&[Asset]>) -> String {
    let Some(assets) = assets.filter(|assets| !assets.is_empty()) else {
        return String::new();
    };
    let mut rendered = format!("\nassets: {}\n", assets.len());
    rendered.push_str(&columns(
        &assets
            .iter()
            .map(|asset| {
                vec![
                    format!("  {}", asset.name),
                    asset.content_type.as_str().to_owned(),
                    asset.sha256.clone(),
                    asset.path.clone().unwrap_or_else(|| "(hosted)".to_owned()),
                ]
            })
            .collect::<Vec<_>>(),
    ));
    rendered
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
    // A home names its member projects, and a member its home: the plan they make together
    // is read with `task list --project <home> --members`.
    let members: Vec<&str> = item
        .metadata
        .get(MetadataKey::MEMBERS_KEY)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect();
    if !members.is_empty() {
        fields.push(("members", members.join(", ")));
    }
    if let Some(home) = item
        .metadata
        .get(MetadataKey::MEMBER_OF_KEY)
        .and_then(serde_json::Value::as_str)
    {
        fields.push(("member of", home.to_owned()));
    }
    detail(
        &mut fields,
        &item.labels,
        item.url.as_deref(),
        item.location.as_ref(),
    );
    body(&fields, item.content.as_deref())
}

/// Where `sources route` says an item would land, and which route sent it there.
pub fn source_route(route: &SourceRoute) -> String {
    columns(&[
        vec!["source:".to_owned(), route.source.to_string()],
        vec![
            "destination:".to_owned(),
            route.placement.destination.to_string(),
        ],
        vec![
            "route:".to_owned(),
            route
                .placement
                .route
                .map_or_else(|| "none".to_owned(), |index| index.to_string()),
        ],
    ])
}

/// The fields a task, a project and a document share below their own.
///
/// `location` is a line of its own rather than folded into `url`, and it says which kind
/// of place it names: a reader handed one has to know whether to open a link or read a
/// file out, and the two are different actions. It does not replace `url` — a source that
/// reports one goes on reporting it, and both lines appear when a source gives both.
fn detail(
    fields: &mut Vec<(&'static str, String)>,
    item_labels: &[Label],
    url: Option<&str>,
    location: Option<&Location>,
) {
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
    if let Some(location) = location {
        fields.push(("location", located(location)));
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
