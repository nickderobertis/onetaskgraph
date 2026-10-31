//! Disposable asset copies through Engine::copy, under the existing live lane admission.
use onetaskgraph_core::{
    ConfiguredSource, CopyItems, CopyRequest, CopyScope, Engine, ResolvedSource,
};
use onetaskgraph_e2e_support::{common::Sandbox, images};
use onetaskgraph_github_projects::{
    Plugin,
    accounting::{Accounting, Endpoint, Method, Outcome, RateLimit, Request, ResponseOutcome},
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

/// How [`read_live_rendering`] asks again after a gateway error: up to `attempts` reads in
/// all, sleeping `pause` times the attempt number after each 5xx.
#[derive(Clone, Copy, Debug)]
struct RenderingRetry {
    attempts: u32,
    pause: std::time::Duration,
}

/// The live journey's own schedule: four reads, two, four and six seconds apart.
const RENDERING_RETRY: RenderingRetry = RenderingRetry {
    attempts: 4,
    pause: std::time::Duration::from_secs(2),
};

/// Reads one uploaded image back with the credential, the way a reader of the rendering does,
/// and fails unless it answers success.
///
/// A gateway error from GitHub's attachment host says nothing about the image: one run met
/// a `504` here for an asset the upload's own verification had just read back `200`. So a
/// 5xx is asked again, up to `retry.attempts` reads in all, and any other answer — a `404`
/// above all — decides at once.
async fn read_live_rendering(
    url: &str,
    token: &str,
    file: &str,
    retry: RenderingRetry,
    ledger: &Accounting,
) -> Result<(), String> {
    let mut attempt = 0;
    let response = loop {
        attempt += 1;
        let sending = reqwest::Client::new()
            .get(url)
            .header("user-agent", "onetaskgraph-live-assets")
            .bearer_auth(token)
            .send()
            .await;
        let request = Request::rest(
            Endpoint::parse(Method::Get, "/user-attachments/assets/{asset}").unwrap(),
        );
        let response = match sending {
            Ok(response) => response,
            Err(error) => {
                ledger.record(
                    request
                        .finished(Outcome::Refused, RateLimit::default())
                        .with_execution(None, ResponseOutcome::TransportError, attempt),
                );
                return Err(error.to_string());
            }
        };
        let limits = RateLimit::read(|header| {
            response
                .headers()
                .get(header)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        });
        let status = response.status();
        ledger.record(
            request
                .finished(Outcome::of_response(status, limits.exhausted(), ""), limits)
                .with_execution(
                    None,
                    if status.is_success() {
                        ResponseOutcome::Success
                    } else {
                        ResponseOutcome::HttpError
                    },
                    attempt,
                ),
        );
        journey::say(&format!(
            "authenticated live rendering read {file} {url}: HTTP {}",
            response.status()
        ));
        if !response.status().is_server_error() || attempt >= retry.attempts {
            break response;
        }
        tokio::time::sleep(retry.pause * attempt).await;
    };
    if !response.status().is_success() {
        return Err(format!("broken live image {file}: {}", response.status()));
    }
    Ok(())
}

/// Record each raw asset-lane query before interpreting its response, including refusals.
async fn asset_graphql(
    endpoint: &str,
    token: &str,
    query: &str,
    variables: &Value,
    ledger: &Accounting,
) -> Result<Value, String> {
    // Held to this session's own board before anything is sent: see `journey::run_board`.
    journey::run_board::admit(query, variables)?;
    let document = ledger.priced_document(query);
    let sending = reqwest::Client::new()
        .post(endpoint)
        .header("user-agent", "onetaskgraph-live-assets")
        .bearer_auth(token)
        .json(&json!({"query":document,"variables":variables}))
        .send()
        .await;
    let response = match sending {
        Ok(response) => response,
        Err(error) => {
            ledger.record(
                Request::graphql(&document, variables, None, None)
                    .finished(Outcome::Refused, RateLimit::default())
                    .with_execution(
                        Some(document.into_owned()),
                        ResponseOutcome::TransportError,
                        1,
                    ),
            );
            return Err(error.to_string());
        }
    };
    let limits = RateLimit::read(|header| {
        response
            .headers()
            .get(header)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    let status = response.status();
    let parsed = response.json::<Value>().await;
    let cost = parsed
        .as_ref()
        .ok()
        .and_then(|answer| answer["data"]["rateLimit"]["cost"].as_u64());
    let response_outcome = ResponseOutcome::of_graphql_reply(status, parsed.as_ref().ok());
    let graphql_error = response_outcome == ResponseOutcome::GraphqlError;
    let outcome = if graphql_error {
        Outcome::Refused
    } else {
        Outcome::of_response(status, limits.exhausted(), "")
    };
    ledger.record(
        Request::graphql(&document, variables, None, cost)
            .finished(outcome, limits)
            .with_execution(Some(document.into_owned()), response_outcome, 1),
    );
    if !status.is_success() {
        return Err(format!("nominated board read returned HTTP {status}"));
    }
    let answer = parsed.map_err(|error| error.to_string())?;
    journey::run_board::learn(&answer);
    if graphql_error {
        return Err("asset query refused by GraphQL".into());
    }
    if response_outcome == ResponseOutcome::TransportError {
        return Err("asset query returned an invalid GraphQL envelope".into());
    }
    Ok(answer)
}

async fn nominated_status_option(
    endpoint: &str,
    token: &str,
    owner: &str,
    number: u32,
    ledger: &Accounting,
) -> Result<String, String> {
    let variables = json!({"owner":owner,"number":number,"nestedFirst":100});
    let answer = asset_graphql(endpoint, token, graphql::BOARD_FIELDS, &variables, ledger).await?;
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

/// How long this entry point waits for the main one's contribution to complete, in seconds,
/// when `GITHUB_LIVE_MAIN_WAIT_SECONDS` does not say: within the one job both run in.
const MAIN_WAIT_SECONDS: u64 = 25 * 60;

/// Wait for the main entry point's record to be complete before this one makes a request.
///
/// The two run as separate Nx tasks at once — nothing of this suite's loopback binaries needs
/// the main session, so they no longer wait behind the plugin's whole test target — and the
/// main record completing, which it does only after its own cleanup, is what keeps the lane's
/// credentialed writes in one process at a time. A main entry point that ended without completing
/// its record, or one that has not completed it by the bound, failed or never ran, and this one
/// refuses before spending anything.
async fn wait_for_current_main(
    directory: &std::path::Path,
    current: &onetaskgraph_github_live::quota::CurrentSession<'_>,
) -> Result<(), String> {
    let seconds = match std::env::var("GITHUB_LIVE_MAIN_WAIT_SECONDS") {
        Ok(value) => value.parse::<u64>().map_err(|_| {
            format!("GITHUB_LIVE_MAIN_WAIT_SECONDS must be whole seconds, not {value:?}")
        })?,
        Err(std::env::VarError::NotPresent) => MAIN_WAIT_SECONDS,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("GITHUB_LIVE_MAIN_WAIT_SECONDS must be Unicode".to_owned());
        }
    };
    let ended =
        journey::quota::ended_marker(directory, onetaskgraph_github_live::quota::EntryPoint::Main);
    let deadline = std::time::Instant::now()
        .checked_add(std::time::Duration::from_secs(seconds))
        .ok_or_else(|| {
            format!(
                "GITHUB_LIVE_MAIN_WAIT_SECONDS is {seconds}, past any deadline this clock holds"
            )
        })?;
    loop {
        match onetaskgraph_github_live::quota::require_current_main(directory, current) {
            Ok(()) => return Ok(()),
            // Its marker follows its final flush, so the record now is the record it left.
            Err(_) if ended.exists() => {
                return onetaskgraph_github_live::quota::require_current_main(directory, current)
                    .map_err(|error| {
                        format!("{error}; the main entry point ended without completing it")
                    });
            }
            Err(error) if std::time::Instant::now() >= deadline => {
                return Err(format!(
                    "{error}, {seconds} seconds after this entry point began waiting for it; the \
                     main contribution is onetaskgraph-github-projects:test's live journey, which \
                     runs beside this one in the same `test budgets` invocation"
                ));
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_secs(5)).await,
        }
    }
}

#[tokio::test]
async fn disposable_task_and_document_asset_copies() {
    if !onetaskgraph_github_live::quota::live_selected().unwrap() {
        journey::say(
            "live asset journey did not run: existing selection policy did not select the lane",
        );
        return;
    }
    let endpoints = journey::Endpoints::configured().unwrap();
    let attachment_prefix = endpoints.attachment_prefix();
    let graphql_endpoint = endpoints.graphql.clone();
    let rest_host = endpoints.rest_host.clone();
    journey::against(endpoints);
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
    let ledger = Arc::new(Accounting::with_query_prices());
    let _recording = journey::quota::start(
        onetaskgraph_github_live::quota::EntryPoint::Assets,
        ledger.clone(),
        onetaskgraph_github_live::quota::reference_workload(),
    )
    .unwrap();
    if let Some(directory) = std::env::var_os("GITHUB_LIVE_SESSION_DIR") {
        use onetaskgraph_github_live::quota::{
            CurrentSession, WORKSPACE_VERSION, reference_workload,
        };
        let id = std::env::var("GITHUB_LIVE_SESSION_ID").unwrap();
        let revision = std::env::var("GITHUB_LIVE_SOURCE_REVISION").unwrap();
        let workload = reference_workload();
        let current = CurrentSession::new(&id, &revision, WORKSPACE_VERSION, &workload).unwrap();
        wait_for_current_main(std::path::Path::new(&directory), &current)
            .await
            .unwrap_or_else(|error| panic!("current main contribution unavailable: {error}"));
    }
    let admitted = journey::budget::precondition_session(
        token,
        &rest_host,
        &ledger,
        onetaskgraph_github_live::quota::EntryPoint::Assets,
    )
    .await
    .unwrap_or_else(|declined| declined.refuse());
    journey::quota::phase(onetaskgraph_github_live::quota::Phase::AssetSetup).unwrap();
    // This session's own board, copied from the nominated template and deleted at its end.
    let send = async |document, _what, variables| {
        asset_graphql(&graphql_endpoint, token, document, &variables, &ledger).await
    };
    let board_stamp = writer.stamp(onetaskgraph_live::artifact::now_micros());
    let board = journey::run_board::copy(&send, &owner, number, &board_stamp)
        .await
        .unwrap_or_else(|error| panic!("the live asset journey cannot copy its board: {error}"));
    let board_guard = journey::run_board::Guard::new(token, &board);
    let number = board.number;
    let option = nominated_status_option(&graphql_endpoint, token, &owner, number, &ledger)
        .await
        .unwrap();
    journey::budget::recheck_remaining(
        &admitted,
        &ledger,
        journey::budget::assets_remaining_design_points(),
    )
    .await
    .unwrap_or_else(|declined| declined.refuse());
    let config = journey::source_config(lane::live_write_config(
        &owner,
        number,
        &repository,
        &option,
    ));
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
            let bytes = images::png(stamp + offset + index, size);
            let workload = onetaskgraph_github_live::quota::reference_workload();
            let position = usize::try_from(offset * 2 + index).unwrap();
            assert_eq!(
                workload.asset_target_bytes[position],
                u64::try_from(size).unwrap()
            );
            assert_eq!(
                workload.asset_bytes[position],
                u64::try_from(bytes.len()).unwrap()
            );
            assert!(bytes.len().abs_diff(size) <= size * images::PNG_SIZE_TOLERANCE_PERCENT / 100);
            std::fs::write(directory.join(file), bytes).unwrap();
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
            journey::quota::phase(if kind == "task" {
                onetaskgraph_github_live::quota::Phase::AssetTask
            } else {
                onetaskgraph_github_live::quota::Phase::AssetDocument
            })?;
            let request = CopyRequest {
                items: CopyItems::new(vec![format!("{notes_name}:{id}").parse().unwrap()]).unwrap(),
                scope: if kind == "task" {
                    CopyScope::Tasks
                } else {
                    CopyScope::Documents
                },
                destination: name("board"),
                target_task: None,
                match_by: None,
                recreate: false,
                create: true,
                dry_run: false,
                comments: false,
                no_link: false,
                expect: None,
                status_policy: onetaskgraph_core::StatusPolicy::Source,
                statuses: Vec::new(),
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
                if !url.starts_with(&attachment_prefix)
                    || !content.contains(&format!("![{alt}]({url})"))
                {
                    return Err(format!("unrewritten or wrong attachment URL {url}"));
                }
                read_live_rendering(url, token, file, RENDERING_RETRY, &ledger).await?;
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
    // Recording failures must not prevent removal of items this invocation created.
    let cleanup_recording = journey::quota::phase(onetaskgraph_github_live::quota::Phase::Cleanup);
    let cleanup_budget = journey::budget::recheck_remaining(
        &admitted,
        &ledger,
        journey::budget::assets_remaining_design_points(),
    )
    .await;
    let cleanup = Plugin
        .build_recording_into(&name("board"), &config, &secrets, ledger.clone())
        .unwrap();
    let mut failures = Vec::new();
    for (kind, id) in &created {
        let native = id.native.clone();
        // Read only this disposable issue's board memberships, to report each created item.
        let answer = asset_graphql(&graphql_endpoint, token,
            "query($id:ID!){node(id:$id){...on Issue{projectItems(first:100){nodes{id project{id}}}}}}",
            &json!({"id":native.0}), &ledger).await;
        if let Ok(answer) = answer {
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
    if let Err(error) = journey::run_board::delete(&send, &board).await {
        failures.push(error);
    }
    board_guard.disarm();
    assert!(
        failures.is_empty(),
        "live cleanup failures: {failures:?}; outcome {outcome:?}"
    );
    assert!(outcome.is_ok(), "live asset journey: {outcome:?}");
    cleanup_recording.unwrap();
    cleanup_budget.unwrap_or_else(|declined| declined.refuse());
    journey::quota::complete().unwrap();
}

/// Every `Status` option name the nominated board's field holds, in the board's order.
async fn status_option_names(
    endpoint: &str,
    token: &str,
    owner: &str,
    number: u32,
    ledger: &Accounting,
) -> Result<Vec<String>, String> {
    let variables = json!({"owner":owner,"number":number,"nestedFirst":100});
    let answer = asset_graphql(endpoint, token, graphql::BOARD_FIELDS, &variables, ledger).await?;
    let nodes = answer["data"]["boardFields"]["projectV2"]["fields"]["nodes"]
        .as_array()
        .ok_or("missing nominated board fields")?;
    let status = nodes
        .iter()
        .find(|field| field["name"] == "Status")
        .ok_or("missing nominated board Status field")?;
    Ok(status["options"]
        .as_array()
        .ok_or("missing nominated board Status options")?
        .iter()
        .filter_map(|option| option["name"].as_str().map(str::to_owned))
        .collect())
}

/// Every issue id the board's own `is:closed` filter answers, page by page: ids alone, so the
/// wait in [`wait_until_closed_filter_answers`] costs one point a read.
const CLOSED_ISSUE_IDS: &str = r#"query($owner:String!,$number:Int!,$after:String){
      closedIds:repositoryOwner(login:$owner){
        ... on ProjectV2Owner{projectV2(number:$number){
          items(first:100,after:$after,query:"is:closed"){
            nodes{content{... on Issue{id}}} pageInfo{hasNextPage endCursor}
          }
        }}
      }
    }"#;

/// How long the reconcile journey waits for the board's `is:closed` filter to answer the two
/// issues it has just closed.
///
/// GitHub answers `ProjectV2.items`, filtered or not, from an index that trails the board by
/// many minutes, while the item read by its own id is already there. A reconcile reads the
/// filter alone by design, so the journey waits for the index to answer what it closed
/// rather than assert against it, and a run that outwaits this fails naming the lag.
const CLOSED_INDEX_WAIT: std::time::Duration = std::time::Duration::from_secs(45 * 60);

/// How often the wait in [`wait_until_closed_filter_answers`] reads the filter again.
const CLOSED_INDEX_POLL: std::time::Duration = std::time::Duration::from_secs(60);

/// Waits until the board's `is:closed` filter answers every issue in `ids`, reading it whole
/// once per [`CLOSED_INDEX_POLL`] for at most [`CLOSED_INDEX_WAIT`], and fails naming the ids
/// it still lacks.
async fn wait_until_closed_filter_answers(
    endpoint: &str,
    token: &str,
    owner: &str,
    number: u32,
    ids: &[&str],
    ledger: &Accounting,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        let mut answered = std::collections::BTreeSet::new();
        let mut after: Option<String> = None;
        loop {
            let page = asset_graphql(
                endpoint,
                token,
                CLOSED_ISSUE_IDS,
                &json!({"owner":owner,"number":number,"after":after}),
                ledger,
            )
            .await?;
            let items = &page["data"]["closedIds"]["projectV2"]["items"];
            for node in items["nodes"]
                .as_array()
                .ok_or("the closed filter answered no items")?
            {
                if let Some(id) = node["content"]["id"].as_str() {
                    answered.insert(id.to_owned());
                }
            }
            if !items["pageInfo"]["hasNextPage"]
                .as_bool()
                .ok_or("the closed filter answered no hasNextPage")?
            {
                break;
            }
            after = Some(
                items["pageInfo"]["endCursor"]
                    .as_str()
                    .ok_or("the closed filter answered a next page and no cursor")?
                    .to_owned(),
            );
        }
        let missing: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| !answered.contains(*id))
            .collect();
        let waited = started.elapsed();
        if missing.is_empty() {
            journey::say(&format!(
                "the board's is:closed filter answered both closed tasks after {}s",
                waited.as_secs()
            ));
            return Ok(());
        }
        if waited >= CLOSED_INDEX_WAIT {
            return Err(format!(
                "GitHub's is:closed filter on the board still did not answer {missing:?} {}s \
                 after they were closed: its index trails the board by more than this journey \
                 waits, so the reconcile could not be asked to find them",
                waited.as_secs()
            ));
        }
        tokio::time::sleep(CLOSED_INDEX_POLL).await;
    }
}

/// `task status reconcile --dry-run` against the live lane's own board: one task closed as
/// completed outside the stack while its card still reads the board's first option, and one
/// written `done` through the source, card and closure in line.
///
/// It reads the board through its own `is:closed` filter, so what GitHub answers that filter
/// with is the fact this journey re-takes on every run: every item the filtered read returns is
/// a closed issue. The dry run then has to list the contradicting task, from its option to the
/// `done` option, and not the one in line — and writes nothing. It first waits for GitHub's
/// index of that filter to answer both tasks (see [`CLOSED_INDEX_WAIT`]). Both tasks are taken
/// back afterwards, whatever came of the assertions.
#[tokio::test]
async fn a_reconcile_dry_run_reads_the_live_board_through_its_closed_filter_alone() {
    if !onetaskgraph_github_live::quota::live_selected().unwrap() {
        journey::say(
            "live reconcile journey did not run: existing selection policy did not select the lane",
        );
        return;
    }
    // This binary's asset journey points the shared journey at its API, once per binary, so
    // this one reads the same configuration and points nothing: the two run in either order.
    let endpoints = journey::Endpoints::configured().unwrap();
    let graphql_endpoint = endpoints.graphql.clone();
    let rest_host = endpoints.rest_host.clone();
    let admission = lane::admit(&|variable| match std::env::var(variable) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => panic!("{variable} must be Unicode"),
    })
    .unwrap();
    let (token, owner, number, repository, writer) = match admission {
        lane::Admission::Skip(reason) => {
            journey::say(&format!("live reconcile journey did not run: {reason}"));
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
    let ledger = Arc::new(Accounting::with_query_prices());
    journey::budget::precondition_reconcile(token, &rest_host, &ledger)
        .await
        .unwrap_or_else(|declined| declined.refuse());
    // This session's own board, copied from the nominated template and deleted at its end.
    let send = async |document, _what, variables| {
        asset_graphql(&graphql_endpoint, token, document, &variables, &ledger).await
    };
    let board_stamp = writer.stamp(onetaskgraph_live::artifact::now_micros());
    let run_board = journey::run_board::copy(&send, &owner, number, &board_stamp)
        .await
        .unwrap_or_else(|error| {
            panic!("the live reconcile journey cannot copy its board: {error}")
        });
    let board_guard = journey::run_board::Guard::new(token, &run_board);
    let number = run_board.number;
    let options = status_option_names(&graphql_endpoint, token, &owner, number, &ledger)
        .await
        .unwrap();
    let [first, done, ..] = options.as_slice() else {
        panic!("the nominated board's Status field holds fewer than two options: {options:?}");
    };
    let mut config = lane::live_write_config(&owner, number, &repository, first);
    for (key, value) in endpoints
        .source
        .as_ref()
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        config[key] = value.clone();
    }
    config["status_mapping"]["done"] = json!(done);
    config["status_mapping"]["cancelled"] = Value::Null;
    let secrets = lane::LiveSecret(token.to_owned().into());
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("notes");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    let stamp = onetaskgraph_live::artifact::now_micros();
    writer.check_clock(stamp, 1).unwrap();
    for (offset, id) in [(0, "LINE"), (1, "CONTRADICT")] {
        std::fs::write(
            root.join(format!("tasks/{id}.md")),
            format!(
                "---\ntitle: {}\nstatus: todo\n---\nA reconcile journey's task.\n",
                lane::artifact_title(writer, stamp + offset)
            ),
        )
        .unwrap();
    }
    let board = Plugin
        .build_recording_into(&name("board"), &config, &secrets, ledger.clone())
        .unwrap();
    let notes_name = name(&format!("live-reconcile-{stamp}"));
    let notes = onetaskgraph_local_md::Plugin
        .build(&notes_name, &json!({"root":root}), &secrets)
        .unwrap();
    let engine = Engine::new(
        vec![
            ConfiguredSource::Ready(ResolvedSource::adopt(notes_name.clone(), notes)),
            ConfiguredSource::Ready(ResolvedSource::adopt(name("board"), board)),
        ],
        vec![notes_name.clone(), name("board")],
    );
    let mut created: Vec<onetaskgraph_core::GlobalId> = Vec::new();
    let outcome: Result<(), String> = async {
        let request = CopyRequest {
            items: CopyItems::new(vec![
                format!("{notes_name}:LINE").parse().unwrap(),
                format!("{notes_name}:CONTRADICT").parse().unwrap(),
            ])
            .unwrap(),
            scope: CopyScope::Tasks,
            destination: name("board"),
            target_task: None,
            match_by: None,
            recreate: false,
            create: true,
            dry_run: false,
            comments: false,
            no_link: true,
            expect: None,
            status_policy: onetaskgraph_core::StatusPolicy::Source,
            statuses: Vec::new(),
        };
        let report = engine
            .copy(&request)
            .await
            .map_err(|error| error.to_string())?;
        for item in &report.items {
            created.push(item.destination().ok_or("a copied task")?.clone());
        }
        let [in_line, contradicting] = created.as_slice() else {
            return Err(format!("two tasks were created: {created:?}"));
        };
        engine
            .set_task_status(in_line, onetaskgraph_plugin_api::StatusCategory::Done)
            .await
            .map_err(|error| error.to_string())?;
        // Closed as a person or a merged change request closes one: the issue alone, its card
        // left where it was.
        asset_graphql(
            &graphql_endpoint,
            token,
            graphql::UPDATE_ISSUE,
            &json!({"input":{"id":contradicting.native.0,
                             "stateInput":{"value":"CLOSED","stateReason":"COMPLETED"}}}),
            &ledger,
        )
        .await?;
        engine
            .end_command()
            .await
            .map_err(|error| error.to_string())?;
        wait_until_closed_filter_answers(
            &graphql_endpoint,
            token,
            &owner,
            number,
            &[in_line.native.0.as_str(), contradicting.native.0.as_str()],
            &ledger,
        )
        .await?;
        // What GitHub answers the board's closed filter with: closed issues, and nothing else.
        let closed = asset_graphql(
            &graphql_endpoint,
            token,
            graphql::CLOSED_ITEMS,
            &json!({"owner":owner,"number":number,"first":100,"after":null,
                    "filter":"is:closed","nestedFirst":50,"duplicates":true}),
            &ledger,
        )
        .await?;
        let nodes = closed["data"]["closedItems"]["projectV2"]["items"]["nodes"]
            .as_array()
            .ok_or("the closed filter answered no items")?;
        for node in nodes {
            let content = &node["content"];
            if content["__typename"] == "Issue" && content["state"] != "CLOSED" {
                return Err(format!(
                    "the board's is:closed filter returned an open issue: {}",
                    content["id"]
                ));
            }
        }
        journey::say(&format!(
            "the board's is:closed filter returned {} item(s) on its first page, every issue \
             among them closed",
            nodes.len()
        ));
        let reconciled = engine
            .reconcile_statuses(&name("board"), true)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("the board does not declare reconcile_status")?;
        let listed = |id: &onetaskgraph_core::GlobalId| {
            reconciled.items.iter().find(|item| item.source == *id)
        };
        let item = listed(contradicting).ok_or("the contradicting task was not listed")?;
        if item.from.as_deref() != Some(first.as_str())
            || item.to.name != *done
            || item.outcome != onetaskgraph_core::ReconcileOutcome::WouldUpdate
        {
            return Err(format!("the contradicting task was listed as {item:?}"));
        }
        if listed(in_line).is_some() {
            return Err("the task in line was listed".into());
        }
        Ok(())
    }
    .await;
    let cleanup = Plugin
        .build_recording_into(&name("board"), &config, &secrets, ledger.clone())
        .unwrap();
    let mut failures = Vec::new();
    for id in &created {
        let result = cleanup.delete_task(&id.native).await;
        journey::say(&format!("removed reconcile journey task {id}: {result:?}"));
        if let Err(error) = result {
            failures.push(error.to_string());
        }
    }
    if let Err(error) = journey::run_board::delete(&send, &run_board).await {
        failures.push(error);
    }
    board_guard.disarm();
    assert!(
        failures.is_empty(),
        "live cleanup failures: {failures:?}; outcome {outcome:?}"
    );
    assert!(outcome.is_ok(), "live reconcile journey: {outcome:?}");
}

#[tokio::test]
async fn board_admission_rejects_non_success_even_with_valid_board_json() {
    use std::io::{BufRead, Read, Write};
    let complete = json!({"data":{"boardFields":{"projectV2":{"id":"fixture-board","fields":{"nodes":[{"name":"Status","options":[{"name":"Todo"}]}]}}}}});
    for (status, answer, expected) in [
        (200, complete.clone(), None),
        (503, complete, Some("503")),
        (
            200,
            json!({}),
            Some("asset query returned an invalid GraphQL envelope"),
        ),
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

#[tokio::test]
async fn raw_asset_queries_retain_actual_prices_and_failure_outcomes() {
    use std::io::{BufRead, Read, Write};
    for (status, body, expected_cost, expected_outcome) in [
        (
            200,
            r#"{"data":{"rateLimit":{"cost":7},"node":{"id":"fixture"}}}"#,
            Some(7),
            ResponseOutcome::Success,
        ),
        (
            200,
            r#"{"data":{"rateLimit":{"cost":3}},"errors":[{"message":"refused"}]}"#,
            Some(3),
            ResponseOutcome::GraphqlError,
        ),
        (
            503,
            r#"{"data":{"rateLimit":{"cost":4}}}"#,
            Some(4),
            ResponseOutcome::HttpError,
        ),
        (200, "unreadable", None, ResponseOutcome::TransportError),
        (
            200,
            r#"{"data":{"node":{"id":"fixture"}}}"#,
            None,
            ResponseOutcome::Success,
        ),
        (200, "null", None, ResponseOutcome::TransportError),
        (200, "[]", None, ResponseOutcome::TransportError),
        (200, "{}", None, ResponseOutcome::TransportError),
        (
            200,
            r#"{"data":null}"#,
            None,
            ResponseOutcome::TransportError,
        ),
        (
            200,
            r#"{"errors":false}"#,
            None,
            ResponseOutcome::TransportError,
        ),
        (
            200,
            r#"{"errors":[{}]}"#,
            None,
            ResponseOutcome::TransportError,
        ),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length: ") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(
                request["query"]
                    .as_str()
                    .unwrap()
                    .contains("rateLimit{cost}")
            );
            assert_eq!(request["variables"], json!({"id":"fixture"}));
            write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            request["query"].as_str().unwrap().to_owned()
        });
        let ledger = Accounting::with_query_prices();
        let result = asset_graphql(
            &endpoint,
            "test-token",
            "query($id:ID!){node(id:$id){id}}",
            &json!({"id":"fixture"}),
            &ledger,
        )
        .await;
        let sent = server.join().unwrap();
        assert_eq!(result.is_ok(), expected_outcome == ResponseOutcome::Success);
        let snapshot = ledger.snapshot();
        assert_eq!(snapshot.requests().len(), 1);
        let record = &snapshot.requests()[0];
        assert_eq!(record.document(), Some(sent.as_str()));
        assert_eq!(record.reported_query_cost(), expected_cost);
        assert_eq!(record.response_outcome(), expected_outcome);
        assert_eq!(record.attempt(), 1);
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    drop(listener);
    let ledger = Accounting::with_query_prices();
    assert!(
        asset_graphql(
            &endpoint,
            "test-token",
            "query{viewer{login}}",
            &json!({}),
            &ledger
        )
        .await
        .is_err()
    );
    let snapshot = ledger.snapshot();
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(snapshot.requests()[0].reported_query_cost(), None);
    assert_eq!(
        snapshot.requests()[0].response_outcome(),
        ResponseOutcome::TransportError
    );
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

/// A loopback attachment host answering each read with the next of `statuses`, then `200`
/// for as long as it is asked, counting every read that carried the credential.
fn attachment_host(statuses: Vec<u16>) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/user-attachments/assets/one",
        listener.local_addr().unwrap()
    );
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = reads.clone();
    let mut statuses = statuses.into_iter();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                headers.push_str(&line.to_ascii_lowercase());
            }
            assert!(headers.contains("authorization: bearer test-token"));
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let status = statuses.next().unwrap_or(200);
            write!(
                stream,
                "HTTP/1.1 {status} Fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        }
    });
    (url, reads)
}

