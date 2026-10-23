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
    private_rollout_of(store, OPEN);
}

/// [`private_rollout`], its public work naming the public repository `public` instead.
fn private_rollout_of(store: &Store, public: &str) {
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
             repositories: [{public}]\ndepends_on: [producer]\nlabels: [urgent]\n\
             priority: high\nmetadata: {{team.owner: rollout-lead}}"
        ),
        "Retry with a capped exponential backoff. See [the spec](https://example.org/retry).",
    );
    store.record(
        "plan",
        "tasks",
        "guide",
        &format!(
            "title: Document the retry\nstatus: todo\nproject: rollout\n\
             repositories: [{public}]\ndepends_on: [widget]"
        ),
        PLAIN,
    );
    store.record(
        "plan",
        "tasks",
        "embargoed",
        &format!(
            "title: Embargoed widget work\nstatus: todo\nproject: rollout\n\
             repositories: [{public}]\nclassification: private"
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

/// Assert no file under `site` carries anything of the private plan, naming the file and what
/// it carries.
fn assert_site_neutral(store: &Store) {
    for (path, bytes) in tree(&store.folder("site")) {
        let text = String::from_utf8_lossy(&bytes);
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
    // Nothing of the canonical record but its title, content, repositories and status: not its
    // labels, priority, metadata or where the private store keeps it.
    assert_eq!(shown_widget["metadata"], json!({}), "{shown_widget:#}");
    assert_eq!(shown_widget["labels"], json!([]), "{shown_widget:#}");
    assert_eq!(shown_widget["priority"], "none", "{shown_widget:#}");
    assert!(
        shown_widget
            .get("delivers")
            .is_none_or(|delivers| delivers == &json!([])),
        "{shown_widget:#}"
    );
    let canonical_widget = shown(&store, "task", widget);
    assert_eq!(
        canonical_widget["labels"][0]["name"], "urgent",
        "{canonical_widget:#}"
    );
    assert_eq!(canonical_widget["priority"], "high", "{canonical_widget:#}");
    let located = shown_widget["location"]["path"].as_str().expect("a path");
    assert!(
        std::path::Path::new(located).starts_with(store.folder("site")),
        "the view's place is its own source's: {located}"
    );
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
fn a_repeat_copy_refreshes_the_views_without_a_duplicate_or_a_change_to_the_plan() {
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

    // The guide's content changes and it stops depending on the widget: its view follows both.
    let guide_view = map["tasks"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["task"] == landed["plan:guide"])
        .and_then(|entry| entry["projection"].as_str())
        .expect("the guide's view")
        .to_owned();
    assert_eq!(edges_of(&store, &guide_view, "depends-on").len(), 1);
    store.record(
        "plan",
        "tasks",
        "guide",
        &format!(
            "title: Document the retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"
        ),
        "Explain the jitter.",
    );
    copy_rollout(&store);
    assert_eq!(
        shown(&store, "task", &guide_view)["content"],
        "Explain the jitter."
    );
    assert!(edges_of(&store, &guide_view, "depends-on").is_empty());
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
    // A targeted update naming the status mirrors it the same way, and answers the view.
    let updated: Value =
        serde_json::from_str(&store.ok(&["task", "update", widget, "--status", "done", "--json"]))
            .expect("an update answer");
    assert_eq!(updated["projection"], view, "{updated:#}");
    assert_eq!(shown(&store, "task", view)["status"]["category"], "done");
    // One naming no status hands the view nothing.
    let renamed: Value = serde_json::from_str(&store.ok(&[
        "task",
        "update",
        widget,
        "--title",
        "Add retry to the widget, again",
        "--json",
    ]))
    .expect("an update answer");
    assert!(renamed.get("projection").is_none(), "{renamed:#}");
    assert_eq!(
        shown(&store, "task", view)["title"],
        "Add retry to the widget",
        "a view takes its content from the next copy, never from a narrow write"
    );
    // A status the delivery rule writes to a canonical ticket reaches its view too: the guide
    // is delivered by the private producer, which starts.
    let guide = &landed["plan:guide"];
    let guide_view = view_of(&store, &landed, guide);
    store.ok(&[
        "task",
        "update",
        &landed["plan:producer"],
        "--delivers",
        guide,
    ]);
    let started: Value = serde_json::from_str(&store.ok(&[
        "task",
        "status",
        "set",
        &landed["plan:producer"],
        "in-progress",
        "--json",
    ]))
    .expect("a status answer");
    let delivered = &started["delivered"][0];
    assert_eq!(delivered["outcome"], "written", "{started:#}");
    let moved = shown(&store, "task", guide)["status"]["category"].clone();
    assert_ne!(moved, "todo", "{started:#}");
    assert_eq!(
        shown(&store, "task", &guide_view)["status"]["category"],
        moved
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
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "Read [the notes](/srv/notes/retry.md) first.".to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "See <a href=\"../notes/retry.md\">the notes</a> first.".to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "<img src='file:///srv/notes/retry.png'>".to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "<img srcset=\"https://example.org/a.png 1x, ../notes/b.png 2x\">".to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "<div style=\"background: url('../notes/c.png')\">c</div>".to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "See <a href=\"https://example.org/faq\">the FAQ</a>.".to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "Read <img/src=\"../notes/d.png\"> first.".to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "<script>\nfetch('../notes/e.json');\n</script>\n".to_owned(),
        ),
        (
            "path",
            format!("title: Add retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
            "As drawn: ![the retry](./never-attached.png)".to_owned(),
        ),
    ] {
        let store = projecting_store();
        private_rollout(&store);
        store.record("plan", "tasks", "leaky", &front, &body);
        let before = store.tree();
        let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
        // An image reference the task holds no asset for is refused by the asset rule every
        // copy keeps, before a view is considered.
        let expected = match field {
            "path" if body.contains("never-attached") => "asset-not-held",
            "path" => "projection-reference",
            _ => "boundary-refused",
        };
        assert_eq!(kind, expected, "{field}: {said}");
        assert_eq!(
            store.tree(),
            before,
            "{field}: nothing was written anywhere"
        );
    }
    // A URL, a mail link and a fragment are carried in Markdown, and so is HTML of bare tags.
    let store = projecting_store();
    private_rollout(&store);
    store.record(
        "plan",
        "tasks",
        "linked",
        &format!("title: Link the docs\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
        "See [the spec](https://example.org/spec), [mail us](mailto:team@example.org) and \
         [below](#below).\n\n<details>\n<summary>More</summary>\n\nCompare a < b.<br/>\n\n</details>\n",
    );
    copy_rollout(&store);
    assert_eq!(
        files_under(&store, "site")
            .iter()
            .filter(|path| path.contains("pt-"))
            .count(),
        3,
        "the linked task has its view beside the other two"
    );
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
fn a_malformed_projection_map_is_refused_by_a_copy_and_a_status_write_rather_than_trusted() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let home = store.folder("vault").join("projects").join(format!(
        "{}.md",
        landed["plan:rollout"].trim_start_matches("vault:")
    ));
    let held = std::fs::read_to_string(&home).expect("the private project");
    let entries = projection_map(&store, &landed["plan:rollout"])["tasks"].clone();
    let first = entries[0].clone();
    for (case, map, said) in [
        (
            "a projection in a private source",
            json!({"schema_version": 1, "tasks": [
                {"task": first["task"], "projection": "vault:pt-elsewhere"}]}),
            "declared private",
        ),
        (
            "a task of another source",
            json!({"schema_version": 1, "tasks": [
                {"task": "plan:widget", "projection": first["projection"]}]}),
            "is not of vault",
        ),
        (
            "another schema version",
            json!({"schema_version": 2, "tasks": entries}),
            "schema_version 2",
        ),
        (
            "one task twice",
            json!({"schema_version": 1, "tasks": [
                first, {"task": first["task"], "projection": "site:pt-other"}]}),
            "twice",
        ),
        (
            "one projection twice",
            json!({"schema_version": 1, "tasks": [
                first, {"task": "vault:other", "projection": first["projection"]}]}),
            "twice",
        ),
        (
            "an id that is not qualified",
            json!({"schema_version": 1, "tasks": [
                {"task": first["task"], "projection": "pt-unqualified"}]}),
            "not a qualified id",
        ),
        (
            "an entry missing its projection",
            json!({"schema_version": 1, "tasks": [{"task": first["task"]}]}),
            "missing field",
        ),
        (
            "a field nobody defined",
            json!({"schema_version": 1, "tasks": entries, "owner": "someone"}),
            "unknown field",
        ),
    ] {
        let tampered: String = held
            .lines()
            .map(|line| {
                if line
                    .trim_start()
                    .starts_with("onetaskgraph.public_projections:")
                {
                    format!("  onetaskgraph.public_projections: {map}\n")
                } else {
                    format!("{line}\n")
                }
            })
            .filter(|line| !line.starts_with("    "))
            .collect();
        std::fs::write(&home, &tampered).expect("tampered");
        let before = store.tree();
        let (kind, said_copy) =
            store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
        assert_eq!(kind, "projections-malformed", "{case}: {said_copy}");
        assert!(said_copy.contains(said), "{case}: {said_copy}");
        assert_eq!(store.tree(), before, "{case}: nothing more was written");
        // A status write lands on the canonical task, and says it handed nothing on.
        let (kind, said_status) =
            store.refused(&["task", "status", "set", &landed["plan:guide"], "done"]);
        assert_eq!(kind, "projection-not-mirrored", "{case}: {said_status}");
        assert!(said_status.contains(said), "{case}: {said_status}");
        assert_eq!(
            shown(&store, "task", &landed["plan:guide"])["status"]["category"],
            "done",
            "{case}"
        );
    }
}

#[test]
fn a_status_the_public_side_cannot_name_refuses_the_whole_copy_before_anything_is_written() {
    // `site` names only `todo` and `done`; the widget is in progress.
    let store = Store::laid(Some(registered), |sandbox| {
        json!({
            "vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]},
            "site": {"config": {
                "root": sandbox.subdirectory("site"),
                "status_mapping": {"todo": "todo", "done": "done"},
            }},
        })
    });
    std::fs::create_dir_all(store.folder("site")).expect("the site folder");
    private_rollout(&store);
    store.record(
        "plan",
        "tasks",
        "widget",
        &format!(
            "title: Add retry to the widget\nstatus: in-progress\nproject: rollout\n\
             repositories: [{OPEN}]\ndepends_on: [producer]"
        ),
        PLAIN,
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_eq!(kind, "refused", "{said}");
    assert!(said.contains("site") && said.contains("status"), "{said}");
    assert_eq!(store.tree(), before, "nothing was written anywhere");
}

#[test]
fn a_view_that_cannot_be_written_takes_the_whole_copy_back() {
    // `site` can make its project and cannot hold a task file: the first view's write fails
    // after the canonical plan and the neutral project landed, and every write is taken back.
    let store = projecting_store();
    private_rollout(&store);
    std::fs::create_dir_all(store.folder("site")).expect("the site folder");
    std::fs::write(store.folder("site").join("tasks"), "not a folder").expect("a blocking file");
    let before = store.tree();
    let output = store.run(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        store.tree(),
        before,
        "the canonical plan, the neutral project and the map are all taken back"
    );
    // They were written first: the folders their files were made in are still there, empty.
    for written in [
        store.folder("vault").join("tasks"),
        store.folder("site").join("projects"),
    ] {
        assert!(written.is_dir(), "{} was written to", written.display());
    }
}

#[test]
fn a_plan_no_longer_routed_anywhere_public_forgets_its_views_and_leaves_them_standing() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let site = tree(&store.folder("site"));
    // The route is taken away.
    let sources = json!({
        "plan": folder(&store.sandbox, "plan", "private"),
        "vault": folder(&store.sandbox, "vault", "private"),
        "site": folder(&store.sandbox, "site", "public"),
    });
    let policy = store.onevcs.as_ref().map(OnevcsHome::write_policy);
    let mut document = json!({ "sources": sources });
    if let Some(policy) = policy {
        document["write_policy"] = policy;
    }
    store.sandbox.project_document(&document.to_string());
    copy_rollout(&store);
    assert_eq!(
        projection_map(&store, &landed["plan:rollout"])["tasks"],
        json!([]),
        "the map names no view"
    );
    assert_eq!(
        tree(&store.folder("site")),
        site,
        "every view is left as it was"
    );
}

#[test]
fn views_that_depend_on_each_other_are_both_landed_with_both_edges() {
    let store = projecting_store();
    private_rollout(&store);
    store.record(
        "plan",
        "tasks",
        "widget",
        &format!(
            "title: Add retry to the widget\nstatus: todo\nproject: rollout\n\
             repositories: [{OPEN}]\ndepends_on: [guide]"
        ),
        PLAIN,
    );
    let landed = copy_rollout(&store);
    let map = projection_map(&store, &landed["plan:rollout"]);
    let view_of = |canonical: &str| {
        map["tasks"]
            .as_array()
            .expect("entries")
            .iter()
            .find(|entry| entry["task"] == canonical)
            .and_then(|entry| entry["projection"].as_str())
            .unwrap_or_else(|| panic!("a view of {canonical}: {map:#}"))
            .to_owned()
    };
    let widget = view_of(&landed["plan:widget"]);
    let guide = view_of(&landed["plan:guide"]);
    assert_eq!(
        edges_of(&store, &widget, "depends-on"),
        vec![(widget.clone(), guide.clone())]
    );
    assert_eq!(
        edges_of(&store, &guide, "depends-on"),
        vec![(guide.clone(), widget.clone())]
    );
    let before = store.tree();
    copy_rollout(&store);
    assert_eq!(store.tree(), before, "a repeat writes nothing");
}

#[test]
fn a_view_carries_its_task_s_image_assets_byte_for_byte() {
    let store = projecting_store();
    private_rollout(&store);
    let shot: Vec<u8> = (0..=255_u8).cycle().take(4096).collect();
    let image = store.sandbox.project().join("shot.png");
    std::fs::write(&image, &shot).expect("the image");
    let body = store.body("Retry as drawn: ![the retry](./shot.png)");
    let image = image.display().to_string();
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "rollout",
        "--title",
        "Draw the retry",
        "--body-file",
        &body,
        "--repository",
        OPEN,
        "--asset",
        &image,
    ]);
    let landed = copy_rollout(&store);
    let map = projection_map(&store, &landed["plan:rollout"]);
    let drawn = landed
        .iter()
        .find(|(source, _)| source.starts_with("plan:draw-the-retry"))
        .map(|(_, destination)| destination.clone())
        .expect("the drawn task landed");
    let view = map["tasks"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["task"] == drawn.as_str())
        .and_then(|entry| entry["projection"].as_str())
        .expect("its view")
        .to_owned();
    let shown_view: Value =
        serde_json::from_str(&store.ok(&["task", "show", &view, "--json"])).expect("the view");
    assert_eq!(
        shown_view["items"][0]["item"]["content"],
        "Retry as drawn: ![the retry](./shot.png)"
    );
    let asset = &shown_view["assets"][0];
    assert_eq!(asset["name"], "shot.png", "{shown_view:#}");
    let held = std::fs::read(asset["path"].as_str().expect("a path")).expect("the bytes");
    assert_eq!(held, shot);
    assert_site_neutral(&store);
}

#[test]
fn a_status_its_view_cannot_take_is_reported_after_the_canonical_write_and_recovers() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let view = projection_map(&store, &landed["plan:rollout"])["tasks"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["task"] == widget.as_str())
        .and_then(|entry| entry["projection"].as_str())
        .expect("the widget's view")
        .to_owned();
    // A person deletes the view in public.
    let file = store
        .folder("site")
        .join("tasks")
        .join(format!("{}.md", view.trim_start_matches("site:")));
    std::fs::remove_file(&file).expect("the view deleted");
    let (kind, said) = store.refused(&["task", "status", "set", widget, "done"]);
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert!(said.contains(widget) && said.contains(&view), "{said}");
    assert_eq!(shown(&store, "task", widget)["status"]["category"], "done");
    // The next copy makes the view again from the plan's canonical record — the same one it
    // lands in `vault` — and the next status write reaches it.
    copy_rollout(&store);
    assert_eq!(
        shown(&store, "task", &view)["status"]["category"],
        shown(&store, "task", widget)["status"]["category"]
    );
    store.ok(&["task", "status", "set", widget, "in-progress"]);
    assert_eq!(
        shown(&store, "task", &view)["status"]["category"],
        "in-progress"
    );
}

#[test]
fn a_copy_killed_after_it_made_a_view_and_before_it_recorded_it_resumes_without_a_second() {
    // `site` is reached over the plugin protocol through a relay that ends the engine once the
    // second view is written — before the engine reads the answer, and so before it records
    // that view in the map. The next copy is a resumed one over the same configuration.
    let relay =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/rewriting_relay.py");
    let store = Store::laid(Some(registered), |sandbox| {
        let marker_text = sandbox.project().join("interrupted").display().to_string();
        let site = sandbox.subdirectory("site");
        std::fs::create_dir_all(&site).expect("the site folder");
        json!({
            "vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]},
            "site": {"plugin": "subprocess", "visibility": "public", "config": {
                "command": crate::document_store::interpreter().to_string_lossy(),
                "args": [relay.to_string_lossy()],
                "settings": {
                    "host": onetaskgraph_e2e_support::source_binary(),
                    "interrupt": "write_task", "interrupt_skip": 1,
                    "interrupt_marker": marker_text,
                    "kind": "local-md", "config": {"root": site},
                },
            }},
        })
    });
    let marker = store.sandbox.project().join("interrupted");
    private_rollout(&store);
    let interrupted = store.run(&["project", "copy", "plan:rollout", "--to", "vault", "--json"]);
    assert_ne!(
        interrupted.status.code(),
        Some(0),
        "the engine was ended part way"
    );
    assert!(marker.exists(), "the relay ended it");
    let views_written = files_under(&store, "site")
        .into_iter()
        .filter(|path| path.contains("pt-"))
        .count();
    assert_eq!(
        views_written, 2,
        "both views were written before the engine ended"
    );

    let landed = copy_rollout(&store);
    let views: Vec<String> = files_under(&store, "site")
        .into_iter()
        .filter(|path| path.contains("pt-"))
        .collect();
    assert_eq!(views.len(), 2, "no view was made twice: {views:?}");
    let map = projection_map(&store, &landed["plan:rollout"]);
    assert_eq!(map["tasks"].as_array().map(Vec::len), Some(2), "{map:#}");
    assert_site_neutral(&store);
}

/// The repository the loopback board files every issue in.
const BOARD_REPOSITORY: &str = "github.com/nickderobertis/onetaskgraph";

/// [`registered`], with the board's own repository answered public.
fn registered_with_board(home: &mut OnevcsHome) {
    registered(home);
    home.public("nickderobertis", "onetaskgraph");
}

#[test]
fn a_public_board_is_sent_the_neutral_views_and_nothing_of_the_private_plan() {
    // The public side is a GitHub board, stood in for at its HTTP transport alone: every request
    // the plugin sends it is recorded, and none may carry anything of the private plan.
    let mut handle = None;
    // The board files issues in its own repository, so the public work names that one, which
    // the policy answers public.
    let store = Store::laid(Some(registered_with_board), |sandbox| {
        let (config, board) = crate::fixtures::github_projects_with_items(sandbox, Vec::new());
        handle = Some(board);
        json!({
            "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
            "vault": {"routes": [{"repositories": [BOARD_REPOSITORY], "to": "board"}]},
        })
    });
    let board = handle.expect("the board");
    private_rollout_of(&store, BOARD_REPOSITORY);
    let landed = copy_rollout(&store);
    let sent = board.served();
    let mutations: Vec<&(String, Value)> = sent
        .iter()
        .filter(|(document, _)| document.trim_start().starts_with("mutation"))
        .collect();
    assert!(!mutations.is_empty(), "the views were written to the board");
    for (document, variables) in &sent {
        let request = format!("{document}\n{variables}");
        for named in NEVER_PUBLIC {
            assert!(
                !variables.to_string().contains(named),
                "a request to the board carries {named:?}:\n{request}"
            );
        }
    }
    let map = projection_map(&store, &landed["plan:rollout"]);
    let entries = map["tasks"].as_array().expect("the entries");
    assert_eq!(entries.len(), 2, "{map:#}");
    for entry in entries {
        let view = entry["projection"].as_str().expect("a view");
        assert!(view.starts_with("board:"), "{map:#}");
        let shown_view = shown(&store, "task", view);
        assert!(
            ["Add retry to the widget", "Document the retry"]
                .contains(&shown_view["title"].as_str().unwrap_or_default()),
            "{shown_view:#}"
        );
    }

    // A repeat sends the board no mutation, and a canonical status change sends it the
    // category alone.
    let before = board.served().len();
    copy_rollout(&store);
    assert!(
        !board.served()[before..]
            .iter()
            .any(|(document, _)| document.trim_start().starts_with("mutation")),
        "a repeat writes nothing to the board"
    );
    let set: Value = serde_json::from_str(&store.ok(&[
        "task",
        "status",
        "set",
        &landed["plan:widget"],
        "done",
        "--json",
    ]))
    .expect("a status answer");
    let view = set["projection"].as_str().expect("mirrored to the view");
    assert_eq!(shown(&store, "task", view)["status"]["category"], "done");
    for (document, variables) in board.served() {
        for named in NEVER_PUBLIC {
            assert!(
                !variables.to_string().contains(named),
                "a request to the board carries {named:?}:\n{document}\n{variables}"
            );
        }
    }
}

#[test]
fn a_task_already_recorded_private_stays_private_and_unprojected_on_every_recopy() {
    // An earlier release copied `widget` private because its project was: its canonical record
    // says so, though its own source record does not. A re-copy and a status write keep it
    // private, and it never gains a view.
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let record = store
        .folder("vault")
        .join("tasks")
        .join(format!("{}.md", widget.trim_start_matches("vault:")));
    let held = std::fs::read_to_string(&record).expect("the canonical widget");
    let legacy = held.replacen("---\n", "---\nclassification: private\n", 1);
    assert_ne!(held, legacy);
    std::fs::write(&record, legacy).expect("recorded private");
    // Its view, made before it was recorded private, is the one a person removes by hand.
    let map = projection_map(&store, &landed["plan:rollout"]);
    let view = map["tasks"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["task"] == widget.as_str())
        .and_then(|entry| entry["projection"].as_str())
        .expect("the widget's view")
        .to_owned();
    std::fs::remove_file(
        store
            .folder("site")
            .join("tasks")
            .join(format!("{}.md", view.trim_start_matches("site:"))),
    )
    .expect("the view removed");

    copy_rollout(&store);
    assert_eq!(shown(&store, "task", widget)["classification"], "private");
    let map = projection_map(&store, &landed["plan:rollout"]);
    assert!(
        !map["tasks"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|entry| entry["task"] == widget.as_str()),
        "the map no longer names a task recorded private: {map:#}"
    );
    assert!(
        !files_under(&store, "site")
            .iter()
            .any(|path| path.ends_with(&format!("{}.md", view.trim_start_matches("site:")))),
        "no view is made of a task recorded private"
    );
    store.ok(&["task", "status", "set", widget, "done"]);
    assert_eq!(shown(&store, "task", widget)["classification"], "private");
    assert_site_neutral(&store);
}

#[test]
fn a_view_s_image_is_brought_in_step_when_only_its_bytes_change() {
    let store = projecting_store();
    private_rollout(&store);
    let image = store.sandbox.project().join("shot.png");
    std::fs::write(&image, [1_u8; 64]).expect("the image");
    let body = store.body("As drawn: ![the retry](./shot.png)");
    let image = image.display().to_string();
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "rollout",
        "--title",
        "Draw it",
        "--body-file",
        &body,
        "--repository",
        OPEN,
        "--asset",
        &image,
    ]);
    let landed = copy_rollout(&store);
    let drawn = &landed["plan:draw-it"];
    let view = projection_map(&store, &landed["plan:rollout"])["tasks"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["task"] == drawn.as_str())
        .and_then(|entry| entry["projection"].as_str())
        .expect("its view")
        .to_owned();
    // The image is redrawn at the source and nothing else of the task changes.
    let source: Value =
        serde_json::from_str(&store.ok(&["task", "show", "plan:draw-it", "--json"]))
            .expect("the task");
    let held = source["assets"][0]["path"]
        .as_str()
        .expect("its image's path")
        .to_owned();
    std::fs::write(&held, [2_u8; 64]).expect("redrawn");
    copy_rollout(&store);
    let shown_view: Value =
        serde_json::from_str(&store.ok(&["task", "show", &view, "--json"])).expect("the view");
    let path = shown_view["assets"][0]["path"]
        .as_str()
        .expect("its image's path");
    assert_eq!(std::fs::read(path).expect("the bytes"), vec![2_u8; 64]);
}

