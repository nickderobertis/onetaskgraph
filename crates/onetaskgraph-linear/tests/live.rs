//! Every capability this source declares, driven against Linear's real API.
//!
//! The lane builds its own fixture on the scratch team `LINEAR_WRITE_TEAM` names — two
//! projects, one issue filed under each, one issue filed under neither, two documents
//! filed the same way, two labels and two workflow states — because that shape is what
//! makes an honoured predicate and an ignored one *different answers*. A workspace holding
//! one project, or one where every issue carries the label, answers a filter the same way
//! whether or not the source applies it.
//!
//! It is an ordinary test in this crate's ordinary `test` target: change this plugin and
//! it runs in the required check, change anything else and affected selection does not
//! select it. Without `LINEAR_API_KEY` or without the scratch team `LINEAR_WRITE_TEAM`
//! names it skips, printing why — a contributor with no keys, and a pull request from a
//! fork, which the host gives no secrets. Where one *is* expected the run sets
//! `ONETASKGRAPH_LIVE_REQUIRED=1` and each of those skips becomes a failure naming the
//! variable, so the required lane cannot pass green for want of a credential.
//!
//! Everything it creates it deletes, whether its assertions passed or failed. It also
//! recovers what a run killed between its writes and its cleanup left behind — but only
//! artifacts the kernel says no live run owns, and only once its own journey is over. See
//! [`sweep_orphans`].

use std::{collections::BTreeMap, env};

use onetaskgraph_live::artifact::{Run, Sweep, now_micros};
use onetaskgraph_live::{Credential, Exclusivity, Session, missing, required};
use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport, Direction,
    Document, DocumentQuery, ItemKind, ItemWrite, Label, LabelFilter, Location, MetadataMatch,
    NativeId, PageRequest, Priority, Project, ProjectFilter, ProjectQuery, SecretResolver,
    SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery, TaskSource,
    TextFields, TextQuery,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct Environment;
impl SecretResolver for Environment {
    fn get(&self, var: &str) -> Option<SecretString> {
        env::var(var).ok().map(SecretString::from)
    }
}

/// A live assertion that returns rather than panics.
///
/// Every check inside the journey has to reach [`run_then_cleanup`] as an `Err`: a panic
/// would unwind past the cleanup and leave this run's issues, projects and labels in the
/// scratch team for the next run to find.
macro_rules! ensure {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            return Err(format!($($message)+));
        }
    };
}

// `cleanup` is shared with `tests/sweep_gate.rs`, which drives the same code against a
// loopback stand-in. What this target does not reach is that drive's — the endpoint seam it
// uses to be pointed somewhere other than Linear — rather than dead code.
#[allow(dead_code)]
mod cleanup;

use cleanup::{
    ISSUE_PAGE_PROBE, LABEL_CREATE, PROJECT_STATUSES, SESSION_NAME, TEAM_STATES, artifact_label,
    artifact_title, is_this_runs, linear, remove_artifacts, run_then_cleanup, sweep_orphans,
};

// `settle` is shared with `tests/settle_gate.rs`, which drives these reads against a loopback
// workspace whose index lags.
mod settle;

use settle::{
    LINEAR_DOCUMENT_LISTING, LINEAR_INDEX, settled, settled_document_absent, settled_documents,
    settled_label, settled_tasks, settled_walk, task_titles, walked_task_titles,
};

/// The two workflow states this fixture files its issues under.
///
/// Chosen by Linear's own `WorkflowState.type` rather than by name, because the display
/// names of a team's states are the team's business: `unstarted` is what this product
/// reads as `todo` and `completed` is what it reads as `done`, so a team that has both
/// gives the fixture two categories a status filter can separate however it spells them.
async fn fixture_states(key: &str, team_key: &str) -> Result<(String, String), String> {
    let data = linear(
        key,
        TEAM_STATES,
        json!({"key":team_key}),
        "live workflow state discovery",
    )
    .await?;
    let states = data
        .pointer("/teams/nodes/0/states/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!("LINEAR_WRITE_TEAM={team_key} names no team this credential can see")
        })?;
    let named = |wanted: &str| {
        states
            .iter()
            .find(|state| state.get("type").and_then(Value::as_str) == Some(wanted))
            .and_then(|state| state.get("name").and_then(Value::as_str))
            .map(str::to_owned)
    };
    match (named("unstarted"), named("completed")) {
        (Some(open), Some(done)) => Ok((open, done)),
        _ => Err(format!(
            "team {team_key} has no workflow state of type unstarted and one of type \
             completed, which this lane needs to file two issues a status filter can \
             separate; add them to that team or point LINEAR_WRITE_TEAM at a scratch team \
             that has them"
        )),
    }
}

