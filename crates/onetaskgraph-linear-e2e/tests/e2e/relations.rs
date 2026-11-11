//! Task parents and typed links on a Linear team, through the binary against the loopback
//! Linear, each read back by a fresh process's `task relations` and held to what the loopback
//! was sent and holds: a parent written through `issueUpdate(parentId)`, a three-level chain read
//! from either end, a cycle and a destination limit refusing their edge alone, a held version
//! moved; a link mirrored as Linear's `related` relation beside its metadata and removed from
//! both, one with no mirror sending no native write, a native relation kept until the last
//! mirrored link between its pair goes, an end a whole-body rewrite lost reported and restored,
//! an endpoint's write refused answered `partial` and completed by repeating the call, Linear's
//! `*` bullets read back without the section, and each word a Linear source cannot mirror a link
//! as refused at load.
//!
//! `measure_linear_requests_per_relation_batch` records the figure for the
//! `linear-requests-per-relation-batch` budget, with each call's requests apart.

use std::path::PathBuf;

use onetaskgraph_e2e_support::fixtures::linear_relations::{
    LINEAR_CHILD_LIMIT, LINEAR_INVALID_PARENT,
};
use onetaskgraph_linear::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_per_item, linear_workspace};
use crate::telemetry::record;

/// One issue at `Todo`.
fn issue(id: &str) -> Value {
    json!({"id": id, "title": format!("Ticket {id}"), "content": format!("The body of {id}."),
           "status": {"category": "todo", "name": "Todo"}, "labels": []})
}

struct Team {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    batches: PathBuf,
}

/// What one command printed and how it exited, and what the loopback was sent while it ran.
struct Ran {
    code: Option<i32>,
    json: Value,
    stderr: String,
    sent: Vec<(String, Value)>,
}

impl Ran {
    /// The one-item documents and variables of what was sent, each alias apart.
    fn items(&self) -> Vec<(String, Value)> {
        linear_per_item(&self.sent)
    }

    /// How many of the items sent were `single`.
    fn count(&self, single: &str) -> usize {
        self.items()
            .iter()
            .filter(|(query, _)| query == single)
            .count()
    }
}

impl Team {
    /// A team of `count` issues `R-0`…, its source `team` mirroring `caused-by` and `related`
    /// as Linear's `related` relation and keeping `mentions` in metadata alone.
    fn of(count: usize) -> Self {
        Self::configured(
            count,
            &json!({"caused-by": {"native": "related"},
                                         "related": {"native": "related"},
                                         "mentions": {}}),
        )
    }

    fn configured(count: usize, link_types: &Value) -> Self {
        Self::holding(count, link_types, &json!([]))
    }

    /// [`Team::configured`], its issues holding `dependencies` — `{from, to, kind}` — as Linear
    /// relations already.
    fn holding(count: usize, link_types: &Value, dependencies: &Value) -> Self {
        let sandbox = Sandbox::new();
        let tasks: Vec<Value> = (0..count).map(|at| issue(&format!("R-{at}"))).collect();
        let held = json!({"tasks": tasks, "projects": [], "documents": [], "labels": [],
                          "task_dependencies": dependencies, "project_dependencies": []});
        let (mut config, workspace) = linear_workspace(&sandbox, held);
        config["link_types"] = link_types.clone();
        // A folder of Markdown beside the team, for the journeys that copy into it.
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the tasks folder");
        let configuration: Value = serde_json::from_str(&document(&json!({
            "team": {"plugin": "linear", "config": config},
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        })))
        .expect("a configuration document");
        sandbox.project_document(&configuration.to_string());
        let batches = sandbox.subdirectory("batches");
        Self {
            sandbox,
            workspace,
            batches,
        }
    }

    fn run(&self, arguments: &[&str]) -> Ran {
        let mark = self.workspace.served_count();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .arg("--json")
            .write_stdin("")
            .assert()
            .get_output()
            .clone();
        let json = serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null);
        Ran {
            code: output.status.code(),
            json,
            stderr: stderr(&output),
            sent: self.workspace.served_since(mark),
        }
    }

    fn ok(&self, arguments: &[&str]) -> Ran {
        let ran = self.run(arguments);
        assert_eq!(
            ran.code,
            Some(0),
            "`{}`: {}{}",
            arguments.join(" "),
            ran.json,
            ran.stderr
        );
        ran
    }

    fn batch(&self, document: &Value) -> String {
        let path = self.batches.join(format!(
            "batch-{}.json",
            std::fs::read_dir(&self.batches).map_or(0, Iterator::count)
        ));
        std::fs::write(&path, document.to_string()).expect("a batch");
        path.to_str().expect("a UTF-8 path").to_owned()
    }

    /// `task parent set-many` of `edges`, each `(child, parent)`.
    fn parents(&self, edges: &[(&str, Option<&str>)]) -> Ran {
        let edges: Vec<Value> = edges
            .iter()
            .map(|(child, parent)| {
                json!({"child": format!("team:{child}"),
                       "parent": parent.map(|parent| format!("team:{parent}"))})
            })
            .collect();
        let input = self.batch(&json!({"schema_version": 1, "edges": edges}));
        self.run(&["task", "parent", "set-many", "--input", &input])
    }

    /// `task link add-many` or `remove-many` of `links`, each `(from, to, type)`.
    fn links(&self, verb: &str, links: &[(&str, &str, &str)]) -> Ran {
        let links: Vec<Value> = links
            .iter()
            .map(|(from, to, kind)| {
                json!({"from": format!("team:{from}"), "to": format!("team:{to}"), "type": kind})
            })
            .collect();
        let input = self.batch(&json!({"schema_version": 1, "links": links}));
        self.run(&["task", "link", verb, "--input", &input])
    }

    /// A fresh process's `task relations` of `ids`, by id.
    fn relations(&self, ids: &[&str]) -> Vec<Value> {
        let mut arguments = vec!["task".to_owned(), "relations".to_owned()];
        arguments.extend(ids.iter().map(|id| format!("team:{id}")));
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        self.ok(&arguments).json["items"]
            .as_array()
            .expect("an item per id")
            .clone()
    }

    fn version(&self, id: &str) -> String {
        self.ok(&["task", "show", &format!("team:{id}")]).json["items"][0]["item"]["version"]
            .as_str()
            .expect("a version")
            .to_owned()
    }
}

