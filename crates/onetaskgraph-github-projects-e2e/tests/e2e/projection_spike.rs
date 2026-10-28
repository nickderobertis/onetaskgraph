//! Planning-only measurement of existing CLI writes and a warm plugin operation context.
//! No projection protocol or durable claim store is implemented by this harness.
use super::{asset_board, plan_copy};
use onetaskgraph_e2e_support::{
    common::Sandbox,
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
    onevcs::OnevcsHome,
};
use onetaskgraph_plugin_api::{
    ItemKind, ItemWrite, NativeId, PageRequest, SecretResolver, SourceName, SourcePlugin as _,
    StatusCategory, Task, TaskQuery, TaskUpdate,
};
use serde_json::{Value, json};
use std::sync::{Arc, Barrier};

struct Secrets(String);
impl SecretResolver for Secrets {
    fn get(&self, _: &str) -> Option<secrecy::SecretString> {
        Some(self.0.clone().into())
    }
}

fn capture(board: &GitHubBoardFields, token: &str, from: usize, phase: &str) -> Value {
    let requests = board.served_to(token);
    let raw: Vec<_> = requests[from..].iter().map(|(query, variables)| {
        let primary = onetaskgraph_github_projects::worst_case_point_cost(query).unwrap();
        let fields = if query.trim_start().starts_with("mutation") { asset_board::top_level_fields(query, variables) } else { 0 };
        json!({"operation":plan_copy::operation_name(query),"points":primary,"mutation_fields":fields,
            "secondary_points":if fields > 0 {5 * fields} else {1},"query":query,"variables":variables})
    }).collect();
    json!({"phase":phase,"requests":raw.len(),"points":raw.iter().map(|r|r["points"].as_u64().unwrap()).sum::<u64>(),
        "mutation_fields":raw.iter().map(|r|r["mutation_fields"].as_u64().unwrap()).sum::<u64>(),
        "secondary_points":raw.iter().map(|r|r["secondary_points"].as_u64().unwrap()).sum::<u64>(),"raw":raw})
}
fn task(n: usize) -> Task {
    serde_json::from_value(json!({"id":format!("human-{n}"),"title":format!("Review synthetic task {n}"),
        "content":"Choose the next public example.","status":{"category":"todo","name":"Todo"},"priority":"high",
        "labels":[],"project":"PROJECT-1","url":null,"created_at":null,"updated_at":null,
        "metadata":{"onetaskgraph.origin":format!("human-task:{n}"),"human.item_id":format!("human-{n}"),"human.owner":{"session":"synthetic","run":null,"plan":"example"},"human.state":"open"},
        "repositories":["github.com/synthco/public-widgets"]})).unwrap()
}
fn update(n: usize) -> TaskUpdate {
    let mut value = json!({"status":{"category":if n%3==0 {"done"} else {"cancelled"},"name":"terminal"},
        "metadata_set":{"human.state":if n%3==0 {"completed"} else {"dismissed"}}});
    if n % 3 == 2 {
        value["metadata_set"]["human.reason"] = json!("Superseded by another example.");
    }
    serde_json::from_value(value).unwrap()
}
fn sum(rows: &[Value], field: &str) -> u64 {
    rows.iter().map(|r| r[field].as_u64().unwrap()).sum()
}

