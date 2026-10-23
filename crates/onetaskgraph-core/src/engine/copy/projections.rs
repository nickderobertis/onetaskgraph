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

use std::collections::{BTreeMap, BTreeSet, HashMap};

use onetaskgraph_plugin_api::{
    AssetPayload, Classification, DependencyEdge, DependencyEndpoint, DependencyKind, ItemKind,
    ItemWrite, MetadataKey, NativeId, Priority, Project, Repository, SourceName, Status,
    StatusCategory, Task, WriteTarget,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::assets;
use super::super::boundary::{Exposure, Outbound};
use super::super::{Engine, EngineError};
use super::{
    CopyOutcome, Creating, Item, Journal, Level, Planned, Undo, classification_of, create_refused,
    described, far_end,
};
use super::{Prior, refused};
use crate::GlobalId;
use crate::config::SourceVisibility;
use crate::resolve::ResolvedSource;

/// The private destination project's map from each canonical task to its public projection.
///
/// Every value of it holds to the shape: its `schema_version` is
/// [`PublicProjections::SCHEMA_VERSION`] and its entries are unique by both ids. Deserializing
/// one is held to both — [`PublicProjections::parse`] names what is wrong — and the entries
/// change only through [`PublicProjections::set`] and [`PublicProjections::retain`], which keep
/// them so. Written back with [`PublicProjections::to_value`].
// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] This type is the one source, and both SDKs' models are generated from its schema; what the schema cannot carry is uniqueness by a member, which JSON Schema has no keyword for. Nothing but the engine ever writes the map — the reserved key is refused for every caller's metadata write, the SDKs' included — so a generated model only reads maps this `TryFrom` already held to that rule, and a tampered map is refused here, as the journeys prove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, try_from = "Unchecked")]
pub struct PublicProjections {
    /// The version of this shape; [`PublicProjections::SCHEMA_VERSION`] is the only one.
    #[schemars(range(min = 1, max = 1))]
    schema_version: u32,
    /// One entry per canonical task that has a public projection, unique by both ids.
    tasks: Vec<PublicProjection>,
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

/// The private destination project's map from each canonical task to its public projection,
/// as it was stored and before it is held to the shape — which is the schema a reader of it is
/// generated from, so it carries [`PublicProjections`]'s name and words.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "PublicProjections")]
struct Unchecked {
    /// The version of this shape; 1 is the only one.
    #[schemars(range(min = 1, max = 1))]
    schema_version: u32,
    /// One entry per canonical task that has a public projection, unique by both ids.
    tasks: Vec<PublicProjection>,
}

impl TryFrom<Unchecked> for PublicProjections {
    type Error = String;

    fn try_from(map: Unchecked) -> Result<Self, String> {
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
        Ok(Self {
            schema_version: map.schema_version,
            tasks: map.tasks,
        })
    }
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
        serde_json::from_value(value.clone()).map_err(|error| {
            format!(
                "{} is not {{\"schema_version\": 1, \"tasks\": [{{\"task\": …, \"projection\": \
                 …}}]}} unique by both ids: {error}",
                Self::KEY
            )
        })
    }

    /// The map as the private destination project records it.
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("the projection map serializes")
    }

    /// Every entry, in the order they were recorded.
    #[must_use]
    pub fn tasks(&self) -> &[PublicProjection] {
        &self.tasks
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

    /// Keep only the entries `keep` answers `true` for, answering whether any was dropped.
    pub fn retain(&mut self, keep: impl FnMut(&PublicProjection) -> bool) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(keep);
        self.tasks.len() != before
    }
}

/// Which view a status mirror could not reach: the one the map names, or — when the map itself
/// could not be read — whichever the map on the private project would have named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorTarget {
    /// The view the map names.
    View(GlobalId),
    /// The map recorded on this private project, which could not be read.
    MapOn(GlobalId),
}

impl std::fmt::Display for MirrorTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::View(view) => write!(formatter, "{view}"),
            Self::MapOn(project) => write!(formatter, "recorded on {project}"),
        }
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