/// The outcomes of a batch's results, in order.
fn outcomes(ran: &Ran) -> Vec<String> {
    ran.json["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results: {}{}", ran.json, ran.stderr))
        .iter()
        .map(|result| result["outcome"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// A relations item's links, each as `(type, direction, other, other_end)`.
fn links_of(item: &Value) -> Vec<(String, String, String, String)> {
    item["links"]
        .as_array()
        .expect("links")
        .iter()
        .map(|link| {
            (
                link["type"].as_str().unwrap_or_default().to_owned(),
                link["direction"].as_str().unwrap_or_default().to_owned(),
                link["other"].as_str().unwrap_or_default().to_owned(),
                link["other_end"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn link(kind: &str, direction: &str, other: &str, end: &str) -> (String, String, String, String) {
    (
        kind.to_owned(),
        direction.to_owned(),
        format!("team:{other}"),
        end.to_owned(),
    )
}

#[test]
fn a_parent_is_written_through_parent_id_and_a_three_level_chain_reads_from_either_end() {
    let team = Team::of(4);
    let set = team.parents(&[("R-1", Some("R-0")), ("R-2", Some("R-1"))]);
    assert_eq!(outcomes(&set), ["written", "written"], "{}", set.json);
    let parents: Vec<Value> = set
        .items()
        .into_iter()
        .filter(|(query, _)| query == graphql::ISSUE_PARENT_UPDATE)
        .map(|(_, variables)| variables)
        .collect();
    assert_eq!(
        parents,
        [
            json!({"id": "R-1", "input": {"parentId": "R-0"}}),
            json!({"id": "R-2", "input": {"parentId": "R-1"}}),
        ]
    );
    assert_eq!(set.count(graphql::ISSUE_PARENT_UPDATE), 2);
    assert_eq!(set.sent.len(), 2, "one read and one write: {:?}", set.sent);
    assert_eq!(team.workspace.parent_of("R-2").as_deref(), Some("R-1"));

    // From the root down, from the leaf up, and from the middle.
    let items = team.relations(&["R-0", "R-1", "R-2"]);
    assert_eq!(items[0]["parent"], Value::Null);
    assert_eq!(items[0]["children"], json!(["team:R-1"]));
    assert_eq!(items[1]["parent"], json!("team:R-0"));
    assert_eq!(items[1]["children"], json!(["team:R-2"]));
    assert_eq!(items[2]["parent"], json!("team:R-1"));
    assert_eq!(items[2]["children"], json!([]));
    assert_eq!(items[2]["children_complete"], json!(true));
    let leaf = team.relations(&["R-2"]);
    assert_eq!(leaf[0]["parent"], json!("team:R-1"));

    // The same edges again write nothing; a re-parent and a removal are one write each.
    let again = team.parents(&[("R-1", Some("R-0")), ("R-2", Some("R-1"))]);
    assert_eq!(outcomes(&again), ["unchanged", "unchanged"]);
    assert_eq!(again.count(graphql::ISSUE_PARENT_UPDATE), 0);
    let moved = team.parents(&[("R-2", Some("R-3")), ("R-1", None)]);
    assert_eq!(outcomes(&moved), ["written", "written"], "{}", moved.json);
    assert!(
        moved
            .items()
            .iter()
            .any(|(query, variables)| query == graphql::ISSUE_PARENT_UPDATE
                && *variables == json!({"id": "R-1", "input": {"parentId": null}}))
    );
    let items = team.relations(&["R-0", "R-2", "R-3"]);
    assert_eq!(items[0]["children"], json!([]));
    assert_eq!(items[1]["parent"], json!("team:R-3"));
    assert_eq!(items[2]["children"], json!(["team:R-2"]));
}

#[test]
fn a_cycle_and_a_destination_limit_each_refuse_their_edge_alone_and_a_moved_version_is_named() {
    let team = Team::of(6);
    team.workspace.limit_children(1);
    let set = team.parents(&[
        ("R-1", Some("R-0")),
        ("R-2", Some("R-0")),
        ("R-0", Some("R-1")),
        ("R-4", Some("R-3")),
    ]);
    assert_eq!(
        outcomes(&set),
        ["written", "refused", "refused", "written"],
        "{}",
        set.json
    );
    let reasons: Vec<&str> = set.json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["reason"].as_str().unwrap_or_default())
        .collect();
    assert!(reasons[1].contains(LINEAR_CHILD_LIMIT), "{}", reasons[1]);
    assert!(reasons[2].contains(LINEAR_INVALID_PARENT), "{}", reasons[2]);
    assert_eq!(set.code, Some(4));
    assert_eq!(team.workspace.parent_of("R-2"), None);
    assert_eq!(team.workspace.parent_of("R-0"), None);
    assert_eq!(team.workspace.parent_of("R-4").as_deref(), Some("R-3"));

    // A version the child has moved past answers `version-moved`, naming where it is.
    let held = team.version("R-5");
    team.workspace
        .edit_description("R-5", "A person edited this.");
    let now = team.version("R-5");
    let input = team.batch(&json!({"schema_version": 1, "edges": [
        {"child": "team:R-5", "parent": "team:R-3", "if_version": held}]}));
    let moved = team.run(&["task", "parent", "set-many", "--input", &input]);
    assert_eq!(outcomes(&moved), ["version-moved"], "{}", moved.json);
    assert_eq!(moved.json["results"][0]["version"], json!(now));
    assert_eq!(moved.count(graphql::ISSUE_PARENT_UPDATE), 0);
    assert_eq!(team.workspace.parent_of("R-5"), None);
}

#[test]
fn a_mirrored_link_is_a_related_relation_beside_its_metadata_and_is_removed_from_both() {
    let team = Team::of(3);
    let added = team.links("add-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"], "{}", added.json);
    let natives: Vec<Value> = added
        .items()
        .into_iter()
        .filter(|(query, _)| query == graphql::ISSUE_RELATION_CREATE)
        .map(|(_, variables)| variables)
        .collect();
    assert_eq!(
        natives,
        [json!({"input": {"issueId": "R-0", "relatedIssueId": "R-1", "type": "related"}})]
    );
    assert_eq!(team.workspace.related_pairs().len(), 1);
    for id in ["R-0", "R-1"] {
        let stored = team.workspace.long_form("tasks", id).expect("held");
        assert!(stored.contains("## Links"), "{stored}");
        assert!(stored.contains("onetaskgraph.links"), "{stored}");
    }
    // The content reads back as it was, without the section the store rendered.
    let shown = team.ok(&["task", "show", "team:R-0"]);
    assert_eq!(
        shown.json["items"][0]["item"]["content"],
        json!("The body of R-0.")
    );
    assert_eq!(
        shown.json["items"][0]["item"]["metadata"]["onetaskgraph.links"],
        json!([{"type": "caused-by", "direction": "out", "other": "team:R-1"}])
    );

    // Read from both ends, by a fresh process, with its type and direction.
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "R-1", "present")]
    );
    assert_eq!(
        links_of(&items[1]),
        [link("caused-by", "in", "R-0", "present")]
    );

    // A repeated add is no write at all.
    let again = team.links("add-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&again), ["exists"]);
    assert_eq!(again.sent.len(), 1, "one read: {:?}", again.sent);

    let removed = team.links("remove-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&removed), ["written"], "{}", removed.json);
    assert_eq!(removed.count(graphql::ISSUE_RELATION_DELETE), 1);
    assert!(team.workspace.related_pairs().is_empty());
    let items = team.relations(&["R-0", "R-1"]);
    assert!(links_of(&items[0]).is_empty() && links_of(&items[1]).is_empty());
    for id in ["R-0", "R-1"] {
        let stored = team.workspace.long_form("tasks", id).expect("held");
        assert!(!stored.contains("## Links"), "{stored}");
    }
    let absent = team.links("remove-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&absent), ["absent"]);
}

#[test]
fn a_link_with_no_mirror_sends_no_native_write_and_reads_from_both_ends() {
    let team = Team::of(2);
    let added = team.links("add-many", &[("R-1", "R-0", "mentions")]);
    assert_eq!(outcomes(&added), ["written"], "{}", added.json);
    assert_eq!(added.count(graphql::ISSUE_RELATION_CREATE), 0);
    assert!(team.workspace.related_pairs().is_empty());
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("mentions", "in", "R-1", "present")]
    );
    assert_eq!(
        links_of(&items[1]),
        [link("mentions", "out", "R-0", "present")]
    );
    let removed = team.links("remove-many", &[("R-1", "R-0", "mentions")]);
    assert_eq!(outcomes(&removed), ["written"]);
    assert_eq!(removed.count(graphql::ISSUE_RELATION_DELETE), 0);
    assert_eq!(removed.count(graphql::ISSUE_RELATIONS), 0);
}

#[test]
fn a_native_relation_stays_until_the_last_mirrored_link_between_its_pair_is_removed() {
    let team = Team::of(2);
    let added = team.links(
        "add-many",
        &[("R-0", "R-1", "caused-by"), ("R-1", "R-0", "related")],
    );
    assert_eq!(outcomes(&added), ["written", "written"], "{}", added.json);
    assert_eq!(team.workspace.related_pairs().len(), 1);
    let first = team.links("remove-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&first), ["written"]);
    assert_eq!(first.count(graphql::ISSUE_RELATION_DELETE), 0);
    assert_eq!(team.workspace.related_pairs().len(), 1);
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("related", "in", "R-1", "present")]
    );
    let second = team.links("remove-many", &[("R-1", "R-0", "related")]);
    assert_eq!(outcomes(&second), ["written"]);
    assert_eq!(second.count(graphql::ISSUE_RELATION_DELETE), 1);
    assert!(team.workspace.related_pairs().is_empty());
}

#[test]
fn an_end_a_whole_body_rewrite_lost_reads_missing_and_a_repeated_add_restores_it() {
    let team = Team::of(2);
    let added = team.links("add-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"]);
    // A person rewrites R-1's whole description, and its metadata slot goes with it.
    team.workspace.edit_description("R-1", "Rewritten by hand.");
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "R-1", "missing")]
    );
    assert!(links_of(&items[1]).is_empty());
    let alone = team.relations(&["R-0"]);
    assert_eq!(
        links_of(&alone[0]),
        [link("caused-by", "out", "R-1", "unread")]
    );
    let restored = team.links("add-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&restored), ["written"], "{}", restored.json);
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[1]),
        [link("caused-by", "in", "R-0", "present")]
    );
    let shown = team.ok(&["task", "show", "team:R-1"]);
    assert_eq!(
        shown.json["items"][0]["item"]["content"],
        json!("Rewritten by hand.")
    );
}

