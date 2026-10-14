//! What a Linear source's cache spends of the shared key's allowance against what the same hour
//! spends without it, recorded as telemetry for the `linear-cache-cost-not-above-uncached` and
//! `linear-cache-largest-query-complexity` budgets.
//!
//! The workload is the budget's, as ruling c-6bed6c20 assumes it: three hosts on one key, each
//! running two passes an hour over 10 projects, 40 documents and 60 issues — 15 of them carrying
//! one to five comments — where a pass lists all three kinds and loads 3 projects with their
//! forward edges, 10 documents and the 15 issues with their comments and forward edges; each
//! host's first pass is cold, and its second warm after 3 of the loaded issues changed; and the
//! same at ten times the size, loads and changes included. A pass pins each load to the version
//! its listing carried, as a host holding that listing does. It is driven through the binary
//! against the loopback Linear once with the cache off — the documents 0.6.0 sends — and once
//! with it on, each over a workspace of its own, and every request is priced from the
//! complexity table below.
//!
//! The Linear key's limit it reaches first is requests (2,500 an hour per user), so the cost
//! budget's figure is the requests the cache saves: the smaller, over the two scales, of the
//! hour's requests with the cache off minus the hour's with it on, which passes only at one or
//! more — the cache sending strictly fewer requests than no cache at both scales. Complexity is held apart, per query: the guard's figure is the
//! dearest single query any read sent with the cache on, at either scale. The detail also
//! reports an all-warm hour — each host's two passes both warm, each after the same changes — as
//! a host whose cache an earlier hour filled runs.

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};
use crate::linear::{eng_mapping, held, issue, linear};
use crate::telemetry;

/// The budget this suite records: the hour's requests the cache saves, cache off minus cache on.
pub(crate) const BUDGET: &str = "linear-cache-cost-not-above-uncached";
/// The guard it records beside it: the dearest single query any read sent with the cache on.
pub(crate) const GUARD: &str = "linear-cache-largest-query-complexity";
/// The workload it records, which the reader holds the recording to.
pub(crate) const WORKLOAD: &str =
    "three-hosts-two-passes-10-projects-40-documents-60-issues-at-1x-and-10x";

