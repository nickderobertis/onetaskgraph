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
        region("a nested block", "NESTED", "A13"),
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
/// metadata slot, without the line breaks that end it.
fn visible(long_form: &str) -> &str {
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
                    let sent: Vec<String> = workspace
                        .served()
                        .into_iter()
                        .filter(|(query, variables)| {
                            query.contains(create) && variables["input"]["title"] == json!(title)
                        })
                        .map(|(_, variables)| {
                            variables["input"][member]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned()
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
        // margin, and one never closed; and of sections nested in Linear, the outer reads back
        // as the authored block, the inner kept as it is, because authored blocks do not nest.
        for (stored, read) in [
            ("+++ \n\nUnder no summary.\n\n+++", None),
            ("+++ Tight\nNo margin above.\n\n+++", None),
            ("+++ Open\n\nNever closed.", None),
            (
                "+++ Outer\n\nOuter text.\n\n+++ Inner\n\nInner text.\n\n+++\n\n+++",
                Some(
                    "<details>\n<summary>Outer</summary>\n\nOuter text.\n\n+++ Inner\n\nInner \
                     text.\n\n+++\n\n</details>",
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

/// The project's description.
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
    // The last hop, through Linear.
    for (key, body) in bodies {
        let content = at_last[key].content(&sandbox);
        let (authored, last) = (by_asset(body, &names), by_asset(&content, &names));
        // Every block's structure lines and summary, byte for byte.
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
        // Every image kept.
        assert_eq!(
            last.matches("](asset:").count(),
            authored.matches("](asset:").count(),
            "{key}: every image reference is kept:\n{last}"
        );
        // Once: every line that differs is accounted for by one recorded Linear normalization.
        for hunk in hunks(&authored, &last) {
            assert!(
                accounting_entry(&hunk).is_some(),
                "{key}: {hunk:?} is no recorded Linear normalization:\n{last}"
            );
        }
        // Twice: the two render to the same HTML.
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
