//! A copy into Linear asks for every origin's counterpart in one request, through the binary.
//!
//! A plan copied out of a folder of Markdown into the loopback Linear looks for the item an
//! earlier copy made of each thing it writes by the origin that item records. The Linear plugin
//! asks for all of them at once — `graphql::COUNTERPART_LOOKUPS`, one request per
//! `ORIGIN_BATCH` origins, carrying the organization id the designator digests — and answers
//! each origin's own lookup from it. These journeys read the workspace's request log for what
//! was sent and its rows for what it holds, and hold the batch to answering exactly what each
//! origin's own lookup would: every match, and nothing taken as absent from a page with more
//! behind it.

use std::path::{Path, PathBuf};
use std::process::Output;

use onetaskgraph_linear::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace_with};

const TEAM_STATES: &[(&str, &str, &str)] = &[
    ("S-ready", "Ready", "unstarted"),
    ("S-working", "Working", "started"),
    ("S-finished", "Finished", "completed"),
    ("S-dropped", "Dropped", "canceled"),
];

const PROJECT_STATUSES: &[(&str, &str)] = &[
    ("Scheduled", "planned"),
    ("Underway", "started"),
    ("Shipped", "completed"),
    ("Abandoned", "canceled"),
];

fn mapping() -> Value {
    json!({
        "todo":        {"task": "Ready",    "project": "Scheduled"},
        "in-progress": {"task": "Working",  "project": "Underway"},
        "done":        {"task": "Finished", "project": "Shipped"},
        "cancelled":   {"task": "Dropped",  "project": "Abandoned"},
    })
}

/// One sandbox: a folder of Markdown called `plan` holding a project `big` and its tasks, and
/// a loopback Linear called `linear`.
struct Copying {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    plan: PathBuf,
    /// The cache home every run is given, so a cache the source turns on stays in the sandbox.
    cache_home: PathBuf,
    /// The team the Linear source is configured with.
    team: Value,
    /// Whether the Linear source's cache is on.
    cache: bool,
}

impl Copying {
    /// A plan of `tasks` tasks, `t0` onward, over a workspace holding `issues` already.
    fn new(tasks: usize, issues: Vec<Value>) -> Self {
        Self::caching(tasks, issues, false)
    }

    /// [`Self::new`], with the Linear source's cache on when `cache` says so.
    fn caching(tasks: usize, issues: Vec<Value>, cache: bool) -> Self {
        let sandbox = Sandbox::new();
        let held = json!({"tasks": issues, "projects": [], "documents": [], "labels": [],
                          "task_dependencies": [], "project_dependencies": []});
        let (mut config, workspace) =
            linear_workspace_with(&sandbox, held, TEAM_STATES, PROJECT_STATUSES);
        config["status_mapping"] = mapping();
        let plan = sandbox.subdirectory("plan");
        for kind in ["tasks", "projects", "documents"] {
            std::fs::create_dir_all(plan.join(kind)).expect("a folder");
        }
        std::fs::write(
            plan.join("projects/big.md"),
            "---\ntitle: Big\nstatus: todo\n---\nThe plan.\n",
        )
        .expect("the project");
        for at in 0..tasks {
            write_task(&plan, at, "Body.");
        }
        sandbox.project_document(&document(&json!({
            "linear": {"plugin": "linear", "config": config, "cache": {"enabled": cache}},
            "plan": {"plugin": "local-md", "config": {"root": plan}},
        })));
        let cache_home = sandbox.subdirectory("cache-home");
        let team = config["team"].clone();
        Self {
            sandbox,
            workspace,
            plan,
            cache_home,
            team,
            cache,
        }
    }

    /// Run the binary; what it printed and every request the workspace answered while it ran.
    fn run(&self, arguments: &[&str]) -> (Output, Vec<(String, Value)>) {
        let from = self.workspace.served().len();
        let output = self
            .sandbox
            .command()
            .env("XDG_CACHE_HOME", &self.cache_home)
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        (output, self.workspace.served()[from..].to_vec())
    }

