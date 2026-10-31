//! This session's own board, and the namespace that holds the lane to it.
//!
//! The nominated board is a template no session writes to: a session copies it with
//! `copyProjectV2` — fields, `Status` options and draft items in one mutation — runs every
//! journey on the copy, and deletes the copy when it ends, however it ends. So a session's
//! items, fields, origin field and counts are its own, and two sessions on one account
//! cannot read, change, count or remove each other's. See
//! [`onetaskgraph_github_live::RUN_BOARD_PREFIX`].
use super::{github_pause, graphql_variables};
use onetaskgraph_github_live::{
    COPY_BOARD_DOCUMENT, DELETE_BOARD_DOCUMENT, RUN_BOARD_PREFIX, TEMPLATE_DOCUMENT,
    quota::MINIMUM_INITIAL_ITEMS, run_board_stamp,
};
use serde_json::{Value, json};
use std::sync::Mutex;

/// The item count of a board, by number.
const BOARD_ITEMS_DOCUMENT: &str = "query LiveRunBoardItems($owner:String!,$number:Int!){repositoryOwner(login:$owner){... on ProjectV2Owner{projectV2(number:$number){id items(first:1){totalCount}}}}}";

/// How many times the copy's item count is read before the copy is refused as incomplete.
/// GitHub copies a board's draft items after it answers the mutation; this bounds the wait.
const COPY_READS: u32 = 30;

/// How many reads in a row past the first must agree before a count below the journey's
/// minimum is taken as the copy's whole: about ten seconds of GitHub's copy standing still.
const SETTLED_READS: u32 = 4;

/// A board this session copied for itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunBoard {
    /// The copy's node id.
    pub id: String,
    /// The copy's number under the nominated owner.
    pub number: u32,
}

/// What this process may name on the account: the boards it copied for its own sessions,
/// and — only to copy it — the template. One process may run more than one session (the e2e
/// binary's asset and reconcile journeys are two tests of one binary), each on a copy of its
/// own; another process's copies are learned from the answers this one relays.
#[derive(Clone, Debug, Default)]
struct Namespace {
    own: Vec<RunBoard>,
    templates: Vec<(String, u32)>,
    /// Boards of other processes' sessions this one has seen answered: never to be named.
    foreign: Vec<(String, u32)>,
}

static NAMESPACE: Mutex<Option<Namespace>> = Mutex::new(None);

/// Refuses, before it is sent, a request whose variables name a board outside this process's
/// namespace — the template, other than to copy it, or another run's copy — by id or number.
pub fn admit(document: &str, variables: &Value) -> Result<(), String> {
    if document == TEMPLATE_DOCUMENT || document == COPY_BOARD_DOCUMENT {
        return Ok(());
    }
    let namespace = NAMESPACE.lock().unwrap();
    let Some(namespace) = namespace.as_ref() else {
        return Ok(());
    };
    let own_number = |number: u32| namespace.own.iter().any(|board| board.number == number);
    for (id, number) in namespace.templates.iter().chain(&namespace.foreign) {
        if !own_number(*number)
            && variables.get("number").and_then(Value::as_u64) == Some(u64::from(*number))
        {
            return Err(format!(
                "refused before sending: this live session named board #{number}, which is not \
                 one this run copied for itself"
            ));
        }
        if names(variables, id) {
            return Err(format!(
                "refused before sending: this live session named board {id}, which is not one \
                 this run copied for itself"
            ));
        }
    }
    Ok(())
}

/// Learns, from an answer, every run board that is not one of this process's.
pub fn learn(response: &Value) {
    let mut found = Vec::new();
    boards_in(response, &mut found);
    if found.is_empty() {
        return;
    }
    let mut namespace = NAMESPACE.lock().unwrap();
    let namespace = namespace.get_or_insert_with(Namespace::default);
    for (id, number) in found {
        if !namespace.own.iter().any(|board| board.id == id)
            && !namespace.foreign.iter().any(|(known, _)| *known == id)
        {
            namespace.foreign.push((id, number));
        }
    }
}

fn boards_in(value: &Value, found: &mut Vec<(String, u32)>) {
    match value {
        Value::Object(object) => {
            if let (Some(id), Some(number), Some(title)) = (
                object.get("id").and_then(Value::as_str),
                object.get("number").and_then(Value::as_u64),
                object.get("title").and_then(Value::as_str),
            ) && run_board_stamp(title).is_some()
                && let Ok(number) = u32::try_from(number)
            {
                found.push((id.to_owned(), number));
            }
            object.values().for_each(|child| boards_in(child, found));
        }
        Value::Array(items) => items.iter().for_each(|item| boards_in(item, found)),
        _ => {}
    }
}

fn names(value: &Value, id: &str) -> bool {
    match value {
        Value::String(text) => text == id,
        Value::Object(object) => object.values().any(|child| names(child, id)),
        Value::Array(items) => items.iter().any(|item| names(item, id)),
        _ => false,
    }
}

