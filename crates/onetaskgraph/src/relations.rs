//! `task parent set-many`, `task link add-many`, `task link remove-many` and `task relations`:
//! the relation calls, read off the command line.
//!
//! What lives here is what only a command line has — the file or standard input a batch is read
//! from, the text rendering and the exit each call amounts to. Each call itself is the engine's
//! (`onetaskgraph_core::Engine::set_parents`, `add_links`, `remove_links`, `task_relations`).

use std::io::Write;

use onetaskgraph_core::{
    Failure, GlobalId, LinkBatch, LinkOp, LinksWritten, Loaded, OtherEnd, ParentBatch, ParentsSet,
    TaskRelations,
};
use onetaskgraph_plugin_api::LinkDirection;

use crate::batch::document;
use crate::cli::RelationBatchArgs;
use crate::{EXIT_OK, EXIT_PARTIAL, emit, engine, rendering};

/// `task parent set-many`.
pub(crate) async fn set_many(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &RelationBatchArgs,
) -> Result<u8, Failure> {
    let batch: ParentBatch = document(&args.input, "--input", "ParentBatch")?;
    let set = engine(loaded)
        .set_parents(&batch)
        .await
        .map_err(|error| Failure::from(&error))?;
    let rendered = rendering(loaded, &set, parents_text, "the parents")?;
    emit(out, rendered.trim_end(), "the parents")?;
    Ok(
        if set.results.iter().all(|result| result.outcome.settled()) {
            EXIT_OK
        } else {
            EXIT_PARTIAL
        },
    )
}

/// `task link add-many` and `task link remove-many`.
pub(crate) async fn links(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &RelationBatchArgs,
    op: LinkOp,
) -> Result<u8, Failure> {
    let batch: LinkBatch = document(&args.input, "--input", "LinkBatch")?;
    let engine = engine(loaded);
    let written = match op {
        LinkOp::Add => engine.add_links(&batch).await,
        LinkOp::Remove => engine.remove_links(&batch).await,
    }
    .map_err(|error| Failure::from(&error))?;
    let rendered = rendering(loaded, &written, links_text, "the links")?;
    emit(out, rendered.trim_end(), "the links")?;
    Ok(
        if written
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

/// `task relations`.
pub(crate) async fn read(
    out: &mut impl Write,
    loaded: &Loaded,
    ids: &[GlobalId],
    complete: bool,
) -> Result<u8, Failure> {
    let relations = engine(loaded)
        .task_relations(ids, complete)
        .await
        .map_err(|error| Failure::from(&error))?;
    let rendered = rendering(loaded, &relations, relations_text, "the relations")?;
    emit(out, rendered.trim_end(), "the relations")?;
    Ok(EXIT_OK)
}

/// A wire word — an outcome or an end — as the machine rendering spells it.
fn word(value: impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|word| word.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn parents_text(set: &ParentsSet) -> String {
    set.results
        .iter()
        .map(|result| {
            let detail = result
                .reason
                .clone()
                .or_else(|| result.version.clone().map(|at| format!("version {at}")))
                .unwrap_or_default();
            format!(
                "{} → {}  {}  {detail}\n",
                result.child,
                result.parent.as_deref().unwrap_or("(none)"),
                word(result.outcome)
            )
        })
        .collect()
}

fn links_text(written: &LinksWritten) -> String {
    written
        .results
        .iter()
        .map(|result| {
            format!(
                "{} -[{}]-> {}  {}  {}\n",
                result.from,
                result.link_type,
                result.to,
                word(result.outcome),
                result.reason.clone().unwrap_or_default()
            )
        })
        .collect()
}

fn relations_text(relations: &TaskRelations) -> String {
    let mut text = String::new();
    for item in &relations.items {
        text.push_str(&format!("{}\n", item.item));
        text.push_str(&format!(
            "  parent: {}\n",
            item.parent
                .as_ref()
                .map_or_else(|| "(none)".to_owned(), ToString::to_string)
        ));
        text.push_str(&format!(
            "  children{}: {}\n",
            if item.children_complete {
                ""
            } else {
                " (incomplete)"
            },
            if item.children.is_empty() {
                "(none)".to_owned()
            } else {
                item.children
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
        for link in &item.links {
            let arrow = match link.direction {
                LinkDirection::Out => "→",
                LinkDirection::In => "←",
            };
            let end = match link.other_end {
                OtherEnd::Present => String::new(),
                other => format!("  (other end {})", word(other)),
            };
            text.push_str(&format!(
                "  {} {arrow} {}{end}\n",
                link.link_type, link.other
            ));
        }
    }
    text
}
