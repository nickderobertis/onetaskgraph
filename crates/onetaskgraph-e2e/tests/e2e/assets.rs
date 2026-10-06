//! Image assets on tasks and documents, driven through the binary the way a person drives them.
//!
//! Every journey here runs against folders of Markdown, which keep a record's assets in a
//! directory beside its file, and — where a journey is about a destination that serves its
//! assets at a URL, or about one that has never heard of assets — `asset_store.py` beside this
//! file: a peer over a real pipe that keeps its records in a JSON file, so one invocation's copy
//! is read back by the next, and that logs exactly what each write delivered to it.
//!
//! Every PNG a journey here stores or copies comes from `onetaskgraph_e2e_support::images::png`, and every record
//! a journey copies carries one of about 500 KB among its assets: the weight a real screenshot
//! on a plan has. Digests are recomputed here from the bytes the journey wrote, never taken
//! from the engine that stored them.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common::{Sandbox, SourceBoundary, stderr, stdout};
use crate::document_store::interpreter;
use crate::fixtures::document;
use onetaskgraph_e2e_support::images;

/// The size of the screenshot every copied record carries: within the generator's range, and
/// about 500 KB.
const SCREENSHOT: usize = 500_000;

/// A template whose content references each picture its answers name.
const PICTURES: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  shots:\n    \
    description: The pictures it shows\n    \
    type: list\n\
---\n\
# The change\n\
{% for shot in shots %}\n\
![{{ shot }}](./{{ shot }})\n\
{% endfor %}\n";

/// Two folders of Markdown — `notes` and `back` — and two asset stores over a real pipe:
/// `served`, which declares assets native and serves each at a URL, `plain`, which says
/// nothing about assets, and `flaky`, which serves them as `served` does and refuses an update
/// of an item titled `Second` after applying it.
struct Folders {
    sandbox: Sandbox,
    notes: PathBuf,
    back: PathBuf,
    inputs: PathBuf,
    served_log: PathBuf,
    plain_log: PathBuf,
    flaky_log: PathBuf,
}

