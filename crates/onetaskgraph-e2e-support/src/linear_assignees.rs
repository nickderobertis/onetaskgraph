//! How the loopback Linear holds and answers an issue's assignee, for a Linear source configured
//! with `assignees: true`: the workspace's users, each issue's one assignee, the
//! `users(filter:{displayName:{in:…}})` lookup, `assigneeId` in an `issueCreate` or
//! `issueUpdate` input — `null` unassigning — and `assignee{displayName}` in every issue
//! selection that asks for it, as Linear's published schema documents each.
//!
//! The rest of the loopback answers the documents a source that has not opted in sends, and
//! never sees an assignee: [`before`] takes the assignee out of a request — the widened
//! selection back to the plain document, `assigneeId` out of each input — and [`after`] puts it
//! into the answer. So a workspace no journey gives a user answers every request exactly as it
//! did before assignees existed.
//!
//! A workspace's users are seeded as `held["_linear_users"]`, `[{"id", "displayName"}]`, and an
//! issue's assignee as `held["_linear_assignees"][<issue id>]`, a user id. Linear moves an
//! issue's `updatedAt` when its assignee changes, and so does this loopback.

use onetaskgraph_linear::graphql;
use serde_json::{Map, Value, json};

use crate::fixtures::{LinearWorkspace, linear_tick};

/// What this loopback answers an `assigneeId` naming no user of the workspace with — Linear's
/// spelling of a reference to an entity it does not hold.
pub const LINEAR_USER_MISSING: &str = "Entity not found: User";

/// What [`before`] took out of one request, for [`after`] to answer with.
#[derive(Default)]
pub(crate) struct Assigning {
    /// Whether the request selected `assignee{displayName}`.
    widened: bool,
    /// The answer to a `users` lookup, which nothing else answers.
    users: Option<Value>,
    /// A request naming a user the workspace does not hold, refused whole.
    refused: bool,
    /// Each create's assignee, by the alias it was sent under (`None` unaliased).
    creates: Vec<(Option<usize>, Value)>,
    /// The assignee map and each moved issue's `_updated` before an update moved them, put back
    /// when the request is refused.
    undo: Option<(Value, Vec<(String, Value)>)>,
}

impl Assigning {
    /// The answer to the request when this module answers it alone.
    pub(crate) fn answered(&self) -> Option<(&'static str, Value)> {
        if self.refused {
            return Some((
                "200 OK",
                json!({"data": null, "errors": [{"message": LINEAR_USER_MISSING}]}),
            ));
        }
        self.users
            .as_ref()
            .map(|users| ("200 OK", json!({"data": users})))
    }
}

