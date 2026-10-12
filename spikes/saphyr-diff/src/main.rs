//! SPIKE (serde-yaml-replacement, not for landing): the differential harness.
//!
//! Drives this repository's own YAML reads and writes — through `onetaskgraph-core` and
//! `onetaskgraph-local-md` built with `yaml-spike`, so the real code paths run — once on
//! `serde_norway` and once on `serde-saphyr`, over real documents and synthetic edge cases,
//! and prints every difference by document kind. `run.sh` beside this is the one command.

mod report;
mod synthetic;
mod trees;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use onetaskgraph_core::template::{Answers, TemplateLoader};
use onetaskgraph_core::{Environment, TemplateAnswers};
use onetaskgraph_plugin_api::{
    Direction, ItemWrite, MetadataKey, NativeId, PageRequest, SecretResolver, SourceName,
    SourcePlugin, StatusCategory, TaskSource,
};
use report::{Kind, line_diff, shape};
use secrecy::SecretString;
use serde_json::Value;

/// Which crate the next calls go through, in both crates at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Norway,
    Saphyr,
}

fn select(side: Side) {
    use onetaskgraph_core::yaml_backend as core;
    use onetaskgraph_local_md::yaml_backend as md;
    match side {
        Side::Norway => {
            core::select(core::Backend::Norway);
            md::select(md::Backend::Norway);
        }
        Side::Saphyr => {
            core::select(core::Backend::Saphyr);
            md::select(md::Backend::Saphyr);
        }
    }
}

/// `f` run once on each side.
fn both<T>(mut f: impl FnMut() -> T) -> (T, T) {
    select(Side::Norway);
    let before = f();
    select(Side::Saphyr);
    let after = f();
    (before, after)
}

/// Which serde-saphyr options the candidate runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Profile {
    /// `Options::default()` and `SerializerOptions::default()`.
    Default,
    /// The options that reproduce today's behaviour where an option can: see [`compat_de`]
    /// and [`compat_ser`].
    Compat,
}

/// The compat profile is defined once, in local-md's backend, which core's re-exports.
use onetaskgraph_local_md::yaml_backend::{compat_de, compat_ser};

fn saphyr_options(profile: Profile) -> serde_saphyr::Options {
    match profile {
        Profile::Default => serde_saphyr::Options::default(),
        Profile::Compat => compat_de(),
    }
}

fn apply(profile: Profile) {
    use onetaskgraph_core::yaml_backend as core;
    use onetaskgraph_local_md::yaml_backend as md;
    match profile {
        Profile::Default => {
            core::saphyr_options(None, None);
            md::saphyr_options(None, None);
        }
        Profile::Compat => {
            core::saphyr_options(Some(compat_de), Some(compat_ser));
            md::saphyr_options(Some(compat_de), Some(compat_ser));
        }
    }
}

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

struct Args {
    profile: Profile,
    store: PathBuf,
    scratch: PathBuf,
    configs: Vec<PathBuf>,
    templates: Vec<PathBuf>,
    answers: Vec<PathBuf>,
    out: PathBuf,
}

