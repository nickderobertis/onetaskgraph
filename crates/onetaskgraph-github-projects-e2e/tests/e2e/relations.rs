//! Parents and typed links on a GitHub board, through the binary against the loopback board: a
//! `sub_issues: parent` source's sub-issue links written with `addSubIssue` and read back as the
//! parent relation, a default source keeping its projects and answering no parent, and links
//! held in both endpoints' metadata beside a `## Links` section — mirrored as GitHub's own
//! `relates-to` for a type `link_types` names, in documents of at most 30 aliased fields, each
//! paced as one content creation, and answered from a read-back when GitHub's gateway gives up
//! part way.
//!
//! Every journey drives its own fresh processes and asserts on what they print and on every
//! request the board was sent.

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{AliasAnswer, GitHubBoardFields, document, github_projects_with_items};

/// One open issue on the board.
pub(crate) fn issue(id: &str, parent: Option<&str>) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Ticket {id}"),
        "body":format!("about {id}"),"state":"OPEN","reason":null,"parent":parent,
        "repo":"nickderobertis/onetaskgraph","status":"Todo","origin":"","labels":[]})
}

/// What one invocation answered.
pub(crate) struct Answered {
    pub(crate) code: Option<i32>,
    pub(crate) json: Value,
    pub(crate) stderr: String,
}

/// A board configured as one source, `board`, with `extra` laid over its configuration.
pub(crate) struct Board {
    pub(crate) sandbox: Sandbox,
    pub(crate) board: GitHubBoardFields,
}

impl Board {
    pub(crate) fn of(items: Vec<Value>, extra: &Value) -> Self {
        Self::with_source(items, extra, &json!({}))
    }

    /// [`Self::of`], with `source` laid over the source's own block — its `cache`, say — and a
    /// folder of Markdown beside it as `notes`.
    pub(crate) fn with_source(items: Vec<Value>, extra: &Value, source: &Value) -> Self {
        let sandbox = Sandbox::new();
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        for (key, value) in extra.as_object().expect("an object") {
            config[key] = value.clone();
        }
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the folder");
        let mut block = json!({"plugin": "github-projects", "config": config});
        for (key, value) in source.as_object().expect("an object") {
            block[key] = value.clone();
        }
        sandbox.project_document(&document(&json!({
            "board": block,
            "notes": {"plugin": "local-md", "config": {"root": notes,
                "status_mapping": {"Todo": "todo"}}},
        })));
        Self { sandbox, board }
    }

    pub(crate) fn run(&self, arguments: &[&str], stdin: &str) -> Answered {
        let output = self
            .sandbox
            .command()
            .env("XDG_CACHE_HOME", self.sandbox.subdirectory("cache-home"))
            .args(arguments)
            .write_stdin(stdin)
            .assert()
            .get_output()
            .clone();
        Answered {
            code: output.status.code(),
            json: serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null),
            stderr: stderr(&output),
        }
    }

    /// [`Self::run`], with every request the board served for it.
    pub(crate) fn measured(
        &self,
        arguments: &[&str],
        stdin: &str,
    ) -> (Answered, Vec<(String, Value)>) {
        let mark = self.board.served_count();
        let answered = self.run(arguments, stdin);
        (answered, self.board.served_since(mark))
    }

    /// `task parent set-many` of `edges`, each `(child, parent)`.
    pub(crate) fn parents(
        &self,
        edges: &[(&str, Option<&str>)],
    ) -> (Answered, Vec<(String, Value)>) {
        let edges: Vec<Value> = edges
            .iter()
            .map(|(child, parent)| {
                json!({"child": format!("board:{child}"),
                       "parent": parent.map(|parent| format!("board:{parent}"))})
            })
            .collect();
        self.measured(
            &["task", "parent", "set-many", "--input", "-", "--json"],
            &json!({"schema_version": 1, "edges": edges}).to_string(),
        )
    }

    /// `task link add-many` or `remove-many` of `links`, each `(from, to, type)`.
    pub(crate) fn links(
        &self,
        verb: &str,
        links: &[(&str, &str, &str)],
    ) -> (Answered, Vec<(String, Value)>) {
        let links: Vec<Value> = links
            .iter()
            .map(|(from, to, kind)| {
                json!({"from": format!("board:{from}"), "to": format!("board:{to}"), "type": kind})
            })
            .collect();
        self.measured(
            &["task", "link", verb, "--input", "-", "--json"],
            &json!({"schema_version": 1, "links": links}).to_string(),
        )
    }

    /// `task relations` of `ids`, in a fresh process, each item by its id.
    pub(crate) fn relations(&self, ids: &[&str]) -> Vec<Value> {
        let qualified: Vec<String> = ids.iter().map(|id| format!("board:{id}")).collect();
        let mut arguments = vec!["task", "relations"];
        arguments.extend(qualified.iter().map(String::as_str));
        arguments.push("--json");
        let read = self.run(&arguments, "");
        assert_eq!(read.code, Some(0), "{}", read.stderr);
        read.json["items"].as_array().expect("the items").clone()
    }
}

