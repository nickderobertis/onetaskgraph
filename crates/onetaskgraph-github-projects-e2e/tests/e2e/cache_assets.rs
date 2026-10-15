//! The verified bytes of a GitHub board's image assets in the store's local cache, driven
//! through the binary against the loopback board and its attachment host, doubling only GitHub.
//!
//! The bytes of a board record's image are read when the record is rendered again keeping it:
//! the board holds the image as an attachment, and the render reads it to carry it. A byte entry
//! is named by its own SHA-256 and served only for a manifest the cache holds at a version the
//! board's token covers — and only when the kept bytes still hash to that name. So a render of a
//! task or a document whose image is unchanged reads no attachment again; one whose manifest
//! names other bytes reads them; a byte file that no longer hashes to its name is read again from
//! the board; and `cache clear` reclaims a source's item and byte entries together and leaves
//! another source's alone.

use std::path::{Path, PathBuf};
use std::process::Output;

use onetaskgraph_e2e_support::{
    clock::SimulatedClock,
    common::{Sandbox, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_board},
    images,
};
use onetaskgraph_plugin_api::asset_sha256;
use serde_json::{Value, json};

use super::asset_board::Board;

/// The size of each image, as the cache budget's workload names it: 256 KiB.
const IMAGE: usize = 256 * 1024;

/// A template rendering one line and one picture, `./shot.png`.
const PICTURE: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  note:\n    \
    description: What changed\n    \
    type: string\n\
---\n\
# The change\n\
{{ note }}\n\
![shot](./shot.png)\n";

/// A caching loopback board behind its attachment host, holding a task and a document each
/// rendered from [`PICTURE`] with one image — and a second source reading the same board into a
/// cache of its own.
struct Assets {
    sandbox: Sandbox,
    board: Board,
    github: GitHubBoardFields,
    clock: SimulatedClock,
    cache_home: PathBuf,
    mirror_cache: PathBuf,
    template: PathBuf,
    /// The bytes the task and the document were created with, each its own.
    created: [Vec<u8>; 2],
    /// The board's ids of the rendered task and document.
    task: String,
    document: String,
}

impl Assets {
    fn new() -> Self {
        let clock = SimulatedClock::start(1);
        let sandbox = Sandbox::new();
        let (mut config, github) = github_projects_with_board(&sandbox);
        let board = Board::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
        config["endpoint"] = json!(board.endpoint);
        config["pacing"] = json!({});
        let mirror_cache = sandbox.subdirectory("mirror-cache");
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config,"cache":{"enabled":true}},
            "mirror": {"plugin":"github-projects","config":config,
                       "cache":{"enabled":true,"dir":mirror_cache}},
        })));
        let cache_home = sandbox.subdirectory("cache-home");
        let template = sandbox.project().join("picture.md");
        std::fs::write(&template, PICTURE).unwrap();
        let mut assets = Self {
            sandbox,
            board,
            github,
            clock,
            cache_home,
            mirror_cache,
            template,
            created: [images::png(0xA1, IMAGE), images::png(0xD1, IMAGE)],
            task: String::new(),
            document: String::new(),
        };
        for kind in ["task", "document"] {
            let shot = assets.image(&format!("created-{kind}"), assets.created(kind));
            let template = assets.template.display().to_string();
            let arguments = vec![
                kind,
                "create",
                "board",
                "--title",
                "Pictures",
                "--template",
                &template,
                "--var",
                "note=created",
                "--no-interactive",
                "--asset",
                &shot,
                "--project",
                "P-1",
                "--json",
            ];
            let (created, _, _) = assets.ok(&arguments);
            let id = created["items"][0]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("the {kind} was created: {created}"))
                .strip_prefix("board:")
                .unwrap()
                .to_owned();
            match kind {
                "task" => assets.task = id,
                _ => assets.document = id,
            }
        }
        assets
    }

    /// The bytes the `kind` record was created with.
    fn created(&self, kind: &str) -> &[u8] {
        &self.created[usize::from(kind != "task")]
    }

    /// `bytes` written as `shot.png` in a directory of their own named `name`, answering its path.
    fn image(&self, name: &str, bytes: &[u8]) -> String {
        let directory = self.sandbox.subdirectory(name);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("shot.png");
        std::fs::write(&path, bytes).unwrap();
        path.display().to_string()
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .env("XDG_CACHE_HOME", &self.cache_home)
            .envs(self.clock.client_env(0))
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    /// One command that had to succeed: what it printed, every GraphQL document the board served
    /// for it, and how many attachments it read from the attachment host.
    fn ok(&self, arguments: &[&str]) -> (Value, Vec<String>, usize) {
        let (documents, calls) = (self.github.served().len(), self.board.calls().len());
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        let served = self.github.served()[documents..]
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        let reads = self.board.calls()[calls..]
            .iter()
            .filter(|call| call.path.starts_with("/user-attachments/assets/"))
            .count();
        (
            serde_json::from_str(&stdout(&output)).expect("JSON"),
            served,
            reads,
        )
    }

    /// Render the `kind` record of `source` again with the note `note`, keeping its picture or —
    /// with `replacing` — giving it new bytes: how many attachments the render read.
    fn render(&self, source: &str, kind: &str, note: &str, replacing: Option<&str>) -> usize {
        let native = if kind == "task" {
            &self.task
        } else {
            &self.document
        };
        let id = format!("{source}:{native}");
        let note = format!("note={note}");
        let mut arguments = vec![
            kind,
            "render",
            &id,
            "--var",
            &note,
            "--no-interactive",
            "--json",
        ];
        if let Some(path) = replacing {
            arguments.extend(["--asset", path]);
        }
        self.ok(&arguments).2
    }

    /// The digest the `kind` record's manifest names its picture by, and the bytes the attachment
    /// host holds at the address it records.
    fn held(&self, kind: &str) -> (String, Vec<u8>) {
        let native = if kind == "task" {
            &self.task
        } else {
            &self.document
        };
        let (shown, _, _) = self.ok(&[kind, "show", &format!("board:{native}"), "--json"]);
        let upload = &shown["items"][0]["item"]["metadata"]["onetaskgraph.assets"]["shot.png"];
        let url = upload["url"].as_str().expect("the picture's address");
        let path = url
            .strip_prefix(self.board.endpoint.trim_end_matches("/graphql"))
            .expect("an address of the attachment host");
        (
            upload["sha256"].as_str().unwrap().to_owned(),
            self.board.attachment(path).expect("the attachment is held"),
        )
    }

    /// The board source's cache directory under the cache home.
    fn board_cache(&self) -> PathBuf {
        let root = self.cache_home.join("onetaskgraph");
        std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .next()
            .expect("the board's cache")
    }
}

