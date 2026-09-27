//! `task create`, `document create`, `task render`, `document render`, `task answers` and
//! `document answers`: items created from a body or a template, regenerated in place, and the
//! answers stored beside them.
//!
//! What lives here is what only a command line has — the flags, the files and standard input
//! they are read from, and the prompts. Creating, regenerating and reading answers are the
//! engine's (`onetaskgraph_core::Engine::create_task` and its siblings), so a Rust caller and
//! this binary cannot do them differently.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read as _, Write};
use std::path::Path;
use std::str::FromStr as _;

use onetaskgraph_core::{
    Body, DocumentCreate, EngineError, Failure, GlobalId, Loaded, OutputFormat, RenderRequest,
    RenderedRecord, TaskCreate,
};
use onetaskgraph_plugin_api::{MetadataKey, NativeId, Repository, SourceName};
use serde_json::Value;

use crate::cli::{
    AnswersArgs, CreateBodyArgs, CreateItemArgs, DocumentCreateArgs, RenderArgs, TaskCreateArgs,
};
use crate::template::{Refusal, answers_given, ask, input, refused};
use crate::{EXIT_OK, delivery_exit, emit, engine, json, qualified, render};

/// `task create`: create the task, then write its qualified id — or, under `--json`, the task
/// exactly as `task show --json` writes it.
pub(crate) async fn create_task(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &TaskCreateArgs,
) -> Result<u8, Failure> {
    let item = match item(&args.item, loaded) {
        Ok(item) => item,
        Err(Refusal::Answers(message)) => return Ok(refused(&message)),
        Err(Refusal::Failed(failure)) => return Err(failure),
    };
    let depends_on = ids(&args.depends_on)?;
    let delivers = ids(&args.delivers)?;
    let engine = engine(loaded);
    let created = engine
        .create_task(&TaskCreate {
            source: item.source,
            project: item.project,
            title: args.item.title.clone(),
            body: item.body,
            status: args.status.map(|status| status.category()),
            labels: args.item.label.clone(),
            repositories: item.repositories,
            depends_on,
            delivers,
            metadata: item.metadata,
        })
        .await
        .map_err(|error| Failure::from(&error))?;
    let id = created.task.id;
    match loaded.config.output() {
        OutputFormat::Text => emit(out, &id.to_string(), "the task's id")?,
        OutputFormat::Json => {
            let detail = engine
                .task_detail(&id)
                .await
                .map_err(|error| Failure::from(&error))?;
            emit(out, &json(&detail, "the task")?, "the task")?;
        }
    }
    Ok(delivery_exit(&created.delivered))
}

/// `document create`: create or replace the document, then write its qualified id — or,
/// under `--json`, the document exactly as `document show --json` writes it.
pub(crate) async fn create_document(
    out: &mut impl Write,
    loaded: &Loaded,
    args: &DocumentCreateArgs,
) -> Result<u8, Failure> {
    let item = match item(&args.item, loaded) {
        Ok(item) => item,
        Err(Refusal::Answers(message)) => return Ok(refused(&message)),
        Err(Refusal::Failed(failure)) => return Err(failure),
    };
    let engine = engine(loaded);
    let created = engine
        .create_document(&DocumentCreate {
            source: item.source,
            project: item.project,
            title: args.item.title.clone(),
            id: args.id.clone(),
            body: item.body,
            labels: args.item.label.clone(),
            repositories: item.repositories,
            metadata: item.metadata,
        })
        .await
        .map_err(|error| Failure::from(&error))?;
    match loaded.config.output() {
        OutputFormat::Text => emit(out, &created.id.to_string(), "the document's id")?,
        OutputFormat::Json => {
            let response = engine
                .document(&created.id)
                .await
                .map_err(|error| Failure::from(&error))?;
            emit(out, &json(&response, "the document")?, "the document")?;
        }
    }
    Ok(EXIT_OK)
}

/// `task render` and `document render`: regenerate the item in place, asking — when
/// interactive — for what its base and the given answers leave unanswered.
///
/// A refusal of the answers — required variables left unanswered, an answer to no declared
/// variable or of the wrong type — is the invocation's mistake, and exits `2`.
pub(crate) async fn regenerate(
    out: &mut impl Write,
    loaded: &Loaded,
    record: RenderedRecord,
    args: &RenderArgs,
) -> Result<u8, Failure> {
    let id = qualified(&args.id)?;
    match regenerated(out, loaded, record, &id, args).await {
        Ok(()) => Ok(EXIT_OK),
        Err(Refusal::Answers(message)) => Ok(refused(&message)),
        Err(Refusal::Failed(failure)) => Err(failure),
    }
}

