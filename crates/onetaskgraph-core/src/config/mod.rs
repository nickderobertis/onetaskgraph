//! The configuration document, the three layers over it, and what they resolve to.
//!
//! Precedence is file, then environment, then command-line flags, lowest to highest,
//! and every setting is reachable at all three — including every field of every named
//! source. That is one mechanism rather than three: each layer is flattened to the
//! same list of leaf settings (see [`layer`]), the stack is merged once, and the
//! result is deserialized into [`Config`]. Nothing per-verb decides precedence, so
//! nothing per-verb can get it wrong.
//!
//! Reading is [`discovery`]'s and nothing else's; everything else here is a function
//! of its arguments.

mod discovery;
mod effective;
mod environment_layer;
mod error;
mod layer;

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::Path;

use onetaskgraph_plugin_api::SourceName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::secrets::Secrets;
use crate::{Environment, PluginKind, plugin_kinds};

pub use discovery::{
    Document, PROJECT_DOCUMENT_NAME, SECRETS_RELATIVE_PATH, USER_DOCUMENT_RELATIVE_PATH, documents,
    read_optional, secrets_path, user_document_path,
};
pub use effective::EffectiveConfig;
pub use environment_layer::{ENVIRONMENT_PREFIX, variable_for};
pub use error::ConfigError;
pub use layer::{Layer, Merged, Origin, Setting, SettingPath, merge, unflatten, value_from_text};

/// The variable that moves the credentials file somewhere else.
pub const SECRETS_FILE_VARIABLE: &str = "ONETASKGRAPH_SECRETS_FILE";

/// How many items a page holds when nothing sets `page_size`.
pub const DEFAULT_PAGE_SIZE: NonZeroU32 = NonZeroU32::new(50).expect("50 is not zero");

/// How output is rendered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    /// For a person reading a terminal.
    #[default]
    Text,
    /// For a program.
    Json,
}

/// One named source, as a document configures it.
///
/// Built by [`Config::from_document`], never deserialized directly: `plugin` is a
/// [`PluginKind`] rather than the string the document spelled, so a source naming a
/// plugin this build does not have cannot be represented here at all.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceConfig {
    /// The plugin kind that builds it.
    pub plugin: PluginKind,
    /// The plugin's own block.
    ///
    /// Opaque here on purpose — a plugin's fields are the plugin's, and typing them in
    /// the engine would put every plugin's shape in the engine. What holds instead is
    /// that [`Config::from_document`] checks each block against the schema its own
    /// plugin declares, so no `Config` carries a block that plugin would refuse.
    pub config: Value,
}

/// One named source as a document spells it, before its plugin name is checked.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceShape {
    plugin: String,
    #[serde(default = "empty_block")]
    config: Value,
}

/// A plugin block nobody wrote, which is different from one nobody may write.
fn empty_block() -> Value {
    Value::Object(Map::new())
}

/// A validated configuration.
///
/// Built by [`Config::from_document`], never deserialized directly: a source's name
/// has to be checked against the pattern the environment mapping depends on, and
/// `default_sources` has to name sources that exist, and both are worth a message
/// that says which key is wrong rather than serde's own.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Which sources answer when a command names none. `None` means every one.
    pub default_sources: Option<Vec<SourceName>>,
    /// How many items a page holds.
    pub page_size: NonZeroU32,
    /// How output is rendered.
    pub output: OutputFormat,
    /// Every configured source, in name order.
    pub sources: BTreeMap<SourceName, SourceConfig>,
}

/// The document's own shape, before the checks serde cannot make.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DocumentShape {
    #[serde(deserialize_with = "one_or_many")]
    default_sources: Option<Vec<String>>,
    page_size: NonZeroU32,
    output: OutputFormat,
    sources: BTreeMap<String, SourceShape>,
}

impl Default for DocumentShape {
    fn default() -> Self {
        Self {
            default_sources: None,
            page_size: DEFAULT_PAGE_SIZE,
            output: OutputFormat::default(),
            sources: BTreeMap::new(),
        }
    }
}

/// Accept one name where a list is expected.
///
/// The environment layer reads a comma-separated value as a list, so
/// `ONETASKGRAPH_DEFAULT_SOURCES=work,notes` is one; a single name has no comma to
/// split on, and refusing `ONETASKGRAPH_DEFAULT_SOURCES=work` would make the layer
/// hold for two sources and not for one.
fn one_or_many<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        None => None,
        Some(OneOrMany::One(name)) => Some(vec![name]),
        Some(OneOrMany::Many(names)) => Some(names),
    })
}

