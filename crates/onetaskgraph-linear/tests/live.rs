//! Every capability this source declares, driven against Linear's real API.
//!
//! The lane builds its own fixture on the scratch team `LINEAR_WRITE_TEAM` names — two
//! projects, one issue filed under each, one issue filed under neither, two documents
//! filed the same way, two labels and two workflow states — because that shape is what
//! makes an honoured predicate and an ignored one *different answers*. A workspace holding
//! one project, or one where every issue carries the label, answers a filter the same way
//! whether or not the source applies it.
//!
//! It skips, printing why, without `LINEAR_API_KEY` or without the scratch team: this
//! lane is non-required by decision (AGENTS.md), because a required check a third party
//! can turn red is a check that stops being trusted.
//!
//! Everything it creates it deletes, whether its assertions passed or failed, and it
//! clears residue titled the way it titles its own before it starts — which is
//! self-healing after a run killed between its writes and its cleanup.

use std::{collections::BTreeMap, env, future::Future};

use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport, Direction,
    Document, DocumentQuery, ItemKind, ItemWrite, Label, LabelFilter, Location, NativeId,
    PageRequest, Project, ProjectFilter, ProjectQuery, SecretResolver, SourceName, SourcePlugin,
    Status, StatusCategory, Support, Task, TaskQuery, TaskSource, TextFields, TextQuery,
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

async fn linear(key: &str, query: &str, variables: Value, what: &str) -> Result<Value, String> {
    let body: Value = reqwest::Client::new()
        .post("https://api.linear.app/graphql")
        .header("Authorization", key)
        .json(&json!({"query":query,"variables":variables}))
        .send()
        .await
        .map_err(|error| format!("{what} could not reach Linear: {error}"))?
        .error_for_status()
        .map_err(|error| format!("{what} failed: {error}"))?
        .json()
        .await
        .map_err(|error| format!("{what} returned invalid JSON: {error}"))?;
    if let Some(errors) = body
        .get("errors")
        .filter(|errors| !errors.as_array().is_some_and(Vec::is_empty))
    {
        return Err(format!("{what} was rejected by Linear: {errors}"));
    }
    body.get("data")
        .cloned()
        .ok_or_else(|| format!("{what} returned no data: {body}"))
}

/// The prefix of every issue, project and label this lane writes.
///
/// The rest of a name is `<process id>-<microsecond timestamp>`, which makes one run's
/// artifacts unique and makes any run's artifacts recognisable to the next run.
const ARTIFACT_PREFIX: &str = "onetaskgraph live cleanup ";

/// The same for a label, whose name Linear shows in its own filter menus.
const LABEL_PREFIX: &str = "onetaskgraph-live-";

fn artifact_title(process_id: u32, stamp_micros: i64) -> String {
    format!("{ARTIFACT_PREFIX}{process_id}-{stamp_micros}")
}

fn artifact_label(process_id: u32, stamp_micros: i64) -> String {
    format!("{LABEL_PREFIX}{process_id}-{stamp_micros}")
}

