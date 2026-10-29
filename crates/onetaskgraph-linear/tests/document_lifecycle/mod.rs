//! Shared infrastructure for the two real Linear document lanes.
pub mod journal;
pub mod observer;
use onetaskgraph_linear::live_artifact::{Scope, Slot, classify_retained, retained_title};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const MODE_VARIABLE: &str = "ONETASKGRAPH_LINEAR_DOCUMENT_MODE";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ordinary,
    Fresh,
    Initialize,
}
impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::Fresh => "fresh",
            Self::Initialize => "initialize",
        }
    }

    pub fn read() -> Result<Self, String> {
        match std::env::var(MODE_VARIABLE).as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("ordinary") => Ok(Self::Ordinary),
            Ok("fresh") => Ok(Self::Fresh),
            Ok("initialize") => Ok(Self::Initialize),
            _ => Err(format!(
                "{MODE_VARIABLE} must be ordinary, fresh or initialize"
            )),
        }
    }
}

pub struct Standing {
    pub titles: BTreeMap<&'static str, String>,
    pub ids: BTreeMap<&'static str, String>,
    pub parent: Option<String>,
}
pub fn slot_key(slot: Slot) -> &'static str {
    match slot {
        Slot::Filed => "filed",
        Slot::Loose => "loose",
        Slot::FollowUp => "follow-up",
        Slot::Assets => "assets",
        Slot::Parent => "parent",
    }
}

