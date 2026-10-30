//! Asset copies through the binary and Linear's documented HTTP boundaries.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use onetaskgraph_e2e_support::{clock::SimulatedClock, images};
use onetaskgraph_plugin_api::asset_sha256;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This deterministic quota model is reconciled with the sourced budget description by loopback_request_limit_and_budget_description_agree. The changing published quota is checked by an external audit rather than a cached measuring journey.
const REQUEST_LIMIT: usize = 2_500;
const REQUEST_WINDOW: Duration = Duration::from_secs(3_600);
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

use crate::fixtures::{LinearMarkdown, LinearWorkspace, document, linear_workspace};
#[path = "../../../onetaskgraph-linear/tests/document_lifecycle/mod.rs"]
pub(crate) mod document_lifecycle;
#[path = "../../../onetaskgraph-linear/tests/document_lifecycle/replay_schema.rs"]
mod document_replay_schema;
use document_lifecycle::Mode;
use onetaskgraph_linear::live_artifact::{Scope, Slot};

#[derive(Default)]
struct Upload {
    name: String,
    size: usize,
    bytes: Vec<u8>,
    verified: usize,
}

#[derive(Default)]
struct State {
    uploads: BTreeMap<usize, Upload>,
    document_journal: Option<document_lifecycle::journal::Journal>,
    refused_document_deletes: std::collections::BTreeSet<String>,
    document_delete_attempts: Vec<String>,
    ignore_asset_reset: bool,
    /// A document id, the content a lagging replica still holds for it, and how many more
    /// `document(id:)` reads that replica answers.
    stale_document: Option<(String, String, usize)>,
    requests: usize,
    tokens: f64,
    last_refill: Duration,
    fail: Option<(&'static str, usize, u16)>,
    fault: Option<&'static str>,
    hint: Option<&'static str>,
    network_time: Duration,
    refused_names: std::collections::BTreeSet<String>,
}

pub(crate) struct Workspace {
    pub(crate) endpoint: String,
    state: Arc<Mutex<State>>,
    pub(crate) held: LinearWorkspace,
    pub(crate) clock: SimulatedClock,
}

impl Workspace {
    fn new(sandbox: &Sandbox, clients: usize) -> (Value, Self) {
        let (mut config, held) = linear_workspace(
            sandbox,
            json!({
                "tasks": [], "projects": [], "documents": [], "labels": [],
                "task_dependencies": [], "project_dependencies": []
            }),
        );
        let backend = config["endpoint"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_owned();
        let clock = SimulatedClock::start(clients);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State {
            tokens: REQUEST_LIMIT as f64,
            ..State::default()
        }));
        let serving = state.clone();
        let timing = clock.clone();
        let address = endpoint.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = serving.clone();
                let clock = timing.clone();
                let backend = backend.clone();
                let address = address.clone();
                std::thread::spawn(move || serve(stream, &backend, &address, &state, &clock));
            }
        });
        config["endpoint"] = json!(format!("{endpoint}/0/graphql"));
        (
            config,
            Self {
                endpoint,
                state,
                held,
                clock,
            },
        )
    }

    /// How many uploads this storage was asked for, how many of them it was sent bytes for, and
    /// how many times anything read an upload back.
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        let state = self.state.lock().unwrap();
        (
            state.uploads.len(),
            state
                .uploads
                .values()
                .filter(|upload| !upload.bytes.is_empty())
                .count(),
            state.uploads.values().map(|upload| upload.verified).sum(),
        )
    }
}

fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = stream.read(&mut chunk).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 2_000_000, "bounded fixture request");
        if let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
            let size = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|size| size.parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= split + 4 + size {
                break;
            }
        }
    }
    bytes
}

fn answer(stream: &mut TcpStream, status: u16, body: &[u8]) {
    answer_hinted(stream, status, body, Some("normal"));
}

fn answer_hinted(stream: &mut TcpStream, status: u16, body: &[u8], hint: Option<&str>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let headers = match hint {
        Some("none" | "body") => String::new(),
        Some("zero") => "Retry-After: 0\r\n".to_owned(),
        Some("conflicting") => "Retry-After: 3\r\n".to_owned(),
        Some("shared") => "Retry-After: 31\r\n".to_owned(),
        Some("epoch") => format!(
            "X-RateLimit-Requests-Reset: {}\r\nX-RateLimit-Endpoint-Requests-Reset: {}\r\nX-RateLimit-Complexity-Reset: {}\r\n",
            now + 2000,
            now + 4000,
            now + 6000
        ),
        Some("expired") => "X-RateLimit-Requests-Reset: 1\r\n".to_owned(),
        Some("malformed") => {
            "X-RateLimit-Requests-Reset: nonsense\r\nRetry-After: 2\r\n".to_owned()
        }
        _ => "Retry-After: 2\r\n".to_owned(),
    };
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
}

fn complete_replay_document(row: &mut Value) {
    if !row.is_object() {
        return;
    }
    let recorded: Value = serde_json::from_str(include_str!(
        "../../../onetaskgraph-linear/tests/fixtures/documents.json"
    ))
    .unwrap();
    for field in ["createdAt", "updatedAt"] {
        if row[field].is_null() {
            row[field] = recorded["data"]["documents"]["nodes"][0][field].clone();
        }
    }
}