/// What in `content` a public projection could not carry: every Markdown link or image whose
/// destination is neither a URL, a bare fragment nor one of `assets` as `./<name>`, and every
/// piece of raw HTML that could name a destination at all — a tag with any attribute, or a
/// `style` or `script` element — since a view cannot tell what such HTML points at. Code spans
/// and blocks hold neither, so nothing in them is one.
#[must_use]
pub fn unresolved_references(content: &str, assets: &[String]) -> Vec<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut unresolved: Vec<String> = Vec::new();
    let mut note = |found: String| {
        if !unresolved.contains(&found) {
            unresolved.push(found);
        }
    };
    // Text too, each run of it whole — the parser hands it over in pieces split at `<` — since
    // what CommonMark reads as text, `<img/src=…>`, another renderer may read as a tag.
    let mut text = String::new();
    // A code block's text is code, shown as written, however it reads.
    let mut in_code = false;
    for event in Parser::new_ext(content, Options::empty()) {
        match &event {
            Event::Start(Tag::CodeBlock(_)) => in_code = true,
            Event::End(TagEnd::CodeBlock) => in_code = false,
            Event::Text(_) if in_code => continue,
            Event::Text(piece) => {
                text.push_str(piece);
                continue;
            }
            _ => {}
        }
        if let Some(tag) = opaque_html(&std::mem::take(&mut text)) {
            note(tag);
        }
        match event {
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                let destination = dest_url.trim();
                if !carried(destination, assets) {
                    note(destination.to_owned());
                }
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                if let Some(tag) = opaque_html(&html) {
                    note(tag);
                }
            }
            _ => {}
        }
    }
    if let Some(tag) = opaque_html(&text) {
        note(tag);
    }
    unresolved
}

/// Whether a view can carry a link to `destination`: a URL of a scheme a reader follows over
/// the network or mail, a fragment of the view itself, or one of its own `assets`.
fn carried(destination: &str, assets: &[String]) -> bool {
    if destination.starts_with('#') {
        return true;
    }
    if let Some(name) = destination.strip_prefix("./") {
        return assets.iter().any(|asset| asset == name);
    }
    let Some((scheme, rest)) = destination.split_once(':') else {
        return false;
    };
    let named = scheme.len() > 1
        && scheme
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character));
    named
        && !scheme.eq_ignore_ascii_case("file")
        && (rest.starts_with("//") || scheme.eq_ignore_ascii_case("mailto"))
}

/// The first tag of raw HTML that could name a destination — one carrying anything past its
/// name, or opening a `style` or `script` element — as written; `None` when every tag is a bare
/// name, which names nowhere.
fn opaque_html(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        let tag = &rest[open..];
        let end = tag.find('>').map_or(tag.len(), |end| end + 1);
        let inside = tag[1..end]
            .trim_end_matches('>')
            .trim_start_matches(['/', '!', '?']);
        // Only `<` straight before a name opens a tag: `a < b` is prose.
        if !inside.starts_with(|character: char| character.is_ascii_alphabetic()) {
            rest = &tag[1..];
            continue;
        }
        let name_end = inside
            .find(|character: char| !character.is_ascii_alphanumeric() && character != '-')
            .unwrap_or(inside.len());
        let (name, after) = inside.split_at(name_end);
        let bare = after.trim().trim_end_matches('/').trim().is_empty();
        if !bare || name.eq_ignore_ascii_case("style") || name.eq_ignore_ascii_case("script") {
            return Some(tag[..end].to_owned());
        }
        rest = &tag[end..];
    }
    None
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

/// Whether `project` reads as a public projection project: public, titled
/// [`PUBLIC_PROJECT_TITLE`], with no prose of its own.
fn is_neutral(project: &Project) -> bool {
    project.title == PUBLIC_PROJECT_TITLE
        && project.classification == Classification::Public
        && project.content.as_deref().is_none_or(str::is_empty)
}

