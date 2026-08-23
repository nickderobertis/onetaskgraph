//! The configuration block this plugin builds a source from.

use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencySupport, Label, Project, Support, Task,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, de::Error as _};

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
    // llmlint: ignore[invalid_states_unrepresentable] this field's wire shape is frozen
    // by the plugin contract every source is written against; only the contract's owner
    // may change it, and tightening it is post-build follow-up.
    /// The largest page this source will serve. Rejected at zero: a source that will
    /// serve no rows cannot be paged, and silently treating it as one hides a typo in a
    /// configuration file behind behaviour that looks deliberate.
    #[serde(deserialize_with = "non_zero_page_size")]
    pub max_page_size: u32,
}

/// Reject a zero page ceiling where the configuration is read, so an invalid declaration
/// never reaches the source.
fn non_zero_page_size<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let value = u32::deserialize(deserializer)?;
    if value == 0 {
        return Err(D::Error::custom(
            "max_page_size must be at least 1; a source that serves no rows cannot be paged",
        ));
    }
    Ok(value)
}

/// The default page ceiling when a configuration block does not set one.
const DEFAULT_MAX_PAGE_SIZE: u32 = 100;

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
            max_page_size: value.max_page_size,
        }
    }
}