fn serve(
    mut stream: TcpStream,
    backend: &str,
    address: &str,
    state: &Mutex<State>,
    clock: &SimulatedClock,
) {
    let bytes = read_request(&mut stream);
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let head = String::from_utf8_lossy(&bytes[..split]);
    let path = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let client = parts[0].parse().unwrap();
    let request = clock.request(client);
    let stage = match parts[1] {
        "upload" => "PUT",
        "asset" => "verifying read",
        _ => "mutation",
    };
    let gql: Value = if parts[1] == "graphql" {
        serde_json::from_slice(&bytes[split..]).unwrap()
    } else {
        Value::Null
    };
    let asset_request = !gql.is_object() || gql["query"].as_str().unwrap().contains("fileUpload(");
    let mut quota_limited = false;
    let refusal = {
        let mut state = state.lock().unwrap();
        state.requests += 1;
        state.network_time += Duration::from_millis(if stage == "PUT" { 800 } else { 200 });
        if gql.is_object() {
            let now = clock.now();
            state.tokens = (state.tokens
                + now.saturating_sub(state.last_refill).as_secs_f64() * REQUEST_LIMIT as f64
                    / REQUEST_WINDOW.as_secs_f64())
            .min(REQUEST_LIMIT as f64);
            state.last_refill = now;
            if asset_request
                && stage == "mutation"
                && state.fault == Some("quota-at-upload")
                && state.refused_names.insert("quota".to_owned())
            {
                state.tokens = 0.0;
            }
            if state.tokens < 1.0 {
                quota_limited = true;
            } else {
                state.tokens -= 1.0;
            }
        }
        if quota_limited {
            Some(429)
        } else if asset_request && matches!(state.fault, Some("shared-stages" | "shared-images")) {
            let key = if state.fault == Some("shared-stages") {
                stage.to_owned()
            } else {
                gql["variables"]["filename"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            };
            let throttled = if state.fault == Some("shared-stages") {
                stage != "verifying read"
            } else {
                stage == "mutation"
            };
            if throttled && state.refused_names.insert(key) {
                Some(429)
            } else {
                None
            }
        } else {
            state.fail.as_mut().and_then(|(failed, remaining, status)| {
                if asset_request && *failed == stage && *remaining > 0 {
                    *remaining -= 1;
                    Some(*status)
                } else {
                    None
                }
            })
        }
    };
    let (fault, hint) = {
        let state = state.lock().unwrap();
        (state.fault, state.hint)
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(request.delay(Duration::from_millis(if stage == "PUT" {
        800
    } else {
        200
    })));
    if asset_request {
        if matches!(
            (fault, stage),
            (Some("redirect-mutation"), "mutation")
                | (Some("redirect-PUT"), "PUT")
                | (Some("redirect-read"), "verifying read")
        ) {
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://example.invalid/credential-target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            return;
        }
        if matches!(
            (fault, stage),
            (Some("disconnect-mutation"), "mutation")
                | (Some("disconnect-PUT"), "PUT")
                | (Some("disconnect-read"), "verifying read")
        ) {
            return;
        }
        if stage == "mutation" {
            let bad = match fault {
                Some("invalid-json") => Some("not JSON"),
                Some("graphql-errors") => Some(r#"{"errors":[{"message":"upload forbidden"}]}"#),
                Some("success-false") => {
                    Some(r#"{"data":{"fileUpload":{"success":false,"uploadFile":null}}}"#)
                }
                Some("malformed-upload") => Some(
                    r#"{"data":{"fileUpload":{"success":true,"uploadFile":{"assetUrl":123}}}}"#,
                ),
                _ => None,
            };
            if let Some(body) = bad {
                answer(&mut stream, 200, body.as_bytes());
                return;
            }
        }
    }
    if let Some(status) = refusal {
        let body = if stage == "mutation" && fault != Some("mutation-http429") {
            json!({"errors":[{"message":"asset refused","extensions":{"code":if status == 429 {"RATELIMITED"} else {"FORBIDDEN"},"retryAfter":2}}]}).to_string()
        } else {
            String::new()
        };
        let mut body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if hint == Some("none") {
            body["errors"][0]["extensions"]
                .as_object_mut()
                .map(|entry| entry.remove("retryAfter"));
        }
        let body = if body.is_null() {
            String::new()
        } else {
            body.to_string()
        };
        answer_hinted(
            &mut stream,
            if stage == "mutation" && status == 429 && fault != Some("mutation-http429") {
                400
            } else {
                status
            },
            body.as_bytes(),
            hint,
        );
        return;
    }
    if !asset_request {
        use onetaskgraph_linear::graphql;
        let mut forwarded = gql.clone();
        let query = gql["query"].as_str().unwrap();
        // Linear refuses a document over its complexity cap with HTTP 400 before reading a
        // row, so this stand-in does too for the lane's own standing-set listings, the documents
        // that model is reconciled for. It does not score the binary's documents: Linear prices
        // `issues` by the rows it returns (it accepts that listing at 250, `MAX_PAGE_SIZE`),
        // which the structural model over-prices — `ISSUES_CACHED` at 100 rows scores 16355 —
        // and those are priced by `cache_budget::price` from Linear's own scores instead.
        if [document_lifecycle::DOCUMENTS, document_lifecycle::PROJECTS].contains(&query)
            && let Some(refused) =
                document_replay_schema::complexity::refusal(query, &gql["variables"])
        {
            answer(
                &mut stream,
                400,
                json!({"errors":[{"message":refused}]})
                    .to_string()
                    .as_bytes(),
            );
            return;
        }
        if query.contains("documentUpdate(")
            && gql["variables"]["input"]["content"]
                .as_str()
                .is_some_and(|content| content.starts_with("Reset for invocation "))
            && state.lock().unwrap().ignore_asset_reset
        {
            answer(
                &mut stream,
                200,
                json!({"data":{"documentUpdate":{"success":true}}})
                    .to_string()
                    .as_bytes(),
            );
            return;
        }
        if query.contains("documentDelete(") {
            let id = gql["variables"]["id"].as_str().unwrap();
            let refused = {
                let mut state = state.lock().unwrap();
                state.document_delete_attempts.push(id.to_owned());
                state.refused_document_deletes.contains(id)
            };
            if refused {
                answer(
                    &mut stream,
                    200,
                    json!({"data":{"documentDelete":{"success":false}}})
                        .to_string()
                        .as_bytes(),
                );
                return;
            }
        }
        let discovery = query.contains("documents(") && query.contains("team{key}");
        if discovery {
            forwarded["query"] = json!(graphql::DOCUMENTS);
            forwarded["variables"]["first"] = json!(100);
        } else if query.contains("documentCreate(") {
            forwarded["query"] = json!(graphql::DOCUMENT_CREATE);
        } else if query.contains("documentUpdate(") {
            forwarded["query"] = json!(graphql::DOCUMENT_UPDATE);
        } else if query.contains("documentDelete(") {
            forwarded["query"] = json!(graphql::DOCUMENT_DELETE);
        }
        let body = forwarded.to_string();
        let mut upstream = TcpStream::connect(backend).unwrap();
        write!(upstream,"POST /graphql HTTP/1.1\r\nHost: {backend}\r\nAuthorization: fixture-key\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        let mut response = Vec::new();
        upstream.read_to_end(&mut response).unwrap();
        let split = response.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
        let mut response: Value = serde_json::from_slice(&response[split..]).unwrap();
        if discovery
            && let Some(nodes) = response
                .pointer_mut("/data/documents/nodes")
                .and_then(Value::as_array_mut)
        {
            for node in nodes {
                node["team"] = json!({"key":"ENG"});
                if node["project"].is_object() {
                    node["project"]["teams"] =
                        json!({"nodes":[{"key":"ENG"}],"pageInfo":{"hasNextPage":false}});
                }
            }
        }
        if let Some(row) = response.pointer_mut("/data/document")
            && let Some((id, content, reads)) = &mut state.lock().unwrap().stale_document
            && row["id"].as_str() == Some(id.as_str())
            && *reads > 0
        {
            *reads -= 1;
            row["content"] = json!(content);
        }
        if let Some(journal) = &state.lock().unwrap().document_journal {
            // Record acknowledgement before validating its shape, including uncertain IDs.
            journal.observe(&gql, &response);
            for pointer in [
                "/data/document",
                "/data/documentCreate/document",
                "/data/documentUpdate/document",
            ] {
                if let Some(row) = response.pointer_mut(pointer) {
                    complete_replay_document(row);
                }
            }
            if let Some(nodes) = response
                .pointer_mut("/data/documents/nodes")
                .and_then(Value::as_array_mut)
            {
                for row in nodes {
                    complete_replay_document(row);
                }
            }
            document_replay_schema::validate(query, &response)
                .expect("recorded Linear response shape");
        }
        answer(&mut stream, 200, response.to_string().as_bytes());
        return;
    }
    let response = {
        let mut state = state.lock().unwrap();
        match stage {
            "mutation" => {
                let id = state.uploads.len();
                state.uploads.insert(
                    id,
                    Upload {
                        name: gql["variables"]["filename"].as_str().unwrap().to_owned(),
                        size: gql["variables"]["size"].as_u64().unwrap() as usize,
                        ..Upload::default()
                    },
                );
                assert_eq!(gql["variables"]["contentType"], "image/png");
                json!({"data":{"fileUpload":{"success":true,"uploadFile":{"uploadUrl":format!("{address}/{client}/upload/{id}"),"assetUrl":format!("{address}/{client}/asset/{id}"),"headers":[{"key":"x-upload-signature","value":"signed"}]}}}}).to_string().into_bytes()
            }
            "PUT" => {
                assert!(head.contains("x-upload-signature: signed"));
                assert!(head.contains("content-type: image/png"));
                assert!(head.contains("cache-control: public, max-age=31536000"));
                assert!(!head.to_ascii_lowercase().contains("authorization:"));
                let upload = state.uploads.get_mut(&parts[2].parse().unwrap()).unwrap();
                assert_eq!(bytes.len() - split, upload.size);
                upload.bytes = bytes[split..].to_vec();
                Vec::new()
            }
            _ => {
                assert!(head.contains("authorization: fixture-key"));
                let upload = state.uploads.get_mut(&parts[2].parse().unwrap()).unwrap();
                assert_eq!(upload.bytes.len(), upload.size);
                upload.verified += 1;
                upload.bytes.clone()
            }
        }
    };
    let mut response = response;
    if state.lock().unwrap().document_journal.is_some() && stage == "mutation" {
        document_replay_schema::validate(
            gql["query"].as_str().unwrap(),
            &serde_json::from_slice(&response).unwrap(),
        )
        .expect("recorded fileUpload response shape");
    }
    if stage == "mutation" {
        let mut body: Value = serde_json::from_slice(&response).unwrap();
        let file = &mut body["data"]["fileUpload"]["uploadFile"];
        match fault {
            Some("untrusted-asset") => {
                file["assetUrl"] = json!("https://example.invalid/credential-target.png")
            }
            Some("wrong-loopback") => {
                file["assetUrl"] = json!("http://127.0.0.1:9/credential-target.png")
            }
            Some("wrong-upload-loopback") => {
                file["uploadUrl"] = json!("http://127.0.0.1:9/upload.png")
            }
            Some("credentials-upload") => {
                file["uploadUrl"] = json!("https://user:secret@example.invalid/upload.png")
            }
            Some("fragment-asset") => {
                file["assetUrl"] = json!("https://uploads.linear.app/image#fragment")
            }
            Some("bad-upload-scheme") => file["uploadUrl"] = json!("file:///upload.png"),
            Some("bad-header-name") => file["headers"][0]["key"] = json!("bad header"),
            Some("bad-header-value") => {
                file["headers"][0]["value"] = json!("value\r\nInjected: true")
            }
            _ => {}
        }
        response = body.to_string().into_bytes();
    }
    answer(&mut stream, 200, &response);
}

pub(crate) struct Journey {
    config: Value,
    pub(crate) sandbox: Sandbox,
    pub(crate) workspace: Workspace,
}

impl Journey {
    pub(crate) fn new() -> Self {
        Self::with_clients(1)
    }

    fn with_clients(clients: usize) -> Self {
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("notes");
        std::fs::create_dir_all(root.join("projects")).unwrap();
        std::fs::write(
            root.join("projects/launch.md"),
            "---\ntitle: Launch\nstatus: todo\n---\nLaunch.\n",
        )
        .unwrap();
        let (config, workspace) = Workspace::new(&sandbox, clients);
        let mut sources = json!({
            "notes":{"plugin":"local-md","config":{"root":root}},
            "dest":{"plugin":"linear","config":config}
        });
        for client in 1..clients {
            let mut config = config.clone();
            config["endpoint"] = json!(format!("{}/{client}/graphql", workspace.endpoint));
            sources[format!("dest{client}")] = json!({"plugin":"linear","config":config});
        }
        sandbox.project_document(&document(&sources));
        Self {
            sandbox,
            workspace,
            config,
        }
    }

    fn create(&self, kind: &str, count: usize) -> (String, Vec<Vec<u8>>) {
        self.create_named(kind, count, kind, false)
    }

    fn create_named(
        &self,
        kind: &str,
        count: usize,
        title: &str,
        large: bool,
    ) -> (String, Vec<Vec<u8>>) {
        let inputs = self.sandbox.subdirectory(&format!("{title}-inputs"));
        let mut body = String::new();
        let mut paths = Vec::new();
        let images: Vec<_> = (0..count)
            .map(|index| {
                images::png(
                    index as u64 + title.bytes().map(u64::from).sum::<u64>() * 100,
                    if large {
                        475_000
                    } else {
                        50_500 + index * 424_500 / count.saturating_sub(1).max(1)
                    },
                )
            })
            .collect();
        for (index, bytes) in images.iter().enumerate() {
            let name = format!("{kind}-{index}.png");
            body.push_str(&format!("![alt {index}](./{name})\n"));
            let path = inputs.join(name);
            std::fs::write(&path, bytes).unwrap();
            paths.push(path.to_string_lossy().into_owned());
        }
        let body_path = inputs.join("body.md");
        std::fs::write(&body_path, body).unwrap();
        let mut args = vec![
            kind,
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            title,
            "--body-file",
            body_path.to_str().unwrap(),
        ];
        for path in &paths {
            args.extend(["--asset", path]);
        }
        let output = self.sandbox.command().args(&args).output().unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        (stdout(&output).trim().to_owned(), images)
    }

    fn copy(&self, kind: &str, id: &str, succeeds: bool) -> Value {
        let output = self
            .sandbox
            .command()
            .envs(self.workspace.clock.client_env(0))
            .args(["--json", kind, "copy", id, "--to", "dest"])
            .output()
            .unwrap();
        assert_eq!(output.status.success(), succeeds, "{}", stderr(&output));
        if succeeds {
            serde_json::from_str(&stdout(&output)).unwrap()
        } else {
            json!(stderr(&output))
        }
    }
}

#[test]
fn issue_and_document_assets_upload_verify_rewrite_and_reuse() {
    let journey = Journey::new();
    let mut sizes = Vec::new();
    let mut new_requests = 0;
    let mut unchanged_requests = 0;
    for (kind, count) in [("task", 2), ("document", 6)] {
        let (id, images) = journey.create(kind, count);
        sizes.extend(images.iter().map(Vec::len));
        let prior = journey.workspace.counts();
        let report = journey.copy(kind, &id, true);
        let dest = report["items"][0]["destination"]
            .as_str()
            .unwrap()
            .split_once(':')
            .unwrap()
            .1;
        let raw = journey
            .workspace
            .held
            .long_form(if kind == "task" { "tasks" } else { "documents" }, dest)
            .unwrap();
        assert!(!raw.contains("](./"));
        assert!(raw.contains("onetaskgraph.metadata"));
        let state = journey.workspace.state.lock().unwrap();
        for (index, image) in images.iter().enumerate() {
            let upload = state
                .uploads
                .values()
                .find(|upload| upload.name == format!("{kind}-{index}.png"))
                .unwrap();
            assert_eq!(&upload.bytes, image);
            // The PUT's own answer says the bytes are held: nothing reads the asset back.
            assert_eq!(upload.verified, 0);
            assert!(raw.contains(&asset_sha256(image)));
            assert!(raw.contains(&format!("![alt {index}]({}", journey.workspace.endpoint)));
        }
        drop(state);
        let before = journey.workspace.counts();
        new_requests += before.0 + before.1 + before.2 - prior.0 - prior.1 - prior.2;
        for _ in 0..3 {
            journey.copy(kind, &id, true);
        }
        let after = journey.workspace.counts();
        unchanged_requests += after.0 + after.1 + after.2 - before.0 - before.1 - before.2;
        assert_eq!(after, before);
        let output = journey
            .sandbox
            .command()
            .args(["--json", kind, "show", &id])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
        let changed = images::png(999 + count as u64, 475_000);
        std::fs::write(shown["assets"][0]["path"].as_str().unwrap(), &changed).unwrap();
        journey.copy(kind, &id, true);
        assert_eq!(
            journey.workspace.counts(),
            (before.0 + 1, before.1 + 1, before.2)
        );
        let raw = journey
            .workspace
            .held
            .long_form(if kind == "task" { "tasks" } else { "documents" }, dest)
            .unwrap();
        assert!(raw.contains(&asset_sha256(&changed)));
        assert!(raw.contains(&format!("/asset/{}", before.0)));
    }
    record_workload(
        "linear-asset-requests-per-new-asset",
        new_requests as f64 / sizes.len() as f64,
        &sizes,
        1,
    );
    record_workload(
        "linear-asset-requests-per-unchanged-asset",
        unchanged_requests as f64 / (sizes.len() * 3) as f64,
        &sizes,
        3,
    );
}

#[test]
fn asset_stage_failures_and_bounded_virtual_rate_limit_recovery() {
    for kind in ["task", "document"] {
        for stage in ["mutation", "PUT"] {
            for (times, status, succeeds) in [(1, 403, false), (1, 429, true), (31, 429, false)] {
                let journey = Journey::new();
                let (id, images) = journey.create(kind, 1);
                journey.workspace.state.lock().unwrap().fail = Some((stage, times, status));
                let result = journey.copy(kind, &id, succeeds);
                if succeeds {
                    assert_eq!(journey.workspace.counts(), (1, 1, 0));
                    assert!(journey.workspace.clock.now() >= Duration::from_secs(2));
                    let destination = result["items"][0]["destination"]
                        .as_str()
                        .unwrap()
                        .split_once(':')
                        .unwrap()
                        .1;
                    let raw = journey
                        .workspace
                        .held
                        .long_form(
                            if kind == "task" { "tasks" } else { "documents" },
                            destination,
                        )
                        .unwrap();
                    assert!(!raw.contains("](./") && raw.contains("![alt 0](http://"));
                    assert!(
                        raw.contains("onetaskgraph.metadata")
                            && raw.contains("onetaskgraph.assets")
                    );
                    assert!(raw.contains(&asset_sha256(&images[0])));
                    assert!(raw.contains(&format!("{}/0/asset/0", journey.workspace.endpoint)));
                } else {
                    let said = result.as_str().unwrap();
                    assert!(
                        said.contains(stage) && said.contains(&format!("{kind}-0.png")),
                        "{said}"
                    );
                    assert!(
                        said.contains(
                            &if stage == "mutation" && status == 429 {
                                400
                            } else {
                                status
                            }
                            .to_string()
                        ),
                        "{said}"
                    );
                    if status == 429 {
                        assert!(said.contains("limiter"));
                    }
                    assert!(!journey.workspace.held.served().iter().any(|(query, _)| {
                        query.contains(if kind == "task" {
                            "issueCreate("
                        } else {
                            "documentCreate("
                        })
                    }));
                }
            }
        }
    }
}

const COUNTED: &str = "counted at the Linear loopback in simulated time";

fn record_workload(budget: &str, value: f64, sizes: &[usize], copies: usize) {
    crate::telemetry::record_assets(budget, value, COUNTED, sizes, copies);
}

/// The concurrent copies' figure, with what each refused copy said. A passing journey's output
/// is captured and never shown, while `onebudgetspec check` prints the detail beside the
/// figure, so this is where a refusal on a runner nobody can reproduce on names its cause.
fn concurrent_refusals_detail(refusals: &[String]) -> String {
    if refusals.is_empty() {
        return COUNTED.to_owned();
    }
    format!("{COUNTED}; refused: {}", refusals.join("; "))
}

fn project_workload(journey: &Journey) -> (Vec<usize>, Vec<String>) {
    for index in 0..7 {
        journey.create_named("task", 0, &format!("Issue {index}"), true);
    }
    let mut sizes = Vec::new();
    let mut documents = Vec::new();
    for index in 0..3 {
        let (id, images) = journey.create_named(
            "document",
            if index < 2 { 12 } else { 0 },
            &format!("Document {index}"),
            true,
        );
        sizes.extend(images.iter().map(Vec::len));
        documents.push(id);
    }
    (sizes, documents)
}

#[test]
fn measure_project_asset_copy_simulated_seconds() {
    let journey = Journey::new();
    let (sizes, documents) = project_workload(&journey);
    let start = journey.workspace.clock.now();
    journey.copy("project", "notes:launch", true);
    for id in documents {
        journey.copy("document", &id, true);
    }
    assert_eq!(journey.workspace.counts(), (24, 24, 0));
    record_workload(
        "linear-asset-copy-seconds",
        journey
            .workspace
            .clock
            .now()
            .saturating_sub(start)
            .as_secs_f64(),
        &sizes,
        1,
    );
}

/// What three copies of the project workload made at once said: how many clients had a copy
/// refused, the figure's detail naming each refusal, the workload's image sizes, and the real
/// time it took.
struct ConcurrentCopies {
    refused: f64,
    detail: String,
    sizes: Vec<usize>,
    elapsed: Duration,
}

/// Copy the project workload — the project, then each document — to each of `destinations` at
/// once, the binary at index `n` running as clock client `n`.
fn copy_plan_concurrently(journey: &Journey, destinations: [&str; 3]) -> ConcurrentCopies {
    let (sizes, documents) = project_workload(journey);
    let real_start = std::time::Instant::now();
    let mut refused_clients = [false; 3];
    let mut refusals = Vec::new();
    for (kind, id) in std::iter::once(("project", "notes:launch"))
        .chain(documents.iter().map(|id| ("document", id.as_str())))
    {
        let mut children = Vec::new();
        for (client, destination) in destinations.iter().enumerate() {
            children.push(
                journey
                    .sandbox
                    .subprocess(onetaskgraph_e2e_support::binary())
                    .current_dir(journey.sandbox.project())
                    .envs(journey.workspace.clock.client_env(client))
                    .args(["--json", kind, "copy", id, "--to", destination])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
        }
        for (client, child) in children.into_iter().enumerate() {
            let output = child.wait_with_output().unwrap();
            if !output.status.success() {
                refused_clients[client] = true;
                let said = stderr(&output);
                eprintln!("{said}");
                refusals.push(format!(
                    "client {client}'s {kind} copy of {id} failed ({}): {}",
                    output.status,
                    said.trim()
                ));
            }
        }
    }
    ConcurrentCopies {
        refused: refused_clients.iter().filter(|refused| **refused).count() as f64,
        detail: concurrent_refusals_detail(&refusals),
        sizes: sizes.iter().copied().cycle().take(72).collect(),
        elapsed: real_start.elapsed(),
    }
}

#[test]
fn measure_three_concurrent_project_asset_copies() {
    let journey = Journey::with_clients(3);
    let copies = copy_plan_concurrently(&journey, ["dest", "dest1", "dest2"]);
    let elapsed = copies.elapsed;
    eprintln!(
        "three concurrent copies real duration: {:.3}s",
        elapsed.as_secs_f64()
    );
    assert!(elapsed < Duration::from_secs(60));
    // Recorded before the copies' items are counted, so a refusal that also loses one still
    // leaves what it said where the budget check prints it.
    crate::telemetry::record_assets(
        "linear-concurrent-copies-refused",
        copies.refused,
        &copies.detail,
        &copies.sizes,
        3,
    );
    assert_eq!(journey.workspace.counts(), (72, 72, 0));
}

/// A refused copy's own words reach what `onebudgetspec check` reports for the figure: the
/// workload copied to a destination nothing configures, recorded as the measuring journey
/// records it and read back through the budget's own entry.
#[test]
fn a_refused_concurrent_copy_is_named_in_what_the_budget_check_reports() {
    let journey = Journey::with_clients(3);
    let copies = copy_plan_concurrently(&journey, ["dest", "dest1", "unconfigured"]);
    let budget = "linear-concurrent-copies-refused";
    let telemetry = journey.sandbox.subdirectory("telemetry");
    crate::telemetry::record_assets_in(
        &telemetry,
        budget,
        copies.refused,
        &copies.detail,
        &copies.sizes,
        3,
    );
    let budgets: Value = serde_saphyr::from_str(include_str!("../../budgets.yaml")).unwrap();
    let mut entry = budgets["budgets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == budget)
        .unwrap()
        .clone();
    entry["command"] = json!(crate::budget_runner::runner());
    let file = journey.sandbox.subdirectory("budgets").join("budgets.yaml");
    std::fs::write(
        &file,
        json!({"schema_version": 1, "budgets": [entry]}).to_string(),
    )
    .unwrap();
    let output = std::process::Command::new("bun")
        .args(["run", "onebudgetspec", "check", "--json"])
        .arg(&file)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .env(crate::telemetry::DIRECTORY, &telemetry)
        .output()
        .expect(
            "bun runs the workspace's pinned onebudgetspec; run `just bootstrap` to install it",
        );
    let report: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("a JSON report: {}", stderr(&output)));
    let result = &report["results"][0];
    assert_eq!(result["verdict"], "over", "{result:#}");
    assert_eq!(result["actual"], 1.0, "{result:#}");
    let detail = result["detail"].as_str().expect("a detail");
    assert!(
        detail.starts_with(&format!(
            "{COUNTED}; refused: client 2's project copy of notes:launch failed ("
        )),
        "{detail}"
    );
    assert!(
        detail.contains(r#"no source named "unconfigured" is configured"#),
        "{detail}"
    );
    assert!(
        !detail.contains("client 0") && !detail.contains("client 1"),
        "{detail}"
    );
}

#[test]
fn loopback_request_limit_and_budget_description_agree() {
    let budgets: Value = serde_saphyr::from_str(include_str!("../../budgets.yaml")).unwrap();
    let description = budgets["budgets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|budget| budget["id"] == "linear-concurrent-copies-refused")
        .unwrap()["description"]
        .as_str()
        .unwrap();
    assert!(description.contains(&format!("{REQUEST_LIMIT} requests")));
    assert!(description.contains(&format!("{} seconds", REQUEST_WINDOW.as_secs())));
}

#[test]
fn project_home_routed_to_a_member_source_carries_issue_and_document_assets() {
    let journey = Journey::new();
    let (issue, _) = journey.create_named("task", 1, "Routed issue", true);
    let (doc, _) = journey.create_named("document", 1, "Routed document", true);
    let root = journey.sandbox.project().join("notes");
    for path in [
        root.join("projects/launch.md"),
        root.join("tasks/routed-issue.md"),
        root.join("documents/routed-document.md"),
    ] {
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replacen(
                "---\n",
                "---\nrepositories: [github.com/example-org/app]\n",
                1,
            ),
        )
        .unwrap();
    }
    journey.sandbox.project_document(&document(&json!({
        "notes":{"plugin":"local-md","config":{"root":root}},
        "dest":{"plugin":"linear","config":journey.config,"routes":[{"repositories":["github.com/example-org/*"],"to":"member"}]},
        "member":{"plugin":"linear","config":journey.config}
    })));
    let project = journey.copy("project", "notes:launch", true);
    let home = project["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["source"] == "notes:launch")
        .unwrap()["destination"]
        .as_str()
        .unwrap();
    assert!(home.starts_with("member:"), "{project}");
    let copied = journey.copy("document", &doc, true);
    assert_eq!(journey.workspace.counts(), (2, 2, 0));
    for (kind, source, report) in [
        ("tasks", issue.as_str(), &project),
        ("documents", doc.as_str(), &copied),
    ] {
        let destination = report["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["source"] == source)
            .unwrap()["destination"]
            .as_str()
            .unwrap();
        assert!(destination.starts_with("member:"));
        let native = destination.split_once(':').unwrap().1;
        let raw = journey.workspace.held.long_form(kind, native).unwrap();
        assert!(!raw.contains("](./"));
        assert!(raw.contains("![alt 0](http://"));
        assert!(raw.contains("onetaskgraph.metadata") && raw.contains("onetaskgraph.assets"));
        let state = journey.workspace.state.lock().unwrap();
        let upload = state
            .uploads
            .values()
            .find(|upload| {
                upload.name
                    == format!(
                        "{}-0.png",
                        if kind == "tasks" { "task" } else { "document" }
                    )
            })
            .unwrap();
        assert!((450_000..=500_000).contains(&upload.bytes.len()));
        assert_eq!(upload.bytes.len(), upload.size);
        assert_eq!(upload.verified, 0);
        assert!(raw.contains(&asset_sha256(&upload.bytes)));
        drop(state);
        assert_eq!(
            if kind == "tasks" {
                journey.workspace.held.project_of(native)
            } else {
                journey.workspace.held.document_project(native)
            },
            Some(home.split_once(':').unwrap().1.to_owned())
        );
    }
}

#[derive(Default)]
struct LiveUploads {
    signed: BTreeMap<usize, String>,
    uploaded: usize,
    items: Vec<(&'static str, String)>,
    observations: Vec<String>,
}

/// A forwarding observer changes only the signed PUT URL, keeping Linear's asset URL intact, so
/// the journey's own authenticated GET of each asset reaches Linear directly.
fn live_observer(
    key: String,
    journal: document_lifecycle::journal::Journal,
    upstream: String,
) -> (String, Arc<Mutex<LiveUploads>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(LiveUploads::default()));
    let recording = state.clone();
    let address = endpoint.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let client = reqwest::Client::new();
        for mut stream in listener.incoming().flatten() {
            let bytes = read_request(&mut stream);
            let split = bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            let head = String::from_utf8_lossy(&bytes[..split]);
            let path = head
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            if let Some(id) = path.strip_prefix("/upload/") {
                let id: usize = id.parse().unwrap();
                let url = recording.lock().unwrap().signed[&id].clone();
                let url = reqwest::Url::parse(&url).unwrap();
                assert_eq!(url.scheme(), "https");
                assert!(
                    url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none()
                );
                let mut request = client.put(url);
                for line in head.lines().skip(1) {
                    if let Some((name, value)) = line.split_once(':')
                        && !["host", "connection", "content-length"]
                            .contains(&name.to_ascii_lowercase().as_str())
                    {
                        request = request.header(name.trim(), value.trim());
                    }
                }
                let response = runtime
                    .block_on(request.body(bytes[split..].to_vec()).send())
                    .unwrap();
                let status = response.status().as_u16();
                let mut state = recording.lock().unwrap();
                state.uploaded += 1;
                state.observations.push(format!(
                    "asset {id} PUT: HTTP {status}, {} bytes",
                    bytes.len() - split
                ));
                answer(&mut stream, status, &[]);
            } else {
                let request: Value = serde_json::from_slice(&bytes[split..]).unwrap();
                let response = runtime
                    .block_on(
                        client
                            .post(&upstream)
                            .header("Authorization", &key)
                            .json(&request)
                            .send(),
                    )
                    .unwrap();
                let status = response.status().as_u16();
                let mut body: Value = runtime.block_on(response.json()).unwrap();
                journal.observe(&request, &body);
                let mut state = recording.lock().unwrap();
                if request["query"].as_str().unwrap().contains("fileUpload(") {
                    let id = state.signed.len();
                    let filename = request["variables"]["filename"].as_str().unwrap();
                    state.observations.push(format!(
                        "{filename} fileUpload: HTTP {status}, success={}, assetUrl={}",
                        body["data"]["fileUpload"]["success"],
                        body["data"]["fileUpload"]["uploadFile"]["assetUrl"]
                    ));
                    if let Some(url) =
                        body["data"]["fileUpload"]["uploadFile"]["uploadUrl"].as_str()
                    {
                        state.signed.insert(id, url.to_owned());
                        body["data"]["fileUpload"]["uploadFile"]["uploadUrl"] =
                            json!(format!("{address}/upload/{id}"));
                    }
                }
                for (root, field, kind) in [
                    ("issueCreate", "issue", "issue"),
                    ("documentCreate", "document", "document"),
                ] {
                    if let Some(id) = body["data"][root][field]["id"].as_str() {
                        state.items.push((kind, id.to_owned()));
                    }
                }
                answer(&mut stream, status, body.to_string().as_bytes());
            }
        }
    });
    (format!("{endpoint}/graphql"), state)
}

/// The line [`asset_origins`] ends each origin's content with, naming the invocation that made it.
const INVOCATION_LINE: &str = "Invocation ";

/// Reads after a copy that may still answer the destination as it stood before, and their spacing.
const SHOW_READS: u32 = 21;

/// Each origin of one invocation, its content ending in an [`INVOCATION_LINE`] no other
/// invocation shares.
///
/// The standing asset slot keeps one title across runs, and the images are seeded from the title,
/// so without that line every ordinary pass would copy byte-for-byte what the previous one wrote.
/// Linear answers a single read from a replica that can still hold that previous content after
/// the reset is confirmed, and a copy reading it finds nothing to change and writes nothing —
/// which is how one ordinary pass showed the reset marker with no `onetaskgraph.assets`.
fn asset_origins(
    journey: &Journey,
    title: impl Fn(&str) -> String,
) -> Vec<(&'static str, String, String)> {
    static INVOCATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let invocation = format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        INVOCATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let mut origins = Vec::new();
    for kind in ["task", "document"] {
        let title = title(kind);
        let (id, _) = journey.create_named(kind, 2, &title, false);
        let output = journey
            .sandbox
            .command()
            .args(["--json", kind, "show", &id])
            .output()
            .unwrap();
        let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
        let path = shown["items"][0]["item"]["location"]["path"]
            .as_str()
            .unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let mut text: String = text
            .lines()
            .filter(|line| !line.starts_with("project:"))
            .map(|line| format!("{line}\n"))
            .collect();
        text.push_str(&format!("\n{INVOCATION_LINE}{invocation}\n"));
        std::fs::write(path, text).unwrap();
        origins.push((kind, id, title));
    }
    origins
}

fn cleanup_asset_items(
    runtime: &tokio::runtime::Runtime,
    endpoint: &str,
    key: &str,
    items: &[(&str, String)],
) -> bool {
    items
        .iter()
        .map(|(kind, id)| {
            let (query, root) = if *kind == "issue" {
                (onetaskgraph_linear::graphql::ISSUE_DELETE, "issueDelete")
            } else {
                (
                    onetaskgraph_linear::graphql::DOCUMENT_DELETE,
                    "documentDelete",
                )
            };
            // Attempt every removal even when another failed, preserving the observer journal.
            runtime
                .block_on(document_lifecycle::send(
                    endpoint,
                    key,
                    query,
                    json!({"id":id}),
                ))
                .is_ok_and(|response| response[root]["success"] == true)
        })
        .fold(true, |all, removed| all & removed)
}

fn retire_asset_invocation(
    runtime: &tokio::runtime::Runtime,
    endpoint: &str,
    key: &str,
    created: &[(&str, String)],
    standing: &document_lifecycle::Standing,
    scope: Scope<'_>,
) -> bool {
    let mut items = created.to_vec();
    if !scope.is_ci() {
        items.extend(standing.ids.values().cloned().map(|id| ("document", id)));
    }
    cleanup_asset_items(runtime, endpoint, key, &items)
}

fn reset_asset_slot(
    runtime: &tokio::runtime::Runtime,
    endpoint: &str,
    key: &str,
    standing: &document_lifecycle::Standing,
    stamp: &str,
    reads: u32,
) {
    if let Some(id) = standing.ids.get("assets") {
        let marker = format!("Reset for invocation {stamp}");
        let response=runtime.block_on(document_lifecycle::send(endpoint,key,
            "mutation($id:String!,$input:DocumentUpdateInput!){documentUpdate(id:$id,input:$input){success}}",
            json!({"id":id,"input":{"content":marker}}))).unwrap();
        assert_eq!(
            response["documentUpdate"]["success"], true,
            "Linear must acknowledge the run-specific reset"
        );
        for attempt in 0..reads {
            let held = runtime
                .block_on(document_lifecycle::send(
                    endpoint,
                    key,
                    onetaskgraph_linear::graphql::DOCUMENT,
                    json!({"id":id}),
                ))
                .unwrap();
            if held["document"]["content"].as_str() == Some(marker.as_str()) {
                return;
            }
            if attempt + 1 < reads {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        panic!("Linear did not retain the run-specific asset reset before copying");
    }
}

/// The live and replay lanes share the exact copy/read/unchanged-recopy assertions.
fn drive_asset_copy_recopy(
    journey: &Journey,
    origins: &[(&str, String, String)],
    mode: Mode,
    asset_read: impl Fn(&str, &str),
    upload_count: impl Fn() -> usize,
) {
    for (kind, id, title) in origins {
        let kind = *kind;
        let run = || {
            let mut command = journey.sandbox.command();
            command.args(["--json", kind, "copy", id.as_str(), "--to", "dest"]);
            if kind == "document" && mode != Mode::Fresh {
                command.args(["--match-by", "title"]);
            }
            command.output().unwrap()
        };
        let show = |id: &str| {
            let output = journey
                .sandbox
                .command()
                .args(["--json", kind, "show", id])
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", stderr(&output));
            serde_json::from_str::<Value>(&stdout(&output)).unwrap()
        };
        let origin = show(id);
        let invocation = origin["items"][0]["item"]["content"]
            .as_str()
            .unwrap()
            .lines()
            .find(|line| line.starts_with(INVOCATION_LINE))
            .expect("every origin names its invocation")
            .to_owned();
        let output = run();
        assert!(output.status.success(), "{}", stderr(&output));
        let report: Value = serde_json::from_str(&stdout(&output)).unwrap();
        let dest = report["items"][0]["destination"].as_str().unwrap();
        // A single Linear read can still answer from before the copy; read until it reaches it.
        let mut shown = show(dest);
        for _ in 1..SHOW_READS {
            if shown["items"][0]["item"]["content"]
                .as_str()
                .is_some_and(|content| content.contains(&invocation))
            {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
            shown = show(dest);
        }
        let item = &shown["items"][0]["item"];
        assert!(
            item["content"]
                .as_str()
                .is_some_and(|content| content.contains(&invocation)),
            "{kind} copy {} never reached {dest} after {SHOW_READS} reads: {}",
            report["items"][0]["action"],
            item["content"]
        );
        assert_eq!(
            item["title"],
            title.as_str(),
            "the destination retains its recoverable ownership title"
        );
        let content = item["content"].as_str().unwrap();
        assert!(!content.contains("](./"));
        let uploads = item["metadata"]["onetaskgraph.assets"].as_object().unwrap();
        assert_eq!(uploads.len(), 2);
        for (name, upload) in uploads {
            let url = upload["url"].as_str().unwrap();
            assert!(content.contains(url));
            asset_read(name, url);
        }
        let before = upload_count();
        let output = run();
        assert!(output.status.success(), "{}", stderr(&output));
        let after = upload_count();
        assert_eq!(after, before, "unchanged copy sends no fileUpload");
    }
}

#[test]
fn live_disposable_issue_and_document_assets_copy_and_recopy() {
    use onetaskgraph_live::{Credential, Exclusivity, Session, missing, required};
    let demand = match std::env::var(onetaskgraph_live::REQUIRED_VARIABLE) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("ONETASKGRAPH_LIVE_REQUIRED is not Unicode")
        }
    };
    let demanded = required(demand.as_deref()).unwrap();
    let key = std::env::var("LINEAR_API_KEY")
        .ok()
        .and_then(Credential::new);
    let team = std::env::var("LINEAR_WRITE_TEAM")
        .ok()
        .filter(|team| !team.trim().is_empty());
    let (Some(key), Some(team)) = (key, team) else {
        eprintln!(
            "{}",
            missing(
                demanded,
                "linear-assets",
                "LINEAR_API_KEY or LINEAR_WRITE_TEAM is absent; no live asset request sent"
            )
            .unwrap()
        );
        return;
    };
    let session = Session::open("linear", key, Exclusivity::OneAtATime).unwrap();
    let key = session.credential().expose().to_owned();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = reqwest::Client::new();
    let response: Value = runtime.block_on(async {
        client
            .post("https://api.linear.app/graphql")
            .header("Authorization", &key)
            .json(
                &json!({"query":onetaskgraph_linear::graphql::RESOLUTION,"variables":{"key":team}}),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    });
    let state_name = response["data"]["teams"]["nodes"][0]["states"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|state| state["type"] == "unstarted")
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_owned();
    let mode = Mode::read().unwrap();
    let ci = std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true");
    let owner_stamp = onetaskgraph_live::artifact::Stamp::new(
        onetaskgraph_live::artifact::Run::current(),
        onetaskgraph_live::artifact::now_micros(),
    )
    .to_string();
    let team_key = team.to_ascii_uppercase();
    let scope = if ci {
        Scope::ci(&team_key).unwrap()
    } else {
        Scope::local(&team_key, &owner_stamp).unwrap()
    };
    let team_id = response["data"]["teams"]["nodes"][0]["id"]
        .as_str()
        .unwrap();
    let effective_mode = mode;
    if !ci && mode == Mode::Initialize {
        eprintln!(
            "non-CI creation pass: initialize the invocation-owned asset destination, exercise reuse assertions, and retire it"
        );
    }
    let journal = document_lifecycle::journal::Journal::new("assets", effective_mode.name())
        .durable(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("telemetry/onetaskgraph-linear-e2e/live")
                .join(&owner_stamp),
        )
        .unwrap();
    let lifecycle_endpoint = document_lifecycle::observer::start(
        "https://api.linear.app/graphql",
        &key,
        journal.clone(),
    );
    let created = document_lifecycle::Created::default();
    guard_asset_invocation(
        &runtime,
        &lifecycle_endpoint,
        &key,
        &team_key,
        team_id,
        scope,
        effective_mode,
        &journal,
        &created,
        |standing| {
            if ci && mode == Mode::Initialize {
                journal.finish().unwrap();
                return;
            }
            drive_live_assets(
                &runtime,
                &client,
                &key,
                &team,
                &state_name,
                standing,
                scope,
                mode,
                &owner_stamp,
                &journal,
                &lifecycle_endpoint,
                &created,
            );
        },
    );
}

/// Prepares the lane's standing slot and runs `drive` over it, so that from the first
/// create on nothing this invocation owns outlives a panic: one partway through preparing,
/// or anywhere in `drive` before its own cleanup ran, retires every slot `created` records
/// before the panic leaves the lane. A CI set is standing and is never retired here.
#[allow(
    clippy::too_many_arguments,
    reason = "prepare_owned's lane identities plus the journal and drive; the offline journey passes each one separately to drive this guard as the live lane does"
)]
fn guard_asset_invocation(
    runtime: &tokio::runtime::Runtime,
    lifecycle_endpoint: &str,
    key: &str,
    team_key: &str,
    team_id: &str,
    scope: Scope<'_>,
    mode: Mode,
    journal: &document_lifecycle::journal::Journal,
    created: &document_lifecycle::Created,
    drive: impl FnOnce(&document_lifecycle::Standing),
) {
    let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let standing = runtime
            .block_on(document_lifecycle::prepare_owned(
                lifecycle_endpoint,
                key,
                team_key,
                team_id,
                scope,
                mode,
                &[Slot::Assets],
                created,
            ))
            .unwrap_or_else(|error| panic!("{error}"));
        // Whatever preparation created was the standing set's cold rebuild; from here on a
        // create is this journey's own.
        journal.journey("live_disposable_issue_and_document_assets_copy_and_recopy");
        drive(&standing);
    }));
    if let Err(panic) = guarded {
        if let Err(error) = runtime.block_on(document_lifecycle::retire(
            lifecycle_endpoint,
            key,
            scope,
            created,
        )) {
            eprintln!("{error}");
        }
        let _ = journal.finish();
        std::panic::resume_unwind(panic);
    }
}

/// Everything the live asset lane does once its standing slot is prepared, ending with the
/// invocation cleanup; a panic here unwinds to the caller's guard over `created`.
#[allow(
    clippy::too_many_arguments,
    reason = "the live lane's locals this body used before it moved under the guard, passed unchanged"
)]
fn drive_live_assets(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    key: &str,
    team: &str,
    state_name: &str,
    standing: &document_lifecycle::Standing,
    scope: Scope<'_>,
    mode: Mode,
    owner_stamp: &str,
    journal: &document_lifecycle::journal::Journal,
    lifecycle_endpoint: &str,
    created: &document_lifecycle::Created,
) {
    let key = key.to_owned();
    reset_asset_slot(
        runtime,
        "https://api.linear.app/graphql",
        &key,
        standing,
        owner_stamp,
        21,
    );
    let journey = Journey::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros();
    let origins = asset_origins(&journey, |kind| {
        if kind == "document" && mode != Mode::Fresh {
            standing.titles["assets"].clone()
        } else {
            format!(
                "{}{}",
                onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX,
                onetaskgraph_live::artifact::Stamp::new(
                    onetaskgraph_live::artifact::Run::current(),
                    stamp as u64
                )
            )
        }
    });
    let (endpoint, observed) = live_observer(
        key.clone(),
        journal.clone(),
        "https://api.linear.app/graphql".into(),
    );
    journey
        .sandbox
        .secrets_file(&format!("LINEAR_API_KEY={key}\n"));
    journey.sandbox.project_document(&document(&json!({
        "notes":{"plugin":"local-md","config":{"root":journey.sandbox.project().join("notes")}},
        "dest":{"plugin":"linear","config":{"endpoint":endpoint,"team":team,"status_mapping":{"todo":state_name}}}
    })));
    // Catch assertion failures so disposable items are removed before rethrowing. Cleanup
    // reads the observer's item journal, never the failed journey state; mutex guards are
    // dropped before assertions, and the runtime/client are only reused for fresh deletes.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drive_asset_copy_recopy(
            &journey,
            &origins,
            mode,
            |name, url| {
                let parsed = reqwest::Url::parse(url).unwrap();
                assert_eq!(parsed.scheme(), "https");
                assert_eq!(parsed.host_str(), Some("uploads.linear.app"));
                assert_eq!(parsed.port_or_known_default(), Some(443));
                assert!(
                    parsed.username().is_empty()
                        && parsed.password().is_none()
                        && parsed.fragment().is_none()
                );
                let response = runtime
                    .block_on(client.get(url).header("Authorization", &key).send())
                    .unwrap();
                eprintln!(
                    "{name} authenticated asset GET {url}: HTTP {}",
                    response.status()
                );
                assert!(response.status().is_success());
            },
            || observed.lock().unwrap().signed.len(),
        );
    }));
    for observation in &observed.lock().unwrap().observations {
        eprintln!("{observation}");
    }
    let items = observed.lock().unwrap().items.clone();
    let cleanup_failed =
        !retire_asset_invocation(runtime, lifecycle_endpoint, &key, &items, standing, scope);
    // That cleanup attempted every slot `created` recorded, so the caller's guard must not
    // send their deletes a second time.
    created.forget();
    journal.finish().unwrap();
    assert!(!cleanup_failed, "live disposable cleanup failed");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn regenerating_a_linear_rendering_stores_changed_assets_with_its_metadata_slot() {
    let journey = Journey::new();
    let inputs = journey.sandbox.subdirectory("render-inputs");
    let template = inputs.join("picture.md");
    let image = inputs.join("picture.png");
    std::fs::write(&template, "![rendered alt](./picture.png)\n").unwrap();
    std::fs::write(&image, images::png(800, 475_000)).unwrap();
    for kind in ["task", "document"] {
        let output = journey
            .sandbox
            .command()
            .envs(journey.workspace.clock.client_env(0))
            .args([
                kind,
                "create",
                "dest",
                "--project",
                "launch",
                "--title",
                "Rendered",
                "--metadata",
                "caller.keep=true",
                "--template",
                template.to_str().unwrap(),
                "--asset",
                image.to_str().unwrap(),
                "--no-interactive",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        let id = stdout(&output).trim().to_owned();
        let changed = images::png(if kind == "task" { 801 } else { 802 }, 475_000);
        std::fs::write(&image, &changed).unwrap();
        let before = journey.workspace.counts();
        let output = journey
            .sandbox
            .command()
            .envs(journey.workspace.clock.client_env(0))
            .args([
                kind,
                "render",
                &id,
                "--asset",
                image.to_str().unwrap(),
                "--no-interactive",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            journey.workspace.counts(),
            (before.0 + 1, before.1 + 1, before.2)
        );
        let raw = journey
            .workspace
            .held
            .long_form(
                if kind == "task" { "tasks" } else { "documents" },
                id.split_once(':').unwrap().1,
            )
            .unwrap();
        assert!(raw.contains("![rendered alt](http://"));
        assert!(raw.contains("onetaskgraph.template") && raw.contains("onetaskgraph.assets"));
        assert!(raw.contains(&asset_sha256(&changed)));
        assert!(!raw.contains("](./"));
        assert!(raw.contains("\"caller.keep\":true"));
        let before = journey.workspace.counts();
        let render = || {
            journey
                .sandbox
                .command()
                .envs(journey.workspace.clock.client_env(0))
                .args([
                    kind,
                    "render",
                    &id,
                    "--asset",
                    image.to_str().unwrap(),
                    "--no-interactive",
                ])
                .output()
                .unwrap()
        };
        let output = render();
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            journey.workspace.counts(),
            before,
            "unchanged rendering reuses the upload"
        );
        std::fs::write(&image, images::png(900 + before.0 as u64, 475_000)).unwrap();
        journey.workspace.state.lock().unwrap().fail = Some(("PUT", 1, 403));
        let output = render();
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("picture.png")
                && stderr(&output).contains("PUT")
                && stderr(&output).contains("403")
        );
        assert_eq!(
            journey
                .workspace
                .held
                .long_form(
                    if kind == "task" { "tasks" } else { "documents" },
                    id.split_once(':').unwrap().1
                )
                .as_deref(),
            Some(raw.as_str()),
            "a failed asset rendering preserves content and provenance"
        );
    }
}

#[test]
fn asset_rendering_of_missing_items_returns_none_without_uploading() {
    use onetaskgraph_plugin_api::{
        AssetName, AssetPayload, AssetWrite, NativeId, SourceName, SourcePlugin,
    };
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace(
        &sandbox,
        json!({"tasks":[],"projects":[],"documents":[],"labels":[],"task_dependencies":[],"project_dependencies":[]}),
    );
    let secrets = onetaskgraph_core::Secrets::load(onetaskgraph_core::Environment::from_pairs([(
        "LINEAR_API_KEY",
        "fixture-key",
    )]))
    .unwrap();
    let source = onetaskgraph_linear::Plugin
        .build(&SourceName::new("dest").unwrap(), &config, &secrets)
        .unwrap();
    let assets = AssetWrite {
        assets: vec![AssetPayload::of(
            AssetName::new("picture.png").unwrap(),
            images::png(500, 475_000),
        )],
        recorded_assets: None,
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let answers = BTreeMap::new();
    let id = NativeId::from("missing");
    assert_eq!(
        runtime
            .block_on(source.set_task_rendering_with_assets(
                &id,
                "![alt](./picture.png)",
                &json!({}),
                &answers,
                &assets
            ))
            .unwrap(),
        None
    );
    assert_eq!(
        runtime
            .block_on(source.set_document_rendering_with_assets(
                &id,
                "![alt](./picture.png)",
                &json!({}),
                &answers,
                &assets
            ))
            .unwrap(),
        None
    );
    assert!(
        !workspace
            .served()
            .iter()
            .any(|(query, _)| query.contains("fileUpload"))
    );
}

#[test]
fn malformed_upload_answers_and_transport_failures_refuse_before_content_is_written() {
    for (fault, stage) in [
        ("invalid-json", "mutation"),
        ("graphql-errors", "mutation"),
        ("success-false", "mutation"),
        ("malformed-upload", "mutation"),
        ("disconnect-mutation", "mutation"),
        ("disconnect-PUT", "PUT"),
    ] {
        let journey = Journey::new();
        let (id, _) = journey.create("task", 1);
        journey.workspace.state.lock().unwrap().fault = Some(fault);
        let refused = journey.copy("task", &id, false);
        let said = refused.as_str().unwrap();
        assert!(
            said.contains(stage) && said.contains("task-0.png"),
            "{fault}: {said}"
        );
        if !fault.starts_with("disconnect") {
            assert!(said.contains("200"), "{said}");
        }
        assert!(
            !journey
                .workspace
                .held
                .served()
                .iter()
                .any(|(query, _)| query.contains("issueCreate("))
        );
    }
}

#[test]
fn reset_hints_defaults_and_non_json_http_429_are_waited_on_the_clock() {
    for (hint, minimum, maximum) in [
        ("normal", 1.99, 2.01),
        ("body", 1.99, 2.01),
        // HTTP Retry-After=3 takes precedence over GraphQL retryAfter=2.
        ("conflicting", 2.99, 3.01),
        ("none", 0.99, 1.01),
        ("zero", 0.0009, 0.0011),
        ("expired", 0.0009, 0.0011),
        ("malformed", 1.99, 2.01),
        ("epoch", 5.0, 6.01),
    ] {
        let journey = Journey::new();
        let (id, _) = journey.create("task", 1);
        {
            let mut state = journey.workspace.state.lock().unwrap();
            state.fail = Some(("mutation", 1, 429));
            state.hint = Some(hint);
        }
        journey.copy("task", &id, true);
        let network = journey.workspace.state.lock().unwrap().network_time;
        let waited = journey
            .workspace
            .clock
            .now()
            .saturating_sub(network)
            .as_secs_f64();
        assert!(
            (minimum..=maximum).contains(&waited),
            "{hint}: waited {waited}s, network {network:?}"
        );
        assert_eq!(journey.workspace.counts(), (1, 1, 0));
    }
    let journey = Journey::new();
    let (id, _) = journey.create("task", 1);
    {
        let mut state = journey.workspace.state.lock().unwrap();
        state.fail = Some(("mutation", 1, 429));
        state.fault = Some("mutation-http429");
    }
    journey.copy("task", &id, true);
    assert_eq!(journey.workspace.counts(), (1, 1, 0));
}

#[test]
fn a_records_wait_allowance_is_shared_across_stages_and_images() {
    for fault in ["shared-stages", "shared-images"] {
        let journey = Journey::new();
        let (id, _) = journey.create("task", 2);
        {
            let mut state = journey.workspace.state.lock().unwrap();
            state.fault = Some(fault);
            state.hint = Some("shared");
        }
        let refused = journey.copy("task", &id, false);
        let said = refused.as_str().unwrap();
        assert!(
            said.contains("limiter") && said.contains("60 second"),
            "{said}"
        );
        assert!(
            said.contains(if fault == "shared-stages" {
                "PUT"
            } else {
                "task-1.png"
            }),
            "{said}"
        );
        let network = journey.workspace.state.lock().unwrap().network_time;
        assert_eq!(
            journey.workspace.clock.now().saturating_sub(network),
            Duration::from_secs(31)
        );
        assert!(
            !journey
                .workspace
                .held
                .served()
                .iter()
                .any(|(query, _)| query.contains("issueCreate("))
        );
    }
}

#[test]
fn public_asset_writes_validate_payloads_and_require_bytes_for_new_uploads() {
    use onetaskgraph_plugin_api::{
        AssetContentType, AssetName, AssetPayload, AssetWrite, ItemWrite, SourceName, SourcePlugin,
    };
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace(
        &sandbox,
        json!({"tasks":[{"id":"T","title":"Existing","status":{"category":"todo","name":"Todo"},"labels":[]}],"projects":[],"documents":[],"labels":[],"task_dependencies":[],"project_dependencies":[]}),
    );
    let secrets = onetaskgraph_core::Secrets::load(onetaskgraph_core::Environment::from_pairs([(
        "LINEAR_API_KEY",
        "fixture-key",
    )]))
    .unwrap();
    let source = onetaskgraph_linear::Plugin
        .build(&SourceName::new("dest").unwrap(), &config, &secrets)
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut item = runtime
        .block_on(source.get_task(&"T".into()))
        .unwrap()
        .unwrap();
    item.content = Some("![alt](./picture.png)".to_owned());
    let write = ItemWrite {
        target: None,
        item,
        depends_on: Vec::new(),
    };
    for broken in ["digest", "bytes", "content-type", "missing-bytes"] {
        let mut payload = AssetPayload::of(
            AssetName::new("picture.png").unwrap(),
            images::png(900, 475_000),
        );
        match broken {
            "digest" => payload.sha256 = "invalid".to_owned(),
            "bytes" => payload.bytes = Some(images::png(901, 475_000)),
            "content-type" => payload.content_type = AssetContentType::Jpeg,
            _ => payload.bytes = None,
        }
        let before = workspace.served().len();
        let error = runtime
            .block_on(source.write_task_with_assets(
                &write,
                None,
                &AssetWrite {
                    assets: vec![payload],
                    recorded_assets: None,
                },
            ))
            .unwrap_err();
        assert!(error.to_string().contains("picture.png"), "{error}");
        assert_eq!(
            workspace.served().len(),
            before,
            "invalid payload sends no request"
        );
    }
    for url in [
        "",
        "https://example.com/image.png",
        "http://127.0.0.1:1/image.png",
    ] {
        let payload = AssetPayload::of(
            AssetName::new("picture.png").unwrap(),
            images::png(902, 475_000),
        );
        let recorded = onetaskgraph_plugin_api::AssetUploads(std::collections::BTreeMap::from([(
            payload.name.clone(),
            onetaskgraph_plugin_api::AssetUpload {
                sha256: payload.sha256.clone(),
                url: url.to_owned(),
            },
        )]));
        let before = workspace.served().len();
        let error = runtime
            .block_on(source.write_task_with_assets(
                &write,
                None,
                &AssetWrite {
                    assets: vec![payload],
                    recorded_assets: Some(recorded),
                },
            ))
            .unwrap_err();
        assert!(error.to_string().contains("picture.png"), "{error}");
        assert_eq!(workspace.served().len(), before);
    }
}

#[test]
fn the_published_quota_refills_in_virtual_time_before_an_asset_retry() {
    let journey = Journey::new();
    let (id, _) = journey.create("task", 1);
    journey.workspace.state.lock().unwrap().fault = Some("quota-at-upload");
    let result = journey.copy("task", &id, true);
    assert_eq!(journey.workspace.counts(), (1, 1, 0));
    let state = journey.workspace.state.lock().unwrap();
    assert!(
        journey
            .workspace
            .clock
            .now()
            .saturating_sub(state.network_time)
            >= Duration::from_secs(2)
    );
    drop(state);
    let id = result["items"][0]["destination"]
        .as_str()
        .unwrap()
        .split_once(':')
        .unwrap()
        .1;
    let raw = journey.workspace.held.long_form("tasks", id).unwrap();
    assert!(!raw.contains("](./") && raw.contains("onetaskgraph.assets"));
}

#[test]
fn records_without_images_keep_the_same_non_asset_request_flow() {
    for kind in ["task", "document"] {
        let plain = Journey::new();
        let pictured = Journey::new();
        let (plain_id, _) = plain.create(kind, 0);
        let (pictured_id, _) = pictured.create(kind, 1);
        plain.copy(kind, &plain_id, true);
        pictured.copy(kind, &pictured_id, true);
        assert_eq!(plain.workspace.counts(), (0, 0, 0));
        assert_eq!(pictured.workspace.counts(), (1, 1, 0));
        let plain_operations: Vec<_> = plain
            .workspace
            .held
            .served()
            .into_iter()
            .map(|(query, _)| query)
            .collect();
        let pictured_operations: Vec<_> = pictured
            .workspace
            .held
            .served()
            .into_iter()
            .map(|(query, _)| query)
            .collect();
        assert_eq!(
            plain_operations, pictured_operations,
            "assets add only their upload protocol, leaving every ordinary request as it was"
        );
        assert_eq!(
            plain.workspace.state.lock().unwrap().requests,
            plain_operations.len()
        );
    }
}

#[test]
fn upload_destinations_and_headers_are_validated_before_a_put_or_authenticated_get() {
    for fault in [
        "untrusted-asset",
        "wrong-loopback",
        "wrong-upload-loopback",
        "bad-upload-scheme",
        "credentials-upload",
        "fragment-asset",
        "bad-header-name",
        "bad-header-value",
    ] {
        let journey = Journey::new();
        let (id, _) = journey.create("task", 1);
        journey.workspace.state.lock().unwrap().fault = Some(fault);
        let refused = journey.copy("task", &id, false);
        assert!(
            refused.as_str().unwrap().contains("task-0.png")
                && refused.as_str().unwrap().contains("mutation"),
            "{refused}"
        );
        assert_eq!(journey.workspace.counts(), (1, 0, 0));
        assert!(
            !journey
                .workspace
                .held
                .served()
                .iter()
                .any(|(query, _)| query.contains("issueCreate("))
        );
    }
}

#[test]
fn redirects_at_each_asset_stage_are_refused_before_content_is_written() {
    for (fault, stage) in [("redirect-mutation", "mutation"), ("redirect-PUT", "PUT")] {
        let journey = Journey::new();
        let (id, _) = journey.create("task", 1);
        journey.workspace.state.lock().unwrap().fault = Some(fault);
        let refused = journey.copy("task", &id, false);
        let said = refused.as_str().unwrap();
        assert!(
            said.contains("task-0.png") && said.contains(stage) && said.contains("302"),
            "{said}"
        );
        assert!(
            !journey
                .workspace
                .held
                .served()
                .iter()
                .any(|(query, _)| query.contains("issueCreate("))
        );
    }
}

#[cfg(unix)]
#[test]
fn malformed_live_demand_is_refused_before_any_session_opens() {
    use std::os::unix::ffi::OsStringExt;
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy",
            "--nocapture",
        ])
        .env(
            onetaskgraph_live::REQUIRED_VARIABLE,
            std::ffi::OsString::from_vec(vec![0xff]),
        )
        .env_remove("LINEAR_API_KEY")
        .env_remove("LINEAR_WRITE_TEAM")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("ONETASKGRAPH_LIVE_REQUIRED is not Unicode"));
}