/// Every file under `root`.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn a_render_keeping_an_unchanged_picture_reads_no_attachment_twice() {
    let assets = Assets::new();
    for kind in ["task", "document"] {
        assert_eq!(
            assets.render("board", kind, "first", None),
            1,
            "{kind}: the first render reads the picture it keeps"
        );
        assert_eq!(
            assets.render("board", kind, "second", None),
            0,
            "{kind}: unchanged, the verified bytes are the cache's"
        );
        let (digest, held) = assets.held(kind);
        assert_eq!(digest, asset_sha256(assets.created(kind)), "{kind}");
        assert_eq!(
            held,
            assets.created(kind),
            "{kind}: what the board holds is byte for byte"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_picture_the_cache_cannot_keep_is_read_from_the_board_at_every_render_until_it_can() {
    use std::os::unix::fs::PermissionsExt;
    let assets = Assets::new();
    // A read of the document reads no picture, and makes the board's cache directory.
    let _ = assets.held("document");
    let bytes = assets.board_cache().join("bytes-v2");
    std::fs::create_dir_all(&bytes).unwrap();
    std::fs::set_permissions(&bytes, std::fs::Permissions::from_mode(0o555)).unwrap();
    let refused: Vec<usize> = ["first", "second"]
        .map(|note| assets.render("board", "document", note, None))
        .to_vec();
    std::fs::set_permissions(&bytes, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        refused,
        [1, 1],
        "bytes the cache could not keep are read from the board each time, and the render lands"
    );
    assert!(
        walk(&bytes).is_empty(),
        "nothing was kept: {:?}",
        walk(&bytes)
    );
    let (digest, held) = assets.held("document");
    assert_eq!(digest, asset_sha256(assets.created("document")));
    assert_eq!(held, assets.created("document"));
    assert_eq!(
        assets.render("board", "document", "third", None),
        1,
        "once the directory takes them, the next render keeps what it reads"
    );
    assert_eq!(assets.render("board", "document", "fourth", None), 0);
}

#[test]
fn a_manifest_naming_other_bytes_reads_them_and_bytes_that_no_longer_hash_are_read_again() {
    let assets = Assets::new();
    assets.render("board", "document", "first", None);
    // The picture replaced: the manifest now names bytes the cache does not hold.
    let replaced = images::png(0xD2, IMAGE);
    let path = assets.image("replaced", &replaced);
    assets.render("board", "document", "replaced", Some(&path));
    assert_eq!(
        assets.render("board", "document", "after", None),
        1,
        "bytes no entry holds are read from the board"
    );
    assert_eq!(
        assets.render("board", "document", "kept", None),
        0,
        "and kept, verified, for the next render"
    );

    // A byte entry that no longer hashes to its own name is never answered with: the bytes are
    // read from the board again, and what the board keeps is what it held.
    let entry = walk(&assets.board_cache().join("bytes-v2"))
        .into_iter()
        .find(|path| path.file_name().unwrap().to_string_lossy() == asset_sha256(&replaced))
        .expect("the replacement's verified bytes are kept under their digest");
    let mut corrupted = replaced.clone();
    corrupted[IMAGE / 2] ^= 0xFF;
    std::fs::write(&entry, &corrupted).unwrap();
    assert_eq!(
        assets.render("board", "document", "after corruption", None),
        1,
        "a corrupted byte entry costs the read it saved"
    );
    let (digest, held) = assets.held("document");
    assert_eq!(digest, asset_sha256(&replaced), "and is never what is kept");
    assert_eq!(held, replaced);
}

#[test]
fn cache_clear_reclaims_a_sources_items_and_bytes_and_leaves_another_sources_alone() {
    let assets = Assets::new();
    // Each source renders the document once, so each keeps its item and byte entries.
    assets.render("board", "document", "board", None);
    assets.ok(&[
        "document",
        "show",
        &format!("board:{}", assets.document),
        "--json",
    ]);
    assets.render("mirror", "document", "mirror", None);
    let board = assets.board_cache();
    let held = |dir: &Path| {
        let files = walk(dir);
        let named = |directory: &str| {
            files
                .iter()
                .filter(|path| {
                    path.components()
                        .any(|component| component.as_os_str() == directory)
                })
                .count()
        };
        (named("documents-v2"), named("bytes-v2"))
    };
    assert!(
        held(&board).0 >= 1 && held(&board).1 >= 1,
        "{:?}",
        walk(&board)
    );
    let mirrored = walk(&assets.mirror_cache);
    assert!(
        held(&assets.mirror_cache).1 >= 1,
        "the mirror keeps its own bytes: {mirrored:?}"
    );

    // A file carrying the board's schema, kind and remote but not the rest of an entry is not
    // one this store wrote, and is left where it is.
    let documents = board.join("documents-v2");
    let remote = walk(&documents)
        .iter()
        .find_map(|path| {
            serde_json::from_slice::<Value>(&std::fs::read(path).ok()?).ok()?["remote"]
                .as_str()
                .map(str::to_owned)
        })
        .expect("an entry names its remote");
    let foreign = documents.join("foreign.json");
    std::fs::write(
        &foreign,
        json!({"schema": 1, "kind": "document", "remote": remote}).to_string(),
    )
    .unwrap();

    // Every item entry the store wrote, of every kind — and no byte entry, which is not an item.
    let items: usize = ["tasks", "projects-v2", "documents-v2", "edges-v2"]
        .iter()
        .map(|directory| {
            walk(&board.join(directory))
                .iter()
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "json")
                        && path.file_name().is_some_and(|name| name != "foreign.json")
                })
                .count()
        })
        .sum();
    assert!(
        items >= 1,
        "the document's entry at least: {:?}",
        walk(&board)
    );
    let (cleared, _, _) = assets.ok(&["cache", "clear", "board", "--json"]);
    assert_eq!(cleared["cleared"][0]["source"], "board", "{cleared}");
    assert_eq!(
        cleared["cleared"][0]["entries"],
        json!(items),
        "every item entry of every kind is counted, and no byte entry: {cleared}"
    );
    assert!(
        foreign.exists(),
        "a file that is no whole entry is left alone"
    );
    std::fs::remove_file(&foreign).unwrap();
    assert_eq!(held(&board), (0, 0), "the board's items and bytes are gone");
    assert_eq!(
        walk(&assets.mirror_cache),
        mirrored,
        "another source's cache is exactly as it was"
    );

    // The same render again reads the bytes from the board, and the board keeps them whole.
    assert_eq!(
        assets.render("board", "document", "refetched", None),
        1,
        "a cleared cache is refilled by reading"
    );
    let (digest, held) = assets.held("document");
    assert_eq!(digest, asset_sha256(assets.created("document")));
    assert_eq!(held, assets.created("document"));
}