/// How an entry point sends one GraphQL document — its own accounting — and gets back the
/// whole answer: `(document, what it is for, variables)`.
pub trait Sender: AsyncFn(&'static str, &'static str, Value) -> Result<Value, String> {}
impl<F: AsyncFn(&'static str, &'static str, Value) -> Result<Value, String>> Sender for F {}

/// The main entry point's sender: the journey's own accounting.
pub fn main_sender(token: &str) -> impl Sender + '_ {
    async move |document, what, variables| graphql_variables(token, document, what, variables).await
}

/// Copies the template `template` of `owner` into a board of this session's own, titled
/// with `stamp`, and waits until GitHub holds the copy's draft items; from here on every lane
/// request is held to that board.
pub async fn copy(
    send: &impl Sender,
    owner: &str,
    template: u32,
    stamp: &str,
) -> Result<RunBoard, String> {
    let title = onetaskgraph_github_live::run_board_title(stamp);
    debug_assert!(title.starts_with(RUN_BOARD_PREFIX));
    let found = send(
        TEMPLATE_DOCUMENT,
        "live template lookup",
        json!({"owner":owner,"number":template}),
    )
    .await?;
    let owner_id = found
        .pointer("/data/repositoryOwner/id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("GH_PROJECTS_OWNER={owner} names no owner this credential can see"))?
        .to_owned();
    let template_id = found
        .pointer("/data/repositoryOwner/projectV2/id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "GH_PROJECTS_OWNER={owner} with GH_PROJECTS_NUMBER={template} names no project \
                 this credential can see"
            )
        })?
        .to_owned();
    let copied = send(
        COPY_BOARD_DOCUMENT,
        "live board copy",
        json!({"input":{"projectId":template_id,"ownerId":owner_id,"title":title,"includeDraftIssues":true}}),
    )
    .await?;
    let board = copied.pointer("/data/copyProjectV2/projectV2");
    let (Some(id), Some(number)) = (
        board
            .and_then(|board| board.get("id"))
            .and_then(Value::as_str),
        board
            .and_then(|board| board.get("number"))
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok()),
    ) else {
        return Err("GitHub did not answer the live board copy with its id and number".into());
    };
    if id.is_empty() || id.chars().any(char::is_control) || id == template_id || number == template
    {
        return Err(format!(
            "the live board copy answered an unusable board: {id:?} #{number}"
        ));
    }
    let board = RunBoard {
        id: id.to_owned(),
        number,
    };
    {
        let mut namespace = NAMESPACE.lock().unwrap();
        let namespace = namespace.get_or_insert_with(Namespace::default);
        // The copy's own answer named it, and taught `learn` it was some run's: it is ours.
        namespace.foreign.retain(|(known, _)| *known != board.id);
        namespace.own.push(board.clone());
        if !namespace
            .templates
            .iter()
            .any(|(known, _)| *known == template_id)
        {
            namespace.templates.push((template_id, template));
        }
    }
    // GitHub copies the draft items after it answers; the session starts once they are there.
    let mut last = None;
    let mut agreeing = 0;
    for read in 0..COPY_READS {
        if read > 0 {
            github_pause(std::time::Duration::from_secs(2)).await;
        }
        let answer = send(
            BOARD_ITEMS_DOCUMENT,
            "live board copy item count",
            json!({"owner":owner,"number":number}),
        )
        .await?;
        let items = answer
            .pointer("/data/repositoryOwner/projectV2/items/totalCount")
            .and_then(Value::as_u64);
        // Settled when two reads agree on a count the journey can run on, or when several in a
        // row agree on any count; whether that count is the registered workload is the
        // pre-flight's to judge, in its own words.
        agreeing = if items.is_some() && items == last {
            agreeing + 1
        } else {
            0
        };
        if (agreeing >= 1 && items.is_some_and(|items| items >= MINIMUM_INITIAL_ITEMS))
            || agreeing >= SETTLED_READS
        {
            return Ok(board);
        }
        last = items;
    }
    Err(format!(
        "the live board copy #{number} did not settle (last read {last:?} items); its template \
         needs at least {MINIMUM_INITIAL_ITEMS} draft items — run `just github-live-template \
         dry-run`"
    ))
}

/// Deletes this session's board, and with it every item and field it held.
pub async fn delete(send: &impl Sender, board: &RunBoard) -> Result<(), String> {
    let answer = send(
        DELETE_BOARD_DOCUMENT,
        "live board deletion",
        json!({"input":{"projectId":board.id}}),
    )
    .await?;
    if answer
        .pointer("/data/deleteProjectV2/projectV2/id")
        .and_then(Value::as_str)
        != Some(board.id.as_str())
    {
        return Err(format!(
            "GitHub did not confirm deleting live board #{}",
            board.number
        ));
    }
    Ok(())
}

/// Deletes the board when dropped unless [`Guard::deleted`] was called: the way out of a
/// session that panicked anywhere between the copy and its own cleanup.
pub struct Guard {
    token: String,
    board: Mutex<Option<RunBoard>>,
}

impl Guard {
    pub fn new(token: &str, board: &RunBoard) -> Self {
        Self {
            token: token.to_owned(),
            board: Mutex::new(Some(board.clone())),
        }
    }