#[test]
fn a_task_created_with_an_asset_retires_the_far_ends_reverse_edges_in_a_caching_destination() {
    let journey = Journey::new();
    let cache = journey.sandbox.subdirectory("cache");
    journey.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": journey.sandbox.subdirectory("notes")}},
        "dest": {"plugin": "linear", "config": journey.config,
                 "cache": {"enabled": true, "dir": cache}},
    })));
    let run = |arguments: &[&str]| {
        let output = journey
            .sandbox
            .command()
            .envs(journey.workspace.clock.client_env(0))
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            stderr(&output)
        );
        stdout(&output)
    };
    let sent = || journey.workspace.held.served().len();
    let copied: Value = serde_json::from_str(&run(&[
        "--json",
        "project",
        "copy",
        "notes:launch",
        "--to",
        "dest",
    ]))
    .unwrap();
    let project = copied["items"][0]["destination"]
        .as_str()
        .and_then(|id| id.split_once(':'))
        .map(|(_, native)| native.to_owned())
        .expect("the project landed");
    let inputs = journey.sandbox.subdirectory("far-inputs");
    let body = inputs.join("body.md");
    std::fs::write(&body, "The far end.\n").unwrap();
    let far = run(&[
        "task",
        "create",
        "dest",
        "--project",
        &project,
        "--title",
        "Far end",
        "--body-file",
        body.to_str().unwrap(),
    ])
    .trim()
    .to_owned();
    let listed: Value =
        serde_json::from_str(&run(&["--json", "task", "list", "--source", "dest"])).unwrap();
    let pin = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == far.as_str())
        .and_then(|item| item["item"]["version"].as_str())
        .expect("the far end carries a token")
        .to_owned();
    let reverse = [
        "--json",
        "task",
        "deps",
        &far,
        "--direction",
        "depended-on-by",
        "--pin",
        &pin,
    ];
    run(&reverse);
    let before = sent();
    run(&reverse);
    assert_eq!(
        sent(),
        before,
        "the far end's reverse edges are filled and served"
    );
    let native = far.split_once(':').unwrap().1;
    let directory = std::fs::read_dir(&cache)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("reverse-edges-v4").is_dir())
        .unwrap_or_else(|| cache.clone())
        .join("reverse-edges-v4");
    let kept = directory.join(format!("{}.json", asset_sha256(native.as_bytes())));
    assert!(kept.exists(), "{}", kept.display());

    let image = inputs.join("shot.png");
    std::fs::write(&image, images::png(17, 475_000)).unwrap();
    let near_body = inputs.join("near.md");
    std::fs::write(&near_body, "The near end.\n\n![shot](./shot.png)\n").unwrap();
    run(&[
        "task",
        "create",
        "dest",
        "--project",
        &project,
        "--title",
        "Near end",
        "--body-file",
        near_body.to_str().unwrap(),
        "--asset",
        image.to_str().unwrap(),
        "--depends-on",
        &far,
    ]);
    assert!(
        !kept.exists(),
        "the asset create takes the far end's reverse edges out of service"
    );
    let before = sent();
    let edges: Value = serde_json::from_str(&run(&reverse)).unwrap();
    assert!(sent() > before, "the next read is Linear's");
    assert_eq!(edges["items"].as_array().map(Vec::len), Some(1), "{edges}");
}

