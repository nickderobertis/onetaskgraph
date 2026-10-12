//! What this source does for a write the store's local cache expects.
//!
//! The store keeps each task it read beside what this source held about it — the
//! [`Resolved`] record a write takes its node ids, its board item, its field options and its
//! current values from — and the board's field definitions beside every task. A write the
//! caller says it knows the task at a given version of hands both back through
//! [`TaskSource::prime_write`](onetaskgraph_plugin_api::TaskSource::prime_write), and every
//! write path here already takes a record held in this process instead of reading the issue
//! again: so the write sends its mutations and nothing else.
//!
//! While such a write runs a journal is open. Every request is recorded with what it came to —
//! landed, refused, or lost — and every mutation that has a twin selecting the whole issue
//! (`graphql::whole_twin`) is sent as that twin, so its response vouches for what the issue
//! holds afterwards, at the version its change token spells, and the store can advance its
//! entry without reading. Nothing here outlives the process: the store, not this source, is
//! what keeps a record between commands.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use onetaskgraph_plugin_api::{
    AssetUploads, JournalEntry, NativeId, PartOutcome, Repository, RequestKind, SourceError,
    WholeTask, WriteJournal,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    BOARD_ITEMS_PAGE_SIZE, BoardFields, BoardKind, GitHubProjectsSource, ItemKind, MAX_PAGE_SIZE,
    NESTED_PAGE_SIZE, RepositoryTarget, Resolved, SERVED_AT, change_token, comment_page, graphql,
    is_mutation, operation_description, optional_time,
};

/// The version of the context this source hands the store, so a context an earlier or a later
/// build wrote is one this build declines rather than misreads.
const CONTEXT_VERSION: u64 = 1;

/// One task's context: its record, and the comments on it this process read, by id, with the
/// body GitHub holds — which an edit keeps a copy's origin marker from.
#[derive(Serialize, Deserialize)]
struct Context {
    version: u64,
    item: Resolved,
    comments: BTreeMap<NativeId, String>,
}

/// The board's context: its own id and field definitions.
#[derive(Serialize, Deserialize)]
struct Board {
    version: u64,
    fields: BoardFields,
}

/// What this source holds for expected writes, for the length of one process.
#[derive(Default)]
pub(super) struct Held {
    /// The journal, while one is open.
    journal: Mutex<Option<WriteJournal>>,
    /// The comments this process read on each issue, with the body each holds.
    learnt: Mutex<BTreeMap<NativeId, BTreeMap<NativeId, String>>>,
    /// The comments a primed context says each issue holds.
    primed: Mutex<BTreeMap<NativeId, BTreeMap<NativeId, String>>>,
}

