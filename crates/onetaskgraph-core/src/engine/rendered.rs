//! Tasks and documents created from a body or a template, and regenerated in place.
//!
//! An item created or regenerated from a template records where it came from under the
//! reserved `onetaskgraph.template` key ([`TemplateProvenance`]); one created from a plain
//! body records none. The answers a regenerate needs are kept only where a source keeps them
//! beside the item — `local-md`'s own file — and never in the item's content or metadata, so
//! nothing of them is duplicated into an issue body. Every write here lands the content, the
//! provenance and those answers in **one** call to the source, which the source makes one
//! write.
//!
//! Regeneration is this engine's, and it never resolves a caller's layers or runs a caller's
//! command: the template is the one given — a file, or a loader document a caller states —
//! else the recorded `template` when that is a readable file, else the regenerate is refused
//! naming the recorded reference.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use onetaskgraph_plugin_api::{
    DependencyEdge, DependencyEndpoint, DependencyKind, Document, ItemKind, ItemWrite, Label,
    MetadataKey, MetadataRecord, NativeId, Priority, Repository, SourceName, Status,
    StatusCategory, Task, TaskRef,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::delivery::{qualified_task, source_failed, targets};
use super::{Delivered, Engine, EngineError, Qualified};
use crate::GlobalId;
use crate::resolve::ResolvedSource;
use crate::template::{
    Answers, RenderedTemplate, Template, TemplateError, TemplateInput, TemplateProvenance,
    body_digest,
};

/// The content a create writes.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// Text given as it is: the item records no provenance and no answers are kept.
    Plain(String),
    /// A template's rendering: the item records [`TemplateProvenance`] naming `template`, and
    /// a source that keeps answers keeps the rendering's resolved answers beside it.
    Rendered {
        /// What the template rendered, with the digest and the answers it rendered with.
        rendered: RenderedTemplate,
        /// The provenance `template`: a file's absolute path, a loader document's
        /// `reference`, or a string of a library caller's own.
        template: String,
    },
}

impl Body {
    /// A template's rendering, recorded by the reference `template` answers.
    #[must_use]
    pub fn rendered(template: &TemplateInput, rendered: RenderedTemplate) -> Self {
        Self::Rendered {
            rendered,
            template: template.reference(),
        }
    }

    /// The content, the metadata the item carries, and the answers kept beside it.
    fn parts(
        &self,
        metadata: &BTreeMap<MetadataKey, Value>,
    ) -> (
        String,
        BTreeMap<String, Value>,
        Option<&BTreeMap<String, Value>>,
    ) {
        let mut carried: BTreeMap<String, Value> = metadata
            .iter()
            .map(|(key, value)| (key.as_str().to_owned(), value.clone()))
            .collect();
        match self {
            Self::Plain(content) => (content.clone(), carried, None),
            Self::Rendered { rendered, template } => {
                carried.insert(
                    TemplateProvenance::KEY.to_owned(),
                    TemplateProvenance::of(template.clone(), rendered).to_value(),
                );
                (rendered.body.clone(), carried, Some(&rendered.answers))
            }
        }
    }
}

/// One task to create in one source.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskCreate {
    /// The configured source to create it in.
    pub source: SourceName,
    /// The project it is filed under, by that source's own id.
    pub project: NativeId,
    /// Its title.
    pub title: String,
    /// Its content.
    pub body: Body,
    /// Its status category; `todo` when none is given.
    pub status: Option<StatusCategory>,
    /// The names of its labels.
    pub labels: Vec<String>,
    /// The repositories its work changes.
    pub repositories: Vec<Repository>,
    /// The tasks it depends on — its own source's written without their source, any other's
    /// qualified.
    pub depends_on: Vec<GlobalId>,
    /// The tasks it delivers. Each is kept in step with it once it lands, as every write of a
    /// task that delivers keeps them.
    pub delivers: Vec<GlobalId>,
    /// The caller's own metadata keys — never one this product reserves, which a
    /// [`MetadataKey`] cannot name.
    pub metadata: BTreeMap<MetadataKey, Value>,
}

