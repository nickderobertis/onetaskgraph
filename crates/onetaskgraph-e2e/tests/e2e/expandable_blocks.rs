//! Expandable blocks — `<details>` with a one-line `<summary>` — copied into each destination's
//! own form, as README "Expandable blocks" states each one.
//!
//! Every expected form here is spelled out from the contract rather than computed by the code
//! under test: each authored summary beside the Linear summary the contract derives from it,
//! written literally. The loopback Linear stores what it is sent as spike
//! `spike-expandable-blocks` recorded real Linear storing it (`LinearMarkdown::Measured`, which
//! `onetaskgraph_e2e_support::linear_normalizations` holds to every recorded fixture).

use std::collections::BTreeMap;
use std::path::Path;

use onetaskgraph_e2e_support::fixtures::LinearMarkdown;
use serde_json::{Value, json};

use crate::common::Sandbox;
use crate::fixtures::empty_folder;
use crate::section_links::{Landed, Store, configure, ok, outcomes, root_of, shown, store};

/// Every kind a block is copied between.
const KINDS: [&str; 3] = ["local-md", "github-projects", "linear"];

/// One block's summary in the authored form, and in Linear's form as the contract derives it:
/// `<code>` as backticks, `<strong>` as `**`, and each entity as its character.
type Summary = (&'static str, &'static str);

/// The three blocks every record body holds, in order.
const SUMMARIES: [Summary; 3] = [
    (
        "Run <code>copy --dry-run</code> first",
        "Run `copy --dry-run` first",
    ),
    (
        "<strong>Decision</strong>: keep the cache",
        "**Decision**: keep the cache",
    ),
    (
        "Costs &amp; limits &lt;per hour&gt;",
        "Costs & limits <per hour>",
    ),
];

/// The block a carried comment holds.
const COMMENT_SUMMARY: Summary = (
    "Why <code>--comments</code> &amp; not more",
    "Why `--comments` & not more",
);

/// One authored block, numbered `n` within its record, whose body holds a paragraph, a list, a
/// heading, a fenced code block and a table.
fn block(summary: &str, n: usize) -> String {
    format!(
        "<details>\n<summary>{summary}</summary>\n\nParagraph of block {n}.\n\n- first item of \
         block {n}\n- second item of block {n}\n\n### Detail of block {n}\n\n```text\ncopy \
         --dry-run {n}\n```\n\n| Budget | Limit |\n| --- | --- |\n| reads {n} | 27 |\n\n</details>"
    )
}

/// A record body: a line naming it, then the three blocks.
fn record_body(what: &str) -> String {
    let blocks: Vec<String> = SUMMARIES
        .iter()
        .enumerate()
        .map(|(index, (summary, _))| block(summary, index + 1))
        .collect();
    format!("The {what}.\n\n{}\n", blocks.join("\n\n"))
}

/// The carried comment's body.
fn comment_body() -> String {
    format!("A comment.\n\n{}\n", block(COMMENT_SUMMARY.0, 4))
}

/// Write the authored project into a folder named `name` of the sandbox — its description, a
/// task carrying a comment and a design document, each holding the three blocks — answering the
/// source configuration naming it.
fn authored(sandbox: &Sandbox, name: &str) -> Value {
    let config = empty_folder(sandbox, name);
    let root = root_of(sandbox, name);
    for folder in ["projects", "tasks", "documents"] {
        std::fs::create_dir_all(root.join(folder)).expect("a folder");
    }
    let task = format!(
        "{}\n## Comments\n\n<!-- onetaskgraph:comment id=\"c1\" created_at=\"2026-11-02T12:00:00Z\" \
         updated_at=\"2026-11-02T12:00:00Z\" -->\n### comment — 2026-11-02T12:00:00Z\n\n{}\n\
         <!-- /onetaskgraph:comment -->\n",
        record_body("task"),
        comment_body()
    );
    for (path, front, body) in [
        (
            "projects/P.md",
            "title: Block plan\nstatus: todo",
            record_body("project"),
        ),
        (
            "tasks/T.md",
            "title: Blocks\nstatus: todo\nproject: P",
            task,
        ),
        (
            "documents/D.md",
            "title: Block design\nproject: P",
            record_body("document"),
        ),
    ] {
        std::fs::write(root.join(path), format!("---\n{front}\n---\n{body}")).expect("a record");
    }
    json!({"plugin": "local-md", "config": config})
}

/// The authored records of the folder `name`, by their keys.
fn authored_ids(name: &str) -> BTreeMap<&'static str, Landed> {
    [("P", "project"), ("T", "task"), ("D", "document")]
        .into_iter()
        .map(|(key, verb)| {
            (
                key,
                Landed {
                    verb,
                    id: format!("{name}:{key}"),
                },
            )
        })
        .collect()
}

/// Copy the project `from["P"]` with its task and the task's comments, and then its document
/// `from["D"]`, into `to`, answering where each landed, keyed as `from` is.
fn copy_plan(
    sandbox: &Sandbox,
    from: &BTreeMap<&'static str, Landed>,
    to: &str,
) -> BTreeMap<&'static str, Landed> {
    let project = outcomes(&ok(
        sandbox,
        &[
            "project",
            "copy",
            &from["P"].id,
            "--to",
            to,
            "--comments",
            "--json",
        ],
    ));
    let document = outcomes(&ok(
        sandbox,
        &["document", "copy", &from["D"].id, "--to", to, "--json"],
    ));
    from.iter()
        .map(|(key, source)| {
            let report = if source.verb == "document" {
                &document
            } else {
                &project
            };
            let outcome = report
                .get(&source.id)
                .unwrap_or_else(|| panic!("{} was copied: {report:#?}", source.id));
            let id = outcome["destination"]
                .as_str()
                .unwrap_or_else(|| panic!("{} landed: {outcome:#}", source.id))
                .to_owned();
            (
                *key,
                Landed {
                    verb: source.verb,
                    id,
                },
            )
        })
        .collect()
}

/// A store of `kind` the journeys copy into: as `section_links::store` makes one, with a Linear
/// workspace storing what it is sent as real Linear was recorded storing it.
fn destination(
    sandbox: &Sandbox,
    kind: &str,
    folder: &str,
    beside: Option<&Store>,
) -> (Store, Value) {
    let (held, config) = store(sandbox, kind, folder, beside);
    if let Store::Linear(workspace) = &held {
        workspace.rewrite_markdown_on_write(LinearMarkdown::Measured);
    }
    (held, config)
}

/// Every body `store` holds for the records `landed` names, as the store itself holds it, by
/// what it is: `P`, `T`, `D`, and — when the copy `carried` comments — `comment` for the one
/// carried on `T`.
fn raw_bodies(
    sandbox: &Sandbox,
    store: &Store,
    landed: &BTreeMap<&'static str, Landed>,
    carried: bool,
) -> BTreeMap<&'static str, String> {
    let mut bodies = BTreeMap::new();
    match store {
        Store::Local => {
            for key in ["P", "T", "D"] {
                let file = std::fs::read_to_string(landed[key].location(sandbox))
                    .expect("the record's file");
                // A task's file holds its comments after its content.
                match file.split_once("\n## Comments\n") {
                    Some((content, comments)) => {
                        bodies.insert(key, content.to_owned());
                        bodies.insert("comment", comments.to_owned());
                    }
                    None => {
                        bodies.insert(key, file);
                    }
                }
            }
        }
        Store::Github(board) => {
            for key in ["P", "T", "D"] {
                bodies.insert(key, store.raw(&landed[key]));
            }
            if carried {
                let comments = board.comments_on(landed["T"].native());
                bodies.insert("comment", only_comment(&comments));
            }
        }
        Store::Linear(workspace) => {
            for key in ["P", "T", "D"] {
                bodies.insert(key, store.raw(&landed[key]));
            }
            if carried {
                let comments = workspace.comments_on(landed["T"].native());
                bodies.insert("comment", only_comment(&comments));
            }
        }
    }
    bodies
}

/// The body of the one comment `comments` holds.
fn only_comment(comments: &[Value]) -> String {
    assert_eq!(comments.len(), 1, "one comment was carried: {comments:#?}");
    comments[0]["body"]
        .as_str()
        .expect("a comment's body")
        .to_owned()
}

/// The lines of the block whose opening lines are `opening`, through the line before `closing`:
/// the body, its blank margins taken off, after asserting each margin is one blank line.
fn block_lines<'a>(
    lines: &[&'a str],
    opening: &[String],
    closing: &str,
    what: &str,
) -> Vec<&'a str> {
    let start = lines
        .windows(opening.len())
        .position(|window| window.iter().zip(opening).all(|(line, open)| *line == open))
        .unwrap_or_else(|| panic!("{what}: holds {opening:?}:\n{}", lines.join("\n")));
    let body_start = start + opening.len();
    let end = body_start
        + lines[body_start..]
            .iter()
            .position(|line| *line == closing)
            .unwrap_or_else(|| panic!("{what}: {opening:?} is closed by {closing:?}"));
    assert_eq!(
        lines[end - 1],
        "",
        "{what}: one blank line before {closing:?}"
    );
    assert_ne!(lines[end - 2], "", "{what}: and only one");
    assert_ne!(
        lines[body_start], "",
        "{what}: one blank line after the summary"
    );
    lines[body_start..end - 1].to_vec()
}

/// Hold `raw`, a body `kind` holds, to holding block `n` with `summary` in that kind's form: the
/// authored form on a folder of Markdown and a GitHub issue — on a GitHub issue's body with the
/// anchor the source writes before the heading inside it — and Linear's collapsible section, its summary
/// translated, on Linear; its body holding the paragraph, the list, the heading, the fenced code
/// and the table it was authored with.
fn holds_block(kind: &str, raw: &str, summary: Summary, n: usize, what: &str) {
    let lines: Vec<&str> = raw.lines().collect();
    let what = format!("{what}, block {n}");
    let body = if kind == "linear" {
        block_lines(
            &lines,
            &[format!("+++ {}", summary.1), String::new()],
            "+++",
            &what,
        )
    } else {
        block_lines(
            &lines,
            &[
                "<details>".to_owned(),
                format!("<summary>{}</summary>", summary.0),
                String::new(),
            ],
            "</details>",
            &what,
        )
    };
    let has = |line: &str| body.contains(&line);
    assert!(
        has(&format!("Paragraph of block {n}.")),
        "{what}: {body:#?}"
    );
    assert!(
        has(&format!("- first item of block {n}")) || has(&format!("* first item of block {n}")),
        "{what}: {body:#?}"
    );
    let heading = format!("### Detail of block {n}");
    let at = body
        .iter()
        .position(|line| *line == heading)
        .unwrap_or_else(|| panic!("{what}: holds {heading}: {body:#?}"));
    // The GitHub source writes its anchors into an item's body, never into a comment.
    if kind == "github-projects" && n != 4 {
        assert_eq!(
            body[at - 1],
            format!("<a id=\"detail-of-block-{n}\"></a>"),
            "{what}: the anchor the GitHub source writes before a heading reaches one in a block"
        );
    }
    assert!(has("```text"), "{what}: {body:#?}");
    assert!(has(&format!("copy --dry-run {n}")), "{what}: {body:#?}");
    assert!(has("| Budget | Limit |"), "{what}: {body:#?}");
    assert!(has(&format!("| reads {n} | 27 |")), "{what}: {body:#?}");
}

