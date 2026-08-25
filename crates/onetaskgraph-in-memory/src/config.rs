//! The configuration block this plugin builds a source from.

use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencySupport, Label, NativeId, Project, Support, Task,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, de::Error as _};
use std::collections::BTreeSet;
use std::num::NonZeroU32;

/// Everything an in-memory source serves, plus what it claims it can do.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct InMemoryConfig {
    /// What this source declares — including its [`DependencySupport`], so two
    /// sources over the same graph can differ deliberately.
    pub capabilities: CapabilityConfig,
    /// The tasks this source serves.
    pub tasks: Vec<Task>,
    /// The projects this source serves.
    pub projects: Vec<Project>,
    /// Every label this source knows.
    pub labels: Vec<Label>,
    /// Forward task dependency edges: `from` depends on `to`.
    pub task_dependencies: Vec<DependencyEdge>,
    /// Forward project dependency edges: `from` depends on `to`.
    pub project_dependencies: Vec<DependencyEdge>,
}

impl InMemoryConfig {
    /// Refuse a configuration this source could not serve coherently.
    ///
    /// [`build`](crate::Plugin::build) receives a `serde_json::Value` straight out of a
    /// user's configuration file, and shape is all serde can check. It accepts two tasks
    /// sharing an id, a task filed under a project that does not exist, and a dependency
    /// edge pointing at nothing — each of which is a query that later answers *wrongly*
    /// rather than loudly: a duplicate id makes `get_task` return whichever copy comes
    /// first, and a dangling edge makes a dependency walk come back short. Neither looks
    /// like an error at the call site, which is what makes them worth refusing here.
    ///
    /// [`SourceError::Config`](onetaskgraph_plugin_api::SourceError::Config) is the
    /// contract's variant for exactly this, so the refusal reaches the user while they
    /// are still looking at the file that caused it.
    ///
    /// Every problem is collected rather than just the first: someone correcting a
    /// configuration wants the whole list, not one round trip per typo.
    ///
    /// # Errors
    ///
    /// Returns every incoherence found, joined into one message.
    pub fn validate(&self) -> Result<(), String> {
        let mut problems = Vec::new();

        for (noun, duplicates) in [
            (
                "tasks",
                duplicate_ids(self.tasks.iter().map(|task| &task.id)),
            ),
            (
                "projects",
                duplicate_ids(self.projects.iter().map(|project| &project.id)),
            ),
            (
                "labels",
                duplicate_ids(self.labels.iter().map(|label| &label.id)),
            ),
        ] {
            for id in duplicates {
                problems.push(format!(
                    "two or more {noun} share the id {id}; ids address one item each, so a \
                     duplicate makes which one a lookup returns arbitrary"
                ));
            }
        }

        let task_ids: BTreeSet<&NativeId> = self.tasks.iter().map(|task| &task.id).collect();
        let project_ids: BTreeSet<&NativeId> =
            self.projects.iter().map(|project| &project.id).collect();

        for task in &self.tasks {
            if let Some(project) = &task.project
                && !project_ids.contains(project)
            {
                problems.push(format!(
                    "task {} is filed under project {project}, which this source does not hold",
                    task.id
                ));
            }
        }

        for (noun, edges, known) in [
            ("task", &self.task_dependencies, &task_ids),
            ("project", &self.project_dependencies, &project_ids),
        ] {
            for edge in edges {
                for (end, endpoint) in [("from", &edge.from), ("to", &edge.to)] {
                    // A qualified endpoint deliberately names an item of another source,
                    // which this one cannot hold and must not be asked to. A native id
                    // containing a colon is still this source's own, so it is checked.
                    let native = NativeId(endpoint.id().to_owned());
                    if !endpoint.is_qualified() && !known.contains(&native) {
                        problems.push(format!(
                            "a {noun} dependency edge's `{end}` names {endpoint}, which this source \
                             does not hold"
                        ));
                    }
                }
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("; "))
        }
    }
}

/// Every id that appears more than once, in a stable order so a message does not reshuffle
/// between runs.
fn duplicate_ids<'a>(ids: impl Iterator<Item = &'a NativeId>) -> Vec<&'a NativeId> {
    let mut seen = BTreeSet::new();
    let mut repeated = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            repeated.insert(id);
        }
    }
    repeated.into_iter().collect()
}

/// The declared capability block, mirroring [`Capabilities`] field for field.
///
/// This is the load-bearing part of the plugin: the engine's compensation — and
/// its emulated reverse-dependency scan in particular — can only be exercised
/// against two sources of deliberately different capability if capability is
/// something a test can set. So it is configuration, not a constant.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityConfig {
    /// Whether this source has projects.
    pub projects: Support,
    /// Whether this source can select tasks belonging to no project.
    pub orphan_tasks: Support,
    /// Whether this source filters by label itself.
    pub filter_by_label: Support,
    /// Whether this source filters by status itself.
    pub filter_by_status: Support,
    /// Whether this source searches titles itself.
    pub search_title: Support,
    /// Whether this source searches bodies itself.
    pub search_content: Support,
    /// How far this source walks task dependencies.
    pub task_dependencies: DependencySupport,
    /// How far this source walks project dependencies.
    pub project_dependencies: DependencySupport,
    /// The largest page this source will serve.
    ///
    /// [`NonZeroU32`] rather than a validated `u32`: a source that will serve no rows
    /// cannot be paged, and silently treating a zero as one hides a typo in a
    /// configuration file behind behaviour that looks deliberate. Deserialization
    /// refuses a zero, and so does every other way of building this value — the
    /// contract's own `Capabilities::max_page_size` is a `u32` this widens into, so
    /// the invalid state exists only on the far side of that frozen field.
    ///
    /// Read through [`non_zero_page_size`] rather than serde's own `NonZeroU32`
    /// support, which reports "expected a nonzero u32" without naming the field —
    /// and the field is the whole of what a user has to go and fix.
    #[serde(deserialize_with = "non_zero_page_size")]
    pub max_page_size: NonZeroU32,
}

/// Refuse a zero page ceiling, naming the setting a user has to correct.
fn non_zero_page_size<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NonZeroU32, D::Error> {
    NonZeroU32::new(u32::deserialize(deserializer)?).ok_or_else(|| {
        D::Error::custom(
            "max_page_size must be at least 1; a source that serves no rows cannot be paged",
        )
    })
}

/// The default page ceiling when a configuration block does not set one.
const DEFAULT_MAX_PAGE_SIZE: NonZeroU32 = NonZeroU32::new(100).expect("100 is not zero");

impl Default for CapabilityConfig {
    fn default() -> Self {
        Self {
            projects: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: DEFAULT_MAX_PAGE_SIZE,
        }
    }
}

impl From<&CapabilityConfig> for Capabilities {
    fn from(value: &CapabilityConfig) -> Self {
        Self {
            projects: value.projects,
            orphan_tasks: value.orphan_tasks,
            filter_by_label: value.filter_by_label,
            filter_by_status: value.filter_by_status,
            search_title: value.search_title,
            search_content: value.search_content,
            task_dependencies: value.task_dependencies,
            project_dependencies: value.project_dependencies,
            max_page_size: value.max_page_size.get(),
        }
    }
}