#[test]
fn a_copy_that_fails_after_updating_a_view_puts_the_view_and_the_plan_back() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let map = projection_map(&store, &landed["plan:rollout"]);
    let view_of = |canonical: &str| {
        map["tasks"]
            .as_array()
            .expect("entries")
            .iter()
            .find(|entry| entry["task"] == canonical)
            .and_then(|entry| entry["projection"].as_str())
            .expect("a view")
            .to_owned()
    };
    let widget_view = view_of(&landed["plan:widget"]);
    let guide_view = view_of(&landed["plan:guide"]);
    // The widget is renamed, so its view is rewritten first; the guide's view then cannot be
    // read, because a folder stands where its file was.
    store.record(
        "plan",
        "tasks",
        "widget",
        &format!(
            "title: Add jittered retry to the widget\nstatus: todo\nproject: rollout\n\
             repositories: [{OPEN}]\ndepends_on: [producer]"
        ),
        "Retry with a capped exponential backoff. See [the spec](https://example.org/retry).",
    );
    let guide_file = store
        .folder("site")
        .join("tasks")
        .join(format!("{}.md", guide_view.trim_start_matches("site:")));
    let guide_held = std::fs::read(&guide_file).expect("the guide's view");
    std::fs::remove_file(&guide_file).expect("moved aside");
    std::fs::create_dir_all(guide_file.join("in-the-way")).expect("a folder in its place");
    let vault_before = tree(&store.folder("vault"));
    let site_before = tree(&store.folder("site"));
    let output = store.run(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        tree(&store.folder("vault")),
        vault_before,
        "the plan is put back"
    );
    assert_eq!(
        tree(&store.folder("site")),
        site_before,
        "every view is put back"
    );
    std::fs::remove_dir_all(&guide_file).expect("the folder taken away");
    std::fs::write(&guide_file, guide_held).expect("the guide's view put back");
    assert_eq!(
        shown(&store, "task", &widget_view)["title"],
        "Add retry to the widget"
    );
}

