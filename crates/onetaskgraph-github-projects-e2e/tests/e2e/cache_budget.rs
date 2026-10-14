//! What the board cache spends against what the same work spends without it, recorded as
//! telemetry for the `github-cache-points-not-above-uncached` budget.
//!
//! The workload is the budget's: a loopback board of 20 projects, 100 documents and 80 tasks —
//! 20 of them carrying one to five comments — of which a pass loads 5 projects, 20 documents and
//! 20 tasks, with 5 changed between passes and 8 of the loaded documents carrying one 256 KiB
//! image; and the same at ten times the size. Each kind is driven through the binary once with
//! the cache off and once with it on, each over a board of its own, through four sequences: a
//! cold load; an unchanged reread pinned to what the cold load returned, made by two passes at
//! once; a listing followed by the loads it pins, after five of each kind changed; and a write
//! of each changed item followed by a read pinned to what a listing after it carries. The
//! image's bytes are read by a render keeping it, which is what reads them.
//!
//! Points are the plugin's own worst-case price of each GraphQL document the board served; an
//! attachment read is a REST request and spends no point, so the detail counts them apart. The
//! figure is the largest difference, cache on minus cache off, over every cell — scale, kind and
//! sequence — and every scale's total, so points added to one kind show however much another
//! saves. The stale-option fallback, the cache's one named exception, is measured beside them
//! and kept out of the figure.

use std::path::PathBuf;

