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
             repositories: [{public}]\ndepends_on: [producer]"
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
    // A URL, a mail link and a fragment are carried, in Markdown and in HTML alike.
    let store = projecting_store();
    private_rollout(&store);
    store.record(
        "plan",
        "tasks",
        "linked",
        &format!("title: Link the docs\nstatus: todo\nproject: rollout\nrepositories: [{OPEN}]"),
        "See [the spec](https://example.org/spec), <a href=\"https://example.org/faq\">the FAQ</a>, \
         [mail us](mailto:team@example.org) and [below](#below).",
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
