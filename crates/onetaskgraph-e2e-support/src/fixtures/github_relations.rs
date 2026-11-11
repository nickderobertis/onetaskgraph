//! The loopback board's relations: sub-issues read and written as GitHub reads and writes them
//! — with GitHub's own refusals of a cycle, a ninth level and a 101st child, recorded by
//! `spike-relations` — and the symmetric `relates-to` relation.
//!
//! Every aliased field is answered on its own, as GitHub answers it: a refused one is `null` at
//! its alias beside an error naming that alias as its path, and the others land.

use std::sync::{Arc, Mutex};

use onetaskgraph_github_projects::{DETAIL_BATCH, RELATION_BATCH, graphql};
use serde_json::{Value, json};

use super::{AliasAnswer, GitHubBoard, GitHubBoardFields, github_answer};

/// GitHub's refusal of a sub-issue under a parent that already has one, without `replaceParent`.
pub const ONE_PARENT: &str = "Sub issue may only have one parent";
/// GitHub's refusal of a sub-issue that would close a cycle.
pub const CIRCULAR: &str = "Sub issue may not create a circular dependency";
/// GitHub's refusal of a sub-issue that would make an eighth layer below a root.
pub const TOO_DEEP: &str = "You can't add more than 7 layers of sub-issues. To add a sub-issue, \
     remove a parent issue at any level.";
/// GitHub's refusal of a 101st sub-issue of one parent.
pub const TOO_MANY: &str = "Parent cannot have more than 100 sub-issues";

/// The most layers of sub-issues below a root GitHub accepts.
const LAYERS: usize = 7;
/// The most sub-issues of one parent GitHub accepts.
const CHILDREN: usize = 100;

/// The board's `relates-to` pairs, and how its next relation write is cut short.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] This is the loopback GitHub board's own state, and AGENTS.md places every loopback server and its state in `onetaskgraph-e2e-support` once, shared by the suites that drive it, rather than in a plugin or a suite of its own; it extends `GitHubBoard`, whose private fields only a module of `fixtures` can reach.
#[derive(Default)]
pub(super) struct Relations {
    /// Every `relates-to` pair, each spelled once in the order it was added.
    pairs: Vec<(String, String)>,
    /// Land only this many fields of the next `relates-to` write, then answer it 504.
    cut_after: Option<usize>,
    /// The request being answered was cut short, and is to be answered 504.
    timed_out: bool,
    /// Answer the next read of what issues relate to as though each had a further page.
    read_back_incomplete: bool,
    /// Answer an id a relations read names that resolves to no node as GitHub does: `null` at
    /// its alias beside a `NOT_FOUND` error naming the id, which GitHub sends for the whole
    /// document.
    unresolvable_named: bool,
    /// Answer the next relations read of this issue without its `subIssues`, once.
    malformed_once: Option<String>,
}

impl Relations {
    /// Whether the request just answered is to be answered 504; asked once per request.
    pub(super) fn timed_out(&mut self) -> bool {
        std::mem::take(&mut self.timed_out)
    }

