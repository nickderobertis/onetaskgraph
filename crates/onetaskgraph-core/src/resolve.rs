//! Turning a configuration into live sources.
//!
//! Two steps, deliberately separable. [`validate_sources`] runs at load, for every
//! verb, and refuses a source whose `plugin:` names nothing this build has or whose
//! `config:` block does not match the schema that plugin declares. [`resolve`] then
//! builds the sources a command actually needs. The order is the point: a typo in a
//! per-source field is refused while the user is still looking at the file that
//! caused it, rather than surfacing as a confusing failure inside the first HTTP
//! call that source makes.

use onetaskgraph_plugin_api::{SecretResolver, SourceName, SourcePlugin, TaskSource};
use serde_json::Value;

use crate::config::{Config, ConfigError, SourceConfig};
use crate::registry::{plugin_for, plugin_kinds};

/// One configured source, built and ready to answer.
pub struct ResolvedSource {
    /// The name the configuration gave it, which qualifies every id it returns.
    pub name: SourceName,
    /// The plugin kind that built it.
    pub kind: &'static str,
    /// The source itself.
    pub source: Box<dyn TaskSource>,
}

/// Check every configured source without building any of them.
///
/// # Errors
///
/// Returns [`ConfigError::Setting`] naming `sources.<name>.plugin` for a plugin this
/// build does not have, and `sources.<name>.config...` for a block that does not
/// match that plugin's declared schema.
pub fn validate_sources(config: &Config) -> Result<(), ConfigError> {
    for (name, source) in &config.sources {
        checked_plugin(name, source)?;
    }
    Ok(())
}

/// Build every configured source, in name order.
///
/// The order is the map's, so two runs over one configuration produce the same
/// sources in the same sequence — which is what makes a multi-source result stable
/// enough to page through.
///
/// # Errors
///
/// Returns what [`validate_sources`] returns, and [`ConfigError::Setting`] naming
/// `sources.<name>` when the plugin itself refuses to build the source.
pub fn resolve(
    config: &Config,
    secrets: &dyn SecretResolver,
) -> Result<Vec<ResolvedSource>, ConfigError> {
    config
        .sources
        .iter()
        .map(|(name, source)| {
            let plugin = checked_plugin(name, source)?;
            let built = plugin.build(name, &source.config, secrets).map_err(|error| {
                ConfigError::setting(
                    format!("sources.{name}"),
                    error.to_string(),
                    format!(
                        "correct that source's configuration, or remove it — \
                         `onetaskgraph config show` reports every setting under \
                         `sources.{name}` and the layer it came from."
                    ),
                )
            })?;
            Ok(ResolvedSource {
                name: name.clone(),
                kind: plugin.kind(),
                source: built,
            })
        })
        .collect()
}

/// The plugin this source names, with its `config:` block already checked.
fn checked_plugin(
    name: &SourceName,
    source: &SourceConfig,
) -> Result<Box<dyn SourcePlugin>, ConfigError> {
    let Some(plugin) = plugin_for(&source.plugin) else {
        return Err(ConfigError::setting(
            format!("sources.{name}.plugin"),
            format!("no plugin named {:?} is built into this binary", source.plugin),
            format!("use one of: {}.", plugin_kinds().join(", ")),
        ));
    };
    check_block(name, &source.config, &plugin)?;
    Ok(plugin)
}

/// Check one source's `config:` block against the schema its plugin declares.
fn check_block(
    name: &SourceName,
    block: &Value,
    plugin: &dyn SourcePlugin,
) -> Result<(), ConfigError> {
    // A plugin's own schema is this build's, not a user's, so a schema that will not
    // compile is a bug in this binary rather than something a user can act on. The
    // registry-wide test `every_registered_plugin_declares_a_schema_that_compiles`
    // is what keeps that from reaching anybody: it fails the gate, here it panics.
    let schema = plugin.config_schema();
    let validator = jsonschema::validator_for(schema.as_value())
        .expect("every registered plugin declares a schema that compiles");

    let Some(problem) = validator.iter_errors(block).next() else {
        return Ok(());
    };
    let pointer = problem.instance_path.to_string();
    let key = format!(
        "sources.{name}.config{}",
        pointer.replace('/', ".").trim_end_matches('.')
    );
    Err(ConfigError::setting(
        key,
        problem.to_string(),
        format!(
            "check that field against the `{}` plugin's schema — `onetaskgraph schema` \
             prints it under `plugin_config.{}`.",
            plugin.kind(),
            plugin.kind()
        ),
    ))
}
