//! Planning spike: ordinary issue lifecycles through the real CLI and Linear plugin.
use onetaskgraph_e2e_support::{
    common::Sandbox,
    fixtures::{dataset, document, linear_workspace},
    onevcs::OnevcsHome,
};
use serde_json::{Value, json};
use std::sync::{Arc, Barrier};

#[test]
fn measure_linear_projection_spike() {
    let Ok(output) = std::env::var("LINEAR_PROJECTION_SPIKE_OUTPUT") else {
        return;
    };
    let started = std::time::Instant::now();
    let host = Sandbox::new();
    let mut data = dataset();
    data["tasks"]=json!((0..2000).map(|n|json!({"id":format!("BG-{n}"),"title":format!("Synthetic background {n}"),"content":"Public example.","status":{"category":"todo","name":"Todo"},"labels":[]})).collect::<Vec<_>>());
    data["task_dependencies"] = json!([]);
    let (config, workspace) = linear_workspace(&host, data);
    let barrier = Arc::new(Barrier::new(10));
    let managers: Vec<_> = (0..10)
        .map(|manager| {
            let config = config.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let sandbox = Sandbox::new();
                let mut home = OnevcsHome::new(&sandbox);
                home.public("synthco", "public-widgets");
                let record = home.recorded(&sandbox);
                let mut configured: Value = serde_json::from_str(&document(
                    &json!({"linear":{"plugin":"linear","config":config}}),
                ))
                .unwrap();
                configured["write_policy"] = record.write_policy().clone();
                sandbox.project_document(&configured.to_string());
                let run = |args: &[&str]| {
                    let output = sandbox
                        .command()
                        .env("LINEAR_API_KEY", "test-token")
                        .env("ONEVCS_HOME", home.path())
                        .args(args)
                        .assert()
                        .success()
                        .get_output()
                        .clone();
                    serde_json::from_slice::<Value>(&output.stdout).unwrap()
                };
                let mut tickets = Vec::new();
                barrier.wait();
                for offset in 0..10 {
                    let n = manager * 10 + offset;
                    let title = format!("Review public example {n}");
                    let identity = format!("human.item_id=\"linear-{n}\"");
                    let created = run(&[
                        "task",
                        "create",
                        "linear",
                        "--project",
                        "P-1",
                        "--title",
                        &title,
                        "--repository",
                        "github.com/synthco/public-widgets",
                        "--metadata",
                        &identity,
                        "--json",
                    ]);
                    let row = &created["items"][0];
                    let id = row["id"].as_str().unwrap();
                    let url = row["item"]["url"].as_str().unwrap();
                    run(&["task", "update", id, "--priority", "high", "--json"]);
                    assert!(
                        url.starts_with("https://linear.app/") && url.contains("/issue/"),
                        "{created}"
                    );
                    assert_eq!(row["item"]["project"], "P-1");
                    let state = if n % 3 == 0 { "done" } else { "cancelled" };
                    let metadata = if n % 3 == 0 {
                        "human.state=\"completed\""
                    } else {
                        "human.state=\"dismissed\""
                    };
                    let mut args = vec![
                        "task",
                        "update",
                        id,
                        "--status",
                        state,
                        "--metadata",
                        metadata,
                        "--json",
                    ];
                    if n % 3 == 2 {
                        args.extend([
                            "--metadata",
                            "human.reason=\"Superseded by another example.\"",
                        ]);
                    }
                    let resolved = if std::env::var_os("PROJECTION_SPIKE_SKIP_RESOLVE").is_some() {
                        json!({"task":created["items"][0]["item"].clone()})
                    } else {
                        run(&args)
                    };
                    assert_eq!(
                        resolved["task"]["status"]["category"], state,
                        "terminal projection did not reach Linear"
                    );
                    assert_eq!(
                        resolved["task"]["metadata"].get("human.reason").is_some(),
                        n % 3 == 2
                    );
                    if offset == 0 {
                        let retry = run(&args);
                        assert_eq!(retry["written"], json!([]), "terminal retry must not write");
                        assert_eq!(retry["id"], json!(id));
                    }
                    tickets
                        .push(json!({"id":id,"url":url,"terminal":state,"reason_supplied":n%3==2}));
                }
                json!({"manager":manager,"tickets":tickets,"screening_calls":record.calls().len()})
            })
        })
        .collect();
    let managers: Vec<_> = managers.into_iter().map(|h| h.join().unwrap()).collect();
    let raw: Vec<_> = workspace
        .served()
        .iter()
        .map(|(query, variables)| json!({"query":query,"variables":variables}))
        .collect();
    let report = json!({"schema_version":1,"board_items_before":2000,"tasks":100,"concurrent_managers":10,"managers":managers,"requests":raw.len(),"raw":raw,"elapsed_seconds":started.elapsed().as_secs_f64(),"points":"not modeled: this plugin exposes request accounting, not GitHub GraphQL points"});
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("linear projection spike: {} requests", report["requests"]);
}
