//! The loopback board's assignees, held per issue and answered as GitHub answers them: the
//! `assignees` of every issue a source opted in to them reads — the documents it spells with
//! `graphql::with_assignees`, answered as the documents they were written as and then with each
//! issue's assignees — a `user(login:)` lookup, and the `assigneeIds` of a `createIssue` or an
//! `updateIssue`, which replace whoever the issue held and move its `updatedAt` as an
//! assignment moves a real issue's.
//!
//! Two of GitHub's answers a journey can ask for: a person whose assignment is **refused** with a
//! GraphQL error at the write's own alias, and a person GitHub **leaves off** the issue while
//! answering the write as a success — what GitHub's REST documentation of the same write says it
//! does with a person who cannot be assigned ("silently ignored").

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use super::{GitHubBoard, GitHubBoardFields};

/// Every person this board knows, by login: the one lookup GitHub answers with an id. Any other
/// login is answered `null` beside a `NOT_FOUND` error, as GitHub answers a login it does not
/// know.
pub const GITHUB_PEOPLE: [&str; 6] = ["octocat", "hubot", "monalisa", "defunkt", "mojombo", "pat"];

/// The node id this board gives the person `login`.
#[must_use]
pub fn github_person_id(login: &str) -> String {
    format!("U_{login}")
}

/// GitHub's message for a login it does not know.
fn unknown_login(login: &str) -> String {
    format!("Could not resolve to a User with the login of '{login}'.")
}

/// The board's assignees and how it answers writes of them.
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] This is the loopback GitHub board's own state, and AGENTS.md places every loopback server and its state in `onetaskgraph-e2e-support` once, shared by the suites that drive it; it extends `GitHubBoard`, whose private fields only a module of `fixtures` can reach.
#[derive(Default)]
pub(super) struct Assignees {
    /// Each issue's assignees, by issue id, in the order they were assigned; an issue absent
    /// here is unassigned.
    held: BTreeMap<String, Vec<String>>,
    /// People whose assignment this board refuses with an error at the write's alias.
    refused: Vec<String>,
    /// People this board leaves off an issue while answering the write as a success.
    left_off: Vec<String>,
    /// Aliases of the request being answered that were refused, with the person each named, to
    /// answer `null` beside an error.
    owed: Vec<(String, String)>,
    /// People this board keeps on an issue that holds them whatever a replacement names.
    kept: Vec<String>,
    /// Whether the next answer to an assignee write leaves the people it assigned out.
    unanswered: bool,
    /// Answers to bend once each, in order: the first whose document holds its text is handed
    /// the `data` the board would have answered.
    bends: Vec<(&'static str, AnswerBend)>,
}

/// How one answer is bent: handed the `data` the board would have answered.
type AnswerBend = Box<dyn FnMut(&mut Value) + Send>;

/// The login of the person whose node id this board gave as `id`.
fn login_of(id: &str) -> Option<&str> {
    id.strip_prefix("U_")
        .filter(|login| GITHUB_PEOPLE.contains(login))
}

/// Each input of the request `variables` writing assignees: the variable's name, the alias its
/// answer comes back at, and whether the request sends it. `root` is the mutation field a single
/// `$input` is answered at.
fn assigning_inputs<'a>(query: &str, variables: &'a Value) -> Vec<(String, String, &'a Value)> {
    let root = if query.contains("createIssue(") {
        "createIssue"
    } else {
        "updateIssue"
    };
    let Some(all) = variables.as_object() else {
        return Vec::new();
    };
    all.iter()
        .filter(|(_, input)| input.get("assigneeIds").is_some())
        .filter_map(|(name, input)| {
            if name == "input" {
                return Some((name.clone(), root.to_owned(), input));
            }
            let slot = name.trim_start_matches(|c: char| c.is_ascii_alphabetic());
            let flag = ["sendUpdate", "send"]
                .iter()
                .map(|flag| format!("{flag}{slot}"))
                .find(|flag| all.contains_key(flag));
            let sent = flag.is_none_or(|flag| all[&flag] == json!(true));
            sent.then(|| (name.clone(), name.clone(), input))
        })
        .collect()
}