    /// [`Self::run`], which has to succeed.
    fn ok(&self, arguments: &[&str]) -> Vec<(String, Value)> {
        let (output, served) = self.run(arguments);
        assert!(
            output.status.success(),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        served
    }
}

fn write_task(plan: &Path, at: usize, body: &str) {
    std::fs::write(
        plan.join(format!("tasks/t{at}.md")),
        format!("---\ntitle: Task {at}\nstatus: todo\nproject: big\n---\n{body}\n"),
    )
    .expect("a task");
}

/// The copy every journey here makes: the whole plan into Linear.
const COPY: [&str; 6] = ["project", "copy", "plan:big", "--to", "linear", "--json"];

/// An issue the workspace holds before anything is copied, recording `origin`.
fn issue_from(id: &str, title: &str, origin: &str) -> Value {
    json!({
        "id": id, "title": title, "content": format!("Held as {id}."),
        "status": {"name": "Ready", "category": "unknown"},
        "_linear_state": {"name": "Ready", "type": "unstarted"},
        "labels": [], "priority": "none",
        "metadata": {"onetaskgraph.origin": origin},
    })
}

/// Every string a `contains` comparator of `filter` asks for: the slot phrases it carries.
fn phrases(filter: &Value) -> Vec<String> {
    let mut found = Vec::new();
    match filter {
        Value::Object(members) => {
            for (member, value) in members {
                match (member.as_str(), value) {
                    ("contains", Value::String(phrase)) => found.push(phrase.clone()),
                    _ => found.extend(phrases(value)),
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                found.extend(phrases(value));
            }
        }
        _ => {}
    }
    found
}

/// The slot phrase an origin is asked for by.
fn phrase(origin: &str) -> String {
    format!("\"{origin}\"")
}

/// Hold `variables`, one batched origin read's, to the projects half the plugin sends: the
/// source's project scope alone — the projects the configured team can reach, with no slot
/// phrase, since Linear's `ProjectFilter` has no member that compares a project's description
/// — on a page of what one projects listing page leaves beside the issues page and the
/// resolution the read carries.
fn assert_projects_half(copying: &Copying, variables: &Value) {
    assert_eq!(
        variables["projectsFilter"],
        json!({"accessibleTeams": {"some": {"key": {"eqIgnoreCase": copying.team}}}}),
        "the projects half lists the source's project scope and nothing else"
    );
    assert!(
        phrases(&variables["projectsFilter"]).is_empty(),
        "no slot phrase is sent in a project filter: {variables}"
    );
    let listing_page = if copying.cache {
        onetaskgraph_linear::PROJECTS_CACHED_PAGE
    } else {
        onetaskgraph_linear::MAX_PAGE_SIZE
    };
    let issues_first = variables["issuesFirst"].as_u64().expect("a page size");
    assert_eq!(
        variables["projectsFirst"].as_u64(),
        Some(
            u64::from(listing_page)
                - issues_first
                - u64::from(onetaskgraph_linear::RESOLUTION_ROWS)
        ),
        "the projects page is the rest of one projects listing page: {variables}"
    );
}

/// The variables of every batched origin read of `served`, in order — with the cache on or off.
fn batched(served: &[(String, Value)]) -> Vec<&Value> {
    served
        .iter()
        .filter(|(query, _)| {
            query == graphql::COUNTERPART_LOOKUPS || query == graphql::COUNTERPART_LOOKUPS_CACHED
        })
        .map(|(_, variables)| variables)
        .collect()
}

/// The origins each single-origin issue lookup of `served` asked for, in order: an `issues`
/// listing carrying a slot phrase.
fn single_issue_lookups(served: &[(String, Value)]) -> Vec<Vec<String>> {
    served
        .iter()
        .filter(|(query, _)| query == graphql::ISSUES || query == graphql::ISSUES_CACHED)
        .map(|(_, variables)| phrases(&variables["filter"]))
        .filter(|asked| !asked.is_empty())
        .collect()
}

/// How many requests of `served` were `document`.
fn count(served: &[(String, Value)], document: &str) -> usize {
    served.iter().filter(|(query, _)| query == document).count()
}

/// Where the first write of `served` is: its first mutation.
fn first_write(served: &[(String, Value)]) -> usize {
    served
        .iter()
        .position(|(query, _)| query.trim_start().starts_with("mutation"))
        .expect("the copy wrote")
}

/// Every task or project the workspace holds, as the binary lists it.
fn listed(copying: &Copying, kind: &str) -> Vec<Value> {
    let (output, _) = copying.run(&[
        kind, "list", "--source", "linear", "--json", "--limit", "100",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let listing: Value = serde_json::from_str(&stdout(&output)).expect("a listing");
    listing["items"]
        .as_array()
        .expect("a listing")
        .iter()
        .map(|row| row["item"].clone())
        .collect()
}

/// Take every copy link out of the plan's files, as a person deleting them by hand would:
/// the next copy can find each counterpart by its origin alone.
fn unlink(plan: &Path) {
    for kind in ["tasks", "projects"] {
        for entry in std::fs::read_dir(plan.join(kind)).expect("a folder") {
            let path = entry.expect("an entry").path();
            let text = std::fs::read_to_string(&path).expect("a file");
            let kept: Vec<&str> = text
                .lines()
                .filter(|line| {
                    !line.trim_start().starts_with("\"onetaskgraph.copies\"")
                        && !line
                            .trim_start()
                            .starts_with("\"onetaskgraph.copy_locations\"")
                })
                .collect();
            // A `metadata:` left with no entry under it holds nothing.
            let mut lines: Vec<&str> = Vec::new();
            for (at, line) in kept.iter().enumerate() {
                let next = kept.get(at + 1).copied().unwrap_or_default();
                if *line == "metadata:" && !next.starts_with(' ') {
                    continue;
                }
                lines.push(line);
            }
            let unlinked = format!("{}\n", lines.join("\n"));
            assert!(
                !unlinked.contains("onetaskgraph.cop"),
                "every link is gone from {}: {unlinked}",
                path.display()
            );
            std::fs::write(&path, unlinked).expect("the file");
        }
    }
}

/// The outcome the copy report gives the task `t{at}`.
fn outcome_of(report: &Value, at: usize) -> Value {
    report["items"]
        .as_array()
        .expect("a copy report")
        .iter()
        .find(|item| item["source"] == format!("plan:t{at}"))
        .unwrap_or_else(|| panic!("the copy reports plan:t{at}: {report}"))
        .clone()
}

#[test]
fn a_project_copy_into_linear_asks_for_every_origin_and_the_organization_in_one_request() {
    let copying = Copying::new(5, Vec::new());
    let served = copying.ok(&COPY);
    let before = &served[..first_write(&served)];
    let asked = batched(before);
    assert_eq!(
        asked.len(),
        1,
        "one batched origin read before the first write: {before:#?}"
    );
    let mut origins: Vec<String> = phrases(&asked[0]["issuesFilter"]);
    origins.sort();
    let mut expected: Vec<String> = [
        "plan:big", "plan:t0", "plan:t1", "plan:t2", "plan:t3", "plan:t4",
    ]
    .iter()
    .map(|origin| phrase(origin))
    .collect();
    expected.sort();
    assert_eq!(origins, expected, "the project's and every task's origin");
    assert_eq!(asked[0]["issuesFirst"], 6, "one row an origin");
    assert_projects_half(&copying, asked[0]);
    assert_eq!(
        single_issue_lookups(&served),
        Vec::<Vec<String>>::new(),
        "no task's origin is asked on its own: {served:#?}"
    );
    assert_eq!(
        count(&served, graphql::PROJECTS),
        0,
        "the project's origin is not asked on its own: {served:#?}"
    );
    assert_eq!(
        count(&served, graphql::ORGANIZATION),
        0,
        "the designator is answered from the batched read: {served:#?}"
    );
    assert_eq!(listed(&copying, "task").len(), 5);
    assert_eq!(listed(&copying, "project").len(), 1);
}

#[test]
fn a_copy_of_a_plan_of_sixty_tasks_sends_two_batched_origin_reads() {
    let copying = Copying::new(60, Vec::new());
    let served = copying.ok(&COPY);
    let before = &served[..first_write(&served)];
    let asked = batched(&served);
    assert_eq!(asked.len(), 2, "fifty origins a request: {}", asked.len());
    assert_eq!(batched(before).len(), 2, "both before the first write");
    let sizes: Vec<usize> = asked
        .iter()
        .map(|variables| phrases(&variables["issuesFilter"]).len())
        .collect();
    assert_eq!(
        sizes,
        vec![50, 11],
        "the project and sixty tasks, fifty at a time"
    );
    for variables in &asked {
        let first = variables["issuesFirst"].as_u64().expect("a page size");
        let projects = variables["projectsFirst"].as_u64().expect("a page size");
        assert_eq!(
            first,
            phrases(&variables["issuesFilter"]).len() as u64,
            "the issues page is one row an origin"
        );
        assert!(first + projects <= u64::from(onetaskgraph_linear::MAX_PAGE_SIZE));
        assert_projects_half(&copying, variables);
    }
    assert!(single_issue_lookups(&served).is_empty(), "{served:#?}");
    assert_eq!(count(&served, graphql::PROJECTS), 0);
    assert_eq!(count(&served, graphql::ORGANIZATION), 0);
    assert_eq!(listed(&copying, "task").len(), 60);
}

#[test]
fn a_recopy_with_no_links_finds_every_counterpart_through_the_batched_read_and_updates_it() {
    let copying = Copying::new(5, Vec::new());
    copying.ok(&COPY);
    let tasks = listed(&copying, "task");
    let projects = listed(&copying, "project");
    unlink(&copying.plan);
    for at in 0..5 {
        write_task(&copying.plan, at, &format!("Edited {at}."));
    }
    let served = copying.ok(&COPY);
    let before = &served[..first_write(&served)];
    assert_eq!(batched(before).len(), 1, "{before:#?}");
    assert!(single_issue_lookups(&served).is_empty(), "{served:#?}");
    assert_eq!(count(&served, graphql::PROJECTS), 0, "{served:#?}");
    assert_eq!(
        count(&served, graphql::ISSUE_CREATE),
        0,
        "no task created again"
    );
    assert_eq!(
        count(&served, graphql::PROJECT_CREATE),
        0,
        "no project created again"
    );
    let after = listed(&copying, "task");
    assert_eq!(
        after.len(),
        tasks.len(),
        "the workspace holds the tasks it held: {after:#?}"
    );
    assert_eq!(listed(&copying, "project").len(), projects.len());
    for at in 0..5 {
        let task = after
            .iter()
            .find(|task| task["title"] == format!("Task {at}"))
            .unwrap_or_else(|| panic!("Task {at} is held: {after:#?}"));
        assert_eq!(
            task["content"],
            format!("Edited {at}."),
            "Task {at} was updated in place: {task}"
        );
        let held = tasks
            .iter()
            .find(|held| held["title"] == format!("Task {at}"))
            .expect("held before");
        assert_eq!(task["id"], held["id"], "the same issue");
    }
}

/// A plan of three tasks copied over a workspace already holding two issues recording `t0`'s
/// origin, with the batched read's page capped at `cap` rows when one is given: the copy's
/// report, its exit, what it sent and the tasks the workspace holds afterwards.
fn over_two_matches(cap: Option<usize>) -> (Output, Vec<(String, Value)>, Vec<Value>) {
    let copying = Copying::new(
        3,
        vec![
            issue_from("L-A", "Held A", "plan:t0"),
            issue_from("L-B", "Held B", "plan:t0"),
        ],
    );
    if let Some(rows) = cap {
        copying.workspace.cap_origin_lookup_page(rows);
    }
    let (output, served) = copying.run(&COPY);
    let tasks = listed(&copying, "task");
    (output, served, tasks)
}

/// What a person reads of a task the workspace holds: its id, title and content.
fn shown(tasks: &[Value]) -> Vec<(Value, Value, Value)> {
    let mut shown: Vec<_> = tasks
        .iter()
        .map(|task| {
            (
                task["id"].clone(),
                task["title"].clone(),
                task["content"].clone(),
            )
        })
        .collect();
    shown.sort_by_key(|(id, _, _)| id.to_string());
    shown
}

#[test]
fn two_issues_recording_one_origin_are_chosen_or_refused_as_that_origins_own_lookup_does() {
    // Batched, and with every origin sent on its own because the batched page cannot hold one
    // row: the same copy over the same workspace.
    let (batch, batch_served, batch_tasks) = over_two_matches(None);
    let (single, single_served, single_tasks) = over_two_matches(Some(1));
    assert_eq!(batched(&batch_served).len(), 1);
    assert!(
        single_issue_lookups(&batch_served).is_empty(),
        "the batched read answered every origin: {batch_served:#?}"
    );
    assert!(
        single_issue_lookups(&single_served).contains(&vec![phrase("plan:t0")]),
        "with the page capped, t0's own lookup is sent: {single_served:#?}"
    );
    assert_eq!(
        batch.status.code(),
        single.status.code(),
        "{}\n{}",
        stderr(&batch),
        stderr(&single)
    );
    let report = |output: &Output| serde_json::from_str::<Value>(&stdout(output)).ok();
    match (report(&batch), report(&single)) {
        (Some(batch), Some(single)) => {
            assert_eq!(outcome_of(&batch, 0), outcome_of(&single, 0));
        }
        (batch, single) => assert_eq!(batch.is_some(), single.is_some()),
    }
    assert_eq!(
        stderr(&batch).replace(char::is_numeric, ""),
        stderr(&single).replace(char::is_numeric, ""),
        "the same diagnostic"
    );
    assert_eq!(
        shown(&batch_tasks),
        shown(&single_tasks),
        "the same workspace afterwards"
    );
    let mut recording: Vec<&Value> = batch_tasks
        .iter()
        .filter(|task| task["metadata"]["onetaskgraph.origin"] == "plan:t0")
        .map(|task| &task["id"])
        .collect();
    recording.sort_by_key(|id| id.to_string());
    assert_eq!(
        recording,
        vec!["L-A", "L-B"],
        "no item is created for t0: {batch_tasks:#?}"
    );
    assert_eq!(
        count(&batch_served, graphql::ISSUE_CREATE),
        2,
        "t1 and t2 alone are created"
    );
}

#[test]
fn an_incomplete_batched_page_answers_no_origin_and_each_is_looked_up_on_its_own() {
    let copying = Copying::new(4, Vec::new());
    copying.ok(&COPY);
    unlink(&copying.plan);
    for at in 0..4 {
        write_task(&copying.plan, at, &format!("Edited {at}."));
    }
    // A second issue recording t0's origin, after every counterpart, so a page of two rows
    // holds t0's and t1's counterparts and neither t2's, t3's nor t0's second match.
    copying
        .workspace
        .add_issue(issue_from("L-LATER", "Held later", "plan:t0"));
    copying.workspace.cap_origin_lookup_page(2);
    let (output, served) = copying.run(&COPY);
    let asked = batched(&served);
    assert_eq!(asked.len(), 1, "{served:#?}");
    assert_eq!(
        count(&served, graphql::ISSUE_CREATE),
        0,
        "no item is created for a task whose counterpart the page omitted"
    );
    assert_eq!(count(&served, graphql::PROJECT_CREATE), 0);
    let tasks = listed(&copying, "task");
    assert_eq!(tasks.len(), 5, "four counterparts and t0's second match");
    for at in 1..4 {
        let task = tasks
            .iter()
            .find(|task| task["title"] == format!("Task {at}"))
            .unwrap_or_else(|| panic!("Task {at}: {tasks:#?}"));
        assert_eq!(task["content"], format!("Edited {at}."), "updated in place");
    }
    let lookups = single_issue_lookups(&served);
    for at in [0, 2, 3] {
        assert!(
            lookups.contains(&vec![phrase(&format!("plan:t{at}"))]),
            "t{at}, omitted from the page or matched again past it, is asked on its own: \
             {lookups:?}"
        );
    }

    // t0, with two issues recording its origin, is answered as its own lookup answers it over
    // a page that holds both: the same copy where no batch could stand in for that lookup.
    let twin = Copying::new(4, Vec::new());
    twin.ok(&COPY);
    unlink(&twin.plan);
    for at in 0..4 {
        write_task(&twin.plan, at, &format!("Edited {at}."));
    }
    twin.workspace
        .add_issue(issue_from("L-LATER", "Held later", "plan:t0"));
    twin.workspace.cap_origin_lookup_page(1);
    let (alone, _) = twin.run(&COPY);
    assert_eq!(
        output.status.code(),
        alone.status.code(),
        "{}\n{}",
        stderr(&output),
        stderr(&alone)
    );
    let report = |output: &Output| serde_json::from_str::<Value>(&stdout(output)).ok();
    if let (Some(batch), Some(alone)) = (report(&output), report(&alone)) {
        assert_eq!(outcome_of(&batch, 0), outcome_of(&alone, 0));
    }
    assert_eq!(shown(&tasks), shown(&listed(&twin, "task")));
}

#[test]
fn with_the_cache_on_a_recopy_finds_every_counterpart_through_the_batched_read_its_listing_spells()
{
    let copying = Copying::caching(5, Vec::new(), true);
    copying.ok(&COPY);
    let tasks = listed(&copying, "task");
    unlink(&copying.plan);
    for at in 0..5 {
        write_task(&copying.plan, at, &format!("Edited {at}."));
    }
    let served = copying.ok(&COPY);
    let before = &served[..first_write(&served)];
    assert_eq!(
        count(before, graphql::COUNTERPART_LOOKUPS_CACHED),
        1,
        "the batched read carrying the token parts a listing of each row spells: {before:#?}"
    );
    assert_projects_half(&copying, batched(before)[0]);
    assert!(single_issue_lookups(&served).is_empty(), "{served:#?}");
    assert_eq!(count(&served, graphql::PROJECTS_CACHED), 0, "{served:#?}");
    assert_eq!(
        count(&served, graphql::ISSUE_CREATE),
        0,
        "no task created again"
    );
    assert_eq!(
        count(&served, graphql::PROJECT_CREATE),
        0,
        "no project created again"
    );
    let after = listed(&copying, "task");
    assert_eq!(after.len(), tasks.len());
    for at in 0..5 {
        let task = after
            .iter()
            .find(|task| task["title"] == format!("Task {at}"))
            .unwrap_or_else(|| panic!("Task {at} is held: {after:#?}"));
        assert_eq!(task["content"], format!("Edited {at}."), "updated in place");
    }
}

/// A plan of four tasks copied into Linear beside six other projects the team reaches, then
/// unlinked and edited, with each batched connection capped at four rows: the issues half holds
/// all four tasks' counterparts with nothing behind them, and the projects half — which no origin
/// narrows — four of seven projects with a next page. Answers the sandbox and the plan's project
/// as the workspace held it.
fn over_a_projects_page_with_more_behind_it() -> (Copying, Value) {
    let copying = Copying::new(4, Vec::new());
    for at in 0..6 {
        std::fs::write(
            copying.plan.join(format!("projects/o{at}.md")),
            format!("---\ntitle: Other {at}\nstatus: todo\n---\nAnother plan.\n"),
        )
        .expect("a project");
        let other = format!("plan:o{at}");
        copying.ok(&["project", "copy", &other, "--to", "linear", "--json"]);
    }
    copying.ok(&COPY);
    let projects = listed(&copying, "project");
    assert_eq!(projects.len(), 7, "{projects:#?}");
    let big = projects
        .iter()
        .find(|project| project["title"] == "Big")
        .expect("the plan's project")
        .clone();
    unlink(&copying.plan);
    std::fs::write(
        copying.plan.join("projects/big.md"),
        "---\ntitle: Big\nstatus: todo\n---\nThe plan, edited.\n",
    )
    .expect("the project");
    for at in 0..4 {
        write_task(&copying.plan, at, &format!("Edited {at}."));
    }
    copying.workspace.cap_origin_lookup_page(4);
    (copying, big)
}

#[test]
fn a_projects_page_with_more_behind_it_answers_no_project_origin_and_the_project_is_walked_for() {
    let (copying, big) = over_a_projects_page_with_more_behind_it();
    let served = copying.ok(&COPY);
    let before = &served[..first_write(&served)];
    let asked = batched(before);
    assert_eq!(asked.len(), 1, "{before:#?}");
    assert_projects_half(&copying, asked[0]);
    assert!(
        single_issue_lookups(&served).is_empty(),
        "the complete issues page answered every task's origin: {served:#?}"
    );
    assert!(
        count(before, graphql::PROJECTS) >= 1,
        "the project's origin, unanswered by a page with more behind it, is walked for on its \
         own before anything is written: {before:#?}"
    );
    assert_eq!(
        count(&served, graphql::PROJECT_CREATE),
        0,
        "no project is created for a counterpart the page omitted"
    );
    assert_eq!(
        count(&served, graphql::ISSUE_CREATE),
        0,
        "no task created again"
    );
    let after = listed(&copying, "project");
    assert_eq!(
        after.len(),
        7,
        "the workspace holds the projects it held: {after:#?}"
    );
    let updated = after
        .iter()
        .find(|project| project["id"] == big["id"])
        .unwrap_or_else(|| panic!("the plan's project is held: {after:#?}"));
    assert_eq!(
        updated["content"], "The plan, edited.",
        "updated in place: {updated}"
    );
}

/// Run the copy over [`over_a_projects_page_with_more_behind_it`] with the walk's `projects`
/// listings bent by `bend` `times` times, which has to be refused naming `refusal` before any
/// write and leave the workspace's projects and tasks as they were.
fn refused_walk(
    times: usize,
    bend: impl FnMut(&Value, &mut Value) + Send + 'static,
    refusal: &str,
) {
    let (copying, _) = over_a_projects_page_with_more_behind_it();
    let tasks = shown(&listed(&copying, "task"));
    let projects = shown(&listed(&copying, "project"));
    copying
        .workspace
        .bend_answers(|query, _| query == graphql::PROJECTS, times, bend);
    let (output, served) = copying.run(&COPY);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(stderr(&output).contains(refusal), "{}", stderr(&output));
    assert!(
        !served
            .iter()
            .any(|(query, _)| query.trim_start().starts_with("mutation")),
        "nothing is written: {served:#?}"
    );
    assert_eq!(shown(&listed(&copying, "task")), tasks);
    assert_eq!(shown(&listed(&copying, "project")), projects);
}

#[test]
fn a_walk_answered_with_more_projects_than_its_page_asked_for_is_refused_writing_nothing() {
    refused_walk(
        1,
        |variables, data| {
            let first = variables["first"].as_u64().expect("a page size");
            let nodes = data["projects"]["nodes"].as_array_mut().expect("projects");
            let row = nodes[0].clone();
            while nodes.len() as u64 <= first {
                nodes.push(row.clone());
            }
        },
        "rows for a page of at most",
    );
}

#[test]
fn a_walk_handed_a_cursor_it_already_handed_out_is_refused_writing_nothing() {
    refused_walk(
        2,
        |_, data| {
            data["projects"]["pageInfo"] = json!({"hasNextPage": true, "endCursor": "again"});
        },
        "a cursor it had already handed out",
    );
}