/// A lock an earlier panic poisoned is read as it stands: what these hold is an
/// optimisation, and a write that cannot use it reads instead.
fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Held {
    /// Whether a journal is open.
    pub(super) fn journaling(&self) -> bool {
        held(&self.journal).is_some()
    }

    /// Open a journal.
    pub(super) fn begin(&self) {
        *held(&self.journal) = Some(WriteJournal::default());
    }

    /// Close the journal, answering with what it recorded.
    pub(super) fn end(&self) -> WriteJournal {
        held(&self.journal).take().unwrap_or_default()
    }

    fn record(&self, entry: JournalEntry) {
        if let Some(journal) = held(&self.journal).as_mut() {
            journal.requests.push(entry);
        }
    }

    /// Learn the comments one detail read of `issue` carried, when it carried them all.
    pub(super) fn learn_comments(&self, issue: &NativeId, node: &Value) {
        let Some(connection) = node.get("comments") else {
            return;
        };
        if connection.pointer("/pageInfo/hasNextPage") != Some(&Value::Bool(false)) {
            return;
        }
        let Some(nodes) = connection.get("nodes").and_then(Value::as_array) else {
            return;
        };
        let comments = nodes
            .iter()
            .filter_map(|comment| {
                Some((
                    NativeId(comment.get("id")?.as_str()?.to_owned()),
                    comment.get("body")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        held(&self.learnt).insert(issue.clone(), comments);
    }

    /// Whether a write of `id` was primed from the store's cache: a write expecting a version,
    /// which sends only what differs from it.
    pub(super) fn is_primed(&self, id: &NativeId) -> bool {
        held(&self.primed).contains_key(id)
    }

    /// The body comment `comment` holds on `issue`, by a primed context: `Some(Some(body))`
    /// when it is one of the issue's, `Some(None)` when it is not, and `None` when no primed
    /// context says.
    pub(super) fn primed_comment(
        &self,
        issue: &NativeId,
        comment: &NativeId,
    ) -> Option<Option<String>> {
        held(&self.primed)
            .get(issue)
            .map(|comments| comments.get(comment).cloned())
    }
}

/// `variables`, with the page sizes every whole-issue twin declares.
pub(super) fn whole_variables(mut variables: Value) -> Value {
    if let Some(members) = variables.as_object_mut() {
        members.insert("first".to_owned(), json!(MAX_PAGE_SIZE));
        members.insert("nestedFirst".to_owned(), json!(NESTED_PAGE_SIZE));
        members.insert("boardItems".to_owned(), json!(BOARD_ITEMS_PAGE_SIZE));
        members.insert("duplicates".to_owned(), json!(true));
    }
    variables
}

/// Which part of a write `document` is, in the words a write's `result` names it by.
fn part_of(document: &str) -> String {
    let part = if [
        graphql::ADD_COMMENT,
        graphql::UPDATE_COMMENT,
        graphql::DELETE_COMMENT,
        graphql::ADD_COMMENT_WHOLE,
        graphql::UPDATE_COMMENT_WHOLE,
    ]
    .contains(&document)
    {
        "comment"
    } else if [graphql::UPDATE_ISSUE, graphql::UPDATE_ISSUE_WHOLE].contains(&document) {
        "issue"
    } else if [
        graphql::UPDATE_FIELD,
        graphql::UPDATE_FIELDS,
        graphql::CLEAR_FIELD,
        graphql::UPDATE_FIELD_WHOLE,
        graphql::CLEAR_FIELD_WHOLE,
    ]
    .contains(&document)
    {
        "fields"
    } else if [graphql::ADD_BLOCKED_BY, graphql::REMOVE_BLOCKED_BY].contains(&document) {
        "dependencies"
    } else if [graphql::ADD_SUB_ISSUE, graphql::REMOVE_SUB_ISSUE].contains(&document) {
        "project"
    } else if document == graphql::UPDATE_DRAFT {
        "draft"
    } else {
        return operation_description(document).to_owned();
    };
    part.to_owned()
}

/// Whether `error` is a refusal of an id a primed context supplied that no longer resolves:
/// a node deleted since it was cached, or a single-select option renamed or deleted.
fn stale(error: &SourceError) -> bool {
    let SourceError::Refused { message } = error else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    message.contains(super::UNRESOLVABLE_NODE) || message.contains("single select option")
}

/// What one request came to, as a journal records it.
fn outcome(sent: &Result<Value, SourceError>) -> PartOutcome {
    match sent {
        Ok(_) => PartOutcome::Landed,
        // The request left, and its answer did not come back whole: whether GitHub applied it
        // cannot be known, and sending it again could apply it twice.
        Err(SourceError::Unavailable { message })
            if message.starts_with("GitHub GraphQL request failed")
                || message.starts_with("GitHub GraphQL response could not be read") =>
        {
            PartOutcome::Unknown {
                error: message.clone(),
            }
        }
        Err(error) => PartOutcome::Rejected {
            error: error.to_string(),
            stale: stale(error),
        },
    }
}

impl GitHubProjectsSource {
    /// The context of `id`: the record this process last resolved for it, and the comments it
    /// read on it.
    pub(super) fn context_of(&self, id: &NativeId) -> Option<Value> {
        let item = self.resolved_cache().ok()?.get(id).cloned()?;
        if item.kind != BoardKind::Work(ItemKind::Task) {
            return None;
        }
        let comments = held(&self.expected.learnt)
            .get(id)
            .cloned()
            .unwrap_or_default();
        serde_json::to_value(Context {
            version: CONTEXT_VERSION,
            item,
            comments,
        })
        .ok()
    }

    /// The board's context, when this process read the board's fields — on their own, or
    /// carried by a read of an issue by its own id, which selects every field the board
    /// defines.
    pub(super) fn board_context_held(&self) -> Option<Value> {
        let held = self.fields_cache().ok()?.clone();
        let fields = match held {
            Some(fields) => fields,
            None => self
                .resolved_cache()
                .ok()?
                .values()
                .find_map(Resolved::carried_board)?,
        };
        serde_json::to_value(Board {
            version: CONTEXT_VERSION,
            fields,
        })
        .ok()
    }

    /// Take `context` as the record of `id`, and `board` as the board's fields.
    pub(super) fn prime(&self, id: &NativeId, context: &Value, board: Option<&Value>) -> bool {
        let Ok(context) = serde_json::from_value::<Context>(context.clone()) else {
            return false;
        };
        if context.version != CONTEXT_VERSION || context.item.id != *id {
            return false;
        }
        let mut item = context.item;
        let board = board
            .and_then(|board| serde_json::from_value::<Board>(board.clone()).ok())
            .filter(|board| board.version == CONTEXT_VERSION);
        if let Some(board) = &board {
            // The board's own fields are what every task of it writes against, and the board
            // context is refreshed whenever a write finds an option re-minted — so it is
            // preferred to the definitions the task's own record carried when it was read.
            if item
                .named_board()
                .is_some_and(|named| named.as_str() == board.fields.id.as_str())
            {
                item.board_fields = Some(board.fields.fields.clone());
            }
        }
        let Ok(mut cache) = self.resolved_cache() else {
            return false;
        };
        cache.insert(id.clone(), item);
        drop(cache);
        held(&self.expected.primed).insert(id.clone(), context.comments);
        if let Some(board) = board
            && let Ok(mut fields) = self.fields_cache()
            && fields.is_none()
        {
            *fields = Some(board.fields);
        }
        true
    }

    /// Drop what [`Self::prime`] took for `id`, and the board's fields, so a write reads them.
    pub(super) fn unprime_held(&self, id: &NativeId) {
        if let Ok(mut cache) = self.resolved_cache() {
            cache.remove(id);
        }
        if held(&self.expected.primed).remove(id).is_some()
            && let Ok(mut fields) = self.fields_cache()
        {
            *fields = None;
        }
    }

    /// Record one request sent while a journal is open.
    pub(super) async fn journal(
        &self,
        document: &str,
        variables: &Value,
        sent: &Result<Value, SourceError>,
    ) {
        let mutation = is_mutation(document);
        let mut entry = JournalEntry {
            part: part_of(document),
            kind: if mutation {
                RequestKind::Mutation
            } else {
                RequestKind::Read
            },
            outcome: outcome(sent),
            fields: BTreeMap::new(),
            version: None,
            whole: None,
        };
        if mutation && let Ok(data) = sent {
            let (issue, fields) = read_back(document, variables, data);
            entry.fields = fields;
            if let Some(issue) = issue {
                entry.version = self.token_of(issue);
                entry.whole = self.whole_of(issue, entry.version.as_deref()).map(Box::new);
                // What a response vouches for is the issue's state now, so a comment learnt
                // on it is learnt afresh rather than added to what was read before.
                if let Some(id) = issue.get("id").and_then(Value::as_str) {
                    self.expected
                        .learn_comments(&NativeId(id.to_owned()), issue);
                }
            }
        }
        self.expected.record(entry);
    }

    /// The change token `issue` spells, as a response shows it: every part present and the
    /// issue settled by the response's own clock.
    fn token_of(&self, issue: &Value) -> Option<String> {
        let updated = optional_time(issue, "updatedAt").ok()??;
        let nodes = issue.pointer("/projectItems/nodes")?.as_array()?;
        let entry = self.board_entry(nodes)?;
        let item = optional_time(entry, "updatedAt").ok()??;
        let served = optional_time(issue, SERVED_AT).ok()??;
        let parts = change_token::Parts {
            issue: updated,
            item,
            comments: issue.pointer("/commentCount/totalCount")?.as_u64()?,
            relationships: issue.pointer("/relationshipEvents/totalCount")?.as_u64()?,
            repository: Repository::try_from(format!(
                "{}/{}",
                RepositoryTarget::HOST,
                issue.pointer("/repository/nameWithOwner")?.as_str()?
            ))
            .ok()?,
        };
        change_token::settled(parts.issue, parts.item, served).then(|| change_token::spell(&parts))
    }

    /// The task `issue` is, whole, at `version` — when the response carried every part the
    /// store's entry holds: the issue on this board, its every comment, and a token.
    fn whole_of(&self, issue: &Value, version: Option<&str>) -> Option<WholeTask> {
        version?;
        let nodes = issue.pointer("/projectItems/nodes")?.as_array()?;
        let held = self.board_entry(nodes)?;
        let item = json!({
            "id": held.get("id")?,
            "updatedAt": held.get("updatedAt"),
            "project": held.get("project"),
            "fieldValues": held.get("fieldValues"),
            "content": issue,
        });
        let resolved = self.resolve(&item).ok()??;
        if resolved.kind != BoardKind::Work(ItemKind::Task) {
            return None;
        }
        let task = resolved.task().ok()?;
        if task.version.as_deref() != version {
            return None;
        }
        let page = comment_page(issue, &resolved.id.0, None).ok()?;
        if page.next.is_some() {
            return None;
        }
        let assets = AssetUploads::read(&resolved.slot)
            .ok()?
            .unwrap_or_default()
            .0
            .into_iter()
            .map(|(name, upload)| onetaskgraph_plugin_api::Asset {
                content_type: name.content_type(),
                name,
                sha256: upload.sha256,
                path: None,
            })
            .collect();
        Some(WholeTask {
            task,
            comments: page.items,
            assets,
            edges: None,
        })
    }
}

/// The issue a mutation's response shows, and each field the mutation set with the value that
/// response shows it holding — `null` where it shows none.
fn read_back<'a>(
    document: &str,
    variables: &Value,
    data: &'a Value,
) -> (Option<&'a Value>, BTreeMap<String, Value>) {
    let mut fields = BTreeMap::new();
    let issue = if [graphql::ADD_COMMENT, graphql::ADD_COMMENT_WHOLE].contains(&document) {
        if let Some(added) = data.pointer("/addComment/commentEdge/node") {
            fields.insert("comment".to_owned(), comment_fields(added));
        }
        data.pointer("/addComment/subject")
    } else if [graphql::UPDATE_COMMENT, graphql::UPDATE_COMMENT_WHOLE].contains(&document) {
        let edited = data.pointer("/updateIssueComment/issueComment");
        if let Some(edited) = edited {
            fields.insert("comment".to_owned(), comment_fields(edited));
        }
        edited.and_then(|edited| edited.get("issue"))
    } else if document == graphql::DELETE_COMMENT {
        fields.insert(
            "comment".to_owned(),
            json!({"deleted": variables.pointer("/input/id")}),
        );
        None
    } else if [graphql::UPDATE_ISSUE, graphql::UPDATE_ISSUE_WHOLE].contains(&document) {
        let issue = data.pointer("/updateIssue/issue");
        let input = variables.get("input");
        for (asked, shown) in [
            ("title", "title"),
            ("body", "body"),
            ("stateInput", "state"),
        ] {
            if input.and_then(|input| input.get(asked)).is_some() {
                fields.insert(
                    shown.to_owned(),
                    issue
                        .and_then(|issue| issue.get(shown))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
        }
        issue
    } else if [graphql::UPDATE_FIELD, graphql::UPDATE_FIELD_WHOLE].contains(&document) {
        let item = data.pointer("/updateProjectV2ItemFieldValue/projectV2Item");
        field_values(variables, item, &mut fields);
        item.and_then(|item| item.get("content"))
    } else if [graphql::CLEAR_FIELD, graphql::CLEAR_FIELD_WHOLE].contains(&document) {
        let item = data.pointer("/clearProjectV2ItemFieldValue/projectV2Item");
        field_values(variables, item, &mut fields);
        item.and_then(|item| item.get("content"))
    } else if document == graphql::UPDATE_FIELDS {
        // Each aliased field write answers with its board item's id alone, so the values it
        // set are not shown: each field it set is named, and holds what no response showed.
        for argument in ["input", "second", "third", "fourth", "fifth", "sixth"] {
            if let Some(field) = variables
                .pointer(&format!("/{argument}/fieldId"))
                .and_then(Value::as_str)
            {
                fields.insert(field.to_owned(), Value::Null);
            }
        }
        None
    } else {
        None
    };
    (issue.filter(|issue| !issue.is_null()), fields)
}

/// What a write's result says of one comment it wrote: its id, its address and its body.
fn comment_fields(comment: &Value) -> Value {
    json!({
        "id": comment.get("id"),
        "url": comment.get("url"),
        "body": comment.get("body"),
    })
}

/// The field one field write set, by its name, with the value its response shows the board
/// item holding there — read off the issue's own memberships, when the response carried them.
fn field_values(variables: &Value, item: Option<&Value>, fields: &mut BTreeMap<String, Value>) {
    let Some(field) = variables.pointer("/input/fieldId").and_then(Value::as_str) else {
        return;
    };
    let memberships = item
        .and_then(|item| item.pointer("/content/projectItems/nodes"))
        .and_then(Value::as_array);
    let item_id = item.and_then(|item| item.get("id")).and_then(Value::as_str);
    let shown = memberships.and_then(|nodes| {
        let entry = nodes
            .iter()
            .find(|node| node.get("id").and_then(Value::as_str) == item_id)?;
        entry
            .pointer("/fieldValues/nodes")?
            .as_array()?
            .iter()
            .find(|value| value.pointer("/field/id").and_then(Value::as_str) == Some(field))
    });
    match shown {
        Some(value) => {
            let name = value
                .pointer("/field/name")
                .and_then(Value::as_str)
                .unwrap_or(field)
                .to_owned();
            let held = value
                .get("name")
                .or_else(|| value.get("text"))
                .cloned()
                .unwrap_or(Value::Null);
            fields.insert(name, held);
        }
        None => {
            // A cleared field holds no value, and a response without the item's memberships
            // shows none: either way the field is named and holds nothing shown.
            fields.insert(field.to_owned(), Value::Null);
        }
    }
}
