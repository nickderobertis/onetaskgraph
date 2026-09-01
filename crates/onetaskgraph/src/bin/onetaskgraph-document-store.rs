//! A file-backed document source, spoken to over `docs/plugin-protocol.md`.
//!
//! It exists for one reason the suite could not otherwise satisfy. A copy is only proven
//! by reading the destination back *afterwards*, and every destination this build has is
//! either a folder of Markdown — which declares it has no documents — or the in-memory
//! source, whose work dies with the process that held it. One command-line invocation is
//! one process, so a document copy driven the way a user drives it had nothing left to
//! look at. This keeps its documents in a JSON file, so what one invocation writes the
//! next one reads.
//!
//! It is **not** a product surface. The `test-support` feature it is built behind is not a
//! default feature, so `cargo install` never produces it; every project target in
//! `crates/onetaskgraph/project.json` passes `--all-features`, so it is built, linted and
//! type-checked in every required check on all three platforms.
//!
//! It shares no code with the engine's own [`serve`](onetaskgraph_core::serve): it reads
//! the envelope itself and answers it itself, against the contract types alone. That is
//! deliberate — a peer written from the protocol document rather than from the engine's
//! implementation of it is the thing the stdio seam claims to support, and a second
//! implementation is what makes the claim testable at all.
//!
//! Its settings, handed over in the `initialize` request (§3), are
//! `{"store": <path>, "documents": "native"|"unsupported", "log": <path>}` — the last two
//! optional. `documents` lets a journey configure a peer that says it has none, and `log`
//! appends the name of every method this source is asked for, which is how a journey
//! proves a refusal happened *before* anything was read.

use std::io::{BufRead, BufReader, Write, stderr, stdin, stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencySupport, Document, DocumentQuery, ItemWrite, NativeId, Page,
    PageRequest, ProjectFilter, SourceError, Support, TextFields, TextQuery, documentless,
    unwritable,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// The kind this source reports at the handshake, and the one its refusals name.
const KIND: &str = "document-store";

/// The protocol version this peer speaks. `docs/plugin-protocol.md` specifies 2.
const PROTOCOL_VERSION: u32 = 2;

/// What the `initialize` request's `config` member carries for this source.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    /// The JSON file this source keeps its documents in.
    ///
    /// Read on every call and written after every change rather than held between them:
    /// what makes this source useful to a journey is that the file *is* the state, so a
    /// second process sees what the first one wrote.
    store: PathBuf,
    /// Whether this source has documents at all.
    ///
    /// Defaults to having them, because that is what this source is for. A journey that
    /// needs a peer declaring it has none sets it, and then every document call here
    /// refuses in the contract's own words.
    #[serde(default = "has_documents")]
    documents: Support,
    /// Where to append the name of every method this source is asked for.
    ///
    /// Absent by default. A journey that has to prove a refusal happened before anything
    /// was read sets it and then reads the file: a log holding the handshake and nothing
    /// else is a source nobody asked a question of.
    #[serde(default)]
    log: Option<PathBuf>,
}

fn has_documents() -> Support {
    Support::Native
}

/// The documents this source holds, as the file spells them.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
struct Store {
    /// Every document, in the order this source serves them.
    documents: Vec<Document>,
}

/// Read the store, treating a file that is not there yet as an empty one.
///
/// A missing file is the ordinary first-run state — a destination nothing has been copied
/// into yet — rather than a failure, so it answers as the empty store it describes.
fn read_store(path: &Path) -> Result<Store, SourceError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(error) => {
            return Err(SourceError::Unavailable {
                message: format!("could not read {}: {error}", path.display()),
            });
        }
    };
    serde_json::from_str(&raw).map_err(|error| SourceError::Malformed {
        message: format!("{} is not a document store: {error}", path.display()),
    })
}

/// Write the store back, creating the directory it lives in.
fn write_store(path: &Path, store: &Store) -> Result<(), SourceError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| SourceError::Unavailable {
            message: format!("could not create {}: {error}", parent.display()),
        })?;
    }
    let rendered = serde_json::to_string_pretty(store).map_err(|error| SourceError::Malformed {
        message: format!("this source's documents will not serialize: {error}"),
    })?;
    std::fs::write(path, rendered).map_err(|error| SourceError::Unavailable {
        message: format!("could not write {}: {error}", path.display()),
    })
}

/// What this source declares once, at the handshake.
///
/// No projects and no tasks of its own: it is a document store, and saying so is what
/// keeps it honest — the engine then reports the predicate unavailable for it rather than
/// asking for rows it does not have.
fn capabilities(settings: &Settings) -> Capabilities {
    Capabilities {
        projects: Support::Unsupported,
        documents: settings.documents,
        orphan_tasks: Support::Native,
        filter_by_label: Support::Native,
        filter_by_status: Support::Native,
        search_title: Support::Native,
        search_content: Support::Native,
        task_dependencies: DependencySupport::BothDirections,
        project_dependencies: DependencySupport::BothDirections,
        max_page_size: 50,
    }
}