/// The outcomes of a batch's results, in order.
fn outcomes(answered: &Answered) -> Vec<String> {
    answered.json["results"]
        .as_array()
        .unwrap_or_else(|| panic!("no results: {:#} {}", answered.json, answered.stderr))
        .iter()
        .map(|result| result["outcome"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// Each link an item holds, as `(type, direction, other, other_end)`.
fn links_of(item: &Value) -> Vec<(String, String, String, String)> {
    item["links"]
        .as_array()
        .expect("links")
        .iter()
        .map(|link| {
            (
                link["type"].as_str().unwrap().to_owned(),
                link["direction"].as_str().unwrap().to_owned(),
                link["other"].as_str().unwrap().to_owned(),
                link["other_end"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn link(kind: &str, direction: &str, other: &str, end: &str) -> (String, String, String, String) {
    (
        kind.to_owned(),
        direction.to_owned(),
        format!("board:{other}"),
        end.to_owned(),
    )
}

/// How many requests of `served` sent `document`.
fn sent(served: &[(String, Value)], document: &str) -> usize {
    served.iter().filter(|(sent, _)| sent == document).count()
}

/// How many fields of `document` each request of `served` sending it sent, by its `send…`
/// flags.
fn fields_sent(served: &[(String, Value)], document: &str) -> Vec<usize> {
    served
        .iter()
        .filter(|(sent, _)| sent == document)
        .map(|(_, variables)| {
            variables
                .as_object()
                .unwrap()
                .iter()
                .filter(|(name, value)| name.starts_with("send") && **value == json!(true))
                .count()
        })
        .collect()
}

const PARENT: &str = r#"{"sub_issues": "parent"}"#;

fn parent_source() -> Value {
    serde_json::from_str(PARENT).unwrap()
}

#[test]
fn a_parent_source_writes_a_three_level_chain_through_add_sub_issue_and_reads_it_from_either_end() {
    let board = Board::of(
        ["A-1", "B-2", "C-3"]
            .iter()
            .map(|id| issue(id, None))
            .collect(),
        &parent_source(),
    );
    let (written, served) = board.parents(&[("B-2", Some("A-1")), ("C-3", Some("B-2"))]);
    assert_eq!(written.code, Some(0), "{}", written.stderr);
    assert_eq!(outcomes(&written), ["written", "written"]);
    assert_eq!(sent(&served, graphql::SET_PARENTS), 1, "one document");
    let (_, variables) = served
        .iter()
        .find(|(sent, _)| sent == graphql::SET_PARENTS)
        .unwrap();
    assert_eq!(
        variables["parent0"],
        json!({"issueId": "A-1", "subIssueId": "B-2"})
    );
    assert_eq!(
        variables["parent1"],
        json!({"issueId": "B-2", "subIssueId": "C-3"})
    );
    assert_eq!(board.board.sub_issue_parent("C-3").as_deref(), Some("B-2"));

    // An issue with sub-issues is a task here, never a project.
    let listed = board.run(&["task", "list", "--json"], "");
    let ids: Vec<&str> = listed.json["items"]
        .as_array()
        .expect("the tasks")
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    for id in ["board:A-1", "board:B-2", "board:C-3"] {
        assert!(ids.contains(&id), "{id} is a task: {ids:?}");
    }
    let projects = board.run(&["project", "list", "--json"], "");
    let projects: Vec<&str> = projects.json["items"]
        .as_array()
        .expect("the projects")
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert!(!projects.contains(&"board:A-1"), "{projects:?}");

    // From the top, from the middle and from the bottom, each in a fresh process.
    let top = &board.relations(&["A-1"])[0];
    assert_eq!(top["parent"], Value::Null);
    assert_eq!(top["children"], json!(["board:B-2"]));
    assert_eq!(top["children_complete"], true);
    let middle = &board.relations(&["B-2"])[0];
    assert_eq!(middle["parent"], "board:A-1");
    assert_eq!(middle["children"], json!(["board:C-3"]));
    let bottom = &board.relations(&["C-3"])[0];
    assert_eq!(bottom["parent"], "board:B-2");
    assert_eq!(bottom["children"], json!([]));

    // A re-parent replaces the parent it had; the same parent again writes nothing.
    let (moved, served) = board.parents(&[("C-3", Some("A-1")), ("B-2", Some("A-1"))]);
    assert_eq!(outcomes(&moved), ["written", "unchanged"]);
    let (_, variables) = served
        .iter()
        .find(|(sent, _)| sent == graphql::SET_PARENTS)
        .unwrap();
    assert_eq!(
        variables["parent0"],
        json!({"issueId": "A-1", "subIssueId": "C-3", "replaceParent": true})
    );
    assert_eq!(fields_sent(&served, graphql::SET_PARENTS), [1]);
    assert_eq!(board.relations(&["C-3"])[0]["parent"], "board:A-1");

    // And a removal is `removeSubIssue` from the parent it has.
    let (removed, served) = board.parents(&[("C-3", None)]);
    assert_eq!(outcomes(&removed), ["written"]);
    let (_, variables) = served
        .iter()
        .find(|(sent, _)| sent == graphql::SET_PARENTS)
        .unwrap();
    assert_eq!(variables["sendUnparent0"], true);
    assert_eq!(
        variables["unparent0"],
        json!({"issueId": "A-1", "subIssueId": "C-3"})
    );
    assert_eq!(board.relations(&["C-3"])[0]["parent"], Value::Null);
}

#[test]
fn a_parent_write_answered_about_another_issue_or_not_at_all_is_never_answered_written() {
    let board = Board::of(
        ["A-1", "B-2", "C-3", "D-4"]
            .iter()
            .map(|id| issue(id, None))
            .collect(),
        &parent_source(),
    );
    // GitHub answers each landed write naming another parent, another sub-issue, or nothing;
    // the edge beside each is written as asked.
    for (how, bent) in [
        (AliasAnswer::NamingAnotherBlocker, "parent0"),
        (AliasAnswer::NamingAnother, "parent0"),
        (AliasAnswer::Missing, "parent0"),
    ] {
        board.board.bend_batch_alias(bent, how);
        let (answered, served) = board.parents(&[("B-2", Some("A-1")), ("D-4", Some("C-3"))]);
        assert_eq!(sent(&served, graphql::SET_PARENTS), 1, "{how:?}");
        let results = answered.json["results"].as_array().expect("results");
        assert_ne!(
            results[0]["outcome"], "written",
            "{how:?}: {}",
            answered.json
        );
        assert_eq!(answered.code, Some(4), "{how:?}: {}", answered.stderr);
        let reason = results[0]["reason"].as_str().unwrap_or_default();
        if how == AliasAnswer::Missing {
            assert!(reason.contains("answered nothing"), "{how:?}: {reason}");
        } else {
            assert!(reason.contains("another issue"), "{how:?}: {reason}");
            assert!(reason.contains("B-2") && reason.contains("A-1"), "{reason}");
        }
        assert!(
            ["written", "unchanged"].contains(&results[1]["outcome"].as_str().unwrap_or_default()),
            "{how:?}: {}",
            answered.json
        );
        // Undo what landed, so the next answer is bent over the same write.
        board.parents(&[("B-2", None), ("D-4", None)]);
    }
}

#[test]
fn a_project_write_to_a_parent_source_is_refused_naming_the_setting() {
    let board = Board::of(
        vec![issue("A-1", None), issue("B-2", Some("A-1"))],
        &parent_source(),
    );
    let (refused, served) = board.measured(
        &["project", "status", "set", "board:A-1", "done", "--json"],
        "",
    );
    assert_ne!(refused.code, Some(0));
    assert!(
        refused.stderr.contains("sub_issues: parent"),
        "{}",
        refused.stderr
    );
    assert!(
        served.iter().all(|(sent, _)| !sent.starts_with("mutation")),
        "nothing was written"
    );
}

#[test]
fn every_project_write_to_a_parent_source_is_refused_naming_the_setting_and_sends_no_mutation() {
    let board = Board::of(
        vec![issue("A-1", None), issue("B-2", Some("A-1"))],
        &parent_source(),
    );
    let notes = board.sandbox.subdirectory("notes");
    std::fs::create_dir_all(notes.join("projects")).expect("the projects folder");
    std::fs::write(
        notes.join("projects").join("plan.md"),
        "---\ntitle: A plan\nstatus: todo\n---\nThe plan.\n",
    )
    .expect("a project");
    let template = board.sandbox.subdirectory("templates").join("plan.md");
    std::fs::write(&template, "A rendered plan.\n").expect("a template");
    let template = template.to_str().expect("a UTF-8 path");
    let verbs: [&[&str]; 5] = [
        &[
            "task",
            "create",
            "board",
            "--title",
            "T",
            "--project",
            "board:A-1",
        ],
        &["project", "create", "board", "--id", "P-9", "--title", "P"],
        &[
            "project",
            "metadata",
            "set",
            "board:A-1",
            "caller.key",
            r#""x""#,
        ],
        &["project", "render", "board:A-1", "--template", template],
        &["project", "copy", "notes:plan", "--to", "board"],
    ];
    for verb in verbs {
        let mut arguments = verb.to_vec();
        arguments.push("--json");
        let (refused, served) = board.measured(&arguments, "");
        let said = format!("{}{}", refused.json, refused.stderr);
        assert_ne!(refused.code, Some(0), "`{}`: {said}", verb.join(" "));
        // A render reads the project first, and a source with no projects has none to render.
        let named = if verb[1] == "render" {
            "no project with the id board:A-1"
        } else {
            "sub_issues: parent"
        };
        assert!(said.contains(named), "`{}`: {said}", verb.join(" "));
        assert!(
            served.iter().all(|(sent, _)| !sent.starts_with("mutation")),
            "`{}` wrote nothing",
            verb.join(" ")
        );
    }
    assert_eq!(board.board.sub_issue_parent("B-2").as_deref(), Some("A-1"));
}

#[test]
fn a_default_source_keeps_its_projects_and_reports_a_member_as_no_parent_relation() {
    let board = Board::of(
        vec![issue("P-4", None), issue("T-5", Some("P-4"))],
        &json!({}),
    );
    let projects = board.run(&["project", "list", "--json"], "");
    assert!(
        projects.json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "board:P-4"),
        "{:#}",
        projects.json
    );
    let member = &board.relations(&["T-5"])[0];
    assert_eq!(member["parent"], Value::Null);
    assert_eq!(member["children"], json!([]));
    // A parent write is refused for that edge, naming the setting, and nothing is sent.
    let (refused, served) = board.parents(&[("T-5", None)]);
    assert_eq!(outcomes(&refused), ["error"]);
    let reason = refused.json["results"][0]["reason"].as_str().unwrap();
    assert!(reason.contains("sub_issues"), "{reason}");
    assert_eq!(sent(&served, graphql::SET_PARENTS), 0);
}

#[test]
fn a_destination_limit_refuses_that_edge_alone_in_github_s_own_words() {
    // A chain of eight levels — a root and seven layers of sub-issues — is accepted, and a ninth
    // level refused; a parent holding 100 sub-issues refuses a 101st; each beside an edge that
    // lands.
    let mut items: Vec<Value> = (0..=8).map(|n| issue(&format!("L-{n}"), None)).collect();
    items.push(issue("P-4", None));
    items.extend((0..100).map(|n| issue(&format!("K-{n}"), Some("P-4"))));
    items.extend(["X-6", "Y-7", "Z-8"].iter().map(|id| issue(id, None)));
    let board = Board::of(items, &parent_source());
    let chain: Vec<(String, String)> = (1..=7)
        .map(|n| (format!("L-{n}"), format!("L-{}", n - 1)))
        .collect();
    let edges: Vec<(&str, Option<&str>)> = chain
        .iter()
        .map(|(child, parent)| (child.as_str(), Some(parent.as_str())))
        .collect();
    let (chained, _) = board.parents(&edges);
    assert_eq!(outcomes(&chained), vec!["written"; 7], "{:#}", chained.json);

    let (limited, _) = board.parents(&[
        ("L-8", Some("L-7")),
        ("X-6", Some("P-4")),
        ("Y-7", Some("Z-8")),
    ]);
    assert_eq!(
        outcomes(&limited),
        ["refused", "refused", "written"],
        "{:#}",
        limited.json
    );
    assert_eq!(limited.code, Some(4));
    let reasons: Vec<&str> = limited.json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["reason"].as_str().unwrap_or_default())
        .collect();
    assert!(
        reasons[0].contains("You can't add more than 7 layers of sub-issues"),
        "{}",
        reasons[0]
    );
    assert!(
        reasons[1].contains("Parent cannot have more than 100 sub-issues"),
        "{}",
        reasons[1]
    );
    assert_eq!(board.board.sub_issue_parent("L-8"), None);
    assert_eq!(board.board.sub_issue_parent("X-6"), None);
    assert_eq!(board.board.sub_issue_parent("Y-7").as_deref(), Some("Z-8"));
    let full = &board.relations(&["P-4"])[0];
    assert_eq!(full["children"].as_array().unwrap().len(), 100);
    assert_eq!(full["children_complete"], true);

    // A cycle is refused for that edge alone, too.
    let (cycle, _) = board.parents(&[("L-0", Some("L-7")), ("Z-8", Some("X-6"))]);
    assert_eq!(outcomes(&cycle), ["refused", "written"], "{:#}", cycle.json);
    assert!(
        cycle.json["results"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("Sub issue may not create a circular dependency")
    );
}

#[test]
fn a_parent_write_held_to_a_version_the_child_moved_past_answers_version_moved_for_that_edge() {
    let board = Board::of(
        ["A-1", "B-2", "C-3"]
            .iter()
            .map(|id| issue(id, None))
            .collect(),
        &parent_source(),
    );
    let shown = board.run(&["task", "show", "board:B-2", "--json"], "");
    let read = shown.json["items"][0]["item"]["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    board.board.edit_body("B-2", "a person edited this");
    let (moved, served) = board.measured(
        &["task", "parent", "set-many", "--input", "-", "--json"],
        &json!({"schema_version": 1, "edges": [
            {"child": "board:B-2", "parent": "board:A-1", "if_version": read},
            {"child": "board:C-3", "parent": "board:A-1"},
        ]})
        .to_string(),
    );
    assert_eq!(
        outcomes(&moved),
        ["version-moved", "written"],
        "{:#}",
        moved.json
    );
    assert_ne!(
        moved.json["results"][0]["version"].as_str(),
        Some(read.as_str())
    );
    assert_eq!(fields_sent(&served, graphql::SET_PARENTS), [1]);
    assert_eq!(board.board.sub_issue_parent("B-2"), None);
}

const MIRRORED: &str =
    r#"{"link_types": {"caused-by": {"native": "relates-to"}, "duplicate-of": {}}}"#;

fn mirrored() -> Value {
    serde_json::from_str(MIRRORED).unwrap()
}

#[test]
fn a_mirrored_link_is_written_with_add_relates_to_beside_its_metadata_and_removed_from_both() {
    let board = Board::of(
        ["A-1", "B-2", "C-3"]
            .iter()
            .map(|id| issue(id, None))
            .collect(),
        &mirrored(),
    );
    let (added, served) = board.links(
        "add-many",
        &[("A-1", "B-2", "caused-by"), ("A-1", "C-3", "duplicate-of")],
    );
    assert_eq!(added.code, Some(0), "{}", added.stderr);
    assert_eq!(outcomes(&added), ["written", "written"]);
    // One relation, for the mirrored type alone.
    assert_eq!(fields_sent(&served, graphql::ADD_RELATES_TO), [1]);
    assert_eq!(board.board.relates_to("A-1"), ["B-2"]);
    assert!(board.board.relates_to("C-3").is_empty());
    // Each endpoint's metadata and `## Links` section, in its body.
    let body = board.board.item("A-1").unwrap()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(body.contains("## Links"), "{body}");
    assert!(body.contains("onetaskgraph.links"), "{body}");
    // Read back without the section.
    let shown = board.run(&["task", "show", "board:A-1", "--json"], "");
    assert_eq!(
        shown.json["items"][0]["item"]["content"], "about A-1",
        "{:#}",
        shown.json
    );

    // Read from every endpoint in a fresh process, with its type and direction.
    let read = board.relations(&["A-1", "B-2", "C-3"]);
    assert_eq!(
        links_of(&read[0]),
        [
            link("caused-by", "out", "B-2", "present"),
            link("duplicate-of", "out", "C-3", "present")
        ]
    );
    assert_eq!(
        links_of(&read[1]),
        [link("caused-by", "in", "A-1", "present")]
    );
    assert_eq!(
        links_of(&read[2]),
        [link("duplicate-of", "in", "A-1", "present")]
    );

    // A repeated add writes nothing.
    let (again, served) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&again), ["exists"]);
    assert!(served.iter().all(|(sent, _)| !sent.starts_with("mutation")));

    // Removed from both ends, and from the relationship panel; an unmirrored removal sends no
    // native mutation.
    let (removed, served) = board.links(
        "remove-many",
        &[("A-1", "B-2", "caused-by"), ("A-1", "C-3", "duplicate-of")],
    );
    assert_eq!(outcomes(&removed), ["written", "written"]);
    assert_eq!(fields_sent(&served, graphql::REMOVE_RELATES_TO), [1]);
    assert!(board.board.relates_to("A-1").is_empty());
    let read = board.relations(&["A-1", "B-2", "C-3"]);
    assert!(
        read.iter().all(|item| item["links"] == json!([])),
        "{read:#?}"
    );
    let (absent, served) = board.links("remove-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&absent), ["absent"]);
    assert!(served.iter().all(|(sent, _)| !sent.starts_with("mutation")));
}

#[test]
fn a_type_with_no_mirror_sends_no_native_mutation() {
    let board = Board::of(vec![issue("A-1", None), issue("B-2", None)], &mirrored());
    let (added, served) = board.links("add-many", &[("A-1", "B-2", "duplicate-of")]);
    assert_eq!(outcomes(&added), ["written"]);
    assert_eq!(sent(&served, graphql::ADD_RELATES_TO), 0);
    assert!(board.board.relates_to("A-1").is_empty());
    let read = board.relations(&["A-1", "B-2"]);
    assert_eq!(
        links_of(&read[0]),
        [link("duplicate-of", "out", "B-2", "present")]
    );
    assert_eq!(
        links_of(&read[1]),
        [link("duplicate-of", "in", "A-1", "present")]
    );
}

#[test]
fn two_mirrored_links_between_one_pair_keep_the_native_relation_until_the_second_is_removed() {
    let board = Board::of(
        vec![issue("A-1", None), issue("B-2", None)],
        &json!({"link_types": {"caused-by": {"native": "relates-to"},
                               "related": {"native": "relates-to"}}}),
    );
    let (added, _) = board.links(
        "add-many",
        &[("A-1", "B-2", "caused-by"), ("B-2", "A-1", "related")],
    );
    assert_eq!(outcomes(&added), ["written", "written"]);
    assert_eq!(board.board.relates_to("A-1"), ["B-2"]);
    let (first, served) = board.links("remove-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&first), ["written"]);
    assert_eq!(sent(&served, graphql::REMOVE_RELATES_TO), 0);
    assert_eq!(
        board.board.relates_to("A-1"),
        ["B-2"],
        "the pair still has a mirrored link"
    );
    let (second, served) = board.links("remove-many", &[("B-2", "A-1", "related")]);
    assert_eq!(outcomes(&second), ["written"]);
    assert_eq!(fields_sent(&served, graphql::REMOVE_RELATES_TO), [1]);
    assert!(board.board.relates_to("A-1").is_empty());
}

#[test]
fn an_end_a_whole_body_rewrite_lost_reads_missing_from_the_other_and_a_repeated_add_restores_it() {
    let board = Board::of(vec![issue("A-1", None), issue("B-2", None)], &json!({}));
    let (added, _) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"]);
    // A person rewrites B's whole body, taking its metadata slot with it.
    board.board.edit_body("B-2", "rewritten by hand");
    let read = board.relations(&["A-1", "B-2"]);
    assert_eq!(
        links_of(&read[0]),
        [link("caused-by", "out", "B-2", "missing")]
    );
    assert_eq!(read[1]["links"], json!([]));
    let alone = board.relations(&["A-1"]);
    assert_eq!(
        links_of(&alone[0]),
        [link("caused-by", "out", "B-2", "unread")]
    );
    let (restored, _) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&restored), ["written"]);
    let read = board.relations(&["A-1", "B-2"]);
    assert_eq!(
        links_of(&read[0]),
        [link("caused-by", "out", "B-2", "present")]
    );
    assert_eq!(
        links_of(&read[1]),
        [link("caused-by", "in", "A-1", "present")]
    );
    let shown = board.run(&["task", "show", "board:B-2", "--json"], "");
    assert_eq!(
        shown.json["items"][0]["item"]["content"],
        "rewritten by hand"
    );
}

#[test]
fn a_link_whose_second_end_is_refused_answers_partial_and_a_repeat_completes_it() {
    let board = Board::of(vec![issue("A-1", None), issue("B-2", None)], &json!({}));
    board.board.reject_mutation_of("B-2");
    let (partial, _) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&partial), ["partial"], "{:#}", partial.json);
    assert_eq!(partial.code, Some(4));
    let reason = partial.json["results"][0]["reason"].as_str().unwrap();
    assert!(reason.starts_with("board:A-1 landed"), "{reason}");
    let read = board.relations(&["A-1", "B-2"]);
    assert_eq!(
        links_of(&read[0]),
        [link("caused-by", "out", "B-2", "missing")]
    );
    let (completed, served) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&completed), ["written"]);
    // Only the end that had not landed is written.
    let updates: usize = served
        .iter()
        .filter(|(sent, _)| sent.starts_with("mutation"))
        .map(|(_, variables)| {
            variables
                .as_object()
                .unwrap()
                .iter()
                .filter(|(name, value)| name.starts_with("sendUpdate") && **value == json!(true))
                .count()
                + usize::from(variables.get("input").is_some())
        })
        .sum();
    assert_eq!(updates, 1, "{served:#?}");
    let read = board.relations(&["A-1", "B-2"]);
    assert_eq!(
        links_of(&read[1]),
        [link("caused-by", "in", "A-1", "present")]
    );
}

#[test]
fn a_mirror_batch_past_thirty_goes_in_documents_of_thirty_and_a_gateway_timeout_is_read_back() {
    let mut items = vec![issue("A-1", None)];
    items.extend((0..35).map(|n| issue(&format!("X-{n}"), None)));
    let board = Board::of(items, &mirrored());
    let others: Vec<String> = (0..35).map(|n| format!("X-{n}")).collect();
    // The second document lands three of its five and is answered 504.
    let first: Vec<(&str, &str, &str)> = others[..30]
        .iter()
        .map(|other| ("A-1", other.as_str(), "caused-by"))
        .collect();
    let (added, served) = board.links("add-many", &first);
    assert_eq!(outcomes(&added), vec!["written"; 30]);
    assert_eq!(fields_sent(&served, graphql::ADD_RELATES_TO), [30]);

    board.board.time_out_next_relation_write_after(3);
    let all: Vec<(&str, &str, &str)> = others
        .iter()
        .map(|other| ("A-1", other.as_str(), "caused-by"))
        .collect();
    let (cut, served) = board.links("add-many", &all);
    let answered = outcomes(&cut);
    assert_eq!(&answered[..30], vec!["exists"; 30]);
    assert_eq!(
        &answered[30..],
        ["written", "written", "written", "error", "error"]
    );
    assert_eq!(fields_sent(&served, graphql::ADD_RELATES_TO), [5]);
    assert_eq!(
        sent(&served, graphql::RELATES_TO_READS),
        1,
        "read back once"
    );
    // What did not land is the only thing a repeat writes.
    let (repeat, served) = board.links("add-many", &all);
    let answered = outcomes(&repeat);
    assert_eq!(
        &answered[30..],
        ["exists", "exists", "exists", "written", "written"]
    );
    let (_, variables) = served
        .iter()
        .find(|(sent, _)| sent == graphql::ADD_RELATES_TO)
        .unwrap();
    assert_eq!(fields_sent(&served, graphql::ADD_RELATES_TO), [2]);
    assert_eq!(variables["relate0"]["relatedIssueId"], "X-33");
    assert_eq!(variables["relate1"]["relatedIssueId"], "X-34");
    assert_eq!(board.board.relates_to("A-1").len(), 35);
}

#[test]
fn a_removal_document_answered_504_after_landing_part_is_read_back_and_a_repeat_removes_the_rest() {
    let mut items = vec![issue("A-1", None)];
    items.extend((0..5).map(|n| issue(&format!("X-{n}"), None)));
    let board = Board::of(items, &mirrored());
    let others: Vec<String> = (0..5).map(|n| format!("X-{n}")).collect();
    let all: Vec<(&str, &str, &str)> = others
        .iter()
        .map(|other| ("A-1", other.as_str(), "caused-by"))
        .collect();
    let (added, _) = board.links("add-many", &all);
    assert_eq!(outcomes(&added), vec!["written"; 5]);
    assert_eq!(board.board.relates_to("A-1").len(), 5);

    // The removal document lands three of its five and is answered 504: each link is answered
    // from one read-back of what A-1 relates to, a relation no longer there as removed.
    board.board.time_out_next_relation_write_after(3);
    let (cut, served) = board.links("remove-many", &all);
    assert_eq!(
        outcomes(&cut),
        ["written", "written", "written", "error", "error"],
        "{:#}",
        cut.json
    );
    assert_eq!(cut.code, Some(4));
    assert_eq!(fields_sent(&served, graphql::REMOVE_RELATES_TO), [5]);
    assert_eq!(
        sent(&served, graphql::RELATES_TO_READS),
        1,
        "read back once"
    );
    assert_eq!(board.board.relates_to("A-1"), ["X-3", "X-4"]);
    let read = board.relations(&["A-1", "X-0", "X-3"]);
    assert_eq!(
        links_of(&read[0]),
        [
            link("caused-by", "out", "X-3", "present"),
            link("caused-by", "out", "X-4", "unread"),
        ]
    );
    assert!(links_of(&read[1]).is_empty(), "{:#}", read[1]);

    // What did not come off is the only thing a repeat removes.
    let (repeat, served) = board.links("remove-many", &all);
    assert_eq!(
        outcomes(&repeat),
        ["absent", "absent", "absent", "written", "written"],
        "{:#}",
        repeat.json
    );
    let (_, variables) = served
        .iter()
        .find(|(sent, _)| sent == graphql::REMOVE_RELATES_TO)
        .unwrap();
    assert_eq!(fields_sent(&served, graphql::REMOVE_RELATES_TO), [2]);
    assert_eq!(variables["unrelate0"]["relatedIssueId"], "X-3");
    assert_eq!(variables["unrelate1"]["relatedIssueId"], "X-4");
    assert!(board.board.relates_to("A-1").is_empty());
    let read = board.relations(&["A-1", "X-3"]);
    assert!(
        read.iter().all(|item| item["links"] == json!([])),
        "{read:#?}"
    );
}

#[test]
fn a_native_word_github_cannot_mirror_is_refused_at_load_naming_the_source_the_type_and_the_word() {
    for word in ["blocks", "blocked-by", "duplicate", "similar", "frobs"] {
        let board = Board::of(
            vec![issue("A-1", None)],
            &json!({"link_types": {"caused-by": {"native": word}}}),
        );
        let refused = board.run(&["task", "list", "--json"], "");
        assert_ne!(refused.code, Some(0), "{word} was accepted");
        for named in ["board", "caused-by", &format!("`{word}`")] {
            assert!(refused.stderr.contains(named), "{word}: {}", refused.stderr);
        }
    }
}

#[test]
fn two_sources_naming_one_board_with_different_sub_issues_are_refused_naming_both() {
    let sandbox = Sandbox::new();
    let (config, _board) = github_projects_with_items(&sandbox, vec![issue("A-1", None)]);
    let mut children = config.clone();
    children["sub_issues"] = json!("parent");
    sandbox.project_document(&document(&json!({
        "plans": {"plugin": "github-projects", "config": config},
        "symptoms": {"plugin": "github-projects", "config": children},
    })));
    let output = sandbox
        .command()
        .args(["task", "list", "--json"])
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0));
    let said = stderr(&output);
    for named in [
        "sources plans and symptoms name one GitHub board, plans with `sub_issues: project` and \
         symptoms with `sub_issues: parent`; the two meanings",
        "give the `sub_issues: parent` source a board of its own, or give plans and symptoms one \
         `sub_issues`",
    ] {
        assert!(said.contains(named), "{said}");
    }
}

