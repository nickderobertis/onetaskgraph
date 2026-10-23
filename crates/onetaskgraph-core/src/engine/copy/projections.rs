//! Public projections: the neutral public views a project copy makes of a private
//! coordination project's public tasks, and the one map that ties each view to its canonical
//! task.
//!
//! **This module is the one definition of that map.** It lives on the private destination
//! project alone, at [`PublicProjections::KEY`], as
//! `{"schema_version": 1, "tasks": [{"task": "<source>:<id>", "projection": "<source>:<id>"}]}`:
//! `task` is the canonical task the private destination holds, `projection` its public view.
//! The copy that writes it, the re-copy that reads it back, the status mirror and both SDKs'
//! serialization tests all go through [`PublicProjections`], so no second spelling of the
//! shape exists to drift from this one.
//!
//! A projection is a **view**, never another executable task: it carries its canonical task's
//! screened title, content, repositories and status, its dependencies only on other
//! projections, and nothing else — no metadata, no origin, no link back, no name of the
//! private project. Everything it is made from is read from the canonical records on every
//! copy, so an edit made to a view is overwritten by the next copy and never reaches the
//! canonical graph.

use std::collections::{BTreeMap, BTreeSet};

use onetaskgraph_plugin_api::{
    AssetPayload, Classification, DependencyEdge, DependencyEndpoint, DependencyKind, ItemKind,
    ItemWrite, NativeId, Priority, Project, Repository, SourceName, Status, StatusCategory, Task,
    WriteTarget,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::assets;
use super::super::boundary::{Exposure, Outbound};
use super::super::{Engine, EngineError};
use super::{CopyOutcome, Creating, Item, Journal, Level, Planned, Undo, create_refused, far_end};
use super::{forward_edges, refused};
use crate::GlobalId;
use crate::config::SourceVisibility;
use crate::resolve::ResolvedSource;

/// The private destination project's map from each canonical task to its public projection.
///
/// Read with [`PublicProjections::read`], which holds a stored value to every rule of the
/// shape, and written back with [`PublicProjections::to_value`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicProjections {
    /// The version of this shape; [`PublicProjections::SCHEMA_VERSION`] is the only one.
    pub schema_version: u32,
    /// One entry per canonical task that has a public projection, unique by both ids.
    pub tasks: Vec<PublicProjection>,
}

/// One canonical task and its public projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicProjection {
    /// The canonical task, qualified: the one the private destination holds.
    pub task: GlobalId,
    /// Its public counterpart, qualified: a view in a source not declared private.
    pub projection: GlobalId,
}

impl Default for PublicProjections {
    fn default() -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            tasks: Vec::new(),
        }
    }
}

impl PublicProjections {
    /// The reserved metadata key the private destination project records the map under.
    pub const KEY: &'static str = "onetaskgraph.public_projections";

    /// The one version of the shape this engine reads and writes.
    pub const SCHEMA_VERSION: u32 = 1;

    /// The map `metadata` records, or an empty one when it records none.
    ///
    /// # Errors
    ///
    /// A message saying what is wrong with a recorded value: not this shape, another
    /// `schema_version`, or two entries naming one task or one projection.
    pub fn read(metadata: &BTreeMap<String, Value>) -> Result<Self, String> {
        match metadata.get(Self::KEY) {
            None => Ok(Self::default()),
            Some(value) => Self::parse(value),
        }
    }

    /// One recorded value, held to every rule of the shape.
    ///
    /// # Errors
    ///
    /// As [`read`](Self::read).
    pub fn parse(value: &Value) -> Result<Self, String> {
        let map: Self = serde_json::from_value(value.clone()).map_err(|error| {
            format!(
                "{} is not {{\"schema_version\": 1, \"tasks\": [{{\"task\": …, \"projection\": \
                 …}}]}}: {error}",
                Self::KEY
            )
        })?;
        if map.schema_version != Self::SCHEMA_VERSION {
            return Err(format!(
                "{} is schema_version {}, and this engine reads only {}",
                Self::KEY,
                map.schema_version,
                Self::SCHEMA_VERSION
            ));
        }
        let mut tasks = BTreeSet::new();
        let mut projections = BTreeSet::new();
        for entry in &map.tasks {
            if !tasks.insert(entry.task.to_string()) {
                return Err(format!("{} names task {} twice", Self::KEY, entry.task));
            }
            if !projections.insert(entry.projection.to_string()) {
                return Err(format!(
                    "{} names projection {} twice",
                    Self::KEY,
                    entry.projection
                ));
            }
        }
        Ok(map)
    }

