//! `task update`: one targeted update of one task, read off the command line.
//!
//! What lives here is what only a command line has — the flags and the file a body is read
//! from. The update itself is the engine's (`onetaskgraph_core::Engine::update_task`), so a
//! Rust caller and this binary cannot update a task differently.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use onetaskgraph_core::{Failure, GlobalId, Loaded};
use onetaskgraph_plugin_api::{
    DependencyEdge, DependencyEndpoint, DependencyKind, EndpointRefusal, ItemKind, MetadataKey,
    Status, TaskRef, TaskUpdate,
};

use crate::cli::TaskUpdateArgs;
use crate::rendered::metadata_entry;
use crate::template::refused;
use crate::{delivery_exit, emit, expecting, qualified, render, rendering, write_failed};

/// `task update`: apply the update, then write what it came to — and exit `4` when a task it
/// delivers could not be kept in step with it.
pub(crate) async fn update_task(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &TaskUpdateArgs,
) -> Result<u8, Failure> {
    // Every flag is read, and refused, before anything is built or asked.
    let id = qualified(&args.id)?;
    let update = request(&id, args)?;
    if update.is_empty() {
        return Ok(refused(
            "task update names no field to write\n\
             next: name at least one of --title, --body-file, --status, --priority, --metadata, \
             --remove-metadata, --delivers, --no-delivers, --depends-on or --no-depends-on — \
             `onetaskgraph task update --help` describes each.",
        ));
    }
    let engine = expecting(loaded, &id, &args.expect)?;
    let updated = engine
        .update_task(&id, &update)
        .await
        .map_err(|error| write_failed(&engine, &error))?;
    let rendered = rendering(loaded, &updated, render::task_updated, "the update")?;
    emit(out, rendered.trim_end(), "the update")?;
    Ok(delivery_exit(&updated.delivered))
}

/// The update the flags name: each field named once, and nothing for a field left out.
fn request(id: &GlobalId, args: &TaskUpdateArgs) -> Result<TaskUpdate, Failure> {
    let status = args.status.map(|category| {
        let category = category.category();
        Status {
            category,
            name: args.status_name.clone().unwrap_or_else(|| {
                serde_json::to_value(category)
                    .ok()
                    .and_then(|word| word.as_str().map(str::to_owned))
                    .expect("a status category serialises as its own word")
            }),
        }
    });
    let metadata_set = args
        .metadata
        .iter()
        .map(|entry| metadata_entry(entry))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let metadata_remove = args
        .remove_metadata
        .iter()
        .map(|key| {
            MetadataKey::new(key.as_str()).map_err(|message| {
                Failure::decided(
                    "invalid-metadata-key",
                    format!("--remove-metadata {key}: {message}"),
                )
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let delivers = list(&args.delivers, args.no_delivers)?.map(|tasks| {
        tasks
            .iter()
            .map(|task| TaskRef::qualified(&task.source, &task.native))
            .collect()
    });
    let depends_on = if args.no_depends_on {
        Some(Vec::new())
    } else if args.depends_on.is_empty() {
        None
    } else {
        Some(
            args.depends_on
                .iter()
                .map(|given| {
                    Ok(DependencyEdge {
                        from: DependencyEndpoint::from_native(id.native.clone(), ItemKind::Task),
                        to: far_end(given)?,
                        kind: DependencyKind::Blocks,
                    })
                })
                .collect::<Result<_, Failure>>()?,
        )
    };
    Ok(TaskUpdate {
        title: args.title.clone(),
        content: args.body_file.as_deref().map(body).transpose()?,
        status,
        priority: args.priority.map(|priority| priority.priority()),
        metadata_set,
        metadata_remove,
        delivers,
        depends_on,
    })
}

/// One `--depends-on`: a qualified task id, or a dependency endpoint object — `{"id":
/// "<source>:<native>", "kind": "task"}`, or the opaque `{"designator", "id", "kind"}` — whose
/// keys are refused by name, never by value, when they are any beyond those three.
fn far_end(given: &str) -> Result<DependencyEndpoint, Failure> {
    if !given.trim_start().starts_with('{') {
        let far = qualified(given)?;
        return Ok(DependencyEndpoint::new(far.to_string(), ItemKind::Task)
            .unwrap_or_else(|_| DependencyEndpoint::from_native(far.native, ItemKind::Task)));
    }
    let object: serde_json::Value = serde_json::from_str(given).map_err(|_| {
        Failure::decided(
            "invalid-dependency",
            "--depends-on: an argument starting with `{` is not a JSON object\n\
             next: give a qualified task id, or a JSON object of `designator`, `id` and `kind`.",
        )
    })?;
    let endpoint = DependencyEndpoint::from_value(&object).map_err(|refusal| match refusal {
        EndpointRefusal::Key(key) => Failure::decided(
            "private-reference-content",
            format!(
                "--depends-on: the dependency endpoint object carries `{key}`, which would \
                 write what the item it names is, so nothing was written\n\
                 next: give only `designator`, `id` and `kind`, or the qualified native id \
                 `<source>:<native id>`."
            ),
        ),
        EndpointRefusal::Malformed(_) => Failure::decided(
            "invalid-dependency",
            "--depends-on: the object is no dependency endpoint\n\
             next: give `{\"id\": \"<source>:<native id>\", \"kind\": \"task\"}` or a qualified \
             task id.",
        ),
    })?;
    if endpoint.is_native() {
        return Err(Failure::decided(
            "invalid-dependency",
            "--depends-on: an endpoint object's `id` is qualified, `<source>:<native id>`\n\
             next: qualify it with the source that holds the task.",
        ));
    }
    Ok(endpoint)
}

/// One replaceable list of qualified task ids: the ids given, none at all under its `--no-…`
/// flag, or not named when neither was given.
fn list(given: &[String], none: bool) -> Result<Option<Vec<GlobalId>>, Failure> {
    if none {
        return Ok(Some(Vec::new()));
    }
    if given.is_empty() {
        return Ok(None);
    }
    given
        .iter()
        .map(|id| qualified(id))
        .collect::<Result<_, _>>()
        .map(Some)
}

/// A task's new content, read byte for byte from `--body-file`.
fn body(path: &std::path::Path) -> Result<String, Failure> {
    let bytes = std::fs::read(path).map_err(|error| {
        Failure::decided(
            "body-file",
            format!(
                "--body-file {}: could not read it: {error}\n\
                 next: name a readable file holding the task's new content.",
                path.display()
            ),
        )
    })?;
    String::from_utf8(bytes).map_err(|error| {
        Failure::decided(
            "body-file",
            format!(
                "the content in --body-file {} is not UTF-8 text: {error}\n\
                 next: save the content as UTF-8 and pass it again.",
                path.display()
            ),
        )
    })
}