/// One project document to create, or replace, in one source.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentCreate {
    /// The configured source to write it in.
    pub source: SourceName,
    /// The project it is filed under, by that source's own id.
    pub project: NativeId,
    /// Its title.
    pub title: String,
    /// The id to write it under: a document the source already holds by this id is
    /// replaced, and otherwise one is created under it where the source lets a caller name
    /// what it creates. `None` creates one under an id derived from the title.
    pub id: Option<NativeId>,
    /// Its content.
    pub body: Body,
    /// The names of its labels.
    pub labels: Vec<String>,
    /// The repositories it concerns.
    pub repositories: Vec<Repository>,
    /// The caller's own metadata keys.
    pub metadata: BTreeMap<MetadataKey, Value>,
}

/// What creating a task came to: the task as its source reads it back, and every task it
/// delivers, kept in step with it.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskCreated {
    /// The task, under its qualified id.
    pub task: Qualified<Task>,
    /// One entry per task it delivers; empty when it delivers none.
    pub delivered: Vec<Delivered>,
}

/// Which kind of item a regenerate or an answers read names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderedRecord {
    /// A task.
    Task,
    /// A project document.
    Document,
}

impl RenderedRecord {
    fn noun(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Document => "document",
        }
    }

    fn no_such(self, id: &GlobalId) -> EngineError {
        match self {
            Self::Task => EngineError::NoSuchTask { id: id.to_string() },
            Self::Document => EngineError::NoSuchDocument { id: id.to_string() },
        }
    }
}

/// What `task render` and `document render` are asked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderRequest {
    /// The template to render; `None` renders the recorded `template` when that is a
    /// readable file.
    pub template: Option<TemplateInput>,
    /// The directories a recorded template file's chain resolves over, when `template` is
    /// `None`.
    pub search_path: Vec<PathBuf>,
    /// The answers laid over the base: the stored ones when they are in step with the item's
    /// provenance, none otherwise. An unset name drops its answer.
    pub answers: Answers,
    /// Read and render everything, and write nothing.
    pub dry_run: bool,
}

/// What `task render` and `document render` answer with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Regenerated {
    /// The item regenerated.
    pub id: GlobalId,
    /// The chain digest it rendered with.
    pub digest: String,
    /// The SHA-256 of the rendered content: what its provenance now records.
    pub body_digest: String,
    /// Whether its content, its provenance or its stored answers differ from what it held —
    /// what a write changed, or for a dry run what one would change.
    pub changed: bool,
    /// The rendered content.
    pub body: String,
}

/// What `task answers` and `document answers` answer with: the resolved answers an item was
/// last rendered from — defaults applied, `null` for an optional variable given neither — by
/// variable name, exactly as its source keeps them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TemplateAnswers(pub BTreeMap<String, Value>);

impl TemplateAnswers {
    /// The answers as a YAML mapping — the document an answers file holds, so what this
    /// prints can be handed back with `--answers`.
    ///
    /// # Errors
    ///
    /// Why the answers could not be written as YAML.
    pub fn to_yaml(&self) -> Result<String, String> {
        serde_norway::to_string(&self.0).map_err(|error| error.to_string())
    }
}

/// Why a regenerate did not start from the answers stored beside the item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnusedAnswers {
    /// The item records no template provenance, so nothing says which answers are its.
    NoProvenance,
    /// Its source keeps no answers beside it — it keeps none at all, or none for this item.
    NoneStored {
        /// The configured name of its source.
        source: String,
    },
    /// The answers stored beside it do not hash to the `answers_digest` its provenance
    /// records.
    OutOfStep,
    /// Its `onetaskgraph.template` entry is not one this product writes, so nothing trusted says
    /// which answers are its.
    MalformedProvenance {
        /// What is wrong with the entry.
        problem: String,
    },
}