    /// The map as the private destination project records it.
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("the projection map serializes")
    }

    /// The projection the map records for `task`.
    #[must_use]
    pub fn projection_of(&self, task: &GlobalId) -> Option<&GlobalId> {
        self.tasks
            .iter()
            .find(|entry| &entry.task == task)
            .map(|entry| &entry.projection)
    }

    /// Record `projection` as `task`'s, in place of any it recorded, answering whether the
    /// map changed. An entry another task held naming the same projection is dropped, so the
    /// map stays unique by both ids.
    pub fn set(&mut self, task: &GlobalId, projection: &GlobalId) -> bool {
        if self.projection_of(task) == Some(projection) {
            return false;
        }
        self.tasks
            .retain(|entry| &entry.task != task && &entry.projection != projection);
        self.tasks.push(PublicProjection {
            task: task.clone(),
            projection: projection.clone(),
        });
        true
    }
}

/// The opaque native id a public projection of `of` is asked to be created under.
///
/// Stable — the same `of` always answers the same id, so a copy interrupted between creating a
/// view and recording it finds that view again by this id rather than creating a second — and
/// opaque: a digest that names nothing it was derived from. A destination that issues ids of
/// its own is free to ignore it, as it is any id a create is handed.
#[must_use]
pub fn opaque_id(prefix: &str, of: &GlobalId) -> NativeId {
    let mut digest = Sha256::new();
    digest.update(b"onetaskgraph public projection\0");
    digest.update(prefix.as_bytes());
    digest.update(b"\0");
    digest.update(of.to_string().as_bytes());
    let hex: String = digest
        .finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    NativeId(format!("{prefix}-{hex}"))
}

/// The neutral title every public projection project carries.
pub const PUBLIC_PROJECT_TITLE: &str = "Public work";

/// The local references in `content` a public projection could not carry: every Markdown
/// link or image whose destination is neither a URL, a bare fragment nor one of `assets` as
/// `./<name>`. Code spans and blocks hold no link, so nothing in them is one.
#[must_use]
pub fn unresolved_references(content: &str, assets: &[String]) -> Vec<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let mut unresolved = Vec::new();
    for event in Parser::new_ext(content, Options::empty()) {
        let destination = match event {
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => dest_url,
            _ => continue,
        };
        let destination = destination.trim();
        let carried = destination.starts_with('#')
            || destination.split_once(':').is_some_and(|(scheme, rest)| {
                !scheme.is_empty()
                    && scheme.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "+-.".contains(character)
                    })
                    && scheme
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_ascii_alphabetic())
                    && scheme.len() > 1
                    && (rest.starts_with("//") || scheme.eq_ignore_ascii_case("mailto"))
                    && !scheme.eq_ignore_ascii_case("file")
            })
            || destination
                .strip_prefix("./")
                .is_some_and(|name| assets.iter().any(|asset| asset == name));
        if !carried && !unresolved.iter().any(|seen| seen == destination) {
            unresolved.push(destination.to_owned());
        }
    }
    unresolved
}

/// One public task of a private plan a project copy projects, as its canonical record read.
pub(super) struct Projected {
    /// The canonical task, as the copy's source holds it.
    source: GlobalId,
    /// The source its repositories route it to, which is not declared private.
    to: SourceName,
    /// The task as its source read it, which the view is made from and nothing else is.
    task: Task,
    /// The image assets its content references, with their bytes.
    assets: Vec<AssetPayload>,
    /// The tasks of the same plan it depends on, by their source ids.
    depends_on: Vec<GlobalId>,
}

impl Projected {
    /// What the copy writes as this task's public view: `id` the id it is asked to be created
    /// under, filed under the public project `project`. Its title, content, repositories and
    /// status, and nothing else of the canonical record — no labels, priority, metadata,
    /// deliveries, origin or classification beyond public.
    fn view(&self, id: NativeId, project: Option<NativeId>) -> Task {
        Task {
            id,
            key: None,
            title: self.task.title.clone(),
            content: self.task.content.clone(),
            status: self.task.status.clone(),
            priority: Priority::None,
            labels: Vec::new(),
            project,
            url: None,
            location: None,
            created_at: None,
            updated_at: None,
            metadata: BTreeMap::new(),
            repositories: self.task.repositories.clone(),
            delivers: Vec::new(),
            delivered_by: Vec::new(),
            version: None,
            classification: Classification::Public,
        }
    }