    /// The session's own cleanup deleted the board, or tried to: nothing is left to do.
    pub fn deleted(&self) {
        *self.board.lock().unwrap() = None;
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let Some(board) = self.board.lock().unwrap().take() else {
            return;
        };
        let token = self.token.clone();
        // A fresh runtime on a thread of its own: this may run while a runtime unwinds.
        let outcome = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?
                .block_on(delete(&main_sender(&token), &board))
        })
        .join();
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                eprintln!("the live board copy was not deleted after a failure: {error}");
            }
            Err(_) => eprintln!("the live board copy's deletion panicked after a failure"),
        }
    }
}

/// The variable asking the main entry point for the operator's template setup instead of its
/// journey: `dry-run` reports, `fill` writes. See `just github-live-template`.
pub const TEMPLATE_VARIABLE: &str = "ONETASKGRAPH_GITHUB_TEMPLATE";

/// What [`TEMPLATE_VARIABLE`] asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateSetup {
    /// Count the template's draft items and say what filling would add; write nothing.
    DryRun,
    /// Add draft items until the template holds [`onetaskgraph_github_live::TEMPLATE_DRAFTS`].
    Fill,
}

impl TemplateSetup {
    /// `None` when the variable is unset: the entry point runs its journey.
    pub fn read() -> Result<Option<Self>, String> {
        match std::env::var(TEMPLATE_VARIABLE).as_deref() {
            Err(std::env::VarError::NotPresent) => Ok(None),
            Ok("dry-run") => Ok(Some(Self::DryRun)),
            Ok("fill") => Ok(Some(Self::Fill)),
            _ => Err(format!("{TEMPLATE_VARIABLE} must be dry-run or fill")),
        }
    }
}

/// The operator's one-time setup of the nominated template every session copies: it counts
/// the template's draft items — the only items a copy carries — and, filling, adds draft items
/// titled [`onetaskgraph_github_live::TEMPLATE_DRAFT_TITLE`], one a second, until it holds
/// [`onetaskgraph_github_live::TEMPLATE_DRAFTS`]. Idempotent: a template already holding that
/// many is left as it is. A template holding more drafts than the registered workload's board
/// is reported, never trimmed. Returns one line per finding.
pub async fn setup_template(
    send: &impl Sender,
    owner: &str,
    template: u32,
    setup: TemplateSetup,
) -> Result<Vec<String>, String> {
    use onetaskgraph_github_live::{
        ADD_DRAFT_DOCUMENT, TEMPLATE_DRAFT_TITLE, TEMPLATE_DRAFTS, TEMPLATE_ITEMS_DOCUMENT,
        quota::reference_workload,
    };
    let found = send(
        TEMPLATE_DOCUMENT,
        "live template lookup",
        json!({"owner":owner,"number":template}),
    )
    .await?;
    let id = found
        .pointer("/data/repositoryOwner/projectV2/id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{owner} has no board #{template} this credential can see"))?
        .to_owned();
    let mut drafts = 0u64;
    let mut after = Value::Null;
    for _ in 0..1_000 {
        let page = send(
            TEMPLATE_ITEMS_DOCUMENT,
            "live template items",
            json!({"id":id,"after":after}),
        )
        .await?;
        let items = page
            .pointer("/data/node/items")
            .ok_or("the template's items were not reported")?;
        drafts += items["nodes"]
            .as_array()
            .ok_or("the template's item nodes were not reported")?
            .iter()
            .filter(|node| node.pointer("/content/__typename") == Some(&json!("DraftIssue")))
            .count() as u64;
        if items.pointer("/pageInfo/hasNextPage") != Some(&json!(true)) {
            break;
        }
        let next = items
            .pointer("/pageInfo/endCursor")
            .filter(|next| **next != after)
            .ok_or("the template's item listing did not advance")?;
        after = next.clone();
    }
    let ceiling = reference_workload().initial_items;
    let mut report = vec![format!("template #{template} holds {drafts} draft items")];
    if drafts > ceiling {
        report.push(format!(
            "template #{template} holds more than the {ceiling} items the registered workload \
             measured; remove draft items by hand — nothing here deletes from the template"
        ));
        return Ok(report);
    }
    let missing = TEMPLATE_DRAFTS.saturating_sub(drafts);
    match (setup, missing) {
        (_, 0) => report.push("nothing to add".into()),
        (TemplateSetup::DryRun, _) => report.push(format!(
            "would add {missing} draft items titled {TEMPLATE_DRAFT_TITLE:?}, reaching {TEMPLATE_DRAFTS}"
        )),
        (TemplateSetup::Fill, _) => {
            for added in 0..missing {
                if added > 0 {
                    github_pause(std::time::Duration::from_secs(1)).await;
                }
                send(
                    ADD_DRAFT_DOCUMENT,
                    "live template draft",
                    json!({"input":{"projectId":id,"title":TEMPLATE_DRAFT_TITLE}}),
                )
                .await?;
            }
            report.push(format!("added {missing} draft items, reaching {TEMPLATE_DRAFTS}"));
        }
    }
    Ok(report)
}
