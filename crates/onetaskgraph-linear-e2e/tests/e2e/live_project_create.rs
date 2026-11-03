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
use crate::fixtures::document;
use crate::linear_assets::document_lifecycle::namespace::Namespace;
use crate::project_create::{outcome, plan, qualified, succeeded, task_title, titles};

const LINEAR: &str = "https://api.linear.app/graphql";

/// Reads after a write that may still answer from before it, and their spacing.
const READS: u32 = 20;

/// What the relay is told to do, and what it saw.
#[derive(Default)]
struct Relay {
    /// Lose the answer to the next `projectCreate`, after Linear applied it.
    lose_create: bool,
    /// Fail the next read of one project by id, without sending it to Linear.
    fail_project_read: bool,
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

/// A relay in front of Linear, holding every request to this run's namespace, and the handle
/// that tells it to lose an answer.
fn relay(key: String, namespace: Arc<Namespace>) -> (String, Arc<Mutex<Relay>>) {
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
            if query == onetaskgraph_linear::graphql::PROJECT
                && std::mem::take(&mut held.lock().unwrap().fail_project_read)
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
                    .post(LINEAR)
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
            let mut relay = held.lock().unwrap();
            for (create, root, field, kind) in [
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
            ] {
                if query != create {
                    continue;
                }
                relay.observations.push(format!(
                    "{root}: HTTP {status}, errors={}",
                    answer.get("errors").cloned().unwrap_or(Value::Null)
                ));
                if let Some(id) = answer["data"][root][field]["id"].as_str() {
                    relay.created.push((kind, id.to_owned()));
                }
            }
            if query == onetaskgraph_linear::graphql::PROJECT_CREATE
                && std::mem::take(&mut relay.lose_create)
            {
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

/// Every project and issue of the team whose title carries `stamp`, with Linear's ids.
fn stamped(
    runtime: &tokio::runtime::Runtime,
    key: &str,
    team_id: &str,
    stamp: &str,
) -> Vec<(&'static str, String, String)> {
    let mut found = Vec::new();
    let projects = graphql(
        runtime,
        key,
        "query($stamp:String!,$team:ID!){ projects(first:50,filter:{name:{contains:$stamp},accessibleTeams:{some:{id:{eq:$team}}}}){ nodes{id name} } }",
        json!({"stamp": stamp, "team": team_id}),
    );
    for node in projects["data"]["projects"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
    {
        found.push((
            "project",
            node["id"].as_str().unwrap_or_default().to_owned(),
            node["name"].as_str().unwrap_or_default().to_owned(),
        ));
    }
    let issues = graphql(
        runtime,
        key,
        "query($stamp:String!,$team:ID!){ issues(first:50,filter:{title:{contains:$stamp},team:{id:{eq:$team}}}){ nodes{id title} } }",
        json!({"stamp": stamp, "team": team_id}),
    );
    for node in issues["data"]["issues"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
    {
        found.push((
            "issue",
            node["id"].as_str().unwrap_or_default().to_owned(),
            node["title"].as_str().unwrap_or_default().to_owned(),
        ));
    }
    if projects.get("errors").is_some() || issues.get("errors").is_some() {
        eprintln!(
            "live project create: listing stamped items was refused: {} {}",
            projects["errors"], issues["errors"]
        );
    }
    found
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
    for (kind, id, _) in stamped(runtime, key, team_id, stamp) {
        if !items.iter().any(|(_, held)| *held == id) {
            items.push((kind, id));
        }
    }
    // Issues before projects, so no issue outlives the project it was filed under.
    items.sort_by_key(|(kind, _)| *kind != "issue");
    let mut all = true;
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
        if answer["data"][root]["success"] != true {
            eprintln!("live project create: could not remove {kind} {id}: {answer}");
            all = false;
        }
    }
    all
}

/// A sandbox copying a folder `plan` — the project `harbor`, titled `title`, with two tasks —
/// into the Linear source `dest`, reached through `endpoint`.
fn sandbox(key: &str, endpoint: &str, team: &str, mapping: &Value, title: &str) -> Sandbox {
    let sandbox = Sandbox::new();
    let root = plan(&sandbox, title, &format!("{title} spare"), 2);
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
    let (endpoint, relayed) = relay(key.to_owned(), namespace);
    let journey = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drive(
            runtime, key, team, &team_id, &mapping, &stamp, &endpoint, &relayed,
        );
    }));
    for observation in &relayed.lock().unwrap().observations {
        eprintln!("live project create: {observation}");
    }
    let created = relayed.lock().unwrap().created.clone();
    let cleaned = cleanup(runtime, key, &team_id, &stamp, &created);
    if let Err(panic) = journey {
        std::panic::resume_unwind(panic);
    }
    assert!(cleaned, "the live project-create journey's cleanup failed");
    let left = stamped(runtime, key, &team_id, &stamp);
    assert!(
        left.is_empty(),
        "the live project-create journey left items behind: {left:?}"
    );
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
    let first = sandbox(key, endpoint, team, mapping, &title);
    let copy = ["--json", "project", "copy", "plan:harbor", "--to", "dest"];
    let output = first.command().args(copy).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "`project copy` of a new project into Linear: {}{}\n{:#?}",
        stdout(&output),
        stderr(&output),
        relayed.lock().unwrap().observations
    );
    let report = succeeded(&copy, &output);
    let landed = outcome(&report, "plan:harbor");
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

    // (b) A create Linear applied whose answer never reached the binary — nor the answer to the
    // read the plugin makes at once for it — so the copy fails, and a second one has to find it.
    let title = format!("{prefix}{stamp} retry");
    let second = sandbox(key, endpoint, team, mapping, &title);
    {
        let mut relay = relayed.lock().unwrap();
        relay.lose_create = true;
        relay.fail_project_read = true;
    }
    let output = second.command().args(copy).output().unwrap();
    let lost = relayed
        .lock()
        .unwrap()
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
        "the lost create reached Linear and Linear made the project: {}\n{:#?}",
        stderr(&output),
        relayed.lock().unwrap().observations
    );
    let output = second.command().args(copy).output().unwrap();
    let report = succeeded(&copy, &output);
    let landed = outcome(&report, "plan:harbor");
    let linked = landed["destination"].as_str().expect("where it landed");
    let made = relayed
        .lock()
        .unwrap()
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
        .into_iter()
        .filter(|(kind, _, name)| *kind == "issue" && name.starts_with(&crane))
        .count();
    assert_eq!(issues, 2, "each task is in the team once");
    let _ = qualified;
}