#[cfg(unix)]
#[test]
fn a_projection_map_the_private_project_will_not_take_takes_the_new_view_back() {
    use std::os::unix::fs::PermissionsExt;
    let store = projecting_store();
    private_rollout(&store);
    copy_rollout(&store);
    // A new public task: its view is made, and recording it on the private project is refused.
    store.record(
        "plan",
        "tasks",
        "extra",
        &format!("title: Tune the retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    let projects = store.folder("vault").join("projects");
    let before = store.tree();
    std::fs::set_permissions(&projects, std::fs::Permissions::from_mode(0o555))
        .expect("the private project's folder made read-only");
    let output = store.run(&["project", "copy", "plan:rollout", "--to", "vault"]);
    std::fs::set_permissions(&projects, std::fs::Permissions::from_mode(0o755))
        .expect("the folder made writable again");
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        store.tree(),
        before,
        "the new view and the new canonical task are taken back"
    );
}

#[test]
fn a_status_whose_private_project_cannot_be_read_says_it_reached_no_view() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let home = store.folder("vault").join("projects").join(format!(
        "{}.md",
        landed["plan:rollout"].trim_start_matches("vault:")
    ));
    std::fs::write(&home, "---\ntitle: [unclosed\n---\n").expect("the project made unreadable");
    let widget = &landed["plan:widget"];
    let (kind, said) = store.refused(&["task", "status", "set", widget, "done"]);
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert!(said.contains(widget), "{said}");
    assert_eq!(shown(&store, "task", widget)["status"]["category"], "done");
}