fn list(path: &str) -> Vec<PathBuf> {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

fn args() -> Args {
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    let mut raw = std::env::args().skip(1);
    while let Some(flag) = raw.next() {
        let value = raw.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        found.insert(flag.trim_start_matches("--").to_owned(), value);
    }
    let get = |k: &str| {
        found
            .get(k)
            .cloned()
            .unwrap_or_else(|| panic!("--{k} is required"))
    };
    Args {
        profile: match found.get("profile").map_or("default", String::as_str) {
            "default" => Profile::Default,
            "compat" => Profile::Compat,
            other => panic!("--profile {other}: use default or compat"),
        },
        store: get("store").into(),
        scratch: get("scratch").into(),
        configs: list(&get("configs")),
        templates: list(&get("templates")),
        answers: list(&get("answers")),
        out: get("out").into(),
    }
}

fn shown<T: std::fmt::Display, E: std::fmt::Display>(r: &Result<T, E>) -> String {
    match r {
        Ok(v) => format!("Ok: {v}"),
        Err(e) => format!("Err: {e}"),
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("json renders")
}

/// Record a typed read through both sides: a value difference, an accept/refuse difference,
/// or an error-text difference.
fn compare_read(
    kind: &mut Kind,
    case: &str,
    path: &str,
    before: &Result<String, String>,
    after: &Result<String, String>,
) {
    match (before, after) {
        (Ok(a), Ok(b)) if a == b => {}
        (Ok(a), Ok(b)) => {
            let d = line_diff(a, b);
            kind.diff(case, &format!("read {path}"), format!("value: {}", shape(&d)), a, b);
        }
        (Ok(a), Err(b)) => kind.diff(
            case,
            &format!("read {path}"),
            "norway accepts, saphyr refuses",
            a,
            b,
        ),
        (Err(a), Ok(b)) => kind.diff(
            case,
            &format!("read {path}"),
            "norway refuses, saphyr accepts",
            a,
            b,
        ),
        (Err(a), Err(b)) if a == b => {}
        (Err(a), Err(b)) => kind.diff(
            case,
            &format!("error {path}"),
            "error text differs",
            a,
            b,
        ),
    }
}

fn value_read(text: &str) -> Result<String, String> {
    onetaskgraph_core::yaml_backend::from_str::<Value>(text)
        .map(|v| pretty(&v))
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------
// Configuration.

fn configuration(args: &Args) -> Kind {
    let mut kind = Kind::new("configuration — onetaskgraph.yaml (config/mod.rs `load` and `requested_output`)");
    let mut cases: Vec<(String, String, &str)> = args
        .configs
        .iter()
        .map(|p| {
            (
                p.display().to_string(),
                fs::read_to_string(p).expect("config reads"),
                "real",
            )
        })
        .collect();
    for (name, text) in synthetic::configs() {
        cases.push((format!("synthetic: {name}"), text, "synthetic"));
    }
    let home = args.scratch.join("config-home");
    fs::create_dir_all(&home).expect("home");
    for (i, (case, text, class)) in cases.iter().enumerate() {
        kind.count(class);
        let (a, b) = both(|| value_read(text));
        compare_read(&mut kind, case, "serde_json::Value", &a, &b);

        // The whole `load`, from a directory holding only this document.
        let dir = args.scratch.join(format!("config-{i}"));
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("onetaskgraph.yaml"), text).expect("write");
        let env = Environment::from_pairs([
            ("HOME", home.display().to_string()),
            ("XDG_CONFIG_HOME", home.join(".config").display().to_string()),
        ]);
        let (a, b) = both(|| {
            onetaskgraph_core::config::load(&dir, &env, &Default::default())
                .map(|loaded| format!("{:#?}", loaded.config))
                .map_err(|e| e.to_string())
        });
        compare_read(&mut kind, case, "config::load", &a, &b);
    }
    kind
}

// ---------------------------------------------------------------------------------------
// Template front matter: the schema-less walk.

/// The front matter of a template file, exactly as `front_matter::split` cuts it.
fn matter_of(source: &str) -> Option<String> {
    let mut lines = source.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end_matches('\n').trim_end_matches('\r') != "---" {
        return None;
    }
    let mut matter = String::new();
    for line in lines {
        if line.trim_end_matches('\n').trim_end_matches('\r') == "---" {
            return Some(matter);
        }
        matter.push_str(line);
    }
    None
}

fn front_matter(args: &Args) -> Kind {
    let options = || saphyr_options(args.profile);
    let mut kind = Kind::new(
        "template front matter (template/front_matter.rs — schema-less walk over serde_norway::Value today)",
    );
    kind.notes.push(
        "candidates: `serde_json::Value` via serde-saphyr as this workspace builds it (sorted keys), \
         `serde_json::Value` with preserve_order's ordering (the harness's `Ordered`), and saphyr 0.2's node tree"
            .to_owned(),
    );
    let mut cases: Vec<(String, String, &str)> = Vec::new();
    for p in &args.templates {
        let text = fs::read_to_string(p).expect("template reads");
        if let Some(matter) = matter_of(&text) {
            cases.push((p.display().to_string(), matter, "real"));
        }
    }
    for (name, text) in synthetic::front_matters() {
        cases.push((format!("synthetic: {name}"), text, "synthetic"));
    }
    for (case, matter, class) in &cases {
        kind.count(class);
        let today = serde_norway::from_str::<serde_norway::Value>(matter)
            .map(|v| trees::project_norway(&v))
            .map_err(|e| e.to_string());
        let candidates: [(&str, Result<Vec<String>, String>); 3] = [
            (
                "serde_json::Value (no preserve_order)",
                serde_saphyr::from_str_with_options::<Value>(matter, options())
                    .map(|v| trees::project_json(&v))
                    .map_err(|e| e.to_string()),
            ),
            (
                "serde_json::Value + preserve_order",
                serde_saphyr::from_str_with_options::<trees::Ordered>(matter, options())
                    .map(|v| trees::project_ordered(&v))
                    .map_err(|e| e.to_string()),
            ),
            (
                "saphyr::Yaml node tree",
                <saphyr::Yaml as saphyr::LoadableYamlNode>::load_from_str(matter)
                    .map_err(|e| e.to_string())
                    .and_then(|docs| match docs.into_iter().next() {
                        Some(doc) => Ok(trees::project_saphyr(&doc)),
                        None => Ok(vec!["$ key:- = null".to_owned()]),
                    }),
            ),
        ];
        for (tree, candidate) in candidates {
            match (&today, &candidate) {
                (Ok(a), Ok(b)) => {
                    let d = trees::compare(a, b);
                    if !d.is_empty() {
                        let class = classify_tree(&d);
                        kind.diff(case.clone(), &format!("read into {tree}"), class, d.clone(), "(see the - and + lines)");
                    }
                }
                (Ok(_), Err(e)) => kind.diff(
                    case.clone(),
                    &format!("read into {tree}"),
                    "norway accepts, candidate refuses",
                    "(accepted)",
                    e.clone(),
                ),
                (Err(e), Ok(_)) => kind.diff(
                    case.clone(),
                    &format!("read into {tree}"),
                    "norway refuses, candidate accepts",
                    e.clone(),
                    "(accepted)",
                ),
                (Err(a), Err(b)) if a == b => {}
                (Err(a), Err(b)) => kind.diff(
                    case.clone(),
                    &format!("error {tree}"),
                    "error text differs",
                    a.clone(),
                    b.clone(),
                ),
            }
        }
    }
    kind
}

/// Why two projections differ: order alone, key type, or a value.
fn classify_tree(diff: &str) -> String {
    let minus: Vec<&str> = diff.lines().filter(|l| l.starts_with('-')).map(|l| &l[1..]).collect();
    let plus: Vec<&str> = diff.lines().filter(|l| l.starts_with('+')).map(|l| &l[1..]).collect();
    let mut a = minus.clone();
    let mut b = plus.clone();
    a.sort_unstable();
    b.sort_unstable();
    if a == b {
        return "key order differs (same nodes)".to_owned();
    }
    let keytype = |l: &&str| l.split(" key:").nth(1).map(|r| r.split(' ').next().unwrap_or("").to_owned());
    if minus.iter().any(|l| keytype(l).is_some_and(|k| k != "str" && k != "-")) {
        return "a non-string key".to_owned();
    }
    let value = |l: &str| l.split(" = ").nth(1).unwrap_or("").split('(').next().unwrap_or("").to_owned();
    let types: Vec<String> = minus
        .iter()
        .zip(&plus)
        .filter(|(x, y)| value(x) != value(y))
        .map(|(x, y)| format!("{} → {}", value(x), value(y)))
        .collect();
    if types.is_empty() {
        "a scalar's value differs".to_owned()
    } else {
        format!("a scalar's type differs: {}", types.join(", "))
    }
}

// ---------------------------------------------------------------------------------------
// `--var` typed values.

fn typed_vars() -> Kind {
    let mut kind = Kind::new("typed --var values (TemplateVariable::parse, template/mod.rs)");
    let source = "---\nonetaskgraph_template: 1\nvariables:\n  \
v_integer: {description: d, type: integer, required: false}\n  \
v_boolean: {description: d, type: boolean, required: false}\n  \
v_list: {description: d, type: list, required: false}\n  \
v_object_list: {description: d, type: list, items: object, required: false}\n  \
v_object: {description: d, type: object, required: false}\n---\nbody\n";
    let template = TemplateLoader::new()
        .with_template("t.md", source)
        .load_name("t.md")
        .expect("the probe template loads");
    for text in synthetic::scalars() {
        for variable in template.variables() {
            kind.count("synthetic");
            let case = format!("--var {}={text:?}", variable.name());
            let (a, b) = both(|| variable.parse(&text).map(|v| v.to_string()));
            compare_read(&mut kind, &case, variable.name(), &a, &b);
        }
    }
    kind
}

// ---------------------------------------------------------------------------------------
// Answers files and stored answers blocks.

fn answers_read(text: &str) -> Result<String, String> {
    Answers::from_yaml(text)
        .map(|a| format!("{a:#?}"))
        .map_err(|e| e.to_string())
}

/// Every stored answers block in the store, as YAML text, with the file it is in.
fn stored_blocks(store: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (path, _) in store_files(store) {
        let text = fs::read_to_string(&path).expect("store file reads");
        let open = format!("{}\n", onetaskgraph_local_md::ANSWERS_OPEN);
        if let Some(at) = text.rfind(&open) {
            let rest = &text[at + open.len()..];
            if let Some(end) = rest.find(&format!("\n{}", onetaskgraph_local_md::ANSWERS_CLOSE)) {
                out.push((path.display().to_string(), rest[..=end].to_owned()));
            }
        }
    }
    out
}

fn answers(args: &Args) -> Kind {
    let mut kind = Kind::new("answers files and stored answers blocks (Answers::from_yaml, answers.rs)");
    for p in &args.answers {
        kind.count("real answers file");
        let text = fs::read_to_string(p).expect("answers reads");
        let (a, b) = both(|| answers_read(&text));
        compare_read(&mut kind, &p.display().to_string(), "Answers::from_yaml", &a, &b);
    }
    for (case, block) in stored_blocks(&args.store) {
        kind.count("real stored answers block");
        let (a, b) = both(|| answers_read(&block));
        compare_read(&mut kind, &case, "Answers::from_yaml", &a, &b);
    }
    for (name, text) in synthetic::answers() {
        kind.count("synthetic");
        let (a, b) = both(|| answers_read(&text));
        compare_read(&mut kind, &format!("synthetic: {name}"), "Answers::from_yaml", &a, &b);
    }
    kind
}

// ---------------------------------------------------------------------------------------
// local-md: a copy of the real store, read and written through the real plugin.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ItemKind {
    Task,
    Project,
    Document,
}

/// Every Markdown record of a store, with its kind.
fn store_files(store: &Path) -> Vec<(PathBuf, ItemKind)> {
    let mut out = Vec::new();
    for (dir, kind) in [
        ("tasks", ItemKind::Task),
        ("projects", ItemKind::Project),
        ("documents", ItemKind::Document),
    ] {
        walk(&store.join(dir), &mut |p| {
            if p.extension().is_some_and(|e| e == "md") {
                out.push((p.to_owned(), kind));
            }
        });
    }
    out.sort();
    out
}

fn walk(dir: &Path, f: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

fn copy_tree(from: &Path, to: &Path) {
    let _ = fs::remove_dir_all(to);
    for sub in ["tasks", "projects", "documents"] {
        let src = from.join(sub);
        walk(&src, &mut |p| {
            let rel = p.strip_prefix(from).expect("under the store");
            let dest = to.join(rel);
            fs::create_dir_all(dest.parent().expect("parent")).expect("mkdir");
            fs::copy(p, &dest).expect("copy");
        });
        fs::create_dir_all(to.join(sub)).expect("mkdir");
    }
}

fn source(root: &Path) -> Box<dyn TaskSource> {
    onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("spike").expect("a source name"),
            &serde_json::json!({ "root": root }),
            &NoSecrets,
        )
        .expect("local-md builds")
}

fn id_of(root: &Path, path: &Path, kind: ItemKind) -> NativeId {
    let dir = match kind {
        ItemKind::Task => "tasks",
        ItemKind::Project => "projects",
        ItemKind::Document => "documents",
    };
    let rel = path
        .strip_prefix(root.join(dir))
        .expect("under its directory")
        .with_extension("");
    NativeId(rel.to_string_lossy().replace('\\', "/"))
}

/// One record read through the plugin, as JSON, with its stored answers.
async fn read_item(
    src: &dyn TaskSource,
    id: &NativeId,
    kind: ItemKind,
) -> (Result<String, String>, Result<String, String>) {
    let item = match kind {
        ItemKind::Task => src.get_task(id).await.map(|t| serde_json::to_string_pretty(&t)),
        ItemKind::Project => src.get_project(id).await.map(|t| serde_json::to_string_pretty(&t)),
        ItemKind::Document => src.get_document(id).await.map(|t| serde_json::to_string_pretty(&t)),
    }
    .map(|r| r.expect("json renders"))
    .map_err(|e| e.to_string());
    let answers = match kind {
        ItemKind::Task => src.task_template_answers(id).await,
        ItemKind::Project => src.project_template_answers(id).await,
        ItemKind::Document => src.document_template_answers(id).await,
    }
    .map(|a| serde_json::to_string_pretty(&a).expect("json renders"))
    .map_err(|e| e.to_string());
    (item, answers)
}

struct Read {
    path: PathBuf,
    kind: ItemKind,
    id: NativeId,
    item: Value,
    answers: Option<BTreeMap<String, Value>>,
    edges: Vec<onetaskgraph_plugin_api::DependencyEdge>,
}

async fn local_md_reads(base: &Path) -> (Kind, Vec<Read>) {
    let mut kind = Kind::new(
        "local-md task, project and document front matter, stored answers blocks and metadata (reads)",
    );
    let src = source(base);
    let mut reads = Vec::new();
    for (path, item_kind) in store_files(base) {
        kind.count(match item_kind {
            ItemKind::Task => "real task",
            ItemKind::Project => "real project",
            ItemKind::Document => "real document",
        });
        let id = id_of(base, &path, item_kind);
        let case = path.strip_prefix(base).expect("under").display().to_string();
        select(Side::Norway);
        let (item_a, answers_a) = read_item(src.as_ref(), &id, item_kind).await;
        select(Side::Saphyr);
        let (item_b, answers_b) = read_item(src.as_ref(), &id, item_kind).await;
        compare_read(&mut kind, &case, "record", &item_a, &item_b);
        compare_read(&mut kind, &case, "stored answers", &answers_a, &answers_b);
        if let Ok(item) = &item_a {
            select(Side::Norway);
            let edges = if item_kind == ItemKind::Task {
                src.task_dependencies(
                    &id,
                    Direction::DependsOn,
                    &PageRequest {
                        cursor: None,
                        limit: 200,
                    },
                )
                .await
                .map(|p| p.items)
                .unwrap_or_default()
            } else {
                Vec::new()
            };
            let item: Value = serde_json::from_str(item).expect("json");
            if item.is_null() {
                continue;
            }
            reads.push(Read {
                path: path.clone(),
                kind: item_kind,
                id,
                item,
                answers: answers_a
                    .ok()
                    .and_then(|a| serde_json::from_str::<Option<BTreeMap<String, Value>>>(&a).ok())
                    .flatten(),
                edges,
            });
        }
    }
    (kind, reads)
}

async fn write_one(src: &dyn TaskSource, read: &Read) -> Result<NativeId, String> {
    let target = Some(read.id.clone());
    let r = match read.kind {
        ItemKind::Task => {
            let write = ItemWrite {
                target,
                item: serde_json::from_value(read.item.clone()).expect("a task"),
                depends_on: read.edges.clone(),
            };
            match &read.answers {
                Some(a) => src.write_task_rendered(&write, a).await,
                None => src.write_task(&write).await,
            }
        }
        ItemKind::Project => {
            let write = ItemWrite {
                target,
                item: serde_json::from_value(read.item.clone()).expect("a project"),
                depends_on: Vec::new(),
            };
            match &read.answers {
                Some(a) => src.write_project_rendered(&write, a).await,
                None => src.write_project(&write).await,
            }
        }
        ItemKind::Document => {
            let write = ItemWrite {
                target,
                item: serde_json::from_value(read.item.clone()).expect("a document"),
                depends_on: Vec::new(),
            };
            match &read.answers {
                Some(a) => src.write_document_rendered(&write, a).await,
                None => src.write_document(&write).await,
            }
        }
    };
    r.map_err(|e| e.to_string())
}

/// Every record read is written back over its own file in two copies of the store, one
/// copy per crate, and the two files compared byte for byte; then each is read back through
/// the crate that wrote it and the two readings compared.
async fn local_md_rewrites(args: &Args, reads: &[Read]) -> Kind {
    let mut kind = Kind::new(
        "local-md whole-file write (WrittenFrontMatter, `|+` block scalars, stored answers block) — every real record rewritten",
    );
    let norway_root = args.scratch.join("rewrite-norway");
    let saphyr_root = args.scratch.join("rewrite-saphyr");
    copy_tree(&args.store, &norway_root);
    copy_tree(&args.store, &saphyr_root);
    let norway = source(&norway_root);
    let saphyr = source(&saphyr_root);
    let mut identical_to_original = 0usize;
    for read in reads {
        kind.count(match (read.kind, read.answers.is_some()) {
            (ItemKind::Task, false) => "real task",
            (ItemKind::Task, true) => "real rendered task",
            (ItemKind::Project, false) => "real project",
            (ItemKind::Project, true) => "real rendered project",
            (ItemKind::Document, false) => "real document",
            (ItemKind::Document, true) => "real rendered document",
        });
        let rel = read.path.strip_prefix(&args.store).expect("under the store");
        let case = rel.display().to_string();
        select(Side::Norway);
        let a = write_one(norway.as_ref(), read).await;
        select(Side::Saphyr);
        let b = write_one(saphyr.as_ref(), read).await;
        let file_a = fs::read_to_string(norway_root.join(rel)).unwrap_or_default();
        let file_b = fs::read_to_string(saphyr_root.join(rel)).unwrap_or_default();
        if file_a == fs::read_to_string(&read.path).unwrap_or_default() {
            identical_to_original += 1;
        }
        match (&a, &b) {
            (Ok(_), Ok(_)) => {
                if file_a != file_b {
                    let d = line_diff(&file_a, &file_b);
                    kind.diff(&case, "write file bytes", shape(&d), d.clone(), "(see the - and + lines)");
                }
                // Each file read back through the crate that wrote it.
                select(Side::Norway);
                let (back_a, _) = read_item(norway.as_ref(), &read.id, read.kind).await;
                select(Side::Saphyr);
                let (back_b, _) = read_item(saphyr.as_ref(), &read.id, read.kind).await;
                let strip = |r: Result<String, String>| {
                    r.map(|s| {
                        // The location is each copy's own path.
                        let mut v: Value = serde_json::from_str(&s).expect("json");
                        if let Some(o) = v.as_object_mut() {
                            o.remove("location");
                            o.remove("version");
                        }
                        pretty(&v)
                    })
                };
                compare_read(&mut kind, &case, "read-back of what each wrote", &strip(back_a), &strip(back_b));
            }
            _ => compare_read(
                &mut kind,
                &case,
                "write",
                &a.map(|i| i.0),
                &b.map(|i| i.0),
            ),
        }
    }
    kind.notes.push(format!(
        "{identical_to_original} of {} records rewritten by serde_norway are byte-identical to the file they were read from (context, not a difference)",
        reads.len()
    ));
    kind
}

/// The `status:` and `title:` scalar splice: the real `set_task_status` over every real task,
/// and the scalar renderer the splice uses over every real title, status word, label and
/// project name and the synthetic scalars.
async fn splices(args: &Args, reads: &[Read]) -> Kind {
    let mut kind = Kind::new("title / status scalar splice (set_task_status, update's `scalar`)");
    let norway_root = args.scratch.join("splice-norway");
    let saphyr_root = args.scratch.join("splice-saphyr");
    copy_tree(&args.store, &norway_root);
    copy_tree(&args.store, &saphyr_root);
    let norway = source(&norway_root);
    let saphyr = source(&saphyr_root);
    for read in reads.iter().filter(|r| r.kind == ItemKind::Task) {
        kind.count("real task status write");
        let current = read.item["status"]["category"].as_str().unwrap_or("");
        let category = if current == "done" {
            StatusCategory::InProgress
        } else {
            StatusCategory::Done
        };
        let rel = read.path.strip_prefix(&args.store).expect("under");
        select(Side::Norway);
        let a = norway.set_task_status(&read.id, category).await;
        select(Side::Saphyr);
        let b = saphyr.set_task_status(&read.id, category).await;
        let file_a = fs::read_to_string(norway_root.join(rel)).unwrap_or_default();
        let file_b = fs::read_to_string(saphyr_root.join(rel)).unwrap_or_default();
        compare_read(
            &mut kind,
            &rel.display().to_string(),
            "set_task_status",
            &a.map(|s| format!("{s:?}")).map_err(|e| e.to_string()),
            &b.map(|s| format!("{s:?}")).map_err(|e| e.to_string()),
        );
        if file_a != file_b {
            let d = line_diff(&file_a, &file_b);
            kind.diff(rel.display().to_string(), "write status splice", shape(&d), d.clone(), "(see lines)");
        }
    }
    let mut scalars: Vec<(String, &str)> = Vec::new();
    for read in reads {
        if let Some(t) = read.item["title"].as_str() {
            scalars.push((t.to_owned(), "real title"));
        }
        if let Some(s) = read.item["status"]["name"].as_str() {
            scalars.push((s.to_owned(), "real status word"));
        }
        if let Some(labels) = read.item["labels"].as_array() {
            for l in labels {
                if let Some(n) = l["name"].as_str() {
                    scalars.push((n.to_owned(), "real label name"));
                }
            }
        }
    }
    for s in synthetic::scalars() {
        scalars.push((s, "synthetic"));
    }
    scalars.sort();
    scalars.dedup();
    for (text, class) in scalars {
        kind.count(&format!("{class} rendered as a scalar"));
        let (whole_a, whole_b) = both(|| {
            onetaskgraph_local_md::yaml_backend::to_string(text.as_str()).map_err(|e| e.to_string())
        });
        // What the splice writes: the rendering less its trailing line break.
        let trim = |r: &Result<String, String>| r.clone().map(|s| s.trim_end().to_owned());
        let (a, b) = (trim(&whole_a), trim(&whole_b));
        if a != b {
            kind.diff(
                format!("{class}: {text:?}"),
                "write scalar",
                scalar_class(&text, &a, &b),
                shown(&a),
                shown(&b),
            );
        }
        // What saphyr wrote, whole, has to read back as the text through either crate.
        if let Ok(written) = &whole_b {
            let (back_a, back_b) = both(|| {
                onetaskgraph_local_md::yaml_backend::from_str::<Value>(written)
                    .map_err(|e| e.to_string())
            });
            let want = Ok(Value::String(text.clone()));
            if back_a != want || back_b != want {
                kind.diff(
                    format!("{class}: {text:?}"),
                    "round trip of saphyr's scalar",
                    "saphyr's scalar does not read back as the text",
                    format!("saphyr wrote {written:?}; norway reads {back_a:?}"),
                    format!("saphyr reads {back_b:?}"),
                );
            }
        }
    }
    kind
}

fn scalar_class(_text: &str, a: &Result<String, String>, b: &Result<String, String>) -> String {
    let style = |r: &Result<String, String>| match r {
        Err(_) => "error".to_owned(),
        Ok(s) if s.starts_with('\'') => "single-quoted".to_owned(),
        Ok(s) if s.starts_with('"') => "double-quoted".to_owned(),
        Ok(s) if s.starts_with('|') => format!("literal block {}", s.lines().next().unwrap_or("")),
        Ok(s) if s.starts_with('>') => format!("folded block {}", s.lines().next().unwrap_or("")),
        Ok(_) => "plain".to_owned(),
    };
    format!("{} → {}", style(a), style(b))
}

/// The stored answers block and `TemplateAnswers::to_yaml`: every real stored answers map
/// and the synthetic maps, emitted by each crate, compared, and read back.
fn answers_writes(reads: &[Read]) -> Kind {
    let mut kind = Kind::new(
        "stored answers block write (local-md answers_block) and TemplateAnswers::to_yaml (engine/rendered.rs)",
    );
    let mut maps: Vec<(String, BTreeMap<String, Value>, &str)> = reads
        .iter()
        .filter_map(|r| {
            r.answers
                .clone()
                .map(|a| (r.path.display().to_string(), a, "real stored answers"))
        })
        .collect();
    for (name, map) in synthetic::answer_maps() {
        maps.push((format!("synthetic: {name}"), map, "synthetic"));
    }
    for (case, map, class) in maps {
        kind.count(class);
        let (a, b) = both(|| {
            onetaskgraph_local_md::yaml_backend::to_string(&map).map_err(|e| e.to_string())
        });
        let (ta, tb) = both(|| TemplateAnswers(map.clone()).to_yaml());
        if a != b {
            let d = line_diff(a.as_deref().unwrap_or(""), b.as_deref().unwrap_or(""));
            kind.diff(&case, "write answers_block", shape(&d), d.clone(), "(see lines)");
        }
        if ta != tb {
            let d = line_diff(ta.as_deref().unwrap_or(""), tb.as_deref().unwrap_or(""));
            kind.diff(&case, "write TemplateAnswers::to_yaml", shape(&d), d.clone(), "(see lines)");
        }
        // What saphyr wrote reads back as the same map, through either crate.
        if let Ok(written) = &b {
            let (back_a, back_b) = both(|| {
                onetaskgraph_local_md::yaml_backend::from_str::<BTreeMap<String, Value>>(written)
                    .map_err(|e| e.to_string())
            });
            for (who, back) in [("norway", back_a), ("saphyr", back_b)] {
                if back.as_ref() != Ok(&map) {
                    kind.diff(
                        &case,
                        "round trip of saphyr's answers block",
                        format!("{who} does not read saphyr's block back as the answers"),
                        pretty(&serde_json::to_value(&map).expect("json")),
                        format!("{who} reads: {back:?}\nsaphyr wrote:\n{written}"),
                    );
                }
            }
        }
    }
    kind
}

/// The metadata write: the read-back gate (`value: <compact json>`) over every real metadata
/// value and the synthetic ones, and the real `set_task_metadata` over every real task.
async fn metadata(args: &Args, reads: &[Read]) -> Kind {
    let mut kind = Kind::new("metadata write and read-back (local-md set_metadata / with_metadata_entry)");
    let mut values: Vec<(String, Value, &str)> = Vec::new();
    for read in reads {
        if let Some(m) = read.item["metadata"].as_object() {
            for (k, v) in m {
                values.push((format!("{}: {k}", read.path.display()), v.clone(), "real metadata value"));
            }
        }
    }
    for (name, v) in synthetic::metadata_values() {
        values.push((format!("synthetic: {name}"), v, "synthetic"));
    }
    for (case, value, class) in &values {
        kind.count(class);
        let compact = serde_json::to_string(value).expect("json");
        let (a, b) = both(|| {
            onetaskgraph_local_md::yaml_backend::from_str::<BTreeMap<String, Value>>(&format!(
                "value: {compact}"
            ))
            .map(|mut m| m.remove("value"))
            .map(|v| if v.as_ref() == Some(value) { "reads back as itself".to_owned() } else { format!("reads back as {v:?}") })
            .map_err(|e| e.to_string())
        });
        compare_read(&mut kind, case, "the `value: <json>` read-back gate", &a, &b);
    }

    let norway_root = args.scratch.join("metadata-norway");
    let saphyr_root = args.scratch.join("metadata-saphyr");
    copy_tree(&args.store, &norway_root);
    copy_tree(&args.store, &saphyr_root);
    let norway = source(&norway_root);
    let saphyr = source(&saphyr_root);
    let key = MetadataKey::try_from("spike.check".to_owned()).expect("a key");
    let probes = synthetic::metadata_values();
    for (i, read) in reads.iter().filter(|r| r.kind == ItemKind::Task).enumerate() {
        kind.count("real task metadata write");
        let (_, value) = &probes[i % probes.len()];
        let rel = read.path.strip_prefix(&args.store).expect("under");
        select(Side::Norway);
        let a = norway.set_task_metadata(&read.id, &key, value).await;
        select(Side::Saphyr);
        let b = saphyr.set_task_metadata(&read.id, &key, value).await;
        compare_read(
            &mut kind,
            &rel.display().to_string(),
            "set_task_metadata",
            &a.map(|_| "written".to_owned()).map_err(|e| e.to_string()),
            &b.map(|_| "written".to_owned()).map_err(|e| e.to_string()),
        );
        let file_a = fs::read_to_string(norway_root.join(rel)).unwrap_or_default();
        let file_b = fs::read_to_string(saphyr_root.join(rel)).unwrap_or_default();
        if file_a != file_b {
            let d = line_diff(&file_a, &file_b);
            kind.diff(rel.display().to_string(), "write metadata entry", shape(&d), d.clone(), "(see lines)");
        }
    }
    kind
}

/// Malformed records: unknown field, bad YAML, wrong types — the error each crate gives.
async fn errors(args: &Args) -> Kind {
    let mut kind = Kind::new("errors — malformed local-md records (unknown front-matter field and malformed YAML)");
    let root = args.scratch.join("errors");
    let _ = fs::remove_dir_all(&root);
    for sub in ["tasks", "projects", "documents"] {
        fs::create_dir_all(root.join(sub)).expect("mkdir");
    }
    let cases = synthetic::malformed_records();
    for (i, (_, dir, text)) in cases.iter().enumerate() {
        fs::write(root.join(dir).join(format!("e{i}.md")), text).expect("write");
    }
    let src = source(&root);
    for (i, (name, dir, _)) in cases.iter().enumerate() {
        kind.count("synthetic");
        let item_kind = match *dir {
            "tasks" => ItemKind::Task,
            "projects" => ItemKind::Project,
            _ => ItemKind::Document,
        };
        let id = NativeId(format!("e{i}"));
        select(Side::Norway);
        let (a, _) = read_item(src.as_ref(), &id, item_kind).await;
        select(Side::Saphyr);
        let (b, _) = read_item(src.as_ref(), &id, item_kind).await;
        compare_read(&mut kind, &format!("synthetic: {name}"), "record", &a, &b);
    }
    kind
}

// ---------------------------------------------------------------------------------------
// Parse time.

fn timing(args: &Args) -> String {
    let mut docs: Vec<(String, String)> = Vec::new();
    for (path, _) in store_files(&args.store) {
        let text = fs::read_to_string(&path).expect("reads");
        if let Some(m) = matter_of(&text) {
            docs.push((format!("front matter of {}", path.display()), m));
        }
    }
    for (case, block) in stored_blocks(&args.store) {
        docs.push((format!("stored answers block of {case}"), block));
    }
    for p in args.configs.iter().chain(&args.answers) {
        docs.push((p.display().to_string(), fs::read_to_string(p).expect("reads")));
    }
    let (name, doc) = docs
        .into_iter()
        .max_by_key(|(_, d)| d.len())
        .expect("a document");
    let rounds = 200;
    let time = |f: &dyn Fn()| {
        f();
        let mut samples: Vec<f64> = (0..rounds)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    };
    let norway_value = time(&|| {
        let _: Value = serde_norway::from_str(&doc).expect("parses");
    });
    let norway_tree = time(&|| {
        let _: serde_norway::Value = serde_norway::from_str(&doc).expect("parses");
    });
    let saphyr_value = time(&|| {
        let _: Value = serde_saphyr::from_str_with_options(&doc, saphyr_options(args.profile))
            .expect("parses");
    });
    let saphyr_tree = time(&|| {
        let _ = <saphyr::Yaml as saphyr::LoadableYamlNode>::load_from_str(&doc).expect("parses");
    });
    format!(
        "\n=== parse time of the largest real YAML document ({} bytes, {} lines): {name}\n  \
         median of {rounds} parses, release build:\n  \
         serde_norway → serde_json::Value      {norway_value:>9.1} µs\n  \
         serde_norway → serde_norway::Value    {norway_tree:>9.1} µs\n  \
         serde-saphyr → serde_json::Value      {saphyr_value:>9.1} µs\n  \
         saphyr 0.2   → saphyr::Yaml tree      {saphyr_tree:>9.1} µs\n",
        doc.len(),
        doc.lines().count()
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = args();
    apply(args.profile);
    fs::create_dir_all(&args.scratch).expect("scratch");
    fs::create_dir_all(&args.out).expect("out");
    // The store is read through a copy: nothing the plugin does touches the real one.
    let base = args.scratch.join("read-copy");
    copy_tree(&args.store, &base);

    let mut kinds = vec![configuration(&args), front_matter(&args), typed_vars(), answers(&args)];
    let (read_kind, reads) = local_md_reads(&base).await;
    kinds.push(read_kind);
    // Rewrites address the real store's paths, so reads are re-rooted on it.
    let reads: Vec<Read> = reads
        .into_iter()
        .map(|mut r| {
            r.path = args.store.join(r.path.strip_prefix(&base).expect("under"));
            r
        })
        .collect();
    kinds.push(local_md_rewrites(&args, &reads).await);
    kinds.push(splices(&args, &reads).await);
    kinds.push(answers_writes(&reads));
    kinds.push(metadata(&args, &reads).await);
    kinds.push(errors(&args).await);

    let mut out = String::new();
    out.push_str(&format!(
        "saphyr-diff: serde_norway {} vs serde-saphyr {} (saphyr {} for the node tree), profile {:?}{}\n",
        "0.9.42",
        "1.3.0",
        "0.2.1",
        args.profile,
        match args.profile {
            Profile::Default => " — Options::default(), SerializerOptions::default()".to_owned(),
            Profile::Compat => " — Options { strict_booleans: true, merge_keys: AsOrdinary, duplicate_keys: LastWins, with_snippet: false }, SerializerOptions { folded_wrap_chars: usize::MAX }".to_owned(),
        }
    ));
    for kind in &kinds {
        out.push_str(&kind.render());
    }
    out.push_str(&timing(&args));
    print!("{out}");
    fs::write(args.out.join("report.txt"), &out).expect("write report");
    fs::write(
        args.out.join("diffs.json"),
        serde_json::to_string_pretty(&kinds).expect("json"),
    )
    .expect("write json");
}