    /// How a refusal names the view: by its canonical task, which only the caller reads.
    fn item(&self) -> String {
        format!("the public projection of {}", self.source)
    }
}

/// The neutral project every view of one plan in one public source is filed under, naming the
/// views' own public repositories — a board files a view only under a project issue of its
/// repository's owner — and nothing else.
fn public_project(id: NativeId, views: &[&Projected]) -> Project {
    let mut repositories: Vec<Repository> = Vec::new();
    for view in views {
        for repository in &view.task.repositories {
            if !repositories.contains(repository) {
                repositories.push(repository.clone());
            }
        }
    }
    Project {
        id,
        title: PUBLIC_PROJECT_TITLE.to_owned(),
        content: None,
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        labels: Vec::new(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories,
        version: None,
        classification: Classification::Public,
    }
}

/// The public tasks of one private plan that a project copy makes views of.
#[derive(Default)]
pub(super) struct Projecting {
    tasks: Vec<Projected>,
}

impl Projecting {
    /// Whether the plan holds no task with a public view to make.
    pub(super) fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Note `task`, landing canonically in the private home, as projected into `to`.
    pub(super) fn add(&mut self, task: &Planned, to: SourceName) {
        let Item::Task(read) = &task.item else {
            return;
        };
        self.tasks.push(Projected {
            source: task.source.clone(),
            to,
            task: (**read).clone(),
            assets: task.assets.clone(),
            depends_on: task
                .edges
                .iter()
                .filter(|edge| {
                    edge.kind == DependencyKind::Blocks && edge.to.kind == ItemKind::Task
                })
                .filter_map(|edge| far_end(&edge.to, &task.source.source))
                .collect(),
        });
    }

    /// The public sources the views land in, each once, in the order the plan reaches them.
    fn destinations(&self) -> Vec<SourceName> {
        let mut destinations: Vec<SourceName> = Vec::new();
        for task in &self.tasks {
            if !destinations.contains(&task.to) {
                destinations.push(task.to.clone());
            }
        }
        destinations
    }

    /// The views landing in `to`, every one after the views it depends on where they can be.
    fn ordered(&self, to: &SourceName) -> Vec<&Projected> {
        let here: Vec<&Projected> = self.tasks.iter().filter(|task| &task.to == to).collect();
        let mut ordered: Vec<&Projected> = Vec::new();
        let mut visiting: Vec<&GlobalId> = Vec::new();
        fn visit<'a>(
            task: &'a Projected,
            here: &[&'a Projected],
            ordered: &mut Vec<&'a Projected>,
            visiting: &mut Vec<&'a GlobalId>,
        ) {
            if ordered.iter().any(|done| done.source == task.source)
                || visiting.contains(&&task.source)
            {
                return;
            }
            visiting.push(&task.source);
            for far in &task.depends_on {
                if let Some(first) = here.iter().find(|other| &other.source == far) {
                    visit(first, here, ordered, visiting);
                }
            }
            visiting.pop();
            ordered.push(task);
        }
        for task in &here {
            visit(task, &here, &mut ordered, &mut visiting);
        }
        ordered
    }
}

impl Engine {
    /// Whether a routed project copy whose home is `home` makes public views of its public
    /// tasks: while the boundary is active, for a plan classified private whose home lands in
    /// a source declared private. Every task of such a plan then lands canonically in the home.
    pub(super) fn makes_projections(&self, home: &SourceName, class: Classification) -> bool {
        self.boundary_active()
            && class == Classification::Private
            && self.declared(home) == SourceVisibility::Private
    }

    /// Where `task`, landing canonically in `home`, is projected: the source the copy's
    /// routing places it at as public, when that is not the home and not declared private.
    pub(super) fn projection_target(
        &self,
        destination: &SourceName,
        home: &SourceName,
        task: &Planned,
    ) -> Option<SourceName> {
        let Item::Task(read) = &task.item else {
            return None;
        };
        if task.classification != Classification::Public || read.repositories.is_empty() {
            return None;
        }
        let placed =
            self.routes
                .place_classified(destination, Classification::Public, &read.repositories);
        (&placed.destination != home
            && self.declared(&placed.destination) != SourceVisibility::Private)
            .then_some(placed.destination)
    }

    /// Hold every view `projecting` would write to the public boundary before anything of the
    /// copy is written: a local link its content could not carry, a status its destination has
    /// no name for, and the caller's check of everything the view and its project put in front
    /// of a reader.
    pub(super) async fn preflight_projections(
        &self,
        projecting: &Projecting,
    ) -> Result<(), EngineError> {
        for to in projecting.destinations() {
            let destination = self.writable(&to)?;
            let project = public_project(NativeId(String::new()), &projecting.ordered(&to));
            let classification = self
                .classify(Classification::Public, &project.repositories)
                .await;
            self.preflight(
                destination,
                &Outbound {
                    item: "the public projection project".to_owned(),
                    classification,
                    target: WriteTarget::New {
                        repositories: &project.repositories,
                        project: None,
                    },
                    exposure: &Exposure::of_project(&project),
                },
            )
            .await?;
        }
        for task in &projecting.tasks {
            let names: Vec<String> = task
                .assets
                .iter()
                .map(|asset| asset.name.as_str().to_owned())
                .collect();
            if let Some(reference) =
                unresolved_references(task.task.content.as_deref().unwrap_or_default(), &names)
                    .into_iter()
                    .next()
            {
                return Err(EngineError::ProjectionReference {
                    task: task.source.to_string(),
                    reference,
                });
            }
            let destination = self.writable(&task.to)?;
            let view = task.view(NativeId(String::new()), None);
            let classification = self
                .classify(Classification::Public, &view.repositories)
                .await;
            self.preflight(
                destination,
                &Outbound {
                    item: task.item(),
                    classification,
                    target: WriteTarget::New {
                        repositories: &view.repositories,
                        project: None,
                    },
                    exposure: &Exposure::of_task(&view).assets(&task.assets),
                },
            )
            .await?;
            destination
                .source()
                .check_status_write(ItemKind::Task, view.status.category, None)
                .await
                .map_err(|error| refused(destination, error))?;
        }
        Ok(())
    }

    /// Land every view of `projecting`, once the canonical copy whose `outcomes` these are has
    /// landed in its private home, and record each in the home's map before anything else is
    /// written — so a copy run again after an interruption finds every view it made.
    pub(super) async fn land_projections(
        &self,
        projecting: &Projecting,
        outcomes: &[CopyOutcome],
        journal: &mut Journal,
    ) -> Result<(), EngineError> {
        let Some(home) = outcomes.first().and_then(CopyOutcome::destination).cloned() else {
            return Ok(());
        };
        let canonical: BTreeMap<String, GlobalId> = outcomes
            .iter()
            .filter_map(|outcome| {
                outcome
                    .destination()
                    .map(|landed| (outcome.source.to_string(), landed.clone()))
            })
            .collect();
        let at = self.writable(&home.source)?;
        let Some(held) = self.prior(at, Level::Project, &home.native).await? else {
            return Ok(());
        };
        let Item::Project(held) = &held.item else {
            return Ok(());
        };
        let mut map = self.recorded_projections(&home, &held.metadata)?;
        for to in projecting.destinations() {
            let destination = self.writable(&to)?;
            let ordered = projecting.ordered(&to);
            let project = self
                .land_public_project(destination, &home, &ordered, &canonical, &map, journal)
                .await?;
            let mut views: BTreeMap<String, NativeId> = BTreeMap::new();
            let mut incomplete = Vec::new();
            for task in &ordered {
                let Some(canonical_task) = canonical.get(&task.source.to_string()) else {
                    continue;
                };
                let complete = task.depends_on.iter().all(|far| {
                    !ordered.iter().any(|other| &other.source == far)
                        || views.contains_key(&far.to_string())
                });
                let id = self
                    .land_view(
                        destination,
                        task,
                        canonical_task,
                        &project,
                        &map,
                        &views,
                        journal,
                    )
                    .await?;
                views.insert(task.source.to_string(), id.clone());
                if map.set(canonical_task, &GlobalId::new(to.clone(), id)) {
                    self.record_projections(at, &home, &map, journal).await?;
                }
                if !complete {
                    incomplete.push(*task);
                }
            }
            // A view that depends on one landed after it — a cycle — is written again now that
            // every view it names is there.
            for task in incomplete {
                let canonical_task = &canonical[&task.source.to_string()];
                self.land_view(
                    destination,
                    task,
                    canonical_task,
                    &project,
                    &map,
                    &views,
                    journal,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// The map `metadata` of the private home `home` records, with every projection held to
    /// being somewhere not declared private.
    fn recorded_projections(
        &self,
        home: &GlobalId,
        metadata: &BTreeMap<String, Value>,
    ) -> Result<PublicProjections, EngineError> {
        let map =
            PublicProjections::read(metadata).map_err(|why| EngineError::ProjectionsMalformed {
                project: home.to_string(),
                why,
            })?;
        if let Some(entry) = map
            .tasks
            .iter()
            .find(|entry| self.declared(&entry.projection.source) == SourceVisibility::Private)
        {
            return Err(EngineError::ProjectionsMalformed {
                project: home.to_string(),
                why: format!(
                    "projection {} is in a source declared private, and a projection is public",
                    entry.projection
                ),
            });
        }
        Ok(map)
    }

    /// The public project the views of one plan in `destination` are filed under: the one an
    /// existing view already sits under, else the one this plan's opaque id names, created
    /// when it is not there.
    async fn land_public_project(
        &self,
        destination: &ResolvedSource,
        home: &GlobalId,
        ordered: &[&Projected],
        canonical: &BTreeMap<String, GlobalId>,
        map: &PublicProjections,
        journal: &mut Journal,
    ) -> Result<NativeId, EngineError> {
        for task in ordered {
            let Some(projection) = canonical
                .get(&task.source.to_string())
                .and_then(|canonical| map.projection_of(canonical))
                .filter(|projection| &projection.source == destination.name())
            else {
                continue;
            };
            if let Some(project) = destination
                .source()
                .get_task(&projection.native)
                .await
                .map_err(|error| refused(destination, error))?
                .and_then(|view| view.project)
            {
                return Ok(project);
            }
        }
        let id = opaque_id("pw", home);
        if destination
            .source()
            .get_project(&id)
            .await
            .map_err(|error| refused(destination, error))?
            .is_some()
        {
            return Ok(id);
        }
        let project = public_project(id, ordered);
        let item = "the public projection project";
        self.at_write(
            destination,
            item,
            &WriteTarget::New {
                repositories: &project.repositories,
                project: None,
            },
        )
        .await?;
        destination
            .source()
            .check_status_write(ItemKind::Project, project.status.category, None)
            .await
            .map_err(|error| refused(destination, error))?;
        let landed = destination
            .source()
            .write_project(&ItemWrite {
                target: None,
                item: project,
                depends_on: Vec::new(),
            })
            .await
            .map_err(|error| create_refused(destination, Creating::Project, error))?;
        journal.record(Undo::Created {
            at: destination.name().clone(),
            kind: Level::Project,
            id: landed.clone(),
        });
        Ok(landed)
    }

    /// Land one view: over the one the map names, or the one its opaque id names, when either
    /// is there — written only when it differs from what the canonical task now says — and
    /// created otherwise. Answers the id it is held under.
    #[allow(clippy::too_many_arguments)]
    async fn land_view(
        &self,
        destination: &ResolvedSource,
        task: &Projected,
        canonical: &GlobalId,
        project: &NativeId,
        map: &PublicProjections,
        views: &BTreeMap<String, NativeId>,
        journal: &mut Journal,
    ) -> Result<NativeId, EngineError> {
        let opaque = opaque_id("pt", canonical);
        let mut held = None;
        for candidate in map
            .projection_of(canonical)
            .filter(|projection| &projection.source == destination.name())
            .map(|projection| projection.native.clone())
            .into_iter()
            .chain(std::iter::once(opaque.clone()))
        {
            if let Some(view) = destination
                .source()
                .get_task(&candidate)
                .await
                .map_err(|error| refused(destination, error))?
                && view.project.as_ref() == Some(project)
            {
                held = Some(view);
                break;
            }
        }
        let id = held.as_ref().map_or(opaque, |view| view.id.clone());
        let view = task.view(id.clone(), Some(project.clone()));
        let edges: Vec<DependencyEdge> = task
            .depends_on
            .iter()
            .filter_map(|far| views.get(&far.to_string()))
            .map(|far| DependencyEdge {
                from: DependencyEndpoint::from_native(id.clone(), ItemKind::Task),
                to: DependencyEndpoint::from_native(far.clone(), ItemKind::Task),
                kind: DependencyKind::Blocks,
            })
            .collect();
        let target = held.as_ref().map(|view| view.id.clone());
        if let Some(held) = &held {
            let held_edges = forward_edges(destination, &held.id, Level::Task).await?;
            let mut were: Vec<&str> = held_edges.iter().map(|edge| edge.to.id()).collect();
            let mut are: Vec<&str> = edges.iter().map(|edge| edge.to.id()).collect();
            were.sort_unstable();
            are.sort_unstable();
            if held.title == view.title
                && held.content == view.content
                && held.status.category == view.status.category
                && held.repositories == view.repositories
                && were == are
            {
                return Ok(held.id.clone());
            }
        }
        let item = task.item();
        let write_target = match &target {
            Some(id) => WriteTarget::Existing(id),
            None => WriteTarget::New {
                repositories: &view.repositories,
                project: Some(project),
            },
        };
        self.at_write(destination, &item, &write_target).await?;
        destination
            .source()
            .check_status_write(ItemKind::Task, view.status.category, target.as_ref())
            .await
            .map_err(|error| refused(destination, error))?;
        if let Some(held) = held {
            let prior = self
                .prior(destination, Level::Task, &held.id)
                .await?
                .expect("the view was just read");
            journal.record(Undo::Updated {
                at: destination.name().clone(),
                id: held.id.clone(),
                prior,
            });
        }
        let write = ItemWrite {
            target: target.clone(),
            item: view,
            depends_on: edges,
        };
        let landed = if task.assets.is_empty() {
            destination.source().write_task(&write).await
        } else {
            destination
                .source()
                .write_task_with_assets(&write, None, &assets::write_of(task.assets.clone(), None))
                .await
                .map(|written| written.id)
        }
        .map_err(|error| match target {
            None => create_refused(destination, Creating::Task, error),
            Some(_) => refused(destination, error),
        })?;
        if target.is_none() {
            journal.record(Undo::Created {
                at: destination.name().clone(),
                kind: Level::Task,
                id: landed.clone(),
            });
        }
        Ok(landed)
    }

    /// Write `map` onto the private home `home` through its own project write, unless it
    /// records exactly that already; the home as it stood is what the journal restores.
    async fn record_projections(
        &self,
        at: &ResolvedSource,
        home: &GlobalId,
        map: &PublicProjections,
        journal: &mut Journal,
    ) -> Result<(), EngineError> {
        let Some(prior) = self.prior(at, Level::Project, &home.native).await? else {
            return Ok(());
        };
        let Item::Project(mut project) = prior.item.clone() else {
            return Ok(());
        };
        let value = map.to_value();
        if project.metadata.get(PublicProjections::KEY) == Some(&value) {
            return Ok(());
        }
        project
            .metadata
            .insert(PublicProjections::KEY.to_owned(), value);
        at.source()
            .check_status_write(
                ItemKind::Project,
                project.status.category,
                Some(&home.native),
            )
            .await
            .map_err(|error| refused(at, error))?;
        self.at_write(at, &home.to_string(), &WriteTarget::Existing(&home.native))
            .await?;
        let edges = prior.edges.clone();
        journal.record(Undo::Updated {
            at: home.source.clone(),
            id: home.native.clone(),
            prior,
        });
        at.source()
            .write_project(&ItemWrite {
                target: Some(home.native.clone()),
                item: *project,
                depends_on: edges,
            })
            .await
            .map_err(|error| refused(at, error))?;
        Ok(())
    }

    /// Mirror a canonical task's new status category to its public projection, through the
    /// projection's own screened status write, answering the projection when there is one.
    ///
    /// Only the category crosses: nothing of why it moved, who moved it or what ran. A task
    /// has a projection only in a source declared private while the boundary is active, filed
    /// under a project whose [`PublicProjections`] names it, so nothing else is read for any
    /// other task.
    pub(in crate::engine) async fn mirror_status(
        &self,
        task: &GlobalId,
        project: Option<&NativeId>,
        category: StatusCategory,
    ) -> Result<Option<GlobalId>, EngineError> {
        if !self.boundary_active() || self.declared(&task.source) != SourceVisibility::Private {
            return Ok(None);
        }
        let Some(project) = project else {
            return Ok(None);
        };
        let source = self.readable(&task.source)?;
        let Some(held) = source
            .source()
            .get_project(project)
            .await
            .map_err(|error| refused(source, error))?
        else {
            return Ok(None);
        };
        let home = GlobalId::new(task.source.clone(), project.clone());
        let map = self.recorded_projections(&home, &held.metadata)?;
        let Some(projection) = map.projection_of(task).cloned() else {
            return Ok(None);
        };
        Box::pin(self.set_task_status(&projection, category))
            .await
            .map_err(|error| EngineError::ProjectionNotMirrored {
                task: task.to_string(),
                projection: projection.to_string(),
                why: error.to_string(),
            })?;
        Ok(Some(projection))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(text: &str) -> GlobalId {
        text.parse().expect("a qualified id")
    }

    #[test]
    fn a_map_round_trips_and_absent_reads_as_empty() {
        let mut map = PublicProjections::default();
        assert!(map.set(&id("vault:a"), &id("site:pt-1")));
        assert!(!map.set(&id("vault:a"), &id("site:pt-1")));
        let value = map.to_value();
        assert_eq!(
            value,
            json!({"schema_version": 1, "tasks": [{"task": "vault:a", "projection": "site:pt-1"}]})
        );
        let mut metadata = BTreeMap::new();
        assert_eq!(
            PublicProjections::read(&metadata),
            Ok(PublicProjections::default())
        );
        metadata.insert(PublicProjections::KEY.to_owned(), value);
        assert_eq!(PublicProjections::read(&metadata), Ok(map));
    }

    #[test]
    fn a_malformed_map_is_refused_naming_what_is_wrong() {
        for (value, said) in [
            (
                json!({"schema_version": 2, "tasks": []}),
                "schema_version 2",
            ),
            (json!({"schema_version": 1}), "missing field"),
            (
                json!({"schema_version": 1, "tasks": [], "extra": 1}),
                "unknown field",
            ),
            (
                json!({"schema_version": 1, "tasks": [{"task": "vault:a", "projection": "site"}]}),
                "not a qualified id",
            ),
            (
                json!({"schema_version": 1, "tasks": [
                    {"task": "vault:a", "projection": "site:1"},
                    {"task": "vault:a", "projection": "site:2"}]}),
                "names task vault:a twice",
            ),
            (
                json!({"schema_version": 1, "tasks": [
                    {"task": "vault:a", "projection": "site:1"},
                    {"task": "vault:b", "projection": "site:1"}]}),
                "names projection site:1 twice",
            ),
        ] {
            let error = PublicProjections::parse(&value).expect_err("refused");
            assert!(error.contains(said), "{value}: {error}");
        }
    }

    #[test]
    fn an_opaque_id_is_stable_and_names_nothing_it_came_from() {
        let first = opaque_id("pt", &id("vault:secret-launch"));
        assert_eq!(first, opaque_id("pt", &id("vault:secret-launch")));
        assert_ne!(first, opaque_id("pt", &id("vault:other")));
        assert!(!first.as_str().contains("secret"), "{first:?}");
        assert!(first.as_str().starts_with("pt-"));
    }

    #[test]
    fn only_urls_fragments_and_held_assets_are_carried() {
        let content = "See [docs](https://example.org/x), [here](#part), \
                       ![shot](./shot.png), [mail](mailto:a@b.c), \
                       [local](../plans/secret.md), [abs](/home/me/x.md), \
                       [file](file:///etc/x) and `[code](./not-a-link.md)`.";
        assert_eq!(
            unresolved_references(content, &["shot.png".to_owned()]),
            vec!["../plans/secret.md", "/home/me/x.md", "file:///etc/x"]
        );
        assert_eq!(
            unresolved_references("![other](./other.png)", &[]),
            vec!["./other.png"]
        );
    }
}
