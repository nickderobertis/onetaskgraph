//! How the loopback Linear answers the relation calls' own documents — an issue's `parent` and
//! `children`, and `issueUpdate(input: {parentId})` — as spike `spike-relations` observed real
//! Linear answer them on 2026-11-08: a parent set, moved and (with `null`) removed by the one
//! mutation, a cycle refused `invalid parent`, and the child's `updatedAt` moved by each.
//!
//! A child of the fixture module, so it answers from the same held workspace and helpers the
//! rest of the loopback does; `linear_response` hands it the two documents below first.

use onetaskgraph_linear::graphql;
use serde_json::{Value, json};

use super::{LINEAR_REFUSED_WRITE, LinearWorkspace, linear_task, linear_tick};

/// What Linear answers a parent write that would close a cycle with (spike request 72).
pub const LINEAR_INVALID_PARENT: &str = "invalid parent";

/// What this loopback answers a parent write past the child limit
/// [`LinearWorkspace::limit_children`] sets with — a destination's own refusal of one edge.
pub const LINEAR_CHILD_LIMIT: &str = "Issue cannot have more sub-issues";

/// The page of children a relation read answers, as the plugin's document asks for it.
const CHILDREN_PAGE: usize = 50;

/// The answer to `operation` when it is one of the relation documents, `None` otherwise.
pub(super) fn answer(
    operation: &str,
    vars: &Value,
    data: &mut Value,
) -> Option<Result<Value, &'static str>> {
    if operation == graphql::ISSUE_PARENT_READ {
        return Some(read(vars, data));
    }
    if operation == graphql::ISSUE_PARENT_UPDATE {
        return Some(write(vars, data));
    }
    None
}

/// One issue, its parent and its first page of children.
fn read(vars: &Value, data: &Value) -> Result<Value, &'static str> {
    let fields = vars.as_object().ok_or("a relation read takes variables")?;
    let id = fields
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| fields.len() == 1 && !id.is_empty())
        .ok_or("a relation read takes exactly an id")?;
    let row = data["tasks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == id))
        .ok_or(graphql::ISSUE_NOT_FOUND_MESSAGE)?;
    Ok(json!({"issue": issue(row, data)}))
}