/// `request` with every assignee taken out of it, and what was taken; an update's assignee is
/// applied to `data` now — moving the issue's `updatedAt` when it changes — so the rest of the
/// loopback answers the issue as it then stands.
pub(crate) fn before(mut request: Value, data: &mut Value) -> (Value, Assigning) {
    let mut assigning = Assigning::default();
    let query = request["query"].as_str().unwrap_or_default().to_owned();
    // A lookup: `USERS` itself, or that document aliased once a page of names.
    let connections = (1..)
        .take_while(|count| query.contains(&format!(" a{}: ", count - 1)))
        .last()
        .filter(|count| graphql::aliased(graphql::USERS, *count) == query);
    if query == graphql::USERS || connections.is_some() {
        let answered = |names: &Value| -> Value {
            let names: Vec<&str> = names
                .as_array()
                .map(|names| names.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let nodes: Vec<Value> = users(data)
                .into_iter()
                .filter(|user| names.contains(&user["displayName"].as_str().unwrap_or_default()))
                .collect();
            json!({"nodes": nodes})
        };
        assigning.users = Some(match connections {
            None => json!({"users": answered(&request["variables"]["names"])}),
            Some(count) => Value::Object(
                (0..count)
                    .map(|at| {
                        (
                            format!("a{at}"),
                            answered(&request["variables"][format!("names_{at}")]),
                        )
                    })
                    .collect(),
            ),
        });
        return (request, assigning);
    }
    if let Some(plain) = graphql::without_assignees(&query) {
        assigning.widened = true;
        request["query"] = json!(plain);
    }
    if !query.trim_start().starts_with("mutation") {
        return (request, assigning);
    }
    let plain = request["query"].as_str().unwrap_or_default().to_owned();
    let singles = graphql::decomposed(&plain);
    let creates_issue = |alias: Option<usize>| match (alias, &singles) {
        (Some(at), Some(singles)) => singles.get(at) == Some(&graphql::ISSUE_CREATE),
        _ => plain == graphql::ISSUE_CREATE,
    };
    let Some(variables) = request.get_mut("variables").and_then(Value::as_object_mut) else {
        return (request, assigning);
    };
    let mut updates: Vec<(String, Value)> = Vec::new();
    let names: Vec<String> = variables.keys().cloned().collect();
    for name in names {
        let Some(suffix) = name.strip_prefix("input") else {
            continue;
        };
        let alias = suffix
            .strip_prefix('_')
            .and_then(|at| at.parse::<usize>().ok());
        let Some(input) = variables.get_mut(&name).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(assignee) = input.remove("assigneeId") else {
            // A create naming nobody takes the workspace's default, as a Linear template or
            // triage rule assigns a new issue.
            if creates_issue(alias)
                && let Some(default) = data.get("_linear_default_assignee").cloned()
            {
                assigning.creates.push((alias, default));
            }
            continue;
        };
        if !assignee.is_null() && !users(data).iter().any(|user| user["id"] == assignee) {
            assigning.refused = true;
        }
        let assignee = instead(data, assignee);
        let id = variables
            .get(&format!("id{suffix}"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        match id {
            Some(id) => {
                // An input naming nothing but the assignee: the rest of the loopback takes no
                // empty input, so it is given the issue's own title, which moves nothing.
                if let Some(input) = variables.get_mut(&name).and_then(Value::as_object_mut)
                    && input.is_empty()
                    && let Some(title) = task(data, &id).map(|row| row["title"].clone())
                {
                    input.insert("title".into(), title);
                }
                updates.push((id, assignee));
            }
            None => assigning.creates.push((alias, assignee)),
        }
    }
    if assigning.refused {
        return (request, assigning);
    }
    if !updates.is_empty() {
        let mut moved = Vec::new();
        let held = data["_linear_assignees"].clone();
        for (id, assignee) in updates {
            if assigned(data, &id) == assignee || task(data, &id).is_none() {
                continue;
            }
            let (_, stamp) = linear_tick(data);
            if let Some(row) = task_mut(data, &id) {
                moved.push((id.clone(), row["_updated"].clone()));
                row["_updated"] = json!(stamp);
            }
            assign(data, &id, assignee);
        }
        assigning.undo = Some((held, moved));
    }
    (request, assigning)
}

/// `response` — the loopback's answer to the request [`before`] took `assigning` out of — with
/// each create's assignee held, every issue it answers carrying its assignee where the request
/// asked for it, and an update's assignee put back when the request was refused.
pub(crate) fn after(assigning: Assigning, mut response: Value, data: &mut Value) -> Value {
    let refused = response.get("errors").is_some() || response["data"].is_null();
    if refused {
        if let Some((held, moved)) = assigning.undo {
            data["_linear_assignees"] = held;
            for (id, updated) in moved {
                if let Some(row) = task_mut(data, &id) {
                    row["_updated"] = updated;
                }
            }
        }
        return response;
    }
    for (alias, assignee) in assigning.creates {
        let answer = match alias {
            Some(at) => &response["data"][format!("a{at}")],
            None => &response["data"]["issueCreate"],
        };
        if let Some(id) = answer["issue"]["id"].as_str().map(str::to_owned) {
            assign(data, &id, assignee);
        }
    }
    if assigning.widened {
        decorate(&mut response["data"], data);
    }
    response
}

/// Every issue in `value` given its assignee: an object an issue selection answered, which is
/// one carrying an `identifier` and a `priority`.
fn decorate(value: &mut Value, data: &Value) {
    match value {
        Value::Object(members) => {
            if members.contains_key("identifier")
                && members.contains_key("priority")
                && let Some(id) = members.get("id").and_then(Value::as_str).map(str::to_owned)
            {
                members.insert("assignee".into(), assignee_node(data, &id));
            }
            for member in members.values_mut() {
                decorate(member, data);
            }
        }
        Value::Array(items) => {
            for item in items {
                decorate(item, data);
            }
        }
        _ => {}
    }
}

/// The user `asked` is assigned as: another one, where [`LinearWorkspace::assign_instead`]
/// says so.
fn instead(data: &Value, asked: Value) -> Value {
    asked
        .as_str()
        .and_then(|asked| data["_linear_assign_instead"].get(asked).cloned())
        .unwrap_or(asked)
}

/// The workspace's users.
fn users(data: &Value) -> Vec<Value> {
    data["_linear_users"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The user id `id` is assigned to, `null` for none.
fn assigned(data: &Value, id: &str) -> Value {
    data["_linear_assignees"]
        .get(id)
        .cloned()
        .unwrap_or(Value::Null)
}

/// Assign `id` to the user `assignee`, or to nobody.
fn assign(data: &mut Value, id: &str, assignee: Value) {
    if !data["_linear_assignees"].is_object() {
        data["_linear_assignees"] = Value::Object(Map::new());
    }
    if assignee.is_null() {
        data["_linear_assignees"]
            .as_object_mut()
            .map(|held| held.remove(id));
    } else {
        data["_linear_assignees"][id] = assignee;
    }
}

/// `assignee{displayName}` as an issue selection answers it for `id`.
fn assignee_node(data: &Value, id: &str) -> Value {
    let user = assigned(data, id);
    users(data)
        .into_iter()
        .find(|held| !user.is_null() && held["id"] == user)
        .map_or(
            Value::Null,
            |held| json!({"displayName": held["displayName"]}),
        )
}

fn task<'a>(data: &'a Value, id: &str) -> Option<&'a Value> {
    data["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|row| row["id"] == id))
}

fn task_mut<'a>(data: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    data["tasks"]
        .as_array_mut()
        .and_then(|tasks| tasks.iter_mut().find(|row| row["id"] == id))
}

impl LinearWorkspace {
    /// Add the user `id`, displayed as `name`, as a person joining the workspace would.
    pub fn add_user(&self, id: &str, name: &str) {
        let mut data = self.state.lock().unwrap();
        let mut held = users(&data);
        held.push(json!({"id": id, "displayName": name}));
        data["_linear_users"] = Value::Array(held);
    }

    /// Assign every issue created naming no assignee to the user `id`, as a Linear template or
    /// triage rule does.
    pub fn assign_by_default(&self, id: &str) {
        self.state.lock().unwrap()["_linear_default_assignee"] = json!(id);
    }

    /// From now on, assign the user `given` wherever a write asks for `asked` — a workspace
    /// answering an assignment with another person than it was sent; `None` stops it.
    pub fn assign_instead(&self, asked: &str, given: Option<&str>) {
        let mut data = self.state.lock().unwrap();
        if !data["_linear_assign_instead"].is_object() {
            data["_linear_assign_instead"] = json!({});
        }
        match given {
            Some(given) => data["_linear_assign_instead"][asked] = json!(given),
            None => {
                data["_linear_assign_instead"]
                    .as_object_mut()
                    .map(|held| held.remove(asked));
            }
        }
    }

    /// The display name of the user the issue `id` is assigned to, as this workspace holds it —
    /// `None` when it is unassigned.
    pub fn assignee_of(&self, id: &str) -> Option<String> {
        let data = self.state.lock().unwrap();
        assignee_node(&data, id)["displayName"]
            .as_str()
            .map(str::to_owned)
    }
}