async fn regenerated(
    out: &mut impl Write,
    loaded: &Loaded,
    record: RenderedRecord,
    id: &GlobalId,
    args: &RenderArgs,
) -> Result<(), Refusal> {
    let template = input(
        args.template.as_deref(),
        &args.search_path,
        args.template_loader.as_deref(),
        args.answers.as_deref(),
    )?;
    let engine = engine(loaded);
    let regeneration = engine
        .regeneration(
            record,
            id,
            &RenderRequest {
                template,
                search_path: args.search_path.clone(),
                ..RenderRequest::default()
            },
        )
        .await
        .map_err(refusal)?;
    let mut given = answers_given(args.answers.as_deref(), &args.var)?;
    for name in &args.unset {
        given.unset(name.clone());
    }
    // Nothing the base settles, and nothing a caller unset, is asked for again: the first is
    // this item's own answer, and the second is a caller asking for the default.
    let settled: HashSet<String> = regeneration
        .settled()
        .map(str::to_owned)
        .chain(args.unset.iter().cloned())
        .collect();
    let answers = ask(
        loaded,
        regeneration.template(),
        regeneration.base().overlay(&given),
        &settled,
    )?;
    let regenerated = engine
        .regenerate(&regeneration, &answers, args.dry_run)
        .await
        .map_err(refusal)?;
    let rendered = match loaded.config.output() {
        OutputFormat::Text => render::regenerated(&regenerated, args.dry_run),
        OutputFormat::Json => json(&regenerated, "the rendering")?,
    };
    emit(out, rendered.trim_end(), "the rendering")?;
    Ok(())
}

/// `task answers` and `document answers`: the answers stored beside the item, as YAML — or,
/// under `--json`, as one JSON object.
pub(crate) async fn stored(
    out: &mut impl Write,
    loaded: &Loaded,
    record: RenderedRecord,
    args: &AnswersArgs,
) -> Result<u8, Failure> {
    let id = qualified(&args.id)?;
    let answers = engine(loaded)
        .template_answers(record, &id)
        .await
        .map_err(|error| Failure::from(&error))?;
    let rendered = match loaded.config.output() {
        OutputFormat::Json => json(&answers, "the answers")?,
        OutputFormat::Text => answers.to_yaml().map_err(|error| {
            Failure::decided("render", format!("could not render the answers: {error}"))
        })?,
    };
    emit(out, rendered.trim_end(), "the answers")?;
    Ok(EXIT_OK)
}

/// A refusal of the answers exits `2`; every other engine refusal is the command failing.
fn refusal(error: EngineError) -> Refusal {
    match &error {
        EngineError::MissingAnswers { .. } => Refusal::Answers(error.to_string()),
        EngineError::Template { error } if error.refuses_answers() => {
            Refusal::Answers(error.to_string())
        }
        _ => Refusal::Failed(Failure::from(&error)),
    }
}

/// What every create names, read and checked before any source is built.
struct Item {
    source: SourceName,
    project: NativeId,
    body: Body,
    repositories: Vec<Repository>,
    metadata: BTreeMap<MetadataKey, Value>,
}