#[test]
fn a_dry_run_of_a_private_plan_with_views_writes_nothing_anywhere() {
    let store = projecting_store();
    private_rollout(&store);
    let before = store.tree();
    store.ok(&[
        "project",
        "copy",
        "plan:rollout",
        "--to",
        "vault",
        "--dry-run",
    ]);
    assert_eq!(store.tree(), before, "nothing was written anywhere");
    // A view a dry run would refuse, it refuses.
    store.record(
        "plan",
        "tasks",
        "leaky",
        &format!("title: Port the quietharbor retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    let (kind, said) = store.refused(&[
        "project",
        "copy",
        "plan:rollout",
        "--to",
        "vault",
        "--dry-run",
    ]);
    assert_eq!(kind, "boundary-refused", "{said}");
}

/// The view the map of `landed`'s private project records for the canonical task `canonical`.
fn view_of(store: &Store, landed: &BTreeMap<String, String>, canonical: &str) -> String {
    projection_map(store, &landed["plan:rollout"])["tasks"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["task"] == canonical)
        .and_then(|entry| entry["projection"].as_str())
        .unwrap_or_else(|| panic!("a view of {canonical}"))
        .to_owned()
}

/// The file a folder of Markdown keeps the item `id` of `kind` in.
fn record_file(store: &Store, kind: &str, id: &str) -> std::path::PathBuf {
    let (folder, native) = id.split_once(':').expect("a qualified id");
    store.folder(folder).join(kind).join(format!("{native}.md"))
}

