//! What a write, a damaged entry and a second process leave a cached board's projects and
//! documents holding, driven through the binary, both SDKs and a Rust caller of the engine
//! against the loopback board, doubling only GitHub.
//!
//! Every write that can change a project or a document takes the entries naming it out of service
//! before it is sent — a metadata write, a rendering, and the write-back of a copy that could not
//! finish — so a read pinned to the version held before it reads the board and answers what the
//! write left. An entry this build cannot read as its own — bytes that do not parse, another
//! schema, another kind's entry under the same id, a release's directory of another schema — is
//! a miss, never a failure; two processes writing one entry never leave a reader part of it; and
//! a pin reaches the cache through every consumer, the command line, both SDKs and a Rust caller.

use std::path::{Path, PathBuf};

use onetaskgraph_plugin_api::asset_sha256;
use serde_json::{Value, json};

use crate::cache::{Cached, sdk};
use crate::cache_kinds::{kind_entries, listed, show, version};
use onetaskgraph_e2e_support::sdk::Sdk;

/// Where the cache keeps the `directory` entry of the native id `native`.
fn entry_path(cached: &Cached, directory: &str, native: &str) -> PathBuf {
    cached
        .default_dir()
        .join(directory)
        .join(format!("{}.json", asset_sha256(native.as_bytes())))
}

/// Fill the entry of the `kind` `id` by a read pinned to its listing's token, answering that
/// token, and hold it to being a hit.
fn filled(cached: &Cached, kind: &str, id: &str) -> String {
    let held = listed(cached, kind, id);
    show(cached, kind, id, Some(&held));
    let (_, sent) = show(cached, kind, id, Some(&held));
    assert_eq!(sent, Vec::<String>::new(), "{id} is filled");
    held
}

/// A template rendering one line of text, as `project create` and `document create` take one.
const NOTE: &str = "---\nonetaskgraph_template: 1\nvariables:\n  note:\n    description: What \
                    changed\n    type: string\n---\n# Notes\n{{ note }}\n";

#[test]
fn a_project_or_document_metadata_write_or_render_removes_its_entry_and_the_next_pinned_read_is_the_boards()
 {
    let cached = Cached::new();
    let template = cached.sandbox.project().join("note.md");
    std::fs::write(&template, NOTE).unwrap();
    let template = template.display().to_string();
    // A rendered project — P-2's rendering replaced from the template — and a rendered document
    // created from it.
    let (_, _) = cached.ok(&[
        "project",
        "create",
        "board",
        "--id",
        "P-2",
        "--title",
        "Docs",
        "--template",
        &template,
        "--var",
        "note=first",
        "--no-interactive",
        "--json",
    ]);
    let (created, _) = cached.ok(&[
        "document",
        "create",
        "board",
        "--project",
        "P-1",
        "--title",
        "Rendered",
        "--template",
        &template,
        "--var",
        "note=first",
        "--no-interactive",
        "--json",
    ]);
    let rendered = [
        "board:P-2".to_owned(),
        created["items"][0]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("the document was created: {created}"))
            .to_owned(),
    ];
    let (rendered_project, rendered_document) = (rendered[0].clone(), rendered[1].clone());
    // (the kind, the item, its entries' directory, the write, what the next read shows)
    let writes: Vec<(&str, String, &str, Vec<String>, &str)> = vec![
        (
            "project",
            "board:P-1".to_owned(),
            "projects-v2",
            [
                "project",
                "metadata",
                "set",
                "board:P-1",
                "caller.note",
                "\"noted\"",
            ]
            .map(str::to_owned)
            .to_vec(),
            "noted",
        ),
        (
            "document",
            "board:D-1".to_owned(),
            "documents-v2",
            [
                "document",
                "metadata",
                "set",
                "board:D-1",
                "caller.note",
                "\"noted\"",
            ]
            .map(str::to_owned)
            .to_vec(),
            "noted",
        ),
        (
            "project",
            rendered_project.clone(),
            "projects-v2",
            vec![
                "project".to_owned(),
                "render".to_owned(),
                rendered_project.clone(),
                "--var".to_owned(),
                "note=rendered again".to_owned(),
                "--no-interactive".to_owned(),
            ],
            "rendered again",
        ),
        (
            "document",
            rendered_document.clone(),
            "documents-v2",
            vec![
                "document".to_owned(),
                "render".to_owned(),
                rendered_document.clone(),
                "--var".to_owned(),
                "note=rendered again".to_owned(),
                "--no-interactive".to_owned(),
            ],
            "rendered again",
        ),
    ];
    for (kind, id, directory, write, shows) in writes {
        let held = filled(&cached, kind, &id);
        let native = id.strip_prefix("board:").unwrap();
        assert!(entry_path(&cached, directory, native).exists(), "{id}");
        let arguments: Vec<&str> = write.iter().map(String::as_str).collect();
        cached.ok(&[arguments.as_slice(), &["--json"]].concat());
        assert!(
            !entry_path(&cached, directory, native).exists(),
            "`{}` takes {id}'s entry out of service",
            write.join(" ")
        );
        let (current, sent) = show(&cached, kind, &id, Some(&held));
        assert!(
            !sent.is_empty(),
            "{id}: the read after the write is the board's"
        );
        assert!(
            current["items"][0]["item"].to_string().contains(shows),
            "{id} answers what the write left: {current}"
        );
    }
}