/// `sha256:` and the hex SHA-256 of `content`, hashed here rather than taken from the engine.
fn content_digest(content: &str) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(content.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
}

#[test]
fn an_asset_carrying_rendering_records_what_linear_stores_and_an_unstable_backend_is_refused() {
    let journey = Journey::new();
    journey
        .workspace
        .held
        .rewrite_markdown_on_write(LinearMarkdown::Measured);
    let inputs = journey.sandbox.subdirectory("stored-inputs");
    let template = inputs.join("shot.md");
    let image = inputs.join("shot.png");
    // A bullet Linear stores as `* `, the image its upload serves at a URL, and a numbered line
    // it keeps as written.
    std::fs::write(&template, "- ![shot](./shot.png)\n1) Then review it.\n").unwrap();
    std::fs::write(&image, images::png(700, 60_000)).unwrap();
    let run = |arguments: &[&str]| {
        journey
            .sandbox
            .command()
            .envs(journey.workspace.clock.client_env(0))
            .args(arguments)
            .output()
            .unwrap()
    };
    let shown = |kind: &str, id: &str| -> Value {
        let output = run(&[kind, "show", id, "--json"]);
        assert!(output.status.success(), "{}", stderr(&output));
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["items"][0]["item"].clone()
    };
    let verifiable = |item: &Value| {
        let content = item["content"].as_str().expect("content");
        assert!(content.starts_with("* ![shot](http://"), "{content}");
        assert_eq!(
            item["metadata"]["onetaskgraph.template"]["body_digest"],
            content_digest(content),
            "{item}"
        );
    };
    let failure_kind = |output: &std::process::Output| -> Value {
        let printed: Value = serde_json::from_str(&stdout(output)).unwrap_or(Value::Null);
        printed["failure"]["kind"].clone()
    };
    let template = template.to_str().unwrap();
    let image_path = image.to_str().unwrap();
    for (index, kind) in ["task", "document"].into_iter().enumerate() {
        let create = |source: &str, title: &str| {
            run(&[
                kind,
                "create",
                source,
                "--project",
                "launch",
                "--title",
                title,
                "--template",
                template,
                "--asset",
                image_path,
                "--no-interactive",
                "--json",
            ])
        };
        // A copy out of a folder of Markdown, then the same rendering created in Linear itself
        // and rendered again there: each records the digest of what Linear stores.
        let created = create("notes", "Shot");
        assert!(created.status.success(), "{}", stderr(&created));
        let source = serde_json::from_str::<Value>(&stdout(&created)).unwrap()["items"][0]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        let copied = journey.copy(kind, &source, true);
        let landed = copied["items"][0]["destination"]
            .as_str()
            .expect("an id")
            .to_owned();
        verifiable(&shown(kind, &landed));
        let direct = create("dest", "Shot in Linear");
        assert!(direct.status.success(), "{}", stderr(&direct));
        let direct = serde_json::from_str::<Value>(&stdout(&direct)).unwrap()["items"][0]["id"]
            .as_str()
            .expect("an id")
            .to_owned();
        verifiable(&shown(kind, &direct));
        std::fs::write(&image, images::png(710 + index as u64, 60_000)).unwrap();
        let render = |id: &str| {
            run(&[
                kind,
                "render",
                id,
                "--asset",
                image_path,
                "--no-interactive",
                "--json",
            ])
        };
        let rendered = render(&direct);
        assert!(rendered.status.success(), "{}", stderr(&rendered));
        verifiable(&shown(kind, &direct));

        // With a backend that does not hold its own stored content stable, the asset-carrying
        // copy, create and render are each refused as diverged once their follow-up write is
        // stored otherwise again, the copy reporting its item failed.
        journey
            .workspace
            .held
            .rewrite_markdown_on_write(LinearMarkdown::Unstable);
        // The record's own asset changes, so the copy writes rather than finding it unchanged.
        let output = run(&["--json", kind, "show", &source]);
        let held = serde_json::from_str::<Value>(&stdout(&output)).unwrap()["assets"][0]["path"]
            .as_str()
            .expect("the record's asset")
            .to_owned();
        std::fs::write(held, images::png(720 + index as u64, 60_000)).unwrap();
        let recopied = run(&["--json", kind, "copy", &source, "--to", "dest"]);
        assert_eq!(recopied.status.code(), Some(4), "{}", stderr(&recopied));
        let report: Value = serde_json::from_str(&stdout(&recopied)).unwrap();
        assert_eq!(
            report["items"][0]["error"]["kind"], "stored-content-diverged",
            "{report}"
        );
        let refused = create("dest", "Refused");
        assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
        assert_eq!(failure_kind(&refused), "stored-content-diverged");
        std::fs::write(&image, images::png(730 + index as u64, 60_000)).unwrap();
        let refused = render(&direct);
        assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
        assert_eq!(failure_kind(&refused), "stored-content-diverged");
        journey
            .workspace
            .held
            .rewrite_markdown_on_write(LinearMarkdown::Measured);
    }
}