/// Linear's complexity for each document this workload sends, as `spike-linear-coverage-report`
/// scored them in three live sessions — each with the page it was scored at, because Linear
/// prices some connections by the rows a response returns and others by the `first` a request
/// names.
// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The documents these prices are keyed on are gated: `cache_documents.rs` holds every one byte for byte to 0.6.0's or to exactly the cache's widening, so a changed query fails there before it could carry a stale price. The prices themselves are Linear's own scores from three live sessions; re-taking them needs a Linear credential, which this suite runs without by design, and the spec fixes this table as its source.
pub(crate) mod price {
    /// `VIEWER`.
    pub(crate) const VIEWER: u64 = 1;
    /// `ISSUE`.
    pub(crate) const ISSUE: u64 = 6;
    /// `ISSUE_DETAIL`, at `last:100`.
    pub(crate) const ISSUE_DETAIL: u64 = 11;
    /// `ISSUE_COMMENTS`, at `last:100`.
    pub(crate) const ISSUE_COMMENTS: u64 = 6;
    /// `ISSUES`: its base, modelled from one-row pages.
    pub(crate) const ISSUES_BASE: u64 = 8;
    /// `ISSUES`: what each row it returns adds to [`ISSUES_BASE`].
    pub(crate) const ISSUES_ROW: u64 = 6;
    /// `ISSUE_RELATIONS` and `PROJECT_RELATIONS`, at `first:100`.
    pub(crate) const RELATIONS_AT_100: u64 = 522;
    /// `ISSUE_RELATIONS_CACHED`, carrying a `lin1` token's other parts, at `first:100`.
    pub(crate) const RELATIONS_CACHED_AT_100: u64 = 525;
    /// `PROJECT`.
    pub(crate) const PROJECT: u64 = 69;
    /// `PROJECTS`, each row it asks for.
    pub(crate) const PROJECTS_ROW: u64 = 70;
    /// `DOCUMENT`.
    pub(crate) const DOCUMENT: u64 = 2;
    /// `DOCUMENTS`, each row it asks for.
    pub(crate) const DOCUMENTS_ROW: u64 = 3;
    /// `LABELS`, at `first:5`.
    pub(crate) const LABELS_AT_5: u64 = 13;
    /// `RESOLUTION`.
    pub(crate) const RESOLUTION: u64 = 2053;
    /// The relations page a `lin1` token's relations part is read from, measured on `ISSUE` and
    /// taken as the same connection on `ISSUE_DETAIL` and on each `ISSUES` row.
    pub(crate) const RELATIONS_PAGE: u64 = 25;
    /// The inverse relations page a `lin1` token's inverse part is read from. Not priced by the
    /// spike: taken as [`RELATIONS_PAGE`], the same connection of the same size and selection
    /// with `issue{id}` in place of `relatedIssue{id}`.
    pub(crate) const INVERSE_RELATIONS_PAGE: u64 = RELATIONS_PAGE;
    /// The comments page a `lin1` token's comments part is read from — `+C`, measured on each
    /// `ISSUES` row — taken as the same connection on `ISSUE` and on `ISSUE_RELATIONS_CACHED`.
    /// `ISSUE_DETAIL_CACHED` spells that part from the comments page it already reads.
    pub(crate) const COMMENTS_PAGE: u64 = 3;
    /// The relations page a project's `lin1` token's relations part is read from — `+R`,
    /// measured on each `PROJECTS` row asked for — taken as the same connection on `PROJECT`.
    pub(crate) const PROJECT_RELATIONS_PAGE: u64 = 25;
    /// Linear's per-query complexity cap.
    pub(crate) const CAP: u64 = 10_000;
    /// The guard on any one query this workload sends with the cache on: 90% of [`CAP`], the
    /// threshold `budgets.yaml` gives the guard's budget, which `budget_commands.rs` holds it to.
    pub(crate) const GUARD: u64 = 9_000;
}

/// What one request cost, priced from [`price`]: by the rows `answer` returned for a connection
/// Linear prices that way, and by the `first` `variables` name for one it prices by what was
/// asked — a relations read scaled linearly in that `first` through its measured `first:100`.
pub(crate) fn complexity(document: &str, variables: &Value, answer: &Value) -> u64 {
    use onetaskgraph_linear::graphql as g;
    let first = |name: &str| variables[name].as_u64().unwrap_or(50);
    let rows = |pointer: &str| {
        answer
            .pointer(pointer)
            .and_then(Value::as_array)
            .map_or(0, Vec::len) as u64
    };
    let relations = |at_100: u64| (at_100 * first("first")).div_ceil(100);
    match document {
        g::VIEWER => price::VIEWER,
        g::ISSUE => price::ISSUE,
        g::ISSUE_CACHED => {
            price::ISSUE
                + price::RELATIONS_PAGE
                + price::INVERSE_RELATIONS_PAGE
                + price::COMMENTS_PAGE
        }
        g::ISSUE_DETAIL => price::ISSUE_DETAIL,
        g::ISSUE_DETAIL_CACHED => {
            price::ISSUE_DETAIL + price::RELATIONS_PAGE + price::INVERSE_RELATIONS_PAGE
        }
        g::ISSUE_COMMENTS => price::ISSUE_COMMENTS,
        g::ISSUES => price::ISSUES_BASE + price::ISSUES_ROW * rows("/data/issues/nodes"),
        g::ISSUES_CACHED => {
            price::ISSUES_BASE
                + (price::ISSUES_ROW
                    + price::RELATIONS_PAGE
                    + price::INVERSE_RELATIONS_PAGE
                    + price::COMMENTS_PAGE)
                    * rows("/data/issues/nodes")
        }
        g::ISSUE_RELATIONS | g::PROJECT_RELATIONS => relations(price::RELATIONS_AT_100),
        g::ISSUE_RELATIONS_CACHED => {
            relations(price::RELATIONS_CACHED_AT_100) + price::COMMENTS_PAGE
        }
        // A project's token parts beside its relations, taken as the issue's measured three.
        g::PROJECT_RELATIONS_CACHED => relations(price::RELATIONS_CACHED_AT_100),
        g::PROJECT => price::PROJECT,
        g::PROJECT_CACHED => price::PROJECT + price::PROJECT_RELATIONS_PAGE,
        g::PROJECTS => price::PROJECTS_ROW * first("first"),
        g::PROJECTS_CACHED => {
            (price::PROJECTS_ROW + price::PROJECT_RELATIONS_PAGE) * first("first")
        }
        g::DOCUMENT => price::DOCUMENT,
        g::DOCUMENTS => price::DOCUMENTS_ROW * first("first"),
        g::LABELS => (price::LABELS_AT_5 * first("first")).div_ceil(5),
        g::RESOLUTION => price::RESOLUTION,
        other => panic!("the workload sent a document the table does not price: {other}"),
    }
}