/// The live journey's own retry bound, without its pauses.
const IMMEDIATE_RETRY: RenderingRetry = RenderingRetry {
    pause: std::time::Duration::ZERO,
    ..RENDERING_RETRY
};

#[tokio::test]
async fn a_rendering_read_rides_out_a_gateway_timeout_and_reads_the_image() {
    let (url, reads) = attachment_host(vec![504]);

    let ledger = Accounting::new();
    let read = read_live_rendering(&url, "test-token", "one.png", IMMEDIATE_RETRY, &ledger).await;

    assert_eq!(read, Ok(()), "a 504 says nothing about the image");
    let snapshot = ledger.snapshot();
    assert_eq!(snapshot.requests().len(), 2);
    assert_eq!(
        snapshot.requests()[0].response_outcome(),
        ResponseOutcome::HttpError
    );
    assert_eq!(snapshot.requests()[0].attempt(), 1);
    assert_eq!(
        snapshot.requests()[1].response_outcome(),
        ResponseOutcome::Success
    );
    assert_eq!(snapshot.requests()[1].attempt(), 2);
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the timed-out read and the one that answered"
    );
}

#[tokio::test]
async fn a_rendering_read_that_keeps_meeting_server_errors_fails_after_its_bound() {
    let (url, reads) = attachment_host(vec![502, 503, 504, 500]);

    let ledger = Accounting::new();
    let read = read_live_rendering(&url, "test-token", "one.png", IMMEDIATE_RETRY, &ledger).await;

    assert_eq!(
        read,
        Err("broken live image one.png: 500 Internal Server Error".to_owned()),
        "it fails naming the last answer"
    );
    let snapshot = ledger.snapshot();
    assert_eq!(snapshot.requests().len(), RENDERING_RETRY.attempts as usize);
    for (index, request) in snapshot.requests().iter().enumerate() {
        assert_eq!(request.attempt(), u32::try_from(index + 1).unwrap());
        assert_eq!(request.response_outcome(), ResponseOutcome::HttpError);
    }
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        RENDERING_RETRY.attempts as usize,
        "and never asks a fifth time, although the host would answer 200"
    );
}

