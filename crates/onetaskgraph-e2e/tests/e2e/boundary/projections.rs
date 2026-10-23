//! Public projections of a private coordination project, driven through the binary.
//!
//! The private plan `plan:rollout` holds a synthetic private producer, public work and work
//! that is private by its own record or an unknown repository. `project copy plan:rollout --to
//! vault` lands every task and edge canonically in `vault`, which routes `openco` work to the
//! public `site`, and makes there one neutral view per public task — and nothing else.
//!
//! Every identity here is synthetic: the private repository is the registered
//! `hiddenco/quietharbor`, and the names a projection must never carry are the ones this module
//! gives the private plan.

use super::*;

/// An unregistered repository, which the policy cannot answer for: unknown, so private.
const UNKNOWN: &str = "github.com/nobodyco/mysterybox";

/// Every string a public view must never hold: the private plan's identity, its private work,
/// its private repositories and the reserved keys that would tie a view back to it.
const NEVER_PUBLIC: &[&str] = &[
    "rollout",
    "Rollout",
    "quietharbor",
    "Quietharbor",
    "hiddenco",
    "nobodyco",
    "mysterybox",
    "Harden",
    "Embargoed",
    "Design",
    "plan:",
    "vault",
    "onetaskgraph.origin",
    "onetaskgraph.copies",
    "onetaskgraph.member",
    "onetaskgraph.public_projections",
    "onetaskgraph.depends_on",
    "classification",
];