/// Whether a name is one this lane wrote, under `prefix`, in this run or an earlier one.
fn is_artifact(prefix: &str, name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(prefix) else {
        return false;
    };
    let Some((process_id, stamp_micros)) = suffix.split_once('-') else {
        return false;
    };
    [process_id, stamp_micros]
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

const TEAM_STATES: &str = "query($key:String!){teams(filter:{key:{eqIgnoreCase:$key}}){nodes{id key states(first:100){nodes{id name type}}}}}";
const PROJECT_STATUSES: &str = "query{projectStatuses{nodes{id name type}}}";
const LABEL_CREATE: &str = "mutation($input:IssueLabelCreateInput!){issueLabelCreate(input:$input){success issueLabel{id name}}}";
const LABEL_DELETE: &str = "mutation($id:String!){issueLabelDelete(id:$id){success}}";
const LABELS_PAGE: &str = "query($first:Int!,$after:String){issueLabels(first:$first,after:$after){nodes{id name} pageInfo{hasNextPage endCursor}}}";
const ISSUES_BY_TITLE: &str = "query($first:Int!,$after:String,$prefix:String!){issues(first:$first,after:$after,filter:{title:{startsWith:$prefix}}){nodes{id title} pageInfo{hasNextPage endCursor}}}";
const PROJECTS_BY_NAME: &str = "query($first:Int!,$after:String,$prefix:String!){projects(first:$first,after:$after,filter:{name:{startsWith:$prefix}}){nodes{id name} pageInfo{hasNextPage endCursor}}}";
const ISSUE_PAGE_PROBE: &str = "query($first:Int!){issues(first:$first){nodes{id}}}";
const DOCUMENTS_BY_TITLE: &str = "query($first:Int!,$after:String,$prefix:String!){documents(first:$first,after:$after,filter:{title:{startsWith:$prefix}}){nodes{id title} pageInfo{hasNextPage endCursor}}}";

/// Every `(id, name)` a paged Linear connection reports.
async fn walk(
    key: &str,
    query: &str,
    connection: &str,
    name_field: &str,
    prefix: Option<&str>,
    what: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut after = Value::Null;
    let mut found = Vec::new();
    for _ in 0..50 {
        let mut variables = json!({"first":250,"after":after});
        if let Some(prefix) = prefix {
            variables["prefix"] = Value::String(prefix.to_owned());
        }
        let data = linear(key, query, variables, what).await?;
        let page = data
            .get(connection)
            .ok_or_else(|| format!("{what} returned no {connection} connection"))?;
        for node in page
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{what} returned {connection}.nodes that is not an array"))?
        {
            let (Some(id), Some(name)) = (
                node.get("id").and_then(Value::as_str),
                node.get(name_field).and_then(Value::as_str),
            ) else {
                return Err(format!(
                    "{what} returned a {connection} node with no id or name"
                ));
            };
            found.push((id.to_owned(), name.to_owned()));
        }
        if page
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Ok(found);
        }
        let next = page
            .pointer("/pageInfo/endCursor")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{what} has no advancing cursor"))?;
        if after.as_str() == Some(next) {
            return Err(format!("{what} cursor did not advance"));
        }
        after = Value::String(next.to_owned());
    }
    Err(format!("{what} did not terminate"))
}

/// Deletes every issue, project, document and label whose name `matches`.
///
/// Called twice: once before the journey, over any run's naming, which is what heals a
/// run killed between its writes and its cleanup; and once after it, over this run's own,
/// which is what makes the lane leave nothing behind whether it passed or failed.
async fn remove_artifacts(key: &str, matches: &dyn Fn(&str, &str) -> bool) -> Result<(), String> {
    let issues = walk(
        key,
        ISSUES_BY_TITLE,
        "issues",
        "title",
        Some(ARTIFACT_PREFIX),
        "live issue residue lookup",
    )
    .await?;
    for (id, title) in issues
        .iter()
        .filter(|(_, title)| matches(ARTIFACT_PREFIX, title))
    {
        let data = linear(
            key,
            onetaskgraph_linear::graphql::ISSUE_DELETE,
            json!({"id":id}),
            "live issue cleanup",
        )
        .await?;
        if data.pointer("/issueDelete/success") != Some(&Value::Bool(true)) {
            return Err(format!("Linear did not confirm deleting issue {title:?}"));
        }
    }
    let projects = walk(
        key,
        PROJECTS_BY_NAME,
        "projects",
        "name",
        Some(ARTIFACT_PREFIX),
        "live project residue lookup",
    )
    .await?;
    for (id, name) in projects
        .iter()
        .filter(|(_, name)| matches(ARTIFACT_PREFIX, name))
    {
        let data = linear(
            key,
            onetaskgraph_linear::graphql::PROJECT_DELETE,
            json!({"id":id}),
            "live project cleanup",
        )
        .await?;
        if data.pointer("/projectDelete/success") != Some(&Value::Bool(true)) {
            return Err(format!("Linear did not confirm deleting project {name:?}"));
        }
    }
    let documents = walk(
        key,
        DOCUMENTS_BY_TITLE,
        "documents",
        "title",
        Some(ARTIFACT_PREFIX),
        "live document residue lookup",
    )
    .await?;
    for (id, title) in documents
        .iter()
        .filter(|(_, title)| matches(ARTIFACT_PREFIX, title))
    {
        let data = linear(
            key,
            onetaskgraph_linear::graphql::DOCUMENT_DELETE,
            json!({"id":id}),
            "live document cleanup",
        )
        .await?;
        if data.pointer("/documentDelete/success") != Some(&Value::Bool(true)) {
            return Err(format!(
                "Linear did not confirm deleting document {title:?}"
            ));
        }
    }
    let labels = walk(
        key,
        LABELS_PAGE,
        "issueLabels",
        "name",
        None,
        "live label lookup",
    )
    .await?;
    for (id, name) in labels
        .iter()
        .filter(|(_, name)| matches(LABEL_PREFIX, name))
    {
        let data = linear(key, LABEL_DELETE, json!({"id":id}), "live label cleanup").await?;
        if data.pointer("/issueLabelDelete/success") != Some(&Value::Bool(true)) {
            return Err(format!("Linear did not confirm deleting label {name:?}"));
        }
    }
    Ok(())
}