use onetaskgraph_e2e_support::{
    clock::SimulatedClock,
    common::{Sandbox, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
    images,
};
use onetaskgraph_github_projects::worst_case_point_cost;
use serde_json::{Value, json};

use super::asset_board::Board;
use crate::budget_runner::directory;
use crate::cache::{Cached, version_of, written_version};

/// The budget this suite records.
pub(crate) const BUDGET: &str = "github-cache-points-not-above-uncached";
/// The workload it records, which the reporter holds the recording to.
pub(crate) const WORKLOAD: &str = "board-20-projects-100-documents-80-tasks-8-images-at-1x-and-10x";
/// The four sequences, in the order measured.
pub(crate) const SEQUENCES: [&str; 4] = [
    "cold",
    "unchanged-pinned-reread",
    "list-then-load-after-edits",
    "write-then-read",
];
/// The kinds each sequence is measured for.
pub(crate) const KINDS: [&str; 5] = ["tasks", "projects", "documents", "manifests", "asset-bytes"];
/// The two scales, by name and factor.
pub(crate) const SCALES: [(&str, usize); 2] = [("1x", 1), ("10x", 10)];

/// One scale of the workload.
#[derive(Clone, Copy)]
struct Scale {
    factor: usize,
}

impl Scale {
    fn projects(self) -> usize {
        20 * self.factor
    }
    fn documents(self) -> usize {
        100 * self.factor
    }
    fn tasks(self) -> usize {
        80 * self.factor
    }
    fn commented(self) -> usize {
        20 * self.factor
    }
    fn loaded_projects(self) -> usize {
        5 * self.factor
    }
    fn loaded_documents(self) -> usize {
        20 * self.factor
    }
    fn images(self) -> usize {
        8 * self.factor
    }
    fn changed(self) -> usize {
        5 * self.factor
    }
}

/// What one sequence of one kind spent: GraphQL requests and their points, the loads it
/// answered with no request, and its attachment reads.
#[derive(Default, Clone, Copy)]
struct Spent {
    requests: u64,
    points: u64,
    hits: u64,
    rest: u64,
}

impl Spent {
    fn add(&mut self, more: Self) {
        self.requests += more.requests;
        self.points += more.points;
        self.hits += more.hits;
        self.rest += more.rest;
    }

    fn json(self) -> Value {
        json!({"requests": self.requests, "points": self.points, "hits": self.hits,
               "rest": self.rest})
    }
}

/// A board of one scale, with the cache on or off, behind its attachment host.
struct Workload {
    sandbox: Sandbox,
    github: GitHubBoardFields,
    board: Board,
    clock: SimulatedClock,
    cache_home: PathBuf,
    scale: Scale,
    template: PathBuf,
    /// The board's ids of the documents rendered with an image.
    imaged: Vec<String>,
}

fn project_id(n: usize) -> String {
    format!("BP-{n}")
}
fn document_id(n: usize) -> String {
    format!("BD-{n}")
}
fn task_id(n: usize) -> String {
    format!("BT-{n}")
}

/// One board item, spelled as the shared dataset spells one, its slot holding `slot`.
fn item(id: &str, title: &str, slot: &Value) -> Value {
    let body = if slot.as_object().is_some_and(serde_json::Map::is_empty) {
        format!("the body of {id}")
    } else {
        format!("the body of {id}\n\n<!-- onetaskgraph.metadata\n{slot}\n-->")
    };
    json!({"item": format!("ITEM-{id}"), "id": id, "type": "Issue", "title": title,
           "body": body, "state": "OPEN", "reason": null, "parent": null,
           "repo": "nickderobertis/onetaskgraph", "status": "Todo", "origin": "",
           "labels": [["L-1", "bug"]]})
}

/// A template rendering a line of text and one picture, `./shot.png`.
const PICTURE: &str = "---\nonetaskgraph_template: 1\nvariables:\n  note:\n    description: What \
                       changed\n    type: string\n---\n# The change\n{{ note }}\n![shot](./shot.png)\n";

impl Workload {
    fn new(scale: Scale, cache: bool) -> Self {
        let clock = SimulatedClock::start(1);
        let sandbox = Sandbox::new();
        let mut items = Vec::new();
        for n in 0..scale.projects() {
            items.push(item(
                &project_id(n),
                &format!("Budget project {n}"),
                &json!({"onetaskgraph.item_kind": "project"}),
            ));
        }
        // The images' documents are rendered below; these are the rest.
        for n in 0..scale.documents() - scale.images() {
            items.push(item(
                &document_id(n),
                &format!("DESIGN: Budget document {n}"),
                &json!({}),
            ));
        }
        for n in 0..scale.tasks() {
            let slot = if n < scale.commented() {
                json!({"budget.pass": "loaded"})
            } else {
                json!({})
            };
            items.push(item(&task_id(n), &format!("Budget task {n}"), &slot));
        }
        let (mut config, github) = github_projects_with_items(&sandbox, items);
        let board = Board::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
        config["endpoint"] = json!(board.endpoint);
        config["pacing"] = json!({});
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": config,
                      "cache": {"enabled": cache}},
        })));
        for n in 0..scale.commented() {
            for comment in 0..=(n % 5) {
                github.add_comment(&task_id(n), &format!("comment {comment} on {n}"));
            }
        }
        let template = sandbox.project().join("picture.md");
        std::fs::write(&template, PICTURE).unwrap();
        let mut workload = Self {
            sandbox,
            github,
            board,
            clock,
            cache_home: PathBuf::new(),
            scale,
            template,
            imaged: Vec::new(),
        };
        workload.cache_home = workload.sandbox.subdirectory("cache-home");
        for n in 0..scale.images() {
            let directory = workload.sandbox.subdirectory(&format!("image-{n}"));
            std::fs::create_dir_all(&directory).unwrap();
            let shot = directory.join("shot.png");
            std::fs::write(&shot, images::png(n as u64, 256 * 1024)).unwrap();
            let template = workload.template.display().to_string();
            let shot = shot.display().to_string();
            let (created, _) = workload.run(&[
                "document",
                "create",
                "board",
                "--project",
                "P-1",
                "--title",
                &format!("Imaged {n}"),
                "--template",
                &template,
                "--var",
                "note=created",
                "--no-interactive",
                "--asset",
                &shot,
                "--json",
            ]);
            workload.imaged.push(
                created["items"][0]["id"]
                    .as_str()
                    .expect("the imaged document")
                    .to_owned(),
            );
        }
        // Nothing the setup read is kept: every sequence starts from what it alone did.
        let _ = std::fs::remove_dir_all(&workload.cache_home);
        workload
    }

    /// One command that had to succeed: what it printed and what it spent.
    fn run(&self, arguments: &[&str]) -> (Value, Spent) {
        let (documents, calls) = (self.github.served().len(), self.board.calls().len());
        let output = self
            .sandbox
            .command()
            .env("XDG_CACHE_HOME", &self.cache_home)
            .envs(self.clock.client_env(0))
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
        let served = &self.github.served()[documents..];
        let spent = Spent {
            requests: served.len() as u64,
            points: served
                .iter()
                .map(|(document, _)| {
                    worst_case_point_cost(document)
                        .expect("every document this source sends prices")
                })
                .sum(),
            hits: 0,
            rest: self.board.calls()[calls..]
                .iter()
                .filter(|call| call.path.starts_with("/user-attachments/assets/"))
                .count() as u64,
        };
        (
            serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null),
            spent,
        )
    }

    /// The ids of the items of `kind` a pass loads, qualified.
    fn loaded(&self, kind: &str) -> Vec<String> {
        let ids: Vec<String> = match kind {
            "tasks" => (0..self.scale.commented()).map(task_id).collect(),
            "projects" => (0..self.scale.loaded_projects()).map(project_id).collect(),
            "documents" => (0..self.scale.loaded_documents() - self.scale.images())
                .map(document_id)
                .collect(),
            _ => self
                .imaged
                .iter()
                .map(|id| id.strip_prefix("board:").unwrap().to_owned())
                .collect(),
        };
        ids.into_iter().map(|id| format!("board:{id}")).collect()
    }

    /// Load every item of `kind` a pass loads, each pinned to `pins`'s version for it when
    /// given: what it spent, and each one's version.
    fn load(
        &self,
        kind: &str,
        pins: Option<&[(String, String)]>,
    ) -> (Spent, Vec<(String, String)>) {
        let ids = self.loaded(kind);
        let pin_of = |id: &str| {
            pins.and_then(|pins| pins.iter().find(|(pinned, _)| pinned == id))
                .map(|(_, version)| version.clone())
        };
        let mut spent = Spent::default();
        let mut versions = Vec::new();
        if kind == "tasks" {
            let mut arguments = vec!["task".to_owned(), "show-many".to_owned()];
            arguments.extend(ids.iter().cloned());
            for id in &ids {
                if let Some(version) = pin_of(id) {
                    arguments.extend(["--pin".to_owned(), format!("{id}={version}")]);
                }
            }
            arguments.push("--json".to_owned());
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            let (read, cost) = self.run(&arguments);
            spent.add(cost);
            for detail in read["details"].as_array().expect("details") {
                let task = &detail["items"][0];
                if detail["cache"]["served"] == "cache" {
                    spent.hits += 1;
                }
                versions.push((
                    task["id"].as_str().unwrap().to_owned(),
                    task["item"]["version"].as_str().unwrap().to_owned(),
                ));
            }
            return (spent, versions);
        }
        let verb = if kind == "projects" {
            "project"
        } else {
            "document"
        };
        for id in ids {
            let mut arguments = vec![verb.to_owned(), "show".to_owned(), id.clone()];
            if let Some(version) = pin_of(&id) {
                arguments.extend(["--pin".to_owned(), version]);
            }
            arguments.push("--json".to_owned());
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            let (read, cost) = self.run(&arguments);
            if cost.requests == 0 {
                spent.hits += 1;
            }
            spent.add(cost);
            versions.push((id, version_of(&read)));
        }
        (spent, versions)
    }

    /// Render every imaged document again keeping its picture: what reads the picture's bytes.
    fn render(&self, note: &str) -> Spent {
        let mut spent = Spent::default();
        for id in &self.imaged {
            let note = format!("note={note}");
            let (_, cost) = self.run(&[
                "document",
                "render",
                id,
                "--var",
                &note,
                "--no-interactive",
                "--json",
            ]);
            if cost.rest == 0 {
                spent.hits += 1;
            }
            spent.add(cost);
        }
        spent
    }

    /// The version a listing of `kind` carries for each item a pass loads.
    fn listed(&self, kind: &str) -> (Spent, Vec<(String, String)>) {
        let arguments: Vec<&str> = match kind {
            "tasks" => vec![
                "task",
                "list",
                "--source",
                "board",
                "--metadata",
                "budget.pass=loaded",
                "--limit",
                "100",
                "--json",
            ],
            "projects" => vec![
                "project", "list", "--source", "board", "--limit", "100", "--json",
            ],
            _ => vec![
                "document", "list", "--source", "board", "--limit", "100", "--json",
            ],
        };
        let mut spent = Spent::default();
        let mut versions = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut walking: Vec<&str> = arguments.clone();
            if let Some(token) = &page {
                walking.extend(["--page", token]);
            }
            let (listing, cost) = self.run(&walking);
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

    /// Change the first `changed` items of `kind` a pass loads, as a person would: a task's
    /// body, and a project's or a document's title — the document's keeping the prefix that
    /// makes it one, and the project's body keeping the marker that makes it one.
    fn edit(&self, kind: &str) {
        for id in self.loaded(kind).iter().take(self.scale.changed()) {
            let native = id.strip_prefix("board:").unwrap();
            match kind {
                "tasks" => self
                    .github
                    .edit_body(native, &format!("{native}, changed by a person")),
                "projects" => self
                    .github
                    .edit_title(native, &format!("{native}, changed by a person")),
                _ => self
                    .github
                    .edit_title(native, &format!("DESIGN: {native}, changed by a person")),
            }
        }
    }

    /// Write the first `changed` items of `kind` a pass loads through the binary.
    fn write(&self, kind: &str) -> Spent {
        let mut spent = Spent::default();
        for id in self.loaded(kind).iter().take(self.scale.changed()) {
            let arguments: Vec<&str> = match kind {
                "tasks" => vec!["task", "status", "set", id, "in-progress", "--json"],
                "projects" => vec![
                    "project",
                    "metadata",
                    "set",
                    id,
                    "budget.note",
                    "\"w\"",
                    "--json",
                ],
                _ => vec![
                    "document",
                    "metadata",
                    "set",
                    id,
                    "budget.note",
                    "\"w\"",
                    "--json",
                ],
            };
            spent.add(self.run(&arguments).1);
        }
        spent
    }

    /// The four sequences of `kind` on this board: what each spent.
    fn sequences(&self, kind: &str) -> Vec<Spent> {
        if kind == "asset-bytes" {
            let cold = self.render("cold");
            let unchanged = self.render("unchanged");
            self.edit("imaged");
            let after = self.render("after edits");
            let mut written = self.render("written");
            written.add(self.render("read again"));
            return vec![cold, unchanged, after, written];
        }
        let loads = if kind == "manifests" { "imaged" } else { kind };
        let (cold, held) = self.load(loads, None);
        // Two passes at once, each pinned to what the cold load returned.
        let unchanged = std::thread::scope(|scope| {
            let passes: Vec<_> = (0..2)
                .map(|_| scope.spawn(|| self.load(loads, Some(&held)).0))
                .collect();
            let mut spent = Spent::default();
            for pass in passes {
                spent.add(pass.join().expect("a pass"));
            }
            spent
        });
        self.edit(loads);
        let listing_kind = match kind {
            "tasks" | "projects" => kind,
            _ => "documents",
        };
        let (mut after, listed) = self.listed(listing_kind);
        after.add(self.load(loads, Some(&listed)).0);
        let mut written = self.write(loads);
        let (listing, listed) = self.listed(listing_kind);
        written.add(listing);
        written.add(self.load(loads, Some(&listed)).0);
        vec![cold, unchanged, after, written]
    }
}

