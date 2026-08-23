//! The environment read as a configuration layer.
//!
//! This is a *layer*, not a set of per-command overrides: it parses into the same
//! shape a document parses into and is appended after the documents, so a setting
//! reached this way flows through every verb without any verb knowing it exists.

use crate::Environment;

use super::layer::{Layer, Origin, Setting, SettingPath, value_from_text};
use super::{ConfigError, SECRETS_FILE_VARIABLE};

/// What every configuration variable's name begins with.
pub const ENVIRONMENT_PREFIX: &str = "ONETASKGRAPH_";

/// Variables that begin with [`ENVIRONMENT_PREFIX`] and are *not* settings.
///
/// `ONETASKGRAPH_SECRETS_FILE` points at the credentials file, which is read before
/// sources are resolved and is nowhere in the configuration document. Without this
/// list it would decode to a setting called `secrets_file` and be refused as an
/// unknown field — turning a documented variable into an error.
const RESERVED: &[&str] = &[SECRETS_FILE_VARIABLE];

/// The separator between path segments in a variable name.
const SEGMENT_SEPARATOR: &str = "__";

/// Every `ONETASKGRAPH_`-prefixed variable, as one configuration layer.
///
/// The rule, and its inverse. A variable's name is [`ENVIRONMENT_PREFIX`] followed by
/// the setting's path, each segment upper-cased with `-` replaced by `_`, segments
/// joined by [`SEGMENT_SEPARATOR`]. Decoding lower-cases each segment, and turns `_`
/// back into `-` in exactly one position: the segment naming a source, immediately
/// after `sources`. That is the only place a `-` can occur, because a
/// [`SourceName`](onetaskgraph_plugin_api::SourceName) may not contain `_` while
/// every other key in the document is `snake_case` — which is what makes the forward
/// mapping injective and this inverse exact rather than a guess.
///
/// # Errors
///
/// Returns [`ConfigError::Setting`] when a variable's name decodes to no path at all,
/// as `ONETASKGRAPH_` and `ONETASKGRAPH_SOURCES__` do.
pub fn layer(environment: &Environment) -> Result<Layer, ConfigError> {
    let mut settings = Vec::new();
    for (variable, raw) in environment.iter() {
        let Some(encoded) = variable.strip_prefix(ENVIRONMENT_PREFIX) else {
            continue;
        };
        if RESERVED.contains(&variable) {
            continue;
        }
        settings.push(Setting {
            key: path_from(encoded, variable)?,
            value: value_from_text(raw),
            origin: Origin::Environment {
                variable: variable.to_owned(),
            },
        });
    }
    Ok(Layer::new(settings))
}

/// Decode one variable name's suffix into the setting path it addresses.
fn path_from(encoded: &str, variable: &str) -> Result<SettingPath, ConfigError> {
    let segments: Vec<String> = encoded
        .split(SEGMENT_SEPARATOR)
        .enumerate()
        .map(|(index, segment)| decode_segment(segment, index, encoded))
        .collect();
    SettingPath::new(segments, variable)
}

/// One segment, lower-cased, with `_` restored to `-` where a source name sits.
fn decode_segment(segment: &str, index: usize, encoded: &str) -> String {
    let lowered = segment.to_ascii_lowercase();
    if index == 1 && encoded.starts_with("SOURCES__") {
        lowered.replace('_', "-")
    } else {
        lowered
    }
}

/// The variable that sets `key`, for a message that tells a user what to export.
#[must_use]
pub fn variable_for(key: &SettingPath) -> String {
    let segments: Vec<String> = key
        .segments()
        .iter()
        .map(|segment| segment.to_ascii_uppercase().replace('-', "_"))
        .collect();
    format!("{ENVIRONMENT_PREFIX}{}", segments.join(SEGMENT_SEPARATOR))
}
