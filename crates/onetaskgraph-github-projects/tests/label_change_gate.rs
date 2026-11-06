//! The live lane's label edit against a loopback repository whose answers go missing.
//!
//! GitHub once answered this edit `500 Internal Server Error` with no body and failed a run
//! over nothing the code had done. The edit is the real one — `journey::change_artifact_label`,
//! the call the credentialed journey makes. What stands in for GitHub is a local HTTP server
//! holding labels and answering each edit as a scenario scripts. No credential and no
//! third-party API.

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{Value, json};

// The credentialed lane's own halves: `journey` holds the edit under test. Most of each belongs
// to the target that drives it against GitHub, for the reason `tests/sweep_gate.rs` gives.
#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

const TOKEN: &str = "test-token";
const REPOSITORY: &str = "acme/work";
const LABELS: &str = "/repos/acme/work/labels/";

/// What the repository does with one edit, and what it answers.
#[derive(Clone, Copy)]
enum Edit {
    /// Applies the edit and answers `200`.
    Lands,
    /// Applies the edit and answers `500` with no body.
    LandsUnanswered,
    /// Leaves the label alone and answers `500` with no body.
    LostUnanswered,
    /// Leaves the label alone and answers `422`.
    Refused,
}

#[derive(Default)]
struct Repository {
    /// Each label's name and color.
    labels: BTreeMap<String, String>,
    /// How the coming edits go; one past the script lands.
    script: VecDeque<Edit>,
    requests: Vec<String>,
}

fn apply(repository: &mut Repository, name: &str, change: &Value) {
    let color = repository
        .labels
        .remove(name)
        .expect("an edit of a label the repository holds");
    let named = change["new_name"].as_str().unwrap_or(name).to_owned();
    let color = change["color"].as_str().map_or(color, str::to_owned);
    repository.labels.insert(named, color);
}

fn answer(repository: &mut Repository, method: &str, path: &str, body: &str) -> (u16, String) {
    let name = path.strip_prefix(LABELS).expect("a label endpoint");
    repository.requests.push(format!("{method} {name}"));
    if method == "GET" {
        return match repository.labels.get(name) {
            Some(color) => (200, json!({"name": name, "color": color}).to_string()),
            None => (404, json!({"message": "Not Found"}).to_string()),
        };
    }
    assert_eq!(method, "PATCH");
    if !repository.labels.contains_key(name) {
        return (404, json!({"message": "Not Found"}).to_string());
    }
    let change: Value = serde_json::from_str(body).expect("an edit's JSON body");
    match repository.script.pop_front().unwrap_or(Edit::Lands) {
        Edit::Lands => {
            apply(repository, name, &change);
            (200, "{}".into())
        }
        Edit::LandsUnanswered => {
            apply(repository, name, &change);
            (500, String::new())
        }
        Edit::LostUnanswered => (500, String::new()),
        Edit::Refused => (422, json!({"message": "Validation Failed"}).to_string()),
    }
}