#[test]
fn each_aliased_relation_field_is_one_content_creation_on_the_simulated_clock_s_pacing_record() {
    use super::asset_board::{Board as Paced, creations};
    use onetaskgraph_e2e_support::{binary, clock::SimulatedClock};
    use std::process::Stdio;
    // Ninety mirrored links in three documents of thirty: two leave at once, sixty creations
    // inside GitHub's eighty a minute, and the third waits for the minute to have room for its
    // thirty, which the binary names on the clock — and the board, counting as GitHub counts,
    // refuses none.
    let clock = SimulatedClock::start(1);
    let sandbox = Sandbox::new();
    let mut items = vec![issue("A-1", None)];
    items.extend((0..90).map(|n| issue(&format!("X-{n}"), None)));
    let (mut config, github) = github_projects_with_items(&sandbox, items);
    let paced = Paced::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
    paced.enforce_limit();
    config["endpoint"] = json!(paced.endpoint);
    // GitHub's own windows, which the loopback fixture otherwise turns off.
    config["pacing"] = json!({});
    config["link_types"] = json!({"caused-by": {"native": "relates-to"}});
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": config},
    })));
    let links: Vec<Value> = (0..90)
        .map(|n| json!({"from": "board:A-1", "to": format!("board:X-{n}"), "type": "caused-by"}))
        .collect();
    let lease = clock.lease(0);
    let mut command = sandbox.subprocess(binary());
    command
        .args(["task", "link", "add-many", "--input", "-", "--json"])
        .envs(clock.client_env(0))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    std::io::Write::write_all(
        child.stdin.as_mut().unwrap(),
        json!({"schema_version": 1, "links": links})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    lease.wait_for_detach();
    drop(lease);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let calls = paced.calls();
    assert!(
        calls.iter().all(|call| call.status != 403),
        "a request was refused: {:?}",
        calls
            .iter()
            .map(|call| (call.status, creations(&call.path, &call.bytes), call.at))
            .collect::<Vec<_>>()
    );
    let mirrors: Vec<_> = calls
        .iter()
        .filter(|call| String::from_utf8_lossy(&call.bytes).contains("addRelatesTo(input:"))
        .collect();
    let charged: Vec<u64> = mirrors
        .iter()
        .map(|call| creations(&call.path, &call.bytes))
        .collect();
    assert_eq!(charged, [30, 30, 30]);
    let wait = clock
        .slept()
        .into_iter()
        .find(|slept| slept.cause.as_deref() == Some("content-creation-minute"))
        .expect("the third document waited for the minute's creations");
    assert_eq!(wait.at + wait.length, mirrors[2].at, "{wait:?}");
    assert_eq!(github.relates_to("A-1").len(), 90);
}