/// Hold every body `store`, of `kind`, holds for `landed` to holding each of its blocks in that
/// kind's form — and, on Linear, to holding no line of the authored form's markup.
fn holds_the_form(
    sandbox: &Sandbox,
    what: &str,
    kind: &str,
    store: &Store,
    landed: &BTreeMap<&'static str, Landed>,
) {
    for (key, raw) in raw_bodies(sandbox, store, landed, true) {
        let what = format!("{what}: {key}");
        if key == "comment" {
            holds_block(kind, &raw, COMMENT_SUMMARY, 4, &what);
        } else {
            for (index, summary) in SUMMARIES.into_iter().enumerate() {
                holds_block(kind, &raw, summary, index + 1, &what);
            }
        }
        if kind == "linear" {
            for line in raw.lines() {
                assert!(
                    !(line.starts_with("<details") || line.starts_with("<summary>"))
                        && line != "</details>",
                    "{what}: no authored markup is left on Linear: {line:?}\n{raw}"
                );
            }
        }
    }
}

/// Every record `landed` names, as `show` reads its metadata back, `onetaskgraph.copied_version`
/// taken out, beside that version.
fn records(
    sandbox: &Sandbox,
    landed: &BTreeMap<&'static str, Landed>,
) -> BTreeMap<&'static str, (Value, Value)> {
    landed
        .iter()
        .map(|(key, record)| {
            let mut metadata = shown(sandbox, record.verb, &record.id)["metadata"].clone();
            let version = metadata
                .as_object_mut()
                .and_then(|held| held.remove("onetaskgraph.copied_version"))
                .unwrap_or(Value::Null);
            (*key, (metadata, version))
        })
        .collect()
}