/// The store: `plan` and `vault` private, `site` public, and `vault` routing `openco` work to
/// `site`.
fn projecting_store() -> Store {
    Store::new(
        Some(registered),
        json!({"vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]}}),
    )
}

/// The private coordination project `plan:rollout`: the private producer `producer`, the public
/// `widget` depending on it, the public `guide` depending on `widget`, `embargoed` naming the
/// public repository but private by its own record, `unknown` naming a repository nobody
/// answers for, and the plan's design document.
fn private_rollout(store: &Store) {
    store.record(
        "plan",
        "projects",
        "rollout",
        "title: Quietharbor rollout\nstatus: todo",
        "The private plan: what runs, in what order, and why.",
    );
    store.record(
        "plan",
        "tasks",
        "producer",
        &format!("title: Harden the ingest\nstatus: todo\nproject: rollout\nrepositories: [{A}]"),
        A_TEXT,
    );
    store.record(
        "plan",
        "tasks",
        "widget",
        &format!(
            "title: Add retry to the widget\nstatus: todo\nproject: rollout\n\
             repositories: [{OPEN}]\ndepends_on: [producer]"
        ),
        "Retry with a capped exponential backoff. See [the spec](https://example.org/retry).",
    );
    store.record(
        "plan",
        "tasks",
        "guide",
        &format!(
            "title: Document the retry\nstatus: todo\nproject: rollout\n\
             repositories: [{OPEN}]\ndepends_on: [widget]"
        ),
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "embargoed",
        &format!(
            "title: Embargoed widget work\nstatus: todo\nproject: rollout\n\
             repositories: [{OPEN}]\nclassification: private"
        ),
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "unknown",
        &format!("title: Unknown-repository work\nstatus: todo\nproject: rollout\nrepositories: [{UNKNOWN}]"),
        PLAIN,
    );
    store.record(
        "plan",
        "documents",
        "design",
        "title: Design of the rollout\nproject: rollout",
        "Why the producer goes first.",
    );
}

/// The copy report's source-to-destination ids.
fn copy_rollout(store: &Store) -> BTreeMap<String, String> {
    landed_ids(&store.ok(&["project", "copy", "plan:rollout", "--to", "vault", "--json"]))
}

/// One record read through the binary.
fn shown(store: &Store, kind: &str, id: &str) -> Value {
    let answer: Value = serde_json::from_str(&store.ok(&[kind, "show", id, "--json"]))
        .unwrap_or_else(|problem| panic!("{kind} show {id}: {problem}"));
    answer["items"][0]["item"].clone()
}

/// Every task `source` holds, by qualified id.
fn tasks_of(store: &Store, source: &str) -> Vec<Value> {
    let answer: Value =
        serde_json::from_str(&store.ok(&["task", "list", "--source", source, "--json"]))
            .expect("a task listing");
    answer["items"].as_array().expect("the tasks").clone()
}

/// The map the private destination project records.
fn projection_map(store: &Store, project: &str) -> Value {
    shown(store, "project", project)["metadata"]["onetaskgraph.public_projections"].clone()
}

/// `record` without the front-matter entry `key` and every line nested under it.
fn without_entry(record: &str, key: &str) -> String {
    let mut kept = String::new();
    let mut dropping: Option<usize> = None;
    for line in record.lines() {
        let indent = line.len() - line.trim_start().len();
        if let Some(under) = dropping {
            if indent > under {
                continue;
            }
            dropping = None;
        }
        if line.trim_start().starts_with(&format!("{key}:")) {
            dropping = Some(indent);
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept
}

/// Assert no file under `site` carries anything of the private plan, naming the file and what
/// it carries.
fn assert_site_neutral(store: &Store) {
    for (path, bytes) in tree(&store.folder("site")) {
        let text = String::from_utf8(bytes).expect("a text record");
        for named in NEVER_PUBLIC {
            assert!(!text.contains(named), "{path} carries {named:?}:\n{text}");
        }
    }
}

#[test]
fn a_private_plan_lands_whole_in_its_private_destination_with_neutral_views_of_its_public_work() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);

    // Every canonical task, the project and its edges are in `vault`, and only there.
    for item in [
        "plan:rollout",
        "plan:producer",
        "plan:widget",
        "plan:guide",
        "plan:embargoed",
        "plan:unknown",
    ] {
        assert!(landed[item].starts_with("vault:"), "{item}: {landed:?}");
    }
    let widget = &landed["plan:widget"];
    let guide = &landed["plan:guide"];
    assert_eq!(
        edges_of(&store, widget, "depends-on"),
        vec![(widget.clone(), landed["plan:producer"].clone())]
    );
    assert_eq!(
        edges_of(&store, guide, "depends-on"),
        vec![(guide.clone(), widget.clone())]
    );
    let document =
        landed_ids(&store.ok(&["document", "copy", "plan:design", "--to", "vault", "--json"]));
    assert!(
        document["plan:design"].starts_with("vault:"),
        "{document:?}"
    );

    // The store owns each task's visibility, from its own record and repositories.
    for (item, private) in [
        ("plan:producer", true),
        ("plan:widget", false),
        ("plan:guide", false),
        ("plan:embargoed", true),
        ("plan:unknown", true),
    ] {
        let task = shown(&store, "task", &landed[item]);
        let read = task["classification"].as_str().unwrap_or("public");
        assert_eq!(read == "private", private, "{item}: {task:#}");
    }
    let project = shown(&store, "project", &landed["plan:rollout"]);
    assert_eq!(project["classification"], "private", "{project:#}");

    // `site` holds exactly one neutral project and one view per public task.
    let views = tasks_of(&store, "site");
    assert_eq!(views.len(), 2, "{views:#?}");
    let projects: Value =
        serde_json::from_str(&store.ok(&["project", "list", "--source", "site", "--json"]))
            .expect("a project listing");
    let projects = projects["items"].as_array().expect("the projects");
    assert_eq!(projects.len(), 1, "{projects:#?}");
    let public_project = &projects[0]["item"];
    assert_eq!(public_project["title"], "Public work");
    let public_project_id = public_project["id"].as_str().expect("an id");
    assert!(public_project_id.starts_with("pw-"), "{public_project_id}");

    // The private project records which view is each canonical task's, and nothing public does.
    let map = projection_map(&store, &landed["plan:rollout"]);
    assert_eq!(map["schema_version"], 1, "{map:#}");
    let entries = map["tasks"].as_array().expect("the entries");
    assert_eq!(entries.len(), 2, "{map:#}");
    let view_of = |canonical: &str| {
        entries
            .iter()
            .find(|entry| entry["task"] == canonical)
            .and_then(|entry| entry["projection"].as_str())
            .unwrap_or_else(|| panic!("a projection of {canonical}: {map:#}"))
            .to_owned()
    };
    let widget_view = view_of(widget);
    let guide_view = view_of(guide);
    for view in [&widget_view, &guide_view] {
        assert!(view.starts_with("site:pt-"), "{view}");
    }
    let shown_widget = shown(&store, "task", &widget_view);
    assert_eq!(shown_widget["title"], "Add retry to the widget");
    assert_eq!(shown_widget["repositories"], json!([OPEN]));
    assert_eq!(shown_widget["project"], public_project_id);
    assert_eq!(shown_widget["metadata"], json!({}), "{shown_widget:#}");
    // A view depends only on another view: the widget's dependency on the private producer is
    // not there, and the guide's on the widget is, between their views.
    assert!(edges_of(&store, &widget_view, "depends-on").is_empty());
    assert_eq!(
        edges_of(&store, &guide_view, "depends-on"),
        vec![(guide_view.clone(), widget_view.clone())]
    );
    assert_site_neutral(&store);
}

#[test]
fn a_repeat_copy_resumes_and_refreshes_the_views_without_a_duplicate_or_a_change_to_the_plan() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let site_files = || files_under(&store, "site");
    let first = site_files();

    // A repeat writes nothing in either folder.
    let before = store.tree();
    assert_eq!(copy_rollout(&store), landed);
    assert_eq!(
        store.tree(),
        before,
        "a repeat leaves every folder as it was"
    );

    // A copy interrupted after it made a view and before it recorded it — the map gone from the
    // private project — finds every view again by its opaque id, and records it afresh.
    let home = store.folder("vault").join("projects").join(format!(
        "{}.md",
        landed["plan:rollout"].trim_start_matches("vault:")
    ));
    let held = std::fs::read_to_string(&home).expect("the private project");
    let without = without_entry(&held, "onetaskgraph.public_projections");
    assert_ne!(held, without, "the map was on the project:\n{held}");
    std::fs::write(&home, without).expect("the map removed");
    copy_rollout(&store);
    assert_eq!(site_files(), first, "no view was made twice");
    assert_eq!(
        projection_map(&store, &landed["plan:rollout"])["tasks"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );

    // A view edited by hand changes nothing of the plan, and the next copy puts the canonical
    // record back over it.
    let map = projection_map(&store, &landed["plan:rollout"]);
    let widget_view = map["tasks"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["task"] == landed["plan:widget"])
        .and_then(|entry| entry["projection"].as_str())
        .expect("the widget's view")
        .to_owned();
    let view_file = store
        .folder("site")
        .join("tasks")
        .join(format!("{}.md", widget_view.trim_start_matches("site:")));
    let edited = std::fs::read_to_string(&view_file)
        .expect("the view")
        .replace("Add retry to the widget", "Edited in public");
    std::fs::write(&view_file, edited).expect("the view edited");
    let plan_before = tree(&store.folder("plan"));
    let vault_before = tree(&store.folder("vault"));
    assert_eq!(
        shown(&store, "task", &landed["plan:widget"])["title"],
        "Add retry to the widget",
        "the canonical task is as it was"
    );
    copy_rollout(&store);
    assert_eq!(tree(&store.folder("plan")), plan_before);
    assert_eq!(tree(&store.folder("vault")), vault_before);
    assert_eq!(
        shown(&store, "task", &widget_view)["title"],
        "Add retry to the widget"
    );
    assert_eq!(site_files(), first, "still one view per public task");
    assert_site_neutral(&store);
}

#[test]
fn a_canonical_status_change_mirrors_its_category_alone_to_the_view() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let set: Value = serde_json::from_str(&store.ok(&[
        "task",
        "status",
        "set",
        widget,
        "in-progress",
        "--json",
    ]))
    .expect("a status answer");
    let view = set["projection"].as_str().expect("the view it mirrored to");
    assert!(view.starts_with("site:pt-"), "{set:#}");
    let shown_view = shown(&store, "task", view);
    assert_eq!(shown_view["status"]["category"], "in-progress");
    assert_eq!(shown_view["metadata"], json!({}), "{shown_view:#}");
    assert_eq!(
        shown(&store, "task", widget)["status"]["category"],
        "in-progress"
    );
    // A private task has no view, and its status stays where it is.
    let producer: Value = serde_json::from_str(&store.ok(&[
        "task",
        "status",
        "set",
        &landed["plan:producer"],
        "done",
        "--json",
    ]))
    .expect("a status answer");
    assert!(producer.get("projection").is_none(), "{producer:#}");
    assert_site_neutral(&store);
}

#[test]
fn a_public_task_naming_private_material_is_refused_before_anything_is_written() {
    for (field, front, body) in [
        (
            "title",
            format!(
                "title: Port the quietharbor retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"
            ),
            PLAIN.to_owned(),
        ),
        (
            "content",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            A_TEXT.to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "Read [the notes](../notes/rollout.md) first.".to_owned(),
        ),
    ] {
        let store = projecting_store();
        private_rollout(&store);
        store.record("plan", "tasks", "leaky", &front, &body);
        let before = store.tree();
        let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
        let expected = if field == "path" {
            "projection-reference"
        } else {
            "boundary-refused"
        };
        assert_eq!(kind, expected, "{field}: {said}");
        assert_eq!(
            store.tree(),
            before,
            "{field}: nothing was written anywhere"
        );
    }
}

#[test]
fn a_private_plan_is_refused_a_public_home_and_writes_nothing() {
    let store = projecting_store();
    private_rollout(&store);
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "site"]);
    assert_eq!(kind, "not-private-destination", "{said}");
    assert_eq!(store.tree(), before, "nothing was written");
}

#[test]
fn a_tampered_projection_map_is_refused_rather_than_trusted() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let home = store.folder("vault").join("projects").join(format!(
        "{}.md",
        landed["plan:rollout"].trim_start_matches("vault:")
    ));
    let held = std::fs::read_to_string(&home).expect("the private project");
    // A projection said to be somewhere declared private is not a public projection.
    let tampered = held.replace("site:pt-", "vault:pt-");
    assert_ne!(held, tampered);
    std::fs::write(&home, tampered).expect("tampered");
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_eq!(kind, "projections-malformed", "{said}");
    assert_eq!(store.tree(), before, "nothing more was written");
}