impl Config {
    /// Read one merged document into a validated configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Setting`] naming the offending key for an unknown
    /// field, a value of the wrong shape, a `plugin:` this build does not have, a
    /// source name that does not match
    /// [`SOURCE_NAME_PATTERN`](onetaskgraph_plugin_api::SOURCE_NAME_PATTERN), a
    /// `default_sources` entry naming a source nothing configures, or a `config:`
    /// block the source's own plugin refuses.
    pub fn from_document(document: Value) -> Result<Self, ConfigError> {
        let shape: DocumentShape = serde_path_to_error::deserialize(document).map_err(|error| {
            let key = error.path().to_string();
            let key = if key.is_empty() || key == "." {
                "the document's root".to_owned()
            } else {
                key
            };
            ConfigError::setting(
                key,
                error.into_inner().to_string(),
                "correct that setting, or remove it — `onetaskgraph config show` lists \
                     every setting this build reads and the layer each came from.",
            )
        })?;

        let mut sources = BTreeMap::new();
        for (name, source) in shape.sources {
            let key = format!("sources.{name}");
            let plugin = PluginKind::parse(&source.plugin).ok_or_else(|| {
                ConfigError::setting(
                    format!("{key}.plugin"),
                    format!(
                        "no plugin named {:?} is built into this binary",
                        source.plugin
                    ),
                    format!("use one of: {}.", plugin_kinds().join(", ")),
                )
            })?;
            let name = SourceName::new(name).map_err(|error| {
                ConfigError::setting(
                    &key,
                    error.to_string(),
                    "rename the source to lower-case letters, digits and hyphens — an \
                     underscore would make the ONETASKGRAPH_SOURCES__<NAME>__ mapping \
                     ambiguous.",
                )
            })?;
            sources.insert(
                name,
                SourceConfig {
                    plugin,
                    config: source.config,
                },
            );
        }

        let default_sources = shape
            .default_sources
            .map(|names| resolve_default_sources(&names, &sources))
            .transpose()?;

        let config = Self {
            default_sources,
            page_size: shape.page_size,
            output: shape.output,
            sources,
        };
        // Here rather than at the call site, so "a `Config` exists" means "every block in
        // it satisfies the schema its own plugin declares". Checked once at the boundary,
        // a mistyped per-source field cannot survive as far as the HTTP call that would
        // otherwise be the first thing to notice it.
        crate::resolve::validate_sources(&config)?;
        Ok(config)
    }

    /// The sources a command answers from when it names none, in a stable order.
    #[must_use]
    pub fn selected_sources(&self) -> Vec<SourceName> {
        self.default_sources
            .clone()
            .unwrap_or_else(|| self.sources.keys().cloned().collect())
    }
}

/// Check every `default_sources` entry against the sources that exist.
fn resolve_default_sources(
    names: &[String],
    sources: &BTreeMap<SourceName, SourceConfig>,
) -> Result<Vec<SourceName>, ConfigError> {
    names
        .iter()
        .map(|name| {
            let selected = SourceName::new(name.clone()).map_err(|error| {
                ConfigError::setting(
                    "default_sources",
                    error.to_string(),
                    "name a configured source; `onetaskgraph config show` lists them.",
                )
            })?;
            if sources.contains_key(&selected) {
                Ok(selected)
            } else {
                Err(ConfigError::setting(
                    "default_sources",
                    format!("no source named {name:?} is configured"),
                    format!(
                        "name one of the configured sources ({}), or configure {name:?} under \
                         `sources`.",
                        source_list(sources)
                    ),
                ))
            }
        })
        .collect()
}

/// The configured source names, for a message.
fn source_list(sources: &BTreeMap<SourceName, SourceConfig>) -> String {
    if sources.is_empty() {
        "none are".to_owned()
    } else {
        sources
            .keys()
            .map(SourceName::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A configuration, the credentials behind it, and where every setting came from.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The configuration itself.
    pub config: Config,
    /// Where a plugin's named credential is looked up. Read before sources resolve.
    pub secrets: Secrets,
    /// Every setting with the layer it came from, for `config show`.
    pub effective: EffectiveConfig,
}

/// Load the configuration: documents, then the environment, then `flags`.
///
/// Each source's `config` block is checked against its plugin's declared schema
/// before this returns, so a mistyped per-source field is a load-time refusal rather
/// than a surprise inside the first call that source makes.
///
/// # Errors
///
/// Returns [`ConfigError`] for a document that cannot be read or parsed, and for any
/// setting that is unknown, unusable, or names a plugin this build does not have.
pub fn load(
    working_directory: &Path,
    environment: &Environment,
    flags: &Layer,
) -> Result<Loaded, ConfigError> {
    let mut layers = Vec::new();
    for document in documents(working_directory, environment)? {
        let parsed: Value =
            serde_norway::from_str(&document.text).map_err(|error| ConfigError::Syntax {
                path: document.path.clone(),
                message: error.to_string(),
            })?;
        layers.push(Layer::from_document(document.path, &parsed)?);
    }
    layers.push(environment_layer::layer(environment)?);
    layers.push(flags.clone());

    let merged = merge(&layers);
    let config = Config::from_document(unflatten(&merged))?;

    // Before the sources are resolved, as the contract says: a plugin reads its
    // credential through this resolver, so it has to exist by the time one is built.
    let secrets = Secrets::load(environment.clone())?;

    Ok(Loaded {
        effective: EffectiveConfig::new(&merged, &config, secrets.report()),
        config,
        secrets,
    })
}
