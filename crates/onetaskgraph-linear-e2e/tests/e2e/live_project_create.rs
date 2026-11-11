//! The project-create journey against real Linear, run inside the asset lane's live session.
//!
//! The lane's other journeys build their fixtures with raw GraphQL creates that send no id, so
//! none of them ever sent the keyed `projectCreate` a `project copy` sends; a release could stop
//! creating projects on Linear and the lane would stay green. This journey drives the binary's
//! `project copy` of a plan Linear does not hold yet into the nominated team, through a relay that
//! can lose one create's answer after Linear applied it, and reads by the plugin ids Linear does
//! not hold. Everything it makes carries this run's stamp in its title and is removed again by
//! that stamp however the journey ends.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{document, linear_workspace};
use crate::linear_assets::document_lifecycle::namespace::Namespace;
use crate::project_create::{outcome, plan_named, qualified, succeeded, task_title, titles};

const LINEAR: &str = "https://api.linear.app/graphql";

/// Reads after a write that may still answer from before it, and their spacing.
const READS: u32 = 20;

/// One line of what Linear answered this journey, written to the process's own standard error
/// rather than through `eprintln!`, which the test harness captures and shows only for a failing
/// test: so the credentialed lane's log carries what Linear said on a passing run as well.
macro_rules! evidence {
    ($($line:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(
            std::io::stderr().lock(),
            "live project create: {}",
            format_args!($($line)*)
        );
    }};
}