#[test]
fn an_endpoint_whose_write_is_refused_answers_partial_and_repeating_the_call_completes_it() {
    let team = Team::of(2);
    team.workspace.refuse_writes_of("R-1");
    let added = team.links("add-many", &[("R-0", "R-1", "mentions")]);
    assert_eq!(outcomes(&added), ["partial"], "{}", added.json);
    let reason = added.json["results"][0]["reason"].as_str().unwrap();
    assert!(reason.starts_with("team:R-0 landed"), "{reason}");
    assert_eq!(added.code, Some(4));
    team.workspace.allow_writes_of("R-1");
    let completed = team.links("add-many", &[("R-0", "R-1", "mentions")]);
    assert_eq!(outcomes(&completed), ["written"], "{}", completed.json);
    // Only the end that had not landed is written.
    let written: Vec<String> = completed
        .items()
        .into_iter()
        .filter(|(query, _)| query == graphql::ISSUE_UPDATE_READ)
        .map(|(_, variables)| variables["id"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(written, ["R-1"]);
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("mentions", "out", "R-1", "present")]
    );
}

#[test]
fn an_endpoint_moved_between_the_read_and_the_write_is_not_written_over() {
    let team = Team::of(2);
    // A person edits R-1 just after the call read both ends, before it writes them.
    let edited = team.workspace.clone();
    team.workspace.after_request(
        |query, _| query.contains("a0: issue(id:$id_0){ id identifier title description url"),
        0,
        move |_| edited.edit_description("R-1", "Edited while the link was written."),
    );
    let added = team.links("add-many", &[("R-0", "R-1", "mentions")]);
    assert_eq!(outcomes(&added), ["partial"], "{}", added.json);
    let reason = added.json["results"][0]["reason"].as_str().unwrap();
    assert!(reason.contains("moved to version"), "{reason}");
    assert!(reason.starts_with("team:R-0 landed"), "{reason}");
    let shown = team.ok(&["task", "show", "team:R-1"]);
    assert_eq!(
        shown.json["items"][0]["item"]["content"],
        json!("Edited while the link was written.")
    );
    let completed = team.links("add-many", &[("R-0", "R-1", "mentions")]);
    assert_eq!(outcomes(&completed), ["written"]);
}