/// One scale of the workload.
#[derive(Clone, Copy)]
struct Scale {
    factor: usize,
}

impl Scale {
    fn projects(self) -> usize {
        10 * self.factor
    }
    fn documents(self) -> usize {
        40 * self.factor
    }
    fn issues(self) -> usize {
        60 * self.factor
    }
    fn loaded_projects(self) -> usize {
        3 * self.factor
    }
    fn loaded_documents(self) -> usize {
        10 * self.factor
    }
    fn loaded_issues(self) -> usize {
        15 * self.factor
    }
    fn changed(self) -> usize {
        3 * self.factor
    }
}

/// What part of a pass spent: requests, complexity, and the dearest one request.
#[derive(Default, Clone, Copy)]
struct Spent {
    requests: u64,
    complexity: u64,
    dearest: u64,
}

impl Spent {
    fn add(&mut self, more: Self) {
        self.requests += more.requests;
        self.complexity += more.complexity;
        self.dearest = self.dearest.max(more.dearest);
    }

    fn json(self) -> Value {
        json!({"requests": self.requests, "complexity": self.complexity,
               "dearest": self.dearest})
    }
}

/// One host's hour: what each part of its cold pass and of its warm pass spent.
type Hour = Vec<[Vec<Spent>; 2]>;

/// The parts of a pass, in the order it makes them.
const PARTS: [&str; 4] = ["listings", "projects", "documents", "issues"];

/// A loopback Linear workspace of one scale, with the cache on or off.
struct Workload {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    scale: Scale,
}

fn issue_id(n: usize) -> String {
    format!("BI-{n}")
}
fn project_id(n: usize) -> String {
    format!("BP-{n}")
}
fn document_id(n: usize) -> String {
    format!("BD-{n}")
}

impl Workload {
    fn new(scale: Scale, cache: bool) -> Self {
        let sandbox = Sandbox::new();
        let issues = (0..scale.issues())
            .map(|n| {
                let mut extra = json!({"project": project_id(n % scale.projects())});
                if n < scale.loaded_issues() {
                    extra["metadata"] = json!({"budget.pass": "loaded"});
                }
                issue(&issue_id(n), "Todo", extra)
            })
            .collect();
        let projects = (0..scale.projects())
            .map(|n| {
                json!({"id": project_id(n), "title": format!("Budget project {n}"),
                       "content": format!("About {n}."),
                       "status": {"category": "in-progress", "name": "In Progress"},
                       "labels": []})
            })
            .collect();
        let documents = (0..scale.documents())
            .map(|n| {
                json!({"id": document_id(n), "title": format!("Budget document {n}"),
                       "content": format!("Document {n}."),
                       "project": project_id(n % scale.projects())})
            })
            .collect();
        let mut held = held(issues, projects, documents);
        // Each loaded issue depends on the next, and each loaded project on the next.
        held["task_dependencies"] = Value::Array(
            (0..scale.loaded_issues())
                .map(|n| json!({"from": issue_id(n), "to": issue_id(n + 1), "kind": "blocks"}))
                .collect(),
        );
        held["project_dependencies"] = Value::Array(
            (0..scale.loaded_projects())
                .map(|n| json!({"from": project_id(n), "to": project_id(n + 1), "kind": "blocks"}))
                .collect(),
        );
        let (config, workspace) = linear_workspace(&sandbox, held);
        for n in 0..scale.loaded_issues() {
            for comment in 0..=(n % 5) {
                workspace.add_comment(&issue_id(n), &format!("comment {comment} on {n}"));
            }
        }
        let mut source = linear(&config, json!({"status_mapping": eng_mapping()}));
        source["cache"] = json!({"enabled": cache});
        let notes = sandbox.subdirectory("notes");
        for kind in ["tasks", "projects", "documents"] {
            std::fs::create_dir_all(notes.join(kind)).expect("a folder");
        }
        sandbox.project_document(&document(&json!({
            "linear": source,
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        })));
        Self {
            sandbox,
            workspace,
            scale,
        }
    }