/// What the board stores of the issue `id`: the members a write sets.
fn stored(board: &GitHubBoardFields, id: &str) -> Value {
    let item = board.item(id).expect("the issue");
    json!({"title": item["title"], "body": item["body"], "status": item["status"],
           "state": item["state"], "parent": item["parent"], "labels": item["labels"]})
}

#[test]
fn an_item_with_no_relation_is_written_byte_for_byte_as_before_relations_existed() {
    // A task created, updated and given metadata on a source configuring a mirrored link type:
    // what the board stores of it is the committed fixture of that item's stored form, so
    // neither setting — nor the `## Links` section a read strips — touches an item holding no
    // relation.
    let board = Board::of(Vec::new(), &mirrored());
    let body = board.sandbox.subdirectory("body");
    std::fs::create_dir_all(&body).unwrap();
    let file = body.join("body.md");
    std::fs::write(&file, "A body\n\n## Links\n\nwritten by a person, kept\n").unwrap();
    let created = board.run(
        &[
            "task",
            "create",
            "board",
            "--project",
            "board:P-1",
            "--title",
            "Relationless",
            "--metadata",
            "caller.shape={\"n\":1}",
            "--body-file",
            file.to_str().unwrap(),
            "--json",
        ],
        "",
    );
    assert_eq!(created.code, Some(0), "{}", created.stderr);
    let id = created.json["items"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the created id: {:#}", created.json))
        .trim_start_matches("board:")
        .to_owned();
    let updated = board.run(
        &[
            "task",
            "update",
            &format!("board:{id}"),
            "--title",
            "Relationless, renamed",
            "--json",
        ],
        "",
    );
    assert_eq!(updated.code, Some(0), "{}", updated.stderr);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/relationless-issue.json");
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture).expect("the fixture")).unwrap();
    assert_eq!(stored(&board.board, &id), expected);
    let shown = board.run(&["task", "show", &format!("board:{id}"), "--json"], "");
    assert_eq!(
        shown.json["items"][0]["item"]["content"],
        "A body\n\n## Links\n\nwritten by a person, kept\n"
    );
}