#[test]
fn document_modes_replay_the_live_binary_asset_entry_over_http() {
    use document_lifecycle::journal::Journal;
    let journey = Journey::with_clients(0);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let endpoint = format!("{}/0/graphql", journey.workspace.endpoint);
    let journal = Journal::new("assets", "initialize");
    journey.workspace.state.lock().unwrap().document_journal = Some(journal.clone());
    let scope = Scope::ci("ENG").unwrap();
    let slots = &[Slot::Assets];
    // A default non-CI invocation refuses before any create and names the explicit mode,
    // which initializes its own set, finds it again on recovery and retires it.
    let local = Scope::local("ENG", "123-456-500").unwrap();
    let absent = runtime.block_on(document_lifecycle::prepare(
        &endpoint,
        "fixture-key",
        "ENG",
        "team",
        local,
        Mode::Ordinary,
        slots,
    ));
    assert!(absent.err().unwrap().contains("initialize"));
    assert_eq!(journal.count(), 0, "ordinary absence refuses before create");
    let created = document_lifecycle::Created::default();
    let standing = runtime
        .block_on(document_lifecycle::prepare_owned(
            &endpoint,
            "fixture-key",
            "ENG",
            "team",
            local,
            Mode::Initialize,
            slots,
            &created,
        ))
        .unwrap();
    assert_eq!(
        journal.count(),
        1,
        "one asset slot initialized at the HTTP boundary"
    );
    let again = runtime
        .block_on(document_lifecycle::prepare(
            &endpoint,
            "fixture-key",
            "ENG",
            "team",
            local,
            Mode::Initialize,
            slots,
        ))
        .unwrap();
    assert_eq!(standing.ids, again.ids);
    assert_eq!(
        journal.count(),
        1,
        "recovery discovers the exact initialized slot"
    );
    assert_eq!(
        runtime
            .block_on(document_lifecycle::retire(
                &endpoint,
                "fixture-key",
                local,
                &created
            ))
            .unwrap(),
        1
    );
    journal.finish().unwrap();
    // The CI set is absent: the first ordinary pass rebuilds its one slot through the lane's
    // own guard, files it as a cold rebuild, and runs every reuse assertion over it; the warm
    // pass after it creates nothing. Then the daily fresh pass.
    for (mode, label, warm) in [
        (Mode::Ordinary, "ordinary", false),
        (Mode::Ordinary, "ordinary", true),
        (Mode::Fresh, "fresh", true),
    ] {
        if !(mode == Mode::Ordinary && warm) {
            journal.begin(label);
        }
        let before = journal.count();
        let rebuilt_before = journal.count_in(document_lifecycle::journal::COLD_REBUILD);
        let run = match (mode, warm) {
            (Mode::Fresh, _) => 2000,
            (_, false) => 1000,
            (_, true) => 1001,
        };
        let mut fresh_title = None;
        guard_asset_invocation(
            &runtime,
            &endpoint,
            "fixture-key",
            "ENG",
            "team",
            scope,
            mode,
            &journal,
            &document_lifecycle::Created::default(),
            |standing| {
                reset_asset_slot(&runtime, &endpoint, "fixture-key", standing, label, 21);
                let origins = asset_origins(&journey, |kind| {
                    if kind == "document" && mode == Mode::Ordinary {
                        standing.titles["assets"].clone()
                    } else {
                        format!(
                            "{}123-456-{run}",
                            onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX,
                        )
                    }
                });
                drive_asset_copy_recopy(
                    &journey,
                    &origins,
                    mode,
                    |name, url| {
                        let response = runtime
                            .block_on(
                                reqwest::Client::new()
                                    .get(url)
                                    .header("Authorization", "fixture-key")
                                    .send(),
                            )
                            .unwrap();
                        assert!(response.status().is_success());
                        let bytes = runtime.block_on(response.bytes()).unwrap();
                        let state = journey.workspace.state.lock().unwrap();
                        let upload = state
                            .uploads
                            .values()
                            .find(|u| u.name == name && u.bytes == bytes.as_ref())
                            .expect("authenticated image read is byte-identical to the upload");
                        assert_eq!(upload.bytes.len(), upload.size);
                        assert_eq!(asset_sha256(&upload.bytes), asset_sha256(&bytes));
                    },
                    || journey.workspace.counts().0,
                );
                fresh_title = origins
                    .iter()
                    .find(|(kind, _, _)| *kind == "document")
                    .map(|origin| origin.2.clone());
            },
        );
        let rebuilt = journal.count_in(document_lifecycle::journal::COLD_REBUILD) - rebuilt_before;
        assert_eq!(
            rebuilt,
            usize::from(!warm),
            "{label}: only the cold ordinary pass rebuilds the missing standing slot"
        );
        assert_eq!(
            journal.count() - before - rebuilt,
            usize::from(mode == Mode::Fresh),
            "{label}: the shared entry path creates the fresh destination once"
        );
        // Removal does not erase a creation from its transport journal.
        if mode == Mode::Fresh {
            let title = fresh_title.unwrap();
            let id = journey.workspace.held.document_titled(&title).unwrap();
            assert!(cleanup_asset_items(
                &runtime,
                &endpoint,
                "fixture-key",
                &[("document", id)]
            ));
            assert!(journey.workspace.held.document_titled(&title).is_none());
            assert_eq!(journal.count(), 1);
        }
        journal.finish().unwrap();
    }
    journal.begin("failure");
    journal.journey("failed_asset_copy_cleanup");
    let title = format!(
        "{}123-456-3000",
        onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX
    );
    let origins = asset_origins(&journey, |kind| format!("{title}-{kind}"));
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drive_asset_copy_recopy(
            &journey,
            &origins,
            Mode::Fresh,
            |_, _| panic!("injected assertion failure after creation"),
            || journey.workspace.counts().0,
        )
    }));
    assert!(failure.is_err());
    // The first origin is an issue; exercise the document failure path separately.
    let document = origins
        .iter()
        .find(|(kind, _, _)| *kind == "document")
        .unwrap()
        .clone();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drive_asset_copy_recopy(
            &journey,
            std::slice::from_ref(&document),
            Mode::Fresh,
            |_, _| panic!("injected document assertion failure"),
            || journey.workspace.counts().0,
        )
    }));
    assert!(failure.is_err());
    let id = journey.workspace.held.document_titled(&document.2).unwrap();
    assert_eq!(
        journal.count(),
        1,
        "failed-run creations remain in the journal"
    );
    assert!(cleanup_asset_items(
        &runtime,
        &endpoint,
        "fixture-key",
        &[("document", id)]
    ));
    assert!(
        journey
            .workspace
            .held
            .document_titled(&document.2)
            .is_none()
    );
    assert_eq!(journal.count(), 1);
    journal.residue(usize::from(
        journey
            .workspace
            .held
            .document_titled(&document.2)
            .is_some(),
    ));
    journal.finish().unwrap();
}