#[test]
fn a_copy_that_cannot_finish_takes_the_entries_of_what_it_writes_back_out_of_service() {
    let cached = Cached::new();
    let plans = cached.sandbox.subdirectory("plans");
    std::fs::create_dir_all(plans.join("projects")).unwrap();
    std::fs::write(
        plans.join("projects/R.md"),
        "---\ntitle: Roadmap\nstatus: todo\n---\nthe plan\n",
    )
    .unwrap();
    std::fs::write(
        plans.join("tasks/R-1.md"),
        "---\ntitle: First step\nstatus: todo\nproject: R\n---\nstep one\n",
    )
    .unwrap();
    let (copied, _) = cached.ok(&["project", "copy", "plans:R", "--to", "board", "--json"]);
    let project = copied["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["source"] == "plans:R")
        .expect("the project was copied")["destination"]
        .as_str()
        .unwrap()
        .to_owned();
    let held = filled(&cached, "project", &project);

    // The plan is retitled and copied again over what the first copy left — and the board
    // refuses a write after the project's own landed, so the copy writes the project back.
    let text = std::fs::read_to_string(plans.join("projects/R.md")).unwrap();
    std::fs::write(
        plans.join("projects/R.md"),
        text.replace("title: Roadmap", "title: Roadmap, renamed"),
    )
    .unwrap();
    std::fs::write(
        plans.join("tasks/R-2.md"),
        "---\ntitle: Second step\nstatus: todo\nproject: R\n---\nstep two\n",
    )
    .unwrap();
    cached.board.refuse_once("createIssue(input:$input)");
    let failed = cached.run_with(
        &["project", "copy", "plans:R", "--to", "board", "--json"],
        None,
    );
    assert_ne!(failed.status.code(), Some(0), "the copy could not finish");
    let native = project.strip_prefix("board:").unwrap();
    assert!(
        !entry_path(&cached, "projects-v2", native).exists(),
        "the project the copy wrote and wrote back is out of service"
    );
    let (current, sent) = show(&cached, "project", &project, Some(&held));
    assert!(!sent.is_empty(), "read from the board");
    assert_eq!(
        current["items"][0]["item"]["title"], "Roadmap",
        "the board holds the project as the copy found it"
    );
}

#[test]
fn an_entry_this_build_cannot_read_as_its_own_is_a_miss_and_the_read_succeeds() {
    let cached = Cached::new();
    let (first, _) = show(&cached, "project", "board:P-1", None);
    let held = version(&first);
    let path = entry_path(&cached, "projects-v2", "P-1");
    let entry: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut other_schema = entry.clone();
    let next = onetaskgraph_core::cache::KIND_ENTRY_SCHEMA + 1;
    other_schema["schema"] = json!(next);
    let mut other_remote = entry.clone();
    other_remote["remote"] = json!("another remote");
    // A document entry for the very same id, under the project's path: an id names one kind's
    // entry, never another's.
    let (document, _) = show(&cached, "document", "board:D-1", None);
    let mut other_kind: Value =
        serde_json::from_slice(&std::fs::read(entry_path(&cached, "documents-v2", "D-1")).unwrap())
            .unwrap();
    other_kind["document"]["id"] = json!("P-1");
    other_kind["document"]["version"] = json!(held);
    for (what, bytes) in [
        (
            "bytes that do not parse",
            b"{\"schema\": 1, \"kind\"".to_vec(),
        ),
        ("an empty file", Vec::new()),
        ("another schema", serde_json::to_vec(&other_schema).unwrap()),
        ("another remote", serde_json::to_vec(&other_remote).unwrap()),
        (
            "another kind's entry",
            serde_json::to_vec(&other_kind).unwrap(),
        ),
    ] {
        std::fs::write(&path, &bytes).unwrap();
        let (shown, sent) = show(&cached, "project", "board:P-1", Some(&held));
        assert_eq!(
            sent,
            [onetaskgraph_github_projects::graphql::ISSUE],
            "{what}: a miss"
        );
        assert_eq!(shown["items"], first["items"], "{what}");
    }
    assert!(!document["items"].as_array().unwrap().is_empty());

    // A release of another schema keeps its entries in a directory of its own, which this build
    // neither reads nor writes nor clears.
    let newer = cached.default_dir().join(format!("projects-v{next}"));
    std::fs::create_dir_all(&newer).unwrap();
    let theirs = newer.join(format!("{}.json", asset_sha256(b"P-1")));
    std::fs::write(&theirs, b"a newer release's entry").unwrap();
    std::fs::remove_file(&path).unwrap();
    let (_, sent) = show(&cached, "project", "board:P-1", Some(&held));
    assert_eq!(sent.len(), 1, "never read from another schema's directory");
    cached.ok(&["cache", "clear", "board", "--json"]);
    assert_eq!(
        std::fs::read(&theirs).unwrap(),
        b"a newer release's entry",
        "nor written or cleared"
    );
    assert!(kind_entries(&cached.default_dir(), "projects-v2").is_empty());
}

#[test]
fn two_processes_writing_one_project_entry_never_leave_a_reader_part_of_it() {
    let cached = Cached::new();
    let (first, _) = show(&cached, "project", "board:P-1", None);
    let held = version(&first);
    let path = entry_path(&cached, "projects-v2", "P-1");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let stop = stop.clone();
        let path = path.clone();
        std::thread::spawn(move || {
            let mut reads = 0_u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(bytes) = std::fs::read(&path) {
                    serde_json::from_slice::<Value>(&bytes)
                        .unwrap_or_else(|error| panic!("a reader saw part of an entry ({error})"));
                    reads += 1;
                }
            }
            reads
        })
    };
    let writers: Vec<_> = (0..2)
        .map(|_| {
            let mut command = cached.sandbox.command();
            command.env("XDG_CACHE_HOME", &cached.cache_home).args([
                "project",
                "show",
                "board:P-1",
                "--json",
            ]);
            std::thread::spawn(move || {
                for _ in 0..4 {
                    command.assert().success();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("a writer");
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.join().expect("the reader") > 0);
    let (_, sent) = show(&cached, "project", "board:P-1", Some(&held));
    assert_eq!(sent, Vec::<String>::new());
    let staged = |dir: &Path| {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|file| file.file_name().to_string_lossy().starts_with(".staged"))
    };
    assert!(
        !staged(&cached.default_dir().join("projects-v2")),
        "no staging file is left behind"
    );
}

#[test]
fn a_pin_reaches_the_cache_through_both_sdks_and_a_rust_caller() {
    let cached = Cached::new();
    let mut held = Vec::new();
    for (kind, id) in [("project", "board:P-1"), ("document", "board:D-1")] {
        let (first, _) = show(&cached, kind, id, None);
        held.push((kind, id, version(&first)));
    }
    for (kind, id, pin) in &held {
        let method = if *kind == "project" {
            "project"
        } else {
            "document"
        };
        let python = format!(
            "result=await c.{method}_show(id=task,pin=pin)\n print(result.model_dump_json(exclude_none=True))"
        );
        let typescript =
            format!("console.log(JSON.stringify(await client.{method}Show(id, {{pin}})));");
        for (which, script) in [(Sdk::Python, python), (Sdk::TypeScript, typescript)] {
            let before = cached.board.served().len();
            let shown = sdk(&cached, which, &script, id, Some(pin));
            assert_eq!(
                cached.board.served().len(),
                before,
                "{id} through the {which:?} SDK sends nothing"
            );
            assert_eq!(shown["items"][0]["item"]["version"], json!(pin));
        }
    }

    // A Rust caller of the engine: a pinned project read, then the copy reading its forward
    // edges — which the board's token does not vouch for, so they are read and nothing else is.
    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            cached.sandbox.config_home().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME",
            cached.cache_home.to_string_lossy().into_owned(),
        ),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &cached.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let (_, project, pin) = &held[0];
    let id: onetaskgraph_core::GlobalId = project.parse().unwrap();
    engine.pin_project(&id, pin);
    let before = cached.board.served().len();
    let read = runtime.block_on(engine.project(&id)).expect("the read");
    assert_eq!(
        cached.board.served().len(),
        before,
        "a pinned read sends nothing"
    );
    assert_eq!(read.items[0].item.version.as_deref(), Some(pin.as_str()));
    let edges = runtime
        .block_on(
            engine.project_dependencies(&onetaskgraph_core::DependencyRequest {
                id: id.clone(),
                direction: onetaskgraph_plugin_api::Direction::DependsOn,
                completeness: onetaskgraph_core::Completeness::NamingUnasked,
                paging: onetaskgraph_core::Paging {
                    limit: std::num::NonZeroU32::new(50).unwrap(),
                    token: None,
                },
            }),
        )
        .expect("the edges");
    let sent: Vec<String> = cached.board.served()[before..]
        .iter()
        .map(|(document, _)| document.clone())
        .collect();
    assert!(
        !sent.is_empty()
            && sent
                .iter()
                .all(|document| document
                    == onetaskgraph_github_projects::graphql::ISSUE_DEPENDENCIES),
        "the project's own fields are the entry's; its forward edges are read, and only they: \
         {sent:?}"
    );
    assert!(!edges.items.is_empty());
    let (_, document, pin) = &held[1];
    let id: onetaskgraph_core::GlobalId = document.parse().unwrap();
    engine.pin_document(&id, pin);
    let before = cached.board.served().len();
    let detail = runtime
        .block_on(engine.document_detail(&id))
        .expect("the read");
    assert_eq!(
        cached.board.served().len(),
        before,
        "a pinned document and its manifest send nothing"
    );
    assert_eq!(
        detail.response.items[0].item.version.as_deref(),
        Some(pin.as_str())
    );
}
