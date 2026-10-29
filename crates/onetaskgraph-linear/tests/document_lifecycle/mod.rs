//! Shared infrastructure for the two real Linear document lanes.
pub mod journal;
pub mod observer;
use onetaskgraph_linear::live_artifact::{Scope, Slot, classify_retained, retained_title};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
const DOCUMENTS: &str = "query($after:String,$filter:DocumentFilter!){documents(first:100,after:$after,filter:$filter){nodes{id title team{key} project{teams(first:100){nodes{key} pageInfo{hasNextPage}}}} pageInfo{hasNextPage endCursor}}}";
const PROJECTS: &str = "query($after:String,$filter:ProjectFilter!){projects(first:100,after:$after,filter:$filter){nodes{id name teams(first:100){nodes{key} pageInfo{hasNextPage}}} pageInfo{hasNextPage endCursor}}}";
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
/// Ordinary discovery is read-only. Initialization alone can fill missing identities.
pub async fn prepare(
    endpoint: &str,
    key: &str,
    team: &str,
    team_id: &str,
    scope: Scope<'_>,
    mode: Mode,
    slots: &[Slot],
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
    if mode == Mode::Ordinary
        && wanted
            .iter()
            .any(|s| !standing.ids.contains_key(slot_key(*s)))
    {
        return Err(format!(
            "standing Linear document set absent or incomplete; reuse would create documents; run {MODE_VARIABLE}=initialize under the shared CI concurrency group before ordinary reuse; no documentCreate sent"
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
    }
    standing.parent = standing.ids.get("parent").cloned();
    Ok(standing)
}
