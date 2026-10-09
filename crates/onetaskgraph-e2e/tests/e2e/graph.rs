//! `project graph`, driven the way a user drives it: the compiled binary over folders of
//! Markdown, its stdout held to the exact bytes the verb's `--help` and the README state.
//!
//! The contract is fixed for its consumers — ai-orchestrator's design document pipes the
//! Mermaid form into mermaid-cli and reads the JSON form to know which task each node is — so
//! every assertion here is on whole outputs rather than on fragments of them: a line moved,
//! renumbered or escaped differently fails a journey here before it fails a render there.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::document;
use crate::machine::{bundle, validates};

/// One task file: its native id, title, project, prerequisites and front-matter extras.
struct Task<'a> {
    id: &'a str,
    title: &'a str,
    project: &'a str,
    depends_on: &'a [&'a str],
    extra: &'a str,
}

/// A folder of Markdown holding `projects` and `tasks`, configured as source `name`.
struct Store {
    root: PathBuf,
}

impl Store {
    /// An empty folder under `sandbox`, at `relative`.
    fn new(sandbox: &Sandbox, relative: &str) -> Self {
        let root = sandbox.subdirectory(relative);
        std::fs::create_dir_all(root.join("projects")).expect("the project folder");
        std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
        Self { root }
    }

    /// A project file.
    fn project(&self, id: &str, title: &str) -> &Self {
        std::fs::write(
            self.root.join(format!("projects/{id}.md")),
            format!("---\ntitle: {}\n---\n", yaml(title)),
        )
        .expect("a project file");
        self
    }

    /// A task file.
    fn task(&self, task: &Task<'_>) -> &Self {
        let depends = if task.depends_on.is_empty() {
            String::new()
        } else {
            format!("depends_on: [{}]\n", task.depends_on.join(", "))
        };
        std::fs::write(
            self.root.join(format!("tasks/{}.md", task.id)),
            format!(
                "---\ntitle: {}\nproject: {}\n{depends}{}---\nThe body.\n",
                yaml(task.title),
                task.project,
                task.extra
            ),
        )
        .expect("a task file");
        self
    }