pub async fn send(
    endpoint: &str,
    key: &str,
    query: &str,
    variables: Value,
) -> Result<Value, String> {
    let body: Value = reqwest::Client::new()
        .post(endpoint)
        .header("Authorization", key)
        .json(&json!({"query":query,"variables":variables}))
        .send()
        .await
        .map_err(|e| {
            format!("Linear lifecycle request failed; discover ownership before retry: {e}")
        })?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if body
        .get("errors")
        .is_some_and(|e| !e.as_array().is_some_and(Vec::is_empty))
    {
        return Err("Linear rejected lifecycle request; discover ownership before retry".into());
    }
    body.get("data")
        .cloned()
        .ok_or("Linear lifecycle response omitted data".into())
}
// `teams(first:2)`: a full nested page puts these 100-row listings over Linear's complexity
// cap (`complexity.rs`), and two still lets `in_team` see a second team and fail closed.
const DOCUMENTS: &str = "query($after:String,$filter:DocumentFilter!){documents(first:100,after:$after,filter:$filter){nodes{id title team{key} project{teams(first:2){nodes{key} pageInfo{hasNextPage}}}} pageInfo{hasNextPage endCursor}}}";
const PROJECTS: &str = "query($after:String,$filter:ProjectFilter!){projects(first:100,after:$after,filter:$filter){nodes{id name teams(first:2){nodes{key} pageInfo{hasNextPage}}} pageInfo{hasNextPage endCursor}}}";
fn in_team(node: &Value, team: &str, project: bool) -> bool {
    if !project {
        match node.get("team") {
            Some(Value::Null) => {}
            Some(Value::Object(direct)) => {
                return direct
                    .get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|key| key.eq_ignore_ascii_case(team));
            }
            _ => return false,
        }
    }
    let teams = if project {
        node.get("teams")
    } else {
        node.pointer("/project/teams")
    };
    teams.is_some_and(|teams| {
        teams.pointer("/pageInfo/hasNextPage") == Some(&json!(false))
            && teams["nodes"].as_array().is_some_and(|nodes| {
                !nodes.is_empty()
                    && nodes.iter().all(|n| {
                        n["key"]
                            .as_str()
                            .is_some_and(|key| key.eq_ignore_ascii_case(team))
                    })
            })
    })
}
async fn discover(
    endpoint: &str,
    key: &str,
    title: &str,
    team: &str,
    project: bool,
) -> Result<Option<String>, String> {
    let root = if project { "projects" } else { "documents" };
    let name = if project { "name" } else { "title" };
    let query = if project { PROJECTS } else { DOCUMENTS };
    let mut after = Value::Null;
    let mut found = Vec::new();
    for _ in 0..50 {
        let filter = json!({name:{"containsIgnoreCase":format!("{}retained",onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX)}});
        let data = send(endpoint, key, query, json!({"after":after,"filter":filter})).await?;
        let page = &data[root];
        for node in page["nodes"]
            .as_array()
            .ok_or("standing discovery omitted nodes")?
        {
            let actual_title = node[name]
                .as_str()
                .ok_or("standing discovery omitted title")?;
            let actual = classify_retained(actual_title)
                .ok_or("unknown standing ownership; discovery required before initialization")?;
            let expected = classify_retained(title).ok_or("invalid requested standing identity")?;
            if actual.scope != expected.scope {
                continue;
            }
            if !in_team(node, team, project) {
                return Err("standing ownership or nominated-team membership cannot be verified; no update sent".into());
            }
            if actual.slot != expected.slot {
                continue;
            }
            found.push(
                node["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("standing discovery omitted id")?
                    .to_owned(),
            );
        }
        match page
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
        {
            Some(false) => {
                return match found.len() {
                    0 => Ok(None),
                    1 => Ok(found.pop()),
                    _ => Err("duplicate standing ownership; no update sent".into()),
                };
            }
            Some(true) => {
                let next = page
                    .pointer("/pageInfo/endCursor")
                    .and_then(Value::as_str)
                    .ok_or("standing discovery omitted cursor")?;
                if after.as_str() == Some(next) {
                    return Err("standing discovery cursor did not advance".into());
                }
                after = json!(next);
            }
            None => return Err("standing discovery omitted pagination".into()),
        }
    }
    Err("standing discovery did not terminate".into())
}
/// Every slot this invocation created, recorded the moment Linear acknowledged its id.
///
/// A local set is the invocation's own, so a failure or panic anywhere after its first
/// create — partway through initialization included — has to retire exactly these ids
/// rather than leave them for the orphan sweep. [`retire`] drains it, so a second call
/// after the ordinary cleanup has run deletes nothing twice.
#[derive(Clone, Default)]
pub struct Created {
    ids: Arc<Mutex<Vec<(Slot, String)>>>,
    panic_after: Option<usize>,
}
impl Created {
    /// Fault injection for the offline journeys: panic straight after the `n`th create
    /// is recorded, which is the earliest point a lane can have stranded a slot.
    pub fn panicking_after(n: usize) -> Self {
        Self {
            panic_after: Some(n),
            ..Self::default()
        }
    }
    fn record(&self, slot: Slot, id: &str) {
        let count = {
            let mut ids = self.ids.lock().unwrap();
            ids.push((slot, id.to_owned()));
            ids.len()
        };
        if self.panic_after == Some(count) {
            panic!("injected failure after standing create {count}");
        }
    }
    /// Drops the record once another cleanup has attempted every recorded slot itself.
    #[allow(
        dead_code,
        reason = "this module is compiled into both lanes' test binaries, and only the asset lane's own cleanup retires slots by its standing record"
    )]
    pub fn forget(&self) {
        self.ids.lock().unwrap().clear();
    }
    pub fn len(&self) -> usize {
        self.ids.lock().unwrap().len()
    }
}