fn workload(warm: bool) -> Value {
    let host = Sandbox::new();
    let mut items: Vec<_> = (0..1995).map(plan_copy::background).collect();
    items.push(json!({"item":"ITEM-PROJECT-1","id":"PROJECT-1","type":"Issue","title":"Synthetic plan","body":"Plan.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\"}\n-->","state":"OPEN","reason":null,"parent":null,"repo":"synthco/public-widgets","status":"Todo","origin":"","labels":[]}));
    let (mut config, board) = github_projects_with_items(&host, items);
    config["repository"] = json!("synthco/public-widgets");
    board.use_repository("synthco/public-widgets");
    board.use_canonical_issue_urls();
    board.file_new_items_without_status(true);
    let barrier = Arc::new(Barrier::new(10));
    let managers: Vec<_> = (0..10).map(|manager| {
        let config=config.clone(); let board=board.clone(); let barrier=barrier.clone();
        std::thread::spawn(move || {
            let sandbox=Sandbox::new();
            let mut home=OnevcsHome::new(&sandbox); home.public("synthco","public-widgets"); home.private("synthco","private-widgets");
            let boundary=home.recorded(&sandbox);
            let mut configured: Value=serde_json::from_str(&document(&json!({"board":{"plugin":"github-projects","config":config,"visibility":"public"}}))).unwrap();
            configured["write_policy"]=boundary.write_policy().clone(); sandbox.project_document(&configured.to_string());
            let body_path=sandbox.project().join("task-body.md");
            std::fs::write(&body_path,"Choose the next public example.").unwrap();
            let body_path=body_path.to_str().unwrap();
            let token=format!("test-token-projection-{manager}");
            let run=|args:&[&str]| {
                let output=sandbox.command().args(args).env("GITHUB_PROJECTS_FIXTURE_TOKEN",&token).env("ONEVCS_HOME",home.path()).assert().success().get_output().clone();
                serde_json::from_slice::<Value>(&output.stdout).unwrap()
            };
            let runtime=tokio::runtime::Runtime::new().unwrap();
            let source=onetaskgraph_github_projects::Plugin.build(&SourceName::new("board").unwrap(),&config,&Secrets(token.clone())).unwrap();
            let mut rows=Vec::new();
            barrier.wait();
            if warm {
                let before=board.served_to(&token).len();
                // The real preflight retains the project's resolved mapping for this context.
                runtime.block_on(source.check_status_write(ItemKind::Project,StatusCategory::Todo,Some(&NativeId("PROJECT-1".into())))).unwrap();
                rows.push(capture(&board,&token,before,"shared_project_preflight"));
            }
            for offset in 0..10 {
                let n=manager*10+offset;
                let before=board.served_to(&token).len();
                let item=task(n);
                let (id,url)=if warm {
                    let id=runtime.block_on(source.write_task(&ItemWrite{target:None,item,depends_on:vec![]})).unwrap();
                    let location=source.written_location(onetaskgraph_plugin_api::MetadataRecord::Task,&id).unwrap();
                    (id,serde_json::to_value(location).unwrap()["url"].as_str().unwrap().to_owned())
                } else {
                    let identity=format!("human.item_id=\"human-{n}\"");
                    let answer=run(&["task","create","board","--project","PROJECT-1","--title",&item.title,"--body-file",body_path,"--repository","github.com/synthco/public-widgets","--metadata",&identity,"--metadata","human.state=\"open\"","--metadata","human.owner={\"session\":\"synthetic\",\"run\":null,\"plan\":\"example\"}","--json"]);
                    let row=&answer["items"][0];
                    let id=row["id"].as_str().or_else(||row["task"].as_str()).unwrap_or_else(||panic!("create response: {answer}"));
                    (NativeId(id.strip_prefix("board:").unwrap_or(id).to_owned()),row["item"]["location"]["url"].as_str().or_else(||row["item"]["url"].as_str()).unwrap_or_else(||panic!("URL: {answer}")).to_owned())
                };
                assert!(url.starts_with("https://github.com/synthco/public-widgets/issues/"),"{url}");
                rows.push(capture(&board,&token,before,"create"));
                let created=rows.last_mut().unwrap(); created["task"]=json!(id.0); created["url"]=json!(url);
                if !warm {
                    let before=board.served_to(&token).len();
                    run(&["task","update",&format!("board:{}",id.0),"--priority","high","--json"]);
                    rows.push(capture(&board,&token,before,"required_priority"));
                }
                if warm && offset==0 {
                    // The create response was received by the plugin but discarded by its caller.
                    // Reconstruct only from the identity carried on the tracker, in a fresh client.
                    let fresh=onetaskgraph_github_projects::Plugin.build(&SourceName::new("board").unwrap(),&config,&Secrets(token.clone())).unwrap();
                    let before=board.served_to(&token).len();
                    let found=runtime.block_on(fresh.query_tasks(&TaskQuery{origin:Some(format!("human-task:{n}")),..TaskQuery::default()},&PageRequest{cursor:None,limit:100})).unwrap();
                    assert_eq!(found.items.len(),1,"visible completed create must recover exactly once");
                    assert_eq!(found.items[0].id,id);
                    assert_eq!(found.items[0].url.as_deref(),Some(url.as_str()));
                    rows.push(capture(&board,&token,before,"lost_create_response_recovery"));
                }
                let before=board.served_to(&token).len();
                let terminal=update(n);
                let skip_resolve=std::env::var_os("PROJECTION_SPIKE_SKIP_RESOLVE").is_some();
                if skip_resolve {} else if warm { runtime.block_on(source.update_task(&id,&terminal)).unwrap(); }
                else {
                    let qualified=format!("board:{}",id.0);
                    let state=if n%3==0 {"done"} else {"cancelled"};
                    let metadata=if n%3==0 {"human.state=\"completed\""} else {"human.state=\"dismissed\""};
                    let mut args=vec!["task","update",&qualified,"--status",state,"--metadata",metadata,"--json"];
                    if n%3==2 {args.extend(["--metadata","human.reason=\"Superseded by another example.\""]);}
                    run(&args);
                }
                rows.push(capture(&board,&token,before,"resolve"));
                assert_eq!(board.state(&id.0).0,"CLOSED","terminal projection did not close the tracker issue");
                let held=board.body(&id.0).to_string();
                assert_eq!(held.contains("Superseded by another example."),n%3==2);
                // A lost terminal response: discard the client's context and retry through the real plugin.
                if offset==0 {
                    let fresh=onetaskgraph_github_projects::Plugin.build(&SourceName::new("board").unwrap(),&config,&Secrets(token.clone())).unwrap();
                    let before=board.served_to(&token).len();
                    let retried=runtime.block_on(fresh.update_task(&id,&terminal)).unwrap();
                    assert!(retried.unwrap().written.is_empty(),"terminal retry wrote again");
                    rows.push(capture(&board,&token,before,"cold_terminal_retry"));
                    let before=board.served_to(&token).len();
                    runtime.block_on(fresh.update_task(&id,&terminal)).unwrap();
                    rows.push(capture(&board,&token,before,"warm_terminal_retry"));
                }
            }
            if !warm && manager==0 {
                for (phase,repositories) in [("private_route_refused",vec!["github.com/synthco/private-widgets"]),("mixed_route_refused",vec!["github.com/synthco/public-widgets","github.com/synthco/private-widgets"])] {
                    let before=board.served_to(&token).len();
                    let mut args=vec!["task","create","board","--project","PROJECT-1","--title","Synthetic private review","--classification","private","--json"];
                    for repository in &repositories {args.extend(["--repository",repository]);}
                    let failed=sandbox.command().args(&args).env("GITHUB_PROJECTS_FIXTURE_TOKEN",&token).env("ONEVCS_HOME",home.path()).assert().failure().get_output().clone();
                    let mut captured=capture(&board,&token,before,phase);
                    assert_eq!(captured["mutation_fields"],0,"private work reached a public destination");
                    captured["failure"]=serde_json::from_slice::<Value>(&failed.stdout).unwrap();
                    rows.push(captured);
                }
            }
            json!({"manager":manager,"rows":rows,"screening":boundary.calls().iter().map(|c|json!({"command":c.command,"exit":c.exit})).collect::<Vec<_>>()})
        })
    }).collect();
    let managers: Vec<_> = managers.into_iter().map(|h| h.join().unwrap()).collect();
    let rows: Vec<Value> = managers
        .iter()
        .flat_map(|m| m["rows"].as_array().unwrap().clone())
        .collect();
    let normal: Vec<Value> = rows
        .iter()
        .filter(|r| {
            matches!(
                r["phase"].as_str(),
                Some("shared_project_preflight" | "create" | "required_priority" | "resolve")
            )
        })
        .cloned()
        .collect();
    json!({"mode":if warm {"warm_plugin_prototype_unscreened"} else {"existing_cli_screened"},"board_items_before":2000,"tasks":100,"managers":managers,
        "normal_requests":sum(&normal,"requests"),"normal_points":sum(&normal,"points"),"points_per_item":sum(&normal,"points") as f64/100.0,
        "requests_including_retries":sum(&rows,"requests"),"points_including_retries":sum(&rows,"points"),"secondary_points":sum(&rows,"secondary_points")})
}
#[test]
fn measure_projection_spike() {
    let Ok(output) = std::env::var("PROJECTION_SPIKE_OUTPUT") else {
        return;
    };
    let started = std::time::Instant::now();
    let existing = workload(false);
    let warm = workload(true);
    let approval = [
        super::approval::spike_approval(false),
        super::approval::spike_approval(true),
    ];
    let report = json!({"approval":approval,"schema_version":1,"existing":existing,"prototype":warm,"elapsed_seconds":started.elapsed().as_secs_f64(),
        "limits":["No external create claim: ambiguous create must refuse, never blindly recreate.","Warm plugin prototype excludes engine routing and write screening; it is not project-human.","GitHub primary points use the plugin model per document; secondary mutation fields are charged individually."]});
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!(
        "projection spike: {}",
        json!({"existing_points":report["existing"]["normal_points"],"prototype_points":report["prototype"]["normal_points"],"elapsed_seconds":report["elapsed_seconds"]})
    );
}
