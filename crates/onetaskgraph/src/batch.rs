//! `task find`, `task update-many` and `task comment add-many`: the batched calls, read off the
//! command line.
//!
//! What lives here is what only a command line has — the file or standard input a batch is read
//! from, and the exit it amounts to. Each call itself is the engine's
//! (`onetaskgraph_core::Engine::find_tasks`, `add_comments`, `update_tasks`), so a Rust caller,
//! an SDK and this binary cannot answer one batch differently.

use std::io::{self, Write};
use std::path::Path;

use onetaskgraph_core::{
    CommentBatch, CommentsAdded, Failure, FindRequest, FoundTasks, Loaded, TasksUpdated,
    UpdateBatch,
};
use serde::de::DeserializeOwned;

use crate::cli::{TaskBatchArgs, TaskFindArgs};
use crate::{EXIT_OK, EXIT_PARTIAL, EXIT_USAGE, emit, engine, rendering, usable_source};

/// The document at `path`, or on standard input for `-`, as `T` — refused before any source is
/// started when it cannot be read or is not one.
fn document<T: DeserializeOwned>(path: &Path, flag: &str, name: &str) -> Result<T, Failure> {
    let (bytes, from) = if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut io::stdin().lock(), &mut bytes).map_err(|error| {
            Failure::decided(
                "invalid-batch",
                format!(
                    "could not read the {name} from standard input: {error}\n\
                     next: pass it with {flag} FILE instead."
                ),
            )
        })?;
        (bytes, "standard input".to_owned())
    } else {
        let bytes = std::fs::read(path).map_err(|error| {
            Failure::decided(
                "invalid-batch",
                format!(
                    "{flag} {}: could not read it: {error}\n\
                     next: name a readable file holding the {name}.",
                    path.display()
                ),
            )
        })?;
        (bytes, format!("{flag} {}", path.display()))
    };
    serde_json::from_slice(&bytes).map_err(|error| {
        Failure::decided(
            "invalid-batch",
            format!(
                "{from} is not a {name}: {error}, so nothing was read or written\n\
                 next: send exactly the members `onetaskgraph schema` publishes for it."
            ),
        )
    })
}

/// `task find`.
pub(crate) async fn find(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &TaskFindArgs,
) -> Result<u8, Failure> {
    let request: FindRequest = document(&args.queries, "--queries", "FindRequest")?;
    let Some(source) = usable_source(&args.source, loaded) else {
        return Ok(EXIT_USAGE);
    };
    let found = engine(loaded)
        .find_tasks(&source, &request, loaded.config.page_size())
        .await
        .map_err(|error| Failure::from(&error))?;
    let rendered = rendering(loaded, &found, found_text, "the searches")?;
    emit(out, rendered.trim_end(), "the searches")?;
    Ok(if found.answers.iter().all(|answer| answer.complete) {
        EXIT_OK
    } else {
        EXIT_PARTIAL
    })
}

/// `task update-many`.
pub(crate) async fn update_many(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &TaskBatchArgs,
) -> Result<u8, Failure> {
    let batch: UpdateBatch = document(&args.input, "--input", "UpdateBatch")?;
    let Some(source) = usable_source(&args.source, loaded) else {
        return Ok(EXIT_USAGE);
    };
    let updated = engine(loaded)
        .update_tasks(&source, &batch)
        .await
        .map_err(|error| Failure::from(&error))?;
    let rendered = rendering(loaded, &updated, updated_text, "the updates")?;
    emit(out, rendered.trim_end(), "the updates")?;
    Ok(
        if updated
            .results
            .iter()
            .all(|result| result.outcome.settled())
        {
            EXIT_OK
        } else {
            EXIT_PARTIAL
        },
    )
}

/// `task comment add-many`.
pub(crate) async fn add_many(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &TaskBatchArgs,
) -> Result<u8, Failure> {
    let batch: CommentBatch = document(&args.input, "--input", "CommentBatch")?;
    let Some(source) = usable_source(&args.source, loaded) else {
        return Ok(EXIT_USAGE);
    };
    let added = engine(loaded)
        .add_comments(&source, &batch)
        .await
        .map_err(|error| Failure::from(&error))?;
    let rendered = rendering(loaded, &added, added_text, "the comments")?;
    emit(out, rendered.trim_end(), "the comments")?;
    Ok(
        if added.results.iter().all(|result| result.outcome.settled()) {
            EXIT_OK
        } else {
            EXIT_PARTIAL
        },
    )
}

fn found_text(found: &FoundTasks) -> String {
    let mut text = String::new();
    for answer in &found.answers {
        text.push_str(&format!(
            "{}: {} task(s){}\n",
            answer.key,
            answer.items.len(),
            answer
                .reason
                .as_ref()
                .map_or_else(String::new, |reason| format!(" — incomplete: {reason}"))
        ));
        for item in &answer.items {
            text.push_str(&format!("  {}  {}\n", item.id, item.item.title));
        }
    }
    text
}

fn outcome_word(outcome: onetaskgraph_core::BatchOutcome) -> String {
    serde_json::to_value(outcome)
        .ok()
        .and_then(|word| word.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn updated_text(updated: &TasksUpdated) -> String {
    updated
        .results
        .iter()
        .map(|result| {
            let detail = result
                .reason
                .clone()
                .or_else(|| {
                    result
                        .current_version
                        .clone()
                        .map(|at| format!("now at {at}"))
                })
                .or_else(|| result.version.clone().map(|at| format!("version {at}")))
                .unwrap_or_default();
            format!(
                "{}  {}  {detail}\n",
                result.item,
                outcome_word(result.outcome)
            )
        })
        .collect()
}

fn added_text(added: &CommentsAdded) -> String {
    added
        .results
        .iter()
        .map(|result| {
            let detail = result
                .reason
                .clone()
                .or_else(|| {
                    result
                        .current_version
                        .clone()
                        .map(|at| format!("now at {at}"))
                })
                .or_else(|| {
                    result
                        .comment
                        .as_ref()
                        .map(|comment| format!("comment {}", comment.id))
                })
                .unwrap_or_default();
            format!(
                "{}  {}  {detail}\n",
                result.item,
                outcome_word(result.outcome)
            )
        })
        .collect()
}
