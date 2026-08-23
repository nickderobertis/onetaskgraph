//! A onetaskgraph source over Markdown task files on the local filesystem.
//!
//! The factory is real from this commit on so the registry can name `local-md`
//! alongside every other plugin, and `onetaskgraph schema` can emit this
//! plugin's configuration schema. Only [`SourcePlugin::build`] is outstanding:
//! implementing this source is an **additive** change to this one crate, with no
//! edit to the contract, the registry, or any sibling.
#![deny(missing_docs)]

use onetaskgraph_plugin_api::{SecretResolver, SourceError, SourceName, SourcePlugin, TaskSource};
use schemars::{Schema, schema_for};
use serde::Deserialize;

/// The plugin kind a `local-md` source's `plugin:` field names.
pub const KIND: &str = "local-md";

/// The configuration block a `local-md` source is built from.
///
/// Empty until the source lands: an empty schema accepts nothing but `{}`, so a
/// configuration written against a shape this plugin does not yet have is
/// rejected at load rather than silently ignored.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LocalMdConfig {}

/// The factory for the local-md source.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn config_schema(&self) -> Schema {
        schema_for!(LocalMdConfig)
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