/// Refuse every document call when this source's settings say it has none.
fn documentary(settings: &Settings) -> Result<(), SourceError> {
    if settings.documents.is_native() {
        return Ok(());
    }
    Err(documentless(KIND))
}

/// Whether one document survives every predicate a query carries.
fn survives(document: &Document, query: &DocumentQuery) -> bool {
    let held: Vec<String> = document
        .labels
        .iter()
        .map(|label| label.name.to_lowercase())
        .collect();
    let holds = |name: &String| held.contains(&name.to_lowercase());
    let labels = &query.labels;
    if !labels.any_of.is_empty() && !labels.any_of.iter().any(holds) {
        return false;
    }
    if !labels.all_of.iter().all(holds) {
        return false;
    }
    if labels.none_of.iter().any(holds) {
        return false;
    }
    let filed = match &query.project {
        ProjectFilter::Any => true,
        ProjectFilter::Orphans => document.project.is_none(),
        ProjectFilter::Is(id) => document.project.as_ref() == Some(id),
    };
    filed && matches(document, query.text.as_ref())
}

/// Whether one document matches a free-text query, over the fields it names.
fn matches(document: &Document, query: Option<&TextQuery>) -> bool {
    let Some(query) = query else { return true };
    let terms = query.terms.to_lowercase();
    let in_title = document.title.to_lowercase().contains(&terms);
    let in_content = document
        .content
        .as_deref()
        .is_some_and(|body| body.to_lowercase().contains(&terms));
    match query.fields {
        TextFields::Title => in_title,
        TextFields::Content => in_content,
        TextFields::TitleOrContent => in_title || in_content,
    }
}

/// Slice `items` into the page asked for, refusing a cursor this source never issued.
fn paginate<T: Clone>(items: &[T], page: &PageRequest) -> Result<Page<T>, SourceError> {
    let start = match &page.cursor {
        None => 0usize,
        Some(Cursor(raw)) => {
            let offset = raw.parse::<usize>().map_err(|_| SourceError::Malformed {
                message: format!("cursor {raw:?} was not issued by this source"),
            })?;
            if offset >= items.len() {
                return Err(SourceError::Malformed {
                    message: format!(
                        "cursor {raw:?} was not issued by this source; it points past the {} \
                         result(s) available",
                        items.len()
                    ),
                });
            }
            offset
        }
    };
    if page.limit == 0 {
        return Err(SourceError::Config {
            message: "a page limit of 0 is not a page; ask for at least 1 row".to_owned(),
        });
    }
    let end = start
        .saturating_add(page.limit.min(50) as usize)
        .min(items.len());
    Ok(Page {
        items: items.get(start..end).unwrap_or_default().to_vec(),
        next: (end < items.len()).then(|| Cursor(end.to_string())),
    })
}

/// `wanted` when nothing holds it, or the first `wanted-N` that is free.
///
/// A destination decides its own ids, exactly as every other writable source does: the id
/// an item was read under at its source is a suggestion.
fn unused(store: &Store, wanted: &NativeId) -> NativeId {
    if !store.documents.iter().any(|held| &held.id == wanted) {
        return wanted.clone();
    }
    (2_u32..)
        .map(|attempt| NativeId(format!("{wanted}-{attempt}")))
        .find(|candidate| !store.documents.iter().any(|held| &held.id == candidate))
        .expect("an unbounded suffix eventually clears a finite set of ids")
}