#[tokio::test]
async fn a_rendering_read_answered_not_found_fails_at_once() {
    let (url, reads) = attachment_host(vec![404]);

    let ledger = Accounting::new();
    let read = read_live_rendering(&url, "test-token", "one.png", IMMEDIATE_RETRY, &ledger).await;

    assert_eq!(
        read,
        Err("broken live image one.png: 404 Not Found".to_owned()),
        "a missing image is a broken rendering"
    );
    let snapshot = ledger.snapshot();
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(snapshot.requests()[0].attempt(), 1);
    assert_eq!(
        snapshot.requests()[0].response_outcome(),
        ResponseOutcome::HttpError
    );
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "and is not asked again, although the host would answer 200"
    );
}

#[tokio::test]
async fn a_rendering_transport_failure_retains_its_rest_call() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/user-attachments/assets/one",
        listener.local_addr().unwrap()
    );
    drop(listener);
    let ledger = Accounting::new();
    assert!(
        read_live_rendering(&url, "test-token", "one.png", IMMEDIATE_RETRY, &ledger)
            .await
            .is_err()
    );
    let snapshot = ledger.snapshot();
    assert_eq!(snapshot.requests().len(), 1);
    let request = &snapshot.requests()[0];
    assert_eq!(
        request.budget(),
        onetaskgraph_github_projects::accounting::Budget::Rest
    );
    assert_eq!(request.response_outcome(), ResponseOutcome::TransportError);
    assert_eq!(request.attempt(), 1);
    assert_eq!(request.document(), None);
    assert_eq!(request.name(), "GET /user-attachments/assets/{asset}");
}

