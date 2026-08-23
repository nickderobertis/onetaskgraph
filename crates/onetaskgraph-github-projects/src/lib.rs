//! A onetaskgraph source over GitHub Projects.
//!
//! The factory is real from this commit on so the registry can name `github-projects`
//! alongside every other plugin, and `onetaskgraph schema` can emit this
//! plugin's configuration schema. Only [`SourcePlugin::build`] is outstanding:
//! implementing this source is an **additive** change to this one crate, with no
//! edit to the contract, the registry, or any sibling.
#![deny(missing_docs)]

use onetaskgraph_plugin_api::{SecretResolver, SourceError, SourceName, SourcePlugin, TaskSource};
use schemars::{Schema, schema_for};
use serde::Deserialize;

/// The plugin kind a `github-projects` source's `plugin:` field names.
pub const KIND: &str = "github-projects";

/// The configuration block a `github-projects` source is built from.
///
/// Empty until the source lands: an empty schema accepts nothing but `{}`, so a
/// configuration written against a shape this plugin does not yet have is
/// rejected at load rather than silently ignored.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GitHubProjectsConfig {}

/// The factory for the github-projects source.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn config_schema(&self) -> Schema {
        schema_for!(GitHubProjectsConfig)
    }

    fn build(
        &self,
        name: &SourceName,
        _config: &serde_json::Value,
        _secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        Err(SourceError::Config {
            message: format!(
                "source {name}: the `{KIND}` plugin is not implemented yet; remove this \
                 source from your configuration, or use the `in-memory` plugin until it \
                 lands"
            ),
        })
    }
}