/// The `## Links` section of a board issue's body — its heading up to the metadata slot.
fn links_section(board: &GitHubBoardFields, id: &str) -> String {
    let body = board.item(id).expect("the issue")["body"]
        .as_str()
        .expect("a body")
        .to_owned();
    let start = body.find("## Links\n").unwrap_or(body.len());
    let end = body.find("<!--").unwrap_or(body.len());
    body[start..end.max(start)].trim_end().to_owned()
}

#[test]
fn every_content_write_of_a_linked_task_keeps_its_links_section() {
    let board = Board::of(
        ["A-1", "B-2"].iter().map(|id| issue(id, None)).collect(),
        &mirrored(),
    );
    let (added, _) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"]);
    let rendered = links_section(&board.board, "A-1");
    assert!(rendered.contains("[Ticket B-2]("), "{rendered}");

    let directory = board.sandbox.subdirectory("bodies");
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("body.md");
    for (verb, text) in [
        ("content", "Rewritten by content set."),
        ("update", "Rewritten by update."),
    ] {
        std::fs::write(&file, text).unwrap();
        let path = file.to_str().unwrap();
        let written = match verb {
            "content" => board.run(
                &[
                    "task",
                    "content",
                    "set",
                    "board:A-1",
                    "--file",
                    path,
                    "--json",
                ],
                "",
            ),
            _ => board.run(
                &["task", "update", "board:A-1", "--body-file", path, "--json"],
                "",
            ),
        };
        assert_eq!(written.code, Some(0), "{verb}: {}", written.stderr);
        let body = board.board.item("A-1").unwrap()["body"]
            .as_str()
            .unwrap()
            .to_owned();
        // The board writes its heading anchor before the section, as before every heading.
        assert!(body.starts_with(&format!("{text}\n\n")), "{verb}: {body}");
        assert_eq!(links_section(&board.board, "A-1"), rendered, "{verb}");
        let shown = board.run(&["task", "show", "board:A-1", "--json"], "");
        assert_eq!(shown.json["items"][0]["item"]["content"], text, "{verb}");
    }
    let items = board.relations(&["A-1", "B-2"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "B-2", "present")]
    );
}