#[test]
fn a_links_section_linear_normalised_to_star_bullets_reads_back_without_the_section() {
    let team = Team::of(2);
    team.links("add-many", &[("R-0", "R-1", "caused-by")]);
    let stored = team.workspace.long_form("tasks", "R-0").expect("held");
    // What Linear stores of a `- ` bullet and the blank lines around it.
    let normalised = stored
        .replace("\n- ", "\n\n\n* ")
        .replace("## Links\n\n", "## Links\n\n\n");
    assert_ne!(normalised, stored);
    assert!(normalised.contains("\n* **caused-by**"), "{normalised}");
    team.workspace.store_description("R-0", &normalised);
    let shown = team.ok(&["task", "show", "team:R-0"]);
    assert_eq!(
        shown.json["items"][0]["item"]["content"],
        json!("The body of R-0.")
    );
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "R-1", "present")]
    );
}

/// The far ends of a `task deps` answer's edges, each with its kind.
fn far_ends(ran: &Ran, near: &str) -> Vec<(String, String)> {
    ran.json["items"]
        .as_array()
        .unwrap_or_else(|| panic!("edges: {}{}", ran.json, ran.stderr))
        .iter()
        .map(|edge| {
            let (from, to) = (
                edge["from"]["id"].as_str().unwrap_or_default(),
                edge["to"]["id"].as_str().unwrap_or_default(),
            );
            let far = if from.ends_with(near) { to } else { from };
            (
                far.trim_start_matches("team:").to_owned(),
                edge["kind"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_mirrored_link_is_no_dependency_and_a_dependency_rewrite_leaves_its_relation_alone() {
    // R-0 already has an ordinary `related` dependency on R-2.
    let team = Team::holding(
        4,
        &json!({"caused-by": {"native": "related"}}),
        &json!([{"from": "R-0", "to": "R-2", "kind": "related"}]),
    );
    let added = team.links("add-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"], "{}", added.json);
    assert_eq!(team.workspace.related_pairs().len(), 2);

    // The mirror is the link's: neither end's dependency read reports it, and the ordinary
    // `related` edge still reads, both ways.
    let forward = team.ok(&["task", "deps", "team:R-0"]);
    assert_eq!(
        far_ends(&forward, "R-0"),
        [("R-2".to_owned(), "related".to_owned())]
    );
    assert_eq!(forward.sent.len(), 1, "one read: {:?}", forward.sent);
    let reverse = team.ok(&["task", "deps", "team:R-1", "--direction", "depended-on-by"]);
    assert!(far_ends(&reverse, "R-1").is_empty(), "{}", reverse.json);
    let ordinary = team.ok(&["task", "deps", "team:R-2", "--direction", "depended-on-by"]);
    assert_eq!(
        far_ends(&ordinary, "R-2"),
        [("R-0".to_owned(), "related".to_owned())]
    );

    // Rewriting R-0's dependencies removes the ordinary edge as before and keeps the mirror.
    let rewritten = team.ok(&["task", "update", "team:R-0", "--depends-on", "team:R-3"]);
    assert_eq!(rewritten.count(graphql::ISSUE_RELATION_DELETE), 1);
    let pairs = team.workspace.related_pairs();
    assert_eq!(pairs, [("R-0".to_owned(), "R-1".to_owned())], "{pairs:?}");
    let forward = team.ok(&["task", "deps", "team:R-0"]);
    assert_eq!(
        far_ends(&forward, "R-0"),
        [("R-3".to_owned(), "blocks".to_owned())]
    );
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[1]),
        [link("caused-by", "in", "R-0", "present")]
    );
}

#[test]
fn each_word_a_linear_source_cannot_mirror_a_link_as_is_refused_at_load() {
    for word in [
        "blocks",
        "blocked-by",
        "duplicate",
        "similar",
        "frobnicates",
    ] {
        let team = Team::configured(1, &json!({"caused-by": {"native": word}}));
        let ran = team.run(&["task", "relations", "team:R-0"]);
        assert_ne!(ran.code, Some(0), "{word} was accepted");
        let said = format!("{}{}", ran.stderr, ran.json);
        for named in ["team", "caused-by", &format!("`{word}`")] {
            assert!(
                said.contains(named),
                "{word}: {named} is not named in {said}"
            );
        }
        assert!(ran.sent.is_empty(), "{word}: {:?}", ran.sent);
    }
}

#[test]
fn an_item_with_no_relation_is_written_byte_for_byte_as_it_was_before_relations() {
    // The update of an issue holding no relation sends exactly the document and the variables
    // committed below, which an update sent before relations existed.
    let team = Team::of(1);
    let body = team.batches.join("body.md");
    std::fs::write(&body, "A new body.").expect("a body file");
    let ran = team.ok(&[
        "task",
        "update",
        "team:R-0",
        "--title",
        "Retitled",
        "--body-file",
        body.to_str().expect("a UTF-8 path"),
        "--metadata",
        "caller.note=\"kept\"",
    ]);
    let writes: Vec<Value> = ran
        .items()
        .into_iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .map(|(query, variables)| json!({"query": query, "variables": variables}))
        .collect();
    let committed: Value =
        serde_json::from_str(include_str!("fixtures/unrelated_issue_update.json"))
            .expect("the committed write");
    assert_eq!(Value::Array(writes), committed);
    assert_eq!(
        team.workspace.long_form("tasks", "R-0").as_deref(),
        committed[0]["variables"]["input"]["description"].as_str()
    );
}

#[test]
fn measure_linear_requests_per_relation_batch() {
    // 20 tickets, every call a fresh process: the relations of all 20, 12 parent edges, 30
    // links every type of which is mirrored natively — then the same three calls again.
    let team = Team::configured(
        20,
        &json!({"caused-by": {"native": "related"}, "related": {"native": "related"}}),
    );
    let ids: Vec<String> = (0..20).map(|at| format!("R-{at}")).collect();
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let edges: Vec<(String, String)> = (8..20)
        .map(|child| (format!("R-{child}"), format!("R-{}", child - 8)))
        .collect();
    let edge_refs: Vec<(&str, Option<&str>)> = edges
        .iter()
        .map(|(child, parent)| (child.as_str(), Some(parent.as_str())))
        .collect();
    let mut links: Vec<(String, String, &str)> = (0..19)
        .map(|at| (format!("R-{at}"), format!("R-{}", at + 1), "caused-by"))
        .collect();
    links.extend((0..11).map(|at| (format!("R-{at}"), format!("R-{}", at + 2), "related")));
    assert_eq!(links.len(), 30);
    let link_refs: Vec<(&str, &str, &str)> = links
        .iter()
        .map(|(from, to, kind)| (from.as_str(), to.as_str(), *kind))
        .collect();
    let mut calls: Vec<(String, usize)> = Vec::new();
    let mut relations_args = vec!["task", "relations"];
    let qualified: Vec<String> = id_refs.iter().map(|id| format!("team:{id}")).collect();
    relations_args.extend(qualified.iter().map(String::as_str));
    for pass in ["first", "repeat"] {
        let read = team.ok(&relations_args);
        calls.push((format!("{pass} relations"), read.sent.len()));
        let set = team.parents(&edge_refs);
        let expected = if pass == "first" {
            "written"
        } else {
            "unchanged"
        };
        assert!(
            outcomes(&set).iter().all(|outcome| outcome == expected),
            "{}",
            set.json
        );
        calls.push((format!("{pass} parent set-many"), set.sent.len()));
        let added = team.links("add-many", &link_refs);
        let expected = if pass == "first" { "written" } else { "exists" };
        assert!(
            outcomes(&added).iter().all(|outcome| outcome == expected),
            "{}",
            added.json
        );
        if pass == "first" {
            assert_eq!(added.count(graphql::ISSUE_RELATION_CREATE), 30);
        }
        calls.push((format!("{pass} link add-many"), added.sent.len()));
    }
    assert!(team.workspace.related_pairs().len() == 30);
    let total: usize = calls.iter().map(|(_, requests)| requests).sum();
    let detail = calls
        .iter()
        .map(|(call, requests)| format!("{call}: {requests}"))
        .collect::<Vec<_>>()
        .join("; ");
    record("linear-requests-per-relation-batch", total, &detail);
}

/// The `## Links` section of a stored description — its heading up to the metadata slot.
fn links_section(stored: &str) -> String {
    let start = stored.find("## Links\n").unwrap_or(stored.len());
    let end = stored.find("<!--").unwrap_or(stored.len());
    stored[start..end.max(start)].trim_end().to_owned()
}

#[test]
fn every_content_write_of_a_linked_task_keeps_its_links_section() {
    let team = Team::of(2);
    let added = team.links("add-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"], "{}", added.json);
    let stored = team.workspace.long_form("tasks", "R-0").expect("held");
    let rendered = links_section(&stored);
    assert!(rendered.contains("[Ticket R-1]("), "{rendered}");

    let file = team.sandbox.subdirectory("bodies").join("body.md");
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("the folder");
    for (verb, text) in [
        ("content", "Rewritten by content set."),
        ("update", "Rewritten by update."),
    ] {
        std::fs::write(&file, text).expect("a body");
        let path = file.to_str().expect("a path");
        match verb {
            "content" => team.ok(&["task", "content", "set", "team:R-0", "--file", path]),
            _ => team.ok(&["task", "update", "team:R-0", "--body-file", path]),
        };
        let stored = team.workspace.long_form("tasks", "R-0").expect("held");
        assert!(
            stored.starts_with(&format!("{text}\n\n## Links\n")),
            "{verb}: {stored}"
        );
        assert_eq!(links_section(&stored), rendered, "{verb}");
        let shown = team.ok(&["task", "show", "team:R-0"]);
        assert_eq!(
            shown.json["items"][0]["item"]["content"],
            json!(text),
            "{verb}"
        );
    }
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "R-1", "present")]
    );
}