/// Read a create's item: its source, project, repositories and metadata checked, and its
/// body rendered — asking, when interactive, for what the answers leave unanswered.
fn item(args: &CreateItemArgs, loaded: &Loaded) -> Result<Item, Refusal> {
    let source = SourceName::new(args.source.clone()).map_err(|error| {
        Refusal::Failed(Failure::decided(
            "invalid-source-name",
            format!(
                "{}: {error}\n\
                 next: name a configured source — `onetaskgraph sources list` reports them.",
                args.source
            ),
        ))
    })?;
    // Qualified with the source it is created in, a project is that source's own; qualified
    // with any other configured source it names a project the item cannot be filed under, and is
    // refused rather than read as a native id full of colons. A prefix naming no configured
    // source is part of a native id, as `task list --project` reads one.
    let project = match GlobalId::from_str(&args.project) {
        Ok(id) if id.source == source => id.native,
        Ok(id) if loaded.config.sources().contains_key(&id.source) => {
            return Err(Refusal::Failed(Failure::decided(
                "project-in-another-source",
                format!(
                    "--project {}: that project is in source {}, and this creates in {source}; an \
                     item is filed under a project of its own source\n\
                     next: name a project of {source}, or create the item in {}.",
                    args.project, id.source, id.source
                ),
            )));
        }
        Ok(_) | Err(_) => NativeId::from(args.project.as_str()),
    };
    let repositories = args
        .repository
        .iter()
        .map(|origin| {
            Repository::try_from(origin.clone()).map_err(|error| {
                Refusal::Failed(Failure::decided(
                    "invalid-repository",
                    format!(
                        "--repository {origin}: {error}\n\
                         next: name it by its normalized origin, such as \
                         github.com/owner/name."
                    ),
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    let metadata = args
        .metadata
        .iter()
        .map(|entry| metadata_entry(entry).map_err(Refusal::Failed))
        .collect::<Result<_, _>>()?;
    let body = body(&args.body, loaded)?;
    Ok(Item {
        source,
        project,
        body,
        repositories,
        metadata,
    })
}

/// One `--metadata KEY=JSON`: a caller's own key, and exactly one JSON value.
fn metadata_entry(entry: &str) -> Result<(MetadataKey, Value), Failure> {
    let Some((key, value)) = entry.split_once('=') else {
        return Err(Failure::decided(
            "invalid-metadata-key",
            format!(
                "--metadata {entry}: that is not KEY=JSON\n\
                 next: write it as --metadata myapp.key='\"value\"'."
            ),
        ));
    };
    let key = MetadataKey::new(key).map_err(|message| {
        Failure::decided(
            "invalid-metadata-key",
            format!("--metadata {entry}: {message}"),
        )
    })?;
    let value = serde_json::from_str(value).map_err(|error| {
        Failure::decided(
            "invalid-metadata-value",
            format!(
                "--metadata {entry}: the value is not JSON: {error}\n\
                 next: pass exactly one JSON value — quote a string as '\"text\"'."
            ),
        )
    })?;
    Ok((key, value))
}

/// A create's body: rendered from a template or a loader document, read from `--body-file`,
/// or read from standard input.
fn body(args: &CreateBodyArgs, loaded: &Loaded) -> Result<Body, Refusal> {
    if let Some(input) = input(
        args.template.as_deref(),
        &args.search_path,
        args.template_loader.as_deref(),
        args.answers.as_deref(),
    )? {
        let template = input.load()?;
        let answers = ask(
            loaded,
            &template,
            answers_given(args.answers.as_deref(), &args.var)?,
            &HashSet::new(),
        )?;
        let rendered = template.render(&answers)?;
        return Ok(Body::rendered(&input, rendered)?);
    }
    plain(args.body_file.as_deref()).map(Body::plain)
}

/// A plain body, byte for byte from `path`, or from standard input without one.
fn plain(path: Option<&Path>) -> Result<String, Refusal> {
    let (bytes, from) = match path {
        Some(path) => (
            std::fs::read(path).map_err(|error| {
                Refusal::Failed(Failure::decided(
                    "body-file",
                    format!(
                        "--body-file {}: could not read it: {error}\n\
                         next: name a readable file, or leave --body-file out and pass the body \
                         on standard input.",
                        path.display()
                    ),
                ))
            })?,
            format!("--body-file {}", path.display()),
        ),
        None => {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes).map_err(|error| {
                Refusal::Failed(Failure::decided(
                    "body-file",
                    format!(
                        "could not read the body from standard input: {error}\n\
                         next: pass the body with --body-file PATH, or render it with \
                         --template FILE."
                    ),
                ))
            })?;
            (bytes, "standard input".to_owned())
        }
    };
    String::from_utf8(bytes).map_err(|error| {
        Refusal::Failed(Failure::decided(
            "body-file",
            format!(
                "the body on {from} is not UTF-8 text: {error}\n\
                 next: save the body as UTF-8 and pass it again."
            ),
        ))
    })
}

/// Every `--depends-on` or `--delivers`, refused before any source is built unless each is
/// qualified: a bare id would be read as naming a task of the source being created in, which a
/// caller copying another source's id did not mean.
fn ids(given: &[String]) -> Result<Vec<GlobalId>, Failure> {
    given.iter().map(|id| qualified(id)).collect()
}