#[test]
fn asset_invocation_cleanup_retires_local_slots_and_continues_after_a_refused_delete() {
    let journey = Journey::with_clients(0);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let endpoint = format!("{}/0/graphql", journey.workspace.endpoint);
    let ci_scope = Scope::ci("ENG").unwrap();
    let local_scope = Scope::local("ENG", "1-2-3").unwrap();
    let prepare = |scope| {
        runtime
            .block_on(document_lifecycle::prepare(
                &endpoint,
                "fixture-key",
                "ENG",
                "team",
                scope,
                Mode::Initialize,
                &[Slot::Assets],
            ))
            .unwrap()
    };
    let ci = prepare(ci_scope);
    let local = prepare(local_scope);
    let create = |title: &str| {
        runtime.block_on(document_lifecycle::send(&endpoint,"fixture-key",onetaskgraph_linear::graphql::DOCUMENT_CREATE,json!({"input":{"title":title,"content":"synthetic disposable cleanup probe","teamId":"team"}}))).unwrap()["documentCreate"]["document"]["id"].as_str().unwrap().to_owned()
    };
    let first_title = format!("{}1-2-4", onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX);
    let second_title = format!("{}1-2-5", onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX);
    let first = create(&first_title);
    let second = create(&second_title);
    let unrelated = create("Unrelated synthetic document");
    journey
        .workspace
        .state
        .lock()
        .unwrap()
        .refused_document_deletes
        .insert(first.clone());
    let created = vec![("document", first.clone()), ("document", second.clone())];
    assert!(
        !retire_asset_invocation(
            &runtime,
            &endpoint,
            "fixture-key",
            &created,
            &local,
            local_scope
        ),
        "a refused removal reports failure"
    );
    let attempted = journey
        .workspace
        .state
        .lock()
        .unwrap()
        .document_delete_attempts
        .clone();
    for id in [&first, &second, &local.ids["assets"]] {
        assert!(
            attempted.contains(id),
            "cleanup attempted every invocation-owned item"
        );
    }
    assert!(
        journey
            .workspace
            .held
            .document_titled(&first_title)
            .is_some()
    );
    assert!(
        journey
            .workspace
            .held
            .document_titled(&second_title)
            .is_none()
    );
    assert!(
        journey
            .workspace
            .held
            .document_titled(&local.titles["assets"])
            .is_none(),
        "local retained destination retires within invocation cleanup"
    );
    assert_eq!(
        journey
            .workspace
            .held
            .document_titled("Unrelated synthetic document"),
        Some(unrelated)
    );
    assert!(retire_asset_invocation(
        &runtime,
        &endpoint,
        "fixture-key",
        &[],
        &ci,
        ci_scope
    ));
    assert_eq!(
        journey.workspace.held.document_titled(&ci.titles["assets"]),
        Some(ci.ids["assets"].clone()),
        "CI standing slot survives the same cleanup entry"
    );
    journey
        .workspace
        .state
        .lock()
        .unwrap()
        .refused_document_deletes
        .clear();
    assert!(
        cleanup_asset_items(&runtime, &endpoint, "fixture-key", &[("document", first)]),
        "retry retires only the refused invocation item"
    );
    assert!(
        journey
            .workspace
            .held
            .document_titled(&first_title)
            .is_none()
    );
    let local = prepare(local_scope);
    assert!(retire_asset_invocation(
        &runtime,
        &endpoint,
        "fixture-key",
        &[],
        &local,
        local_scope
    ));
    assert!(
        journey
            .workspace
            .held
            .document_titled(&local.titles["assets"])
            .is_none(),
        "successful normal local cleanup also retires its set"
    );
}