impl fmt::Display for UnusedAnswers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProvenance => formatter.write_str(
                "it records no template provenance, so no stored answers are known to be its",
            ),
            Self::NoneStored { source } => write!(
                formatter,
                "source {source} holds no stored answers for it — only a source that keeps an \
                 authoring file, such as local-md, stores them"
            ),
            Self::OutOfStep => formatter.write_str(
                "the answers stored beside it do not hash to the answers_digest its provenance \
                 records, so they changed after it was rendered and are not trusted",
            ),
            Self::MalformedProvenance { problem } => write!(
                formatter,
                "{problem}, so nothing trusted says which stored answers are its"
            ),
        }
    }
}

/// Everything a regenerate reads before it renders: the item's own content and provenance,
/// the template, and the answers a render starts from.
///
/// [`Engine::regeneration`] answers one and [`Engine::regenerate`] renders and writes it — the
/// two halves apart so a caller that asks for answers, as the command line does when
/// interactive, can ask between them. [`Engine::render_task`] and
/// [`Engine::render_document`] are the two together.
#[derive(Debug, Clone)]
pub struct Regeneration {
    id: GlobalId,
    record: RenderedRecord,
    template: Template,
    reference: String,
    base: Answers,
    stored: Stored,
    content: String,
    provenance: Option<TemplateProvenance>,
}

/// The answers stored beside an item, and whether a render starts from them — one or the
/// other, never both: answers that are the base are in step with the provenance, and answers
/// that are not carry why.
#[derive(Debug, Clone)]
enum Stored {
    /// In step with the item's provenance, so a render starts from them.
    Trusted(BTreeMap<String, Value>),
    /// Not a base, for `reason`. What is held, if anything, is still what a render's own
    /// answers are compared with to say whether the item changed.
    Unused {
        reason: UnusedAnswers,
        held: Option<BTreeMap<String, Value>>,
    },
}

impl Stored {
    /// Whatever answers the item holds, trusted or not.
    fn held(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Self::Trusted(answers) => Some(answers),
            Self::Unused { held, .. } => held.as_ref(),
        }
    }
}

impl Regeneration {
    /// The item being regenerated.
    #[must_use]
    pub fn id(&self) -> &GlobalId {
        &self.id
    }

    /// The template it renders.
    #[must_use]
    pub fn template(&self) -> &Template {
        &self.template
    }

    /// The answers a render starts from, before any new ones are laid over them.
    #[must_use]
    pub fn base(&self) -> &Answers {
        &self.base
    }

    /// Every variable the stored answers settle when they are the base — an optional one
    /// they left `null` included, which a render leaves unanswered again — and none when
    /// they are not.
    pub fn settled(&self) -> impl Iterator<Item = &str> {
        let trusted = match &self.stored {
            Stored::Trusted(answers) => Some(answers),
            Stored::Unused { .. } => None,
        };
        trusted
            .into_iter()
            .flat_map(BTreeMap::keys)
            .map(String::as_str)
    }

    /// Why the stored answers were not the base, when they were not.
    #[must_use]
    pub fn unused(&self) -> Option<&UnusedAnswers> {
        match &self.stored {
            Stored::Trusted(_) => None,
            Stored::Unused { reason, .. } => Some(reason),
        }
    }

    /// Render from the base with `answers` laid over it.
    ///
    /// A stored answer to a variable the template no longer declares is dropped rather than
    /// refused: it was this product's to keep, not the caller's to answer for.
    ///
    /// # Errors
    ///
    /// [`EngineError::MissingAnswers`] naming every required variable left unanswered and
    /// why the stored answers were not used; [`EngineError::Template`] for any other refusal
    /// of the answers or of the template.
    pub fn render(&self, answers: &Answers) -> Result<RenderedTemplate, EngineError> {
        let mut base = self.base.clone();
        loop {
            match self.template.render(&base.overlay(answers)) {
                Err(TemplateError::UnknownAnswer { names })
                    if names.iter().all(|name| {
                        base.names().any(|held| held == name)
                            && !answers.names().any(|given| given == name)
                    }) =>
                {
                    for name in names {
                        base.unset(name);
                    }
                }
                Err(TemplateError::MissingRequired { names }) => {
                    return Err(EngineError::MissingAnswers {
                        id: self.id.to_string(),
                        names,
                        reason: self.unused().map_or_else(
                            || {
                                "the answers stored beside it were used, and leave these \
                                 unanswered"
                                    .to_owned()
                            },
                            ToString::to_string,
                        ),
                    });
                }
                Err(error) => return Err(EngineError::Template { error }),
                Ok(rendered) => return Ok(rendered),
            }
        }
    }
}

