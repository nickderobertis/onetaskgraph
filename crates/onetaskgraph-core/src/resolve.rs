//! Turning a configuration into live sources.
//!
//! Two steps, deliberately separable. [`validate_sources`] runs at load, for every
//! verb, and refuses a source whose `config:` block does not match the schema its
//! plugin declares — the plugin itself is already one this build has, because
//! [`SourceConfig::plugin`] is a [`PluginKind`](crate::PluginKind) and no other kind
//! can be represented. [`resolve`] then
//! builds the sources a command actually needs. The order is the point: a typo in a
//! per-source field is refused while the user is still looking at the file that
//! caused it, rather than surfacing as a confusing failure inside the first HTTP
//! call that source makes.

use std::fmt;

use jsonschema::error::ValidationErrorKind;
use onetaskgraph_plugin_api::{SecretResolver, SourceName, SourcePlugin, TaskSource};
use serde_json::Value;

use crate::PluginKind;
use crate::config::{Config, ConfigError, SourceConfig};

/// One configured source, built and ready to answer.
///
/// Held behind its accessors, and constructible only by [`resolve`], because `kind` is a
/// claim about `source` rather than a value beside it: a caller that could write the two
/// independently could say `linear` over a source that reports `local-md`, and the plan a
/// query reports names the kind. Building it where the plugin builds the source is what
/// makes the pair an invariant instead of something every reader has to re-check.
pub struct ResolvedSource {
    name: SourceName,
    kind: PluginKind,
    source: Box<dyn TaskSource>,
}

impl ResolvedSource {
    /// The name the configuration gave it, which qualifies every id it returns.
    #[must_use]
    pub fn name(&self) -> &SourceName {
        &self.name
    }

    /// The plugin kind that built it.
    #[must_use]
    pub fn kind(&self) -> PluginKind {
        self.kind
    }

    /// The source itself.
    #[must_use]
    pub fn source(&self) -> &dyn TaskSource {
        self.source.as_ref()
    }
}

impl fmt::Debug for ResolvedSource {
    /// Name and kind, the kind spelled the way a configuration spells it rather than
    /// the way Rust spells the variant. A live source has no meaningful `Debug` of its
    /// own, and one that did would be a rendering of a user's work — which nothing
    /// outside the plugin may hold.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedSource")
            .field("name", &self.name)
            .field("kind", &self.kind.as_str())
            .finish_non_exhaustive()
    }
}

/// Check every configured source without building any of them.
///
/// # Errors
///
/// Returns [`ConfigError::Setting`] naming `sources.<name>.config...` for a block that
/// does not match its plugin's declared schema.
pub fn validate_sources(config: &Config) -> Result<(), ConfigError> {
    for (name, source) in config.sources() {
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
        .sources()
        .iter()
        .map(|(name, source)| {
            let plugin = checked_plugin(name, source)?;
            let built = plugin
                .build(name, source.config(), secrets)
                .map_err(|error| {
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
                kind: source.plugin(),
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
    let plugin = source.plugin().plugin();
    check_block(name, source.config(), plugin.as_ref())?;
    Ok(plugin)
}

/// Check one source's `config:` block against the schema its plugin declares.
fn check_block(
    name: &SourceName,
    block: &Value,
    plugin: &dyn SourcePlugin,
) -> Result<(), ConfigError> {
    // A plugin's own schema is this build's, not a user's, so a schema that will not
    // compile is a defect in this binary rather than something a user did. It is still
    // reported rather than panicked on: a user whose one broken source is a plugin they
    // do not use can drop that source and carry on, which a panic would not let them do.
    // `every_registered_plugin_declares_a_schema_that_compiles_and_accepts_a_valid_block`
    // is what keeps it from reaching anybody in the first place.
    let schema = plugin.config_schema();
    let validator = jsonschema::validator_for(schema.as_value()).map_err(|error| {
        ConfigError::setting(
            format!("sources.{name}.plugin"),
            format!(
                "the `{}` plugin declares a configuration schema this build cannot \
                 compile: {error}",
                plugin.kind()
            ),
            "that is a defect in this binary rather than in your configuration — please \
             report it, naming the plugin above. Removing that source lets the rest of \
             this configuration run in the meantime.",
        )
    })?;

    let Some(problem) = validator.iter_errors(block).next() else {
        return Ok(());
    };

    // A plugin whose source is not written yet declares a schema with no properties at
    // all, which forbids every field — and a validator has nothing to say about that
    // beyond "false schema does not allow 7", which names neither the field nor the
    // reason. Both are worth saying plainly.
    if schema.as_value().get("properties").is_none()
        && let Some(fields) = block.as_object()
        && let Some(first) = fields.keys().next()
    {
        return Err(ConfigError::setting(
            format!("sources.{name}.config.{first}"),
            format!(
                "the `{}` plugin declares no configuration fields, so its `config:` block \
                 must be empty or absent; this one sets {}",
                plugin.kind(),
                fields.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
            format!(
                "remove those fields — `onetaskgraph schema` prints what this plugin \
                 accepts under `plugin_config.{}`.",
                plugin.kind()
            ),
        ));
    }

    // A validator reports an unexpected field against the *object* that holds it, so
    // the path alone would name the block and leave the user to find the field inside
    // the message. The field is the whole of what they have to go and fix, so it is
    // lifted into the key.
    let pointer = problem.instance_path().to_string().replace('/', ".");
    let unexpected = match problem.kind() {
        ValidationErrorKind::AdditionalProperties { unexpected } => unexpected.first(),
        _ => None,
    };
    let key = match unexpected {
        Some(field) => format!("sources.{name}.config{pointer}.{field}"),
        None => format!("sources.{name}.config{pointer}"),
    };
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
