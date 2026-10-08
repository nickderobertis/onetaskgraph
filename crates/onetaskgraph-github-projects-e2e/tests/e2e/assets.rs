//! Copies driven through the binary, including refusal, re-copy and virtual-time workloads.
use super::asset_board::{Board, Call, Refusal, Stage};
use onetaskgraph_e2e_support::{
    binary,
    clock::SimulatedClock,
    common::{Sandbox, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_board},
    images,
};
use onetaskgraph_plugin_api::asset_sha256;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Child, Output, Stdio},
    time::{Duration, Instant},
};

struct Plan {
    sandbox: Sandbox,
    root: PathBuf,
    board: Board,
    github: GitHubBoardFields,
    clock: SimulatedClock,
    client: usize,
    config: Value,
}
impl Plan {
    fn new(clock: &SimulatedClock, client: usize, shared: Option<&Board>) -> Self {
        let sandbox = Sandbox::new();
        let (mut config, github) = github_projects_with_board(&sandbox);
        let board = Board::new(config["endpoint"].as_str().unwrap(), clock, client, shared);
        config["endpoint"] = json!(board.endpoint);
        config["pacing"] = json!({});
        let root = sandbox.subdirectory("notes");
        for directory in ["projects", "tasks", "documents"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        sandbox.project_document(&document(&json!({
            "notes": {"plugin":"local-md","config":{"root":root}},
            "board": {"plugin":"github-projects","config": config}
        })));
        Self {
            sandbox,
            root,
            board,
            github,
            clock: clock.clone(),
            client,
            config,
        }
    }
    fn author(
        &self,
        kind: &str,
        id: &str,
        parent: bool,
        sizes: &[usize],
        seed: u64,
    ) -> Vec<Vec<u8>> {
        let mut body = String::new();
        let directory = self.root.join(format!("{kind}s/{id}.assets"));
        std::fs::create_dir_all(&directory).unwrap();
        let bytes: Vec<_> = sizes
            .iter()
            .enumerate()
            .map(|(index, size)| {
                let bytes = images::png(seed + index as u64, *size);
                std::fs::write(directory.join(format!("shot-{index}.png")), &bytes).unwrap();
                body.push_str(&format!("![Written alt {index}](./shot-{index}.png)\n"));
                bytes
            })
            .collect();
        std::fs::write(
            self.root.join(format!("{kind}s/{id}.md")),
            format!(
                "---\ntitle: {id}\n{}{}---\n{body}",
                if kind == "task" || kind == "project" {
                    "status: todo\n"
                } else {
                    ""
                },
                if parent { "project: P\n" } else { "" }
            ),
        )
        .unwrap();
        bytes
    }
    fn spawn(&self, kind: &str, id: &str) -> Child {
        self.sandbox
            .subprocess(binary())
            .args([
                kind,
                "copy",
                &format!("notes:{id}"),
                "--to",
                "board",
                "--json",
            ])
            .envs(self.clock.client_env(self.client))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
    fn copy(&self, kind: &str, id: &str) -> Value {
        let output = self.spawn(kind, id).wait_with_output().unwrap();
        success(&output)
    }
    fn assert_record(&self, report: &Value, source: &str, bytes: &[Vec<u8>]) -> Value {
        let destination = report["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["source"] == source)
            .unwrap()["destination"]
            .as_str()
            .unwrap();
        let id = destination.strip_prefix("board:").unwrap();
        let body = self.github.body(id);
        let body = body.as_str().unwrap();
        assert!(!body.contains("](./"), "unrewritten body: {body}");
        let output = self
            .sandbox
            .command()
            .args([
                if source.contains(":D") {
                    "document"
                } else {
                    "task"
                },
                "show",
                destination,
                "--json",
            ])
            .envs(self.clock.client_env(self.client))
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
        let record = shown["items"][0]["item"]["metadata"]["onetaskgraph.assets"].clone();
        for (index, bytes) in bytes.iter().enumerate() {
            let upload = &record[format!("shot-{index}.png")];
            assert_eq!(upload["sha256"], asset_sha256(bytes));
            let url = upload["url"].as_str().unwrap();
            assert!(
                body.contains(&format!("![Written alt {index}]({url})")),
                "{body}"
            );
            let path = url
                .strip_prefix(self.board.endpoint.trim_end_matches("/graphql"))
                .unwrap();
            assert!(self.board.calls().iter().any(|call| {
                call.path == path
                    && call.status == 200
                    && call
                        .headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer test-token")
            }));
        }
        record
    }
    fn bundle_outputs(&self) -> Vec<Output> {
        let lease = self.clock.lease(self.client);
        let mut outputs = vec![self.spawn("project", "P").wait_with_output().unwrap()];
        for id in ["D-0", "D-1", "D-2"] {
            lease.wait_for_detach();
            outputs.push(self.spawn("document", id).wait_with_output().unwrap());
        }
        outputs
    }
    fn bundle(&self) -> Vec<Value> {
        self.bundle_outputs().iter().map(success).collect()
    }
    fn long(&self) -> Vec<Vec<u8>> {
        self.author("project", "P", false, &[], 0);
        for index in 0..7 {
            self.author("task", &format!("T-{index}"), true, &[], 0);
        }
        let mut bytes = Vec::new();
        for index in 0..3 {
            bytes.extend(self.author(
                "document",
                &format!("D-{index}"),
                true,
                if index < 2 { &[480_000; 12] } else { &[] },
                (index * 20) as u64 + self.client as u64 * 100,
            ));
        }
        bytes
    }
}
fn success(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        stdout(output),
        stderr(output)
    );
    serde_json::from_str(&stdout(output)).unwrap()
}
fn uploads(calls: &[Call]) -> Vec<&Call> {
    calls
        .iter()
        .filter(|call| call.path.starts_with("/user-attachments/assets?"))
        .collect()
}
fn attachment_reads(calls: &[Call]) -> usize {
    calls
        .iter()
        .filter(|call| call.path.starts_with("/user-attachments/assets/"))
        .count()
}
fn record(
    budget: &str,
    value: f64,
    workload: super::budget_runner::Workload,
    copies: &[Vec<Vec<u8>>],
) {
    let workload = workload.label();
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(onetaskgraph_e2e_support::telemetry::file_in(&directory,budget), serde_json::to_vec(&json!({
        "budget":budget,"value":value,"detail":"Requests and simulated time observed by the real binary's loopback endpoint",
        "workload":workload,"repetitions":if workload=="unchanged" {3} else {1},"images":copies.iter().map(|copy|copy.iter().map(Vec::len).collect::<Vec<_>>()).collect::<Vec<_>>()
    })).unwrap()).unwrap();
}

#[test]
fn task_and_document_assets_are_uploaded_verified_reused_and_replaced() {
    let clock = SimulatedClock::start(1);
    let plan = Plan::new(&clock, 0, None);
    let docs = plan.author(
        "document",
        "D",
        false,
        &[55_000, 100_000, 200_000, 300_000, 400_000, 480_000],
        10,
    );
    let tasks = plan.author("task", "T", false, &[50_500, 480_000], 30);
    let mut new_requests = 0;
    let mut records = Vec::new();
    for (kind, id, bytes) in [("document", "D", &docs), ("task", "T", &tasks)] {
        let before = plan.board.calls().len();
        let report = plan.copy(kind, id);
        let calls = plan.board.calls();
        let calls = &calls[before..];
        assert_eq!(uploads(calls).len(), bytes.len());
        assert_eq!(attachment_reads(calls), bytes.len());
        for (index, call) in uploads(calls).iter().enumerate() {
            assert!(call.path.contains(&format!("name=shot-{index}.png")));
            assert!(call.path.contains("content_type=image%2Fpng"));
            assert!(call.path.contains("repository_id=123456"));
            assert_eq!(&call.bytes, &bytes[index], "the server receives every byte");
        }
        new_requests += uploads(calls).len() + attachment_reads(calls);
        records.push(plan.assert_record(&report, &format!("notes:{id}"), bytes));
    }
    assert_eq!(
        plan.board
            .calls()
            .iter()
            .filter(|call| call.path.starts_with("/repos/"))
            .count(),
        2,
        "one repository read per fresh binary instance"
    );
    record(
        "github-asset-requests-per-new-asset",
        new_requests as f64 / 8.0,
        super::budget_runner::Workload::New,
        &[docs.clone(), tasks.clone()],
    );
    let before = plan.board.calls().len();
    for _ in 0..3 {
        for (kind, id, bytes) in [("document", "D", &docs), ("task", "T", &tasks)] {
            let report = plan.copy(kind, id);
            plan.assert_record(&report, &format!("notes:{id}"), bytes);
        }
    }
    let calls = plan.board.calls();
    assert!(uploads(&calls[before..]).is_empty());
    assert_eq!(attachment_reads(&calls[before..]), 0);
    record(
        "github-asset-requests-per-unchanged-asset",
        0.0,
        super::budget_runner::Workload::Unchanged,
        &[docs.clone(), tasks.clone()],
    );
    for (position, kind, id, mut bytes) in [(0, "document", "D", docs), (1, "task", "T", tasks)] {
        bytes[0] = images::png(90 + position as u64, 250_000);
        std::fs::write(
            plan.root.join(format!("{kind}s/{id}.assets/shot-0.png")),
            &bytes[0],
        )
        .unwrap();
        let before = plan.board.calls().len();
        let report = plan.copy(kind, id);
        let calls = plan.board.calls();
        assert_eq!(uploads(&calls[before..]).len(), 1);
        assert_eq!(attachment_reads(&calls[before..]), 1);
        let changed = plan.assert_record(&report, &format!("notes:{id}"), &bytes);
        assert_ne!(changed["shot-0.png"], records[position]["shot-0.png"]);
        for index in 1..bytes.len() {
            assert_eq!(
                changed[format!("shot-{index}.png")],
                records[position][format!("shot-{index}.png")]
            );
        }
    }
    // An unpublished attachment is private: anonymous 404 beside an authenticated 200.
    let path = plan
        .board
        .calls()
        .iter()
        .find(|call| call.path.starts_with("/user-attachments/assets/"))
        .unwrap()
        .path
        .clone();
    let address = plan
        .board
        .endpoint
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    use std::io::{Read, Write};
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");
}

#[test]
fn refused_uploads_and_broken_verified_urls_never_write_an_issue_body() {
    for (stage, status) in [
        (Stage::Upload, 401),
        (Stage::Upload, 403),
        (Stage::Upload, 422),
        (Stage::Read, 404),
        (Stage::Read, 500),
        (Stage::Repository, 404),
        (Stage::Repository, 500),
    ] {
        let clock = SimulatedClock::start(1);
        let plan = Plan::new(&clock, 0, None);
        plan.author("document", "D", false, &[480_000], 1);
        plan.board.refuse(Refusal {
            stage,
            status,
            headers: String::new(),
            body: "refused".into(),
            remaining: usize::MAX,
        });
        let output = plan.spawn("document", "D").wait_with_output().unwrap();
        assert!(!output.status.success());
        let message = format!("{}{}", stdout(&output), stderr(&output));
        assert!(
            message.contains("shot-0.png") && message.contains(&status.to_string()),
            "{message}"
        );
        if stage == Stage::Upload {
            assert!(
                message.contains("token type may not be accepted"),
                "{message}"
            );
        } else if stage == Stage::Read {
            assert!(message.contains("/user-attachments/assets/1"), "{message}");
        }
        assert!(
            !plan
                .github
                .documents()
                .iter()
                .any(|query| query.contains("createIssue(input:"))
        );
    }
}

#[test]
fn asset_requests_take_mutation_slots_and_each_limiter_recovers_or_exhausts_its_budget() {
    #[derive(Clone, Copy)]
    enum Hint {
        Server,
        Missing,
        Invalid,
    }
    for stage in [Stage::Upload, Stage::Read, Stage::Repository] {
        for primary in [false, true] {
            for hint in [Hint::Server, Hint::Missing, Hint::Invalid] {
                for persistent in [false, true] {
                    let clock = SimulatedClock::start(1);
                    let plan = Plan::new(&clock, 0, None);
                    plan.author("document", "D", false, &[480_000, 480_000], 1);
                    let headers = match (primary, hint) {
                        (true, Hint::Server) => format!(
                            "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: {}\r\n",
                            chrono::Utc::now().timestamp() + 2
                        ),
                        (false, Hint::Server) => "Retry-After: 1\r\n".into(),
                        (true, Hint::Missing) => "x-ratelimit-remaining: 0\r\n".into(),
                        (false, Hint::Missing) => String::new(),
                        (true, Hint::Invalid) => {
                            "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: not-a-time\r\n".into()
                        }
                        (false, Hint::Invalid) => "Retry-After: not-a-time\r\n".into(),
                    };
                    plan.board.refuse(Refusal {
                        stage,
                        status: 403,
                        headers,
                        body: if primary {
                            "rate limit exceeded"
                        } else {
                            "You have exceeded a secondary rate limit"
                        }
                        .into(),
                        remaining: if persistent {
                            usize::MAX
                        } else if matches!(hint, Hint::Server) {
                            1
                        } else {
                            3
                        },
                    });
                    let output = plan.spawn("document", "D").wait_with_output().unwrap();
                    let calls = plan.board.calls();
                    let attempts: Vec<_> = calls
                        .iter()
                        .filter(|call| match stage {
                            Stage::Upload => call.path.starts_with("/user-attachments/assets?"),
                            Stage::Read => call.path.starts_with("/user-attachments/assets/"),
                            Stage::Repository => call.path.starts_with("/repos/"),
                        })
                        .collect();
                    assert!(attempts.len() > 1, "no backoff attempt");
                    assert!(
                        attempts[1].at.saturating_sub(attempts[0].at) >= Duration::from_secs(1)
                    );
                    if !matches!(hint, Hint::Server) {
                        assert!(attempts.len() >= 4);
                        let latency =
                            Duration::from_millis(if stage == Stage::Upload { 800 } else { 200 });
                        for (pair, seconds) in attempts.windows(2).take(3).zip([1, 2, 4]) {
                            assert_eq!(
                                pair[1].at - pair[0].at,
                                latency + Duration::from_secs(seconds),
                                "hintless retries did not double the backoff"
                            );
                        }
                    }
                    if persistent {
                        assert!(!output.status.success());
                        let message = format!("{}{}", stdout(&output), stderr(&output));
                        for text in [
                            "shot-0.png",
                            if primary {
                                "primary API rate limit"
                            } else {
                                "secondary rate limit"
                            },
                            match stage {
                                Stage::Upload => "upload",
                                Stage::Read => "verification",
                                Stage::Repository => "reading repository id",
                            },
                        ] {
                            assert!(message.contains(text), "{message}");
                        }
                        assert!(
                            clock.now() < Duration::from_secs(125),
                            "past retry budget: {:?}",
                            clock.now()
                        );
                        assert!(
                            !plan
                                .github
                                .documents()
                                .iter()
                                .any(|query| query.contains("createIssue(input:"))
                        );
                    } else {
                        let report = success(&output);
                        plan.assert_record(
                            &report,
                            "notes:D",
                            &[images::png(1, 480_000), images::png(2, 480_000)],
                        );
                        let creating: Vec<_> = calls.iter().filter(|call| call.creating).collect();
                        for pair in creating.windows(2) {
                            assert!(
                                pair[1].at.saturating_sub(pair[0].at) >= Duration::from_millis(750)
                            );
                        }
                        let last_upload = uploads(&calls).last().unwrap().at;
                        let read = calls
                            .iter()
                            .find(|call| {
                                call.at >= last_upload
                                    && call.path.starts_with("/user-attachments/assets/")
                            })
                            .unwrap();
                        assert_eq!(
                            read.at - last_upload,
                            Duration::from_millis(800),
                            "verification follows upload latency with no pacing slot"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn measure_single_and_three_concurrent_project_copies_in_virtual_time() {
    let clock = SimulatedClock::start(1);
    let plan = Plan::new(&clock, 0, None);
    let bytes = plan.long();
    let started = clock.now();
    let reports = plan.bundle();
    let elapsed = clock.now() - started;
    assert_eq!(
        reports
            .iter()
            .map(|report| report["items"].as_array().unwrap().len())
            .sum::<usize>(),
        11
    );
    assert_eq!(uploads(&plan.board.calls()).len(), 24);
    for (call, bytes) in uploads(&plan.board.calls()).iter().zip(&bytes) {
        assert_eq!(&call.bytes, bytes);
    }
    let calls = plan.board.calls();
    let upload_count = uploads(&calls).len();
    let reads = attachment_reads(&calls);
    let other = calls.len() - upload_count - reads;
    let holds = Duration::from_millis((upload_count * 800 + (reads + other) * 200) as u64);
    eprintln!(
        "single bundle: {} uploads ({}s holds), {} verifying reads ({}s holds), {} other requests ({}s holds), {:?} pacing, {} limiter refusals; total {:?}",
        upload_count,
        upload_count as f64 * 0.8,
        reads,
        reads as f64 * 0.2,
        other,
        other as f64 * 0.2,
        elapsed.saturating_sub(holds),
        calls.iter().filter(|call| call.status == 403).count(),
        elapsed
    );
    record(
        "github-asset-copy-seconds",
        elapsed.as_secs_f64(),
        super::budget_runner::Workload::Single,
        &[bytes],
    );
    let real = Instant::now();
    let clock = SimulatedClock::start(3);
    let first = Plan::new(&clock, 0, None);
    first.board.enforce_limit();
    let second = Plan::new(&clock, 1, Some(&first.board));
    let third = Plan::new(&clock, 2, Some(&first.board));
    let plans = [first, second, third];
    let bytes: Vec<_> = plans.iter().map(Plan::long).collect();
    let results = std::thread::scope(|scope| {
        let bundles: Vec<_> = plans
            .iter()
            .map(|plan| {
                let clock = &clock;
                scope.spawn(move || {
                    let started = clock.now();
                    let outputs = plan.bundle_outputs();
                    (outputs, clock.now() - started)
                })
            })
            .collect();
        bundles
            .into_iter()
            .map(|bundle| bundle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let elapsed = results.iter().map(|(_, elapsed)| *elapsed).max().unwrap();
    assert_eq!(results.len(), 3);
    let refused = results
        .iter()
        .filter(|(bundle, _)| bundle.iter().any(|output| !output.status.success()))
        .count();
    for (bundle, _) in &results {
        for output in bundle {
            success(output);
        }
    }
    let calls = plans[0].board.calls();
    let starts: Vec<_> = (0..3)
        .map(|client| calls.iter().find(|call| call.client == client).unwrap().at)
        .collect();
    assert!(
        starts
            .iter()
            .max()
            .unwrap()
            .saturating_sub(*starts.iter().min().unwrap())
            < Duration::from_secs(1)
    );
    for client in 0..3 {
        let delivered: Vec<_> = uploads(&calls)
            .into_iter()
            .filter(|call| call.client == client && call.status == 201)
            .collect();
        assert_eq!(delivered.len(), 24);
        // Asset ordering belongs to each record, not project iteration order.
        for bytes in &bytes[client] {
            assert!(delivered.iter().any(|call| call.bytes == *bytes));
        }
    }
    assert!(
        calls.iter().any(|call| call.status == 403),
        "the concurrent workload never met the shared limiter"
    );
    eprintln!(
        "three concurrent copies: real {:?}, simulated {:?}",
        real.elapsed(),
        elapsed
    );
    assert!(real.elapsed() < Duration::from_secs(60));
    record(
        "github-concurrent-copies-refused",
        refused as f64,
        super::budget_runner::Workload::Concurrent,
        &bytes,
    );
    record(
        "github-concurrent-copy-seconds",
        elapsed.as_secs_f64(),
        super::budget_runner::Workload::Concurrent,
        &bytes,
    );
}

#[test]
fn hosted_asset_reads_and_rendering_updates_cross_the_real_source_boundary() {
    use onetaskgraph_plugin_api::{
        AssetName, AssetPayload, AssetUploads, AssetWrite, NativeId, SecretResolver, SourceName,
        SourcePlugin, body_digest,
    };
    struct Secrets;
    impl SecretResolver for Secrets {
        fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
            (variable == "GITHUB_PROJECTS_FIXTURE_TOKEN").then(|| "test-token".into())
        }
    }
    for (kind, id) in [("task", "T"), ("document", "D")] {
        let clock = SimulatedClock::start(1);
        let plan = Plan::new(&clock, 0, None);
        let bytes = plan.author(kind, id, false, &[480_000], 65);
        let report = plan.copy(kind, id);
        let recorded = plan.assert_record(&report, &format!("notes:{id}"), &bytes);
        let target = report["items"][0]["destination"]
            .as_str()
            .unwrap()
            .strip_prefix("board:")
            .unwrap();
        let id = NativeId(target.to_owned());
        let environment = onetaskgraph_core::Environment::from_pairs(clock.client_env(0));
        let clock = onetaskgraph_core::process_clock(&environment).unwrap();
        let source = onetaskgraph_github_projects::Plugin
            .build_with_clock(
                &SourceName::new("board").unwrap(),
                &plan.config,
                &Secrets,
                clock,
            )
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let list=if kind=="task" {source.task_assets(&id).await} else {source.document_assets(&id).await}.unwrap();
            assert_eq!(list.len(),1);assert_eq!(list[0].sha256,asset_sha256(&bytes[0]));assert!(list[0].path.is_none());
            let image=if kind=="task" {source.task_asset(&id,&list[0].name).await} else {source.document_asset(&id,&list[0].name).await}.unwrap();
            assert_eq!(image.as_ref(),Some(&bytes[0]));
            let missing=NativeId("no-such-issue".into());let absent=AssetName::new("absent.png").unwrap();
            if kind=="task" {
                assert!(source.task_assets(&missing).await.unwrap().is_empty());
                assert!(source.task_asset(&missing,&absent).await.unwrap().is_none());
                assert!(source.task_asset(&id,&absent).await.unwrap().is_none());
                assert!(source.document_assets(&id).await.unwrap().is_empty());
            } else {
                assert!(source.document_assets(&missing).await.unwrap().is_empty());
                assert!(source.document_asset(&missing,&absent).await.unwrap().is_none());
                assert!(source.document_asset(&id,&absent).await.unwrap().is_none());
                assert!(source.task_assets(&id).await.unwrap().is_empty());
            }
            let content="![Render kept](./shot-0.png)\n![Render new](./next.png)\n";
            let provenance=json!({"template":"unchanged template","digest":"kept digest","answers_digest":"kept answers","body_digest":body_digest(content)});
            let existing_uploads:AssetUploads=serde_json::from_value(recorded).unwrap();
            let mut reused=AssetPayload::of(AssetName::new("shot-0.png").unwrap(),bytes[0].clone());reused.bytes=None;
            let next=images::png(99,480_000);
            let write=AssetWrite {assets:vec![reused,AssetPayload::of(AssetName::new("next.png").unwrap(),next.clone())],recorded_assets:Some(existing_uploads)};
            let before=plan.board.calls().len();
            let written=if kind=="task" {source.set_task_rendering_with_assets(&id,content,&provenance,&Default::default(),&write).await} else {source.set_document_rendering_with_assets(&id,content,&provenance,&Default::default(),&write).await}.unwrap().unwrap();
            let calls=plan.board.calls();assert_eq!(uploads(&calls[before..]).len(),1);assert_eq!(attachment_reads(&calls[before..]),1);
            let body=plan.github.body(&id.0);let body=body.as_str().unwrap();
            let rewritten=written.content.as_deref().unwrap();assert!(body.contains(rewritten));assert!(!rewritten.contains("](./"));
            source.end_command().await.unwrap();
            let (metadata,stored_content)=if kind=="task" {let item=source.get_task(&id).await.unwrap().unwrap();(item.metadata,item.content)} else {let item=source.get_document(&id).await.unwrap().unwrap();(item.metadata,item.content)};
            let template=&metadata["onetaskgraph.template"];
            assert_eq!(template["template"],provenance["template"]);assert_eq!(template["digest"],provenance["digest"]);assert_eq!(template["answers_digest"],provenance["answers_digest"]);
            assert_eq!(template["body_digest"],body_digest(stored_content.as_deref().unwrap()));
            let held=AssetUploads::read(&metadata).unwrap().unwrap();
            let mut changed=write.clone();changed.recorded_assets=Some(held);changed.assets[1]=AssetPayload::of(AssetName::new("next.png").unwrap(),images::png(100,480_000));
            let before=plan.github.body(&id.0);
            plan.board.refuse(Refusal {stage:Stage::Read,status:404,headers:String::new(),body:"broken".into(),remaining:1});
            let error=if kind=="task" {source.set_task_rendering_with_assets(&id,content,&provenance,&Default::default(),&changed).await} else {source.set_document_rendering_with_assets(&id,content,&provenance,&Default::default(),&changed).await}.unwrap_err();
            assert!(error.to_string().contains("404"));assert_eq!(plan.github.body(&id.0),before,"failed verification changed the issue body");
            let missing=if kind=="task" {source.set_task_rendering_with_assets(&missing,"",&json!({}),&Default::default(),&AssetWrite::default()).await} else {source.set_document_rendering_with_assets(&missing,"",&json!({}),&Default::default(),&AssetWrite::default()).await}.unwrap();
            assert!(missing.is_none());
        });
    }
}

#[test]
fn malformed_asset_inputs_and_http_answers_refuse_writes_and_renders_without_changing_the_issue() {
    use onetaskgraph_plugin_api::{
        AssetName, AssetPayload, AssetUpload, AssetUploads, AssetWrite, ItemWrite, NativeId,
        SecretResolver, SourceName, SourcePlugin,
    };
    struct Secrets;
    impl SecretResolver for Secrets {
        fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
            (variable == "GITHUB_PROJECTS_FIXTURE_TOKEN").then(|| "test-token".into())
        }
    }
    #[derive(Clone, Copy)]
    enum Fault {
        Digest,
        Bytes,
        RepositoryJson,
        RepositoryId,
        RepositoryZero,
        RepositoryTransport,
        RepositoryTruncated,
        UploadJson,
        MissingUrl,
        InvalidUrl,
        ForeignUrl,
        ForeignPort,
        UsernameUrl,
        PasswordUrl,
        Truncated,
        ReusedUrl,
        Transport,
    }
    for (kind, record) in [("task", "T"), ("document", "D")] {
        let clock = SimulatedClock::start(1);
        let plan = Plan::new(&clock, 0, None);
        plan.author(kind, record, false, &[480_000], 80);
        let report = plan.copy(kind, record);
        let target = NativeId(
            report["items"][0]["destination"]
                .as_str()
                .unwrap()
                .strip_prefix("board:")
                .unwrap()
                .to_owned(),
        );
        let lease = clock.lease(0);
        lease.wait_for_detach();
        let shared = onetaskgraph_core::process_clock(&onetaskgraph_core::Environment::from_pairs(
            clock.client_env(0),
        ))
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for fault in [Fault::Digest,Fault::Bytes,Fault::RepositoryJson,Fault::RepositoryId,Fault::RepositoryZero,Fault::RepositoryTransport,Fault::RepositoryTruncated,Fault::UploadJson,Fault::MissingUrl,Fault::InvalidUrl,Fault::ForeignUrl,Fault::ForeignPort,Fault::UsernameUrl,Fault::PasswordUrl,Fault::Truncated,Fault::ReusedUrl,Fault::Transport] {
                let source=onetaskgraph_github_projects::Plugin.build_with_clock(&SourceName::new("board").unwrap(),&plan.config,&Secrets,shared.clone()).unwrap();
                let content="![Unwritten change](./next.png)";
                let task=if kind=="task" {let mut item=source.get_task(&target).await.unwrap().unwrap();item.content=Some(content.into());Some(item)} else {None};
                let document=if kind=="document" {let mut item=source.get_document(&target).await.unwrap().unwrap();item.content=Some(content.into());Some(item)} else {None};
                let mut payload=AssetPayload::of(AssetName::new("next.png").unwrap(),images::png(81,480_000));
                let mut recorded_assets=None;
                match fault {
                    Fault::Digest=>payload.sha256="0".repeat(64),
                    Fault::Bytes=>payload.bytes=None,
                    Fault::ReusedUrl=>{
                        recorded_assets=Some(AssetUploads(std::collections::BTreeMap::from([(payload.name.clone(),AssetUpload {sha256:payload.sha256.clone(),url:"not-a-url".into()})])));
                        payload.bytes=None;
                    },
                    _=>(),
                }
                let assets=AssetWrite {assets:vec![payload],recorded_assets};
                for rendering in [false,true] {
                    match fault {
                        Fault::RepositoryJson|Fault::RepositoryId|Fault::RepositoryZero|Fault::UploadJson|Fault::MissingUrl|Fault::InvalidUrl|Fault::ForeignUrl|Fault::ForeignPort|Fault::UsernameUrl|Fault::PasswordUrl=>{
                            let (stage,body)=match fault {
                                Fault::RepositoryJson=>(Stage::Repository,"not json"),
                                Fault::RepositoryId=>(Stage::Repository,"{\"id\":\"opaque\"}"),
                                Fault::RepositoryZero=>(Stage::Repository,r#"{"id":0}"#),
                                Fault::UploadJson=>(Stage::Upload,"not json"),
                                Fault::MissingUrl=>(Stage::Upload,"{\"href\":\"https://github.com/user-attachments/assets/example\"}"),
                                Fault::InvalidUrl=>(Stage::Upload,"{\"url\":\"not-a-url\"}"),
                                Fault::ForeignUrl=>(Stage::Upload,"{\"url\":\"https://example.invalid/image.png\"}"),
                                Fault::ForeignPort=>(Stage::Upload,r#"{"url":"https://github.com:8443/user-attachments/assets/example"}"#),
                                Fault::UsernameUrl=>(Stage::Upload,"{\"url\":\"https://user@github.com/user-attachments/assets/example\"}"),
                                Fault::PasswordUrl=>(Stage::Upload,"{\"url\":\"https://:secret@github.com/user-attachments/assets/example\"}"),
                                _=>unreachable!(),
                            };
                            let body=if matches!(fault,Fault::UsernameUrl|Fault::PasswordUrl) {
                                let authority=plan.board.endpoint.strip_prefix("http://").unwrap().split('/').next().unwrap();
                                let credentials=if matches!(fault,Fault::UsernameUrl) {"user"} else {":secret"};
                                json!({"url":format!("http://{credentials}@{authority}/user-attachments/assets/example")}).to_string()
                            } else {body.into()};
                            plan.board.refuse(Refusal {stage,status:200,headers:String::new(),body,remaining:1});
                        },
                        Fault::Truncated=>plan.board.truncate_next(Stage::Upload),
                        Fault::RepositoryTruncated=>plan.board.truncate_next(Stage::Repository),
                        Fault::RepositoryTransport=>plan.board.disconnect_next(Stage::Repository),
                        Fault::Transport=>plan.board.disconnect_next(Stage::Upload),
                        _=>(),
                    }
                    let before=plan.github.body(&target.0);
                    let error=if rendering {
                        if kind=="task" {source.set_task_rendering_with_assets(&target,content,&json!({"body_digest":"original"}),&Default::default(),&assets).await.unwrap_err()}
                        else {source.set_document_rendering_with_assets(&target,content,&json!({"body_digest":"original"}),&Default::default(),&assets).await.unwrap_err()}
                    } else if let Some(task)=&task {
                        source.write_task_with_assets(&ItemWrite {target:Some(target.clone()),item:task.clone(),depends_on:vec![]},None,&assets).await.unwrap_err()
                    } else {
                        source.write_document_with_assets(&ItemWrite {target:Some(target.clone()),item:document.as_ref().unwrap().clone(),depends_on:vec![]},None,&assets).await.unwrap_err()
                    };
                    assert!(error.to_string().contains("next.png"),"{error}");
                    if matches!(fault,Fault::ForeignPort|Fault::UsernameUrl|Fault::PasswordUrl) {assert!(error.to_string().contains("unexpected attachment URL"),"{error}");}
                    if matches!(fault,Fault::Truncated|Fault::RepositoryTruncated) {assert!(error.to_string().contains("response could not be read"),"{error}");}
                    assert_eq!(plan.github.body(&target.0),before,"a preparation refusal changed the issue");
                }
            }
        });
    }
    // A draft belongs to no repository: both entry points refuse before creating content.
    let clock = SimulatedClock::start(1);
    let plan = Plan::new(&clock, 0, None);
    let mut config = onetaskgraph_e2e_support::fixtures::github_projects_with_draft(&plan.sandbox);
    let board = Board::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
    config["endpoint"] = json!(board.endpoint);
    let shared = onetaskgraph_core::process_clock(&onetaskgraph_core::Environment::from_pairs(
        clock.client_env(0),
    ))
    .unwrap();
    let source = onetaskgraph_github_projects::Plugin
        .build_with_clock(
            &SourceName::new("board").unwrap(),
            &config,
            &Secrets,
            shared,
        )
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let id = NativeId("DRAFT-1".into());
        let before = source.get_task(&id).await.unwrap().unwrap();
        let mut item = before.clone();
        item.content = Some("![Cannot attach](./next.png)".into());
        let assets = AssetWrite {
            assets: vec![AssetPayload::of(
                AssetName::new("next.png").unwrap(),
                images::png(82, 480_000),
            )],
            recorded_assets: None,
        };
        let write = ItemWrite {
            target: Some(id.clone()),
            item,
            depends_on: vec![],
        };
        let error = source
            .write_task_with_assets(&write, None, &assets)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("repository"));
        let error = source
            .set_task_rendering_with_assets(
                &id,
                write.item.content.as_deref().unwrap(),
                &json!({}),
                &Default::default(),
                &assets,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("repository"));
        source.end_command().await.unwrap();
        assert_eq!(source.get_task(&id).await.unwrap().unwrap(), before);
        assert!(!board.calls().iter().any(|call| call.creating));
    });
}

#[test]
fn attachment_accounting_matches_the_real_http_responses() {
    use onetaskgraph_github_projects::{
        Plugin,
        accounting::{Accounting, AttachmentOperation, Mode, Outcome},
    };
    use onetaskgraph_plugin_api::{
        AssetName, AssetPayload, AssetWrite, NativeId, SecretResolver, SourceName,
    };
    struct Secrets;
    impl SecretResolver for Secrets {
        fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
            (variable == "GITHUB_PROJECTS_FIXTURE_TOKEN").then(|| "test-token".into())
        }
    }
    for (stage, status, outcome) in [
        (Stage::Upload, 201, Outcome::Answered),
        (Stage::Upload, 0, Outcome::Refused),
        (Stage::Upload, 422, Outcome::Refused),
        (Stage::Read, 404, Outcome::Refused),
        (Stage::Upload, 403, Outcome::RateLimited),
        (Stage::Read, 403, Outcome::RateLimited),
    ] {
        let clock = SimulatedClock::start(1);
        let plan = Plan::new(&clock, 0, None);
        plan.author("document", "D", false, &[], 1);
        let report = plan.copy("document", "D");
        let target = NativeId(
            report["items"][0]["destination"]
                .as_str()
                .unwrap()
                .strip_prefix("board:")
                .unwrap()
                .into(),
        );
        let mut config = plan.config.clone();
        // No real-time waits: this journey inspects accounting, not pacing or retry recovery.
        config["pacing"] = json!({"min_mutation_interval_ms":0,"retry_budget_ms":0});
        let ledger = std::sync::Arc::new(Accounting::new());
        let source = Plugin
            .build_recording_into(
                &SourceName::new("board").unwrap(),
                &config,
                &Secrets,
                ledger.clone(),
            )
            .unwrap();
        if status == 0 {
            plan.board.disconnect_next(Stage::Upload);
        } else if status != 201 {
            plan.board.refuse(Refusal {
                stage, status,
                headers:"x-ratelimit-limit: 5000\r\nx-ratelimit-remaining: 4999\r\nx-ratelimit-used: 1\r\nx-ratelimit-reset: 12345\r\nx-ratelimit-resource: core\r\nRetry-After: 1\r\n".into(),
                body:if status == 403 {"You have exceeded a secondary rate limit"} else {"refused"}.into(), remaining:1,
            });
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(source.set_document_rendering_with_assets(
            &target,
            "![Accounting image](./ledger.png)",
            &json!({}),
            &Default::default(),
            &AssetWrite {
                assets: vec![AssetPayload::of(
                    AssetName::new("ledger.png").unwrap(),
                    images::png(150, 480_000),
                )],
                recorded_assets: None,
            },
        ));
        assert_eq!(result.is_ok(), status == 201, "{result:?}");
        let attachments = ledger.attachment_responses();
        assert_eq!(
            attachments.len(),
            if status == 0 {
                0
            } else if stage == Stage::Upload && status != 201 {
                1
            } else {
                2
            }
        );
        for (index, response) in attachments.iter().enumerate() {
            let operation = if index == 0 {
                AttachmentOperation::Upload
            } else {
                AttachmentOperation::Read
            };
            assert_eq!(response.operation, operation);
            assert_eq!(response.name.as_str(), "ledger.png");
            assert_eq!(
                response.url.origin().ascii_serialization(),
                plan.board.endpoint.trim_end_matches("/graphql")
            );
            assert_eq!(
                response.status.as_u16(),
                if index == attachments.len() - 1 {
                    if status == 201 { 200 } else { status }
                } else {
                    201
                }
            );
            if index == 0 {
                assert_eq!(response.url.path(), "/user-attachments/assets");
                assert!(response.url.query().unwrap().contains("name=ledger.png"));
            } else {
                assert_eq!(response.url.path(), "/user-attachments/assets/1");
            }
        }
        let snapshot = ledger.snapshot();
        let last = snapshot
            .requests()
            .iter()
            .find(|request| {
                request.name()
                    == if stage == Stage::Upload && status != 201 {
                        "POST /user-attachments/assets"
                    } else {
                        "GET /user-attachments/assets/{asset}"
                    }
            })
            .unwrap();
        assert_eq!(last.outcome(), outcome);
        assert_eq!(
            last.mode(),
            if stage == Stage::Upload && status != 201 {
                Mode::Write
            } else {
                Mode::Read
            }
        );
        if status == 0 {
            assert_eq!(last.rate_limit().limit(), None);
            assert_eq!(last.rate_limit().remaining(), None);
            assert_eq!(last.rate_limit().used_by_the_account(), None);
            assert_eq!(last.rate_limit().reset(), None);
            assert_eq!(last.rate_limit().resource(), None);
        } else if status != 201 {
            let limits = last.rate_limit();
            assert_eq!(limits.limit(), Some(5000));
            assert_eq!(limits.remaining(), Some(4999));
            assert_eq!(limits.used_by_the_account(), Some(1));
            assert_eq!(limits.reset(), Some(12345));
            assert_eq!(limits.resource(), Some("core"));
        }
    }
}

#[test]
fn malformed_stored_assets_refuse_reads_and_assets_without_content_are_persisted() {
    use onetaskgraph_github_projects::Plugin;
    use onetaskgraph_plugin_api::{
        AssetName, AssetPayload, AssetUploads, AssetWrite, ItemWrite, NativeId, SecretResolver,
        SourceName, SourcePlugin,
    };
    struct Secrets;
    impl SecretResolver for Secrets {
        fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
            (variable == "GITHUB_PROJECTS_FIXTURE_TOKEN").then(|| "test-token".into())
        }
    }
    for (kind, record) in [("task", "T"), ("document", "D")] {
        let clock = SimulatedClock::start(1);
        let plan = Plan::new(&clock, 0, None);
        plan.author(kind, record, false, &[], 1);
        let report = plan.copy(kind, record);
        let target = NativeId(
            report["items"][0]["destination"]
                .as_str()
                .unwrap()
                .strip_prefix("board:")
                .unwrap()
                .into(),
        );
        let mut config = plan.config.clone();
        config["pacing"] =
            json!({"min_mutation_interval_ms":0,"retry_backoff_ms":1000,"retry_budget_ms":2000});
        let lease = clock.lease(0);
        lease.wait_for_detach();
        let shared = onetaskgraph_core::process_clock(&onetaskgraph_core::Environment::from_pairs(
            clock.client_env(0),
        ))
        .unwrap();
        let source = Plugin
            .build_with_clock(
                &SourceName::new("board").unwrap(),
                &config,
                &Secrets,
                shared,
            )
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            if kind == "task" {
                let mut item = source.get_task(&target).await.unwrap().unwrap();
                item.metadata
                    .insert("onetaskgraph.assets".into(), json!("malformed"));
                source
                    .write_task(&ItemWrite {
                        target: Some(target.clone()),
                        item,
                        depends_on: vec![],
                    })
                    .await
                    .unwrap();
            } else {
                let mut item = source.get_document(&target).await.unwrap().unwrap();
                item.metadata
                    .insert("onetaskgraph.assets".into(), json!("malformed"));
                source
                    .write_document(&ItemWrite {
                        target: Some(target.clone()),
                        item,
                        depends_on: vec![],
                    })
                    .await
                    .unwrap();
            }
            source.end_command().await.unwrap();
            assert!(
                plan.github
                    .body(&target.0)
                    .as_str()
                    .unwrap()
                    .contains("malformed")
            );
            let name = AssetName::new("held.png").unwrap();
            let list = if kind == "task" {
                source.task_assets(&target).await
            } else {
                source.document_assets(&target).await
            };
            assert!(
                list.unwrap_err()
                    .to_string()
                    .contains("onetaskgraph.assets")
            );
            let read = if kind == "task" {
                source.task_asset(&target, &name).await
            } else {
                source.document_asset(&target, &name).await
            };
            assert!(
                read.unwrap_err()
                    .to_string()
                    .contains("onetaskgraph.assets")
            );
            let bytes = images::png(151, 480_000);
            let assets = AssetWrite {
                assets: vec![AssetPayload::of(name.clone(), bytes.clone())],
                recorded_assets: None,
            };
            let written = if kind == "task" {
                let mut item = source.get_task(&target).await.unwrap().unwrap();
                item.content = None;
                item.metadata.remove("onetaskgraph.assets");
                source
                    .write_task_with_assets(
                        &ItemWrite {
                            target: Some(target.clone()),
                            item,
                            depends_on: vec![],
                        },
                        None,
                        &assets,
                    )
                    .await
                    .unwrap()
            } else {
                let mut item = source.get_document(&target).await.unwrap().unwrap();
                item.content = None;
                item.metadata.remove("onetaskgraph.assets");
                source
                    .write_document_with_assets(
                        &ItemWrite {
                            target: Some(target.clone()),
                            item,
                            depends_on: vec![],
                        },
                        None,
                        &assets,
                    )
                    .await
                    .unwrap()
            };
            assert!(written.content.is_none());
            assert_eq!(written.id, target);
            source.end_command().await.unwrap();
            let metadata = if kind == "task" {
                source.get_task(&target).await.unwrap().unwrap().metadata
            } else {
                source
                    .get_document(&target)
                    .await
                    .unwrap()
                    .unwrap()
                    .metadata
            };
            let held = AssetUploads::read(&metadata).unwrap().unwrap();
            assert_eq!(held.0[&name].sha256, asset_sha256(&bytes));
            assert_eq!(held.0.len(), 1);
            assert_eq!(
                held.0[&name].url,
                plan.board
                    .endpoint
                    .replace("/graphql", "/user-attachments/assets/1")
            );
            let calls = plan.board.calls();
            assert_eq!(uploads(&calls).len(), 1);
            assert_eq!(attachment_reads(&calls), 1);
            for url in ["not-a-url", held.0[&name].url.as_str()] {
                let mut uploads = held.clone();
                uploads.0.get_mut(&name).unwrap().url = url.into();
                if kind == "task" {
                    let mut item = source.get_task(&target).await.unwrap().unwrap();
                    item.metadata.insert(
                        "onetaskgraph.assets".into(),
                        serde_json::to_value(&uploads).unwrap(),
                    );
                    source
                        .write_task(&ItemWrite {
                            target: Some(target.clone()),
                            item,
                            depends_on: vec![],
                        })
                        .await
                        .unwrap();
                } else {
                    let mut item = source.get_document(&target).await.unwrap().unwrap();
                    item.metadata.insert(
                        "onetaskgraph.assets".into(),
                        serde_json::to_value(&uploads).unwrap(),
                    );
                    source
                        .write_document(&ItemWrite {
                            target: Some(target.clone()),
                            item,
                            depends_on: vec![],
                        })
                        .await
                        .unwrap();
                }
                source.end_command().await.unwrap();
                let before = attachment_reads(&plan.board.calls());
                let read = if kind == "task" {
                    source.task_asset(&target, &name).await
                } else {
                    source.document_asset(&target, &name).await
                };
                if url == "not-a-url" {
                    assert!(read.unwrap_err().to_string().contains("invalid URL"));
                    assert_eq!(attachment_reads(&plan.board.calls()), before);
                } else {
                    assert_eq!(read.unwrap(), Some(bytes.clone()));
                }
            }
            let before_body = plan.github.body(&target.0);
            for failure in ["http", "transport", "truncated", "primary", "secondary"] {
                if failure == "transport" {
                    plan.board.disconnect_next(Stage::Read);
                } else if failure == "truncated" {
                    plan.board.truncate_next(Stage::Read);
                } else {
                    let (status, headers, body) = match failure {
                        "http" => (404, String::new(), "missing"),
                        "primary" => (
                            403,
                            format!(
                                "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: {}\r\n",
                                chrono::Utc::now().timestamp() + 1
                            ),
                            "API rate limit exceeded",
                        ),
                        "secondary" => (
                            403,
                            "Retry-After: 1\r\n".into(),
                            "You have exceeded a secondary rate limit",
                        ),
                        _ => unreachable!(),
                    };
                    plan.board.refuse(Refusal {
                        stage: Stage::Read,
                        status,
                        headers,
                        body: body.into(),
                        remaining: 1,
                    });
                }
                let before = clock.now();
                let read = if kind == "task" {
                    source.task_asset(&target, &name).await
                } else {
                    source.document_asset(&target, &name).await
                };
                if matches!(failure, "primary" | "secondary") {
                    assert_eq!(read.unwrap(), Some(bytes.clone()));
                    assert!(clock.now() - before >= Duration::from_secs(1));
                } else {
                    let error = read.unwrap_err().to_string();
                    assert!(
                        error.contains("held.png") && error.contains("reading attachment"),
                        "{error}"
                    );
                    if failure == "http" {
                        assert!(error.contains("404"));
                    }
                    let recovered = if kind == "task" {
                        source.task_asset(&target, &name).await
                    } else {
                        source.document_asset(&target, &name).await
                    };
                    assert_eq!(recovered.unwrap(), Some(bytes.clone()));
                }
                assert_eq!(plan.github.body(&target.0), before_body);
            }
            let removed = if kind == "task" {
                source
                    .set_task_rendering_with_assets(
                        &target,
                        "Images removed",
                        &json!({}),
                        &Default::default(),
                        &AssetWrite::default(),
                    )
                    .await
                    .unwrap()
                    .unwrap()
            } else {
                source
                    .set_document_rendering_with_assets(
                        &target,
                        "Images removed",
                        &json!({}),
                        &Default::default(),
                        &AssetWrite::default(),
                    )
                    .await
                    .unwrap()
                    .unwrap()
            };
            assert_eq!(removed.content.as_deref(), Some("Images removed"));
            source.end_command().await.unwrap();
            let empty = if kind == "task" {
                source.get_task(&target).await.unwrap().unwrap().metadata
            } else {
                source
                    .get_document(&target)
                    .await
                    .unwrap()
                    .unwrap()
                    .metadata
            };
            assert!(AssetUploads::read(&empty).unwrap().is_none());
            assert!(
                !plan
                    .github
                    .body(&target.0)
                    .as_str()
                    .unwrap()
                    .contains("onetaskgraph.assets")
            );
            if kind == "task" {
                let mut item = source.get_task(&target).await.unwrap().unwrap();
                item.content = Some("![Restored](./held.png)".into());
                source
                    .write_task_with_assets(
                        &ItemWrite {
                            target: Some(target.clone()),
                            item,
                            depends_on: vec![],
                        },
                        None,
                        &assets,
                    )
                    .await
                    .unwrap();
            } else {
                let mut item = source.get_document(&target).await.unwrap().unwrap();
                item.content = Some("![Restored](./held.png)".into());
                source
                    .write_document_with_assets(
                        &ItemWrite {
                            target: Some(target.clone()),
                            item,
                            depends_on: vec![],
                        },
                        None,
                        &assets,
                    )
                    .await
                    .unwrap();
            }
            source.end_command().await.unwrap();
            if kind == "task" {
                let mut item = source.get_task(&target).await.unwrap().unwrap();
                assert!(AssetUploads::read(&item.metadata).unwrap().is_some());
                item.content = Some("Assets removed by write".into());
                source
                    .write_task_with_assets(
                        &ItemWrite {
                            target: Some(target.clone()),
                            item,
                            depends_on: vec![],
                        },
                        None,
                        &AssetWrite::default(),
                    )
                    .await
                    .unwrap();
            } else {
                let mut item = source.get_document(&target).await.unwrap().unwrap();
                assert!(AssetUploads::read(&item.metadata).unwrap().is_some());
                item.content = Some("Assets removed by write".into());
                source
                    .write_document_with_assets(
                        &ItemWrite {
                            target: Some(target.clone()),
                            item,
                            depends_on: vec![],
                        },
                        None,
                        &AssetWrite::default(),
                    )
                    .await
                    .unwrap();
            }
            source.end_command().await.unwrap();
            let empty = if kind == "task" {
                source.get_task(&target).await.unwrap().unwrap().metadata
            } else {
                source
                    .get_document(&target)
                    .await
                    .unwrap()
                    .unwrap()
                    .metadata
            };
            assert!(AssetUploads::read(&empty).unwrap().is_none());
            assert!(
                !plan
                    .github
                    .body(&target.0)
                    .as_str()
                    .unwrap()
                    .contains("onetaskgraph.assets")
            );
        });
    }
}

#[test]
fn truncated_asset_responses_are_refused_and_accounted_without_changing_issue_bodies() {
    use onetaskgraph_github_projects::{
        Plugin,
        accounting::{Accounting, Outcome},
    };
    use onetaskgraph_plugin_api::{
        AssetName, AssetPayload, AssetWrite, NativeId, SecretResolver, SourceName,
    };
    struct Secrets;
    impl SecretResolver for Secrets {
        fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
            (variable == "GITHUB_PROJECTS_FIXTURE_TOKEN").then(|| "test-token".into())
        }
    }
    for (stage, held) in [
        (Stage::Repository, false),
        (Stage::Upload, false),
        (Stage::Read, false),
        (Stage::Read, true),
    ] {
        let clock = SimulatedClock::start(1);
        let plan = Plan::new(&clock, 0, None);
        plan.author("document", "D", false, &[], 1);
        let report = plan.copy("document", "D");
        let target = NativeId(
            report["items"][0]["destination"]
                .as_str()
                .unwrap()
                .strip_prefix("board:")
                .unwrap()
                .into(),
        );
        let mut config = plan.config.clone();
        config["pacing"] = json!({"min_mutation_interval_ms":0,"retry_budget_ms":0});
        let ledger = std::sync::Arc::new(Accounting::new());
        let source = Plugin
            .build_recording_into(
                &SourceName::new("board").unwrap(),
                &config,
                &Secrets,
                ledger.clone(),
            )
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let name = AssetName::new("truncated.png").unwrap();
        let assets = AssetWrite {
            assets: vec![AssetPayload::of(name.clone(), images::png(155, 480_000))],
            recorded_assets: None,
        };
        let content = "![Never broken](./truncated.png)";
        if held {
            runtime
                .block_on(source.set_document_rendering_with_assets(
                    &target,
                    content,
                    &json!({}),
                    &Default::default(),
                    &assets,
                ))
                .unwrap()
                .unwrap();
            runtime.block_on(source.end_command()).unwrap();
        }
        let before = plan.github.body(&target.0);
        plan.board.truncate_next(stage);
        let error = if held {
            runtime
                .block_on(source.document_asset(&target, &name))
                .unwrap_err()
        } else {
            runtime
                .block_on(source.set_document_rendering_with_assets(
                    &target,
                    content,
                    &json!({}),
                    &Default::default(),
                    &assets,
                ))
                .unwrap_err()
        };
        assert!(
            error.to_string().contains("truncated.png")
                && error.to_string().contains("response could not be read"),
            "{error}"
        );
        assert_eq!(plan.github.body(&target.0), before);
        let snapshot = ledger.snapshot();
        let last = snapshot.requests().last().unwrap();
        assert_eq!(
            last.name(),
            match stage {
                Stage::Repository => "GET /repos/{owner}/{repo}",
                Stage::Upload => "POST /user-attachments/assets",
                Stage::Read => "GET /user-attachments/assets/{asset}",
            }
        );
        assert_eq!(last.outcome(), Outcome::Refused);
        assert_eq!(last.rate_limit().remaining(), None);
    }
}