/// `row` as the relation documents select it.
fn issue(row: &Value, data: &Value) -> Value {
    let mut issue = linear_task(row, data);
    if let Some(issue) = issue.as_object_mut() {
        issue.remove("descriptionState");
    }
    let id = &row["id"];
    let children: Vec<Value> = data["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|child| child["_parent"] == *id)
        .filter(|child| child.get("_trashed").is_none_or(Value::is_null))
        .map(|child| json!({"id": child["id"]}))
        .collect();
    let more = children.len() > CHILDREN_PAGE;
    issue["parent"] = row
        .get("_parent")
        .filter(|parent| parent.is_string())
        .map_or(Value::Null, |parent| json!({"id": parent}));
    issue["children"] = json!({
        "nodes": children.into_iter().take(CHILDREN_PAGE).collect::<Vec<_>>(),
        "pageInfo": {"hasNextPage": more},
    });
    issue
}

/// One `issueUpdate` of a parent alone.
fn write(vars: &Value, data: &mut Value) -> Result<Value, &'static str> {
    let fields = vars.as_object().ok_or("a parent write takes variables")?;
    let input = vars["input"]
        .as_object()
        .ok_or("a parent write takes an input")?;
    if fields.len() != 2 || input.len() != 1 || !input.contains_key("parentId") {
        return Err("a parent write sends its id and an input of parentId alone");
    }
    let id = vars["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("update id must be a string")?;
    let parent = match &vars["input"]["parentId"] {
        Value::Null => None,
        Value::String(parent) if !parent.is_empty() => Some(parent.clone()),
        _ => return Err("parentId is an issue id or null"),
    };
    if data["_linear_refused_items"]
        .as_array()
        .is_some_and(|refused| refused.contains(&json!(id)))
    {
        return Err(LINEAR_REFUSED_WRITE);
    }
    let held = |data: &Value, wanted: &str| {
        data["tasks"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["id"] == wanted))
    };
    if !held(data, id) {
        return Err(graphql::ISSUE_NOT_FOUND_MESSAGE);
    }
    if let Some(parent) = &parent {
        if !held(data, parent) {
            return Err(graphql::ISSUE_NOT_FOUND_MESSAGE);
        }
        // A parent that is the child or under it closes a cycle.
        let mut at = Some(parent.clone());
        let mut walked = 0;
        while let Some(ancestor) = at {
            if ancestor == id || walked > 1000 {
                return Err(LINEAR_INVALID_PARENT);
            }
            walked += 1;
            at = data["tasks"]
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["id"] == ancestor))
                .and_then(|row| row["_parent"].as_str().map(str::to_owned));
        }
        if let Some(limit) = data["_linear_child_limit"].as_u64() {
            let children = data["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| row["_parent"] == json!(parent) && row["id"] != id)
                .count();
            if children as u64 >= limit {
                return Err(LINEAR_CHILD_LIMIT);
            }
        }
    }
    let (_, stamp) = linear_tick(data);
    let row = data["tasks"]
        .as_array_mut()
        .and_then(|rows| rows.iter_mut().find(|row| row["id"] == id))
        .ok_or(graphql::ISSUE_NOT_FOUND_MESSAGE)?;
    row["_parent"] = parent.map_or(Value::Null, Value::String);
    row["_updated"] = json!(stamp);
    let row = row.clone();
    let mut answered = linear_task(&row, data);
    answered["parent"] = issue(&row, data)["parent"].clone();
    Ok(json!({"issueUpdate": {"success": true, "issue": answered}}))
}

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] Controls of the loopback Linear's private state, which only its request loop answers from: like `refuse_writes_of` they can only be methods of its handle, and the Linear suite's relation journeys drive them.
impl LinearWorkspace {
    /// Refuse, from now on, a parent write that would give a parent more than `limit` children,
    /// as a destination's own limit on one edge.
    pub fn limit_children(&self, limit: u64) {
        self.state.lock().unwrap()["_linear_child_limit"] = json!(limit);
    }

    /// Hold `raw` as the issue `id`'s whole `description`, metadata slot and all, as a person —
    /// or Linear's own normalisation of what it was sent — leaves it, moving its `updatedAt`.
    pub fn store_description(&self, id: &str, raw: &str) {
        let mut data = self.state.lock().unwrap();
        let (_, stamp) = linear_tick(&mut data);
        let row = data["tasks"]
            .as_array_mut()
            .and_then(|rows| rows.iter_mut().find(|row| row["id"] == id))
            .unwrap_or_else(|| panic!("the workspace holds {id}"));
        row["_linear_description"] = json!(raw);
        row["_updated"] = json!(stamp);
    }

    /// Stop refusing the writes of `id` that [`LinearWorkspace::refuse_writes_of`] refused: Linear
    /// taking that item's writes again.
    pub fn allow_writes_of(&self, id: &str) {
        let mut data = self.state.lock().unwrap();
        if let Some(refused) = data["_linear_refused_items"].as_array_mut() {
            refused.retain(|held| held != id);
        }
    }

    /// The parent the issue `id` holds, as Linear would answer its `parent`.
    pub fn parent_of(&self, id: &str) -> Option<String> {
        self.state.lock().unwrap()["tasks"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"] == id))
            .and_then(|row| row["_parent"].as_str().map(str::to_owned))
    }

    /// Every native `related` relation the workspace holds, each as the pair it was created
    /// between.
    pub fn related_pairs(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap()["task_dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|edge| edge["kind"] == "related")
            .map(|edge| {
                (
                    edge["from"].as_str().unwrap_or_default().to_owned(),
                    edge["to"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }
}
// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]