impl Engine {
    /// Create one task, from a plain body or a template's rendering, and keep every task it
    /// delivers in step with it.
    ///
    /// The content, the provenance and — where the source keeps them — the answers land in
    /// one write. A delivered task that cannot be kept in step is not an error: it is
    /// reported `failed` beside the task that was created.
    ///
    /// # Errors
    ///
    /// [`EngineError::UnknownSource`] and [`EngineError::SourceUnavailable`] for a source that
    /// cannot be reached, [`EngineError::NotCreatable`] for one with no write side — neither is
    /// written — and [`EngineError::SourceFailed`] when the source refuses the task.
    pub async fn create_task(&self, request: &TaskCreate) -> Result<TaskCreated, EngineError> {
        let source = self.creatable(&request.source, MetadataRecord::Task)?;
        let (content, metadata, answers) = request.body.parts(&request.metadata);
        let category = request.status.unwrap_or(StatusCategory::Todo);
        let near = &request.source;
        let id = NativeId::from(slug(&request.title, "task").as_str());
        let depends_on = request
            .depends_on
            .iter()
            .map(|far| DependencyEdge {
                from: DependencyEndpoint::from_native(id.clone(), ItemKind::Task),
                to: endpoint(far, near),
                kind: DependencyKind::Blocks,
            })
            .collect();
        let write = ItemWrite {
            target: None,
            item: Task {
                id,
                key: None,
                title: request.title.clone(),
                content: Some(content),
                status: Status {
                    category,
                    name: category_word(category),
                },
                priority: Priority::None,
                labels: labels(&request.labels),
                project: Some(request.project.clone()),
                url: None,
                location: None,
                created_at: None,
                updated_at: None,
                metadata,
                repositories: request.repositories.clone(),
                delivers: request
                    .delivers
                    .iter()
                    .map(|task| TaskRef::qualified(&task.source, &task.native))
                    .collect(),
                delivered_by: Vec::new(),
            },
            depends_on,
        };
        let written = match answers {
            Some(answers) => source.source().write_task_rendered(&write, answers).await,
            None => source.source().write_task(&write).await,
        }
        .map_err(|error| source_failed(source, error))?;
        let id = GlobalId::new(near.clone(), written);
        let task = source
            .source()
            .get_task(&id.native)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        let delivers = targets(&task.delivers, near);
        let delivered = self
            .deliver(&id, task.status.category, &delivers, &[])
            .await;
        Ok(TaskCreated {
            task: qualified_task(id, task),
            delivered,
        })
    }