#[test]
fn writes_naming_one_child_twice_apply_in_order_each_decided_on_what_the_one_before_left() {
    let team = Team::of(4);
    let read = team.version("R-1");
    let input = team.batch(&json!({"schema_version": 1, "edges": [
        {"child": "team:R-1", "parent": "team:R-0"},
        {"child": "team:R-3", "parent": "team:R-0"},
        {"child": "team:R-1", "parent": "team:R-2"},
        {"child": "team:R-1", "parent": "team:R-2"},
        {"child": "team:R-1", "parent": null, "if_version": read},
    ]}));
    let set = team.run(&["task", "parent", "set-many", "--input", &input]);
    assert_eq!(
        outcomes(&set),
        [
            "written",
            "written",
            "written",
            "unchanged",
            "version-moved"
        ],
        "{}",
        set.json
    );
    // The later edge is answered at the version the write before it left, not the one read.
    let third = set.json["results"][2]["version"].clone();
    assert_eq!(set.json["results"][3]["version"], third);
    assert_eq!(set.json["results"][4]["version"], third);
    assert_ne!(third, json!(read));
    // The first write of each child together, then the later one alone, after it.
    let documents: Vec<Vec<Value>> = set
        .sent
        .iter()
        .filter(|(query, _)| query.contains("issueUpdate"))
        .map(|(query, variables)| {
            linear_per_item(&[(query.clone(), variables.clone())])
                .into_iter()
                .map(|(_, variables)| variables)
                .collect()
        })
        .collect();
    assert_eq!(
        documents,
        [
            vec![
                json!({"id": "R-1", "input": {"parentId": "R-0"}}),
                json!({"id": "R-3", "input": {"parentId": "R-0"}}),
            ],
            vec![json!({"id": "R-1", "input": {"parentId": "R-2"}})],
        ]
    );
    assert_eq!(team.workspace.parent_of("R-1").as_deref(), Some("R-2"));
}

