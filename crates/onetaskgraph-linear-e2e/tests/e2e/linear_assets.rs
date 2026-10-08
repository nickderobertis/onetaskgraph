//! Asset copies through the binary and Linear's documented HTTP boundaries.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use onetaskgraph_e2e_support::{clock::SimulatedClock, images};
use onetaskgraph_plugin_api::asset_sha256;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This task explicitly assigns comparison with the cited published page to the manager's audit after this node settles, outside this node's acceptance bar. The required offline test reconciles this model's number and window with that sourced budget description; fetching the changing page in a measuring journey would defeat its deterministic cached telemetry.
const REQUEST_LIMIT: usize = 2_500;
const REQUEST_WINDOW: Duration = Duration::from_secs(3_600);
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

use crate::fixtures::{LinearWorkspace, document, linear_workspace};

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
    requests: usize,
    tokens: f64,
    last_refill: Duration,
    fail: Option<(&'static str, usize, u16)>,
    fault: Option<&'static str>,
    hint: Option<&'static str>,
    network_time: Duration,
    refused_names: std::collections::BTreeSet<String>,
}

struct Workspace {
    endpoint: String,
    state: Arc<Mutex<State>>,
    held: LinearWorkspace,
    clock: SimulatedClock,
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

    fn counts(&self) -> (usize, usize, usize) {
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
        let mut upstream = TcpStream::connect(backend).unwrap();
        let original = String::from_utf8_lossy(&bytes[..split]).replacen(path, "/graphql", 1);
        upstream.write_all(original.as_bytes()).unwrap();
        upstream.write_all(&bytes[split..]).unwrap();
        let mut response = Vec::new();
        upstream.read_to_end(&mut response).unwrap();
        stream.write_all(&response).unwrap();
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

struct Journey {
    config: Value,
    sandbox: Sandbox,
    workspace: Workspace,
}

impl Journey {
    fn new() -> Self {
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
            assert_eq!(upload.verified, 1);
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
            (before.0 + 1, before.1 + 1, before.2 + 1)
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
        for stage in ["mutation", "PUT", "verifying read"] {
            for (times, status, succeeds) in [(1, 403, false), (1, 429, true), (31, 429, false)] {
                let journey = Journey::new();
                let (id, images) = journey.create(kind, 1);
                journey.workspace.state.lock().unwrap().fail = Some((stage, times, status));
                let result = journey.copy(kind, &id, succeeds);
                if succeeds {
                    assert_eq!(journey.workspace.counts(), (1, 1, 1));
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
                    if stage == "verifying read" {
                        assert!(
                            said.contains(&format!("{}/0/asset/0", journey.workspace.endpoint)),
                            "{said}"
                        );
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

fn record_workload(budget: &str, value: f64, sizes: &[usize], copies: usize) {
    crate::telemetry::record_assets(budget, value, sizes, copies);
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
    assert_eq!(journey.workspace.counts(), (24, 24, 24));
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

#[test]
fn measure_three_concurrent_project_asset_copies() {
    let journey = Journey::with_clients(3);
    let (sizes, documents) = project_workload(&journey);
    let real_start = std::time::Instant::now();
    let mut refused_clients = [false; 3];
    for (kind, id) in std::iter::once(("project", "notes:launch"))
        .chain(documents.iter().map(|id| ("document", id.as_str())))
    {
        let mut children = Vec::new();
        for client in 0..3 {
            let destination = if client == 0 {
                "dest".to_owned()
            } else {
                format!("dest{client}")
            };
            children.push(
                journey
                    .sandbox
                    .subprocess(onetaskgraph_e2e_support::binary())
                    .current_dir(journey.sandbox.project())
                    .envs(journey.workspace.clock.client_env(client))
                    .args(["--json", kind, "copy", id, "--to", &destination])
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
                eprintln!("{}", stderr(&output));
            }
        }
    }
    let elapsed = real_start.elapsed();
    eprintln!(
        "three concurrent copies real duration: {:.3}s",
        elapsed.as_secs_f64()
    );
    assert!(elapsed < Duration::from_secs(60));
    assert_eq!(journey.workspace.counts(), (72, 72, 72));
    let sizes: Vec<_> = sizes.iter().copied().cycle().take(72).collect();
    record_workload(
        "linear-concurrent-copies-refused",
        refused_clients.iter().filter(|refused| **refused).count() as f64,
        &sizes,
        3,
    );
}

#[test]
fn loopback_request_limit_and_budget_description_agree() {
    let budgets: Value = serde_norway::from_str(include_str!("../../budgets.yaml")).unwrap();
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
    assert_eq!(journey.workspace.counts(), (2, 2, 2));
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
        assert_eq!(upload.verified, 1);
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

/// A forwarding observer changes only the signed PUT URL, keeping Linear's asset URL intact.
/// Thus the plugin's own verifying GET reaches Linear directly with its source credential.
fn live_observer(key: String) -> (String, Arc<Mutex<LiveUploads>>) {
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
                            .post("https://api.linear.app/graphql")
                            .header("Authorization", &key)
                            .json(&request)
                            .send(),
                    )
                    .unwrap();
                let status = response.status().as_u16();
                let mut body: Value = runtime.block_on(response.json()).unwrap();
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
    let journey = Journey::new();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros();
    let mut origins = Vec::new();
    for kind in ["task", "document"] {
        let title = format!("onetaskgraph disposable asset {kind} {stamp}");
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
        let text: String = text
            .lines()
            .filter(|line| !line.starts_with("project:"))
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(path, text).unwrap();
        origins.push((kind, id));
    }
    let (endpoint, observed) = live_observer(key.clone());
    journey
        .sandbox
        .secrets_file(&format!("LINEAR_API_KEY={key}\n"));
    journey.sandbox.project_document(&document(&json!({
        "notes":{"plugin":"local-md","config":{"root":journey.sandbox.project().join("notes")}},
        "dest":{"plugin":"linear","config":{"endpoint":endpoint,"team":team,"status_mapping":{"todo":state_name}}}
    })));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for (kind, id) in origins {
            let run = || {
                journey
                    .sandbox
                    .command()
                    .args(["--json", kind, "copy", &id, "--to", "dest"])
                    .output()
                    .unwrap()
            };
            let output = run();
            assert!(output.status.success(), "{}", stderr(&output));
            let report: Value = serde_json::from_str(&stdout(&output)).unwrap();
            let dest = report["items"][0]["destination"].as_str().unwrap();
            let output = journey
                .sandbox
                .command()
                .args(["--json", kind, "show", dest])
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", stderr(&output));
            let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
            let item = &shown["items"][0]["item"];
            let content = item["content"].as_str().unwrap();
            assert!(!content.contains("](./"));
            let uploads = item["metadata"]["onetaskgraph.assets"].as_object().unwrap();
            assert_eq!(uploads.len(), 2);
            for (name, upload) in uploads {
                let url = upload["url"].as_str().unwrap();
                assert!(content.contains(url));
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
            }
            let before = observed.lock().unwrap().signed.len();
            let output = run();
            assert!(output.status.success(), "{}", stderr(&output));
            assert_eq!(
                observed.lock().unwrap().signed.len(),
                before,
                "unchanged copy sends no fileUpload"
            );
        }
    }));
    for observation in &observed.lock().unwrap().observations {
        eprintln!("{observation}");
    }
    let items = observed.lock().unwrap().items.clone();
    let mut cleanup_failed = false;
    for (kind, id) in items {
        let mutation = if kind == "issue" {
            "issueDelete"
        } else {
            "documentDelete"
        };
        let response: Value = runtime.block_on(async {
            client.post("https://api.linear.app/graphql").header("Authorization", &key)
                .json(&json!({"query":format!("mutation($id:String!){{{mutation}(id:$id){{success}}}}"),"variables":{"id":id}}))
                .send().await.unwrap().json().await.unwrap()
        });
        let removed = response["data"][mutation]["success"] == true;
        eprintln!("live disposable {kind} {id}: removed={removed}, response={response}");
        cleanup_failed |= !removed;
    }
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
            (before.0 + 1, before.1 + 1, before.2 + 1)
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
        ("disconnect-read", "verifying read"),
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
        if fault == "disconnect-read" {
            assert!(said.contains(&journey.workspace.endpoint));
        }
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
        assert_eq!(journey.workspace.counts(), (1, 1, 1));
    }
    let journey = Journey::new();
    let (id, _) = journey.create("task", 1);
    {
        let mut state = journey.workspace.state.lock().unwrap();
        state.fail = Some(("mutation", 1, 429));
        state.fault = Some("mutation-http429");
    }
    journey.copy("task", &id, true);
    assert_eq!(journey.workspace.counts(), (1, 1, 1));
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
    assert_eq!(journey.workspace.counts(), (1, 1, 1));
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
        assert_eq!(pictured.workspace.counts(), (1, 1, 1));
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
    for (fault, stage) in [
        ("redirect-mutation", "mutation"),
        ("redirect-PUT", "PUT"),
        ("redirect-read", "verifying read"),
    ] {
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