    /// Create one project document, or replace the one the source holds under
    /// [`DocumentCreate::id`], from a plain body or a template's rendering.
    ///
    /// # Errors
    ///
    /// As [`create_task`](Self::create_task), and [`EngineError::NoDocuments`] for a source
    /// declaring it has none, which is not asked.
    pub async fn create_document(
        &self,
        request: &DocumentCreate,
    ) -> Result<Qualified<Document>, EngineError> {
        let source = self.creatable(&request.source, MetadataRecord::Document)?;
        documentary(source)?;
        let target = match &request.id {
            Some(id) => source
                .source()
                .get_document(id)
                .await
                .map_err(|error| source_failed(source, error))?
                .map(|held| held.id),
            None => None,
        };
        let (content, metadata, answers) = request.body.parts(&request.metadata);
        let write = ItemWrite {
            target,
            item: Document {
                id: request
                    .id
                    .clone()
                    .unwrap_or_else(|| NativeId::from(slug(&request.title, "document").as_str())),
                title: request.title.clone(),
                content: Some(content),
                project: Some(request.project.clone()),
                labels: labels(&request.labels),
                url: None,
                location: None,
                created_at: None,
                updated_at: None,
                metadata,
                repositories: request.repositories.clone(),
            },
            depends_on: Vec::new(),
        };
        let written = match answers {
            Some(answers) => {
                source
                    .source()
                    .write_document_rendered(&write, answers)
                    .await
            }
            None => source.source().write_document(&write).await,
        }
        .map_err(|error| source_failed(source, error))?;
        let id = GlobalId::new(request.source.clone(), written);
        let document = source
            .source()
            .get_document(&id.native)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchDocument { id: id.to_string() })?;
        Ok(Qualified { id, item: document })
    }

    /// The answers the task or document `id` was last rendered from, as its source keeps them.
    ///
    /// # Errors
    ///
    /// [`EngineError::NoSuchTask`] or [`EngineError::NoSuchDocument`] when the item is not
    /// there, [`EngineError::NoStoredAnswers`] naming it when none are stored for it — which
    /// is every item of a source that keeps none — and [`EngineError::SourceFailed`] when the
    /// source cannot answer.
    pub async fn template_answers(
        &self,
        record: RenderedRecord,
        id: &GlobalId,
    ) -> Result<TemplateAnswers, EngineError> {
        let source = self.built(&id.source)?;
        if record == RenderedRecord::Document {
            documentary(source)?;
        }
        self.read_item(source, record, id).await?;
        self.stored(source, record, id)
            .await?
            .map(TemplateAnswers)
            .ok_or_else(|| EngineError::NoStoredAnswers {
                record: record.noun(),
                id: id.to_string(),
                reason: UnusedAnswers::NoneStored {
                    source: id.source.to_string(),
                }
                .to_string(),
            })
    }

    /// Read everything a regenerate of the task or document `id` needs, before it renders.
    ///
    /// # Errors
    ///
    /// [`EngineError::NoSuchTask`] or [`EngineError::NoSuchDocument`] when the item is not
    /// there; [`EngineError::NoTemplate`] when no template is given and the item records
    /// none; [`EngineError::MalformedProvenance`] when no template is given and the entry it
    /// records is not one this product writes; [`EngineError::TemplateNotAFile`] when no template is given and the one it
    /// records is not a readable file, which only a loader document can then stand in for;
    /// [`EngineError::Template`] when the template cannot be loaded; and the refusals of a
    /// source that cannot be reached or cannot answer.
    pub async fn regeneration(
        &self,
        record: RenderedRecord,
        id: &GlobalId,
        request: &RenderRequest,
    ) -> Result<Regeneration, EngineError> {
        let source = self.built(&id.source)?;
        if record == RenderedRecord::Document {
            documentary(source)?;
        }
        let (content, metadata) = self.read_item(source, record, id).await?;
        let read = TemplateProvenance::read(&metadata);
        let template = match (&request.template, &read) {
            (Some(given), _) => given.clone(),
            // An entry this product did not write names nothing it can trust: with no template
            // given there is nothing to render, and the refusal says why.
            (None, Err(problem)) => {
                return Err(EngineError::MalformedProvenance {
                    record: record.noun(),
                    id: id.to_string(),
                    problem: problem.clone(),
                });
            }
            (None, Ok(Some(recorded))) if Path::new(&recorded.template).is_file() => {
                TemplateInput::File {
                    path: PathBuf::from(&recorded.template),
                    search_path: request.search_path.clone(),
                }
            }
            (None, Ok(Some(recorded))) => {
                return Err(EngineError::TemplateNotAFile {
                    record: record.noun(),
                    id: id.to_string(),
                    reference: recorded.template.clone(),
                });
            }
            (None, Ok(None)) => {
                return Err(EngineError::NoTemplate {
                    record: record.noun(),
                    id: id.to_string(),
                });
            }
        };
        let reference = template.reference();
        let template = template
            .load()
            .map_err(|error| EngineError::Template { error })?;
        let held = self.stored(source, record, id).await?;
        let unused = |reason| Stored::Unused {
            reason,
            held: held.clone(),
        };
        let stored = match (&read, &held) {
            (Err(problem), _) => unused(UnusedAnswers::MalformedProvenance {
                problem: problem.clone(),
            }),
            (Ok(None), _) => unused(UnusedAnswers::NoProvenance),
            (Ok(Some(_)), None) => unused(UnusedAnswers::NoneStored {
                source: id.source.to_string(),
            }),
            (Ok(Some(recorded)), Some(answers))
                if crate::template::answers_digest(answers) != recorded.answers_digest.as_str() =>
            {
                unused(UnusedAnswers::OutOfStep)
            }
            (Ok(Some(_)), Some(answers)) => Stored::Trusted(answers.clone()),
        };
        let mut base = Answers::new();
        if let Stored::Trusted(answers) = &stored {
            // An optional variable given nothing resolved to `null`; left unanswered, it
            // resolves to that again.
            for (name, value) in answers.iter().filter(|(_, value)| !value.is_null()) {
                base.set(name.clone(), value.clone());
            }
        }
        Ok(Regeneration {
            id: id.clone(),
            record,
            template,
            reference,
            base,
            stored,
            content,
            provenance: read.ok().flatten(),
        })
    }

    /// Render `regeneration` with `answers` laid over its base, and write the content, the
    /// provenance and the answers in one write — and nothing else about the item — when any of
    /// them differs from what it holds and this is not a dry run.
    ///
    /// # Errors
    ///
    /// Every refusal of [`Regeneration::render`]; [`EngineError::RenderingNotWritable`] for a
    /// source with no write side, which is not asked; and [`EngineError::SourceFailed`] when
    /// the source refuses the write.
    pub async fn regenerate(
        &self,
        regeneration: &Regeneration,
        answers: &Answers,
        dry_run: bool,
    ) -> Result<Regenerated, EngineError> {
        let rendered = regeneration.render(answers)?;
        let provenance = TemplateProvenance::of(regeneration.reference.clone(), &rendered);
        let changed = regeneration.content != rendered.body
            || regeneration.provenance.as_ref() != Some(&provenance)
            || regeneration
                .stored
                .held()
                .is_some_and(|stored| *stored != rendered.answers);
        let id = &regeneration.id;
        if changed && !dry_run {
            let source = self.built(&id.source)?;
            if !source.source().writes().is_supported() {
                return Err(EngineError::RenderingNotWritable {
                    name: source.name().to_string(),
                    kind: source.kind().to_owned(),
                    record: regeneration.record.noun(),
                });
            }
            let value = provenance.to_value();
            match regeneration.record {
                RenderedRecord::Task => {
                    source
                        .source()
                        .set_task_rendering(&id.native, &rendered.body, &value, &rendered.answers)
                        .await
                }
                RenderedRecord::Document => {
                    source
                        .source()
                        .set_document_rendering(
                            &id.native,
                            &rendered.body,
                            &value,
                            &rendered.answers,
                        )
                        .await
                }
            }
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| regeneration.record.no_such(id))?;
        }
        Ok(Regenerated {
            id: id.clone(),
            digest: rendered.digest,
            body_digest: body_digest(&rendered.body),
            changed,
            body: rendered.body,
        })
    }

    /// Regenerate one task in place: [`regeneration`](Self::regeneration) then
    /// [`regenerate`](Self::regenerate), never asking for an answer.
    ///
    /// # Errors
    ///
    /// Every refusal of either half.
    pub async fn render_task(
        &self,
        id: &GlobalId,
        request: &RenderRequest,
    ) -> Result<Regenerated, EngineError> {
        let regeneration = self.regeneration(RenderedRecord::Task, id, request).await?;
        self.regenerate(&regeneration, &request.answers, request.dry_run)
            .await
    }

    /// Regenerate one project document in place, on the terms of
    /// [`render_task`](Self::render_task).
    ///
    /// # Errors
    ///
    /// As [`render_task`](Self::render_task).
    pub async fn render_document(
        &self,
        id: &GlobalId,
        request: &RenderRequest,
    ) -> Result<Regenerated, EngineError> {
        let regeneration = self
            .regeneration(RenderedRecord::Document, id, request)
            .await?;
        self.regenerate(&regeneration, &request.answers, request.dry_run)
            .await
    }

    /// The built source called `name`, when it can be written through.
    fn creatable(
        &self,
        name: &SourceName,
        record: MetadataRecord,
    ) -> Result<&ResolvedSource, EngineError> {
        let source = self.built(name)?;
        if source.source().writes().is_supported() {
            return Ok(source);
        }
        Err(EngineError::NotCreatable {
            name: source.name().to_string(),
            kind: source.kind().to_owned(),
            record,
        })
    }

    /// The content and the metadata of the item `id` names.
    async fn read_item(
        &self,
        source: &ResolvedSource,
        record: RenderedRecord,
        id: &GlobalId,
    ) -> Result<(String, BTreeMap<String, Value>), EngineError> {
        let read = match record {
            RenderedRecord::Task => source
                .source()
                .get_task(&id.native)
                .await
                .map(|task| task.map(|task| (task.content, task.metadata))),
            RenderedRecord::Document => source
                .source()
                .get_document(&id.native)
                .await
                .map(|document| document.map(|document| (document.content, document.metadata))),
        }
        .map_err(|error| source_failed(source, error))?;
        let (content, metadata) = read.ok_or_else(|| record.no_such(id))?;
        Ok((content.unwrap_or_default(), metadata))
    }

    /// The answers the item `id` names has stored beside it, when it has any.
    async fn stored(
        &self,
        source: &ResolvedSource,
        record: RenderedRecord,
        id: &GlobalId,
    ) -> Result<Option<BTreeMap<String, Value>>, EngineError> {
        match record {
            RenderedRecord::Task => source.source().task_template_answers(&id.native).await,
            RenderedRecord::Document => source.source().document_template_answers(&id.native).await,
        }
        .map_err(|error| source_failed(source, error))
    }
}