#[test]
fn credentialless_asset_entry_point_never_writes_live_evidence() {
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("session");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "disposable_task_and_document_asset_copies",
            "--exact",
            "--nocapture",
        ])
        .env("ONETASKGRAPH_LIVE_REQUIRED", "0")
        .env("GITHUB_LIVE_SESSION_DIR", &directory)
        .env("GITHUB_LIVE_SESSION_ID", "credentialless-test")
        .env("GITHUB_LIVE_SOURCE_REVISION", "a".repeat(40))
        .env_remove("GH_PROJECTS_OWNER")
        .env_remove("GH_PROJECTS_NUMBER")
        .env_remove("GH_PROJECTS_TOKEN")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("live asset journey did not run"));
    assert!(!directory.join("assets.json").exists());
}

#[test]
fn propagated_selection_prevents_the_asset_entry_point_from_opening_a_session() {
    for (selection, succeeds, message) in [
        (
            "not-selected",
            true,
            "existing selection policy did not select the lane",
        ),
        (
            "wrong",
            false,
            "GITHUB_LIVE_SELECTION must be run or not-selected",
        ),
    ] {
        let sandbox = Sandbox::new();
        let directory = sandbox.subdirectory("session");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "disposable_task_and_document_asset_copies",
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_LIVE_SELECTION", selection)
            .env("ONETASKGRAPH_LIVE_REQUIRED", "1")
            .env("GH_PROJECTS_TOKEN", "test-token")
            .env("GH_PROJECTS_OWNER", "fixture-owner")
            .env("GH_PROJECTS_NUMBER", "1")
            .env("GH_PROJECTS_REPOSITORY", "fixture-owner/repository")
            .env("GITHUB_LIVE_SESSION_DIR", &directory)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            succeeds,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains(message));
        assert!(!directory.join("assets.json").exists());
    }
}

