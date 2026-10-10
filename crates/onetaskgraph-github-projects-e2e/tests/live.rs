//! Disposable asset copies through Engine::copy, under the existing live lane admission.
use onetaskgraph_core::{
    ConfiguredSource, CopyItems, CopyRequest, CopyScope, Engine, ResolvedSource,
};
use onetaskgraph_e2e_support::{common::Sandbox, images};
use onetaskgraph_github_projects::{
    Plugin,
    accounting::{Accounting, Outcome, RateLimit, Request},
    graphql,
};
use onetaskgraph_live::{Exclusivity, Session};
use onetaskgraph_plugin_api::{AssetUploads, SourceName, SourcePlugin};
use serde_json::{Value, json};
use std::sync::Arc;

// Only admission and stamping are used here; the original live target drives its other helpers.
#[allow(dead_code)]
#[path = "../../onetaskgraph-github-projects/tests/lane/mod.rs"]
mod lane;
// Reuse the existing allowance precondition and first-call header recheck. The original
// live target remains the driver of this module's schema and capability journey.
#[allow(dead_code)]
#[path = "../../onetaskgraph-github-projects/tests/journey/mod.rs"]
mod journey;

fn name(value: &str) -> SourceName {
    SourceName::new(value).unwrap()
}

async fn nominated_status_option(
    endpoint: &str,
    token: &str,
    owner: &str,
    number: u32,
    ledger: &Accounting,
) -> Result<String, String> {
    let variables = json!({"owner":owner,"number":number,"nestedFirst":100});
    let response = reqwest::Client::new()
        .post(endpoint)
        .header("user-agent", "onetaskgraph-live-assets")
        .bearer_auth(token)
        .json(&json!({"query":graphql::BOARD_FIELDS,"variables":variables}))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let limits = RateLimit::read(|header| {
        response
            .headers()
            .get(header)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    let status = response.status();
    let answer: Value = response.json().await.map_err(|error| error.to_string())?;
    let outcome = Outcome::of_response(status, limits.exhausted(), &answer.to_string());
    ledger.record(
        Request::graphql(graphql::BOARD_FIELDS, &variables, None, None).finished(outcome, limits),
    );
    if !status.is_success() {
        return Err(format!("nominated board read returned HTTP {status}"));
    }
    if !answer["errors"].is_null() {
        return Err(format!(
            "nominated board read refused: {}",
            answer["errors"]
        ));
    }
    let board = &answer["data"]["boardFields"]["projectV2"];
    board["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("missing nominated board id")?;
    let nodes = board["fields"]["nodes"]
        .as_array()
        .ok_or("missing nominated board fields")?;
    let status = nodes
        .iter()
        .find(|field| field["name"] == "Status")
        .ok_or("missing nominated board Status field")?;
    let option = status["options"]
        .as_array()
        .and_then(|options| options.first())
        .and_then(|option| option["name"].as_str())
        .filter(|name| !name.is_empty())
        .ok_or("missing nominated board Status option")?;
    Ok(option.to_owned())
}

#[tokio::test]
async fn disposable_task_and_document_asset_copies() {
    let admission = lane::admit(&|variable| match std::env::var(variable) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => panic!("{variable} must be Unicode"),
    })
    .unwrap();
    let (token, owner, number, repository, writer) = match admission {
        lane::Admission::Skip(reason) => {
            journey::say(&format!("live asset journey did not run: {reason}"));
            return;
        }
        lane::Admission::Run {
            token,
            owner,
            project_number,
            repository,
            writer,
        } => (token, owner, project_number, repository, writer),
    };
    let session = Session::open(lane::SESSION_NAME, token, Exclusivity::Shared)
        .unwrap_or_else(|declined| declined.refuse());
    let token = session.credential().expose();
    let ledger = Arc::new(Accounting::new());
    let admitted = journey::budget::precondition(token, "https://api.github.com", &ledger)
        .await
        .unwrap_or_else(|declined| declined.refuse());
    let option = nominated_status_option(
        "https://api.github.com/graphql",
        token,
        &owner,
        number,
        &ledger,
    )
    .await
    .unwrap();
    journey::budget::recheck(&admitted, &ledger).unwrap_or_else(|declined| declined.refuse());
    let config = lane::live_write_config(&owner, number, &repository, &option);
    let secrets = lane::LiveSecret(token.to_owned().into());
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("notes");
    let stamp = onetaskgraph_live::artifact::now_micros();
    writer.check_clock(stamp, 1).unwrap();
    for (offset, kind, id) in [(0, "task", "T"), (1, "document", "D")] {
        let directory = root.join(format!("{kind}s/{id}.assets"));
        std::fs::create_dir_all(&directory).unwrap();
        let body = "![First written alt](./one.png)\n![Second written alt](./two.png)\n";
        for (index, file, size) in [(0, "one.png", 75_000), (1, "two.png", 480_000)] {
            std::fs::write(
                directory.join(file),
                images::png(stamp + offset + index, size),
            )
            .unwrap();
        }
        std::fs::write(
            root.join(format!("{kind}s/{id}.md")),
            format!(
                "---\ntitle: {}\n{}---\n{body}",
                lane::artifact_title(writer, stamp + offset),
                if kind == "task" { "status: todo\n" } else { "" }
            ),
        )
        .unwrap();
    }
    let destination = Plugin
        .build_recording_into(&name("board"), &config, &secrets, ledger.clone())
        .unwrap();
    let notes_name = name(&format!("live-assets-{stamp}"));
    let notes = onetaskgraph_local_md::Plugin
        .build(&notes_name, &json!({"root":root}), &secrets)
        .unwrap();
    let engine = Engine::new(
        vec![
            ConfiguredSource::Ready(ResolvedSource::adopt(notes_name.clone(), notes)),
            ConfiguredSource::Ready(ResolvedSource::adopt(name("board"), destination)),
        ],
        vec![notes_name.clone(), name("board")],
    );
    let mut created = Vec::new();
    let outcome: Result<(), String> = async {
        for (kind, id) in [("task", "T"), ("document", "D")] {
            let request = CopyRequest {
                items: CopyItems::new(vec![format!("{notes_name}:{id}").parse().unwrap()]).unwrap(),
                scope: if kind == "task" {
                    CopyScope::Tasks
                } else {
                    CopyScope::Documents
                },
                destination: name("board"),
                match_by: None,
                recreate: false,
                create: true,
                dry_run: false,
            };
            let report = engine
                .copy(&request)
                .await
                .map_err(|error| error.to_string())?;
            let target = report.items[0].destination().unwrap().clone();
            created.push((kind, target.clone()));
            let item = if kind == "task" {
                let response = engine
                    .task(&target)
                    .await
                    .map_err(|error| error.to_string())?;
                let item = &response.items[0].item;
                (
                    item.content.clone(),
                    item.metadata.clone(),
                    item.url.clone(),
                )
            } else {
                let response = engine
                    .document(&target)
                    .await
                    .map_err(|error| error.to_string())?;
                let item = &response.items[0].item;
                (
                    item.content.clone(),
                    item.metadata.clone(),
                    item.url.clone(),
                )
            };
            journey::say(&format!(
                "created disposable {kind}: {target}; issue URL {:?}",
                item.2
            ));
            let uploads = AssetUploads::read(&item.1)?.ok_or("missing uploaded assets")?;
            if uploads.0.len() != 2 {
                return Err("expected two uploaded images".into());
            }
            let content = item.0.as_deref().ok_or("missing content")?;
            for (file, alt) in [
                ("one.png", "First written alt"),
                ("two.png", "Second written alt"),
            ] {
                let name = onetaskgraph_plugin_api::AssetName::new(file)?;
                let url = &uploads.0.get(&name).ok_or("missing asset")?.url;
                if !url.starts_with("https://github.com/user-attachments/assets/")
                    || !content.contains(&format!("![{alt}]({url})"))
                {
                    return Err(format!("unrewritten or wrong attachment URL {url}"));
                }
                // A gateway error from GitHub's attachment host says nothing about the image:
                // one run met a `504` here for an asset the upload's own verification had
                // just read back `200`. So a 5xx is asked again, a few times, and any other
                // answer — a `404` above all — decides at once.
                let mut attempt = 0;
                let response = loop {
                    attempt += 1;
                    let response = reqwest::Client::new()
                        .get(url)
                        .header("user-agent", "onetaskgraph-live-assets")
                        .bearer_auth(token)
                        .send()
                        .await
                        .map_err(|error| error.to_string())?;
                    journey::say(&format!(
                        "authenticated live rendering read {file} {url}: HTTP {}",
                        response.status()
                    ));
                    if !response.status().is_server_error() || attempt == 4 {
                        break response;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2 * attempt)).await;
                };
                if !response.status().is_success() {
                    return Err(format!("broken live image {file}: {}", response.status()));
                }
            }
            // Engine::copy already bound the local record to this disposable destination.
            engine
                .end_command()
                .await
                .map_err(|error| error.to_string())?;
            let before = ledger.attachment_responses().len();
            let mut again = request;
            again.create = false;
            engine
                .copy(&again)
                .await
                .map_err(|error| error.to_string())?;
            if ledger.attachment_responses()[before..]
                .iter()
                .any(|response| {
                    response.operation
                        == onetaskgraph_github_projects::accounting::AttachmentOperation::Upload
                })
            {
                return Err("unchanged live re-copy uploaded again".into());
            }
            engine
                .end_command()
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
    .await;
    for response in ledger.attachment_responses() {
        journey::say(&format!(
            "live asset {} {} {}: HTTP {}",
            response.name,
            if response.operation
                == onetaskgraph_github_projects::accounting::AttachmentOperation::Upload
            {
                "upload"
            } else {
                "verification"
            },
            response.url,
            response.status
        ));
    }
    let cleanup = Plugin
        .build_recording_into(&name("board"), &config, &secrets, ledger.clone())
        .unwrap();
    let mut failures = Vec::new();
    for (kind, id) in &created {
        let native = id.native.clone();
        // Read only this disposable issue's board memberships, to report each created item.
        let answer=reqwest::Client::new().post("https://api.github.com/graphql").header("user-agent","onetaskgraph-live-assets").bearer_auth(token)
            .json(&json!({"query":"query($id:ID!){node(id:$id){...on Issue{projectItems(first:100){nodes{id project{id}}}}}}","variables":{"id":native.0}})).send().await;
        if let Ok(response) = answer
            && let Ok(answer) = response.json::<Value>().await
        {
            journey::say(&format!(
                "disposable {id} board memberships before removal: {}",
                answer["data"]["node"]["projectItems"]["nodes"]
            ));
        }
        let result = if *kind == "task" {
            cleanup.delete_task(&native).await
        } else {
            cleanup.delete_document(&native).await
        };
        journey::say(&format!(
            "removed disposable {kind} issue {id} and its board item: {result:?}"
        ));
        if let Err(error) = result {
            failures.push(error.to_string());
        }
    }
    assert!(
        failures.is_empty(),
        "live cleanup failures: {failures:?}; outcome {outcome:?}"
    );
    assert!(outcome.is_ok(), "live asset journey: {outcome:?}");
}

#[tokio::test]
async fn board_admission_rejects_non_success_even_with_valid_board_json() {
    use std::io::{BufRead, Read, Write};
    let complete = json!({"data":{"boardFields":{"projectV2":{"id":"fixture-board","fields":{"nodes":[{"name":"Status","options":[{"name":"Todo"}]}]}}}}});
    for (status, answer, expected) in [
        (200, complete.clone(), None),
        (503, complete, Some("503")),
        (200, json!({}), Some("missing nominated board id")),
        (
            200,
            json!({"data":{"boardFields":{"projectV2":{"id":"fixture-board"}}}}),
            Some("missing nominated board fields"),
        ),
        (
            200,
            json!({"data":{"boardFields":{"projectV2":{"id":"fixture-board","fields":{"nodes":[]}}}}}),
            Some("missing nominated board Status field"),
        ),
        (
            200,
            json!({"data":{"boardFields":{"projectV2":{"id":"fixture-board","fields":{"nodes":[{"name":"Status","options":[]}]}}}}}),
            Some("missing nominated board Status option"),
        ),
        (
            200,
            json!({"data":{"boardFields":{"projectV2":{"id":"fixture-board","fields":{"nodes":[{"name":"Status","options":[{"name":null}]}]}}}}}),
            Some("missing nominated board Status option"),
        ),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut headers = String::new();
            let mut length = None;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(number) = line.to_ascii_lowercase().strip_prefix("content-length: ") {
                    length = Some(number.trim().parse::<usize>().unwrap());
                }
                headers.push_str(&line);
            }
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token")
            );
            let mut bytes = vec![0; length.unwrap()];
            reader.read_exact(&mut bytes).unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["query"], graphql::BOARD_FIELDS);
            let body = answer.to_string();
            write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        });
        let ledger = Accounting::new();
        let result =
            nominated_status_option(&endpoint, "test-token", "fixture-owner", 1, &ledger).await;
        server.join().unwrap();
        let snapshot = ledger.snapshot();
        assert_eq!(snapshot.requests().len(), 1);
        if status == 200 {
            if let Some(expected) = expected {
                assert!(result.unwrap_err().contains(expected));
            } else {
                assert_eq!(result.unwrap(), "Todo");
            }
            assert_eq!(snapshot.requests()[0].outcome(), Outcome::Answered);
        } else {
            assert!(result.unwrap_err().contains(expected.unwrap()));
            assert_eq!(snapshot.requests()[0].outcome(), Outcome::Refused);
        }
    }
}

#[cfg(unix)]
#[test]
fn malformed_live_demand_is_refused_before_admission() {
    use std::os::unix::ffi::OsStringExt;
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "disposable_task_and_document_asset_copies",
            "--exact",
            "--nocapture",
        ])
        .env(
            "ONETASKGRAPH_LIVE_REQUIRED",
            std::ffi::OsString::from_vec(vec![0xff]),
        )
        .env_remove("GH_PROJECTS_OWNER")
        .env_remove("GH_PROJECTS_NUMBER")
        .env_remove("GH_PROJECTS_TOKEN")
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "malformed demand became an optional skip"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("ONETASKGRAPH_LIVE_REQUIRED must be Unicode")
    );
}