/// Deletes, newest first, every slot `created` holds — documents before the parent project
/// they are filed under — when the set is invocation-owned. A CI set is standing and is
/// never retired here. Returns how many were deleted; every refusal is named.
pub async fn retire(
    endpoint: &str,
    key: &str,
    scope: Scope<'_>,
    created: &Created,
) -> Result<usize, String> {
    if scope.is_ci() {
        return Ok(0);
    }
    let held = std::mem::take(&mut *created.ids.lock().unwrap());
    let mut refused = Vec::new();
    let mut retired = 0;
    for (slot, id) in held.into_iter().rev() {
        let (query, root) = if slot == Slot::Parent {
            (
                onetaskgraph_linear::graphql::PROJECT_DELETE,
                "projectDelete",
            )
        } else {
            (
                onetaskgraph_linear::graphql::DOCUMENT_DELETE,
                "documentDelete",
            )
        };
        match send(endpoint, key, query, json!({"id":id})).await {
            Ok(data) if data[root]["success"] == true => retired += 1,
            Ok(_) => refused.push(format!(
                "Linear did not confirm retiring {} slot {id}",
                slot_key(slot)
            )),
            Err(error) => refused.push(format!("{} slot {id}: {error}", slot_key(slot))),
        }
    }
    if refused.is_empty() {
        Ok(retired)
    } else {
        Err(format!(
            "invocation-owned standing slots were not all retired: {}",
            refused.join("; ")
        ))
    }
}

/// Polls `future`, turning a panic inside it into `Err` so the caller can clean up first.
pub async fn unwinding<F: std::future::Future>(future: F) -> std::thread::Result<F::Output> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Ok(std::task::Poll::Ready(output)) => std::task::Poll::Ready(Ok(output)),
            Err(panic) => std::task::Poll::Ready(Err(panic)),
        }
    })
    .await
}

/// [`prepare`] for a live lane: a failure or panic after any slot create retires every slot
/// this invocation created before it is reported, so nothing it owns outlives the process.
#[allow(
    clippy::too_many_arguments,
    reason = "prepare's seven lane identities plus the tracker its caller must hold for cleanup; a struct would only restate them"
)]
pub async fn prepare_owned(
    endpoint: &str,
    key: &str,
    team: &str,
    team_id: &str,
    scope: Scope<'_>,
    mode: Mode,
    slots: &[Slot],
    created: &Created,
) -> Result<Standing, String> {
    let outcome = unwinding(prepare_tracked(
        endpoint, key, team, team_id, scope, mode, slots, created,
    ))
    .await;
    if !matches!(outcome, Ok(Ok(_))) {
        discover_stranded(endpoint, key, team, scope, slots, created).await;
    }
    match outcome {
        Ok(Ok(standing)) => Ok(standing),
        Ok(Err(error)) => Err(match retire(endpoint, key, scope, created).await {
            Ok(_) => error,
            Err(retired) => format!("{error}; additionally, {retired}"),
        }),
        Err(panic) => {
            if let Err(retired) = retire(endpoint, key, scope, created).await {
                eprintln!("{retired}");
            }
            std::panic::resume_unwind(panic)
        }
    }
}

/// A local create whose acknowledgement named no id may still have stored the slot, so a
/// failed initialization finds each slot it did not record by its exact identity — the
/// discovery recovery requires — and records it for [`retire`]. A CI set is left as it is.
async fn discover_stranded(
    endpoint: &str,
    key: &str,
    team: &str,
    scope: Scope<'_>,
    slots: &[Slot],
    created: &Created,
) {
    if scope.is_ci() {
        return;
    }
    let mut wanted = slots.to_vec();
    if slots.contains(&Slot::Filed) {
        wanted.insert(0, Slot::Parent);
    }
    for slot in wanted {
        if created
            .ids
            .lock()
            .unwrap()
            .iter()
            .any(|(held, _)| *held == slot)
        {
            continue;
        }
        let Ok(title) = retained_title(scope, slot) else {
            continue;
        };
        match discover(endpoint, key, &title, team, slot == Slot::Parent).await {
            Ok(Some(id)) => {
                // The parent goes last in `retire`, which deletes newest first.
                let mut ids = created.ids.lock().unwrap();
                if slot == Slot::Parent {
                    ids.insert(0, (slot, id));
                } else {
                    ids.push((slot, id));
                }
            }
            Ok(None) => {}
            Err(error) => eprintln!(
                "{} slot could not be discovered for retirement: {error}",
                slot_key(slot)
            ),
        }
    }
}
/// Ordinary non-CI discovery is read-only; initialization, and an ordinary CI pass as a
/// cold rebuild, fill only the missing identities.
pub async fn prepare(
    endpoint: &str,
    key: &str,
    team: &str,
    team_id: &str,
    scope: Scope<'_>,
    mode: Mode,
    slots: &[Slot],
) -> Result<Standing, String> {
    prepare_tracked(
        endpoint,
        key,
        team,
        team_id,
        scope,
        mode,
        slots,
        &Created::default(),
    )
    .await
}