#[test]
fn the_edges_of_one_child_apply_in_batch_order_each_decided_on_what_the_one_before_left() {
    let board = Board::of(
        ["A-1", "B-2", "C-3", "D-4"]
            .iter()
            .map(|id| issue(id, None))
            .collect(),
        &parent_source(),
    );
    let (set, _) = board.parents(&[("C-3", Some("A-1")), ("D-4", Some("C-3"))]);
    assert_eq!(outcomes(&set), ["written", "written"]);

    // A removal then an addition: the addition is sent after the removal landed, as a plain
    // `addSubIssue` — beside it in one document, GitHub would run the addition first and refuse
    // it, the child still under its parent.
    let (moved, served) = board.parents(&[("C-3", None), ("C-3", Some("B-2"))]);
    assert_eq!(outcomes(&moved), ["written", "written"], "{:#}", moved.json);
    let documents: Vec<&Value> = served
        .iter()
        .filter(|(sent, _)| sent == graphql::SET_PARENTS)
        .map(|(_, variables)| variables)
        .collect();
    assert_eq!(documents.len(), 2, "one round each");
    assert_eq!(documents[0]["sendUnparent0"], true);
    assert_eq!(
        documents[1]["parent0"],
        json!({"issueId": "B-2", "subIssueId": "C-3"})
    );
    assert_eq!(board.board.sub_issue_parent("C-3").as_deref(), Some("B-2"));

    // A refused edge leaves the parent it found, and the next edge of that child is decided on
    // it: naming that parent again writes nothing.
    let (refused, served) = board.parents(&[("C-3", Some("D-4")), ("C-3", Some("B-2"))]);
    assert_eq!(
        outcomes(&refused),
        ["refused", "unchanged"],
        "{:#}",
        refused.json
    );
    assert_eq!(fields_sent(&served, graphql::SET_PARENTS), [1]);

    // A later edge held to the version the child was read at finds it moved by the earlier one.
    let shown = board.run(&["task", "show", "board:C-3", "--json"], "");
    let read = shown.json["items"][0]["item"]["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    let (held, _) = board.measured(
        &["task", "parent", "set-many", "--input", "-", "--json"],
        &json!({"schema_version": 1, "edges": [
            {"child": "board:C-3", "parent": "board:A-1"},
            {"child": "board:C-3", "parent": "board:B-2", "if_version": read},
        ]})
        .to_string(),
    );
    assert_eq!(
        outcomes(&held),
        ["written", "version-moved"],
        "{:#}",
        held.json
    );
    assert_eq!(board.board.sub_issue_parent("C-3").as_deref(), Some("A-1"));
}

/// A template rendering one line of text.
const NOTE: &str = "---\nonetaskgraph_template: 1\nvariables:\n  note:\n    description: What \
                    changed\n    type: string\n---\n# Notes\n{{ note }}\n";