/// A panic straight after the asset lane's one local slot create, and one later in the
/// drive before its own cleanup, each retire the invocation's slot before the panic leaves
/// the lane: one documentCreate and one documentDelete of that document at the transport,
/// the create still counted, and the standing CI slot and an unrelated document kept.
#[test]
fn a_panic_after_the_local_asset_slot_create_retires_it_before_the_lane_fails() {
    let journey = Journey::with_clients(0);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let endpoint = format!("{}/0/graphql", journey.workspace.endpoint);
    let journal = document_lifecycle::journal::Journal::new("assets", "initialize");
    journey.workspace.state.lock().unwrap().document_journal = Some(journal.clone());
    let ci = runtime
        .block_on(document_lifecycle::prepare(
            &endpoint,
            "fixture-key",
            "ENG",
            "team",
            Scope::ci("ENG").unwrap(),
            Mode::Initialize,
            &[Slot::Assets],
        ))
        .unwrap();
    let unrelated = runtime.block_on(document_lifecycle::send(&endpoint,"fixture-key",onetaskgraph_linear::graphql::DOCUMENT_CREATE,json!({"input":{"title":"Unrelated synthetic document","content":"synthetic","teamId":"team"}}))).unwrap()["documentCreate"]["document"]["id"].as_str().unwrap().to_owned();
    for (stamp, created, drive_panics) in [
        (
            "1-2-3",
            document_lifecycle::Created::panicking_after(1),
            false,
        ),
        ("1-2-4", document_lifecycle::Created::default(), true),
    ] {
        let scope = Scope::local("ENG", stamp).unwrap();
        let title =
            onetaskgraph_linear::live_artifact::retained_title(scope, Slot::Assets).unwrap();
        let creates = journal.count();
        let deletes = journey
            .workspace
            .state
            .lock()
            .unwrap()
            .document_delete_attempts
            .len();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            guard_asset_invocation(
                &runtime,
                &endpoint,
                "fixture-key",
                "ENG",
                "team",
                scope,
                Mode::Initialize,
                &journal,
                &created,
                |standing| {
                    assert!(standing.ids.contains_key("assets"));
                    assert!(!drive_panics, "injected failure in the drive");
                },
            )
        }));
        assert!(failed.is_err(), "{stamp}: the lane still fails");
        assert_eq!(
            journal.count() - creates,
            1,
            "{stamp}: the create stays counted"
        );
        let attempted = journey
            .workspace
            .state
            .lock()
            .unwrap()
            .document_delete_attempts[deletes..]
            .to_vec();
        assert_eq!(
            attempted.len(),
            1,
            "{stamp}: exactly the created slot is retired"
        );
        assert!(
            journey.workspace.held.document_titled(&title).is_none(),
            "{stamp}: nothing of the invocation's set survives"
        );
        assert_eq!(created.len(), 0);
    }
    assert_eq!(
        journey.workspace.held.document_titled(&ci.titles["assets"]),
        Some(ci.ids["assets"].clone()),
        "the standing CI slot survives"
    );
    assert_eq!(
        journey
            .workspace
            .held
            .document_titled("Unrelated synthetic document"),
        Some(unrelated)
    );
}