#[test]
fn an_opaque_project_id_another_project_already_holds_is_refused_and_the_copy_taken_back() {
    // The neutral project's id is derived from the private project's, so a first store tells
    // a second which id its copy would make it under.
    let first = projecting_store();
    private_rollout(&first);
    let landed = copy_rollout(&first);
    let neutral = shown(
        &first,
        "task",
        &view_of(&first, &landed, &landed["plan:widget"]),
    )["project"]
        .as_str()
        .expect("filed under the neutral project")
        .to_owned();

    let store = projecting_store();
    private_rollout(&store);
    store.record(
        "site",
        "projects",
        &neutral,
        "title: Somebody's roadmap\nstatus: todo",
        "Their own plans.",
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_eq!(kind, "projections-malformed", "{said}");
    assert!(said.contains(&neutral), "{said}");
    assert_eq!(store.tree(), before, "the canonical plan is taken back too");
}

#[test]
fn a_view_filed_under_a_project_that_is_not_neutral_is_refused_by_a_copy_and_a_mirror() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let view = view_of(&store, &landed, widget);
    store.record(
        "site",
        "projects",
        "roadmap",
        "title: Roadmap\nstatus: todo",
        "Prose.",
    );
    let file = record_file(&store, "tasks", &view);
    let moved = std::fs::read_to_string(&file)
        .expect("the view")
        .lines()
        .map(|line| {
            if line.starts_with("project:") {
                "project: roadmap\n".to_owned()
            } else {
                format!("{line}\n")
            }
        })
        .collect::<String>();
    std::fs::write(&file, moved).expect("the view moved");
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_eq!(kind, "projections-malformed", "{said}");
    assert!(said.contains("roadmap"), "{said}");
    assert_eq!(store.tree(), before, "nothing more was written");
    let (kind, said) = store.refused(&["task", "status", "set", widget, "done"]);
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert!(said.contains("roadmap"), "{said}");
    assert_eq!(shown(&store, "task", widget)["status"]["category"], "done");
    assert_eq!(shown(&store, "task", &view)["status"]["category"], "todo");
}

#[test]
fn a_view_a_person_took_out_of_its_project_is_left_and_a_new_view_is_recorded() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let view = view_of(&store, &landed, widget);
    let file = record_file(&store, "tasks", &view);
    let taken_out: String = std::fs::read_to_string(&file)
        .expect("the view")
        .lines()
        .filter(|line| !line.starts_with("project:"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&file, &taken_out).expect("taken out of its project");
    // A mirror will not write to a view filed under no project.
    let (kind, said) = store.refused(&["task", "status", "set", widget, "done"]);
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert!(said.contains("no project"), "{said}");
    // Set back on the canonical task, and again reported as reaching no view.
    let _ = store.run(&["task", "status", "set", widget, "todo"]);
    copy_rollout(&store);
    let recorded = view_of(&store, &landed, widget);
    assert_ne!(recorded, view, "a new view is recorded");
    assert_eq!(
        std::fs::read_to_string(&file).expect("the old one"),
        taken_out,
        "the one a person took out is left as they left it"
    );
    let fresh = shown(&store, "task", &recorded);
    assert_eq!(fresh["title"], "Add retry to the widget");
    assert_eq!(
        fresh["project"],
        shown(
            &store,
            "task",
            &view_of(&store, &landed, &landed["plan:guide"])
        )["project"]
    );
    let before = store.tree();
    copy_rollout(&store);
    assert_eq!(store.tree(), before, "and the next copy writes nothing");
}

#[test]
fn a_status_whose_view_s_source_is_unavailable_says_it_reached_no_view() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let view = view_of(&store, &landed, widget);
    let site = store.folder("site");
    let aside = store.sandbox.project().join("site-aside");
    std::fs::rename(&site, &aside).expect("the public folder taken away");
    let (kind, said) = store.refused(&["task", "status", "set", widget, "done"]);
    std::fs::rename(&aside, &site).expect("the public folder put back");
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert!(said.contains(widget) && said.contains(&view), "{said}");
    assert_eq!(shown(&store, "task", widget)["status"]["category"], "done");
}

#[test]
fn a_plan_projects_into_each_public_source_its_tasks_route_to_and_follows_a_task_that_moves() {
    // `vault` routes `openco` work to `site` and `shopco` work to `shop`, both public.
    let store = Store::laid(Some(registered), |sandbox| {
        json!({
            "shop": folder(sandbox, "shop", "public"),
            "vault": {"routes": [
                {"repositories": ["github.com/openco/*"], "to": "site"},
                {"repositories": ["github.com/shopco/*"], "to": "shop"},
            ]},
        })
    });
    for folder in ["site", "shop"] {
        std::fs::create_dir_all(store.folder(folder)).expect("a public folder");
    }
    private_rollout(&store);
    store.record(
        "plan",
        "tasks",
        "storefront",
        &format!(
            "title: Retry the checkout\nstatus: todo\nproject: rollout\nrepositories: [{SHOP}]"
        ),
        PLAIN,
    );
    let landed = copy_rollout(&store);
    let in_shop = view_of(&store, &landed, &landed["plan:storefront"]);
    assert!(in_shop.starts_with("shop:pt-"), "{in_shop}");
    for (folder, views) in [("site", 2), ("shop", 1)] {
        let files = files_under(&store, folder);
        assert_eq!(
            files.iter().filter(|path| path.contains("pw-")).count(),
            1,
            "{folder} holds its own neutral project: {files:?}"
        );
        assert_eq!(
            files.iter().filter(|path| path.contains("pt-")).count(),
            views,
            "{folder}: {files:?}"
        );
    }
    // The guide moves to `shopco`: its view is made in `shop` and recorded there, and the one
    // in `site` is left standing.
    let in_site = view_of(&store, &landed, &landed["plan:guide"]);
    store.record(
        "plan",
        "tasks",
        "guide",
        &format!(
            "title: Document the retry\nstatus: todo\nproject: rollout\n\
             repositories: [{SHOP}]\ndepends_on: [widget]"
        ),
        PLAIN,
    );
    copy_rollout(&store);
    let moved = view_of(&store, &landed, &landed["plan:guide"]);
    assert!(moved.starts_with("shop:pt-"), "{moved}");
    assert!(
        record_file(&store, "tasks", &in_site).exists(),
        "the old view is left"
    );
    // A view in `shop` depends on no view in `site`: an edge never crosses public sources.
    assert!(edges_of(&store, &moved, "depends-on").is_empty());
}