/// What the relay is told to do, and what it saw.
#[derive(Default)]
struct Relay {
    /// Lose the answer to the next `projectCreate`, after Linear applied it.
    lose_create: bool,
    /// Refuse this many more `issueCreate`s, as Linear refuses a write, without sending them.
    refuse_issue_creates: usize,
    /// Fail the next read of one project by id, without sending it to Linear: armed by the lost
    /// create alone, so it is the read the plugin makes for what that create left.
    fail_project_read: bool,
    /// Each create Linear refused: its root field, the id it was sent under and Linear's first
    /// error, held to the plugin's contract constants by the journey.
    refusals: Vec<(&'static str, String, Value)>,
    /// The root field of every request relayed, in order, for the log.
    sent: Vec<String>,
    /// Every create Linear answered with an id: its kind and the id.
    created: Vec<(&'static str, String)>,
    /// What Linear answered each create with, for the log.
    observations: Vec<String>,
}

fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
        let size = head
            .to_ascii_lowercase()
            .lines()
            .find_map(|line| line.strip_prefix("content-length: ").map(str::to_owned))
            .and_then(|size| size.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if bytes.len() >= split + 4 + size {
            return Some((head, bytes[split + 4..split + 4 + size].to_vec()));
        }
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Relay\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

/// The single documents `query` carries, each with the alias it is answered under: an aliased
/// batch the plugin composed is each of its documents under `a<i>`, and anything else is itself,
/// answered under its own root field.
fn singles(query: &str) -> Vec<(Option<String>, &str)> {
    onetaskgraph_linear::graphql::decomposed(query).map_or_else(
        || vec![(None, query)],
        |singles| {
            singles
                .into_iter()
                .enumerate()
                .map(|(at, single)| (Some(format!("a{at}")), single))
                .collect()
        },
    )
}

/// A relay in front of `upstream` — Linear, or a loopback standing in for it — holding every
/// request to this run's namespace, and the handle that tells it to lose an answer.
fn relay(upstream: String, key: String, namespace: Arc<Namespace>) -> (String, Arc<Mutex<Relay>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(Relay::default()));
    let held = Arc::clone(&state);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::new();
        for mut stream in listener.incoming().flatten() {
            let Some((_, body)) = read_request(&mut stream) else {
                continue;
            };
            let Ok(request) = serde_json::from_slice::<Value>(&body) else {
                respond(
                    &mut stream,
                    400,
                    br#"{"errors":[{"message":"relay: not JSON"}]}"#,
                );
                continue;
            };
            if let Err(refusal) = namespace.admit(&request) {
                respond(
                    &mut stream,
                    200,
                    json!({"errors":[{"message":refusal}]})
                        .to_string()
                        .as_bytes(),
                );
                continue;
            }
            let query = request["query"].as_str().unwrap_or_default().to_owned();
            let root = query
                .split_once('{')
                .map(|(_, rest)| {
                    rest.trim_start()
                        .split(['(', ' ', '{'])
                        .next()
                        .unwrap_or("")
                })
                .unwrap_or("")
                .to_owned();
            lock(&held).sent.push(root);
            // The single documents this request carries, each with the alias it is answered
            // under: an aliased batch of creates is as much a create as one sent alone.
            let parts = singles(&query);
            let first_create = parts
                .iter()
                .find(|(_, single)| *single == onetaskgraph_linear::graphql::ISSUE_CREATE);
            let refusing = lock(&held).refuse_issue_creates > 0;
            if let Some((alias, _)) = first_create
                && refusing
            {
                let refused = match alias {
                    // Refused as Linear refuses an alias of a batch: nothing of the document is
                    // applied, and the error's path names the alias — so the plugin sends that
                    // create again on its own, which is the create this refusal counts.
                    Some(alias) => json!({"errors":[{"message":"relay: this task is refused",
                                                      "path":[alias]}],"data":null}),
                    None => {
                        let mut relay = lock(&held);
                        relay.refuse_issue_creates -= 1;
                        json!({"errors":[{"message":"relay: this task is refused"}],"data":null})
                    }
                };
                respond(&mut stream, 200, refused.to_string().as_bytes());
                continue;
            }
            if query == onetaskgraph_linear::graphql::PROJECT
                && std::mem::take(&mut lock(&held).fail_project_read)
            {
                respond(
                    &mut stream,
                    502,
                    br#"{"errors":[{"message":"relay: lost"}]}"#,
                );
                continue;
            }
            let forwarded = runtime.block_on(async {
                let response = client
                    .post(&upstream)
                    .header("Authorization", &key)
                    .json(&request)
                    .send()
                    .await?;
                let status = response.status().as_u16();
                Ok::<_, reqwest::Error>((status, response.bytes().await?))
            });
            let (status, bytes) = match forwarded {
                Ok(answered) => answered,
                Err(error) => {
                    let said = json!({"errors":[{"message":format!("relay: {error}")}]});
                    respond(&mut stream, 502, said.to_string().as_bytes());
                    continue;
                }
            };
            let answer: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            namespace.learn(&answer);
            let mut relay = lock(&held);
            for (at, (alias, single)) in parts.iter().enumerate() {
                let Some((root, field, kind)) = [
                    (
                        onetaskgraph_linear::graphql::ISSUE_CREATE,
                        "issueCreate",
                        "issue",
                        "issue",
                    ),
                    (
                        onetaskgraph_linear::graphql::PROJECT_CREATE,
                        "projectCreate",
                        "project",
                        "project",
                    ),
                    (
                        onetaskgraph_linear::graphql::DOCUMENT_CREATE,
                        "documentCreate",
                        "document",
                        "document",
                    ),
                ]
                .into_iter()
                .find(|(create, ..)| create == single)
                .map(|(_, root, field, kind)| (root, field, kind)) else {
                    continue;
                };
                // A create sent alone is answered under its root field and takes its variables
                // as they are; an alias of a batch is answered under the alias and takes its own
                // suffixed variables.
                let (answered, variables) = match alias {
                    Some(alias) => (
                        &answer["data"][alias],
                        onetaskgraph_linear::graphql::alias_variables(&request["variables"], at),
                    ),
                    None => (&answer["data"][root], request["variables"].clone()),
                };
                let error = answer["errors"].as_array().and_then(|errors| {
                    errors.iter().find(|error| match alias {
                        Some(alias) => error["path"].get(0).and_then(Value::as_str) == Some(alias),
                        None => true,
                    })
                });
                relay.observations.push(format!(
                    "{}{root}: HTTP {status}, errors={}",
                    alias
                        .as_ref()
                        .map(|alias| format!("{alias}: "))
                        .unwrap_or_default(),
                    error.cloned().unwrap_or(Value::Null)
                ));
                if let Some(id) = answered[field]["id"].as_str() {
                    relay.created.push((kind, id.to_owned()));
                }
                if let Some(error) = error {
                    let sent = variables["input"]["id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    relay.refusals.push((root, sent, error.clone()));
                }
            }
            if query == onetaskgraph_linear::graphql::PROJECT_CREATE
                && std::mem::take(&mut relay.lose_create)
            {
                relay.fail_project_read = true;
                drop(relay);
                respond(
                    &mut stream,
                    502,
                    br#"{"errors":[{"message":"relay: lost"}]}"#,
                );
                continue;
            }
            drop(relay);
            respond(&mut stream, status, &bytes);
        }
    });
    (endpoint, state)
}

/// The relay's state, whether or not a journey assertion panicked while reading it.
fn lock(relay: &Mutex<Relay>) -> std::sync::MutexGuard<'_, Relay> {
    relay
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the relay saw, for a failure's message.
fn seen(relay: &Mutex<Relay>) -> String {
    let relay = lock(relay);
    format!("creates: {:#?}\nsent: {:?}", relay.observations, relay.sent)
}

/// One request to Linear, answered whole — its `errors` included, so a refusal can be read.
fn graphql(runtime: &tokio::runtime::Runtime, key: &str, query: &str, variables: Value) -> Value {
    runtime.block_on(async {
        reqwest::Client::new()
            .post(LINEAR)
            .header("Authorization", key)
            .json(&json!({"query": query, "variables": variables}))
            .send()
            .await
            .expect("Linear answers")
            .json::<Value>()
            .await
            .expect("Linear answers JSON")
    })
}

/// A plan project id of this run's own for `scenario`: a create's key is the same for every
/// copy of one source id into one destination name, so a fixed id would meet an earlier run's
/// copy in the team as this copy's own counterpart.
fn slug(stamp: &str, scenario: &str) -> String {
    let digest = Sha256::digest(format!("{stamp} {scenario}").as_bytes());
    let hex: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{scenario}-{hex}")
}

/// A v4 UUID no item of Linear's holds, derived from this run's stamp and `kind`.
fn absent_id(stamp: &str, kind: &str) -> String {
    let digest = Sha256::digest(format!("{stamp} absent {kind}").as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let key = format!(
        "{}-{}-5{}-8{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    );
    onetaskgraph_linear::linear_id_of_key(&key).expect("a key-shaped id")
}

/// Every project and issue of the team whose title carries `stamp`, with Linear's ids — or why
/// Linear's answer does not say which those are, so a listing nobody could read is never taken
/// for one holding nothing.
fn stamped(
    runtime: &tokio::runtime::Runtime,
    key: &str,
    team_id: &str,
    stamp: &str,
) -> Result<Vec<(&'static str, String, String)>, String> {
    let mut found = Vec::new();
    for (kind, root, title, query) in [
        (
            "project",
            "projects",
            "name",
            "query($stamp:String!,$team:ID!){ projects(first:50,filter:{name:{contains:$stamp},accessibleTeams:{some:{id:{eq:$team}}}}){ nodes{id name} pageInfo{hasNextPage} } }",
        ),
        (
            "issue",
            "issues",
            "title",
            "query($stamp:String!,$team:ID!){ issues(first:50,filter:{title:{contains:$stamp},team:{id:{eq:$team}}}){ nodes{id title} pageInfo{hasNextPage} } }",
        ),
    ] {
        let answer = graphql(
            runtime,
            key,
            query,
            json!({"stamp": stamp, "team": team_id}),
        );
        if answer.get("errors").is_some() {
            return Err(format!(
                "Linear refused the stamped {root} listing: {answer}"
            ));
        }
        let connection = &answer["data"][root];
        let nodes = connection["nodes"]
            .as_array()
            .ok_or_else(|| format!("Linear answered the {root} listing with no nodes: {answer}"))?;
        if connection["pageInfo"]["hasNextPage"] != false {
            return Err(format!(
                "the stamped {root} listing did not end on its first page: {answer}"
            ));
        }
        for node in nodes {
            let (Some(id), Some(name)) = (node["id"].as_str(), node[title].as_str()) else {
                return Err(format!(
                    "Linear answered a {kind} with no id or {title}: {node}"
                ));
            };
            found.push((kind, id.to_owned(), name.to_owned()));
        }
    }
    Ok(found)
}

/// Remove every item `created` names and every stamped item the team still holds; whether all
/// of them went.
fn cleanup(
    runtime: &tokio::runtime::Runtime,
    key: &str,
    team_id: &str,
    stamp: &str,
    created: &[(&'static str, String)],
) -> bool {
    let mut items: Vec<(&'static str, String)> = created.to_vec();
    // What the relay saw created is removed whatever the listing answers; a listing nobody
    // could read fails the cleanup rather than passing it.
    let (listed, mut all) = match stamped(runtime, key, team_id, stamp) {
        Ok(listed) => (listed, true),
        Err(problem) => {
            evidence!("{problem}");
            (Vec::new(), false)
        }
    };
    for (kind, id, _) in listed {
        if !items.iter().any(|(_, held)| *held == id) {
            items.push((kind, id));
        }
    }
    // Issues before projects, so no issue outlives the project it was filed under.
    items.sort_by_key(|(kind, _)| *kind != "issue");
    for (kind, id) in items {
        let (query, root) = match kind {
            "issue" => (onetaskgraph_linear::graphql::ISSUE_DELETE, "issueDelete"),
            "project" => (
                onetaskgraph_linear::graphql::PROJECT_DELETE,
                "projectDelete",
            ),
            _ => (
                onetaskgraph_linear::graphql::DOCUMENT_DELETE,
                "documentDelete",
            ),
        };
        let answer = graphql(runtime, key, query, json!({"id": id}));
        if answer["data"][root]["success"] != true && !already_gone(runtime, key, kind, &id) {
            evidence!("could not remove {kind} {id}: {answer}");
            all = false;
        }
    }
    all
}

/// Whether the `kind` Linear holds under `id` is gone already — not there at all, or in the
/// trash, as a copy's own undo leaves what it took back — so a delete of it has nothing to do.
fn already_gone(runtime: &tokio::runtime::Runtime, key: &str, kind: &str, id: &str) -> bool {
    let (query, root, missing) = match kind {
        "issue" => (
            onetaskgraph_linear::graphql::ISSUE,
            "issue",
            onetaskgraph_linear::graphql::ISSUE_NOT_FOUND_MESSAGE,
        ),
        "project" => (
            onetaskgraph_linear::graphql::PROJECT,
            "project",
            onetaskgraph_linear::graphql::PROJECT_NOT_FOUND_MESSAGE,
        ),
        _ => (
            onetaskgraph_linear::graphql::DOCUMENT,
            "document",
            onetaskgraph_linear::graphql::DOCUMENT_NOT_FOUND_MESSAGE,
        ),
    };
    let read = graphql(runtime, key, query, json!({"id": id}));
    let not_found = read["errors"]
        .as_array()
        .is_some_and(|errors| errors.iter().any(|error| error["message"] == missing));
    let trashed = read["data"][root]["archivedAt"].is_string();
    evidence!("{kind} {id} reads back as {read}");
    not_found || trashed
}

/// A sandbox copying a folder `plan` — the project `slug`, titled `title`, with two tasks —
/// into the Linear source `dest`, reached through `endpoint`.
fn sandbox(
    key: &str,
    endpoint: &str,
    team: &str,
    mapping: &Value,
    slug: &str,
    title: &str,
) -> Sandbox {
    let sandbox = Sandbox::new();
    let root = plan_named(&sandbox, slug, title, &format!("{title} spare"), 2);
    sandbox.secrets_file(&format!("LINEAR_API_KEY={key}\n"));
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": root}},
        "dest": {"plugin": "linear",
                 "config": {"endpoint": endpoint, "team": team, "status_mapping": mapping}},
    })));
    sandbox
}

/// `arguments` through the binary until `done` holds of its JSON, or [`READS`] tries pass.
fn read_until(sandbox: &Sandbox, arguments: &[&str], done: impl Fn(&Value) -> bool) -> Value {
    let mut read = Value::Null;
    for attempt in 0..READS {
        let output = sandbox.command().args(arguments).output().unwrap();
        read = succeeded(arguments, &output);
        if done(&read) {
            return read;
        }
        if attempt + 1 < READS {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    read
}

fn tasks_of(sandbox: &Sandbox, project: &str, expected: &[String]) -> Vec<Value> {
    let arguments = [
        "--json",
        "task",
        "list",
        "--source",
        "dest",
        "--project",
        project,
    ];
    let listed = read_until(sandbox, &arguments, |listed| {
        listed["items"]
            .as_array()
            .is_some_and(|rows| rows.len() >= expected.len())
    });
    listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| row.get("item").unwrap_or(row).clone())
        .collect()
}

/// The journey, then its cleanup, however the journey ended.
///
/// `resolution` is what Linear answered the lane's [`onetaskgraph_linear::graphql::RESOLUTION`]
/// read with for `team`; `namespace` is the lane's guard, which every request the binary sends
/// passes.
pub(crate) fn live_project_create(
    runtime: &tokio::runtime::Runtime,
    key: &str,
    team: &str,
    resolution: &Value,
    namespace: Arc<Namespace>,
) {
    let team_id = resolution["data"]["teams"]["nodes"][0]["id"]
        .as_str()
        .expect("the nominated team")
        .to_owned();
    let state = resolution["data"]["teams"]["nodes"][0]["states"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|state| state["type"] == "unstarted")
        .and_then(|state| state["name"].as_str())
        .expect("an unstarted workflow state")
        .to_owned();
    let statuses = resolution["data"]["projectStatuses"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let project_status = ["planned", "backlog"]
        .iter()
        .find_map(|kind| {
            statuses
                .iter()
                .find(|status| status["type"] == *kind)
                .and_then(|status| status["name"].as_str())
        })
        .expect("a planned or backlog project status")
        .to_owned();
    let mapping = json!({"todo": {"task": state, "project": project_status}});
    let stamp = onetaskgraph_live::artifact::Stamp::new(
        onetaskgraph_live::artifact::Run::current(),
        onetaskgraph_live::artifact::now_micros(),
    )
    .to_string();
    let (endpoint, relayed) = relay(LINEAR.to_owned(), key.to_owned(), namespace);
    let journey = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drive(
            runtime, key, team, &team_id, &mapping, &stamp, &endpoint, &relayed,
        );
    }));
    evidence!("{}", seen(&relayed));
    let created = lock(&relayed).created.clone();
    let cleaned = cleanup(runtime, key, &team_id, &stamp, &created);
    if let Err(panic) = journey {
        std::panic::resume_unwind(panic);
    }
    assert!(cleaned, "the live project-create journey's cleanup failed");
    let mut left =
        stamped(runtime, key, &team_id, &stamp).unwrap_or_else(|problem| panic!("{problem}"));
    left.retain(|(kind, id, _)| !already_gone(runtime, key, kind, id));
    assert!(
        left.is_empty(),
        "the live project-create journey left items behind: {left:?}"
    );
}

/// Linear's refusal of an issue and a document created under an id one already holds, held to
/// the plugin's contract constants — the drift gate on the spelling both loopback fixtures answer
/// with, beside the project's in the undo scenario. The issue is one of this journey's own tasks;
/// the document is made for it and removed with everything else the journey made.
fn conflicts(
    runtime: &tokio::runtime::Runtime,
    key: &str,
    team_id: &str,
    stamp: &str,
    relayed: &Arc<Mutex<Relay>>,
) {
    use onetaskgraph_plugin_api::MetadataRecord;
    let prefix = onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX;
    let issue = lock(relayed)
        .created
        .iter()
        .find(|(kind, _)| *kind == "issue")
        .map(|(_, id)| id.clone())
        .expect("the copy created its tasks");
    let made = graphql(
        runtime,
        key,
        onetaskgraph_linear::graphql::DOCUMENT_CREATE,
        json!({"input": {"teamId": team_id, "title": format!("{prefix}{stamp} conflict notes"),
                         "content": "Notes held under an id a second create is refused."}}),
    );
    let document = made["data"]["documentCreate"]["document"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("Linear created the document: {made}"))
        .to_owned();
    lock(relayed).created.push(("document", document.clone()));
    for (record, query, input) in [
        (
            MetadataRecord::Task,
            onetaskgraph_linear::graphql::ISSUE_CREATE,
            json!({"teamId": team_id, "title": format!("{prefix}{stamp} conflict"), "id": issue}),
        ),
        (
            MetadataRecord::Document,
            onetaskgraph_linear::graphql::DOCUMENT_CREATE,
            json!({"teamId": team_id, "title": format!("{prefix}{stamp} conflict notes"),
                   "content": "A second create under a held id.", "id": document}),
        ),
    ] {
        let sent = input["id"].as_str().unwrap_or_default().to_owned();
        let answer = graphql(runtime, key, query, json!({ "input": input }));
        evidence!("Linear answered a {record:?} create under an id it holds with {answer}");
        let refusal = &answer["errors"][0];
        assert_eq!(
            refusal["message"],
            onetaskgraph_linear::graphql::id_conflict_message(record),
            "{answer}"
        );
        assert_eq!(
            refusal["extensions"]["userPresentableMessage"],
            onetaskgraph_linear::graphql::id_conflict_presentable(record, &sent),
            "{answer}"
        );
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the journey's identities, each one the caller already holds"
)]
fn drive(
    runtime: &tokio::runtime::Runtime,
    key: &str,
    team: &str,
    team_id: &str,
    mapping: &Value,
    stamp: &str,
    endpoint: &str,
    relayed: &Arc<Mutex<Relay>>,
) {
    let prefix = onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX;
    // (c) Reads by ids Linear does not hold, through the plugin itself.
    {
        use onetaskgraph_plugin_api::{SecretResolver, SourceName, SourcePlugin};
        struct Key(String);
        impl SecretResolver for Key {
            fn get(&self, _: &str) -> Option<secrecy::SecretString> {
                Some(self.0.clone().into())
            }
        }
        let source = onetaskgraph_linear::Plugin
            .build(
                &SourceName::new("dest").unwrap(),
                &json!({"endpoint": endpoint, "team": team, "status_mapping": mapping}),
                &Key(key.to_owned()),
            )
            .expect("a Linear source");
        runtime.block_on(async {
            let issue = absent_id(stamp, "issue").into();
            let project = absent_id(stamp, "project").into();
            let document = absent_id(stamp, "document").into();
            let task = source.get_task(&issue).await;
            assert!(
                matches!(task, Ok(None)),
                "an issue id Linear does not hold reads absent: {task:?}"
            );
            let held = source.get_project(&project).await;
            assert!(
                matches!(held, Ok(None)),
                "a project id Linear does not hold reads absent: {held:?}"
            );
            let held = source.get_document(&document).await;
            assert!(
                matches!(held, Ok(None)),
                "a document id Linear does not hold reads absent: {held:?}"
            );
        });
    }

    // (a) A project Linear does not hold yet, created with its two tasks.
    let title = format!("{prefix}{stamp} harbor");
    let harbor = slug(stamp, "harbor");
    let first = sandbox(key, endpoint, team, mapping, &harbor, &title);
    let source = format!("plan:{harbor}");
    let copy = ["--json", "project", "copy", source.as_str(), "--to", "dest"];
    let output = first.command().args(copy).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "`project copy` of a new project into Linear: {}{}\n{}",
        stdout(&output),
        stderr(&output),
        seen(relayed)
    );
    let report = succeeded(&copy, &output);
    let landed = outcome(&report, &source);
    assert_eq!(landed["action"], "created", "{report:#}");
    let project = landed["destination"].as_str().expect("where it landed");
    let show = ["--json", "project", "show", project];
    let shown = read_until(&first, &show, |shown| {
        shown["items"][0]["item"]["title"] == title.as_str()
    });
    assert_eq!(
        shown["items"][0]["item"]["title"],
        title.as_str(),
        "{shown:#}"
    );
    let expected = vec![task_title(&title, 0), task_title(&title, 1)];
    assert_eq!(
        titles(&tasks_of(&first, project, &expected)),
        expected,
        "each task is filed under the project it created"
    );
    conflicts(runtime, key, team_id, stamp, relayed);