#[test]
fn the_actual_asset_entry_point_refuses_missing_main_before_any_request() {
    // Waiting for the main entry point is bounded: with no bound, a refusal at once; with a few
    // seconds, a refusal only once they have passed; a bound that is not whole seconds, and one
    // past any deadline the clock holds, each refused by name. None of them spends a request,
    // and each leaves the entry point marked ended. A main entry point that ended without its
    // record is live_pair.rs's, which runs one.
    for (wait, refused_after, says) in [
        ("0", 0, "current main contribution unavailable"),
        ("3", 3, "3 seconds after this entry point began waiting"),
        (
            "soon",
            0,
            "GITHUB_LIVE_MAIN_WAIT_SECONDS must be whole seconds",
        ),
        (
            "18446744073709551615",
            0,
            "past any deadline this clock holds",
        ),
    ] {
        let sandbox = Sandbox::new();
        let directory = sandbox.subdirectory("session");
        std::fs::create_dir_all(&directory).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let started = std::time::Instant::now();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "disposable_task_and_document_asset_copies",
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_LIVE_SELECTION", "run")
            .env("GITHUB_LIVE_TEST_GRAPHQL", format!("{host}/graphql"))
            .env("GITHUB_LIVE_TEST_REST", &host)
            .env("ONETASKGRAPH_LIVE_REQUIRED", "1")
            .env("GH_PROJECTS_TOKEN", "test-token")
            .env("GH_PROJECTS_OWNER", "fixture-owner")
            .env("GH_PROJECTS_NUMBER", "1")
            .env("GH_PROJECTS_REPOSITORY", "fixture-owner/repository")
            .env("GITHUB_LIVE_SESSION_DIR", &directory)
            .env("GITHUB_LIVE_SESSION_ID", "missing-main-test")
            .env("GITHUB_LIVE_SOURCE_REVISION", "a".repeat(40))
            .env("GITHUB_LIVE_MAIN_WAIT_SECONDS", wait)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{wait}: {said}");
        assert!(
            said.contains("current main contribution unavailable"),
            "{wait}: {said}"
        );
        assert!(said.contains(says), "{wait}: {said}");
        assert!(
            started.elapsed() >= std::time::Duration::from_secs(refused_after),
            "{wait}: refused after {:?}",
            started.elapsed()
        );
        let record: onetaskgraph_github_live::quota::SessionRecord =
            serde_json::from_slice(&std::fs::read(directory.join("assets.json")).unwrap()).unwrap();
        assert!(!record.complete);
        assert!(
            record.calls.is_empty(),
            "missing main evidence must not spend another API request"
        );
        // The refused journey's own record is final, so it says it has ended.
        assert!(
            journey::quota::ended_marker(
                &directory,
                onetaskgraph_github_live::quota::EntryPoint::Assets
            )
            .exists(),
            "{wait}: the refused asset entry point left no ended marker beside its record"
        );
    }
}
