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
//! Ordinary CI reuses three explicitly initialized, team-owned documents and their parent.
//! Daily fresh mode removes its disposable documents after exercising creation and deletion.
//! Invocation-owned local sets retire on cleanup. Both modes also recover what a run killed
//! between its writes and its cleanup left behind — but only
//! artifacts the kernel says no live run owns, and only once its own journey is over. See
//! [`sweep_orphans`].

use std::{collections::BTreeMap, env};

use onetaskgraph_live::artifact::{Run, Sweep, now_micros};
use onetaskgraph_live::{Credential, Exclusivity, Session, missing, required};
use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport, Direction,
    Document, DocumentQuery, ItemKind, ItemWrite, Label, LabelFilter, Location, MetadataMatch,
    NativeId, PageRequest, Priority, Project, ProjectFilter, SecretResolver, SourceError,
    SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery, TaskSource,
    TextFields, TextQuery,
};
use secrecy::SecretString;
mod document_lifecycle;
use document_lifecycle::{Mode, Standing};
use onetaskgraph_linear::live_artifact::{LeaseLane, Scope, Slot};
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
// workspace whose index lags, and the read after a change against a replica that lags.
mod settle;

use settle::{
    Bound, LINEAR_DOCUMENT_LISTING, LINEAR_INDEX, LINEAR_REPLICA, resolving_labels, settled,
    settled_content, settled_document_absent, settled_documents, settled_label, settled_projects,
    settled_tasks, settled_walk, task_titles, token_moved,
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

/// `write`, a task naming labels this run created, through [`resolving_labels`]: Linear's
/// label filter has answered such a label with no match on the plugin's own lookup a moment
/// after it answered it three times in a row to [`settled_label`], so the write is sent again
/// within `bound` while that is its refusal, and ends with that same refusal once `bound` is
/// spent rather than creating the label or dropping it.
async fn write_labelled_task(
    bound: Bound,
    source: &dyn TaskSource,
    write: &ItemWrite<Task>,
) -> Result<NativeId, SourceError> {
    resolving_labels(bound, || source.write_task(write)).await
}

/// Everything this lane needs to reach the one team it may write to.
struct LiveRun {
    endpoint: String,
    key: String,
    /// Which run this is: the machine that can vouch for it and the process on it. Every
    /// artifact below carries it, which is what its own cleanup finds them by and what a
    /// later run's sweep looks this run up by before deciding anything about them.
    id: Run,
    stamp_micros: u64,
    open_state: String,
    done_state: String,
    project_status: String,
    document_mode: Mode,
    standing: Standing,
}

impl LiveRun {
    async fn linear(&self, query: &str, variables: Value, _what: &str) -> Result<Value, String> {
        document_lifecycle::send(&self.endpoint, &self.key, query, variables).await
    }
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
            run.linear(query, variables, "live label lookup")
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
    // And the same for a project, whose spelling `PROJECT_NOT_FOUND_MESSAGE` and its
    // presentable alternative record: a project status write to no project is no such project.
    let missing = source
        .set_project_status(
            &NativeId("00000000-0000-0000-0000-000000000000".into()),
            StatusCategory::Todo,
        )
        .await
        .map_err(|error| format!("the live missing-project status contract drifted: {error}"))?;
    ensure!(
        missing.is_none(),
        "a status mutation to a nonexistent project was not not-found"
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
        version: None,
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
        classification: Default::default(),
    };
    let task = |name: &str, status: &Status, under: Option<&NativeId>, labels: Vec<Label>| Task {
        version: None,
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
        classification: Default::default(),
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
    let first_id = write_labelled_task(
        LINEAR_INDEX,
        source,
        &ItemWrite {
            target: None,
            item: task(
                &first,
                &open,
                Some(&alpha_id),
                vec![label(&run_label), label(&only_label)],
            ),
            depends_on: vec![],
        },
    )
    .await
    .map_err(|error| format!("live task write of {first:?} failed: {error}"))?;
    let second_id = write_labelled_task(
        LINEAR_INDEX,
        source,
        &ItemWrite {
            target: None,
            item: task(&second, &open, Some(&beta_id), vec![label(&run_label)]),
            depends_on: vec![blocks(&first_id, ItemKind::Task)],
        },
    )
    .await
    .map_err(|error| format!("live task write of {second:?} failed: {error}"))?;
    let orphan_id = write_labelled_task(
        LINEAR_INDEX,
        source,
        &ItemWrite {
            target: None,
            item: task(&orphan, &done, None, vec![label(&run_label)]),
            depends_on: vec![],
        },
    )
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
    let listed = settled_projects(LINEAR_INDEX, source, &[alpha.clone(), beta.clone()]).await?;
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
    // exactly once and in the order one whole page reports them. The walk and the page are
    // listings like the others and wait out the index together, so the order is compared over
    // a pair read once the index has caught up rather than over two reads that raced it.
    let walk = "a walk in pages of one over this run's own three issues";
    settled_walk(LINEAR_INDEX, source, &scoped(), 10, walk, &all_three).await?;

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
    run.linear(
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

    drive_document_paths(run, source, &alpha_id, None).await?;
    // Renamed to another of this run's own titles, so the artifact cleanup still knows it.
    every_field_change_moves_a_linear_tasks_token(
        source,
        (&second_id, &first_id, &orphan_id),
        (&title(11), &label(&only_label)),
    )
    .await
}

/// The common document-mode entry used by the real capability lane and HTTP replay.
async fn drive_document_paths(
    run: &LiveRun,
    source: &dyn TaskSource,
    alpha_id: &NativeId,
    journal: Option<&document_lifecycle::journal::Journal>,
) -> Result<(), String> {
    let title = |offset: u64| artifact_title(run.id, run.stamp_micros + offset);
    let document_parent = run.standing.parent.as_ref().map(|id| NativeId(id.clone()));
    let document_parent = document_parent.as_ref().unwrap_or(alpha_id);
    let filed_title = run
        .standing
        .titles
        .get("filed")
        .cloned()
        .unwrap_or_else(|| title(5));
    let loose_title = run
        .standing
        .titles
        .get("loose")
        .cloned()
        .unwrap_or_else(|| title(6));
    if let Some(journal) = journal {
        journal.journey("drive_documents");
    }
    drive_documents(run, source, document_parent, &filed_title, &loose_title).await?;
    let follow_up_title = run
        .standing
        .titles
        .get("follow-up")
        .cloned()
        .unwrap_or_else(|| title(9));
    if let Some(journal) = journal {
        journal.journey("drive_follow_ups");
    }
    drive_follow_ups(
        run,
        source,
        &title(7),
        &title(8),
        &follow_up_title,
        &title(10),
    )
    .await
}

/// A Linear task carries a `lin1` change token through every kind of change a copy has to see,
/// and every change to a member the token covers moves it — to a later one, or to one never
/// ordered beside the last — and what this lane learns along the way is printed as evidence.
///
/// `docs/plugin-protocol.md` §4.13d states what `lin1` covers: a task's fields always, its
/// forward edges only with the store's cache attached — which this source, built without one,
/// is not — and its comments never. So each change is made to one of this run's tasks in turn,
/// a second apart, and every read after it is held to carrying a token; a field change is held
/// to moving it, and a comment or a relation change, which the token does not vouch for here,
/// is only recorded. Whether `updated_at` and the token moved is printed beside each change.
async fn every_field_change_moves_a_linear_tasks_token(
    source: &dyn TaskSource,
    (task, first, orphan): (&NativeId, &NativeId, &NativeId),
    (renamed, extra_label): (&str, &Label),
) -> Result<(), String> {
    use onetaskgraph_plugin_api::{CommentBody, MetadataKey, NewComment, TaskUpdate};
    let read = || async {
        let held = source
            .get_task(task)
            .await
            .map_err(|error| format!("live read of the task failed: {error}"))?
            .ok_or_else(|| "the task was not there".to_owned())?;
        ensure!(
            held.version
                .as_deref()
                .is_some_and(|token| token.starts_with("lin1|")),
            "Linear gave the task {:?} rather than a `lin1` change token, which \
             docs/plugin-protocol.md §4.13d says every whole read carries",
            held.version
        );
        Ok::<Task, String>(held)
    };
    let pause = || tokio::time::sleep(std::time::Duration::from_millis(1_100));
    let mut evidence = Vec::new();
    let mut before = read().await?;
    let mut note = |kind: &str, before: &Task, after: &Task| {
        evidence.push(format!(
            "{kind}: updatedAt {} → {}, token {}",
            before
                .updated_at
                .map_or("none".to_owned(), |at| at.to_rfc3339()),
            after
                .updated_at
                .map_or("none".to_owned(), |at| at.to_rfc3339()),
            if before.version == after.version {
                "unmoved"
            } else {
                "moved"
            },
        ));
    };
    let body = |text: &str| {
        CommentBody::new(text).map_err(|error| format!("a live comment body: {error}"))
    };

    pause().await;
    let comment = source
        .add_comment(
            task,
            &NewComment {
                body: body("temporary credentialed comment; the live lane removes it")?,
                author: None,
            },
        )
        .await
        .map_err(|error| format!("live comment write failed: {error}"))?
        .ok_or_else(|| "the task to comment on was not there".to_owned())?;
    let after = read().await?;
    note("a comment added", &before, &after);
    before = after;

    pause().await;
    source
        .edit_comment(
            task,
            &comment.id,
            &body("temporary credentialed comment, edited")?,
        )
        .await
        .map_err(|error| format!("live comment edit failed: {error}"))?
        .ok_or_else(|| "the comment to edit was not there".to_owned())?;
    let after = read().await?;
    note("a comment edited", &before, &after);
    before = after;

    pause().await;
    source
        .delete_comment(task, &comment.id)
        .await
        .map_err(|error| format!("live comment deletion failed: {error}"))?
        .ok_or_else(|| "the comment to delete was not there".to_owned())?;
    let after = read().await?;
    note("a comment deleted", &before, &after);
    before = after;

    pause().await;
    source
        .set_task_content(task, "temporary credentialed write, its body edited")
        .await
        .map_err(|error| format!("live content write failed: {error}"))?
        .ok_or_else(|| "the task to edit was not there".to_owned())?;
    let after = token_moved(source, LINEAR_REPLICA, "a body edit", &before, &read).await?;
    note("a body edit", &before, &after);
    before = after;

    pause().await;
    source
        .update_task(
            task,
            &TaskUpdate {
                title: Some(renamed.to_owned()),
                ..TaskUpdate::default()
            },
        )
        .await
        .map_err(|error| format!("live title write failed: {error}"))?
        .ok_or_else(|| "the task to rename was not there".to_owned())?;
    let after = token_moved(source, LINEAR_REPLICA, "a title edit", &before, &read).await?;
    note("a title edit", &before, &after);
    before = after;

    pause().await;
    let mut labelled = before.clone();
    labelled.labels.push(extra_label.clone());
    let edges = source
        .task_dependencies(task, Direction::DependsOn, &page(50))
        .await
        .map_err(|error| format!("live dependency read failed: {error}"))?
        .items;
    write_labelled_task(
        LINEAR_INDEX,
        source,
        &ItemWrite {
            target: Some(task.clone()),
            item: labelled,
            depends_on: edges,
        },
    )
    .await
    .map_err(|error| format!("live label write failed: {error}"))?;
    let after = token_moved(source, LINEAR_REPLICA, "a label change", &before, &read).await?;
    note("a label change", &before, &after);
    before = after;

    pause().await;
    source
        .set_task_metadata(
            task,
            &MetadataKey::new("live.change_kind").map_err(|error| format!("a key: {error}"))?,
            &json!("metadata"),
        )
        .await
        .map_err(|error| format!("live metadata write failed: {error}"))?
        .ok_or_else(|| "the task to annotate was not there".to_owned())?;
    let after = token_moved(source, LINEAR_REPLICA, "a metadata change", &before, &read).await?;
    note("a metadata change", &before, &after);
    before = after;

    pause().await;
    source
        .set_task_priority(task, Priority::Low)
        .await
        .map_err(|error| format!("live priority write failed: {error}"))?
        .ok_or_else(|| "the task to prioritise was not there".to_owned())?;
    let after = token_moved(source, LINEAR_REPLICA, "a priority change", &before, &read).await?;
    note("a priority change", &before, &after);
    before = after;

    for (kind, on) in [
        ("a dependency added", vec![first, orphan]),
        ("a dependency removed", vec![first]),
    ] {
        pause().await;
        source
            .update_task(
                task,
                &TaskUpdate {
                    depends_on: Some(
                        on.into_iter()
                            .map(|far| blocks(far, ItemKind::Task))
                            .collect(),
                    ),
                    ..TaskUpdate::default()
                },
            )
            .await
            .map_err(|error| format!("live dependency write failed: {error}"))?
            .ok_or_else(|| "the task to make depend was not there".to_owned())?;
        let after = read().await?;
        note(kind, &before, &after);
        before = after;
    }

    pause().await;
    source
        .set_task_status(task, StatusCategory::Done)
        .await
        .map_err(|error| format!("live status write failed: {error}"))?
        .ok_or_else(|| "the task to finish was not there".to_owned())?;
    let after = token_moved(source, LINEAR_REPLICA, "a status change", &before, &read).await?;
    note("a status change", &before, &after);

    eprintln!(
        "Linear's Issue.updatedAt and lin1 token across every change a copy has to see:\n  {}",
        evidence.join("\n  ")
    );
    Ok(())
}

/// One issue's raw `description`, as Linear hands it back.
async fn raw_description(run: &LiveRun, id: &NativeId) -> Result<String, String> {
    run.linear(
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
async fn raw_content(run: &LiveRun, id: &NativeId, expected: &str) -> Result<String, String> {
    settled_content(LINEAR_REPLICA, expected, || async {
        run.linear(
            "query($id:String!){ document(id:$id){ content } }",
            json!({"id":id.0}),
            "live raw document read",
        )
        .await?
        .pointer("/document/content")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("document {} carried no content", id.0))
    })
    .await
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

/// One instance of each rewrite Linear was observed to make of Markdown on write but the lone
/// image, which `linear_assets`' live journey writes with real uploads: a
/// bullet the next lines run on from, a URL, two domain-like tokens, brackets, a tilde, an
/// underscore emphasis, an escaped quote, a backslash before a letter, a line opening `-->`, a
/// fenced block and a line after it, each set apart, the same text inside the fence and an
/// inline code span, where it is kept, a run of blank lines, and last a table, whose delimiter
/// row is one only outside a list item and which ends the content with no line break after it.
const MARKDOWN_PROBE: &str = "- bullet\n| a | b |\n| --- | --- |\nsee https://example.com/a.\nexample.com and \"caller.live\"\n[x] done ~5 _y_\nsay \\\"hi\\\" a \\d b\n-->\n```\n- [x] _y_ example.com ~\n```\nkept `- [x] _y_ example.com ~` span\n\n\n\n| a | b |\n| --- | --- |\n";

/// What Linear stores [`MARKDOWN_PROBE`] as, and what the live leg asserts Linear hands back: byte
/// for byte what real Linear handed back from an issue's description in CI run 39375395042, of
/// this probe as it was then — one blank line where it now holds a run of them, and no line
/// break after the table. The plugin predicts none of it; it reads what Linear stores off each
/// write's own answer.
const MARKDOWN_PROBE_STORED: &str = "* bullet\n  | a | b |\n  | --- | --- |\n  see [https://example.com/a](<https://example.com/a>).\n  [example.com](<http://example.com>) and \"[caller.live](<http://caller.live>)\"\n  \\[x\\] done \\~5 *y*\n  say \"hi\" a \\\\d b\n  \\-->\n\n```\n- [x] _y_ example.com ~\n```\n\nkept `- [x] _y_ example.com ~` span\n\n| a | b |\n| -- | -- |\n";

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
        version: None,
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
        classification: Default::default(),
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
    run.linear("mutation($id:String!,$input:IssueUpdateInput!){ issueUpdate(id:$id,input:$input){success} }",
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
            target: run.standing.ids.get("follow-up").cloned().map(NativeId),
            item: Document {
                version: None,
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
                classification: Default::default(),
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
    let updated = run.linear(
        "mutation($id:String!,$input:DocumentUpdateInput!){ documentUpdate(id:$id,input:$input){success document{content}} }",
        json!({"id":document_id.0,"input":{"content":PROBE}}),
        "live content probe",
    )
    .await?;
    ensure!(
        updated
            .pointer("/documentUpdate/success")
            .and_then(Value::as_bool)
            == Some(true)
            && updated
                .pointer("/documentUpdate/document/content")
                .and_then(Value::as_str)
                == Some(PROBE_STORED),
        "the document update did not acknowledge the exact stored content: {updated:?}"
    );
    let content = raw_content(run, &document_id, PROBE_STORED).await?;
    ensure!(
        content == PROBE_STORED,
        "a document's HTML comments came back as {content:?}, not as recorded"
    );

    // What Linear does to Markdown on write, rule by rule, in both long-form fields.
    run.linear("mutation($id:String!,$input:IssueUpdateInput!){ issueUpdate(id:$id,input:$input){success} }",
        json!({"id":probe_id.0,"input":{"description":MARKDOWN_PROBE}}),
        "live Markdown description probe",
    )
    .await?;
    let stored = raw_description(run, &probe_id).await?;
    ensure!(
        stored == MARKDOWN_PROBE_STORED,
        "an issue description's Markdown came back as {stored:?}, not as the ruling declares"
    );
    let read = source
        .get_task(&probe_id)
        .await
        .map_err(|error| format!("the Markdown probe's issue could not be read: {error}"))?
        .and_then(|task| task.content);
    ensure!(
        read.as_deref() == Some(MARKDOWN_PROBE_STORED),
        "a read of the Markdown probe's issue handed back {read:?}, not its stored form"
    );
    let updated = run.linear(
        "mutation($id:String!,$input:DocumentUpdateInput!){ documentUpdate(id:$id,input:$input){success document{content}} }",
        json!({"id":document_id.0,"input":{"content":MARKDOWN_PROBE}}),
        "live Markdown content probe",
    )
    .await?;
    ensure!(
        updated
            .pointer("/documentUpdate/success")
            .and_then(Value::as_bool)
            == Some(true)
            && updated
                .pointer("/documentUpdate/document/content")
                .and_then(Value::as_str)
                == Some(MARKDOWN_PROBE_STORED),
        "the document update did not acknowledge the exact stored content: {updated:?}"
    );
    let content = raw_content(run, &document_id, MARKDOWN_PROBE_STORED).await?;
    ensure!(
        content == MARKDOWN_PROBE_STORED,
        "a document's Markdown came back as {content:?}, not as the ruling declares"
    );
    let read = Some(
        settled_content(LINEAR_REPLICA, MARKDOWN_PROBE_STORED, || async {
            source
                .get_document(&document_id)
                .await
                .map_err(|error| {
                    format!("the Markdown probe's document could not be read: {error}")
                })?
                .and_then(|document| document.content)
                .ok_or_else(|| "the Markdown probe's document carried no content".to_owned())
        })
        .await?,
    );
    ensure!(
        read.as_deref() == Some(MARKDOWN_PROBE_STORED),
        "a read of the Markdown probe's document handed back {read:?}, not its stored form"
    );
    // And with this source's metadata slot after it: the table keeps its line break, and the
    // slot follows one blank line later, so a read hands the content back as it was written.
    let tabled = "Owners:\n\n| a | b |\n| -- | -- |\n".to_owned();
    source
        .write_document(&ItemWrite {
            target: Some(document_id.clone()),
            item: Document {
                version: None,
                id: document_id.clone(),
                title: document_title.to_owned(),
                content: Some(tabled.clone()),
                project: None,
                labels: Vec::new(),
                url: None,
                location: None,
                created_at: None,
                updated_at: None,
                metadata: serde_json::from_value(json!({"caller.live": tag})).unwrap_or_default(),
                repositories: Vec::new(),
                classification: Default::default(),
            },
            depends_on: Vec::new(),
        })
        .await
        .map_err(|error| format!("live table document write failed: {error}"))?;
    let read = source
        .get_document(&document_id)
        .await
        .map_err(|error| format!("the table document could not be read: {error}"))?;
    ensure!(
        read.as_ref()
            .and_then(|document| document.content.as_deref())
            == Some(tabled.as_str())
            && read
                .as_ref()
                .and_then(|document| document.metadata.get("caller.live"))
                == Some(&json!(tag)),
        "a document ending in a table, written with a slot, read back as {read:?}, not as {tabled:?}"
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
        let data = run
            .linear(
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
        version: None,
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
        metadata: [
            ("caller.count".to_owned(), json!(3)),
            ("caller.run".to_owned(), json!(run.stamp_micros)),
        ]
        .into_iter()
        .collect(),
        repositories: vec![],
        classification: Default::default(),
    };
    let write = |item: Document| ItemWrite {
        target: if item.title == filed {
            run.standing.ids.get("filed")
        } else {
            run.standing.ids.get("loose")
        }
        .cloned()
        .map(NativeId),
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
        read.metadata.get("caller.run") == Some(&json!(run.stamp_micros)),
        "the retained document did not reset caller.run to this invocation: {:?}",
        read.metadata.get("caller.run")
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
    for (id, title) in [(&filed_id, filed), (&loose_id, loose)]
        .into_iter()
        .filter(|_| run.document_mode == Mode::Fresh)
    {
        source
            .delete_document(id)
            .await
            .map_err(|error| format!("live document removal failed: {error}"))?;
        settled_document_absent(LINEAR_INDEX, source, id, title).await?;
    }
    Ok(())
}

/// Where the lane sends what it sends to Linear — [`document_lifecycle::linear_endpoint`] —
/// with its cleanup pointed there too when that is a loopback stand-in.
fn linear_graphql() -> String {
    let url = document_lifecycle::linear_endpoint();
    if document_lifecycle::loopback_endpoint().is_some() {
        cleanup::against(&url);
    }
    url
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
    let document_mode = Mode::read().unwrap_or_else(|e| panic!("{e}"));
    let owner = Run::current();
    let stamp = onetaskgraph_live::artifact::Stamp::new(owner, now_micros()).to_string();
    let team_key = team.to_ascii_uppercase();
    let ci = env::var("GITHUB_ACTIONS").as_deref() == Ok("true");
    let effective_mode = document_mode;
    if !ci && document_mode == Mode::Initialize {
        eprintln!(
            "non-CI creation pass: initialize invocation-owned plugin document slots, exercise reuse assertions, and retire the set"
        );
    }
    // Where this lane's Linear is: Linear, unless a credential-free drive points it at a
    // loopback stand-in (see `linear_graphql`).
    let linear_graphql = linear_graphql();
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
    // The operator's one-time pool setup, asked for by `just linear-document-pool`, in place
    // of the journey: every member's plugin slots, reported or created under its lease.
    if let Some(setup) = document_lifecycle::PoolSetup::read().unwrap_or_else(|e| panic!("{e}")) {
        let report = document_lifecycle::setup_pool(
            &linear_graphql,
            &key,
            &team_key,
            &team_id,
            LeaseLane::Plugin,
            &[Slot::Filed, Slot::Loose, Slot::FollowUp],
            &stamp,
            setup,
        )
        .await
        .unwrap_or_else(|error| panic!("the Linear plugin pool setup failed: {error}"));
        for line in report {
            println!("{line}");
        }
        return;
    }
    let journal = document_lifecycle::journal::Journal::new("plugin", effective_mode.name())
        .durable(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("telemetry/onetaskgraph-linear/live")
                .join(&stamp),
        )
        .unwrap();
    // Everything the source sends passes this run's namespace guard: a request naming
    // another run's record is refused before it reaches Linear.
    let namespace = std::sync::Arc::new(document_lifecycle::namespace::Namespace::new(
        owner, &team_key,
    ));
    let endpoint = document_lifecycle::observer::guarded(
        &linear_graphql,
        &key,
        journal.clone(),
        std::sync::Arc::clone(&namespace),
    );
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
            &json!({"endpoint":endpoint,"team":team,"status_mapping":{
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
            reconcile_status: Support::Unsupported,
            complete_by_issue: Support::Unsupported,
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

    // A CI run works over the one standing pool member it leases, and no other run's: see
    // `document_lifecycle::lease`. A local run's set is its own by its stamp already.
    let lease = if ci {
        Some(
            document_lifecycle::lease::claim(
                &linear_graphql,
                &key,
                &team_key,
                &team_id,
                LeaseLane::Plugin,
                &stamp,
                now_micros,
            )
            .await
            .unwrap_or_else(|error| panic!("the Linear live lane cannot lease its slots: {error}")),
        )
    } else {
        None
    };
    let scope = match &lease {
        Some(lease) => {
            namespace.leased(lease.index);
            lease.scope()
        }
        None => Scope::local(&team_key, &stamp).unwrap(),
    };
    // From the first create on, one guard retires what this invocation created: preparation,
    // the setup after it and the journey all run inside `guard_plugin_invocation`.
    let created = &document_lifecycle::Created::default();
    let lane_endpoint = endpoint.clone();
    let lane_key = key.clone();
    document_lifecycle::lease::holding(lease.as_ref(), &linear_graphql, &lane_key, async {
        // Exclusivity is Linear's refusal of a second create under a held id: proven
        // against Linear on every claim, before anything is done over the member.
        if let Some(lease) = &lease {
            document_lifecycle::lease::verify_exclusive(
                &linear_graphql,
                &lane_key,
                &team_id,
                lease,
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        }
        guard_plugin_invocation(
            &lane_endpoint,
            &lane_key,
            &team_key,
            &team_id.clone(),
            scope,
            effective_mode,
            created,
            |standing| async move {
                if ci && document_mode == Mode::Initialize {
                    journal.finish().unwrap();
                    return;
                }
                journal.journey("capability-and-follow-up");
                let run = LiveRun {
                    endpoint,
                    key: key.clone(),
                    id: Run::current(),
                    stamp_micros: now_micros(),
                    open_state,
                    done_state,
                    project_status,
                    document_mode,
                    standing,
                };
                let id = run.id;
                // `ensure!` keeps the journey's own checks to `Err`, but anything else that panics
                // in it must still reach the cleanup below; the panic is resumed once cleanup ran.
                let panicked = std::sync::Mutex::new(None);
                let result = run_then_cleanup(
                    || async {
                        match document_lifecycle::unwinding(drive_every_declared_capability(
                            &run,
                            source.as_ref(),
                            &team_id,
                        ))
                        .await
                        {
                            Ok(outcome) => outcome,
                            Err(panic) => {
                                *panicked.lock().unwrap() = Some(panic);
                                Err("the Linear live journey panicked".into())
                            }
                        }
                    },
                    || async {
                        // The invocation-owned slots by their exact ids first, then this run's
                        // own: everything it wrote goes whether the journey passed or failed.
                        // Then what an interrupted EARLIER run left, which is a different
                        // decision on different evidence — and which is deliberately here, at
                        // the end, rather than at the start where it used to be. A sweep before
                        // the journey deleted the in-flight issues of any session running beside
                        // this one; a sweep after it recovers exactly the same orphans and can
                        // reach nothing a live run owns.
                        let slots =
                            document_lifecycle::retire(&run.endpoint, &key, scope, created).await;
                        let mine =
                            remove_artifacts(&key, &|prefix, name| is_this_runs(id, prefix, name))
                                .await
                                .and(slots.map(drop));
                        let orphans = sweep_orphans(&key, &Sweep::of(id, now_micros())).await;
                        match (mine, orphans) {
                            (Ok(()), Ok(())) => Ok(()),
                            (Err(mine), Ok(())) => Err(mine),
                            (Ok(()), Err(orphans)) => Err(format!(
                                "residue left by an earlier interrupted run could not be \
                             cleared: {orphans}"
                            )),
                            (Err(mine), Err(orphans)) => Err(format!(
                                "{mine}; additionally, residue left by an earlier interrupted \
                             run could not be cleared: {orphans}"
                            )),
                        }
                    },
                )
                .await;
                journal.finish().unwrap();
                if let Some(panic) = panicked.into_inner().unwrap() {
                    if let Err(error) = result {
                        eprintln!("{error}");
                    }
                    std::panic::resume_unwind(panic);
                }
                result.unwrap_or_else(|error| {
                    panic!("Linear live capability journey failed: {error}")
                });
            },
        )
        .await
    })
    .await;
    let refused = namespace.refusals();
    assert!(
        refused.is_empty(),
        "the Linear live lane named records outside its run's namespace: {refused:?}"
    );
}

/// The plugin lane's one invocation cleanup guard: it prepares the standing set and runs
/// `invocation` — the setup after preparation and the journey — over it, so that a panic at
/// any point from the first slot create on, whether inside preparation, straight after it or
/// anywhere before the journey's own cleanup ran, retires every slot `created` records before
/// the panic leaves the lane. A CI set is standing and is never retired here, and once the
/// journey's cleanup has retired the slots the record is empty, so nothing is deleted twice.
#[allow(
    clippy::too_many_arguments,
    reason = "prepare_owned's lane identities plus the invocation; the offline journey passes \
              each one separately to drive this guard as the live lane does"
)]
async fn guard_plugin_invocation<F, Fut>(
    endpoint: &str,
    key: &str,
    team_key: &str,
    team_id: &str,
    scope: Scope<'_>,
    mode: Mode,
    created: &document_lifecycle::Created,
    invocation: F,
) where
    F: FnOnce(Standing) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let guarded = document_lifecycle::unwinding(async {
        let standing = document_lifecycle::prepare_owned(
            endpoint,
            key,
            team_key,
            team_id,
            scope,
            mode,
            &[Slot::Filed, Slot::Loose, Slot::FollowUp],
            created,
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        invocation(standing).await;
    })
    .await;
    if let Err(panic) = guarded {
        if let Err(error) = document_lifecycle::retire(endpoint, key, scope, created).await {
            eprintln!("{error}");
        }
        std::panic::resume_unwind(panic);
    }
}

#[path = "document_lifecycle/replay.rs"]
mod document_replay;

fn replay() -> document_replay::Replay {
    document_replay::Replay::start(
        document_lifecycle::journal::Journal::new("plugin", "initialize"),
        [
            (PROBE.to_owned(), PROBE_STORED.to_owned()),
            (MARKDOWN_PROBE.to_owned(), MARKDOWN_PROBE_STORED.to_owned()),
        ]
        .into_iter()
        .collect(),
    )
}
async fn replay_prepare(
    replay: &document_replay::Replay,
    scope: Scope<'_>,
    mode: Mode,
) -> Result<document_lifecycle::Standing, String> {
    document_lifecycle::prepare(
        &replay.endpoint,
        "synthetic",
        "ENG",
        "team",
        scope,
        mode,
        &[Slot::Filed, Slot::Loose, Slot::FollowUp],
    )
    .await
}
async fn replay_drive(
    replay: &document_replay::Replay,
    standing: document_lifecycle::Standing,
    mode: Mode,
    stamp: u64,
) -> Result<(), String> {
    replay_drive_at(&replay.endpoint, &replay.journal, standing, mode, stamp).await
}
async fn replay_drive_at(
    endpoint: &str,
    journal: &document_lifecycle::journal::Journal,
    standing: document_lifecycle::Standing,
    mode: Mode,
    stamp: u64,
) -> Result<(), String> {
    replay_drive_as(endpoint, journal, standing, mode, stamp, Run::current()).await
}
/// [`replay_drive_at`] as `run`, so sessions of one test process can stand for distinct runs.
async fn replay_drive_as(
    endpoint: &str,
    journal: &document_lifecycle::journal::Journal,
    standing: document_lifecycle::Standing,
    mode: Mode,
    stamp: u64,
    owner: Run,
) -> Result<(), String> {
    let source = onetaskgraph_linear::Plugin.build(&SourceName::new("live").unwrap(),
        &json!({"endpoint":endpoint,"team":"ENG","status_mapping":{"todo":{"task":"Todo","project":"Planned"},"done":{"task":"Done"}}}),&ReplaySecrets).unwrap();
    let parent = if let Some(parent) = &standing.parent {
        parent.clone()
    } else {
        let data = document_lifecycle::send(endpoint, "synthetic",
            "mutation($input:ProjectCreateInput!){projectCreate(input:$input){success project{id}}}",
            json!({"input":{"name":artifact_title(owner, stamp),"teamIds":["team"]}})).await?;
        data["projectCreate"]["project"]["id"]
            .as_str()
            .ok_or("fresh parent create omitted id")?
            .to_owned()
    };
    let run = LiveRun {
        endpoint: endpoint.to_owned(),
        key: "synthetic".into(),
        id: owner,
        stamp_micros: stamp,
        open_state: "Todo".into(),
        done_state: "Done".into(),
        project_status: "Planned".into(),
        document_mode: mode,
        standing,
    };
    drive_document_paths(&run, source.as_ref(), &NativeId(parent), Some(journal)).await
}

/// One ordinary CI pass through the plugin lane's own invocation guard, recording the
/// standing set it ran over.
async fn replay_ordinary_ci_pass(
    replay: &document_replay::Replay,
    stamp: u64,
    standing_ids: &std::sync::Mutex<Option<BTreeMap<&'static str, String>>>,
) {
    guard_plugin_invocation(
        &replay.endpoint,
        "synthetic",
        "ENG",
        "team",
        Scope::ci("ENG").unwrap(),
        Mode::Ordinary,
        &document_lifecycle::Created::default(),
        |standing| async move {
            *standing_ids.lock().unwrap() = Some(standing.ids.clone());
            replay_drive(replay, standing, Mode::Ordinary, stamp)
                .await
                .unwrap();
        },
    )
    .await;
}

async fn replay_cleanup_documents(replay: &document_replay::Replay) -> usize {
    let query = "query($filter:DocumentFilter!){documents(first:100,filter:$filter){nodes{id title} pageInfo{hasNextPage endCursor}}}";
    let read = || {
        document_lifecycle::send(
            &replay.endpoint,
            "synthetic",
            query,
            json!({"filter":{"title":{"containsIgnoreCase":onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX}}}),
        )
    };
    let listing = read().await.unwrap();
    let owned = |row: &Value| {
        row["title"].as_str().is_some_and(|title| {
            is_this_runs(
                Run::current(),
                onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX,
                title,
            )
        })
    };
    for document in listing["documents"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| owned(row))
    {
        document_lifecycle::send(
            &replay.endpoint,
            "synthetic",
            onetaskgraph_linear::graphql::DOCUMENT_DELETE,
            json!({"id":document["id"]}),
        )
        .await
        .unwrap();
    }
    let after = read().await.unwrap();
    after["documents"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| owned(row))
        .count()
}

/// Both modes run the same capability and follow-up functions as the credentialed lane.
#[tokio::test]
async fn document_modes_replay_the_live_plugin_paths_over_http() {
    let replay = replay();
    let scope = Scope::ci("ENG").unwrap();
    // A default non-CI invocation refuses before any create and names the explicit mode,
    // which initializes its own set and finds it again on recovery.
    let local = Scope::local("ENG", "1-2-900").unwrap();
    assert!(
        replay_prepare(&replay, local, Mode::Ordinary)
            .await
            .err()
            .unwrap()
            .contains("initialize")
    );
    assert_eq!(replay.journal.count(), 0);
    let created = document_lifecycle::Created::default();
    let standing = document_lifecycle::prepare_owned(
        &replay.endpoint,
        "synthetic",
        "ENG",
        "team",
        local,
        Mode::Initialize,
        &[Slot::Filed, Slot::Loose, Slot::FollowUp],
        &created,
    )
    .await
    .unwrap();
    assert_eq!(
        replay.journal.count(),
        3,
        "one owned document per missing plugin slot"
    );
    let again = replay_prepare(&replay, local, Mode::Initialize)
        .await
        .unwrap();
    assert_eq!(standing.ids, again.ids);
    assert_eq!(replay.journal.count(), 3);
    replay_drive(&replay, standing, Mode::Ordinary, now_micros())
        .await
        .unwrap();
    assert_eq!(replay.journal.count(), 3);
    document_lifecycle::retire(&replay.endpoint, "synthetic", local, &created)
        .await
        .unwrap();
    replay.journal.finish().unwrap();
    // The CI set is absent: the first ordinary pass rebuilds it through the lane's own guard,
    // files the creates as a cold rebuild, and then runs its reuse assertions over the set.
    replay.journal.begin("ordinary");
    let stamp = now_micros();
    let rebuilt = std::sync::Mutex::new(None);
    replay_ordinary_ci_pass(&replay, stamp, &rebuilt).await;
    let rebuilt = rebuilt.into_inner().unwrap().unwrap();
    assert_eq!(rebuilt.len(), 4, "a parent and three document slots");
    assert_eq!(
        replay
            .journal
            .count_in(document_lifecycle::journal::COLD_REBUILD),
        3,
        "exactly the three missing documents, filed as a cold rebuild"
    );
    assert_eq!(
        replay.journal.count(),
        3,
        "the reuse assertions create none"
    );
    // A second, warm ordinary pass resets the same retained contents and creates nothing.
    let warm = std::sync::Mutex::new(None);
    replay_ordinary_ci_pass(&replay, stamp + 1, &warm).await;
    assert_eq!(warm.into_inner().unwrap().unwrap(), rebuilt);
    assert_eq!(
        replay.journal.count(),
        3,
        "a warm ordinary pass creates none"
    );
    replay.journal.finish().unwrap();
    replay.journal.begin("fresh");
    let standing = replay_prepare(&replay, scope, Mode::Fresh).await.unwrap();
    replay_drive(&replay, standing, Mode::Fresh, stamp + 2)
        .await
        .unwrap();
    assert_eq!(
        replay.journal.count(),
        3,
        "two document writes and the follow-up document write"
    );
    replay.journal.finish().unwrap();
    // The original filed/loose deletion assertions ran in drive_documents; retire the
    // follow-up using the same documentDelete HTTP boundary as invocation cleanup.
    let fresh_ids: Vec<_> = replay
        .state
        .lock()
        .unwrap()
        .documents
        .iter()
        .filter(|(_, row)| {
            row["title"].as_str().is_some_and(|t| {
                is_this_runs(
                    Run::current(),
                    onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX,
                    t,
                )
            })
        })
        .map(|(id, _)| id.clone())
        .collect();
    assert_eq!(fresh_ids.len(), 1);
    for id in fresh_ids {
        document_lifecycle::send(
            &replay.endpoint,
            "synthetic",
            onetaskgraph_linear::graphql::DOCUMENT_DELETE,
            json!({"id":id}),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        replay.journal.count(),
        3,
        "deletion never removes successful creations from the journal"
    );
    replay.journal.begin("failure");
    replay.state.lock().unwrap().corrupt_document_create_ack = true;
    let standing = replay_prepare(&replay, scope, Mode::Fresh).await.unwrap();
    let error = replay_drive(&replay, standing, Mode::Fresh, stamp + 3)
        .await
        .unwrap_err();
    assert!(error.contains("visible body"), "{error}");
    assert_eq!(
        replay.journal.count(),
        2,
        "both writes before the assertion failure remain counted"
    );
    let residue = replay_cleanup_documents(&replay).await;
    assert_eq!(
        residue, 0,
        "failed fresh documents retire while standing CI slots survive"
    );
    replay.journal.residue(residue);
    replay.journal.finish().unwrap();
    replay.state.lock().unwrap().corrupt_document_create_ack = false;
    let standing = replay_prepare(&replay, scope, Mode::Ordinary)
        .await
        .unwrap();
    replay_drive(&replay, standing, Mode::Ordinary, stamp + 4)
        .await
        .unwrap();
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}

#[tokio::test]
async fn document_lifecycle_replay_refuses_unsafe_ownership_and_recovers_partial_initialization() {
    let replay = replay();
    let scope = Scope::ci("ENG").unwrap();
    replay.state.lock().unwrap().uncertain = true;
    let error = replay_prepare(&replay, scope, Mode::Initialize)
        .await
        .err()
        .unwrap();
    assert!(error.contains("discover"), "{error}");
    assert_eq!(
        replay.journal.count(),
        1,
        "uncertain acknowledgement still records the stored create"
    );
    // The ordinary CI pass resumes that partial initialization as a cold rebuild.
    let recovered = replay_prepare(&replay, scope, Mode::Ordinary)
        .await
        .unwrap();
    assert_eq!(
        replay.journal.count(),
        3,
        "discovery reuses the uncertain first create and fills only missing slots"
    );
    assert_eq!(recovered.ids.len(), 4);
    let again = replay_prepare(&replay, scope, Mode::Ordinary)
        .await
        .unwrap();
    assert_eq!(again.ids, recovered.ids);
    assert_eq!(
        replay.journal.count(),
        3,
        "a warm pass creates no duplicate"
    );
    // A cancelled invocation leaves reusable contents, not missing identities.
    replay_drive(&replay, recovered, Mode::Ordinary, now_micros())
        .await
        .unwrap();
    for defect in ["wrong-team", "unverifiable-team", "ambiguous", "unknown"] {
        let before = {
            let mut state = replay.state.lock().unwrap();
            state.wrong_team = defect == "wrong-team";
            state.unverifiable_team = defect == "unverifiable-team";
            state.ambiguous = defect == "ambiguous";
            if defect == "unknown" {
                state.documents.values_mut().next().unwrap()["title"] =
                    json!("onetaskgraph live cleanup retained-v2 ci ENG filed");
            }
            state.requests.len()
        };
        let error = replay_prepare(&replay, scope, Mode::Ordinary)
            .await
            .err()
            .unwrap();
        assert!(
            error.contains("ownership") || error.contains("membership"),
            "{defect}: {error}"
        );
        let state = replay.state.lock().unwrap();
        assert!(
            state.requests[before..]
                .iter()
                .all(|r| !r["query"].as_str().unwrap().contains("mutation")),
            "{defect} sent a write"
        );
    }
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}

/// The transport's own record of a replay: each create and delete mutation it answered.
fn replay_mutations(
    replay: &document_replay::Replay,
    from: usize,
) -> BTreeMap<&'static str, usize> {
    let state = replay.state.lock().unwrap();
    let mut counted = BTreeMap::new();
    for request in &state.requests[from..] {
        let query = request["query"].as_str().unwrap();
        for root in [
            "projectCreate",
            "documentCreate",
            "projectDelete",
            "documentDelete",
        ] {
            if query.contains(&format!("{root}(")) {
                *counted.entry(root).or_default() += 1;
            }
        }
    }
    counted
}

/// A panic straight after each individual slot create of a local initialization — the
/// parent project, then each of the three documents — still retires every slot the
/// invocation created before the panic leaves the lane, and nothing else.
#[tokio::test]
async fn a_panic_after_each_local_slot_create_retires_what_the_invocation_created() {
    let replay = replay();
    let ci = replay_prepare(&replay, Scope::ci("ENG").unwrap(), Mode::Initialize)
        .await
        .unwrap();
    for after in 1..=4 {
        let stamp = format!("1-2-{after}");
        let scope = Scope::local("ENG", &stamp).unwrap();
        let before = replay.state.lock().unwrap().requests.len();
        let creates_before = replay.journal.count();
        let created = document_lifecycle::Created::panicking_after(after);
        let outcome = document_lifecycle::unwinding(document_lifecycle::prepare_owned(
            &replay.endpoint,
            "synthetic",
            "ENG",
            "team",
            scope,
            Mode::Initialize,
            &[Slot::Filed, Slot::Loose, Slot::FollowUp],
            &created,
        ))
        .await;
        let panic = outcome.err().expect("the injected failure panics the lane");
        assert_eq!(
            panic.downcast_ref::<String>().map(String::as_str),
            Some(format!("injected failure after standing create {after}").as_str())
        );
        let sent = replay_mutations(&replay, before);
        let projects = 1;
        let documents = after - projects;
        assert_eq!(
            sent.get("projectCreate").copied().unwrap_or(0),
            projects,
            "after {after}"
        );
        assert_eq!(
            sent.get("documentCreate").copied().unwrap_or(0),
            documents,
            "after {after}"
        );
        assert_eq!(
            sent.get("projectDelete").copied().unwrap_or(0),
            projects,
            "after {after}: the parent created is the parent retired"
        );
        assert_eq!(
            sent.get("documentDelete").copied().unwrap_or(0),
            documents,
            "after {after}: every document created is retired"
        );
        assert_eq!(
            replay.journal.count() - creates_before,
            documents,
            "after {after}: retired documents stay counted as created"
        );
        let state = replay.state.lock().unwrap();
        assert!(
            state
                .documents
                .values()
                .chain(state.projects.values())
                .all(|row| !row["title"]
                    .as_str()
                    .or(row["name"].as_str())
                    .unwrap_or("")
                    .contains(&stamp)),
            "after {after}: nothing of the invocation's set survives"
        );
        assert_eq!(created.len(), 0);
    }
    let survived = replay_prepare(&replay, Scope::ci("ENG").unwrap(), Mode::Ordinary)
        .await
        .unwrap();
    assert_eq!(survived.ids, ci.ids, "the standing CI set is untouched");
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}

/// A panic straight after a successful local preparation — before any of the journey's
/// setup — is caught by the plugin lane's own invocation guard, which retires the parent
/// project and all three documents the invocation created; the standing CI set and an
/// unrelated document and project survive, and the creations stay journalled.
#[tokio::test]
async fn a_panic_straight_after_local_preparation_is_retired_by_the_plugin_invocation_guard() {
    let replay = replay();
    let ci = replay_prepare(&replay, Scope::ci("ENG").unwrap(), Mode::Initialize)
        .await
        .unwrap();
    let unrelated_document = document_lifecycle::send(
        &replay.endpoint,
        "synthetic",
        "mutation($input:DocumentCreateInput!){documentCreate(input:$input){success document{id}}}",
        json!({"input":{"title":"Unrelated synthetic document","content":"synthetic","teamId":"team"}}),
    )
    .await
    .unwrap()["documentCreate"]["document"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let unrelated_project = document_lifecycle::send(
        &replay.endpoint,
        "synthetic",
        "mutation($input:ProjectCreateInput!){projectCreate(input:$input){success project{id}}}",
        json!({"input":{"name":"Unrelated synthetic project","teamIds":["team"]}}),
    )
    .await
    .unwrap()["projectCreate"]["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let stamp = "1-2-9";
    let scope = Scope::local("ENG", stamp).unwrap();
    let before = replay.state.lock().unwrap().requests.len();
    let creates_before = replay.journal.count();
    let created = document_lifecycle::Created::default();
    let prepared = std::sync::Mutex::new(None);
    let outcome = document_lifecycle::unwinding(guard_plugin_invocation(
        &replay.endpoint,
        "synthetic",
        "ENG",
        "team",
        scope,
        Mode::Initialize,
        &created,
        |standing| {
            *prepared.lock().unwrap() = Some(standing.ids.clone());
            async {
                panic!("injected failure straight after local preparation");
            }
        },
    ))
    .await;
    let panic = outcome.expect_err("the injected failure still fails the lane");
    assert_eq!(
        panic.downcast_ref::<&str>().copied(),
        Some("injected failure straight after local preparation")
    );
    let prepared = prepared
        .into_inner()
        .unwrap()
        .expect("preparation succeeded first");
    assert_eq!(
        prepared.len(),
        4,
        "the parent and all three local documents were created"
    );
    let sent = replay_mutations(&replay, before);
    assert_eq!(sent.get("projectCreate"), Some(&1));
    assert_eq!(sent.get("documentCreate"), Some(&3));
    assert_eq!(sent.get("projectDelete"), Some(&1), "the parent is retired");
    assert_eq!(
        sent.get("documentDelete"),
        Some(&3),
        "every document is retired"
    );
    assert_eq!(
        replay.journal.count() - creates_before,
        3,
        "retired creations stay journalled"
    );
    {
        let state = replay.state.lock().unwrap();
        for id in prepared.values() {
            assert!(
                !state.documents.contains_key(id) && !state.projects.contains_key(id),
                "{id} survived"
            );
        }
        assert!(state.documents.contains_key(&unrelated_document));
        assert!(state.projects.contains_key(&unrelated_project));
    }
    assert_eq!(created.len(), 0);
    let survived = replay_prepare(&replay, Scope::ci("ENG").unwrap(), Mode::Ordinary)
        .await
        .unwrap();
    assert_eq!(survived.ids, ci.ids, "the standing CI set is untouched");
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}

/// An uncertain local create — stored, but acknowledged without its id — is found by its
/// exact identity and retired with the slot created before it; a CI set keeps its slots for
/// the discovery recovery requires.
#[tokio::test]
async fn an_uncertain_local_slot_create_is_discovered_and_retired_before_the_lane_fails() {
    let replay = replay();
    replay.state.lock().unwrap().uncertain = true;
    let created = document_lifecycle::Created::default();
    let error = document_lifecycle::prepare_owned(
        &replay.endpoint,
        "synthetic",
        "ENG",
        "team",
        Scope::local("ENG", "1-2-3").unwrap(),
        Mode::Initialize,
        &[Slot::Filed, Slot::Loose, Slot::FollowUp],
        &created,
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("discover"), "{error}");
    let sent = replay_mutations(&replay, 0);
    assert_eq!(sent.get("projectCreate"), Some(&1));
    assert_eq!(sent.get("documentCreate"), Some(&1));
    assert_eq!(sent.get("projectDelete"), Some(&1));
    assert_eq!(sent.get("documentDelete"), Some(&1));
    assert_eq!(
        replay.journal.count(),
        1,
        "the retired create stays counted"
    );
    {
        let state = replay.state.lock().unwrap();
        assert!(state.documents.is_empty() && state.projects.is_empty());
    }
    replay.state.lock().unwrap().uncertain = true;
    let before = replay.state.lock().unwrap().requests.len();
    document_lifecycle::prepare_owned(
        &replay.endpoint,
        "synthetic",
        "ENG",
        "team",
        Scope::ci("ENG").unwrap(),
        Mode::Initialize,
        &[Slot::Filed, Slot::Loose, Slot::FollowUp],
        &document_lifecycle::Created::default(),
    )
    .await
    .err()
    .unwrap();
    let sent = replay_mutations(&replay, before);
    assert_eq!(
        sent.get("projectDelete"),
        None,
        "standing CI slots are never retired"
    );
    assert_eq!(
        sent.get("documentDelete"),
        None,
        "standing CI slots are never retired"
    );
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}

#[tokio::test]
async fn local_document_sets_cannot_resolve_ci_or_another_invocation() {
    let replay = replay();
    let ci = replay_prepare(&replay, Scope::ci("ENG").unwrap(), Mode::Initialize)
        .await
        .unwrap();
    let first = Scope::local("ENG", "1-2-3").unwrap();
    let second = Scope::local("ENG", "1-4-5").unwrap();
    for scope in [first, second] {
        assert!(
            replay_prepare(&replay, scope, Mode::Ordinary)
                .await
                .err()
                .unwrap()
                .contains("initialize")
        );
    }
    let local = replay_prepare(&replay, first, Mode::Initialize)
        .await
        .unwrap();
    let other = replay_prepare(&replay, second, Mode::Initialize)
        .await
        .unwrap();
    for key in ["filed", "loose", "follow-up", "parent"] {
        assert_ne!(ci.ids[key], local.ids[key]);
        assert_ne!(local.ids[key], other.ids[key]);
    }
    assert_eq!(
        replay.journal.count(),
        9,
        "each independently owned set has three real document creates"
    );
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}
#[tokio::test]
async fn cancelled_document_initialization_discovers_its_stored_create_before_recovery() {
    use std::{sync::atomic::Ordering, time::Duration};
    let replay = replay();
    replay.state.lock().unwrap().pause_next_create = true;
    let endpoint = replay.endpoint.clone();
    let initializing = tokio::spawn(async move {
        document_lifecycle::prepare(
            &endpoint,
            "synthetic",
            "ENG",
            "team",
            Scope::ci("ENG").unwrap(),
            Mode::Initialize,
            &[Slot::Filed, Slot::Loose, Slot::FollowUp],
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !replay.paused.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("create reached HTTP transport before cancellation");
    initializing.abort();
    assert!(matches!(initializing.await, Err(error) if error.is_cancelled()));
    replay.release.store(true, Ordering::Release);
    tokio::time::timeout(Duration::from_secs(5), async {
        while replay.journal.count() != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("forwarding observer records the upstream acknowledgement after cancellation");
    assert_eq!(
        replay.journal.count(),
        1,
        "a committed create remains counted after the caller is cancelled"
    );
    let scope = Scope::ci("ENG").unwrap();
    let restored = replay_prepare(&replay, scope, Mode::Ordinary)
        .await
        .unwrap();
    assert_eq!(
        replay.journal.count(),
        3,
        "recovery discovers the cancelled create and creates only missing identities"
    );
    replay_drive(&replay, restored, Mode::Ordinary, now_micros())
        .await
        .unwrap();
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}

/// A child role invoked only by the subprocess seat journey below.
#[tokio::test]
async fn document_writer_child() {
    use std::time::Duration;
    let Ok(role) = env::var("ONETASKGRAPH_LINEAR_WRITER_CHILD") else {
        return;
    };
    let root = std::path::PathBuf::from(env::var_os("ONETASKGRAPH_LINEAR_WRITER_ROOT").unwrap());
    let session = Session::open(
        SESSION_NAME,
        Credential::new("synthetic").unwrap(),
        Exclusivity::OneAtATime,
    );
    if role == "decline" {
        assert!(
            session.is_err(),
            "a concurrent writer unexpectedly acquired the Linear seat"
        );
        println!("declined before HTTP");
        return;
    }
    let _session = session
        .unwrap_or_else(|error| panic!("isolated writer could not acquire its seat: {error}"));
    std::fs::write(root.join("held"), b"held").unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while !root.join("release").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("parent releases the writer it started");
    let endpoint = env::var("ONETASKGRAPH_LINEAR_WRITER_ENDPOINT").unwrap();
    let standing = document_lifecycle::prepare(
        &endpoint,
        "synthetic",
        "ENG",
        "team",
        Scope::ci("ENG").unwrap(),
        Mode::Ordinary,
        &[Slot::Filed, Slot::Loose, Slot::FollowUp],
    )
    .await
    .unwrap();
    let journal = document_lifecycle::journal::Journal::new("plugin", "ordinary");
    replay_drive_at(&endpoint, &journal, standing, Mode::Ordinary, now_micros())
        .await
        .unwrap();
}

#[tokio::test]
async fn document_ci_writers_share_the_real_machine_seat_before_http_updates() {
    use std::{
        process::{Child, Command, Stdio},
        time::Duration,
    };
    struct HeldChild(Option<Child>);
    impl Drop for HeldChild {
        fn drop(&mut self) {
            if let Some(child) = self.0.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let replay = replay();
    let standing = replay_prepare(&replay, Scope::ci("ENG").unwrap(), Mode::Initialize)
        .await
        .unwrap();
    replay.journal.begin("ordinary");
    let root = env::var_os("ONEPIPELINE_NODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(format!(
            "linear-writers-{}-{}",
            std::process::id(),
            now_micros()
        ));
    std::fs::create_dir_all(root.join("seats")).unwrap();
    let command = |role: &str| {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", "document_writer_child", "--nocapture"])
            .env("ONETASKGRAPH_LINEAR_WRITER_CHILD", role)
            .env("ONETASKGRAPH_LINEAR_WRITER_ROOT", &root)
            .env("ONETASKGRAPH_LINEAR_WRITER_ENDPOINT", &replay.endpoint)
            .env(
                onetaskgraph_live::SEAT_DIRECTORY_VARIABLE,
                root.join("seats"),
            )
            .env(
                onetaskgraph_live::artifact::REGISTRY_DIRECTORY_VARIABLE,
                root.join("registry"),
            )
            .env_remove("LINEAR_API_KEY")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    };
    let mut first = HeldChild(Some(command("hold").spawn().unwrap()));
    tokio::time::timeout(Duration::from_secs(10), async {
        while !root.join("held").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("first child holds its isolated Linear seat");
    let before = replay.state.lock().unwrap().requests.len();
    let declined = command("decline").output().unwrap();
    assert!(
        declined.status.success(),
        "{}",
        String::from_utf8_lossy(&declined.stderr)
    );
    assert!(String::from_utf8_lossy(&declined.stdout).contains("declined before HTTP"));
    assert_eq!(
        replay.state.lock().unwrap().requests.len(),
        before,
        "declined writer sends no HTTP operation"
    );
    std::fs::write(root.join("release"), b"release").unwrap();
    let completed = first.0.take().unwrap().wait_with_output().unwrap();
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stdout)
    );
    assert_eq!(replay.journal.count(), 0);
    let requests = replay.state.lock().unwrap().requests[before..].to_vec();
    assert!(
        requests
            .iter()
            .filter(|request| request["query"]
                .as_str()
                .unwrap()
                .contains("documentUpdate("))
            .count()
            >= 3,
        "admitted child runs the actual shared ordinary document paths"
    );
    let after = replay_prepare(&replay, Scope::ci("ENG").unwrap(), Mode::Ordinary)
        .await
        .unwrap();
    assert_eq!(standing.ids, after.ids);
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

struct ReplaySecrets;
impl SecretResolver for ReplaySecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        Some(SecretString::from("synthetic"))
    }
}

#[tokio::test]
async fn document_discovery_follows_advancing_http_pages_without_writes() {
    let replay = replay();
    let scope = Scope::ci("ENG").unwrap();
    let initialized = replay_prepare(&replay, scope, Mode::Initialize)
        .await
        .unwrap();
    let before = {
        let mut state = replay.state.lock().unwrap();
        state.discovery_pagination = Some("advance");
        state.requests.len()
    };
    let discovered = replay_prepare(&replay, scope, Mode::Ordinary)
        .await
        .unwrap();
    assert_eq!(discovered.ids, initialized.ids);
    let state = replay.state.lock().unwrap();
    let requests = &state.requests[before..];
    assert!(
        requests
            .iter()
            .all(|request| !request["query"].as_str().unwrap().contains("mutation"))
    );
    let cursors: Vec<_> = requests
        .iter()
        .filter(|request| request["query"].as_str().unwrap().contains("documents("))
        .map(|request| request["variables"]["after"].clone())
        .collect();
    assert_eq!(
        cursors,
        (0..3)
            .flat_map(|_| [Value::Null, json!("1"), json!("2")])
            .collect::<Vec<_>>(),
        "each exact slot discovery reads all three actual stored document pages"
    );
    assert!(state.schema_errors.is_empty(), "{:?}", state.schema_errors);
}

#[tokio::test]
async fn document_discovery_refuses_broken_or_unbounded_http_pagination_before_writes() {
    let replay = replay();
    let scope = Scope::ci("ENG").unwrap();
    replay_prepare(&replay, scope, Mode::Initialize)
        .await
        .unwrap();
    for (fault, expected, pages) in [
        ("missing-cursor", "omitted cursor", 1),
        ("repeated-cursor", "did not advance", 2),
        ("missing-pagination", "omitted pagination", 1),
        ("exhausted", "did not terminate", 50),
    ] {
        let before = {
            let mut state = replay.state.lock().unwrap();
            state.discovery_pagination = Some(fault);
            state.requests.len()
        };
        let error = replay_prepare(&replay, scope, Mode::Ordinary)
            .await
            .err()
            .unwrap();
        assert!(error.contains(expected), "{fault}: {error}");
        let state = replay.state.lock().unwrap();
        let requests = &state.requests[before..];
        assert!(
            requests
                .iter()
                .all(|request| !request["query"].as_str().unwrap().contains("mutation")),
            "{fault} sent a write"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request["query"].as_str().unwrap().contains("documents("))
                .count(),
            pages,
            "{fault}"
        );
        assert!(
            state.schema_errors.is_empty(),
            "{fault}: {:?}",
            state.schema_errors
        );
    }
    assert_eq!(
        replay.journal.count(),
        3,
        "failed ordinary discovery never creates a replacement"
    );
}

#[tokio::test]
async fn document_discovery_requires_well_formed_direct_team_before_project_fallback() {
    let replay = replay();
    let prepare = |mode| {
        document_lifecycle::prepare(
            &replay.endpoint,
            "synthetic",
            "ENG",
            "team",
            Scope::ci("ENG").unwrap(),
            mode,
            &[Slot::Filed],
        )
    };
    let initialized = prepare(Mode::Initialize).await.unwrap();
    for malformed in [
        json!("ENG"),
        json!({}),
        json!({"key":7}),
        json!({"key":"OTHER"}),
    ] {
        let before = {
            let mut state = replay.state.lock().unwrap();
            state.direct_team_override = Some(malformed.clone());
            state.requests.len()
        };
        let error = prepare(Mode::Ordinary).await.err().unwrap();
        assert!(error.contains("membership"), "{malformed}: {error}");
        let state = replay.state.lock().unwrap();
        assert!(
            state.requests[before..]
                .iter()
                .all(|request| !request["query"].as_str().unwrap().contains("mutation")),
            "{malformed} sent a write"
        );
    }
    let before = {
        let mut state = replay.state.lock().unwrap();
        state.direct_team_override = None;
        state.omit_direct_team = true;
        state.requests.len()
    };
    assert!(
        prepare(Mode::Ordinary)
            .await
            .err()
            .unwrap()
            .contains("membership")
    );
    assert!(
        replay.state.lock().unwrap().requests[before..]
            .iter()
            .all(|request| !request["query"].as_str().unwrap().contains("mutation"))
    );
    {
        let mut state = replay.state.lock().unwrap();
        state.omit_direct_team = false;
        state.direct_team_override = Some(Value::Null);
    }
    let recovered = prepare(Mode::Ordinary).await.unwrap();
    assert_eq!(
        recovered.ids, initialized.ids,
        "explicit null uses the verified complete project membership"
    );
    assert_eq!(replay.journal.count(), 1);
}

#[tokio::test]
async fn ordinary_document_run_metadata_reset_is_read_back_and_recovers_after_stale_write() {
    let replay = replay();
    let scope = Scope::ci("ENG").unwrap();
    let first = replay_prepare(&replay, scope, Mode::Initialize)
        .await
        .unwrap();
    replay.journal.begin("ordinary");
    let stamp = now_micros();
    replay_drive(&replay, first, Mode::Ordinary, stamp)
        .await
        .unwrap();
    replay.state.lock().unwrap().freeze_document_run_metadata = true;
    let second = replay_prepare(&replay, scope, Mode::Ordinary)
        .await
        .unwrap();
    let error = replay_drive(&replay, second, Mode::Ordinary, stamp + 1)
        .await
        .unwrap_err();
    assert!(error.contains("reset caller.run"), "{error}");
    replay.state.lock().unwrap().freeze_document_run_metadata = false;
    let recovered = replay_prepare(&replay, scope, Mode::Ordinary)
        .await
        .unwrap();
    replay_drive(&replay, recovered, Mode::Ordinary, stamp + 1)
        .await
        .unwrap();
    assert_eq!(
        replay.journal.count(),
        0,
        "recovery updates the same retained documents"
    );
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}

/// The plugin lane's own write of a task naming labels it created, `write_labelled_task`, over
/// HTTP to a replay whose label filter holds one label only from its third lookup and never
/// holds the other: the first is resolved once held and written with its id, and the second
/// ends, once the bound is spent, in the refusal the plugin gives — with no label created and
/// no issue written for it.
#[tokio::test]
async fn a_task_write_naming_a_label_linear_holds_late_resolves_it_and_one_never_held_is_refused() {
    const BOUND: Bound = Bound {
        reads: 4,
        interval: std::time::Duration::from_millis(20),
    };
    let replay = replay();
    let stamp = now_micros();
    let late = artifact_label(Run::current(), stamp);
    let never = artifact_label(Run::current(), stamp + 1);
    {
        let mut state = replay.state.lock().unwrap();
        state.labels.insert(late.clone(), ("label-late".into(), 2));
        state
            .labels
            .insert(never.clone(), ("label-never".into(), u32::MAX));
    }
    let source = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("live").unwrap(),
            &json!({"endpoint":replay.endpoint,"team":"ENG","status_mapping":{"todo":{"task":"Todo"}}}),
            &ReplaySecrets,
        )
        .unwrap();
    let write = |name: &str, offset: u64| -> ItemWrite<Task> {
        let mut task: Task = serde_json::from_value(json!({
            "id":"authored:labelled","title":artifact_title(Run::current(), stamp + offset),
            "content":"Synthetic labelled task.","status":{"category":"todo","name":"Todo"},
            "labels":[],"project":null,"repositories":[],"metadata":{}
        }))
        .unwrap();
        task.labels = vec![label(name)];
        ItemWrite {
            target: None,
            item: task,
            depends_on: vec![],
        }
    };
    let creates = |replay: &document_replay::Replay| -> Vec<Value> {
        replay
            .state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|request| {
                request["query"]
                    .as_str()
                    .is_some_and(|q| q.contains("issueCreate("))
            })
            .map(|request| request["variables"]["input"]["labelIds"].clone())
            .collect()
    };

    write_labelled_task(BOUND, source.as_ref(), &write(&late, 10))
        .await
        .expect("a label the filter holds within the bound is resolved and written");
    let lookups = |name: &str| replay.state.lock().unwrap().label_lookups[&name.to_lowercase()];
    assert_eq!(
        lookups(&late),
        3,
        "the write asks again until the label is held, and not once more"
    );
    assert_eq!(
        creates(&replay),
        vec![json!(["label-late"])],
        "one issue is written, naming the label once it was resolved"
    );

    let refusal = write_labelled_task(BOUND, source.as_ref(), &write(&never, 11))
        .await
        .expect_err("a label the filter never holds is refused once the bound is spent");
    assert!(
        matches!(&refusal, SourceError::Refused { message }
            if *message == format!("source live cannot resolve label {never:?}: found 0 matches")),
        "the refusal past the bound is the plugin's own: {refusal:?}"
    );
    assert_eq!(
        lookups(&never),
        BOUND.reads,
        "the bound is every lookup made"
    );
    assert_eq!(
        creates(&replay).len(),
        1,
        "no issue is written for a label that was never resolved"
    );
    let state = replay.state.lock().unwrap();
    assert!(
        state.requests.iter().all(|request| {
            let query = request["query"].as_str().unwrap_or_default();
            !query.contains("issueLabel") || query == onetaskgraph_linear::graphql::ISSUE_LABEL
        }),
        "a label is only ever looked up, never created, renamed or removed"
    );
    assert!(state.schema_errors.is_empty(), "{:?}", state.schema_errors);
}

/// A label a live journey created and has not retired, through a cold ordinary CI pass that
/// prepares the standing set and a second pass queued after it: neither pass sends any request
/// that creates, renames or removes a label, and the plugin's lookup still resolves it after
/// both, so neither can have deleted or hidden it. (Residue clearing is driven over the same
/// label in `tests/sweep_gate.rs`.)
#[tokio::test]
async fn standing_set_preparation_and_a_queued_pass_leave_a_live_journeys_label_resolvable() {
    let replay = replay();
    let held = artifact_label(Run::current(), now_micros());
    replay
        .state
        .lock()
        .unwrap()
        .labels
        .insert(held.clone(), ("label-held".into(), 0));
    let standing_ids = std::sync::Mutex::new(None);
    replay_ordinary_ci_pass(&replay, now_micros(), &standing_ids).await;
    let cold = replay.journal.count();
    assert!(cold > 0, "the first pass prepared the standing set cold");
    replay_ordinary_ci_pass(&replay, now_micros(), &standing_ids).await;
    assert_eq!(
        replay.journal.count(),
        cold,
        "the queued pass reused the standing set"
    );
    assert!(
        replay
            .state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|request| !request["query"]
                .as_str()
                .unwrap_or_default()
                .contains("issueLabel")),
        "preparation and the passes over the standing set touched no label"
    );
    let source = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("live").unwrap(),
            &json!({"endpoint":replay.endpoint,"team":"ENG","status_mapping":{"todo":{"task":"Todo"}}}),
            &ReplaySecrets,
        )
        .unwrap();
    let mut task: Task = serde_json::from_value(json!({
        "id":"authored:labelled","title":artifact_title(Run::current(), now_micros()),
        "content":"Synthetic labelled task.","status":{"category":"todo","name":"Todo"},
        "labels":[],"project":null,"repositories":[],"metadata":{}
    }))
    .unwrap();
    task.labels = vec![label(&held)];
    write_labelled_task(
        Bound {
            reads: 1,
            interval: std::time::Duration::ZERO,
        },
        source.as_ref(),
        &ItemWrite {
            target: None,
            item: task,
            depends_on: vec![],
        },
    )
    .await
    .expect("the live journey's label still resolves on the first lookup after both passes");
    assert!(replay.state.lock().unwrap().schema_errors.is_empty());
}

mod pool {
    //! The standing pool and its lease, against the replay: every claim exclusive, every
    //! session of either lane over its own member, the team's document limit and the pool
    //! held with eight sessions at once, and the namespace guard refusing a foreign record.
    use super::*;
    use document_lifecycle::lease::{self, Held, STALE_AFTER};
    use document_lifecycle::namespace::Namespace;
    use document_lifecycle::observer;
    use onetaskgraph_linear::live_artifact::{POOL_SIZE, classify_lease};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    /// Drives every future at once, on this task, and hands back their outputs in order.
    async fn join_all<F: std::future::Future>(futures: Vec<F>) -> Vec<F::Output> {
        let mut futures: Vec<_> = futures.into_iter().map(Box::pin).collect();
        let mut outputs: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();
        std::future::poll_fn(|cx| {
            let mut pending = false;
            for (future, output) in futures.iter_mut().zip(outputs.iter_mut()) {
                if output.is_none() {
                    match future.as_mut().poll(cx) {
                        std::task::Poll::Ready(value) => *output = Some(value),
                        std::task::Poll::Pending => pending = true,
                    }
                }
            }
            if pending {
                std::task::Poll::Pending
            } else {
                std::task::Poll::Ready(())
            }
        })
        .await;
        outputs.into_iter().map(Option::unwrap).collect()
    }

    fn run(process: u32) -> Run {
        Run::read(&format!("9-{process}")).unwrap()
    }
    fn holder(owner: Run) -> String {
        onetaskgraph_live::artifact::Stamp::new(owner, now_micros()).to_string()
    }
    /// The member a holder's claim tries first, as `lease::claim` derives it.
    fn first_member(holder: &str) -> u8 {
        lease::first_member(holder)
    }

    #[test]
    fn lease_ids_and_first_members_are_the_ones_the_standing_pool_was_filled_under() {
        // The pool's standing issues exist under these ids: a change to the derivation would
        // strand every member it filled, so the derivation is pinned by its outputs.
        assert_eq!(
            lease::lease_id("TES2", LeaseLane::Plugin, 1, 1),
            "f064e9d8-01f5-4f77-b19e-d25c82a04462"
        );
        assert_eq!(
            lease::lease_id("TES2", LeaseLane::Assets, 16, 42),
            "6cc370b1-021c-4280-aa3f-e18a5568aabe"
        );
        assert_eq!(first_member("ci-41229550836-1-1793471795575259"), 1);
    }
    async fn claim(
        replay: &document_replay::Replay,
        lane: LeaseLane,
        holder: &str,
    ) -> Result<Held, String> {
        lease::claim(
            &replay.endpoint,
            "synthetic",
            "ENG",
            "team",
            lane,
            holder,
            now_micros,
        )
        .await
    }
    fn leases(replay: &document_replay::Replay) -> Vec<(String, String)> {
        replay
            .state
            .lock()
            .unwrap()
            .issues
            .iter()
            .filter(|(_, row)| {
                row["title"]
                    .as_str()
                    .is_some_and(|t| classify_lease(t).is_some())
            })
            .map(|(id, row)| (id.clone(), row["title"].as_str().unwrap().to_owned()))
            .collect()
    }

    #[tokio::test]
    async fn two_claimants_starting_at_one_member_each_hold_a_different_one() {
        let replay = replay();
        // Two holders whose claims start at the same member, so the create race is real.
        let first = holder(run(1));
        let second = (2..)
            .map(|process| holder(run(process)))
            .find(|candidate| first_member(candidate) == first_member(&first))
            .unwrap();
        let (a, b) = tokio::join!(
            claim(&replay, LeaseLane::Plugin, &first),
            claim(&replay, LeaseLane::Plugin, &second)
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a.index, b.index, "two live claims share one pool member");
        assert_eq!(leases(&replay).len(), 2, "one lease issue per claim");
        // The loser's create of the winner's id was refused by the double, as Linear refuses it.
        let creates = replay
            .state
            .lock()
            .unwrap()
            .keyed
            .iter()
            .filter(|(_, request)| request["query"].as_str().unwrap().contains("issueCreate("))
            .count();
        assert!(
            creates >= 3,
            "the losing claimant tried the contended id first: {creates}"
        );
    }

    #[tokio::test]
    async fn a_released_or_ended_runs_lease_is_reclaimed_and_a_live_one_never_is() {
        let replay = replay();
        let owner = holder(run(1));
        let held = claim(&replay, LeaseLane::Plugin, &owner).await.unwrap();
        // Every other member taken by live runs: the next claim is refused, not queued.
        let mut others = Vec::new();
        for process in 2..=u32::from(POOL_SIZE) {
            others.push(
                claim(&replay, LeaseLane::Plugin, &holder(run(process)))
                    .await
                    .unwrap(),
            );
        }
        let refused = claim(&replay, LeaseLane::Plugin, &holder(run(100)))
            .await
            .unwrap_err();
        assert!(refused.contains("still going"), "{refused}");
        // The asset lane's pool is its own: a full plugin pool takes nothing from it.
        claim(&replay, LeaseLane::Assets, &holder(run(101)))
            .await
            .unwrap()
            .release(&replay.endpoint, "synthetic")
            .await
            .unwrap();
        // Released: the next claim takes that member under the next generation, and the
        // superseded generation is gone.
        held.release(&replay.endpoint, "synthetic").await.unwrap();
        let next = claim(&replay, LeaseLane::Plugin, &holder(run(102)))
            .await
            .unwrap();
        assert_eq!(
            (next.index, next.generation),
            (held.index, held.generation + 1)
        );
        assert!(replay.state.lock().unwrap().trashed.contains(&held.id));
        // A lease older than any hosted job can run belongs to an ended run; one a moment
        // short of that is still somebody's.
        let ended = &others[0];
        let old = |age: std::time::Duration| {
            chrono::DateTime::from_timestamp_micros(
                i64::try_from(now_micros() - u64::try_from(age.as_micros()).unwrap()).unwrap(),
            )
            .unwrap()
            .to_rfc3339()
        };
        replay
            .state
            .lock()
            .unwrap()
            .issues
            .get_mut(&ended.id)
            .unwrap()["createdAt"] = json!(old(STALE_AFTER - std::time::Duration::from_secs(60)));
        assert!(
            claim(&replay, LeaseLane::Plugin, &holder(run(103)))
                .await
                .is_err()
        );
        replay
            .state
            .lock()
            .unwrap()
            .issues
            .get_mut(&ended.id)
            .unwrap()["createdAt"] = json!(old(STALE_AFTER + std::time::Duration::from_secs(60)));
        let reclaimed = claim(&replay, LeaseLane::Plugin, &holder(run(104)))
            .await
            .unwrap();
        assert_eq!(
            (reclaimed.index, reclaimed.generation),
            (ended.index, ended.generation + 1)
        );
    }

    /// What one CI session of a lane did, as the double saw it.
    struct Session {
        key: String,
        lease: Held,
        standing: BTreeMap<&'static str, String>,
        namespace: Arc<Namespace>,
        result: Result<(), String>,
    }

    /// One whole ordinary CI session of `lane` as the live lane runs it: lease a member,
    /// prepare its slots through the guarded observer, drive the lane's document paths over
    /// them, release. Every request carries `key`, so the double can tell sessions apart.
    async fn ci_session(
        replay: &document_replay::Replay,
        lane: LeaseLane,
        key: &str,
        owner: Run,
    ) -> Session {
        let journal = document_lifecycle::journal::Journal::new("plugin", "ordinary");
        let namespace = Arc::new(Namespace::new(owner, "ENG"));
        let raw = observer::start(&replay.endpoint, key, journal.clone());
        let endpoint = observer::guarded(
            &replay.endpoint,
            key,
            journal.clone(),
            Arc::clone(&namespace),
        );
        let stamp = now_micros();
        let lease = lease::claim(&raw, key, "ENG", "team", lane, &holder(owner), now_micros)
            .await
            .unwrap();
        namespace.leased(lease.index);
        let slots: &[Slot] = match lane {
            LeaseLane::Plugin => &[Slot::Filed, Slot::Loose, Slot::FollowUp],
            LeaseLane::Assets => &[Slot::Assets],
        };
        let created = document_lifecycle::Created::default();
        let result = async {
            let standing = document_lifecycle::prepare_owned(
                &endpoint, key, "ENG", "team", lease.scope(), Mode::Ordinary, slots, &created,
            )
            .await?;
            let ids = standing.ids.clone();
            match lane {
                LeaseLane::Plugin => {
                    replay_drive_as(&endpoint, &journal, standing, Mode::Ordinary, stamp, owner)
                        .await?;
                }
                LeaseLane::Assets => {
                    // The asset lane resets its destination to this run's own content first.
                    let id = &ids["assets"];
                    document_lifecycle::send(
                        &endpoint,
                        key,
                        "mutation($id:String!,$input:DocumentUpdateInput!){documentUpdate(id:$id,input:$input){success document{id}}}",
                        json!({"id":id,"input":{"content":format!("asset destination of {key}")}}),
                    )
                    .await?;
                }
            }
            Ok::<_, String>(ids)
        }
        .await;
        lease.release(&raw, key).await.unwrap();
        let (standing, result) = match result {
            Ok(ids) => (ids, Ok(())),
            Err(error) => (BTreeMap::new(), Err(error)),
        };
        Session {
            key: key.to_owned(),
            lease,
            standing,
            namespace,
            result,
        }
    }

    /// Every record id a request names, by the guard's own reading of a request.
    fn named(request: &Value) -> BTreeSet<String> {
        let mut found = Vec::new();
        document_lifecycle::namespace::ids_named(&request["variables"], &mut found);
        found.into_iter().collect()
    }

    fn assert_isolated(replay: &document_replay::Replay, sessions: &[Session]) {
        let keyed = replay.state.lock().unwrap().keyed.clone();
        for session in sessions {
            assert_eq!(session.result, Ok(()), "{} failed", session.key);
            assert_eq!(session.namespace.refusals(), Vec::<String>::new());
            for other in sessions.iter().filter(|other| other.key != session.key) {
                let theirs: BTreeSet<String> = other
                    .standing
                    .values()
                    .cloned()
                    .chain([other.lease.id.clone()])
                    .collect();
                for (_, request) in keyed.iter().filter(|(key, _)| *key == session.key) {
                    let overlap: Vec<_> = named(request).intersection(&theirs).cloned().collect();
                    assert!(
                        overlap.is_empty(),
                        "{} named {}'s records {overlap:?} in {}",
                        session.key,
                        other.key,
                        request["query"]
                    );
                }
            }
        }
        let members: BTreeSet<_> = sessions
            .iter()
            .map(|s| (s.lease.lane == LeaseLane::Plugin, s.lease.index))
            .collect();
        assert_eq!(
            members.len(),
            sessions.len(),
            "two sessions of one lane shared a member"
        );
    }

    #[tokio::test]
    async fn two_concurrent_sessions_of_each_linear_lane_pass_and_touch_only_their_own_records() {
        let replay = replay();
        let sessions = {
            let (a, b, c, d) = tokio::join!(
                ci_session(&replay, LeaseLane::Plugin, "plugin-a", run(1)),
                ci_session(&replay, LeaseLane::Plugin, "plugin-b", run(2)),
                ci_session(&replay, LeaseLane::Assets, "assets-a", run(3)),
                ci_session(&replay, LeaseLane::Assets, "assets-b", run(4)),
            );
            vec![a, b, c, d]
        };
        assert_isolated(&replay, &sessions);
        // Each session's destination holds what that session wrote, and no other's.
        {
            let state = replay.state.lock().unwrap();
            for session in sessions
                .iter()
                .filter(|s| s.lease.lane == LeaseLane::Assets)
            {
                assert_eq!(
                    state.documents[&session.standing["assets"]]["content"],
                    json!(format!("asset destination of {}", session.key))
                );
            }
        }
        // Released on the way out: the next claim of every member may go ahead.
        assert!(
            leases(&replay)
                .iter()
                .all(|(_, title)| title.ends_with(" released"))
        );
    }

    #[tokio::test]
    async fn eight_concurrent_ci_sessions_stay_inside_the_team_document_limit_and_the_pool() {
        let replay = replay();
        // A team with room for exactly the whole pool, cold, and not one document more:
        // 1000 documents, trash included, as Linear counts them. Each plugin member is three
        // documents and each asset member one, so the pool's documents are bounded by its
        // size and not by how many runs ever use it.
        // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] Linear publishes this limit nowhere machine-readable; it is the refusal the required asset journey met in run 40741930571 (#3811), and a changed limit is met by the credentialed lanes, whose document creates Linear itself refuses past it.
        const LIMIT: usize = 1000;
        let pool = usize::from(POOL_SIZE) * (3 + 1);
        let initial = LIMIT - pool;
        {
            let mut state = replay.state.lock().unwrap();
            state.document_limit = Some(LIMIT);
            state.documents_ever = initial;
            // And the key's hourly request quota, which every run shares.
            // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] Linear's documented hourly request quota for one API key, the figure `crates/onetaskgraph-linear-e2e/budgets.yaml` already shares against; Linear publishes it in prose alone, and the plugin backs off on the `RATELIMITED` refusal it answers past it with.
            state.request_limit = Some(2_500);
            // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] Linear's documented hourly complexity quota for one API key, the figure `crates/onetaskgraph-linear-e2e/budgets.yaml` already shares against; Linear publishes it in prose alone, scores each document at its own runtime (`complexity.rs` reconciles the model against Linear's reported scores), and refuses past it with `RATELIMITED`.
            state.complexity_limit = Some(3_000_000.0);
        }
        // Rounds of eight runs at once, each run both lanes, until every member is warm:
        // claims start at a member their holder derives, so rounds spread over the pool.
        for round in 0..4u32 {
            let handles: Vec<_> = (0..8u32)
                .flat_map(|n| {
                    [
                        (
                            LeaseLane::Plugin,
                            format!("plugin-{round}-{n}"),
                            run(100 * round + n + 1),
                        ),
                        (
                            LeaseLane::Assets,
                            format!("assets-{round}-{n}"),
                            run(100 * round + n + 50),
                        ),
                    ]
                })
                .collect();
            let sessions = join_all(
                handles
                    .iter()
                    .map(|(lane, key, owner)| ci_session(&replay, *lane, key, *owner))
                    .collect(),
            )
            .await;
            // None refused — for the limit, for the pool, or for anything else.
            assert_isolated(&replay, &sessions);
        }
        // Not one request of the four rounds was refused for the key's hourly request or
        // complexity quota.
        // llmlint: ignore[tests_hold_no_nonfunctional_thresholds] This is the functional outcome the change owes, not a cost ceiling: the stand-in refuses past each limit exactly as the service does, and the task's acceptance criterion is that eight runs at once are refused for none of them; the per-session figures themselves stay onebudgetspec's.
        assert_eq!(replay.state.lock().unwrap().rate_limited, 0);
        // Every document the four rounds created is a pool member's, made once.
        let created = replay.state.lock().unwrap().documents_ever - initial;
        assert!(created <= pool, "{created} documents for a pool of {pool}");
        let documents = replay.state.lock().unwrap().documents.len();
        assert_eq!(
            documents, created,
            "every created document is a standing member's"
        );
    }

    #[tokio::test]
    async fn pool_setup_dry_runs_without_writing_then_initializes_once_skipping_a_held_member() {
        use document_lifecycle::{PoolSetup, setup_pool};
        let replay = replay();
        let slots = [Slot::Filed, Slot::Loose, Slot::FollowUp];
        let setup = |mode| {
            setup_pool(
                &replay.endpoint,
                "synthetic",
                "ENG",
                "team",
                LeaseLane::Plugin,
                &slots,
                "1-1-1",
                mode,
            )
        };
        let mutations = || {
            replay
                .state
                .lock()
                .unwrap()
                .keyed
                .iter()
                .filter(|(_, request)| request["query"].as_str().unwrap().contains("mutation"))
                .count()
        };
        let planned = setup(PoolSetup::DryRun).await.unwrap();
        assert_eq!(planned.len(), usize::from(POOL_SIZE) * 4);
        assert!(
            planned.iter().all(|line| line.starts_with("would create ")),
            "{planned:?}"
        );
        assert_eq!(mutations(), 0, "a dry run sent a mutation");
        // A live run holding one member keeps it: initialization skips it.
        let busy = claim(&replay, LeaseLane::Plugin, &holder(run(7)))
            .await
            .unwrap();
        let done = setup(PoolSetup::Initialize).await.unwrap();
        assert!(done.iter().any(|line| line
            == &format!(
                "pool member {} is leased by a run still going; skipped",
                busy.index
            )));
        let created = replay.state.lock().unwrap().documents_ever;
        assert_eq!(created, (usize::from(POOL_SIZE) - 1) * 3);
        busy.release(&replay.endpoint, "synthetic").await.unwrap();
        setup(PoolSetup::Initialize).await.unwrap();
        assert_eq!(
            replay.state.lock().unwrap().documents_ever,
            usize::from(POOL_SIZE) * 3
        );
        // Idempotent: a second pass creates nothing, and a dry run then reports every slot present.
        setup(PoolSetup::Initialize).await.unwrap();
        assert_eq!(
            replay.state.lock().unwrap().documents_ever,
            usize::from(POOL_SIZE) * 3
        );
        let present = setup(PoolSetup::DryRun).await.unwrap();
        assert!(
            present.iter().all(|line| line.starts_with("present ")),
            "{present:?}"
        );
    }

    #[tokio::test]
    async fn a_claim_whose_create_acknowledgement_was_lost_holds_the_member_it_created() {
        let replay = replay();
        replay.state.lock().unwrap().lose_next_issue_create_ack = true;
        let held = claim(&replay, LeaseLane::Plugin, &holder(run(1)))
            .await
            .unwrap();
        // The create landed and its answer did not: the read back is what decided, and no
        // second create was sent for the member.
        {
            let state = replay.state.lock().unwrap();
            let creates = state
                .keyed
                .iter()
                .filter(|(_, request)| request["query"].as_str().unwrap().contains("issueCreate("))
                .count();
            assert_eq!(creates, 1);
            assert!(state.issues.contains_key(&held.id));
        }
        // And it is held: the next claimant passes over it.
        let next = claim(&replay, LeaseLane::Plugin, &holder(run(2)))
            .await
            .unwrap();
        assert_ne!(next.index, held.index);
    }

    #[tokio::test]
    async fn a_lane_that_panics_releases_its_lease_before_the_panic_leaves() {
        let replay = replay();
        let held = claim(&replay, LeaseLane::Plugin, &holder(run(1)))
            .await
            .unwrap();
        let released = |id: &str| {
            replay.state.lock().unwrap().issues[id]["title"]
                .as_str()
                .unwrap()
                .ends_with(" released")
        };
        // The plugin lane's wrapper, around a journey that panics.
        let panicked = document_lifecycle::unwinding(document_lifecycle::lease::holding(
            Some(&held),
            &replay.endpoint,
            "synthetic",
            async { panic!("injected journey failure") },
        ))
        .await;
        assert!(panicked.is_err(), "the panic was swallowed");
        assert!(released(&held.id), "a panicking plugin lane kept its lease");
        // The asset lane's, which drives its own runtime, around a body that panics.
        let assets = claim(&replay, LeaseLane::Assets, &holder(run(2)))
            .await
            .unwrap();
        let endpoint = replay.endpoint.clone();
        let id = assets.id.clone();
        tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                document_lifecycle::lease::holding_blocking(
                    &runtime,
                    Some(&assets),
                    &endpoint,
                    "synthetic",
                    || panic!("injected asset journey failure"),
                )
            }));
            assert!(panicked.is_err(), "the panic was swallowed");
        })
        .await
        .unwrap();
        assert!(released(&id), "a panicking asset lane kept its lease");
    }

    #[test]
    fn the_pool_setup_reads_exactly_its_two_actions() {
        use document_lifecycle::PoolSetup;
        assert_eq!(PoolSetup::parse("dry-run"), Ok(PoolSetup::DryRun));
        assert_eq!(PoolSetup::parse("initialize"), Ok(PoolSetup::Initialize));
        for refused in ["", "fill", "Initialize", "dry_run"] {
            assert!(PoolSetup::parse(refused).is_err(), "{refused:?}");
        }
        // The recipe's spelling of both is `.github/linear-documents-policy`'s to hold.
    }

    #[tokio::test]
    async fn a_superseded_lease_linear_will_not_delete_leaves_the_claim_held_and_is_taken_later() {
        let replay = replay();
        let first = claim(&replay, LeaseLane::Plugin, &holder(run(1)))
            .await
            .unwrap();
        first.release(&replay.endpoint, "synthetic").await.unwrap();
        // The next claim of that member supersedes the released generation; its delete is
        // refused, and the claim stands all the same.
        replay.state.lock().unwrap().refuse_issue_deletes = true;
        let second = lease::claim_member(
            &replay.endpoint,
            "synthetic",
            "ENG",
            "team",
            LeaseLane::Plugin,
            &holder(run(2)),
            now_micros,
            first.index,
        )
        .await
        .unwrap()
        .expect("the released member is free");
        assert_eq!(second.generation, first.generation + 1);
        assert!(
            replay.state.lock().unwrap().issues.contains_key(&first.id),
            "the leftover stays"
        );
        // The leftover confers nothing: the member is held by the newer generation.
        assert!(
            lease::claim_member(
                &replay.endpoint,
                "synthetic",
                "ENG",
                "team",
                LeaseLane::Plugin,
                &holder(run(3)),
                now_micros,
                first.index,
            )
            .await
            .unwrap()
            .is_none()
        );
        // And the claim after the next release takes both older generations out of the listing.
        replay.state.lock().unwrap().refuse_issue_deletes = false;
        second.release(&replay.endpoint, "synthetic").await.unwrap();
        let third = lease::claim_member(
            &replay.endpoint,
            "synthetic",
            "ENG",
            "team",
            LeaseLane::Plugin,
            &holder(run(4)),
            now_micros,
            first.index,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(third.generation, second.generation + 1);
        let state = replay.state.lock().unwrap();
        assert!(!state.issues.contains_key(&first.id) && !state.issues.contains_key(&second.id));
    }

    #[tokio::test]
    async fn a_release_linear_refuses_never_hides_the_lanes_own_outcome() {
        let replay = replay();
        let held = claim(&replay, LeaseLane::Plugin, &holder(run(1)))
            .await
            .unwrap();
        replay.state.lock().unwrap().refuse_issue_updates = true;
        // A body that finished answers with its own result, the refused release aside...
        let answered =
            document_lifecycle::lease::holding(Some(&held), &replay.endpoint, "synthetic", async {
                7
            })
            .await;
        assert_eq!(answered, 7);
        // ...and one that panicked leaves with its own panic, not the release's refusal.
        let panicked = document_lifecycle::unwinding(document_lifecycle::lease::holding(
            Some(&held),
            &replay.endpoint,
            "synthetic",
            async { panic!("the journey's own failure") },
        ))
        .await
        .unwrap_err();
        assert_eq!(
            panicked.downcast_ref::<&str>(),
            Some(&"the journey's own failure")
        );
        // The lease was not released, so it stays held until it is stale: never another's.
        let title = replay.state.lock().unwrap().issues[&held.id]["title"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!title.ends_with(" released"), "{title}");
    }

    #[tokio::test]
    async fn a_lease_listing_that_does_not_read_soundly_refuses_the_claim_creating_nothing() {
        for (fault, said) in [
            ("no-page-info", "the lease listing answered no pagination"),
            (
                "repeated-cursor",
                "the lease listing cursor did not advance",
            ),
            ("unreadable-created-at", "unreadable createdAt"),
            ("untitled", "a lease issue answered no title"),
        ] {
            let replay = replay();
            // One lease already there, so a listing holds a row whose time is read.
            claim(&replay, LeaseLane::Plugin, &holder(run(1)))
                .await
                .unwrap();
            let before = replay.state.lock().unwrap().issues.len();
            replay.state.lock().unwrap().lease_listing_fault = Some(fault);
            let refused = claim(&replay, LeaseLane::Plugin, &holder(run(2)))
                .await
                .unwrap_err();
            assert!(refused.contains(said), "{fault}: {refused}");
            assert_eq!(
                replay.state.lock().unwrap().issues.len(),
                before,
                "{fault}: it created"
            );
        }
    }

    #[tokio::test]
    async fn an_issue_titled_as_a_lease_under_another_id_confers_no_lease() {
        let replay = replay();
        // Someone's issue carrying a held lease's exact title, but not the id that lease's
        // generation is created under: it holds nothing, so the member is free.
        let stamp = holder(run(9));
        let forged = onetaskgraph_linear::live_artifact::lease_title(
            onetaskgraph_linear::live_artifact::Lease::new(
                Scope::pooled("ENG", 1).unwrap(),
                LeaseLane::Plugin,
                std::num::NonZeroU32::MIN,
                onetaskgraph_linear::live_artifact::LeaseHolder::Held(
                    onetaskgraph_linear::live_artifact::HolderStamp::new(&stamp).unwrap(),
                ),
            )
            .unwrap(),
        );
        replay.state.lock().unwrap().issues.insert(
            "forged".into(),
            json!({"id":"forged","title":forged,"createdAt":chrono::Utc::now().to_rfc3339()}),
        );
        let held = lease::claim_member(
            &replay.endpoint,
            "synthetic",
            "ENG",
            "team",
            LeaseLane::Plugin,
            &holder(run(1)),
            now_micros,
            1,
        )
        .await
        .unwrap()
        .expect("a forged lease holds nothing");
        assert_eq!(held.generation, 1);
        // And it is no claimant's to delete either.
        assert!(replay.state.lock().unwrap().issues.contains_key("forged"));
    }

    #[tokio::test]
    async fn the_pool_variable_turns_the_lanes_entry_point_into_the_setup_and_skips_the_journey() {
        let replay = replay();
        let seats = std::env::temp_dir().join(format!("linear-pool-entry-{}", std::process::id()));
        std::fs::create_dir_all(&seats).unwrap();
        let output = tokio::task::spawn_blocking({
            let endpoint = replay.endpoint.clone();
            let seats = seats.clone();
            move || {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "real_linear_applies_every_declared_capability_and_leaves_no_residue",
                        "--exact",
                        "--nocapture",
                    ])
                    .env(document_lifecycle::POOL_VARIABLE, "dry-run")
                    .env("LINEAR_LIVE_TEST_GRAPHQL", endpoint)
                    .env("LINEAR_API_KEY", "synthetic")
                    .env("LINEAR_WRITE_TEAM", "ENG")
                    .env(onetaskgraph_live::SEAT_DIRECTORY_VARIABLE, &seats)
                    .env_remove("GITHUB_ACTIONS")
                    .env_remove(onetaskgraph_live::REQUIRED_VARIABLE)
                    .output()
                    .unwrap()
            }
        })
        .await
        .unwrap();
        let _ = std::fs::remove_dir_all(&seats);
        let said = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{said}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let planned = said
            .lines()
            .filter(|line| line.contains("would create "))
            .count();
        assert_eq!(planned, usize::from(POOL_SIZE) * 4, "{said}");
        // A dry run, and the setup in place of the journey: nothing written, no fixture read.
        let state = replay.state.lock().unwrap();
        let sent: Vec<_> = state
            .keyed
            .iter()
            .map(|(_, request)| request["query"].as_str().unwrap().to_owned())
            .collect();
        assert!(
            !sent.iter().any(|query| query.contains("mutation")),
            "{sent:?}"
        );
        assert!(
            !sent.iter().any(|query| query.contains("projectStatuses")),
            "{sent:?}"
        );
    }

    #[tokio::test]
    async fn a_claim_proves_a_second_create_under_its_id_is_refused_and_refuses_to_run_if_not() {
        let replay = replay();
        let held = claim(&replay, LeaseLane::Plugin, &holder(run(1)))
            .await
            .unwrap();
        lease::verify_exclusive(&replay.endpoint, "synthetic", "team", &held)
            .await
            .unwrap();
        // The lease still names its holder: the refused probe changed nothing.
        let title = replay.state.lock().unwrap().issues[&held.id]["title"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!title.ends_with("duplicate-probe"), "{title}");
        // A Linear that took the second create is one no lease can rest on.
        replay.state.lock().unwrap().accept_duplicate_ids = true;
        let refused = lease::verify_exclusive(&replay.endpoint, "synthetic", "team", &held)
            .await
            .unwrap_err();
        assert!(refused.contains("not exclusive"), "{refused}");
        // A probe refused for anything but its id — here, a rate limit — proves nothing.
        replay.state.lock().unwrap().accept_duplicate_ids = false;
        replay.state.lock().unwrap().request_limit = Some(0);
        let unproven = lease::verify_exclusive(&replay.endpoint, "synthetic", "team", &held)
            .await
            .unwrap_err();
        assert!(
            unproven.contains("not answered with Linear's duplicate-id refusal"),
            "{unproven}"
        );
        replay.state.lock().unwrap().request_limit = None;
        // So does a refusal worded otherwise than Linear's recorded one, whatever it says.
        replay
            .state
            .lock()
            .unwrap()
            .duplicate_refusal_worded_otherwise = true;
        let unproven = lease::verify_exclusive(&replay.endpoint, "synthetic", "team", &held)
            .await
            .unwrap_err();
        assert!(
            unproven.contains("not answered with Linear's duplicate-id refusal"),
            "{unproven}"
        );
        // A lease that reads back as another holder's, or not at all, is not this run's to use.
        let replay = self::replay();
        let held = claim(&replay, LeaseLane::Plugin, &holder(run(2)))
            .await
            .unwrap();
        // The same lease, held under another run's stamp.
        let (generation, _) = held.title.rsplit_once(' ').unwrap();
        let other = format!("{generation} {}", holder(run(3)));
        replay
            .state
            .lock()
            .unwrap()
            .issues
            .get_mut(&held.id)
            .unwrap()["title"] = json!(other);
        let refused = lease::verify_exclusive(&replay.endpoint, "synthetic", "team", &held)
            .await
            .unwrap_err();
        assert!(
            refused.contains("no longer reads as this run's"),
            "{refused}"
        );
        replay.state.lock().unwrap().issues.remove(&held.id);
        assert!(
            lease::verify_exclusive(&replay.endpoint, "synthetic", "team", &held)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_pool_setup_that_fails_preparing_a_member_releases_its_lease_first() {
        let replay = replay();
        // A team with no room for a document: the first member's preparation is refused.
        {
            let mut state = replay.state.lock().unwrap();
            state.document_limit = Some(0);
        }
        let failed = document_lifecycle::setup_pool(
            &replay.endpoint,
            "synthetic",
            "ENG",
            "team",
            LeaseLane::Plugin,
            &[Slot::Filed, Slot::Loose, Slot::FollowUp],
            "1-1-1",
            document_lifecycle::PoolSetup::Initialize,
        )
        .await;
        assert!(failed.is_err(), "{failed:?}");
        let leases = leases(&replay);
        assert_eq!(leases.len(), 1, "{leases:?}");
        assert!(leases[0].1.ends_with(" released"), "{leases:?}");
    }

    /// The child: claims a member against the replay its parent names and ends, as a
    /// cancelled job's lane does, before it releases anything.
    #[tokio::test]
    async fn lease_record_child() {
        let Ok(endpoint) = std::env::var("LEASE_RECORD_CHILD_ENDPOINT") else {
            return;
        };
        lease::claim(
            &endpoint,
            "synthetic",
            "ENG",
            "team",
            LeaseLane::Plugin,
            &holder(run(1)),
            now_micros,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_claim_records_its_lease_where_the_cancellation_step_releases_it() {
        let replay = replay();
        let records = std::env::temp_dir().join(format!("lease-records-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&records);
        let output = tokio::task::spawn_blocking({
            let endpoint = replay.endpoint.clone();
            let records = records.clone();
            move || {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["pool::lease_record_child", "--exact"])
                    .env("LEASE_RECORD_CHILD_ENDPOINT", endpoint)
                    .env(lease::RECORD_VARIABLE, &records)
                    .output()
                    .unwrap()
            }
        })
        .await
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // One record, naming the lease issue exactly as Linear holds it.
        let written: Vec<_> = std::fs::read_dir(&records)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(written.len(), 1, "{written:?}");
        let record: Value = serde_json::from_slice(&std::fs::read(&written[0]).unwrap()).unwrap();
        let id = record["id"].as_str().unwrap();
        assert_eq!(
            replay.state.lock().unwrap().issues[id]["title"],
            record["title"],
            "the record does not name the lease as Linear holds it"
        );
        let _ = std::fs::remove_dir_all(&records);
    }

    #[tokio::test]
    async fn a_claim_whose_record_cannot_be_written_releases_its_lease_and_refuses() {
        let replay = replay();
        // A file where the record directory should be: nothing can be written under it.
        let blocked =
            std::env::temp_dir().join(format!("lease-records-blocked-{}", std::process::id()));
        std::fs::write(&blocked, "not a directory").unwrap();
        let output = tokio::task::spawn_blocking({
            let endpoint = replay.endpoint.clone();
            let blocked = blocked.join("leases");
            move || {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["pool::lease_record_child", "--exact"])
                    .env("LEASE_RECORD_CHILD_ENDPOINT", endpoint)
                    .env(lease::RECORD_VARIABLE, &blocked)
                    .output()
                    .unwrap()
            }
        })
        .await
        .unwrap();
        let _ = std::fs::remove_file(&blocked);
        let said = String::from_utf8_lossy(&output.stdout);
        assert!(
            !output.status.success(),
            "a lease nothing could release was taken"
        );
        assert!(said.contains("could not be recorded"), "{said}");
        // The member it claimed was handed straight back.
        let leases = leases(&replay);
        assert_eq!(leases.len(), 1, "{leases:?}");
        assert!(leases[0].1.ends_with(" released"), "{leases:?}");
    }

    #[tokio::test]
    async fn the_namespace_guard_refuses_a_request_naming_another_runs_record() {
        let replay = replay();
        let theirs = ci_session(&replay, LeaseLane::Assets, "theirs", run(1)).await;
        assert_eq!(theirs.result, Ok(()));
        let journal = document_lifecycle::journal::Journal::new("assets", "ordinary");
        let namespace = Arc::new(Namespace::new(run(2), "ENG"));
        let endpoint = observer::guarded(&replay.endpoint, "mine", journal, Arc::clone(&namespace));
        namespace.leased(theirs.lease.index % POOL_SIZE + 1);
        // Reading the team's standing documents teaches the guard whose each one is.
        document_lifecycle::send(
            &endpoint,
            "mine",
            document_lifecycle::DOCUMENTS,
            json!({"after":null,"filter":{"title":{"containsIgnoreCase":"retained"}}}),
        )
        .await
        .unwrap();
        let target = &theirs.standing["assets"];
        let refused = document_lifecycle::send(
            &endpoint,
            "mine",
            "mutation($id:String!,$input:DocumentUpdateInput!){documentUpdate(id:$id,input:$input){success document{id}}}",
            json!({"id":target,"input":{"content":"overwritten by another run"}}),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("rejected"), "{refused}");
        assert_eq!(namespace.refusals().len(), 1, "{:?}", namespace.refusals());
        assert!(namespace.refusals()[0].contains(target.as_str()));
        // So is naming it inside a filter's comparator rather than as a write's target.
        assert!(
            document_lifecycle::send(
                &endpoint,
                "mine",
                document_lifecycle::DOCUMENTS,
                json!({"after":null,"filter":{"id":{"in":[target]}}}),
            )
            .await
            .is_err()
        );
        assert_eq!(namespace.refusals().len(), 2, "{:?}", namespace.refusals());
        // And so is writing another run's identity into a title.
        let title =
            onetaskgraph_linear::live_artifact::retained_title(theirs.lease.scope(), Slot::Assets)
                .unwrap();
        assert!(document_lifecycle::send(
            &endpoint,
            "mine",
            "mutation($input:DocumentCreateInput!){documentCreate(input:$input){success document{id}}}",
            json!({"input":{"title":title,"content":"x","teamId":"team"}}),
        )
        .await
        .is_err());
        assert_eq!(namespace.refusals().len(), 3);
        // No refused write reached the double; their destination is as they left it.
        let state = replay.state.lock().unwrap();
        assert!(!state.keyed.iter().any(|(key, request)| key == "mine"
            && request["query"].as_str().unwrap().contains("mutation")));
        assert_eq!(
            state.documents[target]["content"],
            json!("asset destination of theirs")
        );
    }
}