#[test]
fn more_children_than_one_read_selects_answer_incomplete_and_complete_refuses_it() {
    let team = Team::of(53);
    let edges: Vec<(String, String)> = (1..=51)
        .map(|at| (format!("R-{at}"), "R-0".to_owned()))
        .collect();
    let edges: Vec<(&str, Option<&str>)> = edges
        .iter()
        .map(|(child, parent)| (child.as_str(), Some(parent.as_str())))
        .collect();
    let set = team.parents(&edges);
    assert!(outcomes(&set).iter().all(|outcome| outcome == "written"));
    let read = team.ok(&["task", "relations", "team:R-0", "team:R-52"]);
    let items = read.json["items"].as_array().expect("items");
    assert_eq!(items[0]["children_complete"], json!(false));
    assert_eq!(items[0]["children"].as_array().map(Vec::len), Some(50));
    assert_eq!(items[1]["children_complete"], json!(true));
    let refused = team.run(&["task", "relations", "team:R-0", "--complete"]);
    assert_ne!(refused.code, Some(0));
    assert_eq!(
        refused.json["failure"]["kind"],
        json!("relations-incomplete"),
        "{}{}",
        refused.json,
        refused.stderr
    );
    assert!(
        refused.json["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("team:R-0")),
        "{}",
        refused.json
    );
}

#[test]
fn a_native_relation_linear_refuses_leaves_that_link_unwritten_at_both_ends_alone() {
    let team = Team::of(4);
    // Linear refuses any write naming R-2 — the relation it is the near end of among them — and,
    // an aliased document of mutations being applied whole or not at all, refuses that document.
    team.workspace.refuse_writes_of("R-2");
    let added = team.links(
        "add-many",
        &[("R-0", "R-1", "caused-by"), ("R-2", "R-3", "caused-by")],
    );
    assert_eq!(outcomes(&added), ["written", "error"], "{}", added.json);
    let reason = added.json["results"][1]["reason"].as_str().unwrap();
    assert!(reason.contains("native `related` relation"), "{reason}");
    assert_eq!(
        team.workspace.related_pairs(),
        [("R-0".to_owned(), "R-1".to_owned())]
    );
    for id in ["R-2", "R-3"] {
        let stored = team.workspace.long_form("tasks", id).expect("held");
        assert!(!stored.contains("onetaskgraph.links"), "{id}: {stored}");
        assert!(!stored.contains("## Links"), "{id}: {stored}");
    }
    let items = team.relations(&["R-0", "R-1", "R-2", "R-3"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "R-1", "present")]
    );
    assert!(links_of(&items[2]).is_empty() && links_of(&items[3]).is_empty());
}