/// Answer one method against the store this source's settings name.
fn dispatch(settings: &Settings, method: &str, params: Value) -> Result<Value, SourceError> {
    let read = |member: &str| -> Result<Value, SourceError> {
        params
            .get(member)
            .cloned()
            .ok_or_else(|| SourceError::Malformed {
                message: format!("the parameters of {method} carry no {member}"),
            })
    };
    let page_of = |params: &Value| -> Result<PageRequest, SourceError> {
        serde_json::from_value(params.get("page").cloned().unwrap_or(Value::Null)).map_err(
            |error| SourceError::Malformed {
                message: format!("the page of {method} is not the shape it takes: {error}"),
            },
        )
    };

    match method {
        "health" => Ok(json!({
            "reachable": true,
            "detail": format!("{} document(s) on disk", read_store(&settings.store)?.documents.len()),
        })),
        // This source holds no tasks and no projects, and says so rather than pretending:
        // an empty page is the whole truth here, and `projects: unsupported` at the
        // handshake is what stops the engine reading the empty one as a narrowed answer.
        "get_task" => Ok(json!({"task": Value::Null})),
        "get_project" => Ok(json!({"project": Value::Null})),
        "query_tasks"
        | "query_projects"
        | "labels"
        | "task_dependencies"
        | "project_dependencies" => Ok(json!({"items": [], "next": Value::Null})),
        "get_document" => {
            documentary(settings)?;
            let id: NativeId = serde_json::from_value(read("id")?).map_err(from_params(method))?;
            let found = read_store(&settings.store)?
                .documents
                .into_iter()
                .find(|document| document.id == id);
            Ok(json!({"document": found}))
        }
        "query_documents" => {
            documentary(settings)?;
            let query: DocumentQuery =
                serde_json::from_value(read("query")?).map_err(from_params(method))?;
            let matched: Vec<Document> = read_store(&settings.store)?
                .documents
                .into_iter()
                .filter(|document| survives(document, &query))
                .collect();
            encode(&paginate(&matched, &page_of(&params)?)?)
        }
        "write_document" => {
            documentary(settings)?;
            let write: ItemWrite<Document> =
                serde_json::from_value(read("write")?).map_err(from_params(method))?;
            let mut store = read_store(&settings.store)?;
            let id = match &write.target {
                Some(target) => {
                    let position = store
                        .documents
                        .iter()
                        .position(|held| &held.id == target)
                        .ok_or_else(|| SourceError::Refused {
                            message: format!(
                                "{target} names no document this source holds; next: copy with \
                                 --recreate to create one instead of updating"
                            ),
                        })?;
                    store.documents[position] = Document {
                        id: target.clone(),
                        ..write.item.clone()
                    };
                    target.clone()
                }
                None => {
                    let id = unused(&store, &write.item.id);
                    store.documents.push(Document {
                        id: id.clone(),
                        ..write.item.clone()
                    });
                    id
                }
            };
            write_store(&settings.store, &store)?;
            Ok(json!({"id": id}))
        }
        "delete_document" => {
            documentary(settings)?;
            let id: NativeId = serde_json::from_value(read("id")?).map_err(from_params(method))?;
            let mut store = read_store(&settings.store)?;
            store.documents.retain(|document| document.id != id);
            write_store(&settings.store, &store)?;
            Ok(json!({}))
        }
        "write_task" | "write_project" | "delete_task" | "delete_project" => Err(unwritable(KIND)),
        other => Err(SourceError::Malformed {
            message: format!("protocol version {PROTOCOL_VERSION} has no method called {other:?}"),
        }),
    }
}

/// The refusal a member of the wrong shape earns, naming the method it arrived for.
fn from_params(method: &str) -> impl Fn(serde_json::Error) -> SourceError + '_ {
    move |error| SourceError::Malformed {
        message: format!("the parameters of {method} are not the shape it takes: {error}"),
    }
}

/// One answer as the value the envelope carries.
fn encode<T: serde::Serialize>(value: &T) -> Result<Value, SourceError> {
    serde_json::to_value(value).map_err(|error| SourceError::Malformed {
        message: format!("this source returned data that will not serialize: {error}"),
    })
}

/// Record that this source was asked for `method`, when its settings ask for a record.
///
/// Best effort on purpose: a log this source cannot write is not a reason to fail the
/// request a caller actually made, and the journey that reads the log fails loudly if it
/// is not there.
fn note(settings: &Settings, method: &str) {
    let Some(path) = &settings.log else { return };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{method}");
    }
}

/// The handshake (§3), which is also where this source learns where its store is.
fn initialize(settings: &mut Option<Settings>, params: &Value) -> Result<Value, SourceError> {
    let version = params
        .get("protocol_version")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if version != u64::from(PROTOCOL_VERSION) {
        return Err(SourceError::Config {
            message: format!(
                "protocol version {version} is not supported by this plugin; it speaks version \
                 {PROTOCOL_VERSION}"
            ),
        });
    }
    let read: Settings = serde_json::from_value(
        params.get("config").cloned().unwrap_or(Value::Null),
    )
    .map_err(|error| SourceError::Config {
        message: format!("this source's settings must be {{\"store\": <path>, …}}: {error}"),
    })?;
    let answer = json!({
        "protocol_version": PROTOCOL_VERSION,
        "kind": KIND,
        "capabilities": encode(&capabilities(&read))?,
        "writes": "supported",
    });
    note(&read, "initialize");
    *settings = Some(read);
    Ok(answer)
}

/// Serve one connection until the engine closes its input.
fn main() -> ExitCode {
    let input = BufReader::new(stdin().lock());
    let mut output = stdout().lock();
    let mut settings: Option<Settings> = None;
    for line in input.lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                let _ = writeln!(stderr(), "{KIND}: {error}");
                return ExitCode::FAILURE;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                let _ = writeln!(stderr(), "{KIND}: that is not a request envelope: {error}");
                continue;
            }
        };
        let Some(id) = request.get("id").and_then(Value::as_str).map(str::to_owned) else {
            let _ = writeln!(
                stderr(),
                "{KIND}: ignoring a line with no request id: {line}"
            );
            continue;
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        let outcome = if method == "initialize" {
            initialize(&mut settings, &params)
        } else {
            match &settings {
                None => Err(SourceError::Malformed {
                    message: format!("{method} arrived before the handshake"),
                }),
                Some(settings) => {
                    note(settings, &method);
                    dispatch(settings, &method, params)
                }
            }
        };
        let answered = match outcome {
            Ok(result) => json!({"id": id, "result": result}),
            Err(error) => json!({"id": id, "error": error}),
        };
        // An answer is built from contract types that all serialize.
        let rendered = serde_json::to_string(&answered).expect("an answer is plain data");
        if writeln!(output, "{rendered}").is_err() || output.flush().is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
