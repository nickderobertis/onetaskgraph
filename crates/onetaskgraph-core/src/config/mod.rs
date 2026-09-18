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
//!
//! One thing a leaf setting carries besides its value is load-bearing past this layer:
//! **a relative filesystem path a configuration document supplies is resolved against the
//! directory holding that document**, while one the environment or a flag supplies keeps
//! resolving against the process working directory. [`relative`] is where that happens and
//! why it cannot happen in the plugin that reads the path; `README.md`, under "Relative
//! paths in a configuration document", states it for a user.

mod discovery;
mod effective;
mod environment_layer;
mod error;
mod layer;
mod relative;

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use onetaskgraph_plugin_api::SourceName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::secrets::Secrets;
use crate::{Environment, PluginKind, plugin_kinds};

pub use discovery::{
    Document, PROJECT_DOCUMENT_NAME, SECRETS_RELATIVE_PATH, USER_DOCUMENT_RELATIVE_PATH, documents,
    read_optional, readable_documents, secrets_path, user_document_path,
};
pub use effective::EffectiveConfig;
pub use environment_layer::{ENVIRONMENT_PREFIX, variable_for};
pub use error::ConfigError;
pub use layer::{Layer, Merged, Origin, Setting, SettingPath, merge, unflatten, value_from_text};
pub(crate) use relative::rebased;
pub use relative::resolve_document_relative_paths;

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
    plugin: PluginKind,
    config: Value,
    document_dir: Option<PathBuf>,
}

impl SourceConfig {
    /// The plugin kind that builds this source.
    #[must_use]
    pub fn plugin(&self) -> PluginKind {
        self.plugin
    }

    /// The plugin's own block.
    ///
    /// Opaque here on purpose — a plugin's fields are the plugin's, and typing them in
    /// the engine would put every plugin's shape in the engine. What holds instead is
    /// that [`Config::from_document`] checks each block against the schema its own
    /// plugin declares, and this field is not public, so no `SourceConfig` anybody can
    /// reach carries a block that plugin would refuse.
    #[must_use]
    pub fn config(&self) -> &Value {
        &self.config
    }

    /// The absolute directory holding the configuration document that supplied the
    /// block this source hands a child process, or `None` when no one document did.
    ///
    /// Only a `subprocess` source ever carries one: its `settings:` block is opaque here,
    /// so nothing inside it is rebased in this process, and this is what the child is
    /// told instead — `document_dir` in `docs/plugin-protocol.md` §3 — so the hosted
    /// plugin can resolve its own declared paths exactly as the in-process rule does.
    /// It is an engine-owned fact about where the block came from, which is why it is
    /// read from the merge's origins rather than being a setting anybody can write.
    #[must_use]
    pub fn document_dir(&self) -> Option<&Path> {
        self.document_dir.as_deref()
    }
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
///
/// Its fields are read through the methods below rather than reached into, because
/// "validated" is a claim about the whole value: a public `sources` would let a caller
/// hold a `Config` whose `default_sources` names something it does not contain, and a
/// public `SourceConfig` would let one hold a block its own plugin refuses. Neither
/// state is representable while the only way in is [`Config::from_document`].
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    default_sources: Option<Vec<SourceName>>,
    page_size: NonZeroU32,
    output: OutputFormat,
    sources: BTreeMap<SourceName, SourceConfig>,
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
                    document_dir: None,
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

    /// How many items a page holds.
    #[must_use]
    pub fn page_size(&self) -> NonZeroU32 {
        self.page_size
    }

    /// How output is rendered.
    #[must_use]
    pub fn output(&self) -> OutputFormat {
        self.output
    }

    /// Every configured source, in name order.
    #[must_use]
    pub fn sources(&self) -> &BTreeMap<SourceName, SourceConfig> {
        &self.sources
    }

    /// Which sources answer when a command names none, or `None` for every one.
    #[must_use]
    pub fn default_sources(&self) -> Option<&[SourceName]> {
        self.default_sources.as_deref()
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

/// The output format a run asked for, read from whichever layers can still be read.
///
/// For a run whose configuration did not load, which still owes its failure in the format
/// it asked for: `--json` on a command line beside a document that will not parse asks
/// for machine output as plainly as it does beside one that will. The same layers in the
/// same precedence as [`load`], each skipped on its own when it cannot be read or parsed,
/// so a layer that is itself the failure contributes nothing rather than hiding the others.
/// `working_directory` is `None` when there is none to search from, and then no project
/// document is read. Text when no readable layer sets a usable format.
#[must_use]
pub fn requested_output(
    working_directory: Option<&Path>,
    environment: &Environment,
    flags: &Layer,
) -> OutputFormat {
    let mut layers: Vec<Layer> = readable_documents(working_directory, environment)
        .into_iter()
        .filter_map(|document| {
            let parsed: Value = serde_norway::from_str(&document.text).ok()?;
            Layer::from_document(document.path, &parsed).ok()
        })
        .collect();
    layers.extend(environment_layer::layer(environment).ok());
    layers.push(flags.clone());
    merge(&layers)
        .values()
        .find(|setting| setting.key.segments() == ["output"])
        .and_then(|setting| serde_json::from_value(setting.value.clone()).ok())
        .unwrap_or_default()
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

    let mut merged = merge(&layers);
    // Before the block reaches a plugin, and before `config show` reports it: a plugin is
    // handed values and no origins, so this is the only layer that can tell a path a
    // document supplied from one the environment or a flag did. See [`relative`].
    resolve_document_relative_paths(&mut merged)?;
    let mut config = Config::from_document(unflatten(&merged))?;
    for (name, source) in &mut config.sources {
        if source.plugin == PluginKind::Subprocess {
            source.document_dir = relative::supplying_document_dir(&merged, name.as_str());
        }
    }

    // Before the sources are resolved, as the contract says: a plugin reads its
    // credential through this resolver, so it has to exist by the time one is built.
    let secrets = Secrets::load(environment.clone())?;

    Ok(Loaded {
        effective: EffectiveConfig::new(&merged, &config, secrets.report()),
        config,
        secrets,
    })
}