    /// The source block configuring this folder.
    fn source(&self) -> Value {
        json!({"plugin": "local-md", "config": {"root": self.root}})
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

/// `text` as a YAML double-quoted scalar, so a title holding a quote or a line break reads back
/// exactly as written.
fn yaml(text: &str) -> String {
    serde_json::to_string(text).expect("a string renders as JSON, which YAML reads")
}

/// A sandbox configuring every store in `stores` under its name.
fn configured(sandbox: &Sandbox, stores: &[(&str, &Store)]) {
    let mut sources = serde_json::Map::new();
    for (name, store) in stores {
        sources.insert((*name).to_owned(), store.source());
    }
    sandbox.project_document(&document(&Value::Object(sources)));
}

/// The binary's whole output for one invocation.
fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// What one successful invocation printed, refusing anything but exit 0 with a silent stderr.
fn printed(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{arguments:?} failed\nstdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(stderr(&output), "", "{arguments:?} wrote to stderr");
    stdout(&output)
}

/// The JSON document one successful invocation printed, refusing anything but one document
/// followed by one newline.
fn parsed(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let text = printed(sandbox, arguments);
    assert!(
        text.ends_with("}\n") && !text.ends_with("}\n\n"),
        "{arguments:?} did not print one document and a newline:\n{text:?}"
    );
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{arguments:?}: {error}\n{text}"))
}

/// The title the contract fixture gives its wiring task: every character a label escapes, and
/// a line break.
const WIRING: &str = "Wire \"main\" #2 <panel>\nand >sockets";

/// The contract fixture: project `P` of six tasks — a fan-out from `root` to three, a fan-in to
/// `roof` from two of them and from `permit`, a task of project `Q` — and `paint` closing it.
///
/// `plumbing`'s title is a parameter, so the JSON journey can give it `walls`'s and watch the
/// qualified id break the tie.
fn plan(sandbox: &Sandbox, plumbing: &str) -> Store {
    let store = Store::new(sandbox, "plans");
    store.project("P", "The house").project("Q", "Paperwork");
    for task in [
        Task {
            id: "root",
            title: "Lay the foundation",
            project: "P",
            depends_on: &[],
            extra: "",
        },
        Task {
            id: "walls",
            title: "Raise walls",
            project: "P",
            depends_on: &["root"],
            extra: "",
        },
        Task {
            id: "wiring",
            title: WIRING,
            project: "P",
            depends_on: &["root"],
            extra: "",
        },
        Task {
            id: "plumbing",
            title: plumbing,
            project: "P",
            depends_on: &["root"],
            extra: "",
        },
        Task {
            id: "roof",
            title: "Roof",
            project: "P",
            depends_on: &["walls", "wiring", "permit"],
            extra: "",
        },
        Task {
            id: "paint",
            title: "Paint",
            project: "P",
            depends_on: &["roof", "plumbing"],
            extra: "",
        },
        Task {
            id: "permit",
            title: "Get the permit",
            project: "Q",
            depends_on: &[],
            extra: "",
        },
    ] {
        store.task(&task);
    }
    configured(sandbox, &[("plans", &store)]);
    store
}

/// What `project graph plans:P` prints for the contract fixture, byte for byte.
///
/// `root` alone depends on nothing, so it is first. `plumbing`, `walls` and `wiring` are then
/// ready together and go by title — "Plumb" before "Raise walls" before "Wire …". `roof`
/// waits on `walls` and `wiring`, and `paint` on `roof` and `plumbing`. The ranks are 1/3/1/1,
/// so the widest (3) does not exceed the number of ranks (4) and `auto` lays it out top-down.
const CONTRACT: &str = "\
flowchart TD
  n1[\"Lay the foundation\"]
  n2[\"Plumb\"]
  n3[\"Raise walls\"]
  n4[\"Wire #quot;main#quot; #35;2 #lt;panel#gt; and #gt;sockets\"]
  n5[\"Roof\"]
  n6[\"Paint\"]
  x1[\"Get the permit (plans:permit)\"]:::external
  n1 --> n2
  n1 --> n3
  n1 --> n4
  n2 --> n6
  n3 --> n5
  n4 --> n5
  n5 --> n6
  x1 --> n5
  classDef external stroke-dasharray: 5 5
";

#[test]
fn a_project_prints_its_tasks_and_edges_as_the_contract_states() {
    let sandbox = Sandbox::new();
    plan(&sandbox, "Plumb");
    assert_eq!(
        printed(&sandbox, &["project", "graph", "plans:P"]),
        CONTRACT
    );
    // The default form is Mermaid, said out loud.
    assert_eq!(
        printed(
            &sandbox,
            &["project", "graph", "plans:P", "--format", "mermaid"]
        ),
        CONTRACT
    );
    // And machine output asked for every other way leaves the verb's own `--format` in
    // control: its Mermaid bytes are what a consumer reads, whatever the configured output.
    for asked in [
        &["--json"][..],
        &["--output", "json"][..],
        &["--set", "output=json"][..],
    ] {
        let mut arguments = asked.to_vec();
        arguments.extend(["project", "graph", "plans:P"]);
        assert_eq!(printed(&sandbox, &arguments), CONTRACT, "{asked:?}");
    }
}

#[test]
fn every_spelling_of_a_line_break_is_one_space_in_a_label() {
    let sandbox = Sandbox::new();
    let store = Store::new(&sandbox, "plans");
    store.project("P", "Breaks");
    for (id, title) in [
        ("a", "Windows\r\nline"),
        ("b", "Old Mac\rline"),
        ("c", "Unix\nline"),
        ("d", "Two\r\n\r\nbreaks"),
    ] {
        store.task(&Task {
            id,
            title,
            project: "P",
            depends_on: &[],
            extra: "",
        });
    }
    configured(&sandbox, &[("plans", &store)]);
    assert_eq!(
        printed(&sandbox, &["project", "graph", "plans:P"]),
        "flowchart LR\n  n1[\"Old Mac line\"]\n  n2[\"Two  breaks\"]\n  n3[\"Unix line\"]\n  n4[\"Windows line\"]\n"
    );
    // The JSON form keeps each title exactly as the source holds it.
    let graph = parsed(
        &sandbox,
        &["project", "graph", "plans:P", "--format", "json"],
    );
    assert_eq!(graph["nodes"][0]["title"], "Old Mac\rline");
    assert_eq!(graph["nodes"][3]["title"], "Windows\r\nline");
}

/// The README this repository documents its verbs in, read from the repository root.
fn readme() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    std::fs::read_to_string(path).expect("the README sits at the repository root")
}

#[test]
fn the_readme_carries_the_contract_as_help_states_it_and_the_example_as_the_binary_prints_it() {
    // The contract is stated once, in `--help`; the README carries that text verbatim, so a
    // change to either without the other fails here rather than leaving a reader two answers.
    let sandbox = Sandbox::new();
    let help = printed(&sandbox, &["project", "graph", "--help"]);
    let contract = &help[help
        .find("The graph:\n")
        .expect("`--help` states the contract")..];
    let readme = readme();
    assert!(
        readme.contains(&format!("```text\n{contract}```\n")),
        "the README's copy of the contract is not what `project graph --help` prints:\n{contract}"
    );
    // And its example is the contract fixture's output, which the journey below holds the
    // binary to byte for byte.
    assert!(
        readme.contains(&format!("```text\n{CONTRACT}```\n")),
        "the README's example is not what the contract fixture prints"
    );
}

#[test]
fn the_json_form_describes_the_graph_the_mermaid_form_draws() {
    let sandbox = Sandbox::new();
    // Two tasks titled alike: `plumbing` and `walls` are ready together and tie on the title,
    // so the qualified id decides — `plans:plumbing` before `plans:walls`.
    plan(&sandbox, "Raise walls");
    let mermaid = printed(&sandbox, &["project", "graph", "plans:P"]);
    assert_eq!(
        mermaid,
        CONTRACT.replace("[\"Plumb\"]", "[\"Raise walls\"]")
    );
    let graph = parsed(
        &sandbox,
        &["project", "graph", "plans:P", "--format", "json"],
    );
    // The document the schema bundle names `ProjectGraph`, which both SDKs are generated from.
    validates(
        &bundle(&sandbox),
        "ProjectGraph",
        &graph,
        "project graph --format json",
    );
    let node = |key: &str, id: &str, title: &str, external: bool| json!({"key": key, "id": id, "title": title, "external": external, "group": null});
    let edge = |from: &str, to: &str| json!({"from": from, "to": to});
    assert_eq!(
        graph,
        json!({
            "schema_version": 1,
            "project": "plans:P",
            "direction": "td",
            "group_by": null,
            "nodes": [
                node("n1", "plans:root", "Lay the foundation", false),
                node("n2", "plans:plumbing", "Raise walls", false),
                node("n3", "plans:walls", "Raise walls", false),
                node("n4", "plans:wiring", WIRING, false),
                node("n5", "plans:roof", "Roof", false),
                node("n6", "plans:paint", "Paint", false),
                node("x1", "plans:permit", "Get the permit", true),
            ],
            "edges": [
                edge("plans:root", "plans:plumbing"),
                edge("plans:root", "plans:walls"),
                edge("plans:root", "plans:wiring"),
                edge("plans:plumbing", "plans:paint"),
                edge("plans:walls", "plans:roof"),
                edge("plans:wiring", "plans:roof"),
                edge("plans:roof", "plans:paint"),
                edge("plans:permit", "plans:roof"),
            ],
        })
    );

    // Node for node and edge for edge, read off the two forms themselves: every Mermaid node
    // line is the JSON node of its key, in the same order, and every Mermaid edge is the JSON
    // edge between the ids its two keys name, in the same order.
    let nodes = graph["nodes"].as_array().expect("nodes");
    let keyed = |key: &str| {
        nodes
            .iter()
            .find(|node| node["key"] == key)
            .unwrap_or_else(|| panic!("no JSON node {key}"))
    };
    let mut keys = Vec::new();
    let mut edges = Vec::new();
    for line in mermaid.lines().skip(1) {
        let line = line.trim_start();
        if let Some((from, to)) = line.split_once(" --> ") {
            edges.push(json!({"from": keyed(from)["id"], "to": keyed(to)["id"]}));
        } else if let Some((key, rest)) = line.split_once("[\"") {
            assert_eq!(
                rest.ends_with(":::external"),
                keyed(key)["external"] == true,
                "{line}"
            );
            keys.push(key.to_owned());
        }
    }
    assert_eq!(
        keys,
        nodes
            .iter()
            .map(|node| node["key"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(&edges, graph["edges"].as_array().unwrap());

    // `direction` is the resolved one, whichever was asked for.
    for (asked, resolved) in [("auto", "td"), ("td", "td"), ("lr", "lr")] {
        let graph = parsed(
            &sandbox,
            &[
                "project",
                "graph",
                "plans:P",
                "--format",
                "json",
                "--direction",
                asked,
            ],
        );
        assert_eq!(graph["direction"], resolved, "--direction {asked}");
    }
}

/// A project whose ranks hold `widths` tasks each, in order, every task after the first rank
/// depending on one or two tasks of the rank before — so each task's rank is exactly its own.
///
/// Answers how many edges it wrote.
fn ranked(sandbox: &Sandbox, widths: &[usize]) -> usize {
    let mut edges = 0;
    let store = Store::new(sandbox, "plans");
    store.project("P", "Shaped");
    let id = |rank: usize, index: usize| format!("r{rank}-{index:03}");
    for (rank, &width) in widths.iter().enumerate() {
        for index in 0..width {
            let mut before: Vec<String> = Vec::new();
            if rank > 0 {
                let previous = widths[rank - 1];
                before.push(id(rank - 1, index % previous));
                if previous > 1 && index % 2 == 1 {
                    before.push(id(rank - 1, (index + 1) % previous));
                }
            }
            edges += before.len();
            let before: Vec<&str> = before.iter().map(String::as_str).collect();
            let title = format!("Task {}", id(rank, index));
            store.task(&Task {
                id: &id(rank, index),
                title: &title,
                project: "P",
                depends_on: &before,
                extra: "",
            });
        }
    }
    configured(sandbox, &[("plans", &store)]);
    edges
}

/// The first line `project graph plans:P` prints under `--direction` `direction`.
fn header(sandbox: &Sandbox, direction: &str) -> String {
    let printed = printed(
        sandbox,
        &["project", "graph", "plans:P", "--direction", direction],
    );
    printed.lines().next().expect("a first line").to_owned()
}

#[test]
fn auto_lays_a_graph_out_left_to_right_exactly_when_it_is_wider_than_it_is_deep() {
    for (widths, expected) in [
        // One root with five dependents: two ranks, the widest holding five.
        (&[1, 5][..], "flowchart LR"),
        // A chain of three: three ranks, each holding one.
        (&[1, 1, 1][..], "flowchart TD"),
        // The widest rank exactly as wide as there are ranks.
        (&[1, 2][..], "flowchart TD"),
        // The three plan shapes the rule was measured on: 10 tasks over 1/4/2/1/2, where
        // top-down kept the smallest label at 11.5 px; and 48 over 1/28/15/4 and 100 over
        // 1/56/33/10, where only left to right did.
        (&[1, 4, 2, 1, 2][..], "flowchart TD"),
        (&[1, 28, 15, 4][..], "flowchart LR"),
        (&[1, 56, 33, 10][..], "flowchart LR"),
    ] {
        let sandbox = Sandbox::new();
        let edges = ranked(&sandbox, widths);
        assert_eq!(header(&sandbox, "auto"), expected, "ranks {widths:?}");
        assert_eq!(
            printed(&sandbox, &["project", "graph", "plans:P"])
                .lines()
                .count(),
            1 + widths.iter().sum::<usize>() + edges,
            "every task and every edge of ranks {widths:?} is drawn"
        );
    }

    // An explicit direction overrides `auto`, and changes nothing but the first line.
    let sandbox = Sandbox::new();
    ranked(&sandbox, &[1, 5]);
    let auto = printed(&sandbox, &["project", "graph", "plans:P"]);
    let td = printed(
        &sandbox,
        &["project", "graph", "plans:P", "--direction", "td"],
    );
    let lr = printed(
        &sandbox,
        &["project", "graph", "plans:P", "--direction", "lr"],
    );
    assert_eq!(td.lines().next(), Some("flowchart TD"));
    assert_eq!(lr.lines().next(), Some("flowchart LR"));
    assert_eq!(auto, lr);
    assert_eq!(
        td.lines().skip(1).collect::<Vec<_>>(),
        lr.lines().skip(1).collect::<Vec<_>>()
    );

    // And a direction the parser does not know is refused before anything is read.
    let output = run(
        &sandbox,
        &["project", "graph", "plans:P", "--direction", "bt"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("'bt'"), "{}", stderr(&output));
    let output = run(
        &sandbox,
        &["project", "graph", "plans:P", "--format", "dot"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(stdout(&output), "");
}

/// The metadata key the grouped journeys group by.
const UNIT: &str = "orchestrator.unit";

/// The grouping fixture: project `P` of seven tasks over three groups under [`UNIT`] — `core
/// "v2" #1` holding an edge between its two members, `sdk` of two members, `docs` of one —
/// and a task without the key, one holding `""` under it, and a dependency on `signoff` in
/// project `Q`.
fn grouped(sandbox: &Sandbox) -> Store {
    let store = Store::new(sandbox, "plans");
    store.project("P", "The release").project("Q", "Approvals");
    let unit = |value: &str| format!("metadata:\n  {UNIT}: {}\n", yaml(value));
    let (core, sdk, docs, empty) = (unit("core \"v2\" #1"), unit("sdk"), unit("docs"), unit(""));
    for task in [
        Task {
            id: "design",
            title: "Design API",
            project: "P",
            depends_on: &[],
            extra: &core,
        },
        Task {
            id: "implement",
            title: "Implement API",
            project: "P",
            depends_on: &["design"],
            extra: &core,
        },
        Task {
            id: "bench",
            title: "Benchmark",
            project: "P",
            depends_on: &["design"],
            extra: &sdk,
        },
        Task {
            id: "client",
            title: "Build client",
            project: "P",
            depends_on: &["implement"],
            extra: &sdk,
        },
        Task {
            id: "docs",
            title: "Write docs",
            project: "P",
            depends_on: &["client"],
            extra: &docs,
        },
        Task {
            id: "release",
            title: "Release",
            project: "P",
            depends_on: &["docs", "signoff"],
            extra: "",
        },
        Task {
            id: "announce",
            title: "Announce",
            project: "P",
            depends_on: &["release"],
            extra: &empty,
        },
        Task {
            id: "signoff",
            title: "Get sign-off",
            project: "Q",
            depends_on: &[],
            extra: &unit("legal"),
        },
    ] {
        store.task(&task);
    }
    configured(sandbox, &[("plans", &store)]);
    store
}

/// What the grouping fixture prints without `--group-by`.
const UNGROUPED: &str = "\
flowchart TD
  n1[\"Design API\"]
  n2[\"Benchmark\"]
  n3[\"Implement API\"]
  n4[\"Build client\"]
  n5[\"Write docs\"]
  n6[\"Release\"]
  n7[\"Announce\"]
  x1[\"Get sign-off (plans:signoff)\"]:::external
  n1 --> n2
  n1 --> n3
  n3 --> n4
  n4 --> n5
  n5 --> n6
  n6 --> n7
  x1 --> n6
  classDef external stroke-dasharray: 5 5
";

/// What it prints under `--group-by orchestrator.unit`: the same nodes, numbers and edges,
/// each group's members and its one inside edge in its block, in first-member order.
const GROUPED: &str = "\
flowchart TD
  subgraph g1[\"core #quot;v2#quot; #35;1\"]
    n1[\"Design API\"]
    n3[\"Implement API\"]
    n1 --> n3
  end
  subgraph g2[\"sdk\"]
    n2[\"Benchmark\"]
    n4[\"Build client\"]
  end
  subgraph g3[\"docs\"]
    n5[\"Write docs\"]
  end
  n6[\"Release\"]
  n7[\"Announce\"]
  x1[\"Get sign-off (plans:signoff)\"]:::external
  n1 --> n2
  n3 --> n4
  n4 --> n5
  n5 --> n6
  n6 --> n7
  x1 --> n6
  classDef external stroke-dasharray: 5 5
";

#[test]
fn group_by_draws_each_group_in_its_own_block_and_changes_nothing_else() {
    let sandbox = Sandbox::new();
    let store = grouped(&sandbox);
    assert_eq!(
        printed(&sandbox, &["project", "graph", "plans:P"]),
        UNGROUPED
    );
    assert_eq!(
        printed(
            &sandbox,
            &["project", "graph", "plans:P", "--group-by", UNIT]
        ),
        GROUPED
    );

    let node = |key: &str, id: &str, title: &str, external: bool, group: Value| json!({"key": key, "id": id, "title": title, "external": external, "group": group});
    let core = json!("core \"v2\" #1");
    let expected = |group_by: Value, grouped: bool| {
        let group = |value: &Value| if grouped { value.clone() } else { Value::Null };
        json!({
            "schema_version": 1,
            "project": "plans:P",
            "direction": "td",
            "group_by": group_by,
            "nodes": [
                node("n1", "plans:design", "Design API", false, group(&core)),
                node("n2", "plans:bench", "Benchmark", false, group(&json!("sdk"))),
                node("n3", "plans:implement", "Implement API", false, group(&core)),
                node("n4", "plans:client", "Build client", false, group(&json!("sdk"))),
                node("n5", "plans:docs", "Write docs", false, group(&json!("docs"))),
                // No key, and `""` under it: neither is a group.
                node("n6", "plans:release", "Release", false, Value::Null),
                node("n7", "plans:announce", "Announce", false, Value::Null),
                // A task outside the project never has a group, whatever it holds.
                node("x1", "plans:signoff", "Get sign-off", true, Value::Null),
            ],
            "edges": [
                {"from": "plans:design", "to": "plans:bench"},
                {"from": "plans:design", "to": "plans:implement"},
                {"from": "plans:implement", "to": "plans:client"},
                {"from": "plans:client", "to": "plans:docs"},
                {"from": "plans:docs", "to": "plans:release"},
                {"from": "plans:release", "to": "plans:announce"},
                {"from": "plans:signoff", "to": "plans:release"},
            ],
        })
    };
    let graph = parsed(
        &sandbox,
        &[
            "project",
            "graph",
            "plans:P",
            "--format",
            "json",
            "--group-by",
            UNIT,
        ],
    );
    validates(
        &bundle(&sandbox),
        "ProjectGraph",
        &graph,
        "project graph --group-by",
    );
    assert_eq!(graph, expected(json!(UNIT), true));
    assert_eq!(
        parsed(
            &sandbox,
            &["project", "graph", "plans:P", "--format", "json"]
        ),
        expected(Value::Null, false)
    );

    // A number under the key is no group, and refused naming the task and the key.
    std::fs::write(
        store.path().join("tasks/announce.md"),
        format!(
            "---\ntitle: Announce\nproject: P\ndepends_on: [release]\nmetadata:\n  {UNIT}: 3\n---\n"
        ),
    )
    .expect("the task rewritten");
    for format in ["mermaid", "json"] {
        let output = run(
            &sandbox,
            &[
                "project",
                "graph",
                "plans:P",
                "--group-by",
                UNIT,
                "--format",
                format,
            ],
        );
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert_eq!(stdout(&output), "");
        let said = stderr(&output);
        assert!(
            said.contains("plans:announce") && said.contains(UNIT),
            "the refusal names neither the task nor the key: {said}"
        );
    }
    // An empty key names no metadata at all, and is refused as the invocation it is.
    let output = run(&sandbox, &["project", "graph", "plans:P", "--group-by", ""]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("a metadata key is not empty"),
        "{}",
        stderr(&output)
    );
    // Ungrouped, the value is nobody's business.
    assert_eq!(
        printed(&sandbox, &["project", "graph", "plans:P"]),
        UNGROUPED
    );
}

#[test]
fn the_same_project_prints_the_same_bytes_every_time_and_from_any_store() {
    let sandbox = Sandbox::new();
    // The contract fixture without its external dependency, so no qualified id is printed;
    // twice, the second under other native ids and in another folder.
    let first = Store::new(&sandbox, "first");
    let second = Store::new(&sandbox, "second");
    for (store, prefix) in [(&first, "a-"), (&second, "zz-")] {
        store.project(&format!("{prefix}P"), "The house");
        let id = |name: &str| format!("{prefix}{name}");
        let tasks: [(&str, &str, Vec<String>); 6] = [
            ("root", "Lay the foundation", vec![]),
            ("walls", "Raise walls", vec![id("root")]),
            ("wiring", WIRING, vec![id("root")]),
            ("plumbing", "Plumb", vec![id("root")]),
            ("roof", "Roof", vec![id("walls"), id("wiring")]),
            ("paint", "Paint", vec![id("roof"), id("plumbing")]),
        ];
        for (name, title, before) in &tasks {
            let before: Vec<&str> = before.iter().map(String::as_str).collect();
            store.task(&Task {
                id: &id(name),
                title,
                project: &format!("{prefix}P"),
                depends_on: &before,
                extra: "",
            });
        }
    }
    configured(&sandbox, &[("first", &first), ("second", &second)]);

    let mermaid = |project: &str| printed(&sandbox, &["project", "graph", project]);
    let json =
        |project: &str| printed(&sandbox, &["project", "graph", project, "--format", "json"]);
    assert_eq!(mermaid("first:a-P"), mermaid("first:a-P"));
    assert_eq!(json("first:a-P"), json("first:a-P"));
    assert_eq!(mermaid("first:a-P"), mermaid("second:zz-P"));
    assert!(mermaid("first:a-P").starts_with("flowchart TD\n  n1[\"Lay the foundation\"]\n"));

    // The JSON documents differ in the qualified ids and in nothing else.
    let (one, two): (Value, Value) = (
        serde_json::from_str(&json("first:a-P")).unwrap(),
        serde_json::from_str(&json("second:zz-P")).unwrap(),
    );
    assert_ne!(one, two);
    let renamed = json("first:a-P").replace("first:a-", "second:zz-");
    assert_eq!(serde_json::from_str::<Value>(&renamed).unwrap(), two);
    for field in ["schema_version", "direction", "group_by"] {
        assert_eq!(one[field], two[field], "{field}");
    }
    for (left, right) in one["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .zip(two["nodes"].as_array().unwrap())
    {
        for field in ["key", "title", "external", "group"] {
            assert_eq!(left[field], right[field], "{field}");
        }
    }
}

#[test]
fn an_empty_project_a_long_one_and_an_unknown_one() {
    let sandbox = Sandbox::new();
    let store = Store::new(&sandbox, "plans");
    store
        .project("empty", "Nothing yet")
        .project("long", "Long");
    // More tasks than one page of the folder's listing holds, as one chain, so the order and
    // the edges say every page was read.
    let count: usize = 250;
    for index in 0..count {
        let before = index.checked_sub(1).map(|before| format!("t{before:03}"));
        let before: Vec<&str> = before.iter().map(String::as_str).collect();
        store.task(&Task {
            id: &format!("t{index:03}"),
            title: &format!("Step {index:03}"),
            project: "long",
            depends_on: &before,
            extra: "",
        });
    }
    configured(&sandbox, &[("plans", &store)]);

    assert_eq!(
        printed(&sandbox, &["project", "graph", "plans:empty"]),
        "flowchart TD\n"
    );
    let graph = parsed(
        &sandbox,
        &["project", "graph", "plans:empty", "--format", "json"],
    );
    assert_eq!(graph["nodes"], json!([]));
    assert_eq!(graph["edges"], json!([]));
    assert_eq!(graph["direction"], "td");

    let long = printed(&sandbox, &["project", "graph", "plans:long"]);
    let mut expected = String::from("flowchart TD\n");
    for index in 0..count {
        expected.push_str(&format!("  n{}[\"Step {index:03}\"]\n", index + 1));
    }
    for index in 1..count {
        expected.push_str(&format!("  n{index} --> n{}\n", index + 1));
    }
    assert_eq!(long, expected);

    for format in ["mermaid", "json"] {
        let output = run(
            &sandbox,
            &["project", "graph", "plans:nowhere", "--format", format],
        );
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        assert!(
            stderr(&output).contains("no project with the id plans:nowhere"),
            "{}",
            stderr(&output)
        );
    }
    // An id that is not qualified is refused as `project show` refuses it.
    let graph = run(&sandbox, &["project", "graph", "long"]);
    let show = run(&sandbox, &["project", "show", "long"]);
    assert_eq!(graph.status.code(), show.status.code());
    assert_ne!(graph.status.code(), Some(0));
    assert_eq!(stdout(&graph), "");
    assert_eq!(stderr(&graph), stderr(&show));
}

#[test]
fn an_edge_is_drawn_once_a_project_end_is_not_drawn_and_externals_tie_on_their_id() {
    let sandbox = Sandbox::new();
    let store = Store::new(&sandbox, "plans");
    store.project("P", "The plan").project("Q", "Elsewhere");
    let unit = |value: &str| format!("metadata:\n  {UNIT}: {value}\n");
    for task in [
        Task {
            id: "a",
            title: "Alpha",
            project: "P",
            depends_on: &[],
            extra: &unit("core"),
        },
        // `a` twice, as two kinds of edge, and the whole project `Q`.
        Task {
            id: "b",
            title: "Beta",
            project: "P",
            depends_on: &["a", "{id: a, kind: related}", "{id: Q, item: project}"],
            extra: &unit("null"),
        },
        // Three tasks outside the project, two of them titled alike.
        Task {
            id: "c",
            title: "Gamma",
            project: "P",
            depends_on: &["q-b", "q-c", "q-a"],
            extra: "",
        },
        Task {
            id: "q-a",
            title: "Same title",
            project: "Q",
            depends_on: &[],
            extra: "",
        },
        Task {
            id: "q-b",
            title: "Same title",
            project: "Q",
            depends_on: &[],
            extra: "",
        },
        Task {
            id: "q-c",
            title: "Another",
            project: "Q",
            depends_on: &[],
            extra: "",
        },
    ] {
        store.task(&task);
    }
    configured(&sandbox, &[("plans", &store)]);

    // `a` and `c` are ready together and go by title; `b` waits on `a`. The externals go by
    // title, and the two titled alike by qualified id.
    assert_eq!(
        printed(&sandbox, &["project", "graph", "plans:P"]),
        "\
flowchart TD
  n1[\"Alpha\"]
  n2[\"Beta\"]
  n3[\"Gamma\"]
  x1[\"Another (plans:q-c)\"]:::external
  x2[\"Same title (plans:q-a)\"]:::external
  x3[\"Same title (plans:q-b)\"]:::external
  n1 --> n2
  x1 --> n3
  x2 --> n3
  x3 --> n3
  classDef external stroke-dasharray: 5 5
"
    );
    // A `null` under the key is no group, any more than no key at all is.
    let graph = parsed(
        &sandbox,
        &[
            "project",
            "graph",
            "plans:P",
            "--format",
            "json",
            "--group-by",
            UNIT,
        ],
    );
    let groups: Vec<&Value> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| &node["group"])
        .collect();
    assert_eq!(
        groups,
        [
            &json!("core"),
            &Value::Null,
            &Value::Null,
            &Value::Null,
            &Value::Null,
            &Value::Null
        ]
    );
    assert_eq!(
        printed(
            &sandbox,
            &["project", "graph", "plans:P", "--group-by", UNIT]
        )
        .lines()
        .take(4)
        .collect::<Vec<_>>(),
        [
            "flowchart TD",
            "  subgraph g1[\"core\"]",
            "    n1[\"Alpha\"]",
            "  end"
        ]
    );
}

#[test]
fn a_cycle_and_a_dependency_on_a_missing_task_are_refused_whole() {
    let sandbox = Sandbox::new();
    let store = Store::new(&sandbox, "plans");
    store.project("loop", "Loops").project("lost", "Lost");
    for task in [
        Task {
            id: "r1",
            title: "One",
            project: "loop",
            depends_on: &["r2"],
            extra: "",
        },
        Task {
            id: "r2",
            title: "Two",
            project: "loop",
            depends_on: &["r1"],
            extra: "",
        },
        Task {
            id: "r3",
            title: "Free",
            project: "loop",
            depends_on: &[],
            extra: "",
        },
        Task {
            id: "m1",
            title: "Waits on nothing there",
            project: "lost",
            depends_on: &["ghost"],
            extra: "",
        },
    ] {
        store.task(&task);
    }
    configured(&sandbox, &[("plans", &store)]);

    for format in ["mermaid", "json"] {
        let output = run(
            &sandbox,
            &["project", "graph", "plans:loop", "--format", format],
        );
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert_eq!(stdout(&output), "", "a graph was printed in part");
        assert!(
            stderr(&output)
                .contains("the tasks plans:r1, plans:r2 depend on each other in a cycle"),
            "{}",
            stderr(&output)
        );

        let output = run(
            &sandbox,
            &["project", "graph", "plans:lost", "--format", format],
        );
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert_eq!(stdout(&output), "", "a graph was printed in part");
        assert!(
            stderr(&output).contains("no task with the id plans:ghost"),
            "{}",
            stderr(&output)
        );
    }
}

/// The failure document one refused invocation under `--json` writes: what a program reads to
/// tell one refusal from another without parsing prose.
fn failure_kind(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let mut asked = vec!["--json"];
    asked.extend_from_slice(arguments);
    let output = run(sandbox, &asked);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let document: Value =
        serde_json::from_str(&stdout(&output)).expect("a failure document under --json");
    validates(
        &bundle(sandbox),
        "FailureDocument",
        &document,
        &asked.join(" "),
    );
    document["failure"]["kind"].clone()
}

#[test]
fn each_refusal_of_a_graph_names_its_own_kind_to_a_program() {
    let sandbox = Sandbox::new();
    let store = grouped(&sandbox);
    std::fs::write(
        store.path().join("tasks/announce.md"),
        format!("---\ntitle: Announce\nproject: P\nmetadata:\n  {UNIT}: [a, b]\n---\n"),
    )
    .expect("the task rewritten");
    store.project("loop", "Loops");
    for (id, before) in [("r1", "r2"), ("r2", "r1")] {
        store.task(&Task {
            id,
            title: id,
            project: "loop",
            depends_on: &[before],
            extra: "",
        });
    }
    assert_eq!(
        failure_kind(
            &sandbox,
            &["project", "graph", "plans:P", "--group-by", UNIT]
        ),
        "graph-group"
    );
    assert_eq!(
        failure_kind(&sandbox, &["project", "graph", "plans:loop"]),
        "dependency-cycle"
    );
    assert_eq!(
        failure_kind(&sandbox, &["project", "graph", "plans:nowhere"]),
        "no-such-item"
    );
}

#[test]
fn a_task_of_another_source_is_drawn_by_its_own_title_and_a_source_that_cannot_answer_fails_the_graph()
 {
    let sandbox = Sandbox::new();
    let plans = Store::new(&sandbox, "plans");
    let other = Store::new(&sandbox, "other");
    plans.project("P", "The plan");
    other.project("Q", "Theirs");
    other.task(&Task {
        id: "far",
        title: "Their \"part\"",
        project: "Q",
        depends_on: &[],
        extra: "",
    });
    plans.task(&Task {
        id: "near",
        title: "Ours",
        project: "P",
        depends_on: &["{id: \"other:far\"}"],
        extra: "",
    });
    plans.project("B", "Leans on a broken source");
    plans.task(&Task {
        id: "leaning",
        title: "Leaning",
        project: "B",
        depends_on: &["{id: \"broken:X-1\"}"],
        extra: "",
    });
    // `broken` is configured and cannot be built: a GitHub Projects source naming no board.
    let mut sources = serde_json::Map::new();
    sources.insert("plans".to_owned(), plans.source());
    sources.insert("other".to_owned(), other.source());
    sources.insert(
        "broken".to_owned(),
        json!({"plugin": "github-projects", "config": {}}),
    );
    sandbox.project_document(&document(&Value::Object(sources)));

    assert_eq!(
        printed(&sandbox, &["project", "graph", "plans:P"]),
        "flowchart TD\n  n1[\"Ours\"]\n  x1[\"Their #quot;part#quot; (other:far)\"]:::external\n  x1 --> n1\n  classDef external stroke-dasharray: 5 5\n"
    );
    for format in ["mermaid", "json"] {
        let output = run(
            &sandbox,
            &["project", "graph", "plans:B", "--format", format],
        );
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert_eq!(stdout(&output), "", "a graph was printed in part");
        assert!(
            stderr(&output).contains("source broken"),
            "the failing source is not named: {}",
            stderr(&output)
        );
    }
}

#[test]
fn an_external_tasks_qualified_id_is_escaped_in_its_label_as_a_title_is() {
    // A native id is whatever its source issued: an in-memory source issues any text, so its
    // id can hold every character a label escapes and a line break besides.
    let far = "ext \"1\" #2 <a>\nb";
    let task = |id: &str, title: &str, project: &str| {
        json!({"id": id, "title": title, "status": {"category": "todo", "name": "Todo"},
               "labels": [], "project": project})
    };
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(
        &json!({"mem": {"plugin": "in-memory", "config": {
            "projects": [
                {"id": "P", "title": "Ours", "status": {"category": "todo", "name": "Todo"},
                 "labels": []},
                {"id": "Q", "title": "Theirs", "status": {"category": "todo", "name": "Todo"},
                 "labels": []}
            ],
            "tasks": [task("near", "Near", "P"), task(far, "Far", "Q")],
            "task_dependencies": [{"from": "near", "to": far, "kind": "blocks"}]
        }}}),
    ));

    assert_eq!(
        printed(&sandbox, &["project", "graph", "mem:P"]),
        "flowchart TD\n  n1[\"Near\"]\n  x1[\"Far (mem:ext #quot;1#quot; #35;2 #lt;a#gt; b)\"]:::external\n  x1 --> n1\n  classDef external stroke-dasharray: 5 5\n"
    );
    // The JSON form names it exactly, for a program to read back.
    let graph = parsed(&sandbox, &["project", "graph", "mem:P", "--format", "json"]);
    assert_eq!(graph["nodes"][1]["id"], format!("mem:{far}"));
    assert_eq!(graph["edges"][0]["from"], format!("mem:{far}"));
}