    /// One command of host `host`, which had to succeed: what it printed and what it spent.
    fn run(&self, host: usize, arguments: &[&str]) -> (Value, Spent) {
        let before = self.workspace.served().len();
        let output = self
            .sandbox
            .command()
            .env(
                "XDG_CACHE_HOME",
                self.sandbox.subdirectory(&format!("host-{host}")),
            )
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        let served = self.workspace.served();
        let answered = self.workspace.answered();
        let mut spent = Spent::default();
        for ((document, variables), answer) in served[before..].iter().zip(&answered[before..]) {
            let cost = complexity(document, variables, answer);
            spent.add(Spent {
                requests: 1,
                complexity: cost,
                dearest: cost,
            });
        }
        (
            serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null),
            spent,
        )
    }

    /// Every page of `<kind> list`: what it spent, and the version it carried for each item.
    fn list(&self, host: usize, kind: &str) -> (Spent, Vec<(String, String)>) {
        let mut spent = Spent::default();
        let mut versions = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut arguments = vec![
                kind, "list", "--source", "linear", "--limit", "100", "--json",
            ];
            if let Some(token) = &page {
                arguments.extend(["--page", token]);
            }
            let (listing, cost) = self.run(host, &arguments);
            spent.add(cost);
            for item in listing["items"].as_array().expect("a listing") {
                if let Some(version) = item["item"]["version"].as_str() {
                    versions.push((item["id"].as_str().unwrap().to_owned(), version.to_owned()));
                }
            }
            match listing["next"].as_str() {
                Some(next) => page = Some(next.to_owned()),
                None => return (spent, versions),
            }
        }
    }

    /// One pass of host `host`: what each of its parts spent.
    fn pass(&self, host: usize) -> Vec<Spent> {
        let mut listings = Spent::default();
        let (cost, projects) = self.list(host, "project");
        listings.add(cost);
        let (cost, documents) = self.list(host, "document");
        listings.add(cost);
        listings.add(self.list(host, "task").0);
        let pin = |versions: &[(String, String)], id: &str| {
            versions
                .iter()
                .find(|(listed, _)| listed == id)
                .map(|(_, version)| version.clone())
                .unwrap_or_else(|| panic!("the listing carries {id}"))
        };

        let mut loaded_projects = Spent::default();
        for n in 0..self.scale.loaded_projects() {
            let id = format!("linear:{}", project_id(n));
            let version = pin(&projects, &id);
            loaded_projects.add(
                self.run(host, &["project", "show", &id, "--pin", &version, "--json"])
                    .1,
            );
            loaded_projects.add(
                self.run(host, &["project", "deps", &id, "--pin", &version, "--json"])
                    .1,
            );
        }
        let mut loaded_documents = Spent::default();
        for n in 0..self.scale.loaded_documents() {
            let id = format!("linear:{}", document_id(n));
            let version = pin(&documents, &id);
            loaded_documents.add(
                self.run(
                    host,
                    &["document", "show", &id, "--pin", &version, "--json"],
                )
                .1,
            );
        }
        // The loaded issues with their comments and forward edges, as a copy reads them: listed
        // and read in the one command, which reads everything and writes nothing.
        let (_, loaded_issues) = self.run(
            host,
            &[
                "task",
                "copy",
                "--to",
                "notes",
                "--source",
                "linear",
                "--metadata",
                "budget.pass=loaded",
                "--comments",
                "--dry-run",
                "--no-link",
                "--json",
            ],
        );
        vec![listings, loaded_projects, loaded_documents, loaded_issues]
    }

    /// The changes a person makes before host `host`'s pass `pass` of `hour`: the first few
    /// loaded issues retitled.
    fn change(&self, hour: &str, host: usize, pass: usize) {
        for n in 0..self.scale.changed() {
            self.workspace.retitle(
                "tasks",
                &issue_id(n),
                &format!("Changed in the {hour} hour, host {host}, pass {pass}"),
            );
        }
    }

    /// The hour — each host's cold pass, then its warm pass after the changes between them —
    /// and the all-warm hour after it, each host's two passes each after the same changes.
    fn hours(&self) -> (Hour, Hour) {
        let first = (0..3)
            .map(|host| {
                let cold = self.pass(host);
                self.change("first", host, 1);
                let warm = self.pass(host);
                [cold, warm]
            })
            .collect();
        let warm = (0..3)
            .map(|host| {
                self.change("all-warm", host, 0);
                let one = self.pass(host);
                self.change("all-warm", host, 1);
                let two = self.pass(host);
                [one, two]
            })
            .collect();
        (first, warm)
    }
}

