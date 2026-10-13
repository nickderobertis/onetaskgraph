//! The session bridge records the actual source's requests across a subprocess boundary.
use onetaskgraph_github_live::quota::{
    Charge, EntryPoint, Outcome, Phase, SessionRecord, operation_sha256, reference_workload,
};
use onetaskgraph_github_projects::{Plugin, accounting::Accounting};
use onetaskgraph_plugin_api::{SecretResolver, SourceName};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{Arc, Mutex},
};

#[path = "journey/quota.rs"]
mod quota;

struct Secrets;
impl SecretResolver for Secrets {
    fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
        (variable == "GH_PROJECTS_TOKEN").then(|| "fake-quota-token".into())
    }
}

// llmlint: ignore[tests_assert_real_behavior] This is the subprocess entry point driven by bridge_persists_the_real_sources_prices_and_failure_cleanup below. The parent asserts protocol facts and persisted bytes; an ordinary test run has no child request to execute.
#[tokio::test]
async fn bridge_child() {
    let Ok(endpoint) = std::env::var("QUOTA_WRITER_TEST_ENDPOINT") else {
        return;
    };
    let ledger = Arc::new(Accounting::with_query_prices());
    let _scope = quota::start(EntryPoint::Main, Arc::clone(&ledger), reference_workload()).unwrap();
    quota::observe_initial_board(Some(90), Some(14)).unwrap();
    let source = Plugin
        .build_recording_into(
            &SourceName::new("work").unwrap(),
            &json!({"owner":"octo-org","project_number":7,"endpoint":endpoint,
            "repository":"acme/work","pacing":{"min_mutation_interval_ms":0,"retry_budget_ms":0}}),
            &Secrets,
            ledger,
        )
        .unwrap();
    assert!(source.health().await.unwrap().reachable);
    quota::phase(Phase::Schema).unwrap();
    let outcome = source.health().await;
    quota::phase(Phase::Cleanup).unwrap();
    assert!(source.health().await.unwrap().reachable);
    assert!(outcome.unwrap().reachable);
    quota::complete().unwrap();
}

fn request(stream: &mut std::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let length: usize = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")?
                .parse()
                .ok()
        })
        .unwrap();
    while bytes.len() < header_end + length {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}

#[test]
fn bridge_persists_the_real_sources_prices_and_failure_cleanup() {
    use std::sync::atomic::{AtomicBool, Ordering};
    assert_eq!(
        operation_sha256(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    for case in ["success", "refusal", "missing-price"] {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let documents = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::clone(&documents);
        let server = std::thread::spawn(move || {
            for index in 0..3 {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if stopping.load(Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let sent = request(&mut stream);
                received
                    .lock()
                    .unwrap()
                    .push(sent["query"].as_str().unwrap().to_owned());
                let mut data = json!({"owner":{"projectV2":{"id":"P_board","title":"DO_NOT_STORE_BODY",
                    "items":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},
                    "rateLimit":{"cost":7}});
                if case == "missing-price" && index == 1 {
                    data.as_object_mut().unwrap().remove("rateLimit");
                }
                let mut answer = json!({"data":data});
                if case == "refusal" && index == 1 {
                    answer["errors"] = json!([{"message":"not permitted"}]);
                }
                let body = answer.to_string();
                let used = 400 + (index + 1) * 7;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nx-ratelimit-limit: 5000\r\nx-ratelimit-used: {used}\r\nx-ratelimit-remaining: {}\r\nx-ratelimit-reset: 2000000000\r\nConnection: close\r\n\r\n{body}", body.len(), 5000 - used).unwrap();
            }
        });
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["bridge_child", "--exact", "--nocapture"])
            .env(
                "QUOTA_WRITER_TEST_ENDPOINT",
                format!("http://{address}/graphql"),
            )
            .env("GITHUB_LIVE_SESSION_DIR", directory.path())
            .env("GITHUB_LIVE_SESSION_ID", format!("fixture-{case}"))
            .env("GITHUB_LIVE_SOURCE_REVISION", "a".repeat(40))
            .output()
            .unwrap();
        stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
        assert_eq!(
            output.status.success(),
            case != "refusal",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = std::fs::read_to_string(directory.path().join("main.json")).unwrap();
        let record: SessionRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(record.complete, case != "refusal");
        assert_eq!(record.calls.len(), 3);
        assert_eq!(
            record
                .calls
                .iter()
                .map(|call| call.phase)
                .collect::<Vec<_>>(),
            vec![Phase::Admission, Phase::Schema, Phase::Cleanup]
        );
        let documents = documents.lock().unwrap();
        for (index, call) in record.calls.iter().enumerate() {
            assert_eq!(call.operation_sha256, operation_sha256(&documents[index]));
            assert_eq!(call.sequence, index as u64 + 1);
            assert_eq!(call.attempt, 1);
            assert_eq!(
                call.outcome,
                if case == "refusal" && index == 1 {
                    Outcome::GraphqlError
                } else {
                    Outcome::Success
                }
            );
            if case == "missing-price" && index == 1 {
                assert!(matches!(call.charge, Charge::Unavailable { .. }));
            } else {
                assert_eq!(call.charge, Charge::ReportedQuery { points: 7 });
            }
            assert!(call.allowance.is_some());
        }
        for secret in ["DO_NOT_STORE_BODY", "octo-org", "fake-quota-token"] {
            assert!(!text.contains(secret));
        }
    }
}