/// Every kind's four sequences at one scale, with the cache on or off, on one board: each kind's
/// cold sequence reads none of the entries another kind's sequences kept.
fn measured(scale: Scale, cache: bool) -> Vec<Vec<Spent>> {
    let workload = Workload::new(scale, cache);
    KINDS.iter().map(|kind| workload.sequences(kind)).collect()
}

/// The stale-option fallback, with the cache on and off: a priority write after the option it
/// names was re-minted.
fn stale_option(cache: bool) -> Value {
    let cached = Cached::with(&json!({"enabled": cache}));
    let (first, _) = cached.ok(&["task", "show", "board:T-3", "--json"]);
    let version = version_of(&first);
    cached.board.remint_priority_option("High");
    let before = cached.board.served().len();
    let mut arguments = vec!["task", "priority", "set", "board:T-3", "high", "--json"];
    if cache {
        arguments.extend(["--expect", &version]);
    }
    let (written, _) = cached.ok(&arguments);
    if cache {
        let _ = written_version(&written);
    }
    let served = &cached.board.served()[before..];
    let points: u64 = served
        .iter()
        .map(|(document, _)| worst_case_point_cost(document).expect("priced"))
        .sum();
    json!({"requests": served.len(), "points": points})
}

// llmlint: ignore-block[budgets_reuse_gate_telemetry] The budget compares the same workload with the cache on and with it off, which no behavioural journey runs — the cache's journeys hold small boards to exact requests, cache on alone — and its task fixes the design: the e2e journeys record the four sequences in both modes, and the reporter only reads what they recorded.
#[test]
fn the_cache_spends_no_more_board_points_than_no_cache_over_every_kind_sequence_and_scale() {
    let runs: Vec<(&str, bool, Vec<Vec<Spent>>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = SCALES
            .iter()
            .flat_map(|(name, factor)| {
                [false, true].map(|cache| {
                    let scale = Scale { factor: *factor };
                    (*name, cache, scope.spawn(move || measured(scale, cache)))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|(name, cache, handle)| (name, cache, handle.join().expect("a run")))
            .collect()
    });
    let mut figure = i64::MIN;
    let mut scales = serde_json::Map::new();
    for (name, _) in SCALES {
        let of = |cache: bool| {
            &runs
                .iter()
                .find(|(scale, on, _)| *scale == name && *on == cache)
                .expect("both runs")
                .2
        };
        let (off, on) = (of(false), of(true));
        let mut kinds = serde_json::Map::new();
        let (mut total_off, mut total_on) = (Spent::default(), Spent::default());
        for (k, kind) in KINDS.iter().enumerate() {
            let mut cells = serde_json::Map::new();
            for (s, sequence) in SEQUENCES.iter().enumerate() {
                let difference = i64::try_from(on[k][s].points).unwrap()
                    - i64::try_from(off[k][s].points).unwrap();
                figure = figure.max(difference);
                total_off.add(off[k][s]);
                total_on.add(on[k][s]);
                cells.insert(
                    (*sequence).to_owned(),
                    json!({"on": on[k][s].json(), "off": off[k][s].json(),
                           "difference": difference}),
                );
            }
            kinds.insert((*kind).to_owned(), Value::Object(cells));
        }
        let total =
            i64::try_from(total_on.points).unwrap() - i64::try_from(total_off.points).unwrap();
        figure = figure.max(total);
        scales.insert(
            name.to_owned(),
            json!({"kinds": kinds, "total": {"on": total_on.json(), "off": total_off.json(),
                                             "difference": total}}),
        );
    }
    let stale = json!({"on": stale_option(true), "off": stale_option(false)});
    let detail = format!(
        "largest difference, cache on minus cache off, in GitHub GraphQL points over every \
         kind, sequence and scale and each scale's total: {figure}; by scale, kind and \
         sequence, requests, points, loads answered with no request and attachment reads on \
         and off: {}; the stale-option fallback, the cache's one named exception and outside \
         the figure: {stale}",
        Value::Object(scales.clone())
    );
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({
            "budget": BUDGET,
            "value": figure,
            "detail": detail,
            "workload": WORKLOAD,
            "scales": Value::Object(scales),
            "stale_option": stale,
        }))
        .expect("JSON"),
    )
    .expect("the telemetry is written");
}
// llmlint: ignore-end[budgets_reuse_gate_telemetry]
