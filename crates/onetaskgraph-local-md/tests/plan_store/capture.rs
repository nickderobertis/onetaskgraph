//! What this source reads out of a store's records, and the bytes it writes over them, as one
//! JSON document per record — the observation both halves of `plan_store.rs` compare against
//! what the release before the YAML crate moved observed of the same records.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use onetaskgraph_plugin_api::{
    Direction, ItemWrite, NativeId, PageRequest, SecretResolver, SourceName, SourcePlugin, Status,
    StatusCategory, TaskSource, TaskUpdate,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

const FOLDERS: [&str; 3] = ["tasks", "projects", "documents"];

/// The title a task's title is spliced to and back from, so the splice writes the real title.
const PLACEHOLDER_TITLE: &str = "placeholder";

/// The source over the store at `root`.
pub fn source(root: &Path) -> Box<dyn TaskSource> {
    onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("plans").expect("a source name"),
            &json!({ "root": root }),
            &NoSecrets,
        )
        .expect("the store builds")
}

/// Every record file under `root`, as its path relative to `root` beside its folder.
pub fn records(root: &Path) -> Vec<(String, &'static str)> {
    let mut found = Vec::new();
    for folder in FOLDERS {
        walk(&root.join(folder), &mut |path| {
            if path.extension().is_some_and(|extension| extension == "md") {
                let relative = path.strip_prefix(root).expect("under the store");
                found.push((relative.to_string_lossy().replace('\\', "/"), folder));
            }
        });
    }
    found.sort();
    found
}

fn walk(folder: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|entry| entry.expect("an entry").path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}

/// A copy of the store at `from`, at `to`, which every write lands in.
pub fn copy_store(from: &Path, to: &Path) {
    for (relative, _) in records(from) {
        let target = to.join(&relative);
        fs::create_dir_all(target.parent().expect("a record has a folder")).expect("folders");
        fs::copy(from.join(&relative), target).expect("a record copies");
    }
    for folder in FOLDERS {
        fs::create_dir_all(to.join(folder)).expect("a folder");
    }
}

/// The id the record at `relative` is read under: its path in its folder, less `.md`.
fn id_of(relative: &str, folder: &str) -> NativeId {
    let within = relative
        .strip_prefix(folder)
        .and_then(|rest| rest.strip_prefix('/'))
        .expect("under its folder");
    NativeId(
        within
            .strip_suffix(".md")
            .expect("a record file")
            .to_owned(),
    )
}

