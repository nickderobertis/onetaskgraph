//! The real selection adapter passes the policy's exclusions and fresh job identity to Nx.
use onetaskgraph_e2e_support::common::Sandbox;
use serde_json::{Value, json};

#[test]
fn test_recipe_launches_the_real_nx_script_through_bash() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
    let recipe = std::process::Command::new("just")
        .args(["--dry-run", "test"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(recipe.status.success());
    let rendered = String::from_utf8(recipe.stderr).unwrap();
    assert!(
        rendered.contains("scripts/live-lane-selection.sh bash ./scripts/nx.sh"),
        "the collection adapter must launch the shell script with its interpreter: {rendered}"
    );
    let sandbox = Sandbox::new();
    let graph = sandbox.subdirectory("recipe-launch").join("graph.json");
    std::fs::create_dir_all(graph.parent().unwrap()).unwrap();
    // Keep the recipe's real adapter and launch arguments; ask Nx for its graph
    // instead of executing targets that could open a hosted session.
    let query = rendered.replace(
        "affected -t test budgets",
        &format!(
            "run-many -p onetaskgraph-github-projects -t test --graph='{}'",
            graph.display()
        ),
    );
    assert_ne!(query, rendered);
    let runner = root.join("crates/onetaskgraph-github-projects-e2e/lane_runner.py");
    // Use the adapter's real resolver for the outer shell too, so Windows never
    // resolves System32's WSL launcher before reaching the recipe under test.
    let launch = "import runpy,subprocess,sys; from pathlib import Path; m=runpy.run_path(sys.argv[1]); b=m['executable']('bash'); assert Path(b).is_absolute(); sys.exit(subprocess.call([b,'-uc',sys.argv[2]]))";
    let output = std::process::Command::new("python3")
        .args(["-c", launch])
        .arg(&runner)
        .arg(&query)
        .current_dir(root)
        .env("NX_DAEMON", "false")
        .env("NX_BASE", "missing-base-for-recipe-launch")
        .env_remove("GH_PROJECTS_TOKEN")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&std::fs::read(&graph).unwrap()).unwrap();
    assert!(parsed["tasks"]["tasks"]["onetaskgraph-github-projects:test"].is_object());
    std::fs::remove_file(&graph).unwrap();
    let refused = std::process::Command::new("python3")
        .args(["-c", launch])
        .arg(&runner)
        .arg(query.replace("./scripts/nx.sh", "./scripts/missing-nx-launch.sh"))
        .current_dir(root)
        .env("NX_BASE", "missing-base-for-recipe-launch")
        .env_remove("GH_PROJECTS_TOKEN")
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("missing-nx-launch.sh"));
    assert!(!graph.exists());
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
    let project: Value = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("project.json"))
            .unwrap(),
    )
    .unwrap();
    assert!(
        project["targets"]["coverage"]["dependsOn"]
            .as_array()
            .unwrap()
            .contains(&json!("onetaskgraph-github-projects:coverage")),
        "the instrumented pair needs the plugin's compiled fixture and main binaries first"
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

#[test]
fn budget_adapter_uses_the_real_checker_and_never_substitutes_missing_live_evidence() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("budget-adapter");
    std::fs::create_dir_all(&root).unwrap();
    let runner = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lane_runner.py");
    for (selection, credential, unavailable) in [
        ("run", true, false),
        ("not-selected", true, true),
        ("run", false, true),
    ] {
        let mut command = std::process::Command::new("python3");
        command
            .arg(&runner)
            .arg("budgets")
            .env("GITHUB_ASSET_TELEMETRY_DIR", root.join("missing-offline"))
            .env("GITHUB_LIVE_SESSION_DIR", root.join("missing-live"))
            .env("GITHUB_LIVE_SESSION_ID", "adapter-fixture")
            .env("GITHUB_LIVE_SOURCE_REVISION", "a".repeat(40))
            .env("GITHUB_LIVE_SELECTION", selection)
            .env_remove("GH_PROJECTS_TOKEN");
        if credential {
            command.env("GH_PROJECTS_TOKEN", "fixture-token");
        }
        let output = command.output().unwrap();
        assert!(
            !output.status.success(),
            "missing telemetry must never produce a successful measurement"
        );
        let received = String::from_utf8_lossy(&output.stdout);
        assert!(
            received.contains("budget github-asset-requests-per-new-asset:"),
            "{received}"
        );
        assert_eq!(
            received.contains("budget github-live-session-points:"),
            !unavailable,
            "{received}"
        );
        assert_eq!(
            received.contains("github-live-session-points unavailable"),
            unavailable,
            "{received}"
        );
        if !unavailable {
            assert!(received.contains("live-session measurement unavailable"));
        }
    }
    let output = std::process::Command::new("python3")
        .arg(runner)
        .arg("budgets")
        .env("GITHUB_LIVE_SELECTION", "unknown")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("invalid propagated GitHub live selection")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("budget github-asset"));
}

#[test]
fn the_real_policy_and_nx_graph_preserve_fresh_directories_and_refuse_unsafe_identities() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("session-identity");
    std::fs::create_dir_all(&root).unwrap();
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let runner = manifest.join("lane_runner.py");
    let policy = manifest
        .ancestors()
        .nth(2)
        .unwrap()
        .join("scripts/live-lane-selection.sh");
    for identity in [
        "../escape",
        "/absolute",
        "bad name",
        ".",
        "x\\y",
        "",
        "job-1",
    ] {
        let directory = root.join("must-be-fresh");
        let graph = root.join("graph.json");
        let mut command = std::process::Command::new("python3");
        command
            .arg(&runner)
            .arg("select")
            .arg(&policy)
            .args([
                "bun",
                "x",
                "nx",
                "run-many",
                "-p",
                "onetaskgraph-github-projects",
                "-t",
                "test",
            ])
            .arg(format!("--graph={}", graph.display()))
            .env("NX_DAEMON", "false")
            .env("NX_BASE", "missing-base-for-session-validation")
            .env("GH_PROJECTS_TOKEN", "fixture-token")
            .env("GITHUB_LIVE_SESSION_ID", identity)
            .env("GITHUB_LIVE_SESSION_DIR", &directory);
        let output = command.output().unwrap();
        if identity.is_empty() || identity == "job-1" {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(directory.exists());
            let rendered: Value = serde_json::from_slice(&std::fs::read(&graph).unwrap()).unwrap();
            assert!(
                rendered["tasks"]["tasks"]
                    .as_object()
                    .unwrap()
                    .contains_key("onetaskgraph-github-projects:test")
            );
            std::fs::write(&graph, "duplicate must not reach Nx").unwrap();
            let duplicate = command.output().unwrap();
            assert!(!duplicate.status.success());
            assert!(
                String::from_utf8_lossy(&duplicate.stderr)
                    .contains("GitHub live collection refused")
            );
            assert_eq!(
                std::fs::read_to_string(&graph).unwrap(),
                "duplicate must not reach Nx"
            );
            std::fs::remove_file(&graph).unwrap();
            std::fs::remove_dir(directory).unwrap();
        } else {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("safe path component"));
            assert!(!directory.exists());
            assert!(!graph.exists());
        }
    }
}