/// The logins `input` assigns.
fn people_of(input: &Value) -> Vec<String> {
    input["assigneeIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| login_of(id).unwrap_or(id).to_owned())
        .collect()
}

/// The answer to [`graphql::USER_IDS`], or `None` for any other document.
pub(super) fn answer(
    board: &Arc<Mutex<GitHubBoard>>,
    query: &str,
    variables: &Value,
) -> Option<Value> {
    if query != graphql::USER_IDS {
        return None;
    }
    let mut held = board.lock().unwrap();
    let mut data = json!({});
    for slot in 0..onetaskgraph_github_projects::PEOPLE_BATCH {
        let alias = format!("u{slot}");
        let login = variables[format!("login{slot}")]
            .as_str()
            .unwrap_or_default();
        let known = GITHUB_PEOPLE
            .iter()
            .find(|known| known.eq_ignore_ascii_case(login));
        data[&alias] = match known {
            Some(known) => json!({"id": github_person_id(known), "login": known}),
            None => {
                held.owed_error_objects.push(json!({
                    "type": "NOT_FOUND", "path": [alias],
                    "locations": [{"line": 1, "column": 1}],
                    "message": unknown_login(login),
                }));
                Value::Null
            }
        };
    }
    Some(data)
}

/// The whole answer to a write of one issue — `$input` alone — assigning a person this board
/// refuses: `null` at its field beside an error naming the person, nothing of it applied. `None`
/// for every other request.
pub(super) fn refused(
    board: &Arc<Mutex<GitHubBoard>>,
    query: &str,
    variables: &Value,
) -> Option<Value> {
    let held = board.lock().unwrap();
    let (_, root, input) = assigning_inputs(query, variables)
        .into_iter()
        .find(|(name, _, _)| name == "input")?;
    let person = people_of(input)
        .into_iter()
        .find(|person| held.assignees.refused.contains(person))?;
    Some(json!({
        "data": {root.clone(): null},
        "errors": [{"type": "UNPROCESSABLE", "path": [root],
                    "message": format!("{person} cannot be assigned to this issue")}],
    }))
}

/// `variables` as the board answers them once the assignee writes they carry are applied: each
/// existing issue's assignees replaced — less a person the board leaves off — and its
/// `updatedAt` moved where they changed; an aliased write assigning a person the board refuses
/// not sent at all, and owed `null` beside an error. A create's assignees are applied once it is
/// answered, by [`answered`].
pub(super) fn applied(board: &Arc<Mutex<GitHubBoard>>, query: &str, variables: &Value) -> Value {
    let mut variables = variables.clone();
    let inputs: Vec<(String, String, Value)> = assigning_inputs(query, &variables)
        .into_iter()
        .map(|(name, alias, input)| (name, alias, input.clone()))
        .collect();
    let mut held = board.lock().unwrap();
    for (name, alias, input) in inputs {
        let people = people_of(&input);
        if let Some(person) = people
            .iter()
            .find(|person| held.assignees.refused.contains(person))
        {
            let slot = name.trim_start_matches(|c: char| c.is_ascii_alphabetic());
            for flag in ["sendUpdate", "send"] {
                let flag = format!("{flag}{slot}");
                if variables.get(&flag).is_some() {
                    variables[flag] = json!(false);
                }
            }
            held.assignees.owed.push((alias, person.clone()));
            continue;
        }
        if let Some(issue) = input["id"].as_str() {
            held.assign(issue, &people);
        }
    }
    variables
}

/// `data`, the answer to `variables`, with the assignees of each issue a create in it made
/// applied, each refused alias answered `null` beside its error, and — for a document a source
/// spelled with its assignees — every issue and draft it shows given the people it holds.
pub(super) fn answered(
    board: &Arc<Mutex<GitHubBoard>>,
    query: &str,
    variables: &Value,
    mut data: Value,
    spelled: bool,
) -> Value {
    let mut held = board.lock().unwrap();
    for (_, alias, input) in assigning_inputs(query, variables) {
        if input.get("id").is_some() {
            continue;
        }
        if let Some(created) = data
            .pointer(&format!("/{alias}/issue/id"))
            .and_then(Value::as_str)
        {
            let created = created.to_owned();
            held.assign(&created, &people_of(input));
        }
    }
    for (alias, person) in std::mem::take(&mut held.assignees.owed) {
        data[&alias] = Value::Null;
        held.owed_error_objects.push(json!({
            "type": "UNPROCESSABLE", "path": [alias],
            "message": format!("{person} cannot be assigned to this issue"),
        }));
    }
    if spelled {
        held.show_assignees(&mut data);
    }
    if let Some(at) = held
        .assignees
        .bends
        .iter()
        .position(|(holds, _)| query.contains(holds))
    {
        let (_, mut bend) = held.assignees.bends.remove(at);
        bend(&mut data);
    }
    let writes = assigning_inputs(query, variables);
    if !writes.is_empty() && std::mem::take(&mut held.assignees.unanswered) {
        for (_, alias, _) in writes {
            if let Some(issue) = data
                .get_mut(&alias)
                .and_then(|answer| answer.get_mut("issue"))
                .and_then(Value::as_object_mut)
            {
                issue.remove("assignees");
            }
        }
    }
    data
}

impl GitHubBoard {
    /// Replace the issue `issue`'s assignees with `people`, less any this board leaves off,
    /// moving its `updatedAt` when that changes whom it holds.
    fn assign(&mut self, issue: &str, people: &[String]) {
        let before = self.assignees.held.get(issue).cloned().unwrap_or_default();
        let mut kept: Vec<String> = people
            .iter()
            .filter(|person| !self.assignees.left_off.contains(person))
            .cloned()
            .collect();
        for person in &before {
            if self.assignees.kept.contains(person) && !kept.contains(person) {
                kept.push(person.clone());
            }
        }
        if before == kept {
            return;
        }
        if kept.is_empty() {
            self.assignees.held.remove(issue);
        } else {
            self.assignees.held.insert(issue.to_owned(), kept);
        }
        self.touch_issue(&json!(issue));
    }

    /// Give every issue and draft `value` shows the people it holds, as its `assignees`.
    fn show_assignees(&self, value: &mut Value) {
        match value {
            Value::Object(fields) => {
                let shown = fields.get("id").and_then(Value::as_str).filter(|id| {
                    self.items
                        .iter()
                        .chain(self.pending.iter())
                        .any(|item| item["id"] == *id)
                });
                if let Some(id) = shown.map(str::to_owned) {
                    let nodes: Vec<Value> = self
                        .assignees
                        .held
                        .get(&id)
                        .into_iter()
                        .flatten()
                        .map(|login| json!({"id": github_person_id(login), "login": login}))
                        .collect();
                    fields.insert("assignees".to_owned(), json!({"nodes": nodes}));
                }
                for field in fields.values_mut() {
                    self.show_assignees(field);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.show_assignees(item);
                }
            }
            _ => {}
        }
    }
}

impl GitHubBoardFields {
    /// Assign the issue `issue` to `people`, as somebody did on GitHub before any command ran:
    /// its `updatedAt` stays where it was.
    pub fn seed_assignees(&self, issue: &str, people: &[&str]) {
        self.board.lock().unwrap().assignees.held.insert(
            issue.to_owned(),
            people.iter().map(|person| (*person).to_owned()).collect(),
        );
    }

    /// Whom the issue `issue` is assigned to now.
    #[must_use]
    pub fn assignees_of(&self, issue: &str) -> Vec<String> {
        self.board
            .lock()
            .unwrap()
            .assignees
            .held
            .get(issue)
            .cloned()
            .unwrap_or_default()
    }

    /// Refuse every write assigning `login` with a GraphQL error at that write's own field.
    pub fn refuse_assigning(&self, login: &str) {
        self.board
            .lock()
            .unwrap()
            .assignees
            .refused
            .push(login.to_owned());
    }

    /// Keep `login` on every issue that holds them, whatever a later replacement names, while
    /// answering it as a success.
    pub fn keep_when_replaced(&self, login: &str) {
        self.board
            .lock()
            .unwrap()
            .assignees
            .kept
            .push(login.to_owned());
    }

    /// Bend the next answer to a document holding `holds` through `bend`, once, after the board
    /// applied the request: a GitHub answering as no correct one does.
    pub fn bend_next_answer(
        &self,
        holds: &'static str,
        bend: impl FnMut(&mut Value) + Send + 'static,
    ) {
        self.board
            .lock()
            .unwrap()
            .assignees
            .bends
            .push((holds, Box::new(bend)));
    }

    /// Answer the next assignee write — applied as asked — without the people it left the issue
    /// assigned to, once.
    pub fn answer_next_assignment_without_assignees(&self) {
        self.board.lock().unwrap().assignees.unanswered = true;
    }

    /// Whether the board holds an issue or a draft titled `title`.
    #[must_use]
    pub fn holds_titled(&self, title: &str) -> bool {
        let board = self.board.lock().unwrap();
        board
            .items
            .iter()
            .chain(board.pending.iter())
            .any(|item| item["title"] == title)
    }

    /// Answer every write assigning `login` as a success while leaving them off the issue.
    pub fn leave_off_when_assigned(&self, login: &str) {
        self.board
            .lock()
            .unwrap()
            .assignees
            .left_off
            .push(login.to_owned());
    }
}