fn shown<T: serde::Serialize, E: std::fmt::Display>(result: Result<T, E>) -> Value {
    match result {
        Ok(value) => serde_json::to_value(value).expect("renders as JSON"),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

/// What a read of the record answers: the item, its stored answers and, for a task, its
/// forward edges — with the store's own path spelled `<store>` and its separators `/`, so
/// two copies compare.
pub async fn read(source: &dyn TaskSource, root: &Path, relative: &str, folder: &str) -> Value {
    let id = id_of(relative, folder);
    let (item, answers) = match folder {
        "tasks" => (
            shown(source.get_task(&id).await),
            shown(source.task_template_answers(&id).await),
        ),
        "projects" => (
            shown(source.get_project(&id).await),
            shown(source.project_template_answers(&id).await),
        ),
        _ => (
            shown(source.get_document(&id).await),
            shown(source.document_template_answers(&id).await),
        ),
    };
    let depends_on = if folder == "tasks" {
        let page = PageRequest {
            cursor: None,
            limit: 200,
        };
        shown(
            source
                .task_dependencies(&id, Direction::DependsOn, &page)
                .await
                .map(|page| page.items),
        )
    } else {
        Value::Null
    };
    let observed = json!({ "item": item, "answers": answers, "depends_on": depends_on });
    let spelled = serde_json::to_string(&observed).expect("renders");
    let root = serde_json::to_string(&root.to_string_lossy()).expect("renders");
    let root = &root[1..root.len() - 1];
    let mut observed = serde_json::from_str(&spelled.replace(root, "<store>")).expect("still JSON");
    separated_by_slashes(&mut observed);
    observed
}

/// Spells every path under `<store>` with `/`, so a store read on Windows compares too.
fn separated_by_slashes(value: &mut Value) {
    match value {
        Value::String(text) if text.starts_with("<store>") => *text = text.replace('\\', "/"),
        Value::Array(items) => items.iter_mut().for_each(separated_by_slashes),
        Value::Object(fields) => fields.values_mut().for_each(separated_by_slashes),
        _ => {}
    }
}

/// What every record of the store at `root` reads as.
pub async fn reads(root: &Path) -> BTreeMap<String, Value> {
    let root = fs::canonicalize(root).expect("the store exists");
    let source = source(&root);
    let mut observed = BTreeMap::new();
    for (relative, folder) in records(&root) {
        let reading = read(source.as_ref(), &root, &relative, folder).await;
        observed.insert(relative, reading);
    }
    observed
}

/// The record at `relative` written whole over itself, from what a read of it answered.
async fn rewrite(source: &dyn TaskSource, read: &Value, relative: &str, folder: &str) -> Value {
    let target = Some(id_of(relative, folder));
    let answers: Option<BTreeMap<String, Value>> =
        serde_json::from_value(read["answers"].clone()).expect("answers read back");
    let item = read["item"].clone();
    let written = match folder {
        "tasks" => {
            let write = ItemWrite {
                target,
                item: serde_json::from_value(item).expect("a task"),
                depends_on: serde_json::from_value(read["depends_on"].clone()).expect("edges"),
            };
            match &answers {
                Some(answers) => source.write_task_rendered(&write, answers).await,
                None => source.write_task(&write).await,
            }
        }
        "projects" => {
            let write = ItemWrite {
                target,
                item: serde_json::from_value(item).expect("a project"),
                depends_on: Vec::new(),
            };
            match &answers {
                Some(answers) => source.write_project_rendered(&write, answers).await,
                None => source.write_project(&write).await,
            }
        }
        _ => {
            let write = ItemWrite {
                target,
                item: serde_json::from_value(item).expect("a document"),
                depends_on: Vec::new(),
            };
            match &answers {
                Some(answers) => source.write_document_rendered(&write, answers).await,
                None => source.write_document(&write).await,
            }
        }
    };
    match written {
        Ok(_) => Value::Null,
        Err(error) => json!({ "error": error.to_string() }),
    }
}

/// A task's status set to another category, then its title spliced away and back together
/// with its own status word, so both scalars are the splice's rendering of the real values.
async fn splice(
    source: &dyn TaskSource,
    root: &Path,
    read: &Value,
    relative: &str,
) -> (Value, String, Value) {
    let id = id_of(relative, "tasks");
    let status: Status = serde_json::from_value(read["item"]["status"].clone()).expect("a status");
    let other = if status.category == StatusCategory::Done {
        StatusCategory::InProgress
    } else {
        StatusCategory::Done
    };
    let set = shown(source.set_task_status(&id, other).await);
    let path = root.join(relative);
    let after_set = fs::read_to_string(&path).expect("the record reads");
    let title = read["item"]["title"].as_str().expect("a title").to_owned();
    let away = TaskUpdate {
        title: Some(PLACEHOLDER_TITLE.to_owned()),
        ..TaskUpdate::default()
    };
    let back = TaskUpdate {
        title: Some(title),
        status: Some(status),
        ..TaskUpdate::default()
    };
    let updated = match source.update_task(&id, &away).await {
        Ok(_) => shown(source.update_task(&id, &back).await.map(|_| ())),
        Err(error) => json!({ "error": error.to_string() }),
    };
    (set, after_set, updated)
}

/// The bytes each record of the store at `store` holds after a whole rewrite, after a status
/// write, and after a status write and a title-and-status splice back — each in a copy of the
/// store under `scratch`, which this creates.
pub async fn writes(store: &Path, scratch: &Path) -> BTreeMap<String, Value> {
    let rewritten = scratch.join("rewritten");
    let spliced = scratch.join("spliced");
    copy_store(store, &rewritten);
    copy_store(store, &spliced);
    let rewritten = fs::canonicalize(rewritten).expect("the copy exists");
    let spliced = fs::canonicalize(spliced).expect("the copy exists");
    let rewriting = source(&rewritten);
    let splicing = source(&spliced);
    let mut observed = BTreeMap::new();
    for (relative, folder) in records(store) {
        let reading = read(rewriting.as_ref(), &rewritten, &relative, folder).await;
        if reading["item"].get("error").is_some() || reading["item"].is_null() {
            observed.insert(relative, json!({ "unread": reading["item"] }));
            continue;
        }
        let outcome = rewrite(rewriting.as_ref(), &reading, &relative, folder).await;
        let mut record = json!({
            "rewrite": outcome,
            "rewritten": fs::read_to_string(rewritten.join(&relative)).expect("the record reads"),
        });
        if folder == "tasks" {
            let (set, after_set, updated) =
                splice(splicing.as_ref(), &spliced, &reading, &relative).await;
            let file = fs::read_to_string(spliced.join(&relative)).expect("the record reads");
            record["status_set"] = set;
            record["status_written"] = Value::String(after_set);
            record["spliced_back"] = updated;
            record["spliced"] = Value::String(file);
        }
        observed.insert(relative, record);
    }
    observed
}
