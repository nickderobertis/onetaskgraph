//! Configuring a source that is another program.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{SecretResolver, SourceError, SourceName, SourcePlugin, TaskSource};
use schemars::{JsonSchema, Schema, schema_for};
use secrecy::ExposeSecret;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::source::SubprocessSource;

/// The name a configuration document's `plugin:` field names this kind by.
pub(crate) const KIND: &str = "subprocess";

/// How to run a plugin that speaks `docs/plugin-protocol.md`.
///
/// `settings` is the seam that keeps this one plugin general: it is handed to the child
/// as its `config:` block verbatim, so what a Python source needs and what a Rust one
/// needs are that child's business rather than a field of this schema. Which is also why
/// the credentials a child may see are *named* here rather than inherited: §3.1 forbids a
/// plugin reading credentials from its own environment, and forwarding the engine's whole
/// environment would hand every plugin every secret on the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubprocessConfig {
    /// The program to run.
    #[serde(deserialize_with = "a_program")]
    #[schemars(regex(pattern = r"\S"))]
    pub command: String,
    /// Arguments to run it with.
    #[serde(default)]
    pub args: Vec<String>,
    /// The environment variables whose resolved values the handshake forwards.
    #[serde(default, deserialize_with = "variable_names")]
    #[schemars(inner(regex(pattern = r"^[A-Za-z_][A-Za-z0-9_]*$")))]
    pub secrets: Vec<String>,
    /// This source's own settings, handed to the child verbatim.
    #[serde(default)]
    pub settings: Value,
}

/// The factory a configuration reaches through `plugin: subprocess`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn config_schema(&self) -> Schema {
        schema_for!(SubprocessConfig)
    }

    fn build(
        &self,
        name: &SourceName,
        config: &Value,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let config: SubprocessConfig =
            serde_json::from_value(config.clone()).map_err(|error| SourceError::Config {
                message: format!("source {name}: {error}"),
            })?;
        let forwarded = resolve_named(name, &config.secrets, secrets)?;
        SubprocessSource::connect(
            &config.command,
            &config.args,
            name,
            &config.settings,
            forwarded,
        )
        .map(|source| Box::new(source) as Box<dyn TaskSource>)
    }
}

/// Refuse a `command` that names no program, where the configuration is read.
///
/// A blank command is not a source that fails later; it is a source that was never
/// configured, and the difference matters because the failure it would otherwise cause is
/// a spawn error about an empty path rather than a sentence naming the field to fill in.
fn a_program<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        return Err(D::Error::custom(
            "`command` must name the program that serves this source",
        ));
    }
    Ok(value)
}

/// Refuse anything that could not be an environment variable's name.
///
/// The engine resolves these names through the process environment and the credentials
/// file before spawning anything, and a name neither could ever hold — an empty string, a
/// value with an `=` or a space in it — is a typing mistake whose only symptom later is a
/// credential reported absent. Saying so at the field is the whole of the fix.
fn variable_names<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let names = Vec::<String>::deserialize(deserializer)?;
    for name in &names {
        let mut characters = name.chars();
        let usable = characters
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
            && characters.all(|rest| rest.is_ascii_alphanumeric() || rest == '_');
        if !usable {
            return Err(D::Error::custom(format!(
                "{name:?} is not an environment variable name; `secrets` names variables \
                 such as LINEAR_API_KEY, whose values the handshake forwards"
            )));
        }
    }
    Ok(names)
}

/// The values of exactly the variables this configuration names, and nothing else.
///
/// A named variable nothing defines is refused here rather than forwarded as absent: the
/// plugin asked for it, so a run that spawned anyway would fail later inside the child
/// with a message about a credential, and the thing the user has to fix is on this side.
fn resolve_named(
    name: &SourceName,
    named: &[String],
    secrets: &dyn SecretResolver,
) -> Result<BTreeMap<String, String>, SourceError> {
    let mut forwarded = BTreeMap::new();
    for variable in named {
        let value = secrets.get(variable).ok_or_else(|| SourceError::Auth {
            message: format!(
                "source {name}: nothing defines {variable}, which this source's `secrets` \
                 names; export it, or add it to the credentials file"
            ),
        })?;
        forwarded.insert(variable.clone(), value.expose_secret().to_owned());
    }
    Ok(forwarded)
}