/// A project status name this workspace resolves uniquely.
///
/// The source resolves a project status by name across the whole workspace and refuses a
/// name two statuses answer to, so the fixture picks one nothing else is spelled like
/// rather than assuming a default.
async fn fixture_project_status(key: &str) -> Result<String, String> {
    let data = linear(
        key,
        PROJECT_STATUSES,
        json!({}),
        "live project status discovery",
    )
    .await?;
    let statuses = data
        .pointer("/projectStatuses/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "live project status discovery returned no statuses".to_owned())?;
    let names = statuses
        .iter()
        .filter_map(|status| status.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    // Case-insensitively, because that is how the source's own lookup compares: a name
    // two statuses answer to under `eqIgnoreCase` is one it refuses.
    names
        .iter()
        .find(|name| {
            names
                .iter()
                .filter(|other| other.eq_ignore_ascii_case(name))
                .count()
                == 1
        })
        .map(|name| (*name).to_owned())
        .ok_or_else(|| {
            "this workspace has no project status name that resolves uniquely, and the \
             source refuses one two statuses answer to"
                .to_owned()
        })
}

async fn create_label(key: &str, team: &str, name: &str) -> Result<(), String> {
    let data = linear(
        key,
        LABEL_CREATE,
        json!({"input":{"name":name,"teamId":team,"color":"#bec2c8"}}),
        "live label creation",
    )
    .await?;
    if data
        .pointer("/issueLabelCreate/issueLabel/name")
        .and_then(Value::as_str)
        != Some(name)
    {
        return Err(format!(
            "Linear did not confirm creating the label {name:?}"
        ));
    }
    Ok(())
}

fn label(name: &str) -> Label {
    // Only the name reaches Linear: the source resolves a label by name and never sends
    // an id or a colour it was handed.
    Label {
        id: NativeId("live-source-label".into()),
        name: name.to_owned(),
        color: None,
    }
}

fn blocks(far: &NativeId, kind: ItemKind) -> DependencyEdge {
    DependencyEdge {
        // Only `to` decides where the relation goes: the source names the near end from
        // the item it is writing, which has no id until Linear creates it.
        from: DependencyEndpoint::from_native(NativeId("live-source-item".into()), kind),
        to: DependencyEndpoint::from_native(far.clone(), kind),
        kind: DependencyKind::Blocks,
    }
}

fn page(limit: u32) -> PageRequest {
    PageRequest {
        cursor: None,
        limit,
    }
}

fn sorted(mut titles: Vec<String>) -> Vec<String> {
    titles.sort();
    titles
}

/// Everything this lane needs to reach the one team it may write to.
struct LiveRun {
    key: String,
    /// Which run this is: the machine that can vouch for it and the process on it. Every
    /// artifact below carries it, which is what its own cleanup finds them by and what a
    /// later run's sweep looks this run up by before deciding anything about them.
    id: Run,
    stamp_micros: u64,
    open_state: String,
    done_state: String,
    project_status: String,
}

/// Drives every field of the source's declared `Capabilities` against Linear's real API.
///
/// Nothing here panics: every failure returns, so the caller's cleanup runs over a
/// workspace this run is still holding artifacts in.
async fn drive_every_declared_capability(
    run: &LiveRun,
    source: &dyn TaskSource,
    team_id: &str,
) -> Result<(), String> {
    let title = |offset: u64| artifact_title(run.id, run.stamp_micros + offset);
    let (alpha, beta) = (title(0), title(1));
    let (first, second, orphan) = (title(2), title(3), title(4));
    let run_label = artifact_label(run.id, run.stamp_micros);
    let only_label = artifact_label(run.id, run.stamp_micros + 1);
    create_label(&run.key, team_id, &run_label).await?;
    create_label(&run.key, team_id, &only_label).await?;
    // The first task write names both, and the source resolves a label by name through
    // Linear's label filter, which holds a label only some while after it was created — and
    // for a while after that may answer it on one read and not the next.
    for name in [&run_label, &only_label] {
        settled_label(LINEAR_INDEX, name, |query, variables| {
            linear(&run.key, query, variables, "live label lookup")
        })
        .await?;
    }
    let open = Status {
        category: StatusCategory::Todo,
        name: run.open_state.clone(),
    };
    let done = Status {
        category: StatusCategory::Done,
        name: run.done_state.clone(),
    };
    let project_status = Status {
        category: StatusCategory::Todo,
        name: run.project_status.clone(),
    };
    // A syntactically valid nil UUID names no issue in the nominated test workspace.
    // Exercise the actual mutation error through the public source boundary: if Linear
    // rewords both recognised messages, this lane fails rather than trusting the fixture.
    let missing = source
        .set_task_status(
            &NativeId("00000000-0000-0000-0000-000000000000".into()),
            StatusCategory::Todo,
        )
        .await
        .map_err(|error| format!("the live missing-issue status contract drifted: {error}"))?;
    ensure!(
        missing.is_none(),
        "a status mutation to a nonexistent issue was not not-found"
    );
    // Every listing below is scoped by the label all three issues carry, because Linear's
    // `issues` connection is the whole workspace: without it these would be containments
    // rather than the exact sets that tell an honoured predicate from an ignored one.
    let scoped = || TaskQuery {
        labels: LabelFilter {
            any_of: vec![run_label.clone()],
            ..LabelFilter::default()
        },
        ..TaskQuery::default()
    };
    let project = |name: &str| Project {
        id: NativeId("live-source-item".into()),
        title: name.to_owned(),
        content: Some("temporary credentialed write; the live lane removes this".into()),
        status: project_status.clone(),
        labels: vec![],
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    };
    let task = |name: &str, status: &Status, under: Option<&NativeId>, labels: Vec<Label>| Task {
        id: NativeId("live-source-item".into()),
        key: None,
        title: name.to_owned(),
        content: Some("temporary credentialed write; the live lane removes this".into()),
        status: status.clone(),
        labels,
        priority: Priority::None,
        project: under.cloned(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
        delivers: Vec::new(),
        delivered_by: Vec::new(),
    };

    let alpha_id = source
        .write_project(&ItemWrite {
            target: None,
            item: project(&alpha),
            depends_on: vec![],
        })
        .await
        .map_err(|error| format!("live project write of {alpha:?} failed: {error}"))?;
    let beta_id = source
        .write_project(&ItemWrite {
            target: None,
            item: project(&beta),
            depends_on: vec![blocks(&alpha_id, ItemKind::Project)],
        })
        .await
        .map_err(|error| format!("live project write of {beta:?} failed: {error}"))?;
    let first_id = source
        .write_task(&ItemWrite {
            target: None,
            item: task(
                &first,
                &open,
                Some(&alpha_id),
                vec![label(&run_label), label(&only_label)],
            ),
            depends_on: vec![],
        })
        .await
        .map_err(|error| format!("live task write of {first:?} failed: {error}"))?;
    let second_id = source
        .write_task(&ItemWrite {
            target: None,
            item: task(&second, &open, Some(&beta_id), vec![label(&run_label)]),
            depends_on: vec![blocks(&first_id, ItemKind::Task)],
        })
        .await
        .map_err(|error| format!("live task write of {second:?} failed: {error}"))?;
    let orphan_id = source
        .write_task(&ItemWrite {
            target: None,
            item: task(&orphan, &done, None, vec![label(&run_label)]),
            depends_on: vec![],
        })
        .await
        .map_err(|error| format!("live task write of {orphan:?} failed: {error}"))?;

    // Linear indexes a created issue before it answers a filtered query over it, so the
    // reads below wait for the fixture rather than racing it — each one, through `settle`,
    // because the index does not catch up on every filter at once.
    let all_three = sorted(vec![first.clone(), second.clone(), orphan.clone()]);
    settled_tasks(
        LINEAR_INDEX,
        source,
        &scoped(),
        &format!("the three issues this run created, labelled {run_label},"),
        &all_three,
    )
    .await?;

    // `projects`: two are held, and a listing scoped to one keeps the issue filed under
    // it and no other. Unscoped on purpose — the workspace holds issues of its own, so a
    // filter declared and then ignored returns them too.
    let mut listed = Vec::new();
    let mut cursor = None;
    loop {
        let step = source
            .query_projects(
                &ProjectQuery::default(),
                &PageRequest {
                    cursor,
                    limit: onetaskgraph_linear::MAX_PAGE_SIZE,
                },
            )
            .await
            .map_err(|error| format!("live project listing failed: {error}"))?;
        listed.extend(step.items.into_iter().map(|project| project.title));
        cursor = step.next;
        if cursor.is_none() {
            break;
        }
        ensure!(listed.len() < 100_000, "the project walk must terminate");
    }
    ensure!(
        listed.contains(&alpha) && listed.contains(&beta),
        "the two projects this run created are not in Linear's own project listing"
    );
    let under = |id: &NativeId| TaskQuery {
        project: ProjectFilter::Is(id.clone()),
        ..TaskQuery::default()
    };
    settled_tasks(
        LINEAR_INDEX,
        source,
        &under(&alpha_id),
        "the issues of one of this run's two projects",
        std::slice::from_ref(&first),
    )
    .await?;
    settled_tasks(
        LINEAR_INDEX,
        source,
        &under(&beta_id),
        "the issues of the other of this run's two projects",
        std::slice::from_ref(&second),
    )
    .await?;

    // `orphan_tasks`: the one issue filed under neither project.
    settled_tasks(
        LINEAR_INDEX,
        source,
        &TaskQuery {
            project: ProjectFilter::Orphans,
            ..scoped()
        },
        "this run's issues belonging to no project",
        std::slice::from_ref(&orphan),
    )
    .await?;

    // `filter_by_label`: one of the three carries the second label, and the exclusion
    // keeps exactly the other two.
    settled_tasks(
        LINEAR_INDEX,
        source,
        &TaskQuery {
            labels: LabelFilter {
                any_of: vec![only_label.clone()],
                ..LabelFilter::default()
            },
            ..TaskQuery::default()
        },
        "this run's issues carrying its second label",
        std::slice::from_ref(&first),
    )
    .await?;
    settled_tasks(
        LINEAR_INDEX,
        source,
        &TaskQuery {
            labels: LabelFilter {
                any_of: vec![run_label.clone()],
                none_of: vec![only_label.clone()],
                ..LabelFilter::default()
            },
            ..TaskQuery::default()
        },
        "this run's issues not carrying its second label",
        &[second.clone(), orphan.clone()],
    )
    .await?;

    // `filter_by_status`: two issues sit in an `unstarted` state and one in a `completed`
    // one, so the normalised categories separate them.
    settled_tasks(
        LINEAR_INDEX,
        source,
        &TaskQuery {
            statuses: vec![StatusCategory::Todo],
            ..scoped()
        },
        "this run's unstarted issues",
        &[first.clone(), second.clone()],
    )
    .await?;
    settled_tasks(
        LINEAR_INDEX,
        source,
        &TaskQuery {
            statuses: vec![StatusCategory::Done],
            ..scoped()
        },
        "this run's completed issue",
        std::slice::from_ref(&orphan),
    )
    .await?;

    // `search_title` and `search_content` are declared `Native`: the source narrows by each
    // and returns exactly what the case-insensitive substring rule keeps. The first issue's
    // title is in no issue's content, and every issue's content holds the lane's own sentence.
    for (terms, fields, expected) in [
        (first.clone(), TextFields::Title, vec![first.clone()]),
        (first.clone(), TextFields::Content, Vec::new()),
        (
            first.clone(),
            TextFields::TitleOrContent,
            vec![first.clone()],
        ),
        (
            "TEMPORARY CREDENTIALED WRITE".to_owned(),
            TextFields::Content,
            all_three.clone(),
        ),
    ] {
        settled_tasks(
            LINEAR_INDEX,
            source,
            &TaskQuery {
                text: Some(TextQuery { terms, fields }),
                ..scoped()
            },
            &format!("a {fields:?} search this source narrows itself"),
            &expected,
        )
        .await?;
    }

    // `task_dependencies` and `project_dependencies`, both directions each. One relation
    // reads the same from either end: the waiting item is `from` whichever of Linear's
    // `relations` and `inverseRelations` answered it.
    let task_edge = DependencyEdge {
        from: DependencyEndpoint::from_native(second_id.clone(), ItemKind::Task),
        to: DependencyEndpoint::from_native(first_id.clone(), ItemKind::Task),
        kind: DependencyKind::Blocks,
    };
    let project_edge = DependencyEdge {
        from: DependencyEndpoint::from_native(beta_id.clone(), ItemKind::Project),
        to: DependencyEndpoint::from_native(alpha_id.clone(), ItemKind::Project),
        kind: DependencyKind::Blocks,
    };
    for (near, direction, expected, level) in [
        (&second_id, Direction::DependsOn, &task_edge, "task"),
        (&first_id, Direction::DependedOnBy, &task_edge, "task"),
        (&beta_id, Direction::DependsOn, &project_edge, "project"),
        (&alpha_id, Direction::DependedOnBy, &project_edge, "project"),
    ] {
        let read = if level == "task" {
            source.task_dependencies(near, direction, &page(50)).await
        } else {
            source
                .project_dependencies(near, direction, &page(50))
                .await
        }
        .map_err(|error| format!("live {level} {direction:?} dependency read failed: {error}"))?;
        ensure!(
            read.items == vec![expected.clone()],
            "the {level} {direction:?} read of {} returned {:?}",
            near.0,
            read.items
        );
    }

    // Paging: a limit smaller than the result set walks to exhaustion, reaching every row
    // exactly once and in the order one whole page reports them. The walk is a listing like
    // the others and waits out the index the same way, so the order is compared over a set
    // the index has already caught up with rather than over a walk that raced it.
    let walk = "a walk in pages of one over this run's own three issues";
    settled_walk(LINEAR_INDEX, source, &scoped(), 10, walk, &all_three).await?;
    let whole = source
        .query_tasks(&scoped(), &page(50))
        .await
        .map_err(|error| format!("live whole-page read failed: {error}"))?
        .items
        .into_iter()
        .map(|task| task.title)
        .collect::<Vec<_>>();
    let walked = walked_task_titles(source, &scoped(), 10, walk).await?;
    ensure!(
        walked == whole,
        "a walk in pages of one reached {walked:?} where one whole page reports {whole:?}"
    );

    // `max_page_size`: the source clamps a limit one above its declared ceiling to that
    // ceiling instead of passing it on, so the read succeeds rather than being refused
    // for a page size Linear's connection cannot serve.
    let ceiling = source.capabilities().max_page_size;
    let over_the_ceiling = scoped();
    let what = "a read at a limit one above the declared ceiling, which this source clamps \
                rather than refuses,";
    settled(LINEAR_INDEX, what, &all_three, || {
        task_titles(source, &over_the_ceiling, ceiling + 1, what)
    })
    .await?;
    // And that the ceiling itself is a page Linear really serves, rather than a number
    // this source guessed at. What Linear does with one row *more* is its own business and
    // is documented nowhere, so nothing here asserts on it.
    linear(
        &run.key,
        ISSUE_PAGE_PROBE,
        json!({"first":ceiling}),
        "page size probe at the declared maximum",
    )
    .await?;

    // The values a copy carries: what was written reads back by its own id.
    let written = source
        .get_task(&first_id)
        .await
        .map_err(|error| format!("live task read-back failed: {error}"))?
        .ok_or_else(|| "the written issue was not readable by its own id".to_owned())?;
    ensure!(
        written.title == first && written.status.category == StatusCategory::Todo,
        "the live write did not round-trip its title and status: {written:?}"
    );
    let closed_back = source
        .get_task(&orphan_id)
        .await
        .map_err(|error| format!("live completed-issue read-back failed: {error}"))?
        .ok_or_else(|| "the completed issue was not readable by its own id".to_owned())?;
    ensure!(
        closed_back.status.category == StatusCategory::Done,
        "the live write filed under done read back as {:?}",
        closed_back.status.category
    );

    drive_documents(run, source, &alpha_id, &title(5), &title(6)).await?;
    drive_follow_ups(run, source, &title(7), &title(8), &title(9), &title(10)).await
}

/// One issue's raw `description`, as Linear hands it back.
async fn raw_description(run: &LiveRun, id: &NativeId) -> Result<String, String> {
    linear(
        &run.key,
        "query($id:String!){ issue(id:$id){ description } }",
        json!({"id":id.0}),
        "live raw issue read",
    )
    .await?
    .pointer("/issue/description")
    .and_then(Value::as_str)
    .map(str::to_owned)
    .ok_or_else(|| format!("issue {} carried no description", id.0))
}

/// One document's raw `content`, as Linear hands it back.
async fn raw_content(run: &LiveRun, id: &NativeId) -> Result<String, String> {
    linear(
        &run.key,
        "query($id:String!){ document(id:$id){ content } }",
        json!({"id":id.0}),
        "live raw document read",
    )
    .await?
    .pointer("/document/content")
    .and_then(Value::as_str)
    .map(str::to_owned)
    .ok_or_else(|| format!("document {} carried no content", id.0))
}

/// The text the lane writes into an issue's description, a comment's body and a document's
/// content, to see what Linear does to an HTML comment in each: one on one line holding a
/// domain-like value and an array, one closing on a line of its own, and one on one line with
/// its JSON inside a code span.
const PROBE: &str = "P.\n\n<!-- probe {\"site\":\"example.com\",\"list\":[1]} -->\n\n<!-- probe\n{\"k\":\"v\"}\n-->\n\n<!-- probe `{\"site\":\"example.com\",\"list\":[1]}` -->";

/// What Linear hands [`PROBE`] back as from an issue's description and a document's content,
/// observed 2026-10-02 and recorded in the plugin's module documentation: inside the first
/// comment the domain-like value is autolinked and the array's brackets escaped, the second
/// comment's closing line is escaped, and the code span comes back as it was written.
const PROBE_STORED: &str = "P.\n\n<!-- probe {\"site\":\"[example.com](<http://example.com>)\",\"list\":\\[1\\]} -->\n\n<!-- probe\n{\"k\":\"v\"}\n\\-->\n\n<!-- probe `{\"site\":\"example.com\",\"list\":[1]}` -->";

/// What the follow-up flow rests on, against Linear itself: what Linear does to an HTML comment
/// in an issue's description, a comment's body and a document's content, recorded in the
/// plugin's module documentation; the metadata slot, written as a code span, keeping values
/// Linear rewrites everywhere else; each of the follow-up searches narrowed by Linear and
/// confirmed in process — a decoy whose prose carries every phrase is kept out — and a narrow
/// metadata write that moves only the slot.
async fn drive_follow_ups(
    run: &LiveRun,
    source: &dyn TaskSource,
    matching: &str,
    decoy: &str,
    document_title: &str,
    probe_title: &str,
) -> Result<(), String> {
    let tag = format!("otg-live-{}", run.stamp_micros);
    let origin = format!("elsewhere:{tag}");
    let issue = |title: &str, content: String, metadata: Value, priority: Priority| Task {
        id: NativeId("live-source-item".into()),
        key: None,
        title: title.to_owned(),
        content: Some(content),
        status: Status {
            category: StatusCategory::Todo,
            name: run.open_state.clone(),
        },
        labels: Vec::new(),
        priority,
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: serde_json::from_value(metadata).unwrap_or_default(),
        repositories: Vec::new(),
        delivers: Vec::new(),
        delivered_by: Vec::new(),
    };
    // `caller.live` is a key Linear autolinks anywhere but a code span, and `caller.site`
    // holds a URL-like value, an emphasis-shaped one and an array: each is something Linear
    // rewrites in an HTML comment, and the slot has to keep every one.
    let hostile = json!(["github.com/a/b", "_y_", "a --> b"]);
    let matching_metadata = json!({"caller.tag": tag, "caller.key": tag, "caller.live": tag,
        "caller.site": hostile, "onetaskgraph.origin": origin});
    let matching_id = source
        .write_task(&ItemWrite {
            target: None,
            item: issue(
                matching,
                "A follow-up the lane searches for.".to_owned(),
                matching_metadata.clone(),
                Priority::High,
            ),
            depends_on: Vec::new(),
        })
        .await
        .map_err(|error| format!("live write of {matching:?} failed: {error}"))?;
    let decoy_id = source
        .write_task(&ItemWrite {
            target: None,
            item: issue(
                decoy,
                format!(
                    "Quotes \"caller.key\":\"{tag}\" and \"onetaskgraph.origin\":\"{origin}\" \
                     in its prose, and {tag}-prose."
                ),
                // `caller.key` and `onetaskgraph.origin` are keys Linear leaves alone in
                // prose, so the phrases above make this a candidate Linear returns for both
                // narrowings, and only the confirmation over its slot keeps it out.
                json!({"caller.tag": tag, "caller.key": format!("{tag}-other")}),
                Priority::Low,
            ),
            depends_on: Vec::new(),
        })
        .await
        .map_err(|error| format!("live write of {decoy:?} failed: {error}"))?;
    let read_back = source
        .get_task(&matching_id)
        .await
        .map_err(|error| format!("live read of {matching:?} failed: {error}"))?
        .ok_or_else(|| format!("{matching:?} was not readable by its own id"))?;
    for key in ["caller.tag", "caller.key", "caller.live", "caller.site"] {
        ensure!(
            read_back.metadata.get(key) == matching_metadata.get(key),
            "the metadata slot did not keep {key}: wrote {}, read {:?}",
            matching_metadata[key],
            read_back.metadata.get(key)
        );
    }
    let held = raw_description(run, &matching_id).await?;
    ensure!(
        held.starts_with("A follow-up the lane searches for.\n\n<!-- onetaskgraph.metadata `")
            && held.ends_with("` -->"),
        "the slot did not come back in the one-line code-span spelling: {held:?}"
    );

    // The comment the activity search below finds the matching issue by.
    source
        .add_comment(
            &matching_id,
            &onetaskgraph_plugin_api::NewComment {
                body: onetaskgraph_plugin_api::CommentBody::new(
                    "A comment the activity search finds.".to_owned(),
                )
                .map_err(|error| error.to_string())?,
                author: None,
            },
        )
        .await
        .map_err(|error| format!("live comment add failed: {error}"))?
        .ok_or_else(|| "the matching issue had no comments".to_owned())?;

    // An HTML comment in an issue's description, a comment's body and a document's content.
    let probe_id = source
        .write_task(&ItemWrite {
            target: None,
            item: issue(probe_title, "probe".to_owned(), json!({}), Priority::None),
            depends_on: Vec::new(),
        })
        .await
        .map_err(|error| format!("live write of {probe_title:?} failed: {error}"))?;
    linear(
        &run.key,
        "mutation($id:String!,$input:IssueUpdateInput!){ issueUpdate(id:$id,input:$input){success} }",
        json!({"id":probe_id.0,"input":{"description":PROBE}}),
        "live description probe",
    )
    .await?;
    let stored = raw_description(run, &probe_id).await?;
    ensure!(
        stored == PROBE_STORED,
        "an issue description's HTML comments came back as {stored:?}, not as recorded"
    );
    let added = source
        .add_comment(
            &probe_id,
            &onetaskgraph_plugin_api::NewComment {
                body: onetaskgraph_plugin_api::CommentBody::new(PROBE.to_owned())
                    .map_err(|error| error.to_string())?,
                author: None,
            },
        )
        .await
        .map_err(|error| format!("live comment add failed: {error}"))?
        .ok_or_else(|| "the probe issue had no comments".to_owned())?;
    ensure!(
        added.body.as_str() == PROBE,
        "a comment body's HTML comments did not survive byte for byte: {:?}",
        added.body.as_str()
    );
    let document_id = source
        .write_document(&ItemWrite {
            target: None,
            item: Document {
                id: NativeId("live-source-item".into()),
                title: document_title.to_owned(),
                content: Some("A probed document.".to_owned()),
                project: None,
                labels: Vec::new(),
                url: None,
                location: None,
                created_at: None,
                updated_at: None,
                metadata: serde_json::from_value(json!({"caller.live": tag})).unwrap_or_default(),
                repositories: Vec::new(),
            },
            depends_on: Vec::new(),
        })
        .await
        .map_err(|error| format!("live document write failed: {error}"))?;
    let document = source
        .get_document(&document_id)
        .await
        .map_err(|error| format!("live document read failed: {error}"))?
        .ok_or_else(|| "the probed document was not readable by its own id".to_owned())?;
    ensure!(
        document.metadata.get("caller.live") == Some(&json!(tag))
            && document.content.as_deref() == Some("A probed document."),
        "a document's slot did not keep its metadata: {document:?}"
    );
    linear(
        &run.key,
        "mutation($id:String!,$input:DocumentUpdateInput!){ documentUpdate(id:$id,input:$input){success} }",
        json!({"id":document_id.0,"input":{"content":PROBE}}),
        "live content probe",
    )
    .await?;
    let content = raw_content(run, &document_id).await?;
    ensure!(
        content == PROBE_STORED,
        "a document's HTML comments came back as {content:?}, not as recorded"
    );

    // Each follow-up search, narrowed by Linear and confirmed here: the decoy's prose carries
    // the metadata and origin phrases, and its slot holds another value.
    let both = vec![decoy.to_owned(), matching.to_owned()];
    let only = vec![matching.to_owned()];
    let tag_match = MetadataMatch::new("caller.tag", Vec::new(), tag.clone())?;
    let base = TaskQuery {
        metadata: vec![tag_match.clone()],
        ..TaskQuery::default()
    };
    settled_tasks(LINEAR_INDEX, source, &base, "the two tagged issues", &both).await?;
    for (query, what, expected) in [
        (
            TaskQuery {
                metadata: vec![
                    tag_match.clone(),
                    MetadataMatch::new("caller.key", Vec::new(), tag.clone())?,
                ],
                ..TaskQuery::default()
            },
            "a metadata match the decoy carries only in prose",
            only.clone(),
        ),
        (
            TaskQuery {
                origin: Some(origin.clone()),
                ..base.clone()
            },
            "an origin the decoy carries only in prose",
            only.clone(),
        ),
        (
            TaskQuery {
                priorities: vec![Priority::High],
                ..base.clone()
            },
            "a priority",
            only.clone(),
        ),
        (
            TaskQuery {
                commented_since: chrono::Utc::now()
                    .checked_sub_signed(chrono::Duration::minutes(10)),
                ..base.clone()
            },
            "comment activity in the last ten minutes",
            only.clone(),
        ),
        (
            TaskQuery {
                commented_since: chrono::Utc::now().checked_add_signed(chrono::Duration::days(1)),
                ..base.clone()
            },
            "comment activity tomorrow",
            Vec::new(),
        ),
        (
            TaskQuery {
                text: Some(TextQuery {
                    terms: format!("{tag}-PROSE"),
                    fields: TextFields::Content,
                }),
                ..base.clone()
            },
            "a content search, any case",
            vec![decoy.to_owned()],
        ),
    ] {
        settled_tasks(LINEAR_INDEX, source, &query, what, &expected).await?;
    }

    // Linear's own comparators, as the module documentation records them: `contains` reads
    // the slot and is case-sensitive, `containsIgnoreCase` is not.
    let narrowed = |filter: Value| async move {
        let data = linear(
            &run.key,
            "query($filter:IssueFilter){ issues(first:10,filter:$filter){ nodes{ title } } }",
            json!({ "filter": filter }),
            "live comparator probe",
        )
        .await?;
        let mut titles = data
            .pointer("/issues/nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| "the comparator probe answered no issues".to_owned())?
            .iter()
            .filter_map(|node| node["title"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        titles.sort();
        Ok::<_, String>(titles)
    };
    let phrase = format!("\"caller.tag\":\"{tag}\"");
    for (filter, what, expected) in [
        (
            json!({"description":{"contains":phrase}}),
            "`contains` of a phrase both slots hold",
            both.clone(),
        ),
        (
            json!({"description":{"contains":phrase.to_uppercase()}}),
            "`contains` of the same phrase upper-cased",
            Vec::new(),
        ),
        (
            json!({"description":{"containsIgnoreCase":phrase.to_uppercase()}}),
            "`containsIgnoreCase` of it upper-cased",
            both.clone(),
        ),
    ] {
        settled(LINEAR_INDEX, what, &expected, || narrowed(filter.clone())).await?;
    }

    // A narrow metadata write moves only the slot: every byte above it is as it was.
    let before = raw_description(run, &decoy_id).await?;
    let key = onetaskgraph_plugin_api::MetadataKey::new("caller.review")?;
    source
        .set_task_metadata(&decoy_id, &key, &json!({"approved": true}))
        .await
        .map_err(|error| format!("live metadata write failed: {error}"))?
        .ok_or_else(|| "the decoy was not held".to_owned())?;
    let after = raw_description(run, &decoy_id).await?;
    let above = |field: &str| {
        field
            .rfind("<!-- onetaskgraph.metadata")
            .map(|start| field[..start].to_owned())
    };
    ensure!(
        above(&before).is_some() && above(&before) == above(&after),
        "a narrow metadata write moved bytes above the slot:\n{before:?}\n{after:?}"
    );
    ensure!(
        after.contains("\"caller.review\":{\"approved\":true}"),
        "the narrow metadata write did not land in the slot: {after:?}"
    );
    Ok(())
}

/// `documents`, against Linear's own first-class document type.
///
/// Two of them, filed the way the issues above are — one under a project, one under none —
/// because that difference is what tells a project predicate this source applied from one
/// it ignored, and because a document under no project is the case that needs the
/// configured team to give it a home.
async fn drive_documents(
    run: &LiveRun,
    source: &dyn TaskSource,
    under: &NativeId,
    filed: &str,
    loose: &str,
) -> Result<(), String> {
    let document = |title: &str, project: Option<&NativeId>| Document {
        id: NativeId("live-source-item".into()),
        title: title.to_owned(),
        content: Some("temporary credentialed write; the live lane removes this".into()),
        project: project.cloned(),
        // Linear's own document type has no labels; a write carrying one is refused by
        // name, which is asserted below rather than assumed.
        labels: vec![],
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: [("caller.count".to_owned(), json!(3))]
            .into_iter()
            .collect(),
        repositories: vec![],
    };
    let write = |item: Document| ItemWrite {
        target: None,
        item,
        depends_on: vec![],
    };

    let filed_id = source
        .write_document(&write(document(filed, Some(under))))
        .await
        .map_err(|error| format!("live document write of {filed:?} failed: {error}"))?;
    let loose_id = source
        .write_document(&write(document(loose, None)))
        .await
        .map_err(|error| format!("live document write of {loose:?} failed: {error}"))?;

    // Read back by its own id: the caller's key keeps its JSON type through the slot, the
    // visible body is the text without the slot, and where it is is a link.
    let read = source
        .get_document(&filed_id)
        .await
        .map_err(|error| format!("live document read-back failed: {error}"))?
        .ok_or_else(|| "the written document was not readable by its own id".to_owned())?;
    ensure!(
        read.title == filed,
        "the live document write did not round-trip its title: {read:?}"
    );
    ensure!(
        read.content.as_deref() == Some("temporary credentialed write; the live lane removes this"),
        "the visible body a read reports is the text a person wrote: {:?}",
        read.content
    );
    ensure!(
        read.metadata.get("caller.count") == Some(&json!(3)),
        "a caller's key did not round-trip with its JSON type: {:?}",
        read.metadata
    );
    ensure!(
        read.labels.is_empty(),
        "Linear's own document type has no labels, so a read reports none: {:?}",
        read.labels
    );
    ensure!(
        matches!(&read.location, Some(Location::Url(url)) if url.starts_with("https://")),
        "a document says where it is, as a link: {:?}",
        read.location
    );
    ensure!(
        read.project.as_ref() == Some(under),
        "the document filed under this run's project read back filed under {:?}",
        read.project
    );

    // A label is a field this source cannot carry, and it is refused by name rather than
    // dropped — the one answer a copy must never turn into a silent success.
    let refusal = source
        .write_document(&write(Document {
            labels: vec![label(&artifact_label(run.id, run.stamp_micros))],
            ..document(filed, Some(under))
        }))
        .await;
    ensure!(
        refusal
            .as_ref()
            .err()
            .is_some_and(|error| error.to_string().contains("labels")),
        "a document write carrying a label must be refused by name, and was {refusal:?}"
    );

    // Both this run's documents come back, the project predicate keeps one and the orphan
    // predicate the other, and a label demanded of a document keeps neither. Each listing
    // waits out the index as the issue listings do: a document's project filter has come back
    // without the document filed there while the unfiltered listing already held it.
    settled_documents(
        LINEAR_INDEX,
        LINEAR_DOCUMENT_LISTING,
        source,
        &DocumentQuery::default(),
        &[filed_id.clone(), loose_id.clone()],
        "the two documents this run created",
        &[
            (filed_id.clone(), filed.to_owned()),
            (loose_id.clone(), loose.to_owned()),
        ],
    )
    .await?;
    settled_documents(
        LINEAR_INDEX,
        LINEAR_DOCUMENT_LISTING,
        source,
        &DocumentQuery {
            project: ProjectFilter::Is(under.clone()),
            ..DocumentQuery::default()
        },
        &[filed_id.clone(), loose_id.clone()],
        "a document listing narrowed to this run's project",
        &[(filed_id.clone(), filed.to_owned())],
    )
    .await?;
    settled_documents(
        LINEAR_INDEX,
        LINEAR_DOCUMENT_LISTING,
        source,
        &DocumentQuery {
            project: ProjectFilter::Orphans,
            ..DocumentQuery::default()
        },
        &[filed_id.clone(), loose_id.clone()],
        "a document listing narrowed to the orphans",
        &[(loose_id.clone(), loose.to_owned())],
    )
    .await?;
    settled_documents(
        LINEAR_INDEX,
        LINEAR_DOCUMENT_LISTING,
        source,
        &DocumentQuery {
            labels: LabelFilter {
                any_of: vec![artifact_label(run.id, run.stamp_micros)],
                ..LabelFilter::default()
            },
            ..DocumentQuery::default()
        },
        &[filed_id.clone(), loose_id.clone()],
        "a document listing demanding a label, which no Linear document carries,",
        &[],
    )
    .await?;

    // And removed again, which is what lets a copy that could not finish take one back.
    // The sweep would clear them anyway; driving the verb is what proves it works.
    for (id, title) in [(&filed_id, filed), (&loose_id, loose)] {
        source
            .delete_document(id)
            .await
            .map_err(|error| format!("live document removal failed: {error}"))?;
        settled_document_absent(LINEAR_INDEX, source, id, title).await?;
    }
    Ok(())
}

#[tokio::test]
async fn real_linear_applies_every_declared_capability_and_leaves_no_residue() {
    // llmlint: ignore-block[live_tier_compiles_and_requires_credential,tests_assert_real_behavior] An absent credential or an unnamed scratch team skips only where none was expected — a contributor with no keys, and a fork pull request, which the host gives no secrets. `ONETASKGRAPH_LIVE_REQUIRED=1`, which .github/workflows/ci.yml sets on the one lane the credentials reach, turns each skip below into the failure this rule asks for.
    let live_required = required(
        env::var(onetaskgraph_live::REQUIRED_VARIABLE)
            .ok()
            .as_deref(),
    )
    .unwrap_or_else(|error| panic!("the Linear live lane cannot run: {error}"));
    let skip = |reason: &str| -> Option<String> {
        match missing(live_required, SESSION_NAME, reason) {
            Ok(reason) => {
                eprintln!("skipped live Linear journey: {reason}");
                None
            }
            Err(error) => panic!("the Linear live lane cannot run: {error}"),
        }
    };
    // `Credential::new` is what decides a key is usable, rather than a second reading of
    // "empty" here: the session this lane opens takes one of those and nothing else, and a
    // host expands a secret it does not have to the empty string rather than omitting it.
    let Some(key) = env::var("LINEAR_API_KEY").ok().and_then(Credential::new) else {
        skip("LINEAR_API_KEY is not set");
        return;
    };
    // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] .github/workflows/ci.yml spells this name too, and the drift gate is this lane's own refusal: that workflow sets ONETASKGRAPH_LIVE_REQUIRED=1 on the lane it hands the credential to, so a name spelled differently on either side fails the required check naming the variable rather than skipping green.
    let Some(team) = env::var("LINEAR_WRITE_TEAM")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        skip(
            "LINEAR_WRITE_TEAM is not set, and this lane writes only to the scratch team that \
             name gives rather than discovering one; no mutation was sent. Set it to that \
             team's key — in CI it is the LINEAR_WRITE_TEAM repository variable, and locally \
             it is an environment variable",
        );
        return;
    };
    // llmlint: ignore-end[live_tier_compiles_and_requires_credential,tests_assert_real_behavior]
    // The one gate: nothing below may reach Linear until the session is open, because the
    // key below is the one this returns rather than the one the environment held. A session
    // that is refused did not run and did not pass, and says so.
    //
    // `OneAtATime`: this lane still asks for a seat. Not because its cleanup needs one —
    // every artifact it writes carries this run's process id, its own cleanup removes only
    // those, and what it recovers of an interrupted run's is decided by that artifact's own
    // stamp — but because a seat is the one precondition that can decline this lane without
    // a credential and without reaching Linear, and `scripts/check-live-decline.sh` drives
    // that decline through to a red check. What a seat does not cover, and never did, is two
    // hosted runners: a file on one machine excludes nothing on another.
    let session = Session::open(SESSION_NAME, key, Exclusivity::OneAtATime)
        .unwrap_or_else(|declined| declined.refuse());
    let key = session.credential().expose().to_owned();
    // The names the fixture files its issues and projects under are what this source's
    // `status_mapping` gives `todo` and `done`: Linear has no built-in names, so a source
    // without a mapping refuses every status write and reads every item as `unknown`.
    let (open_state, done_state) = fixture_states(&key, &team)
        .await
        .unwrap_or_else(|error| panic!("the Linear live lane cannot file its fixture: {error}"));
    let project_status = fixture_project_status(&key)
        .await
        .unwrap_or_else(|error| panic!("the Linear live lane cannot file its projects: {error}"));
    let source = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("live").unwrap(),
            &json!({"team":team,"status_mapping":{
                "todo":{"task":open_state,"project":project_status},
                "done":{"task":done_state},
            }}),
            &Environment,
        )
        .unwrap_or_else(|error| panic!("the Linear live lane cannot use this team: {error}"));
    assert!(source.health().await.unwrap().reachable);
    // Every field of the contract's `Capabilities`, spelled out: the struct has no
    // `Default`, so a field added to the contract fails to compile here rather than going
    // unasserted, and the journey below drives each of these against Linear itself.
    assert_eq!(
        source.capabilities(),
        Capabilities {
            projects: Support::Native,
            documents: Support::Native,
            comments: Support::Native,
            assets: Support::Native,
            priority: Support::Native,
            filter_by_priority: Support::Native,
            filter_by_comment_activity: Support::Native,
            filter_by_metadata: Support::Native,
            filter_by_origin: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: onetaskgraph_linear::MAX_PAGE_SIZE,
        }
    );

    let team_id = linear(&key, TEAM_STATES, json!({"key":team}), "live team lookup")
        .await
        .and_then(|data| {
            data.pointer("/teams/nodes/0/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    format!("LINEAR_WRITE_TEAM={team} names no team this credential can see")
                })
        })
        .unwrap_or_else(|error| {
            panic!("the Linear live lane cannot reach its scratch team: {error}")
        });
    let run = LiveRun {
        key: key.clone(),
        id: Run::current(),
        stamp_micros: now_micros(),
        open_state,
        done_state,
        project_status,
    };
    let id = run.id;
    run_then_cleanup(
        || drive_every_declared_capability(&run, source.as_ref(), &team_id),
        || async {
            // This run's own, first: everything it wrote goes whether the journey passed or
            // failed. Then what an interrupted EARLIER run left, which is a different
            // decision on different evidence — and which is deliberately here, at the end,
            // rather than at the start where it used to be. A sweep before the journey
            // deleted the in-flight issues of any session running beside this one; a sweep
            // after it recovers exactly the same orphans and can reach nothing a live run
            // owns.
            let mine = remove_artifacts(&key, &|prefix, name| is_this_runs(id, prefix, name)).await;
            let orphans = sweep_orphans(&key, &Sweep::of(id, now_micros())).await;
            match (mine, orphans) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(mine), Ok(())) => Err(mine),
                (Ok(()), Err(orphans)) => Err(format!(
                    "residue left by an earlier interrupted run could not be cleared: {orphans}"
                )),
                (Err(mine), Err(orphans)) => Err(format!(
                    "{mine}; additionally, residue left by an earlier interrupted run could \
                     not be cleared: {orphans}"
                )),
            }
        },
    )
    .await
    .unwrap_or_else(|error| panic!("Linear live capability journey failed: {error}"));
}