#[test]
fn asset_reset_must_store_its_current_marker_before_copying_and_recovers() {
    let journey = Journey::with_clients(0);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let endpoint = format!("{}/0/graphql", journey.workspace.endpoint);
    let journal = document_lifecycle::journal::Journal::new("assets", "initialize");
    journey.workspace.state.lock().unwrap().document_journal = Some(journal.clone());
    let standing = runtime
        .block_on(document_lifecycle::prepare(
            &endpoint,
            "fixture-key",
            "ENG",
            "team",
            Scope::ci("ENG").unwrap(),
            Mode::Initialize,
            &[Slot::Assets],
        ))
        .unwrap();
    journal.begin("ordinary");
    journey.workspace.state.lock().unwrap().ignore_asset_reset = true;
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reset_asset_slot(
            &runtime,
            &endpoint,
            "fixture-key",
            &standing,
            "current-run",
            1,
        )
    }));
    assert!(
        refused.is_err(),
        "an acknowledged reset that stores nothing must refuse"
    );
    assert_eq!(
        journal.count(),
        0,
        "failed reset never bootstraps a replacement"
    );
    journey.workspace.state.lock().unwrap().ignore_asset_reset = false;
    reset_asset_slot(
        &runtime,
        &endpoint,
        "fixture-key",
        &standing,
        "recovered-run",
        21,
    );
    assert_eq!(
        journey
            .workspace
            .held
            .long_form("documents", &standing.ids["assets"])
            .as_deref(),
        Some("Reset for invocation recovered-run")
    );
    assert_eq!(journal.count(), 0);
}

/// Two ordinary passes over one standing slot, the second with Linear's single read of the slot
/// answering, once, from a replica still holding what the first pass wrote — after the reset
/// was read back. The previous pass's content is what this pass would write were the content not
/// run-specific, so the copy would find nothing to change and leave the reset marker in place,
/// which is the `onetaskgraph.assets` read one live ordinary pass panicked on.
#[test]
fn an_ordinary_asset_pass_writes_over_a_replica_still_holding_the_previous_pass() {
    let journey = Journey::with_clients(0);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let endpoint = format!("{}/0/graphql", journey.workspace.endpoint);
    let journal = document_lifecycle::journal::Journal::new("assets", "initialize");
    journey.workspace.state.lock().unwrap().document_journal = Some(journal.clone());
    let standing = runtime
        .block_on(document_lifecycle::prepare(
            &endpoint,
            "fixture-key",
            "ENG",
            "team",
            Scope::ci("ENG").unwrap(),
            Mode::Initialize,
            &[Slot::Assets],
        ))
        .unwrap();
    let slot = standing.ids["assets"].clone();
    let read = |_: &str, url: &str| {
        let response = runtime
            .block_on(
                reqwest::Client::new()
                    .get(url)
                    .header("Authorization", "fixture-key")
                    .send(),
            )
            .unwrap();
        assert!(response.status().is_success());
    };
    for pass in ["previous-pass", "lagging-pass"] {
        let previous = journey.workspace.held.long_form("documents", &slot);
        reset_asset_slot(&runtime, &endpoint, "fixture-key", &standing, pass, 21);
        if pass == "lagging-pass" {
            let previous = previous.unwrap();
            assert!(previous.contains("onetaskgraph.assets"), "{previous}");
            // Armed once the reset has been read back, as the live pass saw it.
            journey.workspace.state.lock().unwrap().stale_document =
                Some((slot.clone(), previous, 1));
        }
        let origins = asset_origins(&journey, |kind| {
            if kind == "document" {
                standing.titles["assets"].clone()
            } else {
                format!(
                    "{}123-456-{pass}",
                    onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX
                )
            }
        });
        let document = origins
            .iter()
            .find(|(kind, _, _)| *kind == "document")
            .unwrap()
            .clone();
        assert_eq!(
            document.1, "notes:onetaskgraph-live-cleanup-retained-v1-ci-eng-assets",
            "{pass}: the origin has the id every live pass's origin has"
        );
        drive_asset_copy_recopy(
            &journey,
            std::slice::from_ref(&document),
            Mode::Ordinary,
            read,
            || journey.workspace.counts().0,
        );
        // Each live pass copies from a sandbox of its own, so its origin has the same id as the
        // previous pass's; retiring this one's file lets the next pass's origin take that id too.
        let output = journey
            .sandbox
            .command()
            .args(["--json", "document", "show", &document.1])
            .output()
            .unwrap();
        let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
        std::fs::remove_file(
            shown["items"][0]["item"]["location"]["path"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        journey
            .workspace
            .state
            .lock()
            .unwrap()
            .stale_document
            .as_ref()
            .map(|held| held.2),
        Some(0),
        "the lagging replica answered the copy's read"
    );
    assert_eq!(journal.count(), 1, "the passes reuse the one slot");
}

#[test]
fn live_asset_forwarding_transport_counts_created_then_deleted_documents() {
    let journey = Journey::with_clients(0);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let upstream = format!("{}/0/graphql", journey.workspace.endpoint);
    // The synthetic upstream checks the wire shape independently of the real observer.
    journey.workspace.state.lock().unwrap().document_journal =
        Some(document_lifecycle::journal::Journal::new("assets", "fresh"));
    let journal = document_lifecycle::journal::Journal::new("assets", "fresh");
    journal.journey("live-asset-forwarding-transport");
    let (endpoint, observed) = live_observer("fixture-key".into(), journal.clone(), upstream);
    let created=runtime.block_on(document_lifecycle::send(&endpoint,"fixture-key",onetaskgraph_linear::graphql::DOCUMENT_CREATE,json!({"input":{"title":"Synthetic forwarded document","content":"Synthetic body","teamId":"team"}}))).unwrap();
    let id = created["documentCreate"]["document"]["id"]
        .as_str()
        .unwrap();
    assert_eq!(
        journal.count(),
        1,
        "the live forwarding path observes the actual create acknowledgement"
    );
    assert_eq!(
        observed.lock().unwrap().items,
        vec![("document", id.to_owned())]
    );
    assert!(cleanup_asset_items(
        &runtime,
        &endpoint,
        "fixture-key",
        &[("document", id.to_owned())]
    ));
    assert!(
        journey
            .workspace
            .held
            .document_titled("Synthetic forwarded document")
            .is_none()
    );
    assert_eq!(
        journal.count(),
        1,
        "deletion never erases a forwarded create"
    );
    let directory = journey.sandbox.subdirectory("forwarded-journal");
    journal.finish_in(&directory).unwrap();
    let recorded: Value = serde_json::from_slice(
        &std::fs::read(directory.join("linear-live-documents-fresh.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(recorded["operations"].as_array().unwrap().len(), 1);
    assert_eq!(recorded["completed"], true);
}