#[test]
fn a_rendering_of_a_linked_task_keeps_its_links_section() {
    let team = Team::of(2);
    let added = team.links("add-many", &[("R-0", "R-1", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"], "{}", added.json);
    let rendered = links_section(&team.workspace.long_form("tasks", "R-0").expect("held"));
    let template = team.sandbox.subdirectory("templates").join("note.md");
    std::fs::create_dir_all(template.parent().expect("a folder")).expect("the folder");
    std::fs::write(&template, "A rendered body.\n").expect("a template");
    let ran = team.ok(&[
        "task",
        "render",
        "team:R-0",
        "--template",
        template.to_str().expect("a path"),
        "--no-interactive",
    ]);
    assert_eq!(ran.json["changed"], json!(true), "{}", ran.json);
    let stored = team.workspace.long_form("tasks", "R-0").expect("held");
    assert!(
        stored.starts_with("A rendered body.\n\n\n## Links\n"),
        "{stored}"
    );
    assert_eq!(links_section(&stored), rendered);
    assert!(stored.contains("onetaskgraph.template"), "{stored}");
    let shown = team.ok(&["task", "show", "team:R-0"]);
    assert_eq!(
        shown.json["items"][0]["item"]["content"],
        json!("A rendered body.\n")
    );
    let items = team.relations(&["R-0", "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "R-1", "present")]
    );
}

#[test]
fn a_copy_over_a_linked_counterpart_keeps_its_links_section_rendered_from_its_links() {
    let team = Team::of(2);
    let notes = team.sandbox.subdirectory("notes").join("tasks");
    std::fs::write(
        notes.join("N-1.md"),
        "---\ntitle: A note\nstatus: todo\n---\nThe note's first body.\n",
    )
    .expect("a task file");
    let first = team.ok(&["task", "copy", "notes:N-1", "--to", "team"]);
    let landed = first.json["items"][0]["destination_id"]
        .as_str()
        .or_else(|| first.json["items"][0]["landed"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let text = first.json.to_string();
            let start = text.find("team:").expect("the copy names where it landed");
            text[start..].split('"').next().expect("an id").to_owned()
        });
    let native = landed.trim_start_matches("team:").to_owned();
    let added = team.links("add-many", &[(native.as_str(), "R-1", "caused-by")]);
    assert_eq!(outcomes(&added), ["written"], "{}", added.json);

    std::fs::write(
        notes.join("N-1.md"),
        std::fs::read_to_string(notes.join("N-1.md"))
            .expect("the note")
            .replace("The note's first body.", "The note's second body."),
    )
    .expect("the edited note");
    let again = team.ok(&["task", "copy", "notes:N-1", "--to", "team"]);
    assert!(again.json.to_string().contains("updated"), "{}", again.json);
    let stored = team.workspace.long_form("tasks", &native).expect("held");
    assert!(
        stored.starts_with("The note's second body.\n\n## Links\n"),
        "{stored}"
    );
    let section = links_section(&stored);
    assert!(section.contains("**caused-by**"), "{section}");
    assert!(section.contains("`team:R-1`"), "{section}");
    let shown = team.ok(&["task", "show", &landed]);
    assert_eq!(
        shown.json["items"][0]["item"]["content"],
        json!("The note's second body.")
    );
    let items = team.relations(&[native.as_str(), "R-1"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "R-1", "present")]
    );
}

#[test]
fn a_relation_read_linear_answers_malformed_is_refused_rather_than_read_as_no_relation() {
    type Bend = fn(&mut Value);
    let bends: [(&str, Bend); 4] = [
        ("an issue without its parent", |data| {
            data["a0"]
                .as_object_mut()
                .map(|issue| issue.remove("parent"));
        }),
        ("missing field id", |data| {
            data["a0"]["parent"] = json!({});
        }),
        ("children without pageInfo.hasNextPage", |data| {
            data["a0"]["children"]["pageInfo"] = json!({});
        }),
        ("the alias asking for R-0 with R-9", |data| {
            data["a0"]["id"] = json!("R-9");
        }),
    ];
    for (named, bend) in bends {
        let team = Team::of(2);
        team.workspace.bend_answers(
            |query, _| query.contains("a0: issue(id:$id_0){ id identifier title description url"),
            1,
            move |_, data| bend(data),
        );
        let ran = team.run(&["task", "relations", "team:R-0"]);
        assert_ne!(ran.code, Some(0), "{named}: {}", ran.json);
        let said = format!("{}{}", ran.json, ran.stderr);
        assert!(said.contains(named), "{named}: {said}");
    }
}

#[test]
fn a_mirrored_link_whose_native_relation_cannot_be_confirmed_is_not_removed_anywhere() {
    use onetaskgraph_e2e_support::fixtures::linear_relations::LINEAR_RELATIONS_UNREAD;
    for case in ["unread", "another page"] {
        let team = Team::of(2);
        let added = team.links("add-many", &[("R-0", "R-1", "caused-by")]);
        assert_eq!(outcomes(&added), ["written"], "{case}: {}", added.json);
        let before: Vec<String> = ["R-0", "R-1"]
            .iter()
            .map(|id| team.workspace.long_form("tasks", id).expect("held"))
            .collect();
        let pairs = team.workspace.related_pairs();
        let expected = match case {
            "unread" => {
                team.workspace.refuse_relation_reads();
                LINEAR_RELATIONS_UNREAD.to_owned()
            }
            _ => {
                // Linear answers the lookup with another page of relations still to read.
                team.workspace.bend_answers(
                    |query, _| query.contains("a0: issue(id:$id_0){ description relations("),
                    1,
                    |_, data| data["a0"]["relations"]["pageInfo"]["hasNextPage"] = json!(true),
                );
                "holds more relations than one page".to_owned()
            }
        };
        let removed = team.links("remove-many", &[("R-0", "R-1", "caused-by")]);
        assert_eq!(outcomes(&removed), ["error"], "{case}: {}", removed.json);
        let reason = removed.json["results"][0]["reason"].as_str().unwrap();
        assert!(
            reason.contains("native `related` relation"),
            "{case}: {reason}"
        );
        assert!(reason.contains(&expected), "{case}: {reason}");
        assert_eq!(removed.count(graphql::ISSUE_RELATION_DELETE), 0, "{case}");
        assert_eq!(removed.count(graphql::ISSUE_UPDATE_READ), 0, "{case}");
        let after: Vec<String> = ["R-0", "R-1"]
            .iter()
            .map(|id| team.workspace.long_form("tasks", id).expect("held"))
            .collect();
        assert_eq!(after, before, "{case}: both ends' metadata and sections");
        assert!(
            after.iter().all(|stored| stored.contains("## Links")),
            "{case}"
        );
        assert_eq!(team.workspace.related_pairs(), pairs, "{case}");
    }
}

#[test]
fn a_native_relation_linear_refuses_to_delete_keeps_that_link_while_the_others_are_removed() {
    let team = Team::of(4);
    let added = team.links(
        "add-many",
        &[("R-0", "R-1", "caused-by"), ("R-2", "R-3", "caused-by")],
    );
    assert_eq!(outcomes(&added), ["written", "written"], "{}", added.json);
    assert_eq!(team.workspace.related_pairs().len(), 2);
    let held: Vec<String> = ["R-2", "R-3"]
        .iter()
        .map(|id| team.workspace.long_form("tasks", id).expect("held"))
        .collect();
    // Linear refuses deleting the second pair's relation — the loopback's second relation —
    // and, an aliased document of mutations being applied whole or not at all, that document.
    team.workspace.refuse_writes_of("relation:1");
    let removed = team.links(
        "remove-many",
        &[("R-0", "R-1", "caused-by"), ("R-2", "R-3", "caused-by")],
    );
    assert_eq!(outcomes(&removed), ["written", "error"], "{}", removed.json);
    let reason = removed.json["results"][1]["reason"].as_str().unwrap();
    assert!(reason.contains("native `related` relation"), "{reason}");
    assert!(
        reason.contains(crate::fixtures::LINEAR_REFUSED_WRITE),
        "{reason}"
    );
    // The first pair is gone everywhere; the refused one is as it was at both ends.
    assert_eq!(
        team.workspace.related_pairs(),
        [("R-2".to_owned(), "R-3".to_owned())]
    );
    for id in ["R-0", "R-1"] {
        let stored = team.workspace.long_form("tasks", id).expect("held");
        assert!(!stored.contains("## Links"), "{id}: {stored}");
    }
    let after: Vec<String> = ["R-2", "R-3"]
        .iter()
        .map(|id| team.workspace.long_form("tasks", id).expect("held"))
        .collect();
    assert_eq!(after, held, "the refused pair's metadata and sections");
    let items = team.relations(&["R-2", "R-3"]);
    assert_eq!(
        links_of(&items[0]),
        [link("caused-by", "out", "R-3", "present")]
    );

    // Once Linear takes the delete again, repeating the call completes it.
    team.workspace.allow_writes_of("relation:1");
    let repeated = team.links("remove-many", &[("R-2", "R-3", "caused-by")]);
    assert_eq!(outcomes(&repeated), ["written"], "{}", repeated.json);
    assert_eq!(repeated.count(graphql::ISSUE_RELATION_DELETE), 1);
    assert!(team.workspace.related_pairs().is_empty());
    let items = team.relations(&["R-2", "R-3"]);
    assert!(links_of(&items[0]).is_empty() && links_of(&items[1]).is_empty());
}