#[test]
fn a_copy_that_fails_after_changing_a_view_s_image_puts_its_bytes_back() {
    let store = projecting_store();
    private_rollout(&store);
    let image = store.sandbox.project().join("shot.png");
    std::fs::write(&image, [1_u8; 64]).expect("the image");
    let body = store.body("As drawn: ![the retry](./shot.png)");
    let image = image.display().to_string();
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "rollout",
        "--title",
        "Draw it",
        "--body-file",
        &body,
        "--repository",
        OPEN,
        "--asset",
        &image,
    ]);
    // The guide is written after the drawing, which it depends on.
    store.record(
        "plan",
        "tasks",
        "guide",
        &format!(
            "title: Document the retry\nstatus: todo\nproject: rollout\n\
             repositories: [{OPEN}]\ndepends_on: [draw-it]"
        ),
        PLAIN,
    );
    let landed = copy_rollout(&store);
    let guide_view = view_of(&store, &landed, &landed["plan:guide"]);
    let source: Value =
        serde_json::from_str(&store.ok(&["task", "show", "plan:draw-it", "--json"]))
            .expect("the task");
    let held = source["assets"][0]["path"]
        .as_str()
        .expect("its image")
        .to_owned();
    std::fs::write(&held, [2_u8; 64]).expect("redrawn");
    let guide_file = record_file(&store, "tasks", &guide_view);
    let guide_held = std::fs::read(&guide_file).expect("the guide's view");
    std::fs::remove_file(&guide_file).expect("moved aside");
    std::fs::create_dir_all(guide_file.join("in-the-way")).expect("a folder in its place");
    let site_before = tree(&store.folder("site"));
    let output = store.run(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        tree(&store.folder("site")),
        site_before,
        "the drawing's view holds its old bytes again"
    );
    std::fs::remove_dir_all(&guide_file).expect("the folder taken away");
    std::fs::write(&guide_file, guide_held).expect("the guide's view put back");
}

#[test]
fn a_status_its_view_s_source_cannot_name_or_reach_says_it_reached_no_view() {
    // `site` names only `todo` and `done`.
    let store = Store::laid(Some(registered), |sandbox| {
        json!({
            "vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]},
            "site": {"config": {
                "root": sandbox.subdirectory("site"),
                "status_mapping": {"todo": "todo", "done": "done"},
            }},
        })
    });
    std::fs::create_dir_all(store.folder("site")).expect("the site folder");
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let (kind, said) = store.refused(&["task", "status", "set", widget, "in-progress"]);
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert_eq!(
        shown(&store, "task", widget)["status"]["category"],
        "in-progress"
    );
    // A view whose source nothing configures cannot be reached at all.
    let home = record_file(&store, "projects", &landed["plan:rollout"]);
    let held = std::fs::read_to_string(&home).expect("the private project");
    let view = view_of(&store, &landed, widget);
    std::fs::write(&home, held.replace(&view, "nowhere:pt-gone")).expect("re-pointed");
    let (kind, said) = store.refused(&["task", "status", "set", widget, "done"]);
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert!(said.contains("nowhere"), "{said}");
}

#[test]
fn a_copy_that_fails_after_giving_a_view_its_first_image_takes_the_image_away_again() {
    let store = projecting_store();
    private_rollout(&store);
    // Where a folder of Markdown keeps a task's images, learned from a task holding one.
    let image = store.sandbox.project().join("shot.png");
    std::fs::write(&image, [7_u8; 64]).expect("the image");
    let body = store.body("![probe](./shot.png)");
    let image = image.display().to_string();
    // Filed under a project of its own, so the plan's copy never carries it.
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "scratch",
        "--title",
        "Probe",
        "--body-file",
        &body,
        "--asset",
        &image,
    ]);
    let probe: Value = serde_json::from_str(&store.ok(&["task", "show", "plan:probe", "--json"]))
        .expect("the probe");
    let probe_path = probe["assets"][0]["path"]
        .as_str()
        .expect("its image")
        .to_owned();
    let landed = copy_rollout(&store);
    let guide_view = view_of(&store, &landed, &landed["plan:guide"]);
    // The widget, whose view holds no image, now references one, and the guide's view, written
    // after it, cannot be read.
    let widget_image = probe_path.replace("probe", "widget");
    std::fs::create_dir_all(
        std::path::Path::new(&widget_image)
            .parent()
            .expect("a folder"),
    )
    .expect("the widget's image folder");
    std::fs::write(&widget_image, [9_u8; 64]).expect("the widget's image");
    store.record(
        "plan",
        "tasks",
        "widget",
        &format!(
            "title: Add retry to the widget\nstatus: todo\nproject: rollout\n\
             repositories: [{OPEN}]\ndepends_on: [producer]"
        ),
        "As drawn: ![the retry](./shot.png)",
    );
    let shown_widget: Value =
        serde_json::from_str(&store.ok(&["task", "show", "plan:widget", "--json"]))
            .expect("the widget");
    assert_eq!(
        shown_widget["assets"][0]["name"], "shot.png",
        "{shown_widget:#}"
    );
    let guide_file = record_file(&store, "tasks", &guide_view);
    let guide_held = std::fs::read(&guide_file).expect("the guide's view");
    std::fs::remove_file(&guide_file).expect("moved aside");
    std::fs::create_dir_all(guide_file.join("in-the-way")).expect("a folder in its place");
    let site_before = tree(&store.folder("site"));
    let output = store.run(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        tree(&store.folder("site")),
        site_before,
        "the widget's view holds no image again"
    );
    std::fs::remove_dir_all(&guide_file).expect("the folder taken away");
    std::fs::write(&guide_file, guide_held).expect("the guide's view put back");
}

#[test]
fn a_neutral_project_status_the_public_side_cannot_name_refuses_before_anything_is_written() {
    // `site` names `done` alone: every public task is done, and the neutral project, which is
    // made at `todo`, is what it cannot name.
    let store = Store::laid(Some(registered), |sandbox| {
        json!({
            "vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]},
            "site": {"config": {
                "root": sandbox.subdirectory("site"),
                "status_mapping": {"done": "done"},
            }},
        })
    });
    std::fs::create_dir_all(store.folder("site")).expect("the site folder");
    private_rollout(&store);
    for (id, title, depends) in [
        ("widget", "Add retry to the widget", "producer"),
        ("guide", "Document the retry", "widget"),
    ] {
        store.record(
            "plan",
            "tasks",
            id,
            &format!(
                "title: {title}\nstatus: done\nproject: rollout\nrepositories: [{OPEN}]\n\
                 depends_on: [{depends}]"
            ),
            PLAIN,
        );
    }
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_eq!(kind, "refused", "{said}");
    assert!(said.contains("site") && said.contains("todo"), "{said}");
    assert_eq!(store.tree(), before, "nothing was written anywhere");
}