#[test]
fn a_render_of_a_linked_task_keeps_its_links_section() {
    let board = Board::of(vec![issue("B-2", None)], &mirrored());
    let template = board.sandbox.project().join("note.md");
    std::fs::write(&template, NOTE).unwrap();
    let template = template.display().to_string();
    let created = board.run(
        &[
            "task",
            "create",
            "board",
            "--project",
            "board:P-1",
            "--title",
            "Rendered",
            "--template",
            &template,
            "--var",
            "note=first",
            "--no-interactive",
            "--json",
        ],
        "",
    );
    assert_eq!(created.code, Some(0), "{}", created.stderr);
    let id = created.json["items"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the created id: {:#}", created.json))
        .to_owned();
    let native = id.trim_start_matches("board:").to_owned();
    let (added, _) = board.links("add-many", &[(&native, "B-2", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"], "{:#}", added.json);
    let section = links_section(&board.board, &native);
    assert!(section.contains("[Ticket B-2]("), "{section}");

    let rendered = board.run(
        &[
            "task",
            "render",
            &id,
            "--var",
            "note=second",
            "--no-interactive",
            "--json",
        ],
        "",
    );
    assert_eq!(rendered.code, Some(0), "{}", rendered.stderr);
    let body = board.board.item(&native).unwrap()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(body.contains("second") && !body.contains("first"), "{body}");
    assert_eq!(links_section(&board.board, &native), section);
    let shown = board.run(&["task", "show", &id, "--json"], "");
    let content = shown.json["items"][0]["item"]["content"].as_str().unwrap();
    assert!(!content.contains("## Links"), "{content}");
    assert_eq!(
        links_of(&board.relations(&[&native, "B-2"])[0]),
        [link("caused-by", "out", "B-2", "present")]
    );
}

#[test]
fn a_copy_over_a_linked_counterpart_keeps_its_links_section() {
    let board = Board::of(vec![issue("B-2", None)], &mirrored());
    let notes = board.sandbox.subdirectory("notes");
    let file = notes.join("tasks/N-1.md");
    std::fs::write(
        &file,
        "---\ntitle: Noted\nstatus: Todo\n---\nFirst words.\n",
    )
    .unwrap();
    let copy = || {
        board.run(
            &["task", "copy", "notes:N-1", "--to", "board", "--json"],
            "",
        )
    };
    let first = copy();
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let id = first.json["items"][0]["destination"]
        .as_str()
        .or_else(|| first.json["items"][0]["result"]["id"].as_str())
        .unwrap_or_else(|| panic!("the counterpart: {:#}", first.json))
        .to_owned();
    let native = id.trim_start_matches("board:").to_owned();
    let (added, _) = board.links("add-many", &[(&native, "B-2", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"], "{:#}", added.json);
    let section = links_section(&board.board, &native);
    assert!(section.contains("[Ticket B-2]("), "{section}");

    std::fs::write(
        &file,
        "---\ntitle: Noted\nstatus: Todo\n---\nSecond words.\n",
    )
    .unwrap();
    let second = copy();
    assert_eq!(second.code, Some(0), "{}", second.stderr);
    assert_eq!(
        second.json["items"][0]["action"], "updated",
        "{:#}",
        second.json
    );
    let body = board.board.item(&native).unwrap()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(body.starts_with("Second words."), "{body}");
    assert_eq!(links_section(&board.board, &native), section);
    let shown = board.run(&["task", "show", &id, "--json"], "");
    assert_eq!(shown.json["items"][0]["item"]["content"], "Second words.");
    assert_eq!(
        links_of(&board.relations(&[&native, "B-2"])[0]),
        [link("caused-by", "out", "B-2", "present")]
    );
}

#[test]
fn with_the_cache_on_relations_reach_the_board_and_relation_writes_retire_the_entries_they_move() {
    let board = Board::with_source(
        ["A-1", "B-2", "C-3"]
            .iter()
            .map(|id| issue(id, None))
            .collect(),
        &json!({"sub_issues": "parent",
                "link_types": {"caused-by": {"native": "relates-to"}}}),
        &json!({"cache": {"enabled": true}}),
    );
    // A relation read is never served from the cache, a pinned entry held or not.
    let held = filled(&board, "B-2");
    for _ in 0..2 {
        let (read, served) = board.measured(&["task", "relations", "board:B-2", "--json"], "");
        assert_eq!(read.code, Some(0), "{}", read.stderr);
        assert_eq!(sent(&served, graphql::ISSUE_RELATIONS), 1, "{served:?}");
    }
    // A parent write takes the child's entry out of service: the pinned read after it is the
    // board's.
    let (set, _) = board.parents(&[("B-2", Some("A-1"))]);
    assert_eq!(outcomes(&set), ["written"]);
    let (shown, served) = pinned_show(&board, "B-2", &held);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    assert!(
        !served.is_empty(),
        "the pinned read after the write reached the board"
    );

    // A mirrored link write takes its from-end's entry out of service.
    let from = filled(&board, "C-3");
    let (added, _) = board.links("add-many", &[("C-3", "A-1", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"]);
    let (shown, served) = pinned_show(&board, "C-3", &from);
    assert!(
        !served.is_empty(),
        "the pinned read after the write reached the board"
    );
    assert_eq!(
        shown.json["items"][0]["item"]["metadata"]["onetaskgraph.links"][0]["other"], "board:A-1",
        "{:#}",
        shown.json
    );
}

/// `task show` of the board task `id`, pinned to `version`, with every request it sent.
fn pinned_show(board: &Board, id: &str, version: &str) -> (Answered, Vec<(String, Value)>) {
    board.measured(
        &[
            "task",
            "show",
            &format!("board:{id}"),
            "--pin",
            version,
            "--json",
        ],
        "",
    )
}

/// Fill the cache entry of the board task `id` by a read pinned to the version an unpinned
/// read answered, hold it to serving the next such read, and answer that version.
fn filled(board: &Board, id: &str) -> String {
    let shown = board.run(&["task", "show", &format!("board:{id}"), "--json"], "");
    let version = shown.json["items"][0]["item"]["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    pinned_show(board, id, &version);
    let (_, served) = pinned_show(board, id, &version);
    assert!(
        served.is_empty(),
        "{id}'s entry serves a pinned read: {served:?}"
    );
    version
}

#[test]
fn a_removal_whose_second_end_is_refused_answers_partial_and_a_repeat_finishes_it() {
    let board = Board::of(vec![issue("A-1", None), issue("B-2", None)], &mirrored());
    let (added, _) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"]);
    assert_eq!(board.board.relates_to("A-1"), ["B-2"]);

    // The native relation goes first, then A-1's end lands and B-2's is refused.
    board.board.reject_mutation_of("B-2");
    let (partial, served) = board.links("remove-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&partial), ["partial"], "{:#}", partial.json);
    assert_eq!(partial.code, Some(4));
    let reason = partial.json["results"][0]["reason"].as_str().unwrap();
    assert!(reason.starts_with("board:A-1 landed"), "{reason}");
    assert_eq!(fields_sent(&served, graphql::REMOVE_RELATES_TO), [1]);
    assert!(board.board.relates_to("A-1").is_empty());
    let read = board.relations(&["A-1", "B-2"]);
    assert!(links_of(&read[0]).is_empty());
    assert_eq!(
        links_of(&read[1]),
        [link("caused-by", "in", "A-1", "missing")]
    );

    // Repeating the removal takes the end that remains, and the pair stays unrelated.
    let (finished, _) = board.links("remove-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&finished), ["written"], "{:#}", finished.json);
    let read = board.relations(&["A-1", "B-2"]);
    assert!(read.iter().all(|item| links_of(item).is_empty()));
    assert!(board.board.relates_to("A-1").is_empty());
    assert!(board.board.relates_to("B-2").is_empty());
    for id in ["A-1", "B-2"] {
        let body = board.board.item(id).unwrap()["body"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!body.contains("## Links"), "{id}: {body}");
        assert!(!body.contains("onetaskgraph.links"), "{id}: {body}");
    }
}

#[test]
fn a_read_back_that_cannot_establish_a_relation_leaves_the_link_unwritten_until_a_repeat() {
    // A mirror document lands its one relation and is answered 504; the read-back that follows
    // is answered incomplete, and on a second attempt with an error beside its alias. Neither
    // establishes the relation, so the link is `error` and neither end's metadata or `## Links`
    // moves; once the board answers normally, a repeat completes it.
    let board = Board::of(vec![issue("A-1", None), issue("B-2", None)], &mirrored());
    let bodies = |board: &Board| {
        ["A-1", "B-2"].map(|id| {
            board.board.item(id).unwrap()["body"]
                .as_str()
                .unwrap()
                .to_owned()
        })
    };
    let before = bodies(&board);
    for unreadable in ["incomplete", "an error beside its alias"] {
        board.board.time_out_next_relation_write_after(1);
        if unreadable == "incomplete" {
            board.board.leave_next_relates_to_read_incomplete();
        } else {
            board
                .board
                .error_beside_alias_once("r0", "Something went wrong while executing your query");
        }
        let (cut, served) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
        assert_eq!(outcomes(&cut), ["error"], "{unreadable}: {:#}", cut.json);
        assert_eq!(cut.code, Some(4));
        assert_eq!(sent(&served, graphql::RELATES_TO_READS), 1, "{unreadable}");
        assert_eq!(bodies(&board), before, "{unreadable}: no end was written");
        let read = board.relations(&["A-1", "B-2"]);
        assert!(
            read.iter().all(|item| item["links"] == json!([])),
            "{read:#?}"
        );
    }
    // The relation landed each time; the repeat sends it again and lands both ends.
    assert_eq!(board.board.relates_to("A-1"), ["B-2"]);
    let (repeat, served) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&repeat), ["written"], "{:#}", repeat.json);
    assert_eq!(fields_sent(&served, graphql::ADD_RELATES_TO), [1]);
    let read = board.relations(&["A-1", "B-2"]);
    assert_eq!(
        links_of(&read[0]),
        [link("caused-by", "out", "B-2", "present")]
    );
    assert_eq!(
        links_of(&read[1]),
        [link("caused-by", "in", "A-1", "present")]
    );
    assert!(links_section(&board.board, "A-1").contains("[Ticket B-2]("));
}

#[test]
fn a_relations_read_answered_with_an_error_beside_an_alias_is_refused_never_a_whole_answer() {
    let board = Board::of(
        vec![issue("A-1", None), issue("B-2", Some("A-1"))],
        &parent_source(),
    );
    board
        .board
        .error_beside_alias_once("i0", "Something went wrong while executing your query");
    let (read, served) = board.measured(&["task", "relations", "board:A-1", "--json"], "");
    assert_eq!(sent(&served, graphql::ISSUE_RELATIONS), 1);
    assert_ne!(read.code, Some(0), "{:#}", read.json);
    assert!(read.json.get("items").is_none(), "{:#}", read.json);
    assert!(
        read.stderr
            .contains("Something went wrong while executing your query"),
        "{}",
        read.stderr
    );
    // Answered normally, the same read is whole.
    assert_eq!(
        board.relations(&["A-1"])[0]["children"],
        json!(["board:B-2"])
    );
}

/// A template rendering one line of text and one picture.
const PICTURE: &str = "---\nonetaskgraph_template: 1\nvariables:\n  note:\n    description: \
                       What changed\n    type: string\n---\n# The change\n{{ note }}\n\
                       ![shot](./shot.png)\n";

#[test]
fn a_render_with_image_assets_of_a_linked_task_keeps_its_links_section() {
    use super::asset_board::Board as Attachments;
    use onetaskgraph_e2e_support::{binary, clock::SimulatedClock, images};
    // The board behind the attachment host every GitHub asset journey uploads through, on a
    // simulated clock.
    let clock = SimulatedClock::start(1);
    let sandbox = Sandbox::new();
    let (mut config, github) = github_projects_with_items(&sandbox, vec![issue("B-2", None)]);
    let attachments = Attachments::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
    config["endpoint"] = json!(attachments.endpoint);
    config["link_types"] = json!({"caused-by": {"native": "relates-to"}});
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": config},
    })));
    let template = sandbox.project().join("picture.md");
    std::fs::write(&template, PICTURE).unwrap();
    let template = template.display().to_string();
    let picture = |name: &str, seed: u64| {
        let directory = sandbox.subdirectory(name);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("shot.png");
        std::fs::write(&path, images::png(seed, 64 * 1024)).unwrap();
        path.display().to_string()
    };
    let run = |arguments: &[&str], stdin: &str| {
        let lease = clock.lease(0);
        let output = sandbox
            .subprocess(binary())
            .args(arguments)
            .envs(clock.client_env(0))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                std::io::Write::write_all(child.stdin.as_mut().unwrap(), stdin.as_bytes())?;
                drop(child.stdin.take());
                child.wait_with_output()
            })
            .unwrap();
        lease.wait_for_detach();
        drop(lease);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str::<Value>(&stdout(&output)).expect("JSON")
    };
    let first = picture("first", 0xA1);
    let created = run(
        &[
            "task",
            "create",
            "board",
            "--project",
            "board:P-1",
            "--title",
            "Pictures",
            "--template",
            &template,
            "--var",
            "note=created",
            "--no-interactive",
            "--asset",
            &first,
            "--json",
        ],
        "",
    );
    let id = created["items"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the created id: {created:#}"))
        .to_owned();
    let native = id.trim_start_matches("board:").to_owned();
    let added = run(
        &["task", "link", "add-many", "--input", "-", "--json"],
        &json!({"schema_version": 1, "links": [
            {"from": id, "to": "board:B-2", "type": "caused-by"}]})
        .to_string(),
    );
    assert_eq!(added["results"][0]["outcome"], "written", "{added:#}");
    let section = links_section(&github, &native);
    assert!(section.contains("[Ticket B-2]("), "{section}");

    // Rendered again with new words and a new picture: the section is re-rendered unchanged.
    let second = picture("second", 0xB2);
    run(
        &[
            "task",
            "render",
            &id,
            "--var",
            "note=rendered",
            "--no-interactive",
            "--asset",
            &second,
            "--json",
        ],
        "",
    );
    let body = github.item(&native).unwrap()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        body.contains("rendered") && !body.contains("created"),
        "{body}"
    );
    assert_eq!(links_section(&github, &native), section);
    let shown = run(&["task", "show", &id, "--json"], "");
    let content = shown["items"][0]["item"]["content"].as_str().unwrap();
    assert!(content.contains("rendered"), "{content}");
    assert!(!content.contains("## Links"), "{content}");
    assert!(
        shown["items"][0]["item"]["metadata"]["onetaskgraph.assets"]["shot.png"].is_object(),
        "{shown:#}"
    );
    assert_eq!(github.relates_to(&native), ["B-2"]);
}

#[test]
fn with_the_cache_on_a_link_removal_retires_its_from_end_s_entry() {
    let board = Board::with_source(
        ["A-1", "B-2"].iter().map(|id| issue(id, None)).collect(),
        &mirrored(),
        &json!({"cache": {"enabled": true}}),
    );
    let (added, _) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"]);
    let held = filled(&board, "A-1");
    let (removed, served) = board.links("remove-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&removed), ["written"]);
    assert_eq!(fields_sent(&served, graphql::REMOVE_RELATES_TO), [1]);
    let (shown, served) = pinned_show(&board, "A-1", &held);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    assert!(
        !served.is_empty(),
        "the pinned read after the removal reached the board"
    );
    assert!(
        shown.json["items"][0]["item"]["metadata"]
            .get("onetaskgraph.links")
            .is_none(),
        "the pre-removal entry was served: {:#}",
        shown.json
    );
}

#[test]
fn an_id_github_cannot_resolve_is_that_items_answer_alone_and_every_other_item_is_written() {
    // GitHub answers a read naming an id that resolves to no node with `null` at its alias and
    // a `NOT_FOUND` error for the whole document. That id is its own item's answer: the read is
    // made again an id at a time, and every other item of the same call is written as it would
    // be had the bad id not been in it.
    let mut extra = mirrored();
    extra["sub_issues"] = json!("parent");
    let board = Board::of(
        ["A-1", "B-2", "C-3"]
            .iter()
            .map(|id| issue(id, None))
            .collect(),
        &extra,
    );
    board
        .board
        .answer_unresolvable_relation_reads_as_github_does();

    let (added, served) = board.links(
        "add-many",
        &[("A-1", "NOPE", "caused-by"), ("B-2", "C-3", "caused-by")],
    );
    assert_eq!(outcomes(&added), ["error", "written"], "{:#}", added.json);
    assert_eq!(added.code, Some(4), "an item's error still fails the call");
    let reason = added.json["results"][0]["reason"]
        .as_str()
        .unwrap_or_default();
    assert!(reason.contains("board:NOPE"), "{reason}");
    assert_eq!(fields_sent(&served, graphql::ADD_RELATES_TO), [1]);
    assert_eq!(board.board.relates_to("B-2"), ["C-3"]);
    assert!(board.board.relates_to("A-1").is_empty());

    let (set, _) = board.parents(&[("NOPE", Some("A-1")), ("B-2", Some("A-1"))]);
    assert_eq!(outcomes(&set)[1], "written", "{:#}", set.json);
    assert_ne!(outcomes(&set)[0], "written", "{:#}", set.json);
    assert_eq!(board.board.sub_issue_parent("B-2").as_deref(), Some("A-1"));

    let (removed, _) = board.links(
        "remove-many",
        &[("A-1", "NOPE", "caused-by"), ("B-2", "C-3", "caused-by")],
    );
    assert_eq!(outcomes(&removed)[1], "written", "{:#}", removed.json);
    assert!(board.board.relates_to("B-2").is_empty());
    let read = board.relations(&["A-1", "B-2", "C-3"]);
    assert_eq!(read[0]["children"], json!(["board:B-2"]));
    assert!(links_of(&read[1]).is_empty() && links_of(&read[2]).is_empty());
}

#[test]
fn a_batch_recovered_an_id_at_a_time_answers_an_id_failing_otherwise_alone_and_reads_on() {
    // Twenty-five issues and an id that resolves to no node, read in two batches: the first
    // GitHub refuses whole for that id, so it is read again an id at a time — one of which GitHub
    // fails for another reason — and the second batch is read as it would be anyway.
    let ids: Vec<String> = (0..25).map(|at| format!("T-{at:02}")).collect();
    let mut extra = mirrored();
    extra["sub_issues"] = json!("parent");
    let board = Board::of(ids.iter().map(|id| issue(id, None)).collect(), &extra);
    board
        .board
        .answer_unresolvable_relation_reads_as_github_does();
    // The first id-at-a-time read with an issue at its first alias: `0-NOPE`'s own read has
    // none, so this is T-00's.
    board
        .board
        .error_beside_alias_once("i0", "Something went wrong while executing your query");
    // Every one of the twenty-six ids named: `0-NOPE` and T-00 to T-22 the first batch of
    // twenty-four, T-23 and T-24 the second.
    let mut links: Vec<(String, String)> = vec![
        ("T-01".into(), "0-NOPE".into()),
        ("T-00".into(), "T-02".into()),
        ("T-01".into(), "T-03".into()),
    ];
    links.extend(
        (4..24)
            .step_by(2)
            .map(|at| (ids[at].clone(), ids[at + 1].clone())),
    );
    links.push(("T-24".into(), "T-04".into()));
    let named: Vec<(&str, &str, &str)> = links
        .iter()
        .map(|(from, to)| (from.as_str(), to.as_str(), "caused-by"))
        .collect();
    let (added, served) = board.links("add-many", &named);
    let mut expected = vec!["error", "error"];
    expected.extend(std::iter::repeat_n("written", links.len() - 2));
    assert_eq!(outcomes(&added), expected, "{:#}", added.json);
    let reason = |at: usize| {
        added.json["results"][at]["reason"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    assert!(reason(0).contains("board:0-NOPE"), "{}", reason(0));
    assert!(
        reason(1).contains("Something went wrong while executing your query"),
        "{}",
        reason(1)
    );
    // The first batch, its twenty-four ids again one at a time, and the second batch.
    assert_eq!(sent(&served, graphql::ISSUE_RELATIONS), 1 + 24 + 1);
    assert_eq!(board.board.relates_to("T-01"), ["T-03"]);
    assert_eq!(board.board.relates_to("T-22"), ["T-23"]);
    assert_eq!(board.board.relates_to("T-24"), ["T-04"]);
    assert!(board.board.relates_to("T-00").is_empty());
}

#[test]
fn an_issue_a_relations_read_answers_malformed_is_that_items_error_alone() {
    let mut extra = mirrored();
    extra["sub_issues"] = json!("parent");
    let board = Board::of(
        ["A-1", "B-2", "C-3"]
            .iter()
            .map(|id| issue(id, None))
            .collect(),
        &extra,
    );
    board
        .board
        .answer_next_relations_of_without_sub_issues("A-1");
    let (added, served) = board.links(
        "add-many",
        &[("A-1", "B-2", "caused-by"), ("B-2", "C-3", "caused-by")],
    );
    assert_eq!(outcomes(&added), ["error", "written"], "{:#}", added.json);
    let reason = added.json["results"][0]["reason"]
        .as_str()
        .unwrap_or_default();
    assert!(reason.contains("missing subIssues"), "{reason}");
    assert_eq!(sent(&served, graphql::ISSUE_RELATIONS), 1);
    assert_eq!(fields_sent(&served, graphql::ADD_RELATES_TO), [1]);
    assert_eq!(board.board.relates_to("B-2"), ["C-3"]);
    assert!(board.board.relates_to("A-1").is_empty());
    // Answered rightly, the same link is written.
    let (added, _) = board.links("add-many", &[("A-1", "B-2", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"], "{:#}", added.json);
}