/// The `(name, sha256)` of every asset in `assets`, in name order.
fn digests(assets: &[AssetPayload]) -> Vec<(String, String)> {
    let mut digests: Vec<(String, String)> = assets
        .iter()
        .map(|asset| (asset.name.as_str().to_owned(), asset.sha256.clone()))
        .collect();
    digests.sort_unstable();
    digests
}

/// Whether one private plan's copy makes public views, and of which of its tasks.
///
/// Only an active plan holds tasks: a copy that makes no views has nowhere to note one.
#[derive(Default)]
pub(super) enum Projecting {
    /// The plan makes no view: it is not private, its home is not declared private, or it
    /// neither has a public task to show nor a map recorded before to bring in step.
    #[default]
    Inactive,
    /// The plan's public tasks to make views of — none, when the map recorded before still has
    /// to forget a task that has no view any more.
    Active(Vec<Projected>),
}

impl Projecting {
    /// A plan whose public tasks this copy makes views of, none noted yet.
    pub(super) fn active() -> Self {
        Self::Active(Vec::new())
    }

    /// Whether this copy has views to land, or a recorded map to bring in step.
    pub(super) fn lands(&self) -> bool {
        matches!(self, Self::Active(_))
    }

    /// This plan, active only when it has a view to land or `project`'s destination already
    /// records a map to bring in step — so a private plan with neither reads nothing more.
    pub(super) fn settled(self, project: &Planned) -> Self {
        let recorded = project
            .held
            .as_ref()
            .is_some_and(|held| described(&held.item).1.contains_key(PublicProjections::KEY));
        match self {
            Self::Active(tasks) if !tasks.is_empty() || recorded => Self::Active(tasks),
            Self::Active(_) | Self::Inactive => Self::Inactive,
        }
    }

    /// Every task this plan makes a view of.
    fn tasks(&self) -> &[Projected] {
        match self {
            Self::Active(tasks) => tasks,
            Self::Inactive => &[],
        }
    }