#[test]
fn every_field_a_person_adds_to_a_view_is_taken_off_by_the_next_copy() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let view = view_of(&store, &landed, &landed["plan:widget"]);
    let file = record_file(&store, "tasks", &view);
    let held = std::fs::read_to_string(&file).expect("the view");
    let edited = held.replacen(
        "---\n",
        "---\nlabels: [hot]\npriority: urgent\nmetadata: {team.note: edited in public}\n",
        1,
    );
    std::fs::write(&file, edited).expect("edited in public");
    let edited_view = shown(&store, "task", &view);
    assert_eq!(edited_view["priority"], "urgent", "{edited_view:#}");
    let plan_before = tree(&store.folder("plan"));
    let vault_before = tree(&store.folder("vault"));
    copy_rollout(&store);
    let restored = shown(&store, "task", &view);
    assert_eq!(restored["labels"], json!([]), "{restored:#}");
    assert_eq!(restored["priority"], "none", "{restored:#}");
    assert_eq!(restored["metadata"], json!({}), "{restored:#}");
    assert_eq!(
        tree(&store.folder("plan")),
        plan_before,
        "the plan is untouched"
    );
    assert_eq!(
        tree(&store.folder("vault")),
        vault_before,
        "the canonical copy is untouched"
    );
}

#[test]
fn a_view_recorded_private_is_never_written_public_again() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let view = view_of(&store, &landed, &landed["plan:widget"]);
    let file = record_file(&store, "tasks", &view);
    let held = std::fs::read_to_string(&file).expect("the view");
    std::fs::write(
        &file,
        held.replacen("---\n", "---\nclassification: private\n", 1),
    )
    .expect("recorded private");
    // The widget changes, so its view would be written.
    store.record(
        "plan",
        "tasks",
        "widget",
        &format!(
            "title: Add jittered retry to the widget\nstatus: todo\nproject: rollout\n\
             repositories: [{OPEN}]\ndepends_on: [producer]"
        ),
        PLAIN,
    );
    let before = store.tree();
    let (kind, said) = store.refused(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_eq!(kind, "projections-malformed", "{said}");
    assert!(said.contains("recorded private"), "{said}");
    assert_eq!(store.tree(), before, "nothing more was written");
}