/// Every part of every pass of the hour, summed by part and pass, and the hour's total.
fn summed(hour: &[[Vec<Spent>; 2]]) -> (Vec<[Spent; 2]>, Spent) {
    let mut parts = vec![[Spent::default(); 2]; PARTS.len()];
    let mut total = Spent::default();
    for host in hour {
        for (pass, measured) in host.iter().enumerate() {
            for (part, spent) in measured.iter().enumerate() {
                parts[part][pass].add(*spent);
                total.add(*spent);
            }
        }
    }
    (parts, total)
}

// llmlint: ignore-block[budgets_reuse_gate_telemetry] The budget compares the same hour with the cache on and with it off, which no behavioural journey runs — the cache's journeys hold small workspaces to exact requests — and its task fixes the design: this journey records both modes through the binary, and the shared runner only reads what it recorded.
#[test]
fn the_cache_sends_fewer_of_the_linear_keys_requests_in_an_hour_than_no_cache() {
    let runs: Vec<(usize, bool, (Hour, Hour))> = std::thread::scope(|scope| {
        let handles: Vec<_> = [1, 10]
            .into_iter()
            .flat_map(|factor| {
                [false, true].map(|cache| {
                    (
                        factor,
                        cache,
                        scope.spawn(move || Workload::new(Scale { factor }, cache).hours()),
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|(factor, cache, handle)| (factor, cache, handle.join().expect("an hour")))
            .collect()
    });
    let mut figure = i64::MAX;
    let mut largest = 0;
    let mut scales = serde_json::Map::new();
    for factor in [1, 10] {
        let of = |cache: bool| {
            &runs
                .iter()
                .find(|(at, on, _)| *at == factor && *on == cache)
                .expect("both modes")
                .2
        };
        let (off_parts, off) = summed(&of(false).0);
        let (on_parts, on) = summed(&of(true).0);
        let (_, off_warm) = summed(&of(false).1);
        let (_, on_warm) = summed(&of(true).1);
        let saved = i64::try_from(off.requests).unwrap() - i64::try_from(on.requests).unwrap();
        figure = figure.min(saved);
        let dearest = on.dearest.max(on_warm.dearest);
        largest = largest.max(dearest);
        let mut parts = serde_json::Map::new();
        for (index, part) in PARTS.iter().enumerate() {
            parts.insert(
                (*part).to_owned(),
                json!({
                    "cold": {"on": on_parts[index][0].json(), "off": off_parts[index][0].json()},
                    "warm": {"on": on_parts[index][1].json(), "off": off_parts[index][1].json()},
                }),
            );
        }
        scales.insert(
            format!("{factor}x"),
            json!({
                "parts": parts,
                "hour": {"on": on.json(), "off": off.json(), "saved": saved},
                "requests_against_2500": {"on": on.requests, "off": off.requests},
                "complexity": {"on": on.complexity, "off": off.complexity},
                "all_warm_hour": {
                    "on": on_warm.json(), "off": off_warm.json(),
                    "requests_against_2500": {"on": on_warm.requests, "off": off_warm.requests},
                    "complexity": {"on": on_warm.complexity, "off": off_warm.complexity},
                },
                "largest_single_query": {"on": dearest,
                                          "off": off.dearest.max(off_warm.dearest),
                                          "cap": price::CAP, "guard": price::GUARD},
            }),
        );
    }
    let scale = |factor: &str, pointer: &str| scales[factor].pointer(pointer).cloned();
    let detail = format!(
        "the requests the cache saves: the smaller, over 1x and 10x, of one hour's Linear \
         requests with the cache off minus with it on — three hosts, each a cold and a warm pass \
         — {figure}, passing only at 1 or more; the hour's requests \
         against the key's 2,500, on and off: 1x {} and {}, 10x {} and {}; an all-warm hour's, \
         each host's two passes warm after the same changes: 1x {} and {}, 10x {} and {}; the \
         hour's complexity on and off, priced from spike-linear-coverage-report's table: 1x {} \
         and {}, 10x {} and {}, and the all-warm hour's: 1x {} and {}, 10x {} and {}; by scale, \
         part and pass: {}",
        scale("1x", "/hour/on/requests").unwrap_or_default(),
        scale("1x", "/hour/off/requests").unwrap_or_default(),
        scale("10x", "/hour/on/requests").unwrap_or_default(),
        scale("10x", "/hour/off/requests").unwrap_or_default(),
        scale("1x", "/all_warm_hour/on/requests").unwrap_or_default(),
        scale("1x", "/all_warm_hour/off/requests").unwrap_or_default(),
        scale("10x", "/all_warm_hour/on/requests").unwrap_or_default(),
        scale("10x", "/all_warm_hour/off/requests").unwrap_or_default(),
        scale("1x", "/complexity/on").unwrap_or_default(),
        scale("1x", "/complexity/off").unwrap_or_default(),
        scale("10x", "/complexity/on").unwrap_or_default(),
        scale("10x", "/complexity/off").unwrap_or_default(),
        scale("1x", "/all_warm_hour/complexity/on").unwrap_or_default(),
        scale("1x", "/all_warm_hour/complexity/off").unwrap_or_default(),
        scale("10x", "/all_warm_hour/complexity/on").unwrap_or_default(),
        scale("10x", "/all_warm_hour/complexity/off").unwrap_or_default(),
        Value::Object(scales.clone())
    );
    let guard = format!(
        "the dearest single Linear query any cached or remote read of the hour and the all-warm \
         hour sent with the cache on, over 1x and 10x, priced from \
         spike-linear-coverage-report's table: {largest}, against a guard of {} — 90% of \
         Linear's per-query cap of {}; 1x {}, 10x {}",
        price::GUARD,
        price::CAP,
        scale("1x", "/largest_single_query/on").unwrap_or_default(),
        scale("10x", "/largest_single_query/on").unwrap_or_default(),
    );
    let directory = telemetry::directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    for (budget, value, detail) in [
        (BUDGET, json!(figure), detail),
        (GUARD, json!(largest), guard),
    ] {
        std::fs::write(
            telemetry::file_in(&directory, budget),
            json!({"budget": budget, "value": value, "detail": detail, "workload": WORKLOAD,
                   "scales": Value::Object(scales.clone())})
            .to_string(),
        )
        .expect("the telemetry is written");
    }
}
// llmlint: ignore-end[budgets_reuse_gate_telemetry]