impl Folders {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        let back = sandbox.subdirectory("back");
        let inputs = sandbox.subdirectory("inputs");
        let stores = sandbox.subdirectory("stores");
        let served_log = stores.join("served.log");
        let plain_log = stores.join("plain.log");
        let flaky_log = stores.join("flaky.log");
        std::fs::create_dir_all(notes.join("projects")).expect("a projects folder");
        std::fs::write(
            notes.join("projects/launch.md"),
            "---\ntitle: Launch\nstatus: todo\n---\nThe launch.\n",
        )
        .expect("a project");
        sandbox.project_document(&document(&json!({
            "notes": {"plugin": "local-md", "config": {"root": notes}},
            "back": {"plugin": "local-md", "config": {"root": back}},
            "served": store(&stores.join("served.json"), &served_log, json!({"assets": "native"})),
            "plain": store(&stores.join("plain.json"), &plain_log, json!({})),
            "flaky": store(
                &stores.join("flaky.json"),
                &flaky_log,
                json!({"assets": "native", "half_written": ["Second"]}),
            ),
        })));
        Self {
            sandbox,
            notes,
            back,
            inputs,
            served_log,
            plain_log,
            flaky_log,
        }
    }

    /// Write `bytes` as `name` under a directory of inputs of its own, and answer its path.
    fn image(&self, directory: &str, name: &str, bytes: &[u8]) -> String {
        let directory = self.inputs.join(directory);
        std::fs::create_dir_all(&directory).expect("an input directory");
        let path = directory.join(name);
        std::fs::write(&path, bytes).expect("an input image");
        spelled(&path)
    }

    /// Write `text` as a file of inputs, and answer its path.
    fn text(&self, name: &str, text: &str) -> String {
        let path = self.inputs.join(name);
        std::fs::write(&path, text).expect("an input file");
        spelled(&path)
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    fn exits(&self, arguments: &[&str], code: i32) -> Output {
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}` exited {:?}\nstdout:\n{}\nstderr:\n{}",
            arguments.join(" "),
            output.status.code(),
            stdout(&output),
            stderr(&output)
        );
        output
    }

    /// The qualified id a create printed.
    fn created(&self, arguments: &[&str]) -> String {
        stdout(&self.exits(arguments, 0)).trim().to_owned()
    }

    /// Standard error of a run refused with exit `1`.
    fn refused(&self, arguments: &[&str]) -> String {
        stderr(&self.exits(arguments, 1))
    }

    /// The `kind` of the failure document a run refused with exit `1` writes under `--json`.
    fn failure_kind(&self, arguments: &[&str]) -> String {
        let mut all = arguments.to_vec();
        all.push("--json");
        let output = self.exits(&all, 1);
        let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        failure["failure"]["kind"]
            .as_str()
            .expect("a failure kind")
            .to_owned()
    }

    /// `<verb> show <id> --json`, whole.
    fn show(&self, verb: &str, id: &str) -> Value {
        serde_json::from_str(&stdout(&self.exits(&[verb, "show", id, "--json"], 0)))
            .expect("show writes JSON")
    }

    /// What a copy reports, under `--json`.
    fn copy(&self, arguments: &[&str]) -> Value {
        let mut all = arguments.to_vec();
        all.push("--json");
        serde_json::from_str(&stdout(&self.exits(&all, 0))).expect("a copy reports JSON")
    }

    /// Every write `log` recorded, in order.
    fn logged(log: &Path) -> Vec<Value> {
        match std::fs::read_to_string(log) {
            Ok(text) => text
                .lines()
                .map(|line| serde_json::from_str(line).expect("a log line is JSON"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Every file under `root`, relative to it, sorted.
    fn files(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    found.push(
                        path.strip_prefix(root)
                            .expect("under the root")
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        found.sort();
        found
    }
}

/// The asset store over a real pipe, keeping its records at `at`, logging every write to
/// `log`, and taking `more` of its settings besides.
fn store(at: &Path, log: &Path, more: Value) -> Value {
    let mut settings = json!({"store": at, "log": log});
    for (key, value) in more.as_object().expect("settings") {
        settings[key] = value.clone();
    }
    json!({
        "plugin": "subprocess",
        "config": {
            "command": interpreter().to_string_lossy(),
            "args": [Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/e2e/asset_store.py")
                .to_string_lossy()],
            "settings": settings,
        },
    })
}

fn spelled(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").to_owned()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn body_digest(content: &str) -> String {
    format!("sha256:{}", sha256(content.as_bytes()))
}

/// The `assets` a show answered, as `(name, sha256, content_type)` in the order listed.
fn listed(shown: &Value) -> Vec<(String, String, String)> {
    shown["assets"]
        .as_array()
        .expect("a show lists assets")
        .iter()
        .map(|asset| {
            (
                asset["name"].as_str().expect("a name").to_owned(),
                asset["sha256"].as_str().expect("a digest").to_owned(),
                asset["content_type"].as_str().expect("a type").to_owned(),
            )
        })
        .collect()
}

/// The bytes at each listed asset's `path`, which must be an existing file.
fn held_bytes(shown: &Value) -> Vec<Vec<u8>> {
    shown["assets"]
        .as_array()
        .expect("a show lists assets")
        .iter()
        .map(|asset| {
            let path = asset["path"].as_str().expect("a local path");
            assert!(Path::new(path).is_absolute(), "{path} is absolute");
            std::fs::read(path).unwrap_or_else(|error| panic!("{path} holds the bytes: {error}"))
        })
        .collect()
}

fn item(shown: &Value) -> &Value {
    &shown["items"][0]["item"]
}

/// Content referencing assets, and around them every link that is not one.
fn with_decoys(references: &str) -> String {
    format!(
        "# Settings\n\n{references}\n\n\
         Not assets: ![remote](https://example.invalid/remote.png) ![nested](./img/nested.png) \
         ![up](../up.png) ![bare](bare.png) [notes](./notes.txt) [plain](./plain.png)\n"
    )
}

#[test]
fn a_document_and_a_task_store_their_assets_and_show_lists_each_in_first_reference_order() {
    let folders = Folders::new();
    let shot = images::png(1, SCREENSHOT);
    let wide = images::png(2, 60_000);
    let photo = images::jpeg(3);
    let scan = images::jpeg(4);
    let anim = images::gif(5);
    let pic = images::webp(6);
    let content = with_decoys(
        "![scan](./Scan.JpEg) ![shot](./shot.png) ![wide](./Wide.PNG) ![photo](./photo.jpg) \
         ![anim](./anim.gif) ![pic](./pic.webp) ![shot again](./shot.png)",
    );
    let body = folders.text("design.md", &content);
    // Given in an order of their own, which is not the order the content references them in.
    let given = [
        folders.image("a", "shot.png", &shot),
        folders.image("a", "pic.webp", &pic),
        folders.image("a", "anim.gif", &anim),
        folders.image("a", "photo.jpg", &photo),
        folders.image("a", "Wide.PNG", &wide),
        folders.image("a", "Scan.JpEg", &scan),
    ];
    let mut arguments = vec![
        "document",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Design",
        "--body-file",
        &body,
    ];
    for path in &given {
        arguments.extend_from_slice(&["--asset", path]);
    }
    let id = folders.created(&arguments);

    let shown = folders.show("document", &id);
    assert_eq!(
        item(&shown)["content"],
        json!(content),
        "stored exactly as written"
    );
    assert_eq!(
        listed(&shown),
        vec![
            (
                "Scan.JpEg".to_owned(),
                sha256(&scan),
                "image/jpeg".to_owned()
            ),
            ("shot.png".to_owned(), sha256(&shot), "image/png".to_owned()),
            ("Wide.PNG".to_owned(), sha256(&wide), "image/png".to_owned()),
            (
                "photo.jpg".to_owned(),
                sha256(&photo),
                "image/jpeg".to_owned()
            ),
            ("anim.gif".to_owned(), sha256(&anim), "image/gif".to_owned()),
            ("pic.webp".to_owned(), sha256(&pic), "image/webp".to_owned()),
        ]
    );
    assert_eq!(
        held_bytes(&shown),
        vec![scan, shot.clone(), wide, photo, anim, pic]
    );

    let before = images::png(7, 120_000);
    let task_body = folders.text("task.md", "![before](./before.png)\n");
    let before_path = folders.image("b", "before.png", &before);
    let task = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Fix the page",
        "--body-file",
        &task_body,
        "--asset",
        &before_path,
    ]);
    let shown = folders.show("task", &task);
    assert_eq!(
        listed(&shown),
        vec![(
            "before.png".to_owned(),
            sha256(&before),
            "image/png".to_owned()
        )]
    );
    assert_eq!(held_bytes(&shown), vec![before]);
    // The human rendering names it after the body.
    let text = stdout(&folders.exits(&["task", "show", &task], 0));
    assert!(
        text.contains("assets: 1") && text.contains("before.png"),
        "{text}"
    );

    // A record holding none lists none — an empty list, not an absent one.
    let plain_body = folders.text("plain.md", &with_decoys("no pictures of its own"));
    for verb in ["task", "document"] {
        let plain = folders.created(&[
            verb,
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            "Plain",
            "--body-file",
            &plain_body,
        ]);
        let shown = folders.show(verb, &plain);
        assert_eq!(shown["assets"], json!([]), "{verb}");
        assert_eq!(
            item(&shown)["content"],
            json!(with_decoys("no pictures of its own"))
        );
    }
}

#[test]
fn every_refusal_a_create_owes_names_the_file_or_reference_and_writes_nothing() {
    let folders = Folders::new();
    let png = images::png(10, 50_000);
    let body = folders.text("one.md", "![a](./a.png)\n");
    let a = folders.image("first", "a.png", &png);
    let again = folders.image("second", "a.png", &images::png(11, 50_000));
    let bitmap = folders.image("first", "a.bmp", b"BM not an accepted image");
    let other = folders.image("first", "other.png", &png);
    for (verb, create) in [
        (
            "task",
            vec![
                "task",
                "create",
                "notes",
                "--project",
                "launch",
                "--title",
                "T",
            ],
        ),
        (
            "document",
            vec![
                "document",
                "create",
                "notes",
                "--project",
                "launch",
                "--title",
                "D",
            ],
        ),
    ] {
        let with = |extra: &[&str]| {
            let mut all = create.clone();
            all.extend_from_slice(&["--body-file", &body]);
            all.extend_from_slice(extra);
            folders.refused(&all)
        };
        let said = with(&["--asset", &a, "--asset", &bitmap]);
        assert!(
            said.contains("a.bmp") && said.contains(".png"),
            "{verb}: {said}"
        );
        let said = with(&["--asset", &a, "--asset", &again]);
        assert!(said.contains(&a) && said.contains(&again), "{verb}: {said}");
        let said = with(&[]);
        assert!(
            said.contains("./a.png") && said.contains("--asset"),
            "{verb}: {said}"
        );
        let kind = |extra: &[&str]| {
            let mut all = create.clone();
            all.extend_from_slice(&["--body-file", &body]);
            all.extend_from_slice(extra);
            folders.failure_kind(&all)
        };
        assert_eq!(kind(&[]), "asset-not-given");
        assert_eq!(
            kind(&["--asset", &a, "--asset", &again]),
            "asset-given-twice"
        );
        assert_eq!(
            kind(&["--asset", &a, "--asset", &other]),
            "asset-not-referenced"
        );
        assert_eq!(kind(&["--asset", &a, "--asset", &bitmap]), "asset-name");
        let said = with(&["--asset", &a, "--asset", &other]);
        assert!(
            said.contains("other.png") && said.contains("does not reference"),
            "{verb}: {said}"
        );
        // A path that names no file it can read — here a directory called `a.png`.
        let unreadable = spelled(&folders.inputs.join("unreadable").join("a.png"));
        std::fs::create_dir_all(&unreadable).expect("a directory in the file's place");
        let said = with(&["--asset", &unreadable]);
        assert!(
            said.contains(&unreadable) && said.contains("could not be read"),
            "{verb}: {said}"
        );
    }
    assert_eq!(
        Folders::files(&folders.notes),
        vec!["projects/launch.md".to_owned()],
        "nothing was written"
    );
}

#[test]
fn a_render_keeps_stored_assets_replaces_one_by_name_and_drops_one_no_longer_referenced() {
    let folders = Folders::new();
    let template = folders.text("pictures.md", PICTURES);
    let three = folders.text("three.yaml", "shots: [keep.png, swap.png, drop.png]\n");
    let two = folders.text("two.yaml", "shots: [keep.png, swap.png]\n");
    let keep = images::png(20, 70_000);
    let swap = images::png(21, 70_000);
    let drop = images::gif(22);
    let swapped = images::png(23, 80_000);
    let (keep_path, swap_path, drop_path) = (
        folders.image("old", "keep.png", &keep),
        folders.image("old", "swap.png", &swap),
        folders.image("old", "drop.png", &drop),
    );
    let swapped_path = folders.image("new", "swap.png", &swapped);
    for verb in ["task", "document"] {
        let id = folders.created(&[
            verb,
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            "Pictures",
            "--template",
            &template,
            "--answers",
            &three,
            "--no-interactive",
            "--asset",
            &keep_path,
            "--asset",
            &swap_path,
            "--asset",
            &drop_path,
        ]);
        let shown = folders.show(verb, &id);
        let directory = Path::new(shown["assets"][0]["path"].as_str().expect("a path"))
            .parent()
            .expect("the asset directory")
            .to_path_buf();
        assert_eq!(listed(&shown).len(), 3, "{verb}");

        folders.exits(
            &[
                verb,
                "render",
                &id,
                "--answers",
                &two,
                "--no-interactive",
                "--asset",
                &swapped_path,
            ],
            0,
        );
        let shown = folders.show(verb, &id);
        assert_eq!(
            listed(&shown),
            vec![
                ("keep.png".to_owned(), sha256(&keep), "image/png".to_owned()),
                (
                    "swap.png".to_owned(),
                    sha256(&swapped),
                    "image/png".to_owned()
                ),
            ],
            "{verb}"
        );
        assert_eq!(held_bytes(&shown), vec![keep.clone(), swapped.clone()]);
        // The dropped asset left no file behind in the record's own directory.
        let mut left: Vec<String> = std::fs::read_dir(&directory)
            .expect("the asset directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        left.sort();
        assert_eq!(left, ["keep.png", "swap.png"], "{verb}");
        // The rendering still vouches for what the record holds.
        let entry = &item(&shown)["metadata"]["onetaskgraph.template"];
        assert_eq!(
            entry["body_digest"],
            json!(body_digest(
                item(&shown)["content"].as_str().expect("content")
            ))
        );
    }
}

#[test]
fn every_refusal_a_render_owes_names_the_asset_and_leaves_the_record_as_it_was() {
    let folders = Folders::new();
    let template = folders.text("pictures.md", PICTURES);
    let one = folders.text("one.yaml", "shots: [keep.png]\n");
    let two = folders.text("two.yaml", "shots: [keep.png, new.png]\n");
    let keep_path = folders.image("render", "keep.png", &images::png(130, 55_000));
    let stray = folders.image("render", "stray.png", &images::png(131, 55_000));
    let again = folders.image("again", "keep.png", &images::png(132, 55_000));
    let held = |folders: &Folders| {
        Folders::files(&folders.notes)
            .into_iter()
            .map(|file| {
                (
                    std::fs::read(folders.notes.join(&file)).expect("a file"),
                    file,
                )
            })
            .collect::<Vec<_>>()
    };
    for verb in ["task", "document"] {
        let id = folders.created(&[
            verb,
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            "Refusing",
            "--template",
            &template,
            "--answers",
            &one,
            "--no-interactive",
            "--asset",
            &keep_path,
        ]);
        let before = held(&folders);
        for (extra, kind, named) in [
            (
                vec!["--asset", stray.as_str()],
                "asset-not-referenced",
                "stray.png",
            ),
            (
                vec!["--answers", two.as_str()],
                "asset-not-given",
                "new.png",
            ),
            (
                vec!["--asset", keep_path.as_str(), "--asset", again.as_str()],
                "asset-given-twice",
                "keep.png",
            ),
        ] {
            let mut arguments = vec![verb, "render", id.as_str(), "--no-interactive"];
            arguments.extend_from_slice(&extra);
            let said = folders.refused(&arguments);
            assert!(said.contains(named), "{verb} {extra:?}: {said}");
            assert_eq!(folders.failure_kind(&arguments), kind, "{verb} {extra:?}");
        }
        assert_eq!(held(&folders), before, "{verb}: nothing was written");
    }
}

#[test]
fn two_records_keep_their_own_bytes_under_one_asset_name() {
    let folders = Folders::new();
    let body = folders.text("shot.md", "![shot](./shot.png)\n");
    let first = images::png(30, 90_000);
    let second = images::png(31, 90_000);
    let one = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "One",
        "--body-file",
        &body,
        "--asset",
        &folders.image("one", "shot.png", &first),
    ]);
    let two = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Two",
        "--body-file",
        &body,
        "--asset",
        &folders.image("two", "shot.png", &second),
    ]);
    let (shown_one, shown_two) = (folders.show("task", &one), folders.show("task", &two));
    assert_eq!(listed(&shown_one)[0].1, sha256(&first));
    assert_eq!(listed(&shown_two)[0].1, sha256(&second));
    assert_ne!(
        shown_one["assets"][0]["path"],
        shown_two["assets"][0]["path"]
    );
    assert_eq!(held_bytes(&shown_two), vec![second.clone()]);

    // Replacing one record's asset leaves the other's byte for byte.
    let template = folders.text("pictures.md", PICTURES);
    let answers = folders.text("one.yaml", "shots: [shot.png]\n");
    let third = images::png(32, 90_000);
    let rendered = folders.created(&[
        "document",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Rendered",
        "--template",
        &template,
        "--answers",
        &answers,
        "--no-interactive",
        "--asset",
        &folders.image("three", "shot.png", &first),
    ]);
    folders.exits(
        &[
            "document",
            "render",
            &rendered,
            "--no-interactive",
            "--asset",
            &folders.image("four", "shot.png", &third),
        ],
        0,
    );
    assert_eq!(
        held_bytes(&folders.show("document", &rendered)),
        vec![third]
    );
    assert_eq!(held_bytes(&folders.show("task", &one)), vec![first.clone()]);
    assert_eq!(held_bytes(&folders.show("task", &two)), vec![second]);
}

#[test]
fn a_document_replaced_by_id_holds_exactly_the_replacing_calls_assets() {
    let folders = Folders::new();
    let both = folders.text("both.md", "![x](./x.png) ![y](./y.gif)\n");
    let only = folders.text("only.md", "![x](./x.png)\n");
    let none = folders.text("none.md", "no pictures\n");
    let x = images::png(40, 75_000);
    let new_x = images::png(41, 75_000);
    let id = folders.created(&[
        "document",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Replaced",
        "--id",
        "replaced",
        "--body-file",
        &both,
        "--asset",
        &folders.image("old", "x.png", &x),
        "--asset",
        &folders.image("old", "y.gif", &images::gif(42)),
    ]);
    let directory = folders.notes.join("documents/replaced.assets");
    assert!(directory.join("y.gif").is_file());

    folders.exits(
        &[
            "document",
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            "Replaced",
            "--id",
            "replaced",
            "--body-file",
            &only,
            "--asset",
            &folders.image("new", "x.png", &new_x),
        ],
        0,
    );
    let shown = folders.show("document", &id);
    assert_eq!(
        listed(&shown),
        vec![("x.png".to_owned(), sha256(&new_x), "image/png".to_owned())]
    );
    assert!(
        !directory.join("y.gif").exists(),
        "the dropped asset left no file"
    );

    folders.exits(
        &[
            "document",
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            "Replaced",
            "--id",
            "replaced",
            "--body-file",
            &none,
        ],
        0,
    );
    assert_eq!(folders.show("document", &id)["assets"], json!([]));
    assert!(
        !directory.exists(),
        "no directory is left for a record with no assets"
    );
}

#[test]
fn a_document_replaced_by_id_on_a_plugin_serving_assets_takes_its_uploads_away() {
    // A destination over the protocol lists no assets, so what it serves is known only from its
    // own record of uploads: a replacement naming none still has to take those away.
    let folders = Folders::new();
    let with = folders.text("with.md", "![x](./x.png)\n");
    let none = folders.text("none.md", "no pictures\n");
    let create = |body: &str, assets: &[&str]| {
        let mut arguments = vec![
            "document",
            "create",
            "served",
            "--project",
            "launch",
            "--title",
            "Replaced",
            "--id",
            "replaced",
            "--body-file",
            body,
        ];
        for asset in assets {
            arguments.extend(["--asset", asset]);
        }
        folders.created(&arguments)
    };
    let x = folders.image("x", "x.png", &images::png(43, 75_000));
    let id = create(&with, &[&x]);
    let shown = folders.show("document", &id);
    assert_eq!(recorded(&shown).len(), 1, "{shown}");

    let writes_before = Folders::logged(&folders.served_log).len();
    assert_eq!(create(&none, &[]), id);
    let writes = Folders::logged(&folders.served_log);
    let replacement = &writes[writes_before];
    assert_eq!(replacement["target"], json!("replaced"));
    assert_eq!(
        replacement["received"],
        json!([]),
        "the replacement was a write of no assets at all"
    );
    let after = folders.show("document", &id);
    assert!(
        item(&after)["metadata"]
            .get("onetaskgraph.assets")
            .is_none(),
        "{after}"
    );
    assert_eq!(item(&after)["content"], json!("no pictures\n"));
}

/// A rendered task and a rendered document in `notes`, each holding a 500 KB screenshot and a
/// second picture, filed under the project `launch`.
fn rendered_pair(folders: &Folders, seed: u64) -> (String, String, Vec<u8>, Vec<u8>) {
    let template = folders.text("pictures.md", PICTURES);
    let answers = folders.text("pair.yaml", "shots: [screen.png, detail.webp]\n");
    let screen = images::png(seed, SCREENSHOT);
    let detail = images::webp(seed + 1);
    let screen_path = folders.image(&format!("pair-{seed}"), "screen.png", &screen);
    let detail_path = folders.image(&format!("pair-{seed}"), "detail.webp", &detail);
    let task = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Show the change",
        "--template",
        &template,
        "--answers",
        &answers,
        "--no-interactive",
        "--asset",
        &screen_path,
        "--asset",
        &detail_path,
    ]);
    let document = folders.created(&[
        "document",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "The change",
        "--template",
        &template,
        "--answers",
        &answers,
        "--no-interactive",
        "--asset",
        &screen_path,
        "--asset",
        &detail_path,
    ]);
    (task, document, screen, detail)
}

/// The destination id a copy report gave its first item.
fn landed(report: &Value) -> String {
    report["items"][0]["destination"]
        .as_str()
        .expect("a destination id")
        .to_owned()
}

/// Whether the copied record `copied` holds the same assets, byte for byte, and the same
/// content as `original`, with provenance that still verifies.
fn carried_whole(original: &Value, copied: &Value) {
    assert_eq!(listed(copied), listed(original));
    assert_eq!(held_bytes(copied), held_bytes(original));
    assert_ne!(copied["assets"][0]["path"], original["assets"][0]["path"]);
    assert_eq!(item(copied)["content"], item(original)["content"]);
    let entry = &item(copied)["metadata"]["onetaskgraph.template"];
    assert_eq!(
        entry,
        &item(original)["metadata"]["onetaskgraph.template"],
        "the provenance is carried as it was"
    );
    assert_eq!(
        entry["body_digest"],
        json!(body_digest(
            item(copied)["content"].as_str().expect("content")
        ))
    );
}

#[test]
fn a_project_copy_carries_its_tasks_assets_and_its_document_follows_with_its_own() {
    let folders = Folders::new();
    let (task, document, ..) = rendered_pair(&folders, 50);
    let report = folders.copy(&["project", "copy", "notes:launch", "--to", "back"]);
    let landed_as = |source: &str| {
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|outcome| outcome["source"] == json!(source))
            .and_then(|outcome| outcome["destination"].as_str())
            .unwrap_or_else(|| panic!("{source} was copied with its project"))
            .to_owned()
    };
    let copied_project = landed_as("notes:launch");
    let copied_task = landed_as(&task);
    let task_copy = folders.show("task", &copied_task);
    carried_whole(&folders.show("task", &task), &task_copy);
    // A project copy carries its tasks; its document follows by `document copy`, into the
    // project the copy just made.
    let report = folders.copy(&["document", "copy", &document, "--to", "back"]);
    let document_copy = folders.show("document", &landed(&report));
    carried_whole(&folders.show("document", &document), &document_copy);

    // Both copies sit in the copied project, and each one's references name its own uploaded
    // copies: the files beside its own record in the destination folder.
    let project_native = copied_project.split_once(':').expect("qualified").1;
    for (copy, folder) in [(&task_copy, "tasks"), (&document_copy, "documents")] {
        assert_eq!(item(copy)["project"], json!(project_native), "{folder}");
        let record = item(copy)["location"]["path"]
            .as_str()
            .expect("a local record");
        assert!(Path::new(record).starts_with(&folders.back), "{record}");
        let own = Path::new(record).with_extension("assets");
        let content = item(copy)["content"].as_str().expect("content");
        for asset in copy["assets"].as_array().expect("assets") {
            let name = asset["name"].as_str().expect("a name");
            assert!(
                content.contains(&format!("](./{name})")),
                "{folder} references {name}"
            );
            assert_eq!(
                Path::new(asset["path"].as_str().expect("a path")),
                own.join(name),
                "{folder}: {name} is the copied record's own"
            );
        }
    }
}

#[test]
fn a_task_copy_and_a_document_copy_each_carry_their_assets_byte_for_byte() {
    let folders = Folders::new();
    let (task, document, ..) = rendered_pair(&folders, 60);
    for (verb, id) in [("task", &task), ("document", &document)] {
        let report = folders.copy(&[verb, "copy", id, "--to", "back"]);
        carried_whole(
            &folders.show(verb, id),
            &folders.show(verb, &landed(&report)),
        );
        // A second copy of what has not changed changes nothing.
        let again = folders.copy(&[verb, "copy", id, "--to", "back"]);
        assert_eq!(again["items"][0]["action"], json!("unchanged"), "{verb}");
    }
}

#[test]
fn a_copy_whose_assets_cannot_be_installed_leaves_the_destination_as_it_was() {
    let folders = Folders::new();
    let (task, ..) = rendered_pair(&folders, 180);
    let report = folders.copy(&["task", "copy", &task, "--to", "back"]);
    let copied = landed(&report);
    let native = copied.split_once(':').expect("qualified").1;
    let file = folders.back.join(format!("tasks/{native}.md"));
    let before = std::fs::read(&file).expect("the copied record");
    let held = folders.back.join(format!("tasks/{native}.assets"));
    let screen = std::fs::read(held.join("screen.png")).expect("the copied asset");

    // The source gains a picture whose place at the destination is taken by a directory.
    let extra = folders.image("extra", "extra.gif", &images::gif(181));
    let both = folders.text("both.yaml", "shots: [screen.png, detail.webp, extra.gif]\n");
    folders.exits(
        &[
            "task",
            "render",
            &task,
            "--no-interactive",
            "--answers",
            &both,
            "--asset",
            &extra,
        ],
        0,
    );
    std::fs::create_dir_all(held.join("extra.gif").join("inside")).expect("a blocking directory");

    let said = folders.refused(&["task", "copy", &task, "--to", "back"]);
    assert!(said.contains("extra.gif"), "{said}");
    assert_eq!(
        std::fs::read(&file).expect("the record"),
        before,
        "the record as it was"
    );
    assert_eq!(
        std::fs::read(held.join("screen.png")).expect("the asset"),
        screen
    );
}

#[test]
fn a_copy_into_a_source_that_stores_no_assets_is_refused_naming_it_and_writes_nothing() {
    let folders = Folders::new();
    let (task, document, ..) = rendered_pair(&folders, 70);
    for (arguments, record) in [
        (vec!["task", "copy", &task, "--to", "plain"], task.clone()),
        (
            vec!["document", "copy", &document, "--to", "plain"],
            document.clone(),
        ),
        (
            vec!["project", "copy", "notes:launch", "--to", "plain"],
            task.clone(),
        ),
    ] {
        let said = folders.refused(&arguments);
        for named in ["plain", record.as_str(), "screen.png"] {
            assert!(said.contains(named), "{arguments:?} names {named}: {said}");
        }
        assert!(
            !Folders::logged(&folders.plain_log).iter().any(|write| {
                write["method"] == json!("write_task") || write["method"] == json!("write_document")
            }),
            "{arguments:?}: nothing was written for the record"
        );
        assert_eq!(folders.failure_kind(&arguments), "assets-unsupported");
    }
    // A create carrying an asset into it is refused on the same terms, before it is written.
    let body = folders.text("create.md", "![screen](./screen.png)\n");
    let screen = folders.image("create", "screen.png", &images::png(71, 50_000));
    for verb in ["task", "document"] {
        let arguments = [
            verb,
            "create",
            "plain",
            "--project",
            "launch",
            "--title",
            "Refused",
            "--body-file",
            &body,
            "--asset",
            &screen,
        ];
        let said = folders.refused(&arguments);
        assert!(
            said.contains("plain") && said.contains("screen.png") && said.contains("Refused"),
            "{verb}: {said}"
        );
        assert_eq!(folders.failure_kind(&arguments), "assets-unsupported");
    }
    assert!(Folders::logged(&folders.plain_log).iter().all(|write| {
        write["method"] != json!("write_task") && write["method"] != json!("write_document")
    }));
}

#[test]
fn a_copy_of_a_record_referencing_an_asset_it_does_not_hold_is_refused_naming_both() {
    let folders = Folders::new();
    std::fs::create_dir_all(folders.notes.join("tasks")).expect("a tasks folder");
    std::fs::write(
        folders.notes.join("tasks/ghost.md"),
        "---\ntitle: Ghost\nstatus: todo\n---\n![missing](./ghost.png)\n",
    )
    .expect("a task referencing an asset it does not hold");
    let said = folders.refused(&["task", "copy", "notes:ghost", "--to", "back"]);
    assert_eq!(
        folders.failure_kind(&["task", "copy", "notes:ghost", "--to", "back"]),
        "asset-not-held"
    );
    assert!(
        said.contains("notes:ghost") && said.contains("ghost.png"),
        "{said}"
    );
    assert!(!folders.back.join("tasks").exists(), "nothing was written");
}

#[test]
fn a_copy_of_a_record_with_no_asset_reference_sends_no_asset_member_to_any_plugin() {
    let folders = Folders::new();
    let body = folders.text("plain.md", &with_decoys("no pictures of its own"));
    let task = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Plain",
        "--body-file",
        &body,
    ]);
    let document = folders.created(&[
        "document",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Plain",
        "--body-file",
        &body,
    ]);
    for to in ["plain", "served"] {
        for (verb, id) in [("task", &task), ("document", &document)] {
            let report = folders.copy(&[verb, "copy", id, "--to", to]);
            let shown = folders.show(verb, &landed(&report));
            assert_eq!(
                item(&shown)["content"],
                json!(with_decoys("no pictures of its own"))
            );
        }
    }
    for log in [&folders.plain_log, &folders.served_log] {
        let writes = Folders::logged(log);
        assert_eq!(writes.len(), 2);
        for write in writes {
            assert_eq!(write["members"], json!(["write"]), "{write}");
        }
    }
}

/// A template whose content holds image syntax that is no reference — escaped, in code spans,
/// in fenced code blocks of both fences and in an indented code block — naming both an asset the
/// record holds and one it does not, and after it a real reference to each picture its answers
/// name.
const CODE_PICTURES: &str = r#"---
onetaskgraph_template: 1
variables:
  shots:
    description: The pictures it shows
    type: list
---
# The change

Escaped: \![escaped](./screen.png) and \![gone](./ghost.png)

Spans: `![span](./screen.png)` and ``![double](./ghost.png)``

```
![fenced](./screen.png)
![fenced too](./ghost.png)
```

~~~md
![tilde](./screen.png)
~~~

    ![indented](./screen.png)
    ![indented too](./ghost.png)
{% for shot in shots %}
![{{ shot }}](./{{ shot }})
{% endfor %}
"#;

/// Each kind of image syntax [`CODE_PICTURES`] holds that is no reference, as it reads in the
/// rendered content.
const NOT_REFERENCES: [&str; 9] = [
    r"\![escaped](./screen.png)",
    r"\![gone](./ghost.png)",
    "`![span](./screen.png)`",
    "``![double](./ghost.png)``",
    "```\n![fenced](./screen.png)\n![fenced too](./ghost.png)\n```",
    "~~~md\n![tilde](./screen.png)\n~~~",
    "    ![indented](./screen.png)\n    ![indented too](./ghost.png)\n",
    "![tilde](./screen.png)",
    "![fenced](./screen.png)",
];

/// A task and a document in `notes` rendered from [`CODE_PICTURES`] with `shots`, each given
/// `assets` as `--asset`.
fn code_pair(folders: &Folders, shots: &str, assets: &[String]) -> (String, String) {
    let template = folders.text("code-pictures.md", CODE_PICTURES);
    let answers = folders.text("code-shots.yaml", &format!("shots: [{shots}]\n"));
    let create = |verb: &str| {
        let mut arguments = vec![
            verb,
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            "Code",
            "--template",
            &template,
            "--answers",
            &answers,
            "--no-interactive",
        ];
        for asset in assets {
            arguments.extend(["--asset", asset.as_str()]);
        }
        folders.created(&arguments)
    };
    (create("task"), create("document"))
}

#[test]
fn image_syntax_escaped_or_in_code_is_no_reference_and_a_copy_leaves_it_byte_for_byte() {
    let folders = Folders::new();
    let screen = images::png(170, SCREENSHOT);
    let screen_path = folders.image("code", "screen.png", &screen);
    let ghost_path = folders.image("code", "ghost.png", &images::png(171, 50_000));

    // `ghost.png` is named only by syntax that is no reference: given as an asset, it is one the
    // content does not reference, and refused by name with nothing written.
    let template = folders.text("code-pictures.md", CODE_PICTURES);
    let answers = folders.text("code-shots.yaml", "shots: [screen.png]\n");
    for verb in ["task", "document"] {
        let said = folders.refused(&[
            verb,
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            "Code",
            "--template",
            &template,
            "--answers",
            &answers,
            "--no-interactive",
            "--asset",
            &screen_path,
            "--asset",
            &ghost_path,
        ]);
        assert!(said.contains("ghost.png"), "{verb}: {said}");
        assert!(!folders.notes.join(format!("{verb}s")).exists(), "{verb}");
    }

    let (task, document) = code_pair(&folders, "screen.png", &[screen_path]);
    for (verb, id) in [("task", &task), ("document", &document)] {
        let source = folders.show(verb, id);
        let content = item(&source)["content"]
            .as_str()
            .expect("content")
            .to_owned();
        for kind in NOT_REFERENCES {
            assert!(content.contains(kind), "{verb} holds {kind:?}: {content}");
        }
        // The one reference is the real one, and its asset is the only one held.
        assert_eq!(
            listed(&source),
            vec![(
                "screen.png".to_owned(),
                sha256(&screen),
                "image/png".to_owned()
            )]
        );
        assert_eq!(held_bytes(&source), vec![screen.clone()]);

        // Into a folder of Markdown: the content, every non-reference included, is as it was.
        let report = folders.copy(&[verb, "copy", id, "--to", "back"]);
        carried_whole(&source, &folders.show(verb, &landed(&report)));

        // Into a plugin serving assets: only the real reference is pointed at its URL.
        let writes_before = Folders::logged(&folders.served_log).len();
        let report = folders.copy(&[verb, "copy", id, "--to", "served"]);
        let write = &Folders::logged(&folders.served_log)[writes_before];
        assert_eq!(
            write["received"],
            json!([{"name": "screen.png", "content_type": "image/png",
                    "decoded_sha256": sha256(&screen)}]),
            "{verb}: the real reference's asset alone reached the plugin"
        );
        let copied = folders.show(verb, &landed(&report));
        let uploads = recorded(&copied);
        assert_eq!(uploads.len(), 1, "{uploads:?}");
        let url = &uploads[0].2;
        assert_eq!(
            item(&copied)["content"],
            json!(content.replace(
                "![screen.png](./screen.png)",
                &format!("![screen.png]({url})")
            )),
            "{verb}: every non-reference is byte for byte as it was"
        );
        assert_eq!(
            item(&copied)["metadata"]["onetaskgraph.template"]["body_digest"],
            json!(body_digest(
                item(&copied)["content"].as_str().expect("content")
            ))
        );
    }
}

#[test]
fn a_record_whose_only_image_syntax_is_escaped_or_in_code_is_a_record_without_assets() {
    let folders = Folders::new();
    // Created with no `--asset` at all: nothing it names is a reference it would not hold.
    let (task, document) = code_pair(&folders, "", &[]);
    for (verb, id) in [("task", &task), ("document", &document)] {
        let source = folders.show(verb, id);
        assert_eq!(source["assets"], json!([]), "{verb}");
        let content = item(&source)["content"]
            .as_str()
            .expect("content")
            .to_owned();
        assert_eq!(
            item(&source)["metadata"]["onetaskgraph.template"]["body_digest"],
            json!(body_digest(&content))
        );
        // A render changes nothing, and stores nothing.
        let rendered = stdout(&folders.exits(&[verb, "render", id, "--no-interactive"], 0));
        assert_eq!(folders.show(verb, id), source, "{verb}: {rendered}");
        let own = Path::new(item(&source)["location"]["path"].as_str().expect("a path"))
            .with_extension("assets");
        assert!(!own.exists(), "{verb}: no asset storage");
        // It copies, unchanged, into a plugin that has never heard of assets and into one that
        // serves them, and neither is sent an asset member.
        for to in ["plain", "served", "back"] {
            let report = folders.copy(&[verb, "copy", id, "--to", to]);
            let copied = folders.show(verb, &landed(&report));
            assert_eq!(item(&copied)["content"], json!(content), "{verb} into {to}");
            assert_eq!(copied["assets"], json!([]), "{verb} into {to}");
            assert_eq!(
                item(&copied)["metadata"]["onetaskgraph.template"],
                item(&source)["metadata"]["onetaskgraph.template"],
                "{verb} into {to}: its provenance is carried as it was"
            );
            assert!(
                item(&copied)["metadata"]
                    .get("onetaskgraph.assets")
                    .is_none()
            );
        }
    }
    for log in [&folders.plain_log, &folders.served_log] {
        let writes = Folders::logged(log);
        assert_eq!(writes.len(), 2);
        for write in writes {
            assert_eq!(write["members"], json!(["write"]), "{write}");
        }
    }
}

/// Content whose list item opens a block quote holding a fence, and after it a real reference
/// to the very asset the fenced image names.
const QUOTED_FENCE: &str = "# Nested\n\n- > ~~~\n  > ![example](./before.png)\n  > ~~~\n\n\
                            ![real](./before.png)\n";

#[test]
fn a_fence_in_a_quote_in_a_list_item_stays_code_through_a_copy_into_a_plugin_serving_assets() {
    let folders = Folders::new();
    let before = images::png(180, SCREENSHOT);
    let before_path = folders.image("quoted", "before.png", &before);
    let body = folders.text("quoted.md", QUOTED_FENCE);
    for verb in ["task", "document"] {
        let id = folders.created(&[
            verb,
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            "Quoted",
            "--body-file",
            &body,
            "--asset",
            &before_path,
        ]);
        let source = folders.show(verb, &id);
        assert_eq!(item(&source)["content"], json!(QUOTED_FENCE), "{verb}");
        assert_eq!(
            listed(&source),
            vec![(
                "before.png".to_owned(),
                sha256(&before),
                "image/png".to_owned()
            )]
        );

        let writes_before = Folders::logged(&folders.served_log).len();
        let report = folders.copy(&[verb, "copy", &id, "--to", "served"]);
        let write = &Folders::logged(&folders.served_log)[writes_before];
        assert_eq!(
            write["received"],
            json!([{"name": "before.png", "content_type": "image/png",
                    "decoded_sha256": sha256(&before)}]),
            "{verb}"
        );
        let copied = folders.show(verb, &landed(&report));
        let uploads = recorded(&copied);
        assert_eq!(uploads.len(), 1, "{uploads:?}");
        let url = &uploads[0].2;
        let expected = QUOTED_FENCE.replace("![real](./before.png)", &format!("![real]({url})"));
        assert!(
            expected.contains("  > ![example](./before.png)\n"),
            "the fenced image is left as written"
        );
        assert_eq!(item(&copied)["content"], json!(expected), "{verb}");
        assert_eq!(write["answered"]["content"], json!(expected), "{verb}");
    }
}

/// The destination's `onetaskgraph.assets`, by name, as `(sha256, url)`.
fn recorded(shown: &Value) -> Vec<(String, String, String)> {
    let mut recorded: Vec<(String, String, String)> =
        item(shown)["metadata"]["onetaskgraph.assets"]
            .as_object()
            .expect("the destination records what it uploaded")
            .iter()
            .map(|(name, upload)| {
                (
                    name.clone(),
                    upload["sha256"].as_str().expect("a digest").to_owned(),
                    upload["url"].as_str().expect("a url").to_owned(),
                )
            })
            .collect();
    recorded.sort();
    recorded
}

#[test]
fn a_copy_into_a_plugin_serving_assets_rewrites_references_and_reuploads_only_changed_bytes() {
    let folders = Folders::new();
    let (task, document, screen, detail) = rendered_pair(&folders, 80);
    let changed = images::png(90, SCREENSHOT);
    let changed_path = folders.image("changed", "screen.png", &changed);
    for (verb, id) in [("task", &task), ("document", &document)] {
        let source = folders.show(verb, id);
        let writes_before = Folders::logged(&folders.served_log).len();

        // The first copy creates, and delivers every asset whole.
        let report = folders.copy(&[verb, "copy", id, "--to", "served"]);
        let copied = landed(&report);
        let writes = Folders::logged(&folders.served_log);
        let create = &writes[writes_before];
        assert_eq!(create["method"], json!(format!("write_{verb}")));
        assert_eq!(create["target"], Value::Null);
        assert_eq!(create["recorded_assets"], Value::Null);
        assert_eq!(
            create["received"],
            json!([
                {"name": "screen.png", "content_type": "image/png",
                 "decoded_sha256": sha256(&screen)},
                {"name": "detail.webp", "content_type": "image/webp",
                 "decoded_sha256": sha256(&detail)},
            ])
        );
        let shown = folders.show(verb, &copied);
        let first = recorded(&shown);
        assert_eq!(
            first
                .iter()
                .map(|(name, sha, _)| (name.as_str(), sha.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("detail.webp", sha256(&detail)),
                ("screen.png", sha256(&screen))
            ]
        );
        let content = item(&shown)["content"]
            .as_str()
            .expect("content")
            .to_owned();
        assert!(!content.contains("./screen.png") && !content.contains("./detail.webp"));
        for (_, _, url) in &first {
            assert!(content.contains(url.as_str()), "{content} references {url}");
        }
        assert_eq!(
            content, create["answered"]["content"],
            "what the plugin answered"
        );
        assert_eq!(
            json!(copied.split_once(':').expect("qualified").1),
            create["answered"]["id"]
        );
        // The rendering vouches for the rewritten content, and nothing else of it moved.
        let entry = &item(&shown)["metadata"]["onetaskgraph.template"];
        let original = &item(&source)["metadata"]["onetaskgraph.template"];
        assert_eq!(entry["body_digest"], json!(body_digest(&content)));
        for kept in ["template", "digest", "answers_digest"] {
            assert_eq!(entry[kept], original[kept], "{kept}");
        }

        // A re-copy of unchanged bytes passes the plugin no bytes and keeps every URL.
        let again = folders.copy(&[verb, "copy", id, "--to", "served"]);
        assert_eq!(landed(&again), copied);
        assert_eq!(again["items"][0]["action"], json!("unchanged"), "{verb}");
        let writes = Folders::logged(&folders.served_log);
        assert!(
            writes[writes_before + 1..]
                .iter()
                .flat_map(|write| write["received"].as_array().cloned().unwrap_or_default())
                .all(|asset| asset["decoded_sha256"].is_null()),
            "no asset bytes reached the plugin"
        );
        assert_eq!(recorded(&folders.show(verb, &copied)), first);

        // One asset's bytes change at the source, and the re-copy updates the record it made.
        folders.exits(
            &[
                verb,
                "render",
                id,
                "--no-interactive",
                "--asset",
                &changed_path,
            ],
            0,
        );
        let writes_before = Folders::logged(&folders.served_log).len();
        folders.copy(&[verb, "copy", id, "--to", "served"]);
        let writes = Folders::logged(&folders.served_log);
        let update = &writes[writes_before];
        assert_eq!(
            update["target"],
            json!(copied.split_once(':').expect("qualified").1)
        );
        assert_eq!(
            update["received"],
            json!([
                {"name": "screen.png", "content_type": "image/png",
                 "decoded_sha256": sha256(&changed)},
                {"name": "detail.webp", "content_type": "image/webp", "decoded_sha256": null},
            ])
        );
        assert_ne!(sha256(&changed), sha256(&screen));
        assert_eq!(
            update["recorded_assets"],
            item(&shown)["metadata"]["onetaskgraph.assets"],
            "the update was handed the record the first copy made"
        );
        let after = folders.show(verb, &copied);
        let now = recorded(&after);
        assert_eq!(
            now[0], first[0],
            "the unchanged asset keeps its sha256 and url"
        );
        assert_eq!(now[1].1, sha256(&changed));
        assert_ne!(
            now[1].2, first[1].2,
            "the changed asset has the plugin's new url"
        );
        let content = item(&after)["content"].as_str().expect("content");
        assert!(content.contains(now[1].2.as_str()) && !content.contains(first[1].2.as_str()));
        assert_eq!(json!(content), update["answered"]["content"]);

        // The source stops showing pictures: the re-copy takes every upload away with them.
        let none = folders.text("none.yaml", "shots: []\n");
        folders.exits(
            &[verb, "render", id, "--no-interactive", "--answers", &none],
            0,
        );
        let writes_before = Folders::logged(&folders.served_log).len();
        folders.copy(&[verb, "copy", id, "--to", "served"]);
        let removal = &Folders::logged(&folders.served_log)[writes_before];
        assert_eq!(
            removal["received"],
            json!([]),
            "{verb}: a write of no assets at all"
        );
        let after = folders.show(verb, &copied);
        assert!(
            item(&after)["metadata"]
                .get("onetaskgraph.assets")
                .is_none()
        );
        assert!(
            !item(&after)["content"]
                .as_str()
                .unwrap_or_default()
                .contains("https://")
        );
    }
}

#[test]
fn an_undone_copy_through_the_protocol_puts_back_the_uploads_it_replaced() {
    let folders = Folders::new();
    let template = folders.text("pictures.md", PICTURES);
    let answers = folders.text("one.yaml", "shots: [screen.png]\n");
    let mut tasks = Vec::new();
    for (title, seed) in [("First", 160), ("Second", 161)] {
        tasks.push(folders.created(&[
            "task",
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            title,
            "--template",
            &template,
            "--answers",
            &answers,
            "--no-interactive",
            "--asset",
            &folders.image(title, "screen.png", &images::png(seed, SCREENSHOT)),
        ]));
    }
    let report = folders.copy(&["project", "copy", "notes:launch", "--to", "flaky"]);
    let first = report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|outcome| outcome["source"] == json!(tasks[0]))
        .and_then(|outcome| outcome["destination"].as_str())
        .expect("the first task landed")
        .to_owned();
    let before = folders.show("task", &first);

    // Both pictures change; the second task's update is applied and refused, and the copy is
    // undone — the first task put back over the protocol with the uploads it held.
    for (index, title) in ["First", "Second"].into_iter().enumerate() {
        let changed = folders.image(
            &format!("{title}-changed"),
            "screen.png",
            &images::png(170 + index as u64, SCREENSHOT),
        );
        folders.exits(
            &[
                "task",
                "render",
                &tasks[index],
                "--no-interactive",
                "--asset",
                &changed,
            ],
            0,
        );
    }
    let writes_before = Folders::logged(&folders.flaky_log).len();
    let said = folders.refused(&["project", "copy", "notes:launch", "--to", "flaky"]);
    assert!(said.contains("applied and then refused"), "{said}");
    assert_eq!(item(&folders.show("task", &first)), item(&before));
    let restored = Folders::logged(&folders.flaky_log)[writes_before..]
        .iter()
        .rev()
        .find(|write| write["target"] == json!(first.split_once(':').expect("qualified").1))
        .cloned()
        .expect("the first task was written back");
    assert_eq!(
        restored["received"],
        json!([{"name": "screen.png", "content_type": "image/png", "decoded_sha256": null}]),
        "put back by reusing the upload it held, sending no bytes"
    );
    assert_eq!(
        restored["recorded_assets"],
        item(&before)["metadata"]["onetaskgraph.assets"]
    );
}

#[test]
fn the_png_generator_holds_its_range_and_its_tolerance() {
    for (seed, size) in [(1, 50_000), (2, 123_456), (3, 500_000)] {
        let png = images::png(seed, size);
        assert!(
            png.len().abs_diff(size) <= size * images::PNG_SIZE_TOLERANCE_PERCENT / 100,
            "{} bytes for {size}",
            png.len()
        );
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
    assert!((450_000..=500_000).contains(&images::png(4, SCREENSHOT).len()));
    for seed in 0..8 {
        assert_ne!(
            images::png(seed, 50_000),
            images::png(seed + 1, 50_000),
            "{seed}"
        );
    }
    assert_ne!(images::gif(1), images::gif(2));
    assert_ne!(images::webp(1), images::webp(2));
    assert_eq!(images::png(5, 50_000), images::png(5, 50_000));
    for outside in [
        *images::PNG_SIZE_RANGE.start() - 1,
        *images::PNG_SIZE_RANGE.end() + 1,
    ] {
        let refused = std::panic::catch_unwind(|| images::png(7, outside))
            .expect_err("a size outside the range fails the test");
        let said = refused
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(said.contains(&outside.to_string()), "{said}");
    }
}

#[test]
fn a_record_whose_assets_cannot_be_read_is_shown_without_them_and_the_failure_named() {
    let folders = Folders::new();
    let body = folders.text("pictured.md", "![p](./p.png)\n");
    let png = folders.image("p", "p.png", &images::png(100, 50_000));
    let task = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Broken",
        "--body-file",
        &body,
        "--asset",
        &png,
    ]);
    let document = folders.created(&[
        "document",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Broken",
        "--body-file",
        &body,
        "--asset",
        &png,
    ]);
    // Each record's asset directory replaced by a file: its assets can no longer be listed.
    for directory in ["tasks/broken.assets", "documents/broken.assets"] {
        let directory = folders.notes.join(directory);
        std::fs::remove_dir_all(&directory).expect("the asset directory");
        std::fs::write(&directory, "not a directory").expect("a file in its place");
    }
    for arguments in [
        vec!["task", "show", task.as_str(), "--json"],
        vec!["task", "show", task.as_str(), "--no-comments", "--json"],
        vec!["document", "show", document.as_str(), "--json"],
    ] {
        let output = folders.exits(&arguments, 4);
        let shown: Value = serde_json::from_str(&stdout(&output)).expect("a partial answer");
        assert!(shown.get("assets").is_none(), "{arguments:?}: {shown}");
        assert_eq!(item(&shown)["title"], json!("Broken"));
        assert!(
            shown["errors"][0]["error"]["message"]
                .as_str()
                .expect("the failure")
                .contains("broken.assets")
        );
        assert!(stderr(&output).contains("broken.assets"), "{arguments:?}");
    }
}

#[test]
fn every_way_of_showing_a_record_lists_its_assets_and_says_where_each_is() {
    let folders = Folders::new();
    let body = folders.text("shown.md", "![b](./b.png) ![a](./a.gif)\n");
    let b = images::png(150, 50_000);
    let a = images::gif(151);
    let (b_path, a_path) = (
        folders.image("s", "b.png", &b),
        folders.image("s", "a.gif", &a),
    );
    let task = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Shown",
        "--body-file",
        &body,
        "--asset",
        &b_path,
        "--asset",
        &a_path,
    ]);
    let document = folders.created(&[
        "document",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Shown",
        "--body-file",
        &body,
        "--asset",
        &b_path,
        "--asset",
        &a_path,
    ]);
    // An asset a person put beside the record that its content does not reference is listed
    // after those it does, by name.
    std::fs::write(
        folders.notes.join("tasks/shown.assets/0-extra.webp"),
        images::webp(152),
    )
    .expect("an extra asset");
    let names = |shown: &Value| -> Vec<String> {
        listed(shown).into_iter().map(|(name, ..)| name).collect()
    };
    assert_eq!(
        names(&folders.show("task", &task)),
        ["b.png", "a.gif", "0-extra.webp"]
    );

    // Each detail `task show-many` answers carries its task's assets, as `task show` does.
    let plain = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Bare",
        "--body-file",
        &folders.text("bare.md", "nothing\n"),
    ]);
    let many: Value = serde_json::from_str(&stdout(
        &folders.exits(&["task", "show-many", &task, &plain, "--json"], 0),
    ))
    .expect("details");
    assert_eq!(
        many["details"][0]["assets"],
        folders.show("task", &task)["assets"]
    );
    assert_eq!(many["details"][1]["assets"], json!([]));

    // The human renderings list each asset after the body: name, type, digest and path.
    for (verb, id) in [("task", &task), ("document", &document)] {
        let shown = folders.show(verb, id);
        let text = stdout(&folders.exits(&[verb, "show", id], 0));
        let expected = listed(&shown).len();
        assert!(
            text.contains(&format!("assets: {expected}")),
            "{verb}: {text}"
        );
        for asset in shown["assets"].as_array().expect("assets") {
            for field in ["name", "content_type", "sha256", "path"] {
                let value = asset[field].as_str().expect("a field");
                assert!(text.contains(value), "{verb} names {field} {value}: {text}");
            }
        }
    }
    // And a record holding none says nothing about assets.
    assert!(!stdout(&folders.exits(&["task", "show", &plain], 0)).contains("assets:"));

    // A detail whose assets cannot be read carries the failure, and the others are whole.
    let directory = folders.notes.join("tasks/shown.assets");
    std::fs::remove_dir_all(&directory).expect("the asset directory");
    std::fs::write(&directory, "not a directory").expect("a file in its place");
    let output = folders.exits(&["task", "show-many", &task, &plain, "--json"], 4);
    let many: Value = serde_json::from_str(&stdout(&output)).expect("details");
    assert!(many["details"][0].get("assets").is_none());
    assert!(
        many["details"][0]["errors"][0]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("shown.assets"))
    );
    assert_eq!(many["details"][1]["assets"], json!([]));
}

#[test]
fn a_render_dry_run_reports_an_asset_change_and_writes_nothing() {
    let folders = Folders::new();
    let template = folders.text("pictures.md", PICTURES);
    let answers = folders.text("one.yaml", "shots: [shot.png]\n");
    let shot = images::png(110, 60_000);
    let id = folders.created(&[
        "task",
        "create",
        "notes",
        "--project",
        "launch",
        "--title",
        "Dry",
        "--template",
        &template,
        "--answers",
        &answers,
        "--no-interactive",
        "--asset",
        &folders.image("old", "shot.png", &shot),
    ]);
    let before = Folders::files(&folders.notes);
    let output = folders.exits(
        &[
            "task",
            "render",
            &id,
            "--no-interactive",
            "--dry-run",
            "--json",
            "--asset",
            &folders.image("new", "shot.png", &images::png(111, 60_000)),
        ],
        0,
    );
    let report: Value = serde_json::from_str(&stdout(&output)).expect("a report");
    assert_eq!(report["changed"], json!(true), "the asset differs");
    assert_eq!(Folders::files(&folders.notes), before);
    assert_eq!(held_bytes(&folders.show("task", &id)), vec![shot]);
}

#[test]
fn the_reference_host_carries_a_copys_assets_and_refuses_a_rendered_create_with_them() {
    let sandbox = Sandbox::new();
    let notes = sandbox.subdirectory("notes");
    let hosted = sandbox.subdirectory("hosted");
    let inputs = sandbox.subdirectory("inputs");
    sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "hosted": SourceBoundary::Subprocess.source("local-md", json!({"root": hosted})),
    })));
    let run = |arguments: &[&str], code: i32| {
        let output = sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "{arguments:?}: {}",
            stderr(&output)
        );
        output
    };
    let screen = images::png(120, SCREENSHOT);
    let screen_path = inputs.join("screen.png");
    std::fs::write(&screen_path, &screen).expect("an image");
    let body = inputs.join("body.md");
    std::fs::write(&body, "![screen](./screen.png)\n").expect("a body");
    for verb in ["task", "document"] {
        let id = stdout(&run(
            &[
                verb,
                "create",
                "notes",
                "--project",
                "launch",
                "--title",
                "Hosted",
                "--body-file",
                &spelled(&body),
                "--asset",
                &spelled(&screen_path),
            ],
            0,
        ))
        .trim()
        .to_owned();
        let report: Value = serde_json::from_str(&stdout(&run(
            &[verb, "copy", &id, "--to", "hosted", "--json"],
            0,
        )))
        .expect("a report");
        let landed = report["items"][0]["destination"].as_str().expect("an id");
        let native = landed.split_once(':').expect("qualified").1;
        let folder = if verb == "task" { "tasks" } else { "documents" };
        // The host stored them where its folder keeps a record's assets.
        assert_eq!(
            std::fs::read(
                hosted
                    .join(folder)
                    .join(format!("{native}.assets/screen.png"))
            )
            .expect("the hosted folder holds the asset"),
            screen,
            "{verb}"
        );
    }
    // A create rendered from a template is not carried over the protocol, assets or none.
    let template = inputs.join("pictures.md");
    std::fs::write(&template, PICTURES).expect("a template");
    let answers = inputs.join("answers.yaml");
    std::fs::write(&answers, "shots: [screen.png]\n").expect("answers");
    for verb in ["task", "document"] {
        let said = stderr(&run(
            &[
                verb,
                "create",
                "hosted",
                "--project",
                "launch",
                "--title",
                "Rendered",
                "--template",
                &spelled(&template),
                "--answers",
                &spelled(&answers),
                "--no-interactive",
                "--asset",
                &spelled(&screen_path),
            ],
            1,
        ));
        assert!(
            said.contains("from a template") && said.contains("stdio plugin protocol"),
            "{verb}: {said}"
        );
    }
    assert!(!hosted.join("tasks").join("rendered.md").exists());
}

/// One line of the stdio protocol written to `host` and the response it answers with.
fn exchange(
    host: &mut std::process::Child,
    reader: &mut impl std::io::BufRead,
    request: &Value,
) -> Value {
    use std::io::Write as _;
    let input = host.stdin.as_mut().expect("the host's input");
    writeln!(input, "{request}").expect("the request is written");
    input.flush().expect("flushed");
    let mut line = String::new();
    reader.read_line(&mut line).expect("the host answers");
    serde_json::from_str(&line).expect("one JSON response")
}

#[test]
fn the_reference_host_refuses_a_write_recording_uploads_it_carries_no_assets_for() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("hosted");
    let mut host = std::process::Command::new(onetaskgraph_e2e_support::source_binary())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the host starts");
    let mut reader = std::io::BufReader::new(host.stdout.take().expect("the host's output"));
    let initialized = exchange(
        &mut host,
        &mut reader,
        &json!({"id": "0", "method": "initialize", "params": {
            "protocol_version": 2,
            "engine": {"name": "onetaskgraph", "version": "0"},
            "source_name": "hosted",
            "config": {"kind": "local-md", "config": {"root": root}},
            "secrets": {},
        }}),
    );
    assert_eq!(
        initialized["result"]["capabilities"]["assets"],
        json!("native")
    );
    let item = |kind: &str| match kind {
        "task" => json!({"id": "t", "title": "T", "content": "x",
                         "status": {"category": "todo", "name": "todo"}, "labels": []}),
        _ => json!({"id": "d", "title": "D", "content": "x", "labels": []}),
    };
    for (index, kind) in ["task", "document"].into_iter().enumerate() {
        let answered = exchange(
            &mut host,
            &mut reader,
            &json!({"id": format!("{}", index + 1), "method": format!("write_{kind}"),
            "params": {
                "write": {"target": null, "item": item(kind), "depends_on": []},
                "recorded_assets": {}
            }}),
        );
        assert_eq!(
            answered["error"]["kind"],
            json!("malformed"),
            "{kind}: {answered}"
        );
        assert!(
            answered["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("recorded_assets")),
            "{kind}: {answered}"
        );
    }
    // An asset's bytes are base64 when present, and absent — never `null` — when reused.
    let digest = format!("{:064x}", 0);
    for (index, bytes) in [json!("not base64!"), Value::Null].into_iter().enumerate() {
        let answered = exchange(
            &mut host,
            &mut reader,
            &json!({"id": format!("b{index}"), "method": "write_task", "params": {
                "write": {"target": null, "item": item("task"), "depends_on": []},
                "assets": [{"name": "a.png", "sha256": digest, "content_type": "image/png",
                            "bytes": bytes}],
            }}),
        );
        assert_eq!(answered["error"]["kind"], json!("malformed"), "{answered}");
    }
    drop(host.stdin.take());
    assert!(host.wait().expect("the host exits").success());
    assert!(Folders::files(&root).is_empty(), "nothing was written");
}

#[test]
fn the_python_peer_refuses_a_write_recording_uploads_it_carries_no_assets_for() {
    let folders = Folders::new();
    let store = folders.inputs.join("peer.json");
    let mut peer = std::process::Command::new(interpreter())
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/asset_store.py"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the peer starts");
    let mut reader = std::io::BufReader::new(peer.stdout.take().expect("its output"));
    let initialized = exchange(
        &mut peer,
        &mut reader,
        &json!({"id": "0", "method": "initialize", "params": {
            "protocol_version": 2, "config": {"store": store, "assets": "native"},
        }}),
    );
    assert_eq!(
        initialized["result"]["capabilities"]["assets"],
        json!("native")
    );
    let answered = exchange(
        &mut peer,
        &mut reader,
        &json!({"id": "1", "method": "write_document", "params": {
            "write": {"target": null, "item": {"id": "d", "title": "D", "content": "x"}},
            "recorded_assets": {},
        }}),
    );
    assert_eq!(answered["error"]["kind"], json!("malformed"), "{answered}");
    drop(peer.stdin.take());
    assert!(peer.wait().expect("the peer exits").success());
    assert!(!store.exists(), "nothing was written");
}

#[test]
fn a_source_over_the_protocol_refuses_a_regenerate_storing_assets_before_sending_anything() {
    use onetaskgraph_plugin_api::{
        AssetName, AssetPayload, AssetWrite, NativeId, SourceName, SourcePlugin,
    };
    let secrets = onetaskgraph_core::Secrets::load(onetaskgraph_core::Environment::from_pairs(
        std::iter::empty::<(String, String)>(),
    ))
    .expect("no credentials");
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("hosted");
    let source = onetaskgraph_core::SubprocessPlugin
        .build(
            &SourceName::new("hosted").expect("a name"),
            &json!({
                "command": onetaskgraph_e2e_support::source_binary(),
                "settings": {"kind": "local-md", "config": {"root": root}},
            }),
            &secrets,
        )
        .expect("the source starts");
    let assets = AssetWrite {
        assets: vec![AssetPayload::of(
            AssetName::new("a.png").expect("a name"),
            images::png(140, 50_000),
        )],
        recorded_assets: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let id = NativeId::from("t");
    for refused in [
        runtime.block_on(source.set_task_rendering_with_assets(
            &id,
            "![a](./a.png)",
            &json!({}),
            &Default::default(),
            &assets,
        )),
        runtime.block_on(source.set_document_rendering_with_assets(
            &id,
            "![a](./a.png)",
            &json!({}),
            &Default::default(),
            &assets,
        )),
    ] {
        let refused = refused.expect_err("refused");
        assert!(
            refused.to_string().contains("regenerate in place")
                && refused.to_string().contains("stdio plugin protocol"),
            "{refused}"
        );
    }
    assert!(Folders::files(&root).is_empty(), "nothing was written");
}

/// What the asset store over its real pipe makes of each of `names` and `contents`: whether a
/// `write_document` carrying an asset under each name is accepted, and the content each of
/// `contents` lands as when a write carries an asset under every name the contract accepts.
fn peer_reading(folders: &Folders, names: &[&str], contents: &[&str]) -> (Vec<bool>, Vec<Value>) {
    use onetaskgraph_plugin_api::AssetName;
    let mut peer = std::process::Command::new(interpreter())
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/asset_store.py"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the peer starts");
    let mut reader = std::io::BufReader::new(peer.stdout.take().expect("its output"));
    let initialized = exchange(
        &mut peer,
        &mut reader,
        &json!({"id": "0", "method": "initialize", "params": {
            "protocol_version": 2,
            "config": {"store": folders.inputs.join("reading.json"), "assets": "native"},
        }}),
    );
    assert_eq!(
        initialized["result"]["capabilities"]["assets"],
        json!("native")
    );
    // One byte, `x`, under every name: what each name is served at depends only on the name.
    let payload = |name: &str| {
        let content_type =
            AssetName::new(name).map_or("image/png", |name| name.content_type().as_str());
        json!({"name": name, "sha256": sha256(b"x"), "content_type": content_type, "bytes": "eA=="})
    };
    let mut write = |assets: Vec<Value>, content: &str| {
        exchange(
            &mut peer,
            &mut reader,
            &json!({"id": "1", "method": "write_document", "params": {
                "write": {"target": null, "item": {"id": "d", "title": "D", "content": content}},
                "assets": assets,
            }}),
        )
    };
    let accepted = names
        .iter()
        .map(|name| {
            let answered = write(vec![payload(name)], "");
            assert!(
                answered.get("result").is_some() || answered["error"]["kind"] == json!("malformed"),
                "{name}: {answered}"
            );
            answered.get("result").is_some()
        })
        .collect();
    let served: Vec<Value> = names
        .iter()
        .filter(|name| AssetName::new(**name).is_ok())
        .map(|name| payload(name))
        .collect();
    let landed = contents
        .iter()
        .map(|content| write(served.clone(), content)["result"]["content"].clone())
        .collect();
    drop(peer.stdin.take());
    assert!(peer.wait().expect("the peer exits").success());
    (accepted, landed)
}

#[test]
fn the_python_peer_reads_the_asset_convention_exactly_as_the_contract_does() {
    use onetaskgraph_plugin_api::{AssetName, rewrite_asset_references};
    let folders = Folders::new();
    let names = [
        "a.png",
        "Scan.JpEg",
        "x.jpg",
        "y.gif",
        "z.webp",
        "W.PNG",
        ".x.png",
        "a/b.png",
        "a\\b.png",
        "..png",
        "a..b.png",
        "a b.png",
        ".png",
        "a.txt",
        "a(b).png",
        "a.png.txt",
        "",
    ];
    let contents = [
        "![a](./a.png) ![w](./W.PNG \"title\") ![again](./a.png)",
        "![nested](./img/a.png) ![up](../a.png) ![bare](a.png) [plain](./a.png)",
        "![remote](https://example.invalid/a.png) ![x](./x.jpg)\n![y](./y.gif)",
        "![multi\nline](./a.png) ![unclosed](./a.png",
        "![spaced](./a.png \n![title](./a.png \"t\")\n![dangling](./a.png ",
        "![newline](./a.png\n)",
        // Only an image outside code is a reference: each of these is text.
        "\\![escaped](./a.png) \\\\![after](./x.jpg) `![span](./a.png)` ``![x](./y.gif)``",
        "```\n![fenced](./a.png)\n```\n![real](./x.jpg)\n~~~~md\n![tilde](./y.gif)\n~~~\n~~~~",
        "para\n    ![continued](./a.png)\n\n    ![indented](./x.jpg)\n\n![real](./y.gif)",
        "- item\n\n  ```\n  ![listed](./a.png)\n  ```\n\n      ![code](./x.jpg)\n\n  ![real](./y.gif)",
        "> ```\n> ![quoted](./a.png)\n> ```\n>\n>     ![code](./x.jpg)\n\n![a`](./y.gif)` `unclosed ![u](./z.webp)",
        "1. one\n   ```\n   ![n](./a.png)\n   ```\n# h\n    ![after heading](./x.jpg)\n```\n![open](./y.gif)",
        "    ![first line](./a.png)\n![real](./x.jpg) ```![triple](./y.gif)``` `` ` ![mixed](./z.webp) ``",
        // A block quote opened after a list marker, holding a fence: its image is code.
        "- > ~~~\n  > ![example](./a.png)\n  > ~~~\n\n![real](./a.png)",
        "1. > > ```\n   > > ![deep](./a.png)\n   > > ```\n   > ![quoted](./x.jpg)\n- ![item](./y.gif)",
    ];
    let (peer_names, peer_contents) = peer_reading(&folders, &names, &contents);
    let accepted: Vec<bool> = names
        .iter()
        .map(|name| AssetName::new(*name).is_ok())
        .collect();
    assert_eq!(peer_names, accepted);
    let served: std::collections::BTreeMap<AssetName, String> = names
        .iter()
        .filter_map(|name| AssetName::new(*name).ok())
        .map(|name| {
            let url = format!("https://assets.example.invalid/{}/{name}", sha256(b"x"));
            (name, url)
        })
        .collect();
    let rewritten: Vec<Value> = contents
        .iter()
        .map(|content| json!(rewrite_asset_references(content, &served)))
        .collect();
    assert_eq!(peer_contents, rewritten);
}