    /// Note `task`, landing canonically in the private home, as projected into `to`.
    pub(super) fn add(&mut self, task: &Planned, to: SourceName) {
        let (Self::Active(tasks), Item::Task(read)) = (self, &task.item) else {
            return;
        };
        tasks.push(Projected {
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
        for task in self.tasks() {
            if !destinations.contains(&task.to) {
                destinations.push(task.to.clone());
            }
        }
        destinations
    }

    /// The views landing in `to`, every one after the views it depends on where they can be.
    fn ordered(&self, to: &SourceName) -> Vec<&Projected> {
        let here: Vec<&Projected> = self.tasks().iter().filter(|task| &task.to == to).collect();
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

/// What every view of one plan in one public source lands against: the project they are
/// filed under, the map as recorded so far, and the views already landed by source task id.
struct Landing<'a> {
    home: &'a GlobalId,
    project: &'a NativeId,
    map: &'a PublicProjections,
    views: &'a HashMap<GlobalId, NativeId>,
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
        // A canonical task its destination already records private — an earlier copy's, an
        // earlier release's by inheritance — stays private, and has no view.
        let recorded = task
            .held
            .as_ref()
            .map_or(Classification::Public, |held| classification_of(&held.item));
        if task.classification.strictest(recorded) != Classification::Public
            || read.repositories.is_empty()
        {
            return None;
        }
        let placed =
            self.routes
                .place_classified(destination, Classification::Public, &read.repositories);
        (&placed.destination != home
            && self.declared(&placed.destination) != SourceVisibility::Private)
            .then_some(placed.destination)
    }

    /// Hold every view `projecting` would write, and the neutral project each is filed under, to
    /// the public boundary before anything of the copy is written: a local link its content
    /// could not carry, a status its destination has no name for, and the caller's check of
    /// everything the view and its project put in front of a reader.
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
            destination
                .source()
                .check_status_write(ItemKind::Project, project.status.category, None)
                .await
                .map_err(|error| refused(destination, error))?;
        }
        for task in projecting.tasks() {
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
                    task: task.source.clone(),
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
        let canonical: HashMap<GlobalId, GlobalId> = outcomes
            .iter()
            .filter_map(|outcome| {
                outcome
                    .destination()
                    .map(|landed| (outcome.source.clone(), landed.clone()))
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
        // A canonical task of this plan with no view this time — recorded private since, or no
        // longer routed anywhere public — is no longer named; its old view is left as a person
        // finds it, never deleted.
        let projected: Vec<&GlobalId> = projecting
            .tasks()
            .iter()
            .filter_map(|task| canonical.get(&task.source))
            .collect();
        if map.retain(|entry| {
            !canonical.values().any(|landed| landed == &entry.task)
                || projected.contains(&&entry.task)
        }) {
            self.record_projections(at, &home, &map, journal).await?;
        }
        for to in projecting.destinations() {
            let destination = self.writable(&to)?;
            let ordered = projecting.ordered(&to);
            let project = self
                .land_public_project(destination, &home, &ordered, &canonical, &map, journal)
                .await?;
            let mut views: HashMap<GlobalId, NativeId> = HashMap::new();
            // The tasks under the public project no map entry names, listed once when a view is
            // looked for by what it says.
            let mut unrecorded: Option<Vec<Task>> = None;
            let mut incomplete = Vec::new();
            for task in &ordered {
                let Some(canonical_task) = canonical.get(&task.source) else {
                    continue;
                };
                let complete = task.depends_on.iter().all(|far| {
                    !ordered.iter().any(|other| &other.source == far) || views.contains_key(far)
                });
                let landing = Landing {
                    home: &home,
                    project: &project,
                    map: &map,
                    views: &views,
                };
                let id = self
                    .land_view(
                        destination,
                        task,
                        canonical_task,
                        &landing,
                        &mut unrecorded,
                        journal,
                    )
                    .await?;
                views.insert(task.source.clone(), id.clone());
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
                let canonical_task = &canonical[&task.source];
                let landing = Landing {
                    home: &home,
                    project: &project,
                    map: &map,
                    views: &views,
                };
                self.land_view(
                    destination,
                    task,
                    canonical_task,
                    &landing,
                    &mut unrecorded,
                    journal,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// The map `metadata` of the private home `home` records, held to [`Engine::held_to_home`].
    fn recorded_projections(
        &self,
        home: &GlobalId,
        metadata: &BTreeMap<String, Value>,
    ) -> Result<PublicProjections, EngineError> {
        PublicProjections::read(metadata)
            .and_then(|map| self.held_to_home(home, map))
            .map_err(|why| EngineError::ProjectionsMalformed {
                project: home.clone(),
                why,
            })
    }

    /// `map` as the private home `home` may record it, or why not: every task of the source
    /// holding the home, and every projection somewhere not declared private.
    fn held_to_home(
        &self,
        home: &GlobalId,
        map: PublicProjections,
    ) -> Result<PublicProjections, String> {
        for entry in map.tasks() {
            if entry.task.source != home.source {
                return Err(format!(
                    "task {} is not of {}, the source holding the project and its canonical \
                     tasks",
                    entry.task, home.source
                ));
            }
            if self.declared(&entry.projection.source) == SourceVisibility::Private {
                return Err(format!(
                    "projection {} is in a source declared private, and a projection is public",
                    entry.projection
                ));
            }
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
        canonical: &HashMap<GlobalId, GlobalId>,
        map: &PublicProjections,
        journal: &mut Journal,
    ) -> Result<NativeId, EngineError> {
        for task in ordered {
            let Some(projection) = canonical
                .get(&task.source)
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
                return match self.not_neutral(destination, projection, &project).await? {
                    None => Ok(project),
                    Some(why) => Err(EngineError::ProjectionsMalformed {
                        project: home.clone(),
                        why,
                    }),
                };
            }
        }
        let id = opaque_id("pw", home);
        if let Some(held) = destination
            .source()
            .get_project(&id)
            .await
            .map_err(|error| refused(destination, error))?
        {
            if !is_neutral(&held) {
                return Err(EngineError::ProjectionsMalformed {
                    project: home.clone(),
                    why: format!(
                        "{}:{id} holds the id its public projection project is made under, and is \
                         not one",
                        destination.name()
                    ),
                });
            }
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

    /// Why a view `projection` filed under `project`, of `destination`, may not be written —
    /// `None` when that project is a public projection project — so a view a person moved, or a
    /// map naming somebody else's task, never redirects a write into a project that is not one.
    ///
    /// # Errors
    ///
    /// The project's read failing, as that source's failure.
    async fn not_neutral(
        &self,
        destination: &ResolvedSource,
        projection: &GlobalId,
        project: &NativeId,
    ) -> Result<Option<String>, EngineError> {
        let held = destination
            .source()
            .get_project(project)
            .await
            .map_err(|error| refused(destination, error))?;
        Ok((!held.as_ref().is_some_and(is_neutral)).then(|| {
            format!(
                "projection {projection} is filed under {}:{project}, which is not a public \
                 projection project",
                destination.name()
            )
        }))
    }

    /// Land one view: over the one the map names, or the one its opaque id names, when either
    /// is there — written only when it differs from what the canonical task now says — and
    /// created otherwise. Answers the id it is held under.
    ///
    /// A view this copy made and never learned the id of — its create's answer lost, at a
    /// destination that issues ids of its own and so ignores the opaque one — is found by what
    /// it says instead: the one task under the public project no map entry names that has the
    /// view's title and repositories. Two such tasks are ambiguous, and neither is taken.
    async fn land_view(
        &self,
        destination: &ResolvedSource,
        task: &Projected,
        canonical: &GlobalId,
        landing: &Landing<'_>,
        unrecorded: &mut Option<Vec<Task>>,
        journal: &mut Journal,
    ) -> Result<NativeId, EngineError> {
        let opaque = opaque_id("pt", canonical);
        // What the view now holds, read once: it is what is compared, and what the journal puts
        // back if the copy cannot finish.
        let mut held: Option<(Task, Prior)> = None;
        for candidate in landing
            .map
            .projection_of(canonical)
            .filter(|projection| &projection.source == destination.name())
            .map(|projection| projection.native.clone())
            .into_iter()
            .chain(std::iter::once(opaque.clone()))
        {
            if let Some(prior) = self.prior(destination, Level::Task, &candidate).await?
                && let Item::Task(view) = &prior.item
                && view.project.as_ref() == Some(landing.project)
            {
                held = Some(((**view).clone(), prior));
                break;
            }
        }
        if held.is_none() {
            let listed = match unrecorded {
                Some(listed) => listed,
                None => unrecorded.insert(
                    self.project_member_tasks(
                        &GlobalId::new(destination.name().clone(), landing.project.clone()),
                        "for a public projection",
                    )
                    .await?
                    .into_iter()
                    .map(|task| task.item)
                    .filter(|view| {
                        !landing.map.tasks().iter().any(|entry| {
                            &entry.projection.source == destination.name()
                                && entry.projection.native == view.id
                        }) && !landing.views.values().any(|landed| landed == &view.id)
                    })
                    .collect(),
                ),
            };
            let mut matching = listed.iter().filter(|view| {
                view.title == task.task.title && view.repositories == task.task.repositories
            });
            if let (Some(found), None) = (matching.next(), matching.next()) {
                let found = found.id.clone();
                listed.retain(|view| view.id != found);
                if let Some(prior) = self.prior(destination, Level::Task, &found).await?
                    && let Item::Task(view) = &prior.item
                {
                    held = Some(((**view).clone(), prior));
                }
            }
        }
        if let Some((view, _)) = &held
            && view.classification == Classification::Private
        {
            return Err(EngineError::ProjectionsMalformed {
                project: landing.home.clone(),
                why: format!(
                    "view {}:{} is recorded private, and a view is public; nothing loosens a \
                     private record",
                    destination.name(),
                    view.id
                ),
            });
        }
        let id = held.as_ref().map_or(opaque, |(view, _)| view.id.clone());
        let view = task.view(id.clone(), Some(landing.project.clone()));
        let edges: Vec<DependencyEdge> = task
            .depends_on
            .iter()
            .filter_map(|far| landing.views.get(far))
            .map(|far| DependencyEdge {
                from: DependencyEndpoint::from_native(id.clone(), ItemKind::Task),
                to: DependencyEndpoint::from_native(far.clone(), ItemKind::Task),
                kind: DependencyKind::Blocks,
            })
            .collect();
        let target = held.as_ref().map(|(view, _)| view.id.clone());
        if let Some((held, prior)) = &held {
            let mut were: Vec<&str> = prior.edges.iter().map(|edge| edge.to.id()).collect();
            let mut are: Vec<&str> = edges.iter().map(|edge| edge.to.id()).collect();
            were.sort_unstable();
            are.sort_unstable();
            // Every member a view is written with — what a person may have edited in public
            // included — so the next copy puts back whatever differs. A destination's own
            // reserved keys are its bookkeeping, never a field of the view.
            let own_keys = held
                .metadata
                .keys()
                .any(|key| !key.starts_with(&format!("{}.", MetadataKey::RESERVED_NAMESPACE)));
            if held.title == view.title
                && held.content == view.content
                && held.status.category == view.status.category
                && held.repositories == view.repositories
                && held.labels.is_empty()
                && held.priority == Priority::None
                && held.delivers.is_empty()
                && held.delivered_by.is_empty()
                && !own_keys
                && were == are
                && digests(prior.assets.as_deref().unwrap_or_default()) == digests(&task.assets)
            {
                return Ok(held.id.clone());
            }
        }
        let item = task.item();
        let write_target = match &target {
            Some(id) => WriteTarget::Existing(id),
            None => WriteTarget::New {
                repositories: &view.repositories,
                project: Some(landing.project),
            },
        };
        self.at_write(destination, &item, &write_target).await?;
        destination
            .source()
            .check_status_write(ItemKind::Task, view.status.category, target.as_ref())
            .await
            .map_err(|error| refused(destination, error))?;
        let recorded = held.as_ref().and_then(|(_, prior)| prior.recorded.clone());
        // A view carries assets when its task references any, and when the view it overwrites
        // holds any: what lands then holds exactly the task's, none included.
        let carrying = !task.assets.is_empty()
            || held
                .as_ref()
                .is_some_and(|(_, prior)| prior.assets.is_some());
        if let Some((held, mut prior)) = held {
            if !task.assets.is_empty() && prior.assets.is_none() {
                // Putting the view back then has to take away the assets this write adds.
                prior.assets = Some(Vec::new());
            }
            journal.record(Undo::Updated {
                at: destination.name().clone(),
                id: held.id,
                prior,
            });
        }
        let write = ItemWrite {
            target: target.clone(),
            item: view,
            depends_on: edges,
        };
        let landed = if !carrying {
            destination.source().write_task(&write).await
        } else {
            destination
                .source()
                .write_task_with_assets(
                    &write,
                    None,
                    &assets::write_of(task.assets.clone(), recorded),
                )
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
    /// has a projection only in a source declared private while the boundary is active, in a
    /// store where some source routes — a projection is made only by a routed copy — filed
    /// under a project whose [`PublicProjections`] names it; so nothing is read for any other
    /// task, and a store with no route spends what it did before.
    pub(in crate::engine) async fn mirror_status(
        &self,
        task: &GlobalId,
        held: &Task,
        category: StatusCategory,
    ) -> Result<Option<GlobalId>, EngineError> {
        if !self.boundary_active()
            || !self.routes.any()
            || self.declared(&task.source) != SourceVisibility::Private
        {
            return Ok(None);
        }
        // A task private by its record or any of its repositories has no view to hand anything
        // to, whatever a map says.
        if self.classify(held.classification, &held.repositories).await == Classification::Private {
            return Ok(None);
        }
        let Some(project) = held.project.as_ref() else {
            return Ok(None);
        };
        // The canonical status has landed by now, so anything that keeps it from reaching the
        // view is reported as a mirror that did not happen, never as a write that did not.
        let not_mirrored =
            |projection: MirrorTarget, why: String| EngineError::ProjectionNotMirrored {
                task: task.clone(),
                projection: Box::new(projection),
                why,
            };
        let home = GlobalId::new(task.source.clone(), project.clone());
        let recorded_on = MirrorTarget::MapOn(home.clone());
        let source = self.readable(&task.source)?;
        let Some(record) = source
            .source()
            .get_project(project)
            .await
            .map_err(|error| {
                not_mirrored(recorded_on.clone(), refused(source, error).to_string())
            })?
        else {
            return Ok(None);
        };
        let map = PublicProjections::read(&record.metadata)
            .and_then(|map| self.held_to_home(&home, map))
            .map_err(|why| not_mirrored(recorded_on.clone(), why))?;
        let Some(projection) = map.projection_of(task).cloned() else {
            return Ok(None);
        };
        // The view has to be one: a task filed under a public projection project.
        let named = projection.to_string();
        let view = MirrorTarget::View(projection.clone());
        let public = self
            .writable(&projection.source)
            .map_err(|error| not_mirrored(view.clone(), error.to_string()))?;
        let filed = public
            .source()
            .get_task(&projection.native)
            .await
            .map_err(|error| not_mirrored(view.clone(), refused(public, error).to_string()))?
            .ok_or_else(|| not_mirrored(view.clone(), format!("no task {named} is there")))?
            .project
            .ok_or_else(|| {
                not_mirrored(view.clone(), format!("{named} is filed under no project"))
            })?;
        if let Some(why) = self
            .not_neutral(public, &projection, &filed)
            .await
            .map_err(|error| not_mirrored(view.clone(), error.to_string()))?
        {
            return Err(not_mirrored(view, why));
        }
        Box::pin(self.set_task_status(&projection, category))
            .await
            .map_err(|error| not_mirrored(view.clone(), error.to_string()))?;
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
    fn only_urls_fragments_held_assets_and_bare_html_are_carried() {
        let content = "See [docs](https://example.org/x), [here](#part), \
                       ![shot](./shot.png), [mail](mailto:a@b.c), \
                       [local](../plans/secret.md), [abs](/home/me/x.md), \
                       [file](file:///etc/x) and `[code](./not-a-link.md)` and \
                       `<a href=\"../code.md\">`.\n\n```\n<img src=\"../block.png\">\n```\n\n\
                       <details>\n<summary>More</summary>\n\nText<br/>\n\n</details>\n";
        assert_eq!(
            unresolved_references(content, &["shot.png".to_owned()]),
            vec!["../plans/secret.md", "/home/me/x.md", "file:///etc/x"]
        );
        assert_eq!(
            unresolved_references("![other](./other.png)", &[]),
            vec!["./other.png"]
        );
        for html in [
            "<a href=\"https://example.org/y\">y</a>",
            "<a title=\">\" href=\"../private.md\">x</a>",
            "<img/src=\"../private.png\">",
            "<img srcset=\"a.png 1x, ../b.png 2x\">",
            "<div style=\"background: u\\72l('../c.png')\">c</div>",
            "<style>body { background: url(../d.png) }</style>",
            "<a ping=\"../e\">e</a>",
        ] {
            // Prose comparing two things is no tag.
            assert!(unresolved_references("Keep a < b and c > d.", &[]).is_empty());
            assert!(
                !unresolved_references(&format!("Text {html} text."), &[]).is_empty(),
                "{html}"
            );
        }
    }
}