#[test]
fn a_board_view_whose_create_answer_was_lost_is_found_again_rather_than_made_twice() {
    // A board issues its own ids, so the opaque one a copy asks for finds nothing there: the
    // view the board stored, whose answer the copy never read, is found by what it says.
    let mut handle = None;
    let store = Store::laid(Some(registered_with_board), |sandbox| {
        let (config, board) = crate::fixtures::github_projects_with_items(sandbox, Vec::new());
        handle = Some(board);
        json!({
            "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
            "vault": {"routes": [{"repositories": [BOARD_REPOSITORY], "to": "board"}]},
        })
    });
    let board = handle.expect("the board");
    private_rollout_of(&store, BOARD_REPOSITORY);
    let landed = copy_rollout(&store);
    // A new public task: the board stores its view and drops the connection before answering.
    store.record(
        "plan",
        "tasks",
        "tune",
        &format!(
            "title: Tune the retry\nstatus: todo\nproject: rollout\nrepositories: [{BOARD_REPOSITORY}]"
        ),
        PLAIN,
    );
    let creates = |board: &crate::fixtures::GitHubBoardFields| {
        board
            .served()
            .iter()
            .filter(|(document, variables)| {
                document.contains("createIssue(input:$input)")
                    && variables.to_string().contains("Tune the retry")
            })
            .count()
    };
    board.drop_response_once("createIssue(input:$input)");
    let output = store.run(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(creates(&board), 1, "the board stored the view once");

    copy_rollout(&store);
    assert_eq!(creates(&board), 1, "and no copy made it a second time");
    let tune = landed_ids(&store.ok(&[
        "project",
        "copy",
        "plan:rollout",
        "--to",
        "vault",
        "--json",
    ]))["plan:tune"]
        .clone();
    let view = view_of(&store, &landed, &tune);
    assert!(view.starts_with("board:"), "{view}");
    assert_eq!(shown(&store, "task", &view)["title"], "Tune the retry");
}

#[test]
fn an_unrecorded_task_is_taken_as_a_view_only_when_it_alone_says_what_the_view_says() {
    for strays in [1_usize, 2] {
        let store = projecting_store();
        private_rollout(&store);
        let landed = copy_rollout(&store);
        let neutral = shown(
            &store,
            "task",
            &view_of(&store, &landed, &landed["plan:widget"]),
        )["project"]
            .as_str()
            .expect("the neutral project")
            .to_owned();
        // A new public task, and under the neutral project tasks no map names that read as
        // its view would: one is the view a lost answer left behind, two are ambiguous.
        store.record(
            "plan",
            "tasks",
            "tune",
            &format!(
                "title: Tune the retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"
            ),
            PLAIN,
        );
        let stray_ids: Vec<String> = (1..=strays).map(|n| format!("stray-{n}")).collect();
        for stray in &stray_ids {
            store.record(
                "site",
                "tasks",
                stray,
                &format!(
                    "title: Tune the retry\nstatus: todo\nproject: {neutral}\nrepositories: [{OPEN}]"
                ),
                PLAIN,
            );
        }
        let held: Vec<Vec<u8>> = stray_ids
            .iter()
            .map(|stray| {
                std::fs::read(record_file(&store, "tasks", &format!("site:{stray}")))
                    .expect("a stray")
            })
            .collect();
        let tune =
            landed_ids(&store.ok(&["project", "copy", "plan:rollout", "--to", "vault", "--json"]))
                ["plan:tune"]
                .clone();
        let view = view_of(&store, &landed, &tune);
        if strays == 1 {
            assert_eq!(
                view, "site:stray-1",
                "the one that says what the view says is taken"
            );
        } else {
            assert!(
                view.starts_with("site:pt-"),
                "neither of two is taken: {view}"
            );
            for (stray, bytes) in stray_ids.iter().zip(&held) {
                assert_eq!(
                    &std::fs::read(record_file(&store, "tasks", &format!("site:{stray}")))
                        .expect("still there"),
                    bytes,
                    "{stray} is left as it was"
                );
            }
        }
        let before = store.tree();
        copy_rollout(&store);
        assert_eq!(
            store.tree(),
            before,
            "{strays}: the next copy writes nothing"
        );
    }
}

#[test]
fn a_view_whose_task_drops_its_last_image_drops_it_too() {
    let store = projecting_store();
    private_rollout(&store);
    let image = store.sandbox.project().join("shot.png");
    std::fs::write(&image, [3_u8; 64]).expect("the image");
    let body = store.body("As drawn: ![the retry](./shot.png)");
    let image = image.display().to_string();
    store.ok(&[
        "task",
        "create",
        "plan",
        "--project",
        "rollout",
        "--title",
        "Draw it",
        "--body-file",
        &body,
        "--repository",
        OPEN,
        "--asset",
        &image,
    ]);
    let landed = copy_rollout(&store);
    let view = view_of(&store, &landed, &landed["plan:draw-it"]);
    let illustrated: Value =
        serde_json::from_str(&store.ok(&["task", "show", &view, "--json"])).expect("the view");
    let held = illustrated["assets"][0]["path"]
        .as_str()
        .expect("its image")
        .to_owned();
    assert!(std::path::Path::new(&held).exists());
    // The task's prose no longer draws it.
    store.record(
        "plan",
        "tasks",
        "draw-it",
        &format!("title: Draw it\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
        "Described in words instead.",
    );
    copy_rollout(&store);
    let plain: Value =
        serde_json::from_str(&store.ok(&["task", "show", &view, "--json"])).expect("the view");
    assert_eq!(
        plain["items"][0]["item"]["content"],
        "Described in words instead."
    );
    assert_eq!(plain["assets"], json!([]), "{plain:#}");
    assert!(
        !std::path::Path::new(&held).exists(),
        "its image is gone: {held}"
    );
}

#[test]
fn an_update_or_a_delivery_whose_view_is_gone_says_it_reached_no_view_and_recovers() {
    let store = projecting_store();
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let guide = &landed["plan:guide"];
    let widget_view = view_of(&store, &landed, widget);
    let guide_view = view_of(&store, &landed, guide);
    for view in [&widget_view, &guide_view] {
        std::fs::remove_file(record_file(&store, "tasks", view)).expect("a view deleted in public");
    }
    // An update naming the status writes the canonical task and says the view was not reached.
    let (kind, said) = store.refused(&["task", "update", widget, "--status", "done"]);
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert!(said.contains(&widget_view), "{said}");
    assert_eq!(shown(&store, "task", widget)["status"]["category"], "done");
    // The delivery rule moves the guide, and reports that ticket failed for its view alone.
    store.ok(&[
        "task",
        "update",
        &landed["plan:producer"],
        "--delivers",
        guide,
    ]);
    let output = store.run(&[
        "task",
        "status",
        "set",
        &landed["plan:producer"],
        "in-progress",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let set: Value = serde_json::from_str(&stdout(&output)).expect("a status answer");
    let delivered = &set["delivered"][0];
    assert_eq!(delivered["outcome"], "failed", "{set:#}");
    assert_eq!(
        delivered["failure"]["kind"], "projection-not-mirrored",
        "{set:#}"
    );
    assert_ne!(shown(&store, "task", guide)["status"]["category"], "todo");
    // The next copy makes both views again, and the next status reaches them.
    copy_rollout(&store);
    let updated: Value = serde_json::from_str(&store.ok(&[
        "task",
        "update",
        widget,
        "--status",
        "in-progress",
        "--json",
    ]))
    .expect("an update answer");
    assert_eq!(updated["projection"], widget_view.as_str(), "{updated:#}");
    assert_eq!(
        shown(&store, "task", &widget_view)["status"]["category"],
        "in-progress"
    );
    assert!(
        record_file(&store, "tasks", &guide_view).exists(),
        "the guide's view is back"
    );
}

#[test]
fn a_project_holding_a_projection_map_copied_onward_never_carries_the_map() {
    let store = Store::laid(Some(registered), |sandbox| {
        json!({
            "archive": folder(sandbox, "archive", "private"),
            "vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]},
        })
    });
    std::fs::create_dir_all(store.folder("archive")).expect("the archive folder");
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let home = &landed["plan:rollout"];
    assert!(
        projection_map(&store, home).is_object(),
        "the private project records a map"
    );
    let onward = landed_ids(&store.ok(&["project", "copy", home, "--to", "archive", "--json"]));
    let copied = shown(&store, "project", &onward[home.as_str()]);
    assert!(
        copied["metadata"]
            .get("onetaskgraph.public_projections")
            .is_none(),
        "the map stays with the project it was recorded on: {copied:#}"
    );
    // And a copy onward again over what it wrote leaves the map where it was.
    copy_rollout(&store);
    assert!(projection_map(&store, home).is_object());
}

#[test]
fn a_linear_view_whose_create_answer_was_lost_is_found_again_rather_than_made_twice() {
    // Linear issues its own ids too: the issue it stored, whose answer the copy could not read,
    // is found by what it says on the next copy.
    let mut handle = None;
    let store = Store::laid(Some(registered), |sandbox| {
        let (mut config, workspace) =
            crate::fixtures::linear_workspace(sandbox, crate::fixtures::dataset());
        config["status_mapping"] = crate::fixtures::linear_default_mapping();
        handle = Some(workspace);
        sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
        json!({
            "tracker": {"plugin": "linear", "config": config, "visibility": "public"},
            "vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "tracker"}]},
        })
    });
    let workspace = handle.expect("the workspace");
    private_rollout(&store);
    let landed = copy_rollout(&store);
    assert!(
        view_of(&store, &landed, &landed["plan:widget"]).starts_with("tracker:"),
        "the views are in Linear"
    );
    store.record(
        "plan",
        "tasks",
        "tune",
        &format!("title: Tune the retry\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
        PLAIN,
    );
    let creates = |workspace: &crate::fixtures::LinearWorkspace| {
        workspace
            .served()
            .iter()
            .filter(|(document, variables)| {
                document.contains("issueCreate") && variables.to_string().contains("Tune the retry")
            })
            .count()
    };
    // Linear stores the issue and its answer comes back with no issue in it.
    workspace.bend_answers(
        |document, variables| {
            document.contains("issueCreate") && variables.to_string().contains("Tune the retry")
        },
        1,
        |_, data| *data = json!({"issueCreate": null}),
    );
    let output = store.run(&["project", "copy", "plan:rollout", "--to", "vault"]);
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(creates(&workspace), 1, "Linear was asked once");

    let again =
        landed_ids(&store.ok(&["project", "copy", "plan:rollout", "--to", "vault", "--json"]));
    assert_eq!(creates(&workspace), 1, "and no copy asked it a second time");
    let view = view_of(&store, &landed, &again["plan:tune"]);
    assert!(view.starts_with("tracker:"), "{view}");
    assert_eq!(shown(&store, "task", &view)["title"], "Tune the retry");
}

#[test]
fn links_and_html_written_as_code_reach_the_view_byte_for_byte() {
    let store = projecting_store();
    private_rollout(&store);
    let content = "Write `[the notes](../notes/x.md)` to link them, or:\n\n\
                   ```html\n<a href=\"../notes/x.md\">the notes</a>\n<script>run()</script>\n```\n";
    store.record(
        "plan",
        "tasks",
        "teach",
        &format!("title: Teach linking\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
        content,
    );
    let landed = copy_rollout(&store);
    let view = view_of(&store, &landed, &landed["plan:teach"]);
    assert_eq!(
        shown(&store, "task", &view)["content"],
        shown(&store, "task", "plan:teach")["content"],
        "code is carried as written"
    );
}

/// A check passing every write but one naming the status `cancelled`.
const CANCELLED_REFUSED: &str = "import json, sys\n\
    if 'cancelled' in sys.stdin.read():\n\
    \x20   print(json.dumps({'verdict': 'refuse', 'surface': 'metadata'}))\n\
    \x20   sys.exit(1)\n\
    print(json.dumps({'verdict': 'pass'}))\n";

#[test]
fn a_status_the_check_refuses_for_the_view_lands_canonically_and_says_it_reached_no_view() {
    let sandbox = Sandbox::new();
    let store = Store::configured(
        sandbox,
        None,
        Some(json!({
            "visibility_command": ["python3", "-c", OPENCO_PUBLIC],
            "check_command": ["python3", "-c", CANCELLED_REFUSED],
        })),
        json!({"vault": {"routes": [{"repositories": ["github.com/openco/*"], "to": "site"}]}}),
    );
    private_rollout(&store);
    let landed = copy_rollout(&store);
    let widget = &landed["plan:widget"];
    let view = view_of(&store, &landed, widget);
    let site = tree(&store.folder("site"));
    let (kind, said) = store.refused(&["task", "status", "set", widget, "cancelled"]);
    assert_eq!(kind, "projection-not-mirrored", "{said}");
    assert!(said.contains(&view), "{said}");
    assert_eq!(
        shown(&store, "task", widget)["status"]["category"],
        "cancelled"
    );
    assert_eq!(tree(&store.folder("site")), site, "the view is as it was");
}