/// The first-column names of the table headed `header` inside `section`.
fn table(section: &str, header: &str) -> Vec<String> {
    let mut lines = section.lines().skip_while(|line| *line != header);
    assert!(
        lines.next().is_some(),
        "the protocol has a table headed {header}"
    );
    lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            line.split('|')
                .nth(1)
                .expect("a first cell")
                .trim()
                .trim_matches('`')
                .to_owned()
        })
        .collect()
}

/// The property names the emitted schema's root `root` declares, sorted.
fn properties(bundle: &Value, root: &str) -> Vec<String> {
    let mut names: Vec<String> = bundle["roots"][root]["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("the bundle declares {root}"))
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

#[test]
fn the_protocol_document_states_exactly_the_asset_members_the_binary_emits() {
    let bundle: Value =
        serde_json::from_str(&stdout(&Folders::new().exits(&["schema"], 0))).expect("a bundle");
    let protocol = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/plugin-protocol.md"),
    )
    .expect("the protocol document");
    let section: String = protocol
        .lines()
        .skip_while(|line| !line.starts_with("### 4.9a "))
        .take_while(|line| !line.starts_with("### 4.10 "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!section.is_empty(), "§4.9a states the asset members");

    assert_eq!(
        sorted(table(&section, "| Member | Type | Meaning |")),
        properties(&bundle, "AssetWrite")
    );
    assert_eq!(
        sorted(table(&section, "| Asset member | Type | Meaning |")),
        properties(&bundle, "AssetPayload")
    );
    assert_eq!(
        sorted(table(&section, "| Result member | Type | Meaning |")),
        properties(&bundle, "AssetsWritten")
    );

    // The content types the document spells are the ones the schema allows.
    let row = section
        .lines()
        .find(|line| line.starts_with("| `content_type` |"))
        .expect("a content_type row");
    let mut stated: Vec<String> = row
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    stated.sort();
    let mut allowed: Vec<String> = bundle["roots"]["AssetContentType"]["oneOf"]
        .as_array()
        .map(|variants| {
            variants
                .iter()
                .filter_map(|variant| variant["const"].as_str().map(str::to_owned))
                .collect()
        })
        .or_else(|| {
            bundle["roots"]["AssetContentType"]["enum"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_owned))
                        .collect()
                })
        })
        .expect("the content types are an enumeration");
    allowed.sort();
    assert_eq!(stated, allowed);

    // `onetaskgraph.assets` is stated as an object keyed by asset name whose every value is
    // the shape `AssetUpload` is.
    let record = section
        .split("keyed by asset name, each value")
        .nth(1)
        .expect("the document states the onetaskgraph.assets value");
    let shape = &record[record.find("`{").expect("a stated shape") + 1..];
    let shape = &shape[..shape.find("}`").expect("the shape closes") + 1];
    let mut keys: Vec<String> = shape
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    keys.sort();
    assert_eq!(keys, properties(&bundle, "AssetUpload"));

    assert!(bundle["roots"]["Capabilities"]["properties"]["assets"].is_object());
}