async fn run_then_cleanup<J, JF, C, CF>(journey: J, cleanup: C) -> Result<(), String>
where
    J: FnOnce() -> JF,
    JF: Future<Output = Result<(), String>>,
    C: FnOnce() -> CF,
    CF: Future<Output = Result<(), String>>,
{
    let journey_result = journey().await;
    let cleanup_result = cleanup().await;
    match (journey_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(journey), Ok(())) => Err(journey),
        (Ok(()), Err(cleanup)) => Err(format!("live cleanup failed: {cleanup}")),
        (Err(journey), Err(cleanup)) => Err(format!(
            "{journey}; additionally, live cleanup failed: {cleanup}"
        )),
    }
}

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

async fn task_titles(
    source: &dyn TaskSource,
    query: &TaskQuery,
    what: &str,
) -> Result<Vec<String>, String> {
    Ok(sorted(
        source
            .query_tasks(query, &page(50))
            .await
            .map_err(|error| format!("live {what} failed: {error}"))?
            .items
            .into_iter()
            .map(|task| task.title)
            .collect(),
    ))
}

/// Everything this lane needs to reach the one team it may write to.
struct LiveRun {
    key: String,
    process_id: u32,
    stamp_micros: i64,
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
    let title = |offset: i64| artifact_title(run.process_id, run.stamp_micros + offset);
    let (alpha, beta) = (title(0), title(1));
    let (first, second, orphan) = (title(2), title(3), title(4));
    let run_label = artifact_label(run.process_id, run.stamp_micros);
    let only_label = artifact_label(run.process_id, run.stamp_micros + 1);
    create_label(&run.key, team_id, &run_label).await?;
    create_label(&run.key, team_id, &only_label).await?;
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
        title: name.to_owned(),
        content: Some("temporary credentialed write; the live lane removes this".into()),
        status: status.clone(),
        labels,
        project: under.cloned(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
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
    // reads below wait for the fixture rather than racing it.
    let mut settled = false;
    for _ in 0..20 {
        if task_titles(source, &scoped(), "fixture settling read")
            .await?
            .len()
            == 3
        {
            settled = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let all_three = sorted(vec![first.clone(), second.clone(), orphan.clone()]);
    ensure!(
        settled,
        "the live fixture never became readable: Linear never returned all three issues \
         labelled {run_label}"
    );
    ensure!(
        task_titles(source, &scoped(), "scoped read").await? == all_three,
        "the three issues this run created did not come back as {all_three:?}"
    );

    // `projects`: two are held, and a listing scoped to one keeps the issue filed under
    // it and no other. Unscoped on purpose — the workspace holds issues of its own, so a
    // filter declared and then ignored returns them too.
    let mut listed = Vec::new();
    let mut cursor = None;
    loop {
        let step = source
            .query_projects(
                &ProjectQuery::default(),
                &PageRequest { cursor, limit: 250 },
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
    let under_alpha = task_titles(source, &under(&alpha_id), "project filter").await?;
    ensure!(
        under_alpha == vec![first.clone()],
        "the issues of one of this run's two projects came back as {under_alpha:?}"
    );
    let under_beta = task_titles(source, &under(&beta_id), "project filter").await?;
    ensure!(
        under_beta == vec![second.clone()],
        "the issues of the other of this run's two projects came back as {under_beta:?}"
    );

    // `orphan_tasks`: the one issue filed under neither project.
    let orphans = task_titles(
        source,
        &TaskQuery {
            project: ProjectFilter::Orphans,
            ..scoped()
        },
        "orphan selection",
    )
    .await?;
    ensure!(
        orphans == vec![orphan.clone()],
        "this run's issues belonging to no project came back as {orphans:?}"
    );

    // `filter_by_label`: one of the three carries the second label, and the exclusion
    // keeps exactly the other two.
    let carrying = task_titles(
        source,
        &TaskQuery {
            labels: LabelFilter {
                any_of: vec![only_label.clone()],
                ..LabelFilter::default()
            },
            ..TaskQuery::default()
        },
        "label filter",
    )
    .await?;
    ensure!(
        carrying == vec![first.clone()],
        "this run's issues carrying its second label came back as {carrying:?}"
    );
    let without = task_titles(
        source,
        &TaskQuery {
            labels: LabelFilter {
                any_of: vec![run_label.clone()],
                none_of: vec![only_label.clone()],
                ..LabelFilter::default()
            },
            ..TaskQuery::default()
        },
        "label exclusion",
    )
    .await?;
    ensure!(
        without == sorted(vec![second.clone(), orphan.clone()]),
        "this run's issues not carrying its second label came back as {without:?}"
    );

    // `filter_by_status`: two issues sit in an `unstarted` state and one in a `completed`
    // one, so the normalised categories separate them.
    let todo = task_titles(
        source,
        &TaskQuery {
            statuses: vec![StatusCategory::Todo],
            ..scoped()
        },
        "status filter",
    )
    .await?;
    ensure!(
        todo == sorted(vec![first.clone(), second.clone()]),
        "this run's unstarted issues came back as {todo:?}"
    );
    let finished = task_titles(
        source,
        &TaskQuery {
            statuses: vec![StatusCategory::Done],
            ..scoped()
        },
        "status filter",
    )
    .await?;
    ensure!(
        finished == vec![orphan.clone()],
        "this run's completed issue came back as {finished:?}"
    );

    // `search_title` and `search_content` are declared `Unsupported`, and capability rule
    // 2 is what that declaration promises: the source **ignores** the predicate and
    // returns the wider set, so the engine can narrow it. A source that half-applied one
    // would return fewer rows here, which is the one failure no test above the plugin can
    // catch.
    for fields in [
        TextFields::Title,
        TextFields::Content,
        TextFields::TitleOrContent,
    ] {
        let searched = task_titles(
            source,
            &TaskQuery {
                text: Some(TextQuery {
                    terms: first.clone(),
                    fields,
                }),
                ..scoped()
            },
            "ignored search",
        )
        .await?;
        ensure!(
            searched == all_three,
            "a {fields:?} search this source declares unsupported narrowed the result to \
             {searched:?} instead of returning the wider set"
        );
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
    // exactly once and in the order one whole page reports them.
    let whole = source
        .query_tasks(&scoped(), &page(50))
        .await
        .map_err(|error| format!("live whole-page read failed: {error}"))?
        .items
        .into_iter()
        .map(|task| task.title)
        .collect::<Vec<_>>();
    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let step = source
            .query_tasks(&scoped(), &PageRequest { cursor, limit: 1 })
            .await
            .map_err(|error| format!("live paged read failed: {error}"))?;
        ensure!(
            step.items.len() <= 1,
            "a page of one returned {} rows",
            step.items.len()
        );
        walked.extend(step.items.into_iter().map(|task| task.title));
        cursor = step.next;
        if cursor.is_none() {
            break;
        }
        ensure!(
            walked.len() <= 10,
            "the paged walk over this run's own three issues must terminate"
        );
    }
    ensure!(
        walked == whole,
        "a walk in pages of one reached {walked:?} where one whole page reports {whole:?}"
    );

    // `max_page_size`: the source clamps a limit one above its declared ceiling to that
    // ceiling instead of passing it on, so the read succeeds rather than being refused
    // for a page size Linear's connection cannot serve.
    let ceiling = source.capabilities().max_page_size;
    let clamped = sorted(
        source
            .query_tasks(
                &scoped(),
                &PageRequest {
                    cursor: None,
                    limit: ceiling + 1,
                },
            )
            .await
            .map_err(|error| {
                format!(
                    "a limit one above the declared ceiling was refused rather than clamped: \
                     {error}"
                )
            })?
            .items
            .into_iter()
            .map(|task| task.title)
            .collect(),
    );
    ensure!(
        clamped == all_three,
        "a limit above the declared ceiling returned {clamped:?} rather than this run's own \
         three issues"
    );
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

    drive_documents(run, source, &alpha_id, &title(5), &title(6)).await
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
            labels: vec![label(&artifact_label(run.process_id, run.stamp_micros))],
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
    // predicate the other, and a label demanded of a document keeps neither.
    let titles = |query: DocumentQuery| async move {
        let mut found = source
            .query_documents(&query, &page(250))
            .await
            .map_err(|error| format!("live document read failed: {error}"))?
            .items
            .into_iter()
            .map(|document| document.title)
            .filter(|title| is_artifact(ARTIFACT_PREFIX, title))
            .collect::<Vec<_>>();
        found.sort();
        Ok::<_, String>(found)
    };
    let both = sorted(vec![filed.to_owned(), loose.to_owned()]);
    ensure!(
        titles(DocumentQuery::default()).await? == both,
        "the two documents this run created did not come back as {both:?}"
    );
    ensure!(
        titles(DocumentQuery {
            project: ProjectFilter::Is(under.clone()),
            ..DocumentQuery::default()
        })
        .await?
            == vec![filed.to_owned()],
        "a document listing narrowed to this run's project kept the wrong documents"
    );
    ensure!(
        titles(DocumentQuery {
            project: ProjectFilter::Orphans,
            ..DocumentQuery::default()
        })
        .await?
            == vec![loose.to_owned()],
        "a document listing narrowed to the orphans kept the wrong documents"
    );
    ensure!(
        titles(DocumentQuery {
            labels: LabelFilter {
                any_of: vec![artifact_label(run.process_id, run.stamp_micros)],
                ..LabelFilter::default()
            },
            ..DocumentQuery::default()
        })
        .await?
        .is_empty(),
        "no Linear document carries a label, so a query demanding one keeps nothing"
    );

    // And removed again, which is what lets a copy that could not finish take one back.
    // The sweep would clear them anyway; driving the verb is what proves it works.
    for id in [&filed_id, &loose_id] {
        source
            .delete_document(id)
            .await
            .map_err(|error| format!("live document removal failed: {error}"))?;
    }
    ensure!(
        source
            .get_document(&loose_id)
            .await
            .is_ok_and(|held| held.is_none()),
        "a document this run removed is still readable"
    );
    Ok(())
}

#[ignore = "the live lane: run it with `just test-live onetaskgraph-linear`"]
#[tokio::test]
async fn real_linear_applies_every_declared_capability_and_leaves_no_residue() {
    // llmlint: ignore-block[live_tier_compiles_and_requires_credential,tests_assert_real_behavior] This non-required third-party lane deliberately reports an absent credential or an unnamed scratch team as a skip, matching every live target in this repository (AGENTS.md: a required check a third party can turn red stops being trusted); required checks compile but never execute this ignored test.
    let Some(key) = env::var("LINEAR_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!("skipped live Linear journey: LINEAR_API_KEY is missing");
        return;
    };
    let Some(team) = env::var("LINEAR_WRITE_TEAM")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!(
            "skipped live Linear journey: LINEAR_WRITE_TEAM is missing, and this lane writes \
             only to the scratch team that name gives rather than discovering one; no mutation \
             was sent"
        );
        return;
    };
    // llmlint: ignore-end[live_tier_compiles_and_requires_credential,tests_assert_real_behavior]
    let source = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("live").unwrap(),
            &json!({"team":team}),
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
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Unsupported,
            search_content: Support::Unsupported,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: 250,
        }
    );

    // Self-healing after an interrupted run: a process killed between its writes and its
    // cleanup never reaches `run_then_cleanup`, so the next run clears what it left. What
    // bounds where this lane may write is `LINEAR_WRITE_TEAM`, not this sweep.
    remove_artifacts(&key, &is_artifact)
        .await
        .unwrap_or_else(|error| {
            panic!("live residue left by an earlier interrupted run could not be cleared: {error}")
        });
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
    let (open_state, done_state) = fixture_states(&key, &team)
        .await
        .unwrap_or_else(|error| panic!("the Linear live lane cannot file its fixture: {error}"));
    let project_status = fixture_project_status(&key)
        .await
        .unwrap_or_else(|error| panic!("the Linear live lane cannot file its projects: {error}"));
    let run = LiveRun {
        key: key.clone(),
        process_id: std::process::id(),
        stamp_micros: chrono::Utc::now().timestamp_micros(),
        open_state,
        done_state,
        project_status,
    };
    let mine = format!("{}-", run.process_id);
    let is_this_runs = move |prefix: &str, name: &str| {
        is_artifact(prefix, name) && name.starts_with(&format!("{prefix}{mine}"))
    };
    run_then_cleanup(
        || drive_every_declared_capability(&run, source.as_ref(), &team_id),
        || remove_artifacts(&key, &is_this_runs),
    )
    .await
    .unwrap_or_else(|error| panic!("Linear live capability journey failed: {error}"));
}