    // (b) A create Linear applied whose answer never reached the binary — nor the answer to the
    // read the plugin makes at once for it — so the copy fails, and a second one has to find it.
    let title = format!("{prefix}{stamp} retry");
    let retry = slug(stamp, "retry");
    let second = sandbox(key, endpoint, team, mapping, &retry, &title);
    let source = format!("plan:{retry}");
    let copy = ["--json", "project", "copy", source.as_str(), "--to", "dest"];
    lock(relayed).lose_create = true;
    let output = second.command().args(copy).output().unwrap();
    let lost = lock(relayed)
        .created
        .iter()
        .filter(|(kind, _)| *kind == "project")
        .count();
    assert_ne!(
        output.status.code(),
        Some(0),
        "a copy whose create went unanswered does not succeed: {}",
        stdout(&output)
    );
    assert_eq!(
        lost,
        2,
        "the lost create reached Linear and Linear made the project: {}\n{}",
        stderr(&output),
        seen(relayed)
    );
    let disarmed = !lock(relayed).fail_project_read;
    assert!(
        disarmed,
        "the plugin read the project its lost create made: {}",
        seen(relayed)
    );
    let output = second.command().args(copy).output().unwrap();
    let report = succeeded(&copy, &output);
    let landed = outcome(&report, &source);
    let linked = landed["destination"].as_str().expect("where it landed");
    let made = lock(relayed)
        .created
        .iter()
        .filter(|(kind, _)| *kind == "project")
        .map(|(_, id)| format!("dest:{id}"))
        .collect::<Vec<_>>();
    assert_eq!(made.len(), 2, "the retry created no project: {made:?}");
    assert_eq!(
        linked, made[1],
        "the retry links the project the lost create made: {report:#}"
    );
    let show = ["--json", "project", "show", linked];
    let shown = read_until(&second, &show, |shown| {
        shown["items"][0]["item"]["title"] == title.as_str()
    });
    let create_key = shown["items"][0]["item"]["metadata"]
        [onetaskgraph_plugin_api::MetadataKey::CREATE_KEY]
        .as_str()
        .expect("the project carries the copy's create key")
        .to_owned();
    // The team holds exactly one project carrying that key, read from Linear itself.
    let carrying = graphql(
        runtime,
        key,
        "query($name:String!,$team:ID!){ projects(first:50,filter:{name:{eq:$name},accessibleTeams:{some:{id:{eq:$team}}}}){ nodes{id content} } }",
        json!({"name": title, "team": team_id}),
    );
    let nodes = carrying["data"]["projects"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node["content"]
                .as_str()
                .is_some_and(|content| content.contains(&create_key)))
            .count(),
        1,
        "one project carries the copy's create key: {carrying:#}"
    );
    assert_eq!(nodes.len(), 1, "one project for the copy: {carrying:#}");
    let expected = vec![task_title(&title, 0), task_title(&title, 1)];
    assert_eq!(
        titles(&tasks_of(&second, linked, &expected)),
        expected,
        "each task once, under the project the first attempt made"
    );
    let crane = format!("{title} crane");
    let issues = stamped(runtime, key, team_id, stamp)
        .unwrap_or_else(|problem| panic!("{problem}"))
        .into_iter()
        .filter(|(kind, _, name)| *kind == "issue" && name.starts_with(&crane))
        .count();
    assert_eq!(issues, 2, "each task is in the team once");

    // (d) A copy whose undo took back the project it created: the id its key derives is now one
    // a deleted project held, and every later copy of the plan sends that key. Linear's answer
    // to a create under that id is in the relay's log either way.
    let title = format!("{prefix}{stamp} undo");
    let undo = slug(stamp, "undo");
    let third = sandbox(key, endpoint, team, mapping, &undo, &title);
    let source = format!("plan:{undo}");
    let copy = ["--json", "project", "copy", source.as_str(), "--to", "dest"];
    let before = lock(relayed).created.len();
    lock(relayed).refuse_issue_creates = usize::from(onetaskgraph_linear::CREATE_ID_GENERATIONS);
    let output = third.command().args(copy).output().unwrap();
    let refused_left = lock(relayed).refuse_issue_creates;
    assert_ne!(
        output.status.code(),
        Some(0),
        "a copy refused its task does not succeed: {}\n{}",
        stdout(&output),
        seen(relayed)
    );
    assert_eq!(
        refused_left,
        0,
        "every id the task's key derives was tried: {}",
        seen(relayed)
    );
    let undone: Vec<String> = lock(relayed).created[before..]
        .iter()
        .filter(|(kind, _)| *kind == "project")
        .map(|(_, id)| id.clone())
        .collect();
    assert_eq!(
        undone.len(),
        1,
        "the copy created its project first: {}",
        seen(relayed)
    );
    let gone = graphql(
        runtime,
        key,
        onetaskgraph_linear::graphql::PROJECT,
        json!({"id": undone[0]}),
    );
    evidence!("the undone project reads back as {gone}");
    // Linear's listings can still answer a project for a moment after it is trashed — one run's
    // origin lookup did, and the re-copy then wrote to it and was refused `Entity is trashed` —
    // so the re-copy waits until Linear lists it no more, as a person copying again later would.
    let mut listed = true;
    for attempt in 0..READS {
        listed = stamped(runtime, key, team_id, stamp)
            .unwrap_or_else(|problem| panic!("{problem}"))
            .iter()
            .any(|(_, id, _)| *id == undone[0]);
        if !listed {
            break;
        }
        if attempt + 1 < READS {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    assert!(
        !listed,
        "Linear still lists the project the undo trashed after {READS} reads: {}",
        undone[0]
    );
    let output = third.command().args(copy).output().unwrap();
    let report = succeeded(&copy, &output);
    let landed = outcome(&report, &source);
    assert_eq!(landed["action"], "created", "{report:#}\n{}", seen(relayed));
    let linked = landed["destination"]
        .as_str()
        .expect("where it landed")
        .to_owned();
    let show = ["--json", "project", "show", linked.as_str()];
    let shown = read_until(&third, &show, |shown| {
        shown["items"][0]["item"]["title"] == title.as_str()
    });
    let create_key = shown["items"][0]["item"]["metadata"]
        [onetaskgraph_plugin_api::MetadataKey::CREATE_KEY]
        .as_str()
        .expect("the project carries the copy's create key")
        .to_owned();
    let generations = onetaskgraph_linear::linear_ids_of_key(&create_key).unwrap();
    assert_eq!(
        format!("dest:{}", generations[0]),
        format!("dest:{}", undone[0]),
        "the undone project was made under the key's first id"
    );
    let generation = generations
        .iter()
        .position(|id| linked == format!("dest:{id}"))
        .unwrap_or_else(|| panic!("{linked} is one of the key's ids: {generations:?}"));
    evidence!("the re-copy landed under generation {generation}");
    // Linear refused the re-copy's create under the deleted project's id in the words the plugin's
    // contract — and both loopback fixtures — spell: this is the drift gate on that spelling.
    let refusal = lock(relayed)
        .refusals
        .iter()
        .find(|(root, sent, _)| *root == "projectCreate" && *sent == undone[0])
        .map(|(_, _, error)| error.clone())
        .unwrap_or_else(|| {
            panic!(
                "Linear refused the create under the deleted project's id: {}",
                seen(relayed)
            )
        });
    assert_eq!(
        refusal["message"],
        onetaskgraph_linear::graphql::PROJECT_ID_CONFLICT_MESSAGE,
        "{refusal}"
    );
    assert_eq!(
        refusal["extensions"]["userPresentableMessage"],
        onetaskgraph_linear::graphql::id_conflict_presentable(
            onetaskgraph_plugin_api::MetadataRecord::Project,
            &undone[0],
        ),
        "{refusal}"
    );
    evidence!("Linear refused the create under the deleted project's id: {refusal}");
    let expected = vec![task_title(&title, 0), task_title(&title, 1)];
    assert_eq!(
        titles(&tasks_of(&third, &linked, &expected)),
        expected,
        "each task once, under the project the re-copy made"
    );
    let _ = qualified;
}

/// The relay this journey reads every create through, in front of the loopback Linear rather than
/// Linear, so `cargo test` holds it without a credential: a `project copy` sends its tasks'
/// creates as one aliased `issueCreate` document, and the relay records each of them — which it
/// once did not, so the credentialed journey found no task the copy had created — and refuses
/// that document as Linear refuses an alias of a batch, so the copy then sends the task's create
/// on its own under every id its key derives.
#[test]
fn the_relay_reads_and_refuses_each_create_of_an_aliased_batch() {
    let sandbox = Sandbox::new();
    let (config, _workspace) = linear_workspace(
        &sandbox,
        json!({"tasks": [], "projects": [], "documents": [], "labels": [],
               "task_dependencies": [], "project_dependencies": []}),
    );
    let upstream = config["endpoint"]
        .as_str()
        .expect("the loopback's endpoint")
        .to_owned();
    let namespace = Arc::new(Namespace::new(
        onetaskgraph_live::artifact::Run::current(),
        config["team"].as_str().expect("the loopback's team"),
    ));
    let (endpoint, relayed) = relay(upstream, "loopback-key".to_owned(), namespace);
    let mut relayed_config = config.clone();
    relayed_config["endpoint"] = json!(endpoint);
    let root = plan_named(&sandbox, "harbor", "Harbor", "Harbor spare", 2);
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": root}},
        "dest": {"plugin": "linear", "config": relayed_config},
    })));
    let copy = ["--json", "project", "copy", "plan:harbor", "--to", "dest"];

    // Refused: the batch, then the task's create alone under each id its key derives.
    lock(&relayed).refuse_issue_creates = usize::from(onetaskgraph_linear::CREATE_ID_GENERATIONS);
    let output = sandbox.command().args(copy).output().unwrap();
    assert_ne!(
        output.status.code(),
        Some(0),
        "a copy whose task is refused does not succeed: {}\n{}",
        stdout(&output),
        seen(&relayed)
    );
    assert_eq!(
        lock(&relayed).refuse_issue_creates,
        0,
        "every id the task's key derives was tried alone: {}",
        seen(&relayed)
    );
    assert!(
        lock(&relayed).sent.iter().any(|root| root == "a0:"),
        "the copy sent its creates as one aliased document: {}",
        seen(&relayed)
    );

    // Answered: every create of the batch is one the relay saw made.
    let before = lock(&relayed).created.len();
    let output = sandbox.command().args(copy).output().unwrap();
    let report = succeeded(&copy, &output);
    assert_eq!(
        outcome(&report, "plan:harbor")["action"],
        "created",
        "{report:#}"
    );
    let made: Vec<&'static str> = lock(&relayed).created[before..]
        .iter()
        .map(|(kind, _)| *kind)
        .collect();
    assert_eq!(
        made,
        ["project", "issue", "issue"],
        "the relay saw the project and both tasks of the batch created: {}",
        seen(&relayed)
    );
    assert!(
        lock(&relayed)
            .observations
            .iter()
            .any(|seen| seen.starts_with("a1: issueCreate:")),
        "{}",
        seen(&relayed)
    );
}
