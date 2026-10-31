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
//! of each changed item followed by a read pinned to what a listing after it carries. An image's
//! bytes are read only by a render keeping it, so that kind's four are renders instead: a first
//! render, a second with the picture unchanged, one after the document was edited, and one after
//! a render that wrote it.
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
use onetaskgraph_github_projects::{DESIGN_TITLE_PREFIX, DETAIL_BATCH, worst_case_point_cost};
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
/// The four sequences of the asset-bytes kind, whose bytes only a render reads, so each is a
/// render rather than a pinned load: a first render, a second with the picture unchanged, one
/// after the document was edited, and one after a render that wrote it.
pub(crate) const RENDER_SEQUENCES: [&str; 4] = [
    "cold-render",
    "unchanged-render",
    "render-after-edits",
    "render-after-render",
];

/// The sequences `kind` is measured through, in the order measured.
pub(crate) const fn sequences_of(kind: &str) -> [&'static str; 4] {
    if matches!(kind.as_bytes(), b"asset-bytes") {
        RENDER_SEQUENCES
    } else {
        SEQUENCES
    }
}
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
    /// The same board's ledger behind a listener of its own, which the second of two concurrent
    /// passes reaches as clock client 1: two live processes may not share one client.
    second: Board,
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
        // Two clients: the second of the two concurrent passes runs as client 1, because a second
        // live process attaching as client 0 waits out the reattach grace and is then refused.
        let clock = SimulatedClock::start(2);
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
                &format!("{DESIGN_TITLE_PREFIX}Budget document {n}"),
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
        let upstream = config["endpoint"].as_str().unwrap().to_owned();
        let board = Board::new(&upstream, &clock, 0, None);
        let second = Board::new(&upstream, &clock, 1, Some(&board));
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
            second,
            clock,
            cache_home: PathBuf::new(),
            scale,
            template,
            imaged: Vec::new(),
        };
        workload.cache_home = workload.sandbox.subdirectory("cache-home");
        // Virtual time stands still until both clients have attached once, so client 1 attaches
        // before anything could wait on it rather than first in the concurrent passes.
        let (effective, _) = workload.run_as(1, &["config", "show", "--json"]);
        assert!(
            effective.to_string().contains(&workload.second.endpoint),
            "client 1 reaches the board through its own listener: {effective}"
        );
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

    /// Where the board's and the attachment host's ledgers stand now.
    fn mark(&self) -> (usize, usize) {
        (self.github.served_count(), self.board.calls_count())
    }

    /// Every GraphQL request and attachment read the ledgers took since `mark`, each once.
    fn spent_since(&self, (documents, calls): (usize, usize)) -> Spent {
        let served = &self.github.served_since(documents);
        Spent {
            requests: served.len() as u64,
            points: served
                .iter()
                .map(|(document, _)| {
                    worst_case_point_cost(document)
                        .expect("every document this source sends prices")
                })
                .sum(),
            hits: 0,
            rest: self
                .board
                .calls_since(calls)
                .iter()
                .filter(|call| call.path.starts_with("/user-attachments/assets/"))
                .count() as u64,
        }
    }

    /// One command that had to succeed: what it printed and what the ledgers took while it ran
    /// — its own spend, as long as no other command runs beside it.
    fn run(&self, arguments: &[&str]) -> (Value, Spent) {
        self.run_as(0, arguments)
    }

    /// [`run`](Self::run) as clock client `client`, through that client's own listener.
    fn run_as(&self, client: usize, arguments: &[&str]) -> (Value, Spent) {
        let mark = self.mark();
        let endpoint = [&self.board, &self.second][client].endpoint.as_str();
        let output = self
            .sandbox
            .command()
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("ONETASKGRAPH_SOURCES__BOARD__CONFIG__ENDPOINT", endpoint)
            .envs(self.clock.client_env(client))
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
        (
            serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null),
            self.spent_since(mark),
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
        self.load_as(0, kind, pins)
    }

    /// [`load`](Self::load) as clock client `client`.
    fn load_as(
        &self,
        client: usize,
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
            // Whole detail batches per invocation, so the batched requests are the ones a single
            // invocation sends, and the 10x pass's pinned ids stay under Windows' 32,767-character
            // command line (`os error 206`).
            for chunk in ids.chunks(2 * DETAIL_BATCH) {
                let mut arguments = vec!["task".to_owned(), "show-many".to_owned()];
                arguments.extend(chunk.iter().cloned());
                for id in chunk {
                    if let Some(version) = pin_of(id) {
                        arguments.extend(["--pin".to_owned(), format!("{id}={version}")]);
                    }
                }
                arguments.push("--json".to_owned());
                let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
                let (read, cost) = self.run_as(client, &arguments);
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
            let (read, cost) = self.run_as(client, &arguments);
            // Said by the answer itself, so it holds while another pass runs beside this one.
            if read["cache"]["served"] == "cache" {
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
                _ => self.github.edit_title(
                    native,
                    &format!("{DESIGN_TITLE_PREFIX}{native}, changed by a person"),
                ),
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
            let mut rendered_again = self.render("written");
            rendered_again.add(self.render("read again"));
            return vec![cold, unchanged, after, rendered_again];
        }
        let loads = if kind == "manifests" { "imaged" } else { kind };
        let (cold, held) = self.load(loads, None);
        // Two passes at once, each pinned to what the cold load returned. Each command's slice
        // of the ledgers would hold the other's requests too, so the two are measured as one
        // span, every request once; the hits are what each answer said of itself.
        let mark = self.mark();
        let held = held.as_slice();
        let hits: u64 = std::thread::scope(|scope| {
            // Each pass is a clock client of its own, so neither's process is refused an attach.
            let passes: Vec<_> = (0..2)
                .map(|client| scope.spawn(move || self.load_as(client, loads, Some(held)).0.hits))
                .collect();
            passes
                .into_iter()
                .map(|pass| pass.join().expect("a pass"))
                .sum()
        });
        let unchanged = Spent {
            hits,
            ..self.spent_since(mark)
        };
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

/// One scale and cache mode's sequences, by kind: its scale's name, whether the cache was on,
/// and each kind's four sequences in [`KINDS`] order.
type Run = (&'static str, bool, Vec<Vec<Spent>>);

/// One kind's four sequences at one scale, with the cache on or off, on a board of its own: its
/// cold sequence reads nothing another kind kept, and every kind of every scale runs at once.
fn measured(scale: Scale, cache: bool, kind: &str) -> Vec<Spent> {
    let sequences = Workload::new(scale, cache).sequences(kind);
    if !cache && kind != "asset-bytes" {
        // With nothing kept, each of the two concurrent passes makes exactly the cold load's
        // reads, so a span that counted any request twice — or missed one — reads otherwise.
        let (cold, unchanged) = (sequences[0], sequences[1]);
        assert_eq!(
            (unchanged.requests, unchanged.points),
            (2 * cold.requests, 2 * cold.points),
            "{kind}: two concurrent passes are counted once each"
        );
    }
    sequences
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
    // Every cell of the grid at once — each scale, cache mode and kind on its own board — so
    // the test takes as long as its slowest kind rather than every kind of a scale in turn.
    let (runs, stale): (Vec<Run>, Value) = std::thread::scope(|scope| {
        let handles: Vec<_> = SCALES
            .iter()
            .flat_map(|(name, factor)| {
                [false, true].map(|cache| {
                    let scale = Scale { factor: *factor };
                    let kinds: Vec<_> = KINDS
                        .iter()
                        .map(|kind| scope.spawn(move || measured(scale, cache, kind)))
                        .collect();
                    (*name, cache, kinds)
                })
            })
            .collect();
        let stale = [true, false].map(|cache| scope.spawn(move || stale_option(cache)));
        let runs = handles
            .into_iter()
            .map(|(name, cache, kinds)| {
                let kinds = kinds.into_iter().map(|kind| kind.join().expect("a run"));
                (name, cache, kinds.collect())
            })
            .collect();
        let [on, off] = stale.map(|run| run.join().expect("a stale-option run"));
        (runs, json!({"on": on, "off": off}))
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
            for (s, sequence) in sequences_of(kind).iter().enumerate() {
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