#[allow(
    clippy::too_many_arguments,
    reason = "the same lane identities as prepare_owned, which forwards each one unchanged"
)]
async fn prepare_tracked(
    endpoint: &str,
    key: &str,
    team: &str,
    team_id: &str,
    scope: Scope<'_>,
    mode: Mode,
    slots: &[Slot],
    created: &Created,
) -> Result<Standing, String> {
    let nominated = scope.team();
    if !nominated.eq_ignore_ascii_case(team)
        || team_id.trim().is_empty()
        || team_id.chars().any(char::is_control)
    {
        return Err("standing scope and nominated team disagree or team id is unverifiable".into());
    }
    let mut standing = Standing {
        titles: BTreeMap::new(),
        ids: BTreeMap::new(),
        parent: None,
    };
    if mode == Mode::Fresh {
        return Ok(standing);
    }
    let mut wanted = slots.to_vec();
    if slots.contains(&Slot::Filed) {
        wanted.insert(0, Slot::Parent);
    }
    // Discover the entire lane before sending even an initialization mutation.
    for slot in &wanted {
        let title = retained_title(scope, *slot)?;
        if let Some(id) = discover(endpoint, key, &title, team, *slot == Slot::Parent).await? {
            standing.ids.insert(slot_key(*slot), id);
        }
        standing.titles.insert(slot_key(*slot), title);
    }
    // An ordinary CI pass rebuilds a missing standing slot itself, under the shared CI
    // concurrency group, and its journal files that create as a cold rebuild outside the
    // ordinary total. A non-CI set is the invocation's own and is created only on request.
    if mode == Mode::Ordinary
        && !scope.is_ci()
        && wanted
            .iter()
            .any(|s| !standing.ids.contains_key(slot_key(*s)))
    {
        return Err(format!(
            "invocation-owned Linear document set absent or incomplete; non-CI ordinary reuse would create documents; run {MODE_VARIABLE}=initialize for an explicit creation pass; no documentCreate sent"
        ));
    }
    for slot in wanted {
        let name = slot_key(slot);
        if standing.ids.contains_key(name) {
            continue;
        }
        let title = &standing.titles[name];
        let (query, input, root, field) = if slot == Slot::Parent {
            (
                "mutation($input:ProjectCreateInput!){projectCreate(input:$input){success project{id}}}",
                json!({"name":title,"teamIds":[team_id]}),
                "projectCreate",
                "project",
            )
        } else {
            let mut input =
                json!({"title":title,"content":"Explicit standing-set initialization."});
            if slot == Slot::Filed {
                input["projectId"] = json!(standing.ids["parent"]);
            } else {
                input["teamId"] = json!(team_id);
            }
            (
                "mutation($input:DocumentCreateInput!){documentCreate(input:$input){success document{id}}}",
                input,
                "documentCreate",
                "document",
            )
        };
        let data = send(endpoint, key, query, json!({"input":input}))
            .await
            .map_err(|error| {
                format!(
                    "uncertain standing create; discover exact identity before recovery: {error}"
                )
            })?;
        if data[root]["success"] != true {
            return Err("uncertain standing create; discover exact identity before recovery; no automatic retry".into());
        }
        let id=data[root][field]["id"].as_str().filter(|s| !s.is_empty()).ok_or("uncertain standing create acknowledgement; discover exact identity before recovery; no automatic retry")?;
        standing.ids.insert(name, id.to_owned());
        created.record(slot, id);
    }
    standing.parent = standing.ids.get("parent").cloned();
    Ok(standing)
}
