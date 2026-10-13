//! The real selection adapter passes the policy's exclusions and fresh job identity to Nx.
use onetaskgraph_e2e_support::common::Sandbox;
use serde_json::{Value, json};

#[test]
fn selection_adapter_propagates_exclusions_and_reserves_only_selected_sessions() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("selection");
    std::fs::create_dir_all(&root).unwrap();
    let recorder = root.join("nx.py");
    std::fs::write(&recorder, r#"import json, os, sys
from pathlib import Path
Path(os.environ['NX_RECORD']).write_text(json.dumps({'arguments':sys.argv[1:], 'selection':os.environ['GITHUB_LIVE_SELECTION'], 'id':os.environ.get('GITHUB_LIVE_SESSION_ID'), 'revision':os.environ.get('GITHUB_LIVE_SOURCE_REVISION'), 'directory':os.environ.get('GITHUB_LIVE_SESSION_DIR')}))
"#).unwrap();
    let runner = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lane_runner.py");
    for (case, policy, credential, selected, exclusion) in [
        (
            "selected",
            "printf '%s\\n' '--exclude=onetaskgraph-linear'",
            true,
            "run",
            Some("--exclude=onetaskgraph-linear"),
        ),
        (
            "version",
            "printf '%s\\n' '--exclude=onetaskgraph-github-projects,onetaskgraph-linear'",
            true,
            "not-selected",
            Some("--exclude=onetaskgraph-github-projects,onetaskgraph-linear"),
        ),
        ("credentialless", "exit 0", false, "run", None),
        ("unanswered", "exit 1", true, "run", None),
    ] {
        let helper = root.join(format!("{case}.sh"));
        std::fs::write(&helper, policy).unwrap();
        let record = root.join(format!("{case}.json"));
        let directory = root.join(format!("{case}-session"));
        let mut command = std::process::Command::new("python3");
        command
            .arg(&runner)
            .arg("select")
            .arg(&helper)
            .arg("python3")
            .arg(&recorder)
            .args(["affected", "-t", "test", "budgets"])
            .env("NX_RECORD", &record)
            .env("GITHUB_LIVE_SESSION_ID", case)
            .env("GITHUB_LIVE_SOURCE_REVISION", "b".repeat(40))
            .env("GITHUB_LIVE_SESSION_DIR", &directory)
            .env_remove("GH_PROJECTS_TOKEN");
        if credential {
            command.env("GH_PROJECTS_TOKEN", "fixture-credential");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let received: Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
        assert_eq!(received["selection"], selected);
        assert_eq!(received["id"], case);
        assert_eq!(received["revision"], "b".repeat(40));
        let mut arguments = json!(["affected", "-t", "test", "budgets"]);
        if let Some(exclusion) = exclusion {
            arguments.as_array_mut().unwrap().push(json!(exclusion));
        }
        assert_eq!(received["arguments"], arguments);
        assert_eq!(directory.exists(), selected == "run" && credential);
        if directory.exists() {
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
            let duplicate = command.output().unwrap();
            assert!(!duplicate.status.success());
            assert!(
                String::from_utf8_lossy(&duplicate.stderr)
                    .contains("GitHub live collection refused")
            );
        }
    }
}

#[test]
fn real_nx_orders_both_contributions_and_keeps_one_main_task() {
    let sandbox = Sandbox::new();
    let graph = sandbox.subdirectory("nx").join("session-graph.json");
    std::fs::create_dir_all(graph.parent().unwrap()).unwrap();
    let output = std::process::Command::new("bun")
        .args([
            "x",
            "nx",
            "run-many",
            "-p",
            "onetaskgraph-github-projects,onetaskgraph-github-projects-e2e",
            "-t",
            "test,budgets",
        ])
        .arg(format!("--graph={}", graph.display()))
        .env("NX_DAEMON", "false")
        .current_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let graph: Value = serde_json::from_slice(&std::fs::read(graph).unwrap()).unwrap();
    let tasks = graph["tasks"]["tasks"].as_object().unwrap();
    assert_eq!(
        tasks
            .keys()
            .filter(|key| *key == "onetaskgraph-github-projects:test")
            .count(),
        1
    );
    let dependencies = &graph["tasks"]["dependencies"];
    assert!(
        dependencies["onetaskgraph-github-projects-e2e:test"]
            .as_array()
            .unwrap()
            .contains(&json!("onetaskgraph-github-projects:test"))
    );
    let budgets = dependencies["onetaskgraph-github-projects-e2e:budgets"]
        .as_array()
        .unwrap();
    assert!(budgets.contains(&json!("onetaskgraph-github-projects-e2e:test")));
    assert!(budgets.contains(&json!("onetaskgraph-github-projects:test")));
    for name in [
        "onetaskgraph-github-projects",
        "onetaskgraph-github-projects-e2e",
    ] {
        let project: Value = serde_json::from_slice(
            &std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join(name)
                    .join("project.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let inputs = project["targets"]["test"]["inputs"].as_array().unwrap();
        for identity in [
            "GITHUB_LIVE_SESSION_ID",
            "GITHUB_LIVE_SOURCE_REVISION",
            "GITHUB_LIVE_SESSION_DIR",
            "GITHUB_LIVE_SELECTION",
        ] {
            assert!(
                inputs.contains(&json!({"env":identity})),
                "{name}: missing {identity}"
            );
        }
        assert!(
            project["targets"]["test"]["outputs"]
                .as_array()
                .unwrap()
                .contains(&json!(
                    "{workspaceRoot}/target/debug/telemetry/github-live-session"
                ))
        );
    }
}