    fn related(&self, id: &str) -> Vec<String> {
        self.pairs
            .iter()
            .filter_map(|(a, b)| {
                if a == id {
                    Some(b.clone())
                } else if b == id {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn holds(&self, a: &str, b: &str) -> bool {
        self.pairs
            .iter()
            .any(|(x, y)| (x == a && y == b) || (x == b && y == a))
    }
}

impl GitHubBoard {
    fn parent_of(&self, id: &str) -> Option<String> {
        self.items
            .iter()
            .find(|item| item["id"] == id)
            .and_then(|item| item["parent"].as_str())
            .map(str::to_owned)
    }

    fn children_of(&self, id: &str) -> Vec<String> {
        self.items
            .iter()
            .filter(|item| item["parent"] == id)
            .filter_map(|item| item["id"].as_str().map(str::to_owned))
            .collect()
    }

    /// How many layers above `id` there are: none for a root.
    fn depth(&self, id: &str) -> usize {
        let mut depth = 0;
        let mut at = self.parent_of(id);
        while let Some(parent) = at {
            depth += 1;
            if depth > 64 {
                break;
            }
            at = self.parent_of(&parent);
        }
        depth
    }

    /// How many layers of sub-issues hang below `id`.
    fn height(&self, id: &str) -> usize {
        self.children_of(id)
            .iter()
            .map(|child| 1 + self.height(child))
            .max()
            .unwrap_or(0)
    }

    /// GitHub's refusal of `child` under `parent`, or `None` where it accepts it.
    fn sub_issue_refusal(&self, parent: &str, child: &str, replace: bool) -> Option<&'static str> {
        if !replace && self.parent_of(child).is_some() {
            return Some(ONE_PARENT);
        }
        let mut at = Some(parent.to_owned());
        while let Some(ancestor) = at {
            if ancestor == child {
                return Some(CIRCULAR);
            }
            at = self.parent_of(&ancestor);
        }
        if self.depth(parent) + 1 + self.height(child) > LAYERS {
            return Some(TOO_DEEP);
        }
        if self
            .children_of(parent)
            .iter()
            .filter(|held| held.as_str() != child)
            .count()
            >= CHILDREN
        {
            return Some(TOO_MANY);
        }
        None
    }
}

impl GitHubBoardFields {
    /// The issues `id` is related to by `relates-to`, in the order the relations were added.
    #[must_use]
    pub fn relates_to(&self, id: &str) -> Vec<String> {
        self.board.lock().unwrap().relations.related(id)
    }

    /// Land only the first `landed` fields of the next `relates-to` write, and answer it HTTP
    /// 504, as GitHub's gateway answered a 60-alias `addRelatesTo` after landing 58 of its 60.
    pub fn time_out_next_relation_write_after(&self, landed: usize) {
        self.board.lock().unwrap().relations.cut_after = Some(landed);
    }

    /// Answer the next read of what issues relate to — the read a relation write whose answer
    /// never arrived is followed by — with every connection saying it has a further page.
    pub fn leave_next_relates_to_read_incomplete(&self) {
        self.board.lock().unwrap().relations.read_back_incomplete = true;
    }

    /// Answer a relations read naming an id that resolves to no node as GitHub answers it —
    /// `null` at that alias beside a `NOT_FOUND` error saying it could not resolve the id —
    /// rather than with the bare `null` every other journey's reads are answered with.
    pub fn answer_unresolvable_relation_reads_as_github_does(&self) {
        self.board.lock().unwrap().relations.unresolvable_named = true;
    }

    /// Answer the next relations read naming `id` with that issue's slot missing its
    /// `subIssues` — an answer no correct GitHub gives — once, every other slot answered
    /// rightly.
    pub fn answer_next_relations_of_without_sub_issues(&self, id: &str) {
        self.board.lock().unwrap().relations.malformed_once = Some(id.to_owned());
    }

    /// The parent issue of `id`, by its sub-issue link.
    #[must_use]
    pub fn sub_issue_parent(&self, id: &str) -> Option<String> {
        self.board.lock().unwrap().parent_of(id)
    }

    /// Make `child` a sub-issue of `parent`, as a person adding it on GitHub would.
    pub fn add_sub_issue(&self, parent: &str, child: &str) {
        let mut board = self.board.lock().unwrap();
        board.find(&json!(child))["parent"] = json!(parent);
    }
}

/// The board's answer to a relation document, or `None` for any other document.
pub(super) fn answer(
    board: &Arc<Mutex<GitHubBoard>>,
    query: &str,
    variables: &Value,
) -> Option<Value> {
    if query == graphql::ISSUE_RELATIONS {
        let mut held = board.lock().unwrap();
        let first = usize::try_from(variables["first"].as_u64().expect("$first")).unwrap();
        let mut answered = serde_json::Map::new();
        for slot in 0..DETAIL_BATCH {
            let id = variables[format!("id{slot}")].clone();
            let mut node = held.detail_node(&id, &Value::Null, &json!(100), false);
            if node.is_null() && held.relations.unresolvable_named {
                let message = format!(
                    "Could not resolve to a node with the global id of '{}'",
                    id.as_str().unwrap_or_default()
                );
                held.owed_error_objects.push(
                    json!({"type": "NOT_FOUND", "path": [format!("i{slot}")], "message": message}),
                );
            }
            if node["__typename"] == "Issue" {
                let children = held.children_of(id.as_str().expect("an id"));
                node["subIssues"] = json!({
                    "nodes": children.iter().take(first).map(|child| json!({"id": child}))
                        .collect::<Vec<_>>(),
                    "pageInfo": {"hasNextPage": children.len() > first},
                });
            }
            answered.insert(format!("i{slot}"), node);
        }
        if let Some(bent) = held.relations.malformed_once.take() {
            let mut found = false;
            for node in answered.values_mut() {
                if node["id"] == json!(bent) {
                    found = true;
                    if let Some(node) = node.as_object_mut() {
                        node.remove("subIssues");
                    }
                }
            }
            if !found {
                held.relations.malformed_once = Some(bent);
            }
        }
        return Some(Value::Object(answered));
    }
    if query == graphql::RELATES_TO_READS {
        let mut held = board.lock().unwrap();
        let incomplete = std::mem::take(&mut held.relations.read_back_incomplete);
        let mut answered = serde_json::Map::new();
        for slot in 0..DETAIL_BATCH {
            let id = variables[format!("id{slot}")].as_str().expect("an id");
            let related = held.relations.related(id);
            answered.insert(
                format!("r{slot}"),
                json!({"id": id, "relatesTo": {
                    "nodes": related.iter().map(|other| json!({"id": other})).collect::<Vec<_>>(),
                    "pageInfo": {"hasNextPage": incomplete}}}),
            );
        }
        return Some(Value::Object(answered));
    }
    if query == graphql::SET_PARENTS {
        let mut answered = serde_json::Map::new();
        for (prefix, send) in [("parent", "sendParent"), ("unparent", "sendUnparent")] {
            for slot in 0..RELATION_BATCH {
                if variables[format!("{send}{slot}")] != json!(true) {
                    continue;
                }
                let alias = format!("{prefix}{slot}");
                let input = variables[&alias].clone();
                let parent = input["issueId"].as_str().expect("issueId").to_owned();
                let child = input["subIssueId"].as_str().expect("subIssueId").to_owned();
                let refusal = if prefix == "parent" {
                    board.lock().unwrap().sub_issue_refusal(
                        &parent,
                        &child,
                        input["replaceParent"] == json!(true),
                    )
                } else {
                    None
                };
                if let Some(message) = refusal {
                    board.lock().unwrap().owed_error_objects.push(
                        json!({"type": "UNPROCESSABLE", "path": [alias], "message": message}),
                    );
                    answered.insert(alias, Value::Null);
                    continue;
                }
                let (document, root) = if prefix == "parent" {
                    (graphql::ADD_SUB_ISSUE, "addSubIssue")
                } else {
                    (graphql::REMOVE_SUB_ISSUE, "removeSubIssue")
                };
                let landed = github_answer(board, document, &json!({"input": input}));
                let mut answer = landed[root].clone();
                // A write `bend_batch_alias` names lands, and is answered malformed, once.
                let bent = {
                    let mut held = board.lock().unwrap();
                    match held.bent_alias {
                        Some((bent, how)) if bent == alias => {
                            held.bent_alias = None;
                            Some(how)
                        }
                        _ => None,
                    }
                };
                match bent {
                    Some(AliasAnswer::Missing) => continue,
                    Some(AliasAnswer::NamingAnother) => {
                        answer["subIssue"]["id"] = json!("ANOTHER-ITEM");
                    }
                    Some(AliasAnswer::NamingAnotherBlocker) => {
                        answer["issue"]["id"] = json!("ANOTHER-ITEM");
                    }
                    None => {}
                }
                answered.insert(alias, answer);
            }
        }
        return Some(Value::Object(answered));
    }
    let adding = query == graphql::ADD_RELATES_TO;
    if adding || query == graphql::REMOVE_RELATES_TO {
        let prefix = if adding { "relate" } else { "unrelate" };
        let mut held = board.lock().unwrap();
        let cut = held.relations.cut_after.take();
        let mut answered = serde_json::Map::new();
        let mut landed = 0;
        for slot in 0..RELATION_BATCH {
            if variables[format!("send{slot}")] != json!(true) {
                continue;
            }
            if cut.is_some_and(|cut| landed >= cut) {
                break;
            }
            let alias = format!("{prefix}{slot}");
            let issue = variables[&alias]["issueId"]
                .as_str()
                .expect("issueId")
                .to_owned();
            let related = variables[&alias]["relatedIssueId"]
                .as_str()
                .expect("relatedIssueId")
                .to_owned();
            if issue == related {
                held.owed_error_objects
                    .push(json!({"type": "UNPROCESSABLE", "path": [alias],
                    "message": "An issue cannot be related to itself"}));
                answered.insert(alias, Value::Null);
                continue;
            }
            if adding && !held.relations.holds(&issue, &related) {
                held.relations.pairs.push((issue.clone(), related.clone()));
            } else if !adding {
                held.relations.pairs.retain(|(a, b)| {
                    !((*a == issue && *b == related) || (*a == related && *b == issue))
                });
            }
            landed += 1;
            answered.insert(
                alias,
                json!({"issue": {"id": issue}, "relatedIssue": {"id": related}}),
            );
        }
        if cut.is_some() {
            held.relations.timed_out = true;
        }
        return Some(Value::Object(answered));
    }
    None
}