/// Refuse a source declaring it has no documents, before it is asked anything.
fn documentary(source: &ResolvedSource) -> Result<(), EngineError> {
    if source.source().capabilities().documents.is_native() {
        return Ok(());
    }
    Err(EngineError::NoDocuments {
        name: source.name().to_string(),
        kind: source.kind().to_owned(),
    })
}

/// A dependency's far end as the near source writes it: its own item by its own id, another
/// source's qualified.
fn endpoint(far: &GlobalId, near: &SourceName) -> DependencyEndpoint {
    if &far.source == near {
        return DependencyEndpoint::from_native(far.native.clone(), ItemKind::Task);
    }
    DependencyEndpoint::new(far.to_string(), ItemKind::Task)
        .unwrap_or_else(|_| DependencyEndpoint::from_native(far.native.clone(), ItemKind::Task))
}

/// A create names each label by name alone, so its id is that name too: the id is what a
/// folder of Markdown writes, and the name is what a source that matches labels reads.
fn labels(names: &[String]) -> Vec<Label> {
    names
        .iter()
        .map(|name| Label {
            id: NativeId::from(name.as_str()),
            name: name.clone(),
            color: None,
        })
        .collect()
}

/// A category as this product spells it — the word every source's default mapping reads back
/// as that category.
fn category_word(category: StatusCategory) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|word| word.as_str().map(str::to_owned))
        .unwrap_or_else(|| "todo".to_owned())
}

/// The id a created item is suggested under: its title in lower case, every run of anything
/// but an ASCII letter or digit one dash. A source is free to file it under another.
fn slug(title: &str, fallback: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        fallback.to_owned()
    } else {
        slug.to_owned()
    }
}