fn serve(repository: Arc<Mutex<Repository>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a stand-in listener");
    let host = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("a stand-in connection");
            let (method, path, body) = read_request(&mut stream);
            let (status, payload) = answer(&mut repository.lock().unwrap(), &method, &path, &body);
            let response = format!(
                "HTTP/1.1 {status} Stand-in\r\nContent-Type: application/json\r\n\
                 x-ratelimit-limit: 5000\r\nx-ratelimit-used: 1\r\n\
                 x-ratelimit-remaining: 4999\r\nx-ratelimit-resource: core\r\n\
                 x-ratelimit-reset: 1775000000\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    host
}

fn read_request(stream: &mut impl Read) -> (String, String, String) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
        let count = stream.read(&mut chunk).expect("a stand-in request");
        assert!(count > 0, "the request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
    };
    let head = String::from_utf8_lossy(&bytes[..end]).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("a numeric length"))
        })
        .unwrap_or(0);
    while bytes.len() < end + length {
        let count = stream.read(&mut chunk).expect("a stand-in body");
        assert!(count > 0, "the request ended before its body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    let mut line = head.lines().next().unwrap_or("").split(' ');
    let method = line.next().unwrap_or("").to_owned();
    let path = line.next().unwrap_or("").to_owned();
    let body = String::from_utf8_lossy(&bytes[end..end + length]).into_owned();
    (method, path, body)
}

/// One edit of label `a` of color `ededed`, scripted; what it returned, and what the repository
/// then holds and was sent.
async fn edit(
    repository: &Arc<Mutex<Repository>>,
    script: &[Edit],
    change: Value,
) -> (Result<(), String>, BTreeMap<String, String>, Vec<String>) {
    {
        let mut repository = repository.lock().unwrap();
        repository.labels = BTreeMap::from([("a".to_owned(), "ededed".to_owned())]);
        repository.script = script.iter().copied().collect();
        repository.requests.clear();
    }
    let result = journey::change_artifact_label(TOKEN, REPOSITORY, "a", change).await;
    let repository = repository.lock().unwrap();
    (
        result,
        repository.labels.clone(),
        repository.requests.clone(),
    )
}

// One test, because a binary is pointed at one API once (`journey::against`).
#[tokio::test]
async fn a_label_edit_github_never_answered_is_settled_by_the_label() {
    let repository = Arc::new(Mutex::new(Repository::default()));
    let host = serve(Arc::clone(&repository));
    journey::against(journey::Endpoints {
        graphql: format!("{host}/graphql"),
        rest_host: host,
        source: None,
        settle: std::time::Duration::ZERO,
        fixture_wait: journey::STAND_IN_FIXTURE_WAIT,
    });
    let rename = || json!({"new_name": "b"});
    let recolour = || json!({"color": "5319e7"});

    // The rename landed behind its 500: sent again it would be refused, so it is not sent.
    let (result, labels, requests) = edit(&repository, &[Edit::LandsUnanswered], rename()).await;
    assert_eq!(result, Ok(()));
    assert_eq!(labels["b"], "ededed");
    assert_eq!(requests, ["PATCH a", "GET b"]);

    // The rename did not land: it is sent again, and the label ends as asked.
    let (result, labels, requests) = edit(&repository, &[Edit::LostUnanswered], rename()).await;
    assert_eq!(result, Ok(()));
    assert_eq!(labels["b"], "ededed");
    assert_eq!(requests, ["PATCH a", "GET b", "PATCH a"]);

    // A recolour is read back by its color, under the name it already has.
    let (result, labels, requests) = edit(&repository, &[Edit::LostUnanswered], recolour()).await;
    assert_eq!(result, Ok(()));
    assert_eq!(labels["a"], "5319e7");
    assert_eq!(requests, ["PATCH a", "GET a", "PATCH a"]);
    let (result, labels, requests) = edit(&repository, &[Edit::LandsUnanswered], recolour()).await;
    assert_eq!(result, Ok(()));
    assert_eq!(labels["a"], "5319e7");
    assert_eq!(requests, ["PATCH a", "GET a"]);

    // A refusal GitHub answered is not asked again or read past.
    let (result, labels, requests) = edit(&repository, &[Edit::Refused], rename()).await;
    let problem = result.expect_err("a refused edit fails");
    assert!(problem.contains("HTTP 422"), "{problem}");
    assert_eq!(labels["a"], "ededed");
    assert_eq!(requests, ["PATCH a"]);

    // One after an unanswered attempt fails too, when the label says that attempt did not land.
    let (result, labels, requests) = edit(
        &repository,
        &[Edit::LostUnanswered, Edit::Refused],
        rename(),
    )
    .await;
    let problem = result.expect_err("a refused retry fails");
    assert!(problem.contains("HTTP 422"), "{problem}");
    assert_eq!(labels["a"], "ededed");
    assert_eq!(requests, ["PATCH a", "GET b", "PATCH a", "GET b"]);

    // Never answered and never landed: the bound holds, and the failure says what it saw.
    let (result, labels, requests) = edit(
        &repository,
        &[
            Edit::LostUnanswered,
            Edit::LostUnanswered,
            Edit::LostUnanswered,
        ],
        rename(),
    )
    .await;
    let problem = result.expect_err("an edit that never lands fails");
    assert!(
        problem.contains("after 3 attempts") && problem.contains("HTTP 500"),
        "{problem}"
    );
    assert_eq!(labels["a"], "ededed");
    assert_eq!(requests.iter().filter(|r| *r == "PATCH a").count(), 3);
}