/// Repeat a copy, `repeat`, into `store` twice — `root` the folder's, for a folder of Markdown.
///
/// The first repeat leaves every body `landed` names byte for byte as it was, changes none of
/// their metadata but `onetaskgraph.copied_version`, and moves that on exactly the records
/// `restamped` names: a re-stamp of the copy record, because the source version that record
/// names is one the source has since moved past — a GitHub source answers a link write with the
/// change token from before it, and a `--comments` task copy's source moves after the copy read
/// it. The second repeat writes nothing at all.
fn repeats_settling(
    sandbox: &Sandbox,
    what: &str,
    repeat: &dyn Fn(),
    (store, root): (&Store, Option<&Path>),
    (landed, carried): (&BTreeMap<&'static str, Landed>, bool),
    restamped: &[&str],
) {
    let content = |bodies: BTreeMap<&'static str, String>| -> BTreeMap<&'static str, String> {
        bodies
            .into_iter()
            .map(|(key, body)| {
                // What is under the copy record: a file's front matter, a hosted body's slot.
                let body = if matches!(store, Store::Local) {
                    body.splitn(3, "---\n")
                        .nth(2)
                        .unwrap_or_default()
                        .to_owned()
                } else {
                    body
                };
                (key, body)
            })
            .collect()
    };
    let (bodies, held) = (
        content(raw_bodies(sandbox, store, landed, carried)),
        records(sandbox, landed),
    );
    repeat();
    let (again, now) = (
        content(raw_bodies(sandbox, store, landed, carried)),
        records(sandbox, landed),
    );
    assert_eq!(again, bodies, "{what}: the first repeat changes no body");
    let mut moved = Vec::new();
    for (key, (metadata, version)) in &held {
        assert_eq!(
            &now[key].0, metadata,
            "{what}: {key}'s metadata is unchanged"
        );
        if &now[key].1 != version {
            moved.push(*key);
        }
    }
    assert_eq!(
        moved, restamped,
        "{what}: the first repeat re-stamps the copy record of exactly these"
    );
    let mark = store.mark(root);
    repeat();
    assert_eq!(
        store.writes_since(&mark, root),
        Vec::<String>::new(),
        "{what}: the second repeat writes nothing"
    );
}

/// The records whose copy record the first repeat of a `--comments` copy of the plan from `from`
/// into `to` re-stamps, as measured: the task, but into Linear from a folder of Markdown, and
/// the project and the document too from a board into Linear.
fn restamped(from: &str, to: &str) -> &'static [&'static str] {
    match (from, to) {
        ("local-md", "linear") => &[],
        ("github-projects", "linear") => &["D", "P", "T"],
        _ => &["T"],
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: folders of Markdown and the loopback stand-ins this suite already drives for both hosted destinations. AGENTS.md puts a journey crossing more than one plugin in this suite.
#[test]
fn every_pair_of_kinds_writes_each_block_in_the_destinations_form_and_repeats_write_nothing() {
    let mut pairs = Vec::new();
    for from in KINDS {
        for to in KINDS {
            let what = format!("{from} -> {to}");
            let sandbox = Sandbox::new();
            let authored_config = authored(&sandbox, "authored");
            let mut sources = vec![("authored", authored_config)];
            let (source, ids) = if from == "local-md" {
                (Store::Local, authored_ids("authored"))
            } else {
                let (source, config) = destination(&sandbox, from, "src", None);
                sources.push(("src", config));
                (source, BTreeMap::new())
            };
            let (dst, config) = destination(&sandbox, to, "dst", Some(&source));
            sources.push(("dst", config));
            let named: Vec<(&str, &Value)> = sources
                .iter()
                .map(|(name, config)| (*name, config))
                .collect();
            configure(&sandbox, &named);
            let ids = if from == "local-md" {
                ids
            } else {
                // The source holds the project as a copy from a folder of Markdown wrote it.
                let seeded = copy_plan(&sandbox, &authored_ids("authored"), "src");
                holds_the_form(
                    &sandbox,
                    &format!("{what} (seeded)"),
                    from,
                    &source,
                    &seeded,
                );
                seeded
            };

            let landed = copy_plan(&sandbox, &ids, "dst");
            holds_the_form(&sandbox, &what, to, &dst, &landed);
            let root = (to == "local-md").then(|| root_of(&sandbox, "dst"));
            repeats_settling(
                &sandbox,
                &what,
                &|| {
                    let again = copy_plan(&sandbox, &ids, "dst");
                    for (key, record) in &landed {
                        assert_eq!(again[key].id, record.id, "{what}: {key} landed once");
                    }
                },
                (&dst, root.as_deref()),
                (&landed, true),
                restamped(from, to),
            );
            pairs.push(what);
        }
    }
    // The matrix, stated once: every ordered pair, same-kind pairs included.
    assert_eq!(pairs.len(), 9, "{pairs:?}");
}

/// The lines of `text` from the `<details>` line opening the block whose summary starts
/// `Probe <probe>:` through the `</details>` line closing it, blocks inside it counted.
fn probe_region(text: &str, probe: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let summary = lines
        .iter()
        .position(|line| line.starts_with(&format!("<summary>Probe {probe}:")))
        .unwrap_or_else(|| panic!("probe {probe} is recorded"));
    let start = summary - 1;
    assert_eq!(lines[start], "<details>", "probe {probe} opens a block");
    let mut depth = 0_usize;
    for (at, line) in lines.iter().enumerate().skip(start) {
        if line.starts_with("<details") {
            depth += 1;
        } else if *line == "</details>" {
            depth -= 1;
            if depth == 0 {
                return lines[start..=at].join("\n");
            }
        }
    }
    panic!("probe {probe} is closed")
}

/// What one shape the authored form does not recognise is carried in: the record the journey
/// writes it into, its authored Markdown, and what the loopback Linear is to store for it — the
/// recorded fixture's stored Markdown where the spike recorded that shape, and `None` for a shape
/// it did not record, which the recorded normalizations are to account for instead.
struct Shape {
    what: &'static str,
    verb: &'static str,
    key: &'static str,
    authored: String,
    recorded: Option<String>,
}

/// Every shape the contract says is not the authored form, and the three summaries whose Linear
/// form would not read back as themselves.
fn shapes() -> Vec<Shape> {
    use onetaskgraph_e2e_support::linear_normalizations::fixture;
    let whole = |what, verb, key, name: &str| {
        let recorded = fixture(name);
        Shape {
            what,
            verb,
            key,
            authored: recorded.sent,
            recorded: Some(recorded.stored),
        }
    };
    let authored = fixture("issue-authored");
    let region = |what, key, probe: &str| Shape {
        what,
        verb: "task",
        key,
        authored: probe_region(&authored.sent, probe),
        recorded: Some(probe_region(&authored.stored, probe)),
    };
    let unrecorded = |what, key, authored: String| Shape {
        what,
        verb: "task",
        key,
        authored,
        recorded: None,
    };
    let summarised = |summary: &str| {
        format!(
            "<details>\n<summary>{summary}</summary>\n\nA paragraph of its body.\n\n</details>\n"
        )
    };
    vec![
        whole("<details open>", "task", "OPEN", "issue-details-open"),
        whole(
            "<details open>",
            "document",
            "DOPEN",
            "document-details-open",
        ),
        whole(
            "no blank line after </summary>",
            "task",
            "TIGHT",
            "issue-no-blank-after-summary",
        ),
        whole(
            "no blank line after </summary>",
            "document",
            "DTIGHT",
            "document-no-blank-after-summary",
        ),
        whole(
            "inside a fenced block",
            "task",
            "FENCED",
            "issue-in-fenced-code",
        ),
        whole(
            "inside a fenced block",
            "document",
            "DFENCED",
            "document-in-fenced-code",
        ),
        whole(
            "indented under a list item",
            "task",
            "INDENTED",
            "issue-in-list-item",
        ),
        whole(
            "indented under a list item",
            "document",
            "DINDENTED",
            "document-in-list-item",
        ),
        unrecorded(
            "a nested block missing a blank line",
            "NESTED",
            "<details>\n<summary>Outer</summary>\n\nOuter text.\n\n<details>\n<summary>Inner\
             </summary>\nInner text.\n\n</details>\n\n</details>\n"
                .to_owned(),
        ),
        unrecorded(
            "a nested summary holding another tag",
            "NESTAG",
            "<details>\n<summary>Outer</summary>\n\nOuter text.\n\n<details>\n<summary>Uses \
             <em>emphasis</em></summary>\n\nInner text.\n\n</details>\n\n</details>\n"
                .to_owned(),
        ),
        region("a summary holding a literal backtick", "TICK", "A03"),
        region("a summary holding a literal *", "STAR", "A05"),
        unrecorded(
            "a summary holding another tag",
            "TAG",
            summarised("Uses <em>emphasis</em> here"),
        ),
        unrecorded(
            "a summary holding a raw &",
            "AMP",
            summarised("Questions & answers"),
        ),
        unrecorded("an empty summary", "EMPTY", summarised("")),
        unrecorded("an empty code span", "SPAN", summarised("Empty <code></code> span")),
        unrecorded(
            "a strong span nested in a code span",
            "INNER",
            summarised("<code>a <strong>b</strong></code>"),
        ),
        unrecorded("an unclosed code span", "UNCLOSED", summarised("Open <code>span")),
        unrecorded("a code span holding a *", "MARK", summarised("<code>a*b</code>")),
        unrecorded("a summary opening with a space", "LEAD", summarised(" Padded")),
        unrecorded("a summary ending with a space", "TRAIL", summarised("Padded ")),
        unrecorded(
            "a block written with carriage returns",
            "CRLF",
            "<details>\r\n<summary>Returns</summary>\r\n\r\nA paragraph of its body.\r\n\r\n</details>\r\n"
                .to_owned(),
        ),
        unrecorded(
            "no blank line before </details>",
            "LAST",
            "<details>\n<summary>Ends tight</summary>\n\nA paragraph of its body.\n</details>\n"
                .to_owned(),
        ),
        unrecorded(
            "an empty body",
            "BARE",
            "<details>\n<summary>Holds nothing</summary>\n\n</details>\n".to_owned(),
        ),
    ]
}

/// The visible part of a long form a hosted store holds or was sent: what comes before its
/// metadata slot — a board's comment, or Linear's last collapsed block — without the line breaks
/// that end it.
fn visible(long_form: &str) -> &str {
    let long_form = long_form
        .rsplit_once("\n\n+++ onetaskgraph metadata\n\n")
        .map_or(long_form, |(above, _)| above);
    long_form
        .split("<!-- onetaskgraph.metadata")
        .next()
        .unwrap_or_default()
        .trim_end_matches(['\r', '\n'])
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown copied into a second folder, the loopback board and the loopback Linear workspace.
#[test]
fn a_shape_that_is_not_the_authored_form_is_never_translated_and_linear_stores_it_as_recorded() {
    let shapes = shapes();
    for kind in KINDS {
        let sandbox = Sandbox::new();
        let config = json!({"plugin": "local-md", "config": empty_folder(&sandbox, "authored")});
        let root = root_of(&sandbox, "authored");
        for folder in ["projects", "tasks", "documents"] {
            std::fs::create_dir_all(root.join(folder)).expect("a folder");
        }
        std::fs::write(
            root.join("projects/P.md"),
            "---\ntitle: Shapes\nstatus: todo\n---\nShapes the authored form does not recognise.\n",
        )
        .expect("the project");
        for shape in &shapes {
            let (folder, front) = if shape.verb == "task" {
                ("tasks", "status: todo\nproject: P")
            } else {
                ("documents", "project: P")
            };
            std::fs::write(
                root.join(folder).join(format!("{}.md", shape.key)),
                format!(
                    "---\ntitle: Shape {}\n{front}\n---\n{}",
                    shape.key, shape.authored
                ),
            )
            .expect("a shape's record");
        }
        let (dst, dst_config) = destination(&sandbox, kind, "dst", None);
        configure(&sandbox, &[("authored", &config), ("dst", &dst_config)]);
        let mut landed = outcomes(&ok(
            &sandbox,
            &["project", "copy", "authored:P", "--to", "dst", "--json"],
        ));
        for shape in shapes.iter().filter(|shape| shape.verb == "document") {
            let id = format!("authored:{}", shape.key);
            landed.extend(outcomes(&ok(
                &sandbox,
                &["document", "copy", &id, "--to", "dst", "--json"],
            )));
        }
        for shape in &shapes {
            let what = format!("{kind}: {} ({})", shape.what, shape.key);
            let at = Landed {
                verb: shape.verb,
                id: landed[&format!("authored:{}", shape.key)]["destination"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{what}: landed"))
                    .to_owned(),
            };
            let authored = shape.authored.trim_end_matches(['\r', '\n']);
            match &dst {
                // Byte for byte: read back, and as the folder or the board itself holds it.
                Store::Local => {
                    assert_eq!(
                        at.content(&sandbox).trim_end_matches(['\r', '\n']),
                        authored,
                        "{what}"
                    );
                    let file = std::fs::read_to_string(at.location(&sandbox)).expect("its file");
                    assert!(
                        file.contains(authored),
                        "{what}: the file holds it as authored"
                    );
                }
                Store::Github(_) => {
                    assert_eq!(
                        at.content(&sandbox).trim_end_matches(['\r', '\n']),
                        authored,
                        "{what}"
                    );
                    // The board holds every line as authored, beside the anchor its source
                    // writes before a heading.
                    let raw = dst.raw(&at);
                    let held: Vec<&str> = raw
                        .lines()
                        .filter(|line| !line.starts_with("<a id=\""))
                        .collect();
                    assert_eq!(held, authored.lines().collect::<Vec<_>>(), "{what}");
                }
                Store::Linear(workspace) => {
                    // What the copy sent is the authored Markdown, byte for byte.
                    let (create, member) = if shape.verb == "task" {
                        ("issueCreate", "description")
                    } else {
                        ("documentCreate", "content")
                    };
                    let title = format!("Shape {}", shape.key);
                    // A create sent alone carries `input`; creates the source sends together
                    // in one aliased document carry `input_<n>`, one per alias.
                    let sent: Vec<String> = workspace
                        .served()
                        .into_iter()
                        .filter(|(query, _)| query.contains(create))
                        .flat_map(|(_, variables)| {
                            variables
                                .as_object()
                                .into_iter()
                                .flatten()
                                .filter(|(name, input)| {
                                    (*name == "input" || name.starts_with("input_"))
                                        && input["title"] == json!(title)
                                })
                                .map(|(_, input)| {
                                    input[member].as_str().unwrap_or_default().to_owned()
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    assert_eq!(sent.len(), 1, "{what}: one create");
                    assert_eq!(visible(&sent[0]), authored, "{what}: sent as authored");
                    // And the workspace stores it as Linear was recorded storing it.
                    let raw = workspace
                        .long_form(at.collection(), at.native())
                        .expect("the record");
                    let stored = visible(&raw);
                    match &shape.recorded {
                        Some(recorded) => assert_eq!(
                            stored,
                            recorded.trim_end_matches(['\r', '\n']),
                            "{what}: stored as the spike recorded Linear storing it"
                        ),
                        None => assert_eq!(
                            onetaskgraph_e2e_support::linear_normalizations::unaccounted(
                                authored, stored
                            ),
                            Vec::new(),
                            "{what}: stored with no change the recorded normalizations do \
                             not account for"
                        ),
                    }
                    assert!(
                        !stored.lines().any(|line| line.starts_with("+++ ")),
                        "{what}: no collapsible section: {stored}"
                    );
                }
            }
        }
    }
}

/// Hold `content`, a body read back through the store from a Linear workspace, to `authored`:
/// equal line for line but for differences each of which is a recorded Linear normalization, and
/// with every block's `<details>`, `<summary>` and closing lines, its summary among them, byte for
/// byte as authored.
fn reads_as_authored_from_linear(content: &str, authored: &str, what: &str) {
    use onetaskgraph_e2e_support::linear_normalizations::unaccounted;
    assert_eq!(
        unaccounted(authored, content),
        Vec::new(),
        "{what}: every difference from the authored body is a recorded Linear normalization:\n{content}"
    );
    let structure = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|line| {
                *line == "<details>" || line.starts_with("<summary>") || *line == "</details>"
            })
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        structure(content),
        structure(authored),
        "{what}: every block's structure lines and summary, byte for byte"
    );
}

/// The SHA-256 of `text`, as lowercase hex.
fn sha256(text: &str) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown, the loopback board and the loopback Linear workspace.
#[test]
fn every_copied_item_reads_back_through_the_store_as_authored() {
    for kind in ["github-projects", "linear"] {
        let sandbox = Sandbox::new();
        let authored_config = authored(&sandbox, "authored");
        let (dst, config) = destination(&sandbox, kind, "dst", None);
        configure(
            &sandbox,
            &[("authored", &authored_config), ("dst", &config)],
        );
        let landed = copy_plan(&sandbox, &authored_ids("authored"), "dst");
        // Stated here rather than read from what the copy wrote: the authored bodies.
        for (key, what) in [("P", "project"), ("T", "task"), ("D", "document")] {
            let expected = record_body(what);
            let content = landed[key].content(&sandbox);
            let what = format!("{kind}: {key}");
            if kind == "github-projects" {
                assert_eq!(
                    content.lines().collect::<Vec<_>>(),
                    expected.lines().collect::<Vec<_>>(),
                    "{what} reads back as authored, line for line"
                );
            } else {
                reads_as_authored_from_linear(&content, &expected, &what);
            }
        }
        let comments: Value = serde_json::from_str(&ok(
            &sandbox,
            &["task", "comment", "list", &landed["T"].id, "--json"],
        ))
        .expect("comments as JSON");
        let body = comments["comments"][0]["body"]
            .as_str()
            .expect("the carried comment");
        if kind == "github-projects" {
            assert_eq!(
                body.trim_end(),
                comment_body().trim_end(),
                "{kind}: the comment"
            );
        } else {
            reads_as_authored_from_linear(body, &comment_body(), &format!("{kind}: the comment"));
        }

        let Store::Linear(workspace) = &dst else {
            continue;
        };
        // A collapsible section a person made in Linear reads back as an authored block.
        workspace.edit_description(
            landed["T"].native(),
            "Made in Linear.\n\n+++ Made **here** & `there` <now>\n\nInside it.\n\n+++",
        );
        assert_eq!(
            landed["T"].content(&sandbox),
            "Made in Linear.\n\n<details>\n<summary>Made <strong>here</strong> &amp; \
             <code>there</code> &lt;now&gt;</summary>\n\nInside it.\n\n</details>",
            "a section made in Linear reads back as an authored block"
        );

        // A shape Linear holds that is not one section in its form reads back as it is stored:
        // a section with an empty summary — Linear's own after a stray `>>>` — one missing a
        // margin, and one never closed; and sections nested in Linear read back as a block
        // inside a block — an inner one that is no section in Linear's form left as it is
        // stored inside the outer one's block.
        for (stored, read) in [
            ("+++ \n\nUnder no summary.\n\n+++", None),
            ("+++ Tight\nNo margin above.\n\n+++", None),
            ("+++ Open\n\nNever closed.", None),
            (
                "+++ Outer\n\nOuter text.\n\n+++ Inner\n\nInner text.\n\n+++\n\n+++",
                Some(
                    "<details>\n<summary>Outer</summary>\n\nOuter text.\n\n<details>\n\
                     <summary>Inner</summary>\n\nInner text.\n\n</details>\n\n</details>",
                ),
            ),
            (
                "+++ Outer\n\nOuter text.\n\n+++ \n\nUnder no summary.\n\n+++\n\n+++",
                Some(
                    "<details>\n<summary>Outer</summary>\n\nOuter text.\n\n+++ \n\nUnder no \
                     summary.\n\n+++\n\n</details>",
                ),
            ),
        ] {
            workspace.edit_description(landed["T"].native(), stored);
            assert_eq!(
                landed["T"].content(&sandbox),
                read.unwrap_or(stored),
                "{stored:?} reads back as it should"
            );
        }

        // A rendering copied to Linear still verifies against its recorded `body_digest`.
        let template = sandbox.subdirectory("templates").join("design.md");
        std::fs::create_dir_all(template.parent().expect("a folder")).expect("the folder");
        std::fs::write(
            &template,
            format!(
                "---\nonetaskgraph_template: 1\nvariables:\n  goal:\n    description: What it is \
                 for\n---\n## Goal\n\n{{{{ goal }}}}\n\n{}\n",
                block(SUMMARIES[2].0, 5)
            ),
        )
        .expect("the template");
        ok(
            &sandbox,
            &[
                "document",
                "create",
                "authored",
                "--id",
                "R",
                "--project",
                "P",
                "--title",
                "Rendered design",
                "--template",
                template.to_str().expect("a path"),
                "--var",
                "goal=Collapse the detail",
                "--no-interactive",
            ],
        );
        let copied = outcomes(&ok(
            &sandbox,
            &["document", "copy", "authored:R", "--to", "dst", "--json"],
        ));
        let rendered = crate::section_links::shown(
            &sandbox,
            "document",
            copied["authored:R"]["destination"]
                .as_str()
                .expect("landed"),
        );
        let content = rendered["content"].as_str().expect("its content");
        assert!(
            content.contains(&format!("<summary>{}</summary>", SUMMARIES[2].0)),
            "the rendered block reads back as authored:\n{content}"
        );
        assert_eq!(
            rendered["metadata"]["onetaskgraph.template"]["body_digest"],
            json!(format!("sha256:{}", sha256(content))),
            "a rendering copied to Linear verifies against its recorded body_digest"
        );
    }
}

/// A design document shaped as the design-document template renders one: headings, bullet and
/// numbered lists, fenced code with a language, a high-cost decision with a collapsed `Examples`
/// block, collapsed reversible decisions — one holding an image — and the planned tasks table in
/// a collapsed block under a visible graph image.
const DESIGN: &str = "## Summary

Collapse the design document's detail behind expandable blocks.

## Decisions

### High-cost decision: write each destination's own form

Each destination gets the form it renders. Getting it wrong costs:

1. A block shown expanded on Linear.
2. A summary that loses its markup.

<details>
<summary>Examples</summary>

- A design document's budget table.
- A task's acceptance detail.

```rust
fn written(form: ExpandableForm, body: &str) -> String {
    form.written(body)
}
```

</details>

### Reversible decisions

<details>
<summary>Translate with <code>ExpandableForm</code></summary>

Each destination kind has one entry.

![The form on a board](./form.png)

</details>

<details>
<summary><strong>Never</strong> translate a malformed block</summary>

A shape that is not the authored form is carried as it stands.

</details>

<details>
<summary>Summaries keep &amp;, &lt; and &gt;</summary>

Linear stores them decoded, and reading one back escapes them again.

</details>

## Planned tasks

![Dependency graph](./graph.png)

<details>
<summary>Planned tasks (3)</summary>

| Task | Depends on | Detail |
| --- | --- | --- |
| Write the form | none | the core module |
| Read it back | Write the form | the Linear plugin |
| Prove it | Read it back | the chain journey |

</details>
";

/// A task body holding a block with a list and code — the code holding a `+++` line and a
/// `</details>` line, which close no block inside a fence, on Linear or in the authored form.
const TASK: &str = "Build the form.

<details>
<summary>Steps for <code>expandable_blocks</code></summary>

1. Recognise the authored form.
2. Write Linear's section.

```text
+++ summary
</details>
```

</details>
";

/// A project description holding a block with a list and code.
const PROJECT: &str = "The plan.

<details>
<summary><strong>Scope</strong></summary>

- Tasks, projects, documents and comments.
- Every destination kind.

```json
{\"kinds\": 3}
```

</details>
";

/// `content` with each image's destination replaced by the name of the asset it shows, as
/// `names` maps a destination to one: a record's own `./<name>` reference is that name.
fn by_asset(content: &str, names: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(at) = rest.find("![") {
        let Some(open) = rest[at..].find("](").map(|open| at + open + 2) else {
            break;
        };
        let Some(close) = rest[open..].find(')').map(|close| open + close) else {
            break;
        };
        let destination = rest[open..close]
            .trim_start_matches('<')
            .trim_end_matches('>');
        let name = names.get(destination).cloned().unwrap_or_else(|| {
            destination
                .strip_prefix("./")
                .unwrap_or(destination)
                .to_owned()
        });
        out.push_str(&rest[..open]);
        out.push_str("asset:");
        out.push_str(&name);
        out.push(')');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Every destination of an asset `shown` — a `show --json` item — lists or records under
/// `onetaskgraph.assets`, mapped to its name.
fn asset_names(shown: &Value, names: &mut BTreeMap<String, String>) {
    for (name, recorded) in shown["metadata"]["onetaskgraph.assets"]
        .as_object()
        .into_iter()
        .flatten()
    {
        if let Some(url) = recorded["url"].as_str() {
            names.insert(url.to_owned(), name.clone());
        }
    }
    for asset in shown["assets"].as_array().into_iter().flatten() {
        let Some(name) = asset["name"].as_str() else {
            continue;
        };
        for value in asset.as_object().into_iter().flat_map(|held| held.values()) {
            if let Some(text) = value.as_str() {
                names.insert(text.to_owned(), name.to_owned());
            }
        }
    }
}

/// `content` rendered to HTML by one CommonMark/GFM renderer, without the line break a raw
/// HTML block ending the content carries over from the content's own last line break, which a
/// store dropping that break drops from it.
fn html(content: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut rendered = String::new();
    html::push_html(
        &mut rendered,
        Parser::new_ext(
            content,
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
        ),
    );
    rendered.trim_end().to_owned()
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: two folders of Markdown, the loopback board and the loopback Linear workspace.
#[test]
fn a_chain_through_a_board_and_linear_gives_back_the_authored_document_and_no_repeat_writes() {
    use onetaskgraph_e2e_support::images::png;
    use onetaskgraph_e2e_support::linear_normalizations::{accounting_entry, hunks};

    let sandbox = Sandbox::new();
    let authored_config =
        json!({"plugin": "local-md", "config": empty_folder(&sandbox, "authored")});
    let root = root_of(&sandbox, "authored");
    for folder in ["projects", "tasks", "documents/D.assets"] {
        std::fs::create_dir_all(root.join(folder)).expect("a folder");
    }
    for (path, front, body) in [
        ("projects/P.md", "title: Block plan\nstatus: todo", PROJECT),
        (
            "tasks/T.md",
            "title: Build the form\nstatus: todo\nproject: P",
            TASK,
        ),
        ("documents/D.md", "title: Design\nproject: P", DESIGN),
    ] {
        std::fs::write(root.join(path), format!("---\n{front}\n---\n{body}")).expect("a record");
    }
    std::fs::write(root.join("documents/D.assets/graph.png"), png(11, 60_000)).expect("an image");
    std::fs::write(root.join("documents/D.assets/form.png"), png(12, 70_000)).expect("an image");

    let (board, board_config) = destination(&sandbox, "github-projects", "board", None);
    let (linear, linear_config) = destination(&sandbox, "linear", "linear", None);
    let last_config = json!({"plugin": "local-md", "config": empty_folder(&sandbox, "last")});
    configure(
        &sandbox,
        &[
            ("authored", &authored_config),
            ("board", &board_config),
            ("linear", &linear_config),
            ("last", &last_config),
        ],
    );
    let copy = |from: &BTreeMap<&'static str, Landed>, to: &str| {
        let project = outcomes(&ok(
            &sandbox,
            &["project", "copy", &from["P"].id, "--to", to, "--json"],
        ));
        let document = outcomes(&ok(
            &sandbox,
            &["document", "copy", &from["D"].id, "--to", to, "--json"],
        ));
        from.iter()
            .map(|(key, source)| {
                let report = if source.verb == "document" {
                    &document
                } else {
                    &project
                };
                let id = report[&source.id]["destination"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{} landed: {report:#?}", source.id))
                    .to_owned();
                (
                    *key,
                    Landed {
                        verb: source.verb,
                        id,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let authored = authored_ids("authored");
    let on_board = copy(&authored, "board");
    let in_linear = copy(&on_board, "linear");
    let at_last = copy(&in_linear, "last");

    let bodies = [("P", PROJECT), ("T", TASK), ("D", DESIGN)];
    let mut names = BTreeMap::new();
    for hop in [&authored, &on_board, &in_linear, &at_last] {
        asset_names(&shown(&sandbox, "document", &hop["D"].id), &mut names);
    }
    // The GitHub hop: byte for byte as authored, but for each image's served URL and the dropped
    // trailing line break.
    for (key, body) in bodies {
        assert_eq!(
            by_asset(
                on_board[key].content(&sandbox).trim_end_matches('\n'),
                &names
            ),
            by_asset(body.trim_end_matches('\n'), &names),
            "the board's {key} is the authored body"
        );
    }
    for (key, body) in bodies {
        let content = at_last[key].content(&sandbox);
        let (authored, last) = (by_asset(body, &names), by_asset(&content, &names));
        let structure = |text: &str| -> Vec<String> {
            text.lines()
                .filter(|line| {
                    *line == "<details>" || line.starts_with("<summary>") || *line == "</details>"
                })
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(
            structure(&last),
            structure(&authored),
            "{key}: every block's lines"
        );
        assert_eq!(
            last.matches("](asset:").count(),
            authored.matches("](asset:").count(),
            "{key}: every image reference is kept:\n{last}"
        );
        // Proven twice: line by line against the recorded normalizations, then as rendered
        // HTML, which a recorded-looking line that changes meaning cannot pass.
        for hunk in hunks(&authored, &last) {
            assert!(
                accounting_entry(&hunk).is_some(),
                "{key}: {hunk:?} is no recorded Linear normalization:\n{last}"
            );
        }
        assert_eq!(html(&last), html(&authored), "{key}: renders as authored");
    }

    // A change of meaning fails that proof, though each of its lines looks like a recorded one.
    let expanded = DESIGN.replacen(
        "<details>\n<summary>Examples",
        "<details open>\n<summary>Examples",
        1,
    );
    assert_ne!(
        html(&by_asset(&expanded, &names)),
        html(&by_asset(DESIGN, &names))
    );
    let moved = DESIGN
        .replacen("Each destination kind has one entry.\n\n", "", 1)
        .replacen(
            "## Planned tasks\n",
            "## Planned tasks\n\nEach destination kind has one entry.\n",
            1,
        );
    assert_ne!(
        html(&by_asset(&moved, &names)),
        html(&by_asset(DESIGN, &names))
    );

    // Each hop repeated: the first repeat changes no body and re-stamps only the records
    // measured, the second writes nothing.
    for (what, from, to, store, folder, measured) in [
        ("the board", &authored, "board", &board, None, &[][..]),
        (
            "Linear",
            &on_board,
            "linear",
            &linear,
            None,
            &["D", "P"][..],
        ),
        (
            "the last folder",
            &in_linear,
            "last",
            &Store::Local,
            Some(root_of(&sandbox, "last")),
            &[][..],
        ),
    ] {
        let landed = match to {
            "board" => &on_board,
            "linear" => &in_linear,
            _ => &at_last,
        };
        repeats_settling(
            &sandbox,
            what,
            &|| {
                copy(from, to);
            },
            (store, folder.as_deref()),
            (landed, false),
            measured,
        );
    }
}

// Every authored block a write sends to Linear lands as a section, wherever it stands and on
// every path that writes a body — issue onetaskgraph#4087.

/// Issue onetaskgraph#4087's three blocks, each summary beside its body: two lists, then a
/// fenced code block.
const ISSUE_BLOCKS: [(&str, &str); 3] = [
    ("Pros and cons", "- a point"),
    ("Test results", "- a result"),
    ("Reproduction", "Run:\n\n```\nsome command --count=10\n```"),
];

fn details(summary: &str, body: &str) -> String {
    format!("<details>\n<summary>{summary}</summary>\n\n{body}\n\n</details>")
}

fn issue_body() -> String {
    let [pros, results, repro] = ISSUE_BLOCKS.map(|(summary, body)| details(summary, body));
    format!("{pros}\n\n{results}\n\n{repro}\n")
}

fn issue_variants() -> Vec<(&'static str, String)> {
    let [pros, results, repro] = ISSUE_BLOCKS.map(|(summary, body)| details(summary, body));
    vec![
        ("the issue's body", issue_body()),
        ("fenced first", format!("{repro}\n\n{pros}\n\n{results}\n")),
        (
            "fenced in the middle",
            format!("{pros}\n\n{repro}\n\n{results}\n"),
        ),
        ("fenced last", format!("{pros}\n\n{results}\n\n{repro}\n")),
        ("fenced alone", format!("{repro}\n")),
        (
            "ending in the fenced block",
            format!("Intro.\n\n{results}\n\n{repro}"),
        ),
    ]
}

/// `body` with every block written as its summary's paragraph then its body: the same text with
/// no expandable block in it.
fn without_blocks(body: &str) -> String {
    let mut plain = body.to_owned();
    for (summary, held) in ISSUE_BLOCKS {
        plain = plain.replace(&details(summary, held), &format!("{summary}\n\n{held}"));
    }
    assert!(
        !plain.contains("<details>"),
        "every block taken out: {plain}"
    );
    plain
}

fn summaries_of(body: &str) -> Vec<&'static str> {
    let mut at: Vec<(usize, &'static str)> = ISSUE_BLOCKS
        .iter()
        .filter_map(|(summary, held)| body.find(&details(summary, held)).map(|at| (at, *summary)))
        .collect();
    at.sort_unstable();
    at.into_iter().map(|(_, summary)| summary).collect()
}

/// `authored` as Linear reads back: the one recorded normalization these bodies meet, a `- `
/// bullet stored as `* ` (`issue-linear-plus.json`, probe P08), and no line break at the end.
fn as_linear_reads_it(authored: &str) -> String {
    authored
        .trim_end()
        .lines()
        .map(|line| {
            line.strip_prefix("- ")
                .map_or_else(|| line.to_owned(), |rest| format!("* {rest}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn unfenced(text: &str) -> Vec<&str> {
    let mut fence: Option<&str> = None;
    let mut outside = Vec::new();
    for line in text.lines() {
        let marker = line.trim_start();
        let opens = marker.starts_with("```") || marker.starts_with("~~~");
        match fence {
            Some(open) if opens && marker.starts_with(open) => fence = None,
            Some(_) => {}
            None if opens => fence = Some(&marker[..3]),
            None => outside.push(line),
        }
    }
    outside
}

/// Hold `stored`, the body a Linear workspace holds, to `summaries`: each a `+++ <summary>` line,
/// a closing `+++` line for each, and no `<details>` or `</details>` line outside fenced code.
fn lands_as_sections(stored: &str, summaries: &[&str], what: &str) {
    let lines = unfenced(stored);
    for summary in summaries {
        assert!(
            lines.contains(&format!("+++ {summary}").as_str()),
            "{what}: `{summary}` lands as a section:\n{stored}"
        );
    }
    assert_eq!(
        lines.iter().filter(|line| **line == "+++").count(),
        summaries.len(),
        "{what}: every section closed, and no other:\n{stored}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| *line == "<details>" || *line == "</details>"),
        "{what}: no block is left as literal HTML:\n{stored}"
    );
}

/// The requests `served` holds, counted by kind: each GraphQL document's operation and its
/// first field.
fn by_kind(served: &[(String, Value)]) -> BTreeMap<String, usize> {
    let mut kinds = BTreeMap::new();
    for (document, _) in served {
        let operation = document
            .trim_start()
            .split([' ', '(', '{'])
            .next()
            .unwrap_or_default();
        let field = document
            .split_once('{')
            .map(|(_, rest)| {
                rest.trim_start()
                    .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
                    .next()
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        *kinds.entry(format!("{operation} {field}")).or_insert(0) += 1;
    }
    kinds
}

/// Every path that writes a body to Linear, each one journey: the paths that write over a
/// record read it first, which is what let a block it held as literal text decide the form of the
/// caller's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WritePath {
    TaskContentSet,
    TaskCreate,
    TaskUpdate,
    ProjectCreate,
    ProjectReplace,
    DocumentCreate,
    DocumentReplace,
    CommentAdd,
    CommentEdit,
    TaskCopy,
    ProjectCopy,
    DocumentCopy,
}

impl WritePath {
    /// The record of the copied plan this path writes over, when it writes over one, by its key:
    /// the one a write over it reads first.
    fn over(self) -> Option<&'static str> {
        match self {
            Self::TaskContentSet | Self::TaskUpdate | Self::TaskCopy | Self::CommentEdit => {
                Some("T")
            }
            Self::ProjectReplace | Self::ProjectCopy => Some("P"),
            Self::DocumentReplace | Self::DocumentCopy => Some("D"),
            Self::TaskCreate | Self::ProjectCreate | Self::DocumentCreate | Self::CommentAdd => {
                None
            }
        }
    }
}

/// A plan the authored folder copied into a loopback Linear storing what it is sent as real
/// Linear was recorded storing it: a project, a task carrying one comment and a document, each
/// with metadata, so the store's slot follows each body.
struct LinearPlan {
    sandbox: Sandbox,
    workspace: onetaskgraph_e2e_support::fixtures::LinearWorkspace,
    landed: BTreeMap<&'static str, Landed>,
}

/// What one write left: the body the workspace stores, without its slot, the body a read gives
/// back through the store, and the requests the write sent, by kind.
struct Written {
    stored: String,
    read: String,
    requests: BTreeMap<String, usize>,
}

impl LinearPlan {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let authored_config = authored(&sandbox, "authored");
        let (store, config) = destination(&sandbox, "linear", "dest", None);
        configure(
            &sandbox,
            &[("authored", &authored_config), ("dest", &config)],
        );
        let landed = copy_plan(&sandbox, &authored_ids("authored"), "dest");
        let Store::Linear(workspace) = store else {
            unreachable!("a Linear store")
        };
        Self {
            sandbox,
            workspace,
            landed,
        }
    }

    fn file(&self, name: &str, body: &str) -> String {
        let file = self.sandbox.project().join(name);
        std::fs::write(&file, body).expect("the body file");
        file.display().to_string()
    }

    fn comment(&self) -> String {
        let comments = self.workspace.comments_on(self.landed["T"].native());
        comments[0]["id"].as_str().expect("a comment id").to_owned()
    }

    /// Have the record `at` hold `body` as literal text, its slot kept: what Linear stores of it
    /// sent with its `<details>` as written — as a client that never converted them, or 0.15.1
    /// for a block it left as HTML, sends it.
    fn hold_as_text(&self, at: &Landed, body: &str) {
        let held = onetaskgraph_e2e_support::linear_markdown::stored(body);
        let held = held.trim_end();
        let raw = self
            .workspace
            .long_form(at.collection(), at.native())
            .expect("the record");
        let slot = raw
            .find("\n\n+++ onetaskgraph metadata\n\n")
            .map_or("", |at| &raw[at..]);
        let whole = format!("{held}{slot}");
        match at.verb {
            "task" => self.workspace.store_description(at.native(), &whole),
            "project" => self.workspace.edit_project_content(at.native(), &whole),
            _ => self.workspace.edit_document_content(at.native(), &whole),
        }
    }

    fn stored(&self, at: &Landed) -> String {
        visible(
            &self
                .workspace
                .long_form(at.collection(), at.native())
                .expect("the record"),
        )
        .to_owned()
    }

    fn author(&self, key: &str, body: &str) {
        let folder = match key {
            "P" => "projects",
            "T" => "tasks",
            _ => "documents",
        };
        let path = root_of(&self.sandbox, "authored")
            .join(folder)
            .join(format!("{key}.md"));
        let held = std::fs::read_to_string(&path).expect("the authored record");
        let front_end = held[3..].find("\n---\n").expect("front matter") + 3 + "\n---\n".len();
        std::fs::write(&path, format!("{}{body}", &held[..front_end])).expect("the record");
    }

    /// Write `body` along `path` — over a record holding `held_before` as literal text, for a
    /// path that writes over one — answering what it left.
    fn write(&self, path: WritePath, body: &str, held_before: &str) -> Written {
        if let Some(key) = path.over() {
            if path == WritePath::CommentEdit {
                self.workspace.edit_comment(
                    &self.comment(),
                    &onetaskgraph_e2e_support::linear_markdown::stored(held_before),
                );
            } else {
                self.hold_as_text(&self.landed[key], held_before);
            }
        }
        let file = self.file("written.md", body);
        let (project, task) = (&self.landed["P"], &self.landed["T"]);
        let mark = self.workspace.served_count();
        let created = |rendered: String, verb: &'static str| Landed {
            verb,
            id: serde_json::from_str::<Value>(&rendered).expect("JSON")["items"][0]["id"]
                .as_str()
                .expect("the created id")
                .to_owned(),
        };
        let at = match path {
            WritePath::TaskContentSet => {
                ok(
                    &self.sandbox,
                    &["task", "content", "set", &task.id, "--file", &file],
                );
                task.clone()
            }
            WritePath::TaskUpdate => {
                ok(
                    &self.sandbox,
                    &["task", "update", &task.id, "--body-file", &file],
                );
                task.clone()
            }
            WritePath::TaskCreate => created(
                ok(
                    &self.sandbox,
                    &[
                        "task",
                        "create",
                        "dest",
                        "--project",
                        project.native(),
                        "--title",
                        "Written",
                        "--body-file",
                        &file,
                        "--json",
                    ],
                ),
                "task",
            ),
            WritePath::ProjectCreate => created(
                ok(
                    &self.sandbox,
                    &[
                        "project",
                        "create",
                        "dest",
                        "--id",
                        "written-plan",
                        "--title",
                        "Written",
                        "--body-file",
                        &file,
                        "--json",
                    ],
                ),
                "project",
            ),
            WritePath::ProjectReplace => {
                ok(
                    &self.sandbox,
                    &[
                        "project",
                        "create",
                        "dest",
                        "--id",
                        project.native(),
                        "--title",
                        "Block plan",
                        "--body-file",
                        &file,
                    ],
                );
                project.clone()
            }
            WritePath::DocumentCreate => created(
                ok(
                    &self.sandbox,
                    &[
                        "document",
                        "create",
                        "dest",
                        "--project",
                        project.native(),
                        "--title",
                        "Written",
                        "--body-file",
                        &file,
                        "--json",
                    ],
                ),
                "document",
            ),
            WritePath::DocumentReplace => {
                let document = &self.landed["D"];
                ok(
                    &self.sandbox,
                    &[
                        "document",
                        "create",
                        "dest",
                        "--id",
                        document.native(),
                        "--project",
                        project.native(),
                        "--title",
                        "Block design",
                        "--body-file",
                        &file,
                    ],
                );
                document.clone()
            }
            WritePath::CommentAdd | WritePath::CommentEdit => {
                let before: Vec<String> = self
                    .workspace
                    .comments_on(task.native())
                    .iter()
                    .map(|comment| comment["id"].as_str().unwrap_or_default().to_owned())
                    .collect();
                let comment = if path == WritePath::CommentAdd {
                    ok(
                        &self.sandbox,
                        &["task", "comment", "add", &task.id, "--body-file", &file],
                    );
                    self.workspace
                        .comments_on(task.native())
                        .iter()
                        .map(|comment| comment["id"].as_str().unwrap_or_default().to_owned())
                        .find(|id| !before.contains(id))
                        .expect("the comment added")
                } else {
                    let comment = self.comment();
                    ok(
                        &self.sandbox,
                        &[
                            "task",
                            "comment",
                            "edit",
                            &task.id,
                            &comment,
                            "--body-file",
                            &file,
                        ],
                    );
                    comment
                };
                let requests = by_kind(&self.workspace.served_since(mark));
                let stored = self
                    .workspace
                    .comments_on(task.native())
                    .into_iter()
                    .find(|held| held["id"] == json!(comment))
                    .and_then(|held| held["body"].as_str().map(str::to_owned))
                    .expect("the comment written");
                let listed: Value = serde_json::from_str(&ok(
                    &self.sandbox,
                    &["task", "comment", "list", &task.id, "--json"],
                ))
                .expect("comments");
                let read = listed["comments"]
                    .as_array()
                    .expect("comments")
                    .iter()
                    .find(|listed| listed["id"] == json!(comment))
                    .and_then(|listed| listed["body"].as_str())
                    .expect("the comment, listed")
                    .to_owned();
                return Written {
                    stored: stored.trim_end().to_owned(),
                    read,
                    requests,
                };
            }
            WritePath::TaskCopy | WritePath::ProjectCopy | WritePath::DocumentCopy => {
                let (key, verb) = match path {
                    WritePath::TaskCopy => ("T", "task"),
                    WritePath::ProjectCopy => ("P", "project"),
                    _ => ("D", "document"),
                };
                self.author(key, body);
                let mark = self.workspace.served_count();
                let source = format!("authored:{key}");
                let mut arguments = vec![verb, "copy", source.as_str(), "--to", "dest", "--json"];
                if path == WritePath::ProjectCopy {
                    arguments.push("--no-tasks");
                }
                let report = outcomes(&ok(&self.sandbox, &arguments));
                assert_eq!(
                    report[&source]["destination"],
                    json!(self.landed[key].id),
                    "{path:?}: the copy writes over the counterpart: {report:#?}"
                );
                let at = &self.landed[key];
                return Written {
                    stored: self.stored(at),
                    read: at.content(&self.sandbox),
                    requests: by_kind(&self.workspace.served_since(mark)),
                };
            }
        };
        let requests = by_kind(&self.workspace.served_since(mark));
        Written {
            stored: self.stored(&at),
            read: at.content(&self.sandbox),
            requests,
        }
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn the_issues_body_set_over_an_issue_holding_it_as_text_lands_every_block_as_a_section() {
    // The issue reproduced: its body set with `task content set` on an issue already holding it
    // as literal text, which the write reads first. Linear stored the lists' `-` as `*`, so the
    // two list blocks differ from what it holds; the fenced block does not.
    let plan = LinearPlan::new();
    let body = issue_body();
    let written = plan.write(WritePath::TaskContentSet, &body, &body);
    let summaries = summaries_of(&body);
    assert_eq!(summaries, ["Pros and cons", "Test results", "Reproduction"]);
    lands_as_sections(&written.stored, &summaries, "the issue's body");
    assert_eq!(
        written.stored,
        "+++ Pros and cons\n\n* a point\n\n+++\n\n+++ Test results\n\n* a result\n\n+++\n\n\
         +++ Reproduction\n\nRun:\n\n```\nsome command --count=10\n```\n\n+++",
        "every block stored as a section"
    );
    assert_eq!(
        written.read,
        as_linear_reads_it(&body),
        "read back as authored"
    );
    reads_as_authored_from_linear(&written.read, &body, "the issue's body");
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn every_block_lands_as_a_section_wherever_the_fenced_one_stands_with_metadata_or_without() {
    // Each variant set over the copied task, whose metadata slot follows the body, and over a
    // created task carrying none — each holding the issue's body as literal text first, and
    // fresh.
    let plan = LinearPlan::new();
    let bare = Landed {
        verb: "task",
        id: {
            let file = plan.file("bare.md", "Bare.\n");
            let created: Value = serde_json::from_str(&ok(
                &plan.sandbox,
                &[
                    "task",
                    "create",
                    "dest",
                    "--project",
                    plan.landed["P"].native(),
                    "--title",
                    "Bare",
                    "--body-file",
                    &file,
                    "--json",
                ],
            ))
            .expect("JSON");
            created["items"][0]["id"]
                .as_str()
                .expect("an id")
                .to_owned()
        },
    };
    let raw = plan
        .workspace
        .long_form("tasks", bare.native())
        .expect("the bare task");
    assert!(
        !raw.contains("onetaskgraph metadata"),
        "the bare task has no slot: {raw}"
    );
    for (what, body) in issue_variants() {
        for (at, slot) in [(&plan.landed["T"], "a slot"), (&bare, "no slot")] {
            for held_first in [true, false] {
                let what = format!("{what}, {slot}, held as text first: {held_first}");
                if held_first {
                    plan.hold_as_text(at, &issue_body());
                }
                let file = plan.file("variant.md", &body);
                ok(
                    &plan.sandbox,
                    &["task", "content", "set", &at.id, "--file", &file],
                );
                lands_as_sections(&plan.stored(at), &summaries_of(&body), &what);
                let read = at.content(&plan.sandbox);
                assert_eq!(
                    read,
                    as_linear_reads_it(&body),
                    "{what}: read back as authored"
                );
                reads_as_authored_from_linear(&read, &body, &what);
                let raw = plan
                    .workspace
                    .long_form("tasks", at.native())
                    .expect("the task");
                assert_eq!(
                    raw.contains("\n\n+++ onetaskgraph metadata\n\n"),
                    slot == "a slot",
                    "{what}: the slot follows the body only where the item has metadata"
                );
            }
        }
    }
}

/// `path` sends the issue's body — over a record holding it as literal text, where the path
/// writes over one — and the same body with no block in it, each in a plan of its own: every
/// block lands as a section, reads back as authored, and costs no request the plain body does not.
fn every_block_lands_as_a_section_along(path: WritePath) {
    let body = issue_body();
    let plain = without_blocks(&body);
    let what = format!("{path:?}");
    // The loopback stores a comment's body as it is sent, and a long form's `-` bullets as `*`,
    // as Linear was recorded storing it.
    let reads = |authored: &str| match path {
        WritePath::CommentAdd | WritePath::CommentEdit => authored.trim_end().to_owned(),
        _ => as_linear_reads_it(authored),
    };
    let with = LinearPlan::new().write(path, &body, &body);
    lands_as_sections(&with.stored, &summaries_of(&body), &what);
    assert_eq!(
        with.read.trim_end(),
        reads(&body),
        "{what}: read back as authored"
    );
    reads_as_authored_from_linear(&with.read, &body, &what);
    let without = LinearPlan::new().write(path, &plain, &plain);
    assert_eq!(
        without.read.trim_end(),
        reads(&plain),
        "{what}: the plain body"
    );
    assert_eq!(
        with.requests, without.requests,
        "{what}: converting blocks sends no request the same write without them does not"
    );
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn task_content_set_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::TaskContentSet);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn task_create_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::TaskCreate);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn task_update_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::TaskUpdate);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn project_create_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::ProjectCreate);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn project_replace_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::ProjectReplace);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn document_create_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::DocumentCreate);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn document_replace_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::DocumentReplace);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn comment_add_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::CommentAdd);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn comment_edit_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::CommentEdit);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn task_copy_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::TaskCopy);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn project_copy_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::ProjectCopy);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn document_copy_lands_every_block_as_a_section_at_no_extra_request() {
    every_block_lands_as_a_section_along(WritePath::DocumentCopy);
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn a_body_linear_holds_as_text_read_and_written_back_is_stored_as_it_was() {
    // A body read from Linear and set back as it reads — as `task show` gives it — keeps every
    // block Linear held as literal text as literal text.
    let plan = LinearPlan::new();
    let task = &plan.landed["T"];
    plan.hold_as_text(task, &issue_body());
    let held = plan.stored(task);
    assert!(held.contains("<details>"), "held as literal text: {held}");
    let read = task.content(&plan.sandbox);
    let file = plan.file("read.md", &read);
    ok(
        &plan.sandbox,
        &["task", "content", "set", &task.id, "--file", &file],
    );
    assert_eq!(plan.stored(task), held, "written back as Linear stored it");
    assert_eq!(task.content(&plan.sandbox), read, "and read back as before");
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: two folders of Markdown and the loopback board.
#[test]
fn the_issues_body_set_on_a_folder_and_a_board_is_stored_byte_for_byte() {
    let body = issue_body();
    for kind in ["local-md", "github-projects"] {
        let sandbox = Sandbox::new();
        let authored_config = authored(&sandbox, "authored");
        let (store, config) = destination(&sandbox, kind, "dest", None);
        configure(
            &sandbox,
            &[("authored", &authored_config), ("dest", &config)],
        );
        let landed = copy_plan(&sandbox, &authored_ids("authored"), "dest");
        let task = &landed["T"];
        let file = sandbox.project().join("issue.md");
        std::fs::write(&file, &body).expect("the body");
        let file = file.display().to_string();
        ok(
            &sandbox,
            &["task", "content", "set", &task.id, "--file", &file],
        );
        let held = match &store {
            Store::Local => {
                let file = std::fs::read_to_string(task.location(&sandbox)).expect("its file");
                let after = file[3..].find("\n---\n").expect("front matter") + 3 + "\n---\n".len();
                file[after..]
                    .split("\n## Comments\n")
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            }
            _ => store.raw(task),
        };
        assert_eq!(
            held.trim_end(),
            body.trim_end(),
            "{kind}: stored as authored"
        );
        assert_eq!(
            task.content(&sandbox).trim_end(),
            body.trim_end(),
            "{kind}: read back"
        );
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn a_copys_follow_up_keeps_literal_text_linear_stored_over_the_section_it_replaced() {
    // The counterpart holds a section; the copy reads it, then sends a shape the authored form
    // does not recognise — a list straight after the summary — which Linear stores as literal
    // text reading back as that very section's block. The copy's follow-up sends what Linear
    // stored back, and the block stays the literal text the latest read found.
    let plan = LinearPlan::new();
    let task = &plan.landed["T"];
    let section = details("Held", "* a point");
    let file = plan.file("section.md", &format!("{section}\n"));
    ok(
        &plan.sandbox,
        &["task", "content", "set", &task.id, "--file", &file],
    );
    assert_eq!(
        plan.stored(task),
        "+++ Held\n\n* a point\n\n+++",
        "held as a section"
    );
    let shape = "<details>\n<summary>Held</summary>\n- a point\n\n</details>\n";
    let recorded = onetaskgraph_e2e_support::linear_markdown::stored(shape);
    assert_eq!(
        recorded.trim_end(),
        section,
        "Linear stores it as the section's block"
    );
    plan.author("T", shape);
    let mark = plan.workspace.served_count();
    ok(
        &plan.sandbox,
        &["task", "copy", "authored:T", "--to", "dest", "--json"],
    );
    let updates = plan
        .workspace
        .served_since(mark)
        .into_iter()
        .filter(|(document, _)| document.contains("issueUpdate("))
        .count();
    assert_eq!(updates, 2, "the copy's write, then its follow-up");
    assert_eq!(
        plan.stored(task),
        section,
        "stored as literal text, as Linear stored it"
    );
    assert_eq!(task.content(&plan.sandbox), section, "and read back so");
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn a_callers_body_differing_from_the_held_one_only_in_prose_lands_its_block_as_a_section() {
    // The issue holds the fenced block alone as literal text, and the caller sets a line of prose
    // and that same block: the caller's own body, so the block lands as a section.
    let plan = LinearPlan::new();
    let task = &plan.landed["T"];
    let (summary, held) = ISSUE_BLOCKS[2];
    let fenced = details(summary, held);
    plan.hold_as_text(task, &format!("{fenced}\n"));
    assert_eq!(plan.stored(task), fenced, "held as literal text");
    let body = format!("New.\n\n{fenced}\n");
    let file = plan.file("new.md", &body);
    ok(
        &plan.sandbox,
        &["task", "content", "set", &task.id, "--file", &file],
    );
    lands_as_sections(&plan.stored(task), &[summary], "prose and the held block");
    assert_eq!(
        task.content(&plan.sandbox),
        body.trim_end(),
        "read back as authored"
    );
}

/// Two tasks on one Linear source holding the same block's text, the first as literal text and
/// the second as a section after a line of prose: the plan's own task and one created beside it,
/// each with metadata, so each body reads back alike, a slot following it.
fn two_items_holding_one_block(plan: &LinearPlan, fenced: &str) -> (Landed, Landed) {
    let file = plan.file("beside.md", "Beside.\n");
    let created: Value = serde_json::from_str(&ok(
        &plan.sandbox,
        &[
            "task",
            "create",
            "dest",
            "--project",
            plan.landed["P"].native(),
            "--title",
            "Beside",
            "--metadata",
            "review.note=\"beside\"",
            "--body-file",
            &file,
            "--json",
        ],
    ))
    .expect("JSON");
    let beside = Landed {
        verb: "task",
        id: created["items"][0]["id"]
            .as_str()
            .expect("an id")
            .to_owned(),
    };
    let task = plan.landed["T"].clone();
    plan.hold_as_text(&task, &format!("{fenced}\n"));
    let file = plan.file("section.md", &format!("Intro.\n\n{fenced}"));
    ok(
        &plan.sandbox,
        &["task", "content", "set", &beside.id, "--file", &file],
    );
    assert_eq!(
        plan.stored(&task),
        fenced,
        "the plan's task holds it as literal text"
    );
    assert!(
        plan.stored(&beside).starts_with("Intro.\n\n+++ "),
        "the other holds it as a section, after a line of prose"
    );
    (task, beside)
}

/// One `task update-many` of `first` and then `second`, each held at the version it is at, so
/// the call reads both before it writes either.
fn update_both(plan: &LinearPlan, first: (&Landed, Value), second: (&Landed, Value)) {
    let mut updates = Vec::new();
    for (at, mut update) in [first, second] {
        update["item"] = json!(at.id);
        update["if_version"] = shown(&plan.sandbox, "task", &at.id)["version"].clone();
        updates.push(update);
    }
    let file = plan.file(
        "updates.json",
        &json!({"schema_version": 1, "updates": updates}).to_string(),
    );
    let answered: Value = serde_json::from_str(&ok(
        &plan.sandbox,
        &["task", "update-many", "dest", "--input", &file, "--json"],
    ))
    .expect("JSON");
    for result in answered["results"].as_array().expect("results") {
        assert_eq!(result["outcome"], json!("written"), "{answered:#}");
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn another_items_read_of_the_same_body_never_holds_back_a_callers_block() {
    // The caller sets, on the task holding the block as a section after prose, the block alone —
    // byte for byte the body of the other task, which holds it as literal text — in a call that
    // reads both, either one last: the other's read is no read of this one, and the block lands
    // as a section.
    let (summary, held) = ISSUE_BLOCKS[2];
    let fenced = details(summary, held);
    for read_last in ["the writer", "the other"] {
        let plan = LinearPlan::new();
        let (text, section) = two_items_holding_one_block(&plan, &fenced);
        let write = (&section, json!({"body": fenced}));
        let other = (&text, json!({"title": "Blocks, again"}));
        match read_last {
            "the writer" => update_both(&plan, other, write),
            _ => update_both(&plan, write, other),
        }
        let what = format!("{read_last} read last");
        lands_as_sections(&plan.stored(&section), &[summary], &what);
        assert_eq!(
            plan.stored(&text),
            fenced,
            "{what}: the other task is as it was"
        );
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn another_items_read_of_the_same_block_never_changes_what_a_write_back_keeps() {
    // A metadata write on the task holding the block as literal text sends its body back as it
    // read it, in a call that also reads the task holding that block as a section: its own read
    // decides, and its block stays literal text, whichever task was read last.
    let (summary, held) = ISSUE_BLOCKS[2];
    let fenced = details(summary, held);
    for read_last in ["the writer", "the other"] {
        let plan = LinearPlan::new();
        let (text, section) = two_items_holding_one_block(&plan, &fenced);
        let write = (&text, json!({"metadata": {"review.note": "kept"}}));
        let other = (&section, json!({"title": "Beside, again"}));
        match read_last {
            "the writer" => update_both(&plan, other, write),
            _ => update_both(&plan, write, other),
        }
        let what = format!("{read_last} read last");
        assert_eq!(plan.stored(&text), fenced, "{what}: kept as literal text");
        let raw = plan
            .workspace
            .long_form("tasks", text.native())
            .expect("the task");
        assert!(
            raw.contains("review.note"),
            "{what}: the metadata write landed: {raw}"
        );
        lands_as_sections(&plan.stored(&section), &[summary], &what);
    }
}

/// A design document's collapsed visual change, as spike `spike-linear-nested-blocks` probed it:
/// an outer block holding a paragraph and the after image, and inside it a block holding the
/// before image — each image `after` and `before` names.
fn nested_body(what: &str, (after, before): (&str, &str)) -> String {
    format!(
        "The {what}.\n\n<details>\n<summary>Visual <strong>change</strong> of the {what}</summary>\
         \n\nParagraph inside the {what}'s change.\n\n![After]({after})\n\n<details>\n\
         <summary>Before</summary>\n\n![Before]({before})\n\n</details>\n\n</details>\n\n\
         After both.\n"
    )
}

/// The images each record of the nested plan shows: a task's and a document's are its own image
/// assets; a project holds none (README "Image assets"), so its description shows the images the
/// spike uploaded to Linear by their addresses.
fn nested_images(key: &str) -> (String, String) {
    if key == "P" {
        use onetaskgraph_e2e_support::linear_normalizations::nested_image;
        (nested_image("after.png"), nested_image("before.png"))
    } else {
        ("./after.png".to_owned(), "./before.png".to_owned())
    }
}

/// Write the nested plan into the folder `name`: a project, a task under it and a design
/// document under it, each holding [`nested_body`], the task and the document with their assets.
fn nested_plan(sandbox: &Sandbox, name: &str) -> (Value, BTreeMap<&'static str, String>) {
    use onetaskgraph_e2e_support::images::png;
    let config = empty_folder(sandbox, name);
    let root = root_of(sandbox, name);
    let mut bodies = BTreeMap::new();
    for (key, folder, what, front) in [
        (
            "P",
            "projects",
            "project",
            "title: Nested plan\nstatus: todo",
        ),
        (
            "T",
            "tasks",
            "task",
            "title: Nested task\nstatus: todo\nproject: P",
        ),
        (
            "D",
            "documents",
            "document",
            "title: Nested design\nproject: P",
        ),
    ] {
        std::fs::create_dir_all(root.join(folder)).expect("a folder");
        let (after, before) = nested_images(key);
        let body = nested_body(what, (&after, &before));
        std::fs::write(
            root.join(folder).join(format!("{key}.md")),
            format!("---\n{front}\n---\n{body}"),
        )
        .expect("a record");
        if key != "P" {
            let assets = root.join(folder).join(format!("{key}.assets"));
            std::fs::create_dir_all(&assets).expect("its assets");
            std::fs::write(assets.join("after.png"), png(21, 60_000)).expect("an image");
            std::fs::write(assets.join("before.png"), png(22, 70_000)).expect("an image");
        }
        bodies.insert(key, body);
    }
    (json!({"plugin": "local-md", "config": config}), bodies)
}

/// Copy the nested plan's project with its task, and then its document, from the folder
/// `authored` into `dst`, answering where each landed: [`copy_plan`] without `--comments`, which
/// the nested plan holds none of.
fn copy_nested(sandbox: &Sandbox) -> BTreeMap<&'static str, Landed> {
    let project = outcomes(&ok(
        sandbox,
        &["project", "copy", "authored:P", "--to", "dst", "--json"],
    ));
    let document = outcomes(&ok(
        sandbox,
        &["document", "copy", "authored:D", "--to", "dst", "--json"],
    ));
    authored_ids("authored")
        .into_iter()
        .map(|(key, source)| {
            let report = if source.verb == "document" {
                &document
            } else {
                &project
            };
            let id = report[&source.id]["destination"]
                .as_str()
                .unwrap_or_else(|| panic!("{} landed: {report:#?}", source.id))
                .to_owned();
            (
                key,
                Landed {
                    verb: source.verb,
                    id,
                },
            )
        })
        .collect()
}

/// Each collapsible section of a decoded editor state at any depth, as its summary and the
/// sections and images inside it, indented by depth: an `image` node by its `src`, and an image
/// the loopback keeps in a paragraph's text by its address.
fn sections_of(nodes: &Value, depth: usize, found: &mut Vec<String>) {
    fn text(node: &Value) -> String {
        let own: String = node["runs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|run| run["text"].as_str())
            .collect();
        node["content"]
            .as_array()
            .into_iter()
            .flatten()
            .fold(own, |held, child| held + &text(child))
    }
    for node in nodes.as_array().into_iter().flatten() {
        let indent = "  ".repeat(depth);
        if node["type"] == "collapsible_section" {
            let content = node["content"].as_array().cloned().unwrap_or_default();
            let heading = content.first().map(text).unwrap_or_default();
            found.push(format!("{indent}section {}", heading.trim()));
            sections_of(&Value::Array(content[1..].to_vec()), depth + 1, found);
        } else if depth > 0 {
            let text = text(node);
            if let Some(at) = text.find("![") {
                found.push(format!("{indent}image {}", &text[at..]));
            }
        }
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: folders of Markdown, the loopback board and the loopback Linear workspace.
#[test]
fn a_nested_block_lands_on_linear_as_a_section_inside_a_section_and_reads_back_as_authored() {
    use onetaskgraph_e2e_support::fixtures::LinearRecord;
    use onetaskgraph_e2e_support::linear_editor::decoded_tree;
    for kind in KINDS {
        let sandbox = Sandbox::new();
        let (authored_config, bodies) = nested_plan(&sandbox, "authored");
        let (dst, config) = destination(&sandbox, kind, "dst", None);
        configure(
            &sandbox,
            &[("authored", &authored_config), ("dst", &config)],
        );
        let landed = copy_nested(&sandbox);
        let mut names = BTreeMap::new();
        for verb_key in ["T", "D"] {
            asset_names(
                &shown(&sandbox, landed[verb_key].verb, &landed[verb_key].id),
                &mut names,
            );
        }
        for (key, body) in &bodies {
            let what = format!("{kind}: {key}");
            let at = &landed[key];
            let raw = match &dst {
                Store::Local => {
                    let file = std::fs::read_to_string(at.location(&sandbox)).expect("its file");
                    file.splitn(3, "---\n")
                        .nth(2)
                        .unwrap_or_default()
                        .to_owned()
                }
                _ => dst.raw(at),
            };
            let lines: Vec<&str> = raw.lines().collect();
            if kind == "linear" {
                // A section inside a section, each closed by a bare `+++`, and nothing else
                // that marks a block: no authored markup and no `>>>`.
                let marks: Vec<&str> = lines
                    .iter()
                    .copied()
                    .filter(|line| line.starts_with("+++") || line.starts_with(">>>"))
                    .collect();
                let what_kind = ["P", "T", "D"]
                    .iter()
                    .zip(["project", "task", "document"])
                    .find(|(held, _)| *held == key)
                    .map(|(_, what)| what)
                    .expect("a key");
                assert_eq!(
                    marks,
                    [
                        format!("+++ Visual **change** of the {what_kind}").as_str(),
                        "+++ Before",
                        "+++",
                        "+++"
                    ],
                    "{what}: stored as a section inside a section:\n{raw}"
                );
                for line in &lines {
                    assert!(
                        !(line.starts_with("<details") || line.starts_with("<summary>"))
                            && *line != "</details>",
                        "{what}: no authored markup is left on Linear: {line:?}\n{raw}"
                    );
                }
                let position = |wanted: &dyn Fn(&str) -> bool| {
                    lines
                        .iter()
                        .position(|line| wanted(line))
                        .unwrap_or_else(|| panic!("{what}: holds the line:\n{raw}"))
                };
                let (outer, inner) = (
                    position(&|line| line.starts_with("+++ Visual")),
                    position(&|line| line == "+++ Before"),
                );
                let after = position(&|line| line.starts_with("![After]("));
                let before = position(&|line| line.starts_with("![Before]("));
                let close = position(&|line| line == "+++");
                assert!(
                    outer < after && after < inner && inner < before && before < close,
                    "{what}: the after image in the outer section, the before image in the inner \
                     one:\n{raw}"
                );
                // And Linear's editor state holds that nesting, each image where it was written.
                let Store::Linear(workspace) = &dst else {
                    unreachable!()
                };
                let record = match at.verb {
                    "task" => LinearRecord::Task,
                    "project" => LinearRecord::Project,
                    _ => LinearRecord::Document,
                };
                let state = workspace
                    .editor_state(record, at.native())
                    .unwrap_or_else(|| panic!("{what}: an editor state"));
                let tree = decoded_tree(&state).expect("the state decodes");
                let mut outline = Vec::new();
                sections_of(&tree["content"], 0, &mut outline);
                // The slot is a collapsed block of its own after the content.
                outline.retain(|line| line != "section onetaskgraph metadata");
                let (after, before) = nested_images(key);
                assert_eq!(
                    outline
                        .iter()
                        .map(|line| by_asset(line, &names))
                        .collect::<Vec<_>>(),
                    [
                        format!("section Visual change of the {what_kind}"),
                        by_asset(&format!("  image ![After]({after})"), &names),
                        "  section Before".to_owned(),
                        by_asset(&format!("    image ![Before]({before})"), &names),
                    ],
                    "{what}: the editor state nests the sections, each image in its own"
                );
            } else {
                // The authored form, byte for byte: a folder's file as authored, a board's body
                // as authored with each asset at the address the board serves it from.
                assert_eq!(
                    by_asset(raw.trim_end_matches('\n'), &names),
                    by_asset(body.trim_end_matches('\n'), &names),
                    "{what}: held in the authored form"
                );
                if kind == "local-md" {
                    assert_eq!(raw, *body, "{what}: the folder holds it byte for byte");
                }
            }
            // Read back through the store: the authored nested blocks, byte for byte, each
            // image at the address its destination serves it from.
            assert_eq!(
                by_asset(at.content(&sandbox).trim_end_matches('\n'), &names),
                by_asset(body.trim_end_matches('\n'), &names),
                "{what}: reads back as the authored nested blocks"
            );
        }
        let root = (kind == "local-md").then(|| root_of(&sandbox, "dst"));
        repeats_settling(
            &sandbox,
            &format!("{kind}: nested"),
            &|| {
                let again = copy_nested(&sandbox);
                for (key, record) in &landed {
                    assert_eq!(again[key].id, record.id, "{kind}: {key} landed once");
                }
            },
            (&dst, root.as_deref()),
            (&landed, false),
            &[],
        );
    }
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] No service and no credential: a folder of Markdown and the loopback Linear workspace.
#[test]
fn a_nested_block_linear_cannot_spell_or_holds_as_text_stays_as_it_stands_inside_its_section() {
    let plan = LinearPlan::new();
    let task = &plan.landed["T"];
    let set = |name: &str, body: &str| {
        let file = plan.file(name, body);
        ok(
            &plan.sandbox,
            &["task", "content", "set", &task.id, "--file", &file],
        );
    };

    // An inner block whose summary's text holds a literal backtick, which Linear would read as a
    // code mark: carried as HTML inside the outer block's section, and read back as authored.
    let unspellable = "<details>\n<summary>A `tick` inside</summary>\n\nInner text.\n\n</details>";
    let body = format!(
        "<details>\n<summary>Outer</summary>\n\nOuter text.\n\n{unspellable}\n\n</details>"
    );
    set("unspellable.md", &body);
    assert_eq!(
        plan.stored(task),
        format!("+++ Outer\n\nOuter text.\n\n{unspellable}\n\n+++"),
        "the outer block is a section, the inner one carried as it stands inside it"
    );
    assert_eq!(
        task.content(&plan.sandbox),
        body,
        "and it reads back as authored"
    );

    // An inner block Linear holds as literal text inside a section reads back inside the outer
    // block, and set back as it reads it is written back as Linear stored it.
    let literal = "<details>\n<summary>Held as text</summary>\n\n- a list\n\n</details>";
    plan.hold_as_text(
        task,
        &format!("+++ Outer\n\nOuter text.\n\n{literal}\n\n+++\n"),
    );
    let held = plan.stored(task);
    let stored_literal = literal.replace("- a list", "* a list");
    assert_eq!(
        held,
        format!("+++ Outer\n\nOuter text.\n\n{stored_literal}\n\n+++"),
        "held as literal text inside the section"
    );
    let read = task.content(&plan.sandbox);
    assert_eq!(
        read,
        format!(
            "<details>\n<summary>Outer</summary>\n\nOuter text.\n\n{stored_literal}\n\n</details>"
        ),
        "the section reads back as the outer block, the literal one inside it"
    );
    set("read.md", &read);
    assert_eq!(plan.stored(task), held, "written back as Linear stored it");
    // Edited, it is the caller's own: the inner block is a section inside the outer one.
    set("edited.md", &read.replace("Outer text.", "Edited text."));
    assert_eq!(
        plan.stored(task),
        "+++ Outer\n\nEdited text.\n\n+++ Held as text\n\n* a list\n\n+++\n\n+++",
        "a caller's own body lands the inner block as a section"
    );
}
