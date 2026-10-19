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
    // Windows pipes carry the ANSI code page unless a producer names its encoding;
    // forcing it everywhere makes every platform write what that runner writes. The
    // unresolvable base makes the selection helper explain itself with an em dash
    // (cp1252 byte 0x97), which the collection step decodes and passes on.
    let output = std::process::Command::new("python3")
        .args(["-c", launch])
        .arg(&runner)
        .arg(&query)
        .current_dir(root)
        .env("NX_DAEMON", "false")
        .env("NX_BASE", "missing-base-for-recipe-launch")
        .env("PYTHONIOENCODING", "cp1252")
        .env_remove("GH_PROJECTS_TOKEN")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let explained = "cannot be read — answering `run`".as_bytes();
    assert!(
        output
            .stderr
            .windows(explained.len())
            .any(|window| window == explained),
        "the helper's em dash must arrive as the UTF-8 it was written in: {}",
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
    // The suite's loopback binaries run beside the plugin's main session: its test target does
    // not wait on the plugin's, and the asset journey itself waits for the main record before it
    // makes a request. Budgets, which report the session, still wait on both.
    let dependencies = &graph["tasks"]["dependencies"];
    assert!(
        !dependencies["onetaskgraph-github-projects-e2e:test"]
            .as_array()
            .unwrap()
            .contains(&json!("onetaskgraph-github-projects:test")),
        "the suite's loopback binaries must not wait behind the plugin's main session"
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
fn the_real_policy_gives_each_invocation_a_fresh_directory_and_refuses_unsafe_identities() {
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
            std::fs::remove_file(&graph).unwrap();
            // A repeat in the same job (the gate's recipe checks drive `just check` and
            // `just gate` again under the job's own location) proceeds into a fresh
            // sibling, and the earlier invocation's records stay exactly as written.
            let earlier = directory.join("main.json");
            std::fs::write(&earlier, "earlier invocation's record").unwrap();
            let mut received = Vec::new();
            for ordinal in [2, 3] {
                let recorded = root.join(format!("received-{ordinal}"));
                let repeat = std::process::Command::new("python3")
                    .arg(&runner)
                    .arg("select")
                    .arg(&policy)
                    .args([
                        "python3",
                        "-c",
                        "import os,sys; open(sys.argv[1],'w').write(os.environ['GITHUB_LIVE_SESSION_DIR'])",
                    ])
                    .arg(&recorded)
                    .env("NX_BASE", "missing-base-for-session-validation")
                    .env("GH_PROJECTS_TOKEN", "fixture-token")
                    .env("GITHUB_LIVE_SESSION_ID", identity)
                    .env("GITHUB_LIVE_SESSION_DIR", &directory)
                    .output()
                    .unwrap();
                let said = String::from_utf8_lossy(&repeat.stderr);
                assert!(repeat.status.success(), "{said}");
                let sibling = root.join(format!("must-be-fresh.{ordinal}"));
                assert!(
                    said.contains("holds an earlier invocation's records"),
                    "{said}"
                );
                assert_eq!(
                    std::path::PathBuf::from(std::fs::read_to_string(&recorded).unwrap())
                        .canonicalize()
                        .unwrap(),
                    sibling.canonicalize().unwrap()
                );
                assert_eq!(std::fs::read_dir(&sibling).unwrap().count(), 0);
                received.push(sibling);
                std::fs::remove_file(recorded).unwrap();
            }
            assert_eq!(
                std::fs::read_to_string(&earlier).unwrap(),
                "earlier invocation's record"
            );
            std::fs::remove_file(earlier).unwrap();
            std::fs::remove_dir(directory).unwrap();
            for sibling in received {
                std::fs::remove_dir(sibling).unwrap();
            }
        } else {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("safe path component"));
            assert!(!directory.exists());
            assert!(!graph.exists());
        }
    }
}
