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
use crate::section_links::{Landed, Store, configure, ok, outcomes, root_of, store};

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
/// what it is: `P`, `T`, `D`, and `comment` for the comment carried on `T`.
fn raw_bodies(
    sandbox: &Sandbox,
    store: &Store,
    landed: &BTreeMap<&'static str, Landed>,
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
            let comments = board.comments_on(landed["T"].native());
            bodies.insert("comment", only_comment(&comments));
        }
        Store::Linear(workspace) => {
            for key in ["P", "T", "D"] {
                bodies.insert(key, store.raw(&landed[key]));
            }
            let comments = workspace.comments_on(landed["T"].native());
            bodies.insert("comment", only_comment(&comments));
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
    for (key, raw) in raw_bodies(sandbox, store, landed) {
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

/// Repeat the copy of `from` into `to` twice: the first may refresh the copy record a `--comments`
/// copy keeps — the copy before it recorded its link at the source, which moved the source's
/// version — but leaves every body `landed` names byte for byte as it was, and the second sends
/// `store` no write at all — `root` the folder's, for a folder of Markdown.
fn repeats_writing_nothing(
    sandbox: &Sandbox,
    what: &str,
    from: &BTreeMap<&'static str, Landed>,
    to: &str,
    (store, root): (&Store, Option<&Path>),
    landed: &BTreeMap<&'static str, Landed>,
) {
    let before = raw_bodies(sandbox, store, landed);
    copy_plan(sandbox, from, to);
    let after = raw_bodies(sandbox, store, landed);
    for (key, body) in &before {
        let unslotted = |body: &str| {
            body.split("\nonetaskgraph.copied_version")
                .next()
                .map(str::to_owned)
        };
        if matches!(store, Store::Local) {
            // A file's front matter holds the copy record; its content is what follows it.
            let content = |body: &str| body.splitn(3, "---\n").nth(2).map(str::to_owned);
            assert_eq!(
                content(&after[key]),
                content(body),
                "{what}: {key} is unchanged"
            );
        } else {
            assert_eq!(
                unslotted(&after[key]),
                unslotted(body),
                "{what}: {key} is unchanged"
            );
        }
    }
    let mark = store.mark(root);
    let again = copy_plan(sandbox, from, to);
    let writes = store.writes_since(&mark, root);
    assert_eq!(
        writes,
        Vec::<String>::new(),
        "{what}: a repeat copy of the unchanged project writes nothing"
    );
    for (key, record) in landed {
        assert_eq!(again[key].id, record.id, "{what}: {key} landed once");
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
            repeats_writing_nothing(
                &sandbox,
                &what,
                &ids,
                "dst",
                (&dst, root.as_deref()),
                &landed,
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
    let unrecorded = |what, key, summary: &str| Shape {
        what,
        verb: "task",
        key,
        authored: format!(
            "<details>\n<summary>{summary}</summary>\n\nA paragraph of its body.\n\n</details>\n"
        ),
        recorded: None,
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
            "Uses <em>emphasis</em> here",
        ),
        unrecorded("a summary holding a raw &", "AMP", "Questions & answers"),
    ]
}

/// The visible part of a long form a hosted store holds or was sent: what comes before its
/// metadata slot, without the line breaks that end it.
fn visible(long_form: &str) -> &str {
    long_form
        .split("<!-- onetaskgraph.metadata")
        .next()
        .unwrap_or_default()
        .trim_end_matches('\n')
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
            let authored = shape.authored.trim_end_matches('\n');
            match &dst {
                // Byte for byte: read back, and as the folder or the board itself holds it.
                Store::Local => {
                    assert_eq!(
                        at.content(&sandbox).trim_end_matches('\n'),
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
                        at.content(&sandbox).trim_end_matches('\n'),
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
                            recorded.trim_end_matches('\n'),
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
