//! Where a relative path in a configuration document is measured from.
//!
//! **The rule, stated once for a reader of either side:** a relative filesystem path a
//! *configuration document* supplies is resolved against the directory holding that
//! document. A relative path supplied through the environment layer or a command-line flag
//! is resolved against the **process working directory**, because there is no document to
//! rebase it on. `README.md`, under "Relative paths in a configuration document", is the
//! same rule for a user, and `docs/local-md.md` points a plugin's reader at it.
//!
//! Why it is here rather than in the plugin that reads the path: by the time a plugin is
//! built it holds a block of values and nothing about where they came from, and the
//! document a project's configuration was discovered in is *not* the working directory —
//! [`documents`](super::documents) walks upward from that directory to find it. The origin
//! is the one thing this layer already keeps per setting, so this is the only layer that
//! can answer the question at all.
//!
//! Which fields are paths is each plugin's to say and no part of this module's: every
//! plugin is asked, through [`SourcePlugin::document_relative_paths`](onetaskgraph_plugin_api::SourcePlugin::document_relative_paths), and one that names
//! none — `subprocess` among them, whose `settings:` block belongs to a plugin this build
//! may never have compiled — is simply never rebased.

use std::path::Path;

use serde_json::Value;

use crate::PluginKind;

use super::{ConfigError, Merged, Origin, SettingPath};

/// Rebase every relative path a configuration document supplied, in place.
///
/// The origins are left exactly as they were, so `onetaskgraph config show` still reports
/// which file supplied the setting — what changes is that the value it reports is the one
/// the run will really use. A setting from any other layer, a value that is not a string,
/// and a path that is already absolute are each left alone.
///
/// # Errors
///
/// A setting is refused, by name, when the directory holding its document is not valid
/// UTF-8, because a merged value is a JSON string and there is no such string to put
/// there. The alternative is the one thing this must not do: replacing the undecodable
/// bytes would hand the plugin a path built out of replacement characters, naming a
/// directory nobody has, and dropping the rebasing instead would silently send the run
/// back to the working directory this exists to stop it using. Neither degradation says
/// anything; [`ConfigError::Setting`] names the setting, the document and what to change.
pub fn resolve_document_relative_paths(settings: &mut Merged) -> Result<(), ConfigError> {
    let mut rewrites: Vec<(SettingPath, Value)> = Vec::new();
    for setting in settings.values() {
        let Origin::File { path: document } = &setting.origin else {
            continue;
        };
        let Some((source, field)) = source_config_field(&setting.key) else {
            continue;
        };
        let Some(declared) = declared_paths_of(settings, source) else {
            continue;
        };
        if !declared.contains(&field.as_str()) {
            continue;
        }
        let Some(raw) = setting.value.as_str() else {
            continue;
        };
        // A value that said nothing goes on saying nothing: rebasing `""` would turn
        // a setting that fails today into a silent "the document's own directory".
        if raw.is_empty() || Path::new(raw).is_absolute() {
            continue;
        }
        let Some(directory) = document.parent() else {
            continue;
        };
        let rebased = directory
            .join(raw)
            .into_os_string()
            .into_string()
            .map_err(|_| {
                ConfigError::setting(
                    setting.key.to_string(),
                    format!(
                        "this path is measured from {}, the directory holding the configuration \
                     document that set it, and that directory's name is not valid UTF-8, so \
                     the path it resolves to cannot be written down",
                        directory.display()
                    ),
                    "give this setting an absolute path, or move the configuration document \
                 under a directory whose name is valid UTF-8.",
                )
            })?;
        rewrites.push((setting.key.clone(), Value::String(rebased)));
    }
    for (key, value) in rewrites {
        settings.set_value(&key, value);
    }
    Ok(())
}

/// `("work", "root")` for `sources.work.config.root`, and nothing for any other key.
///
/// The field is the whole of the dotted path *inside* the block, so a plugin may name a
/// nested one.
fn source_config_field(key: &SettingPath) -> Option<(&str, String)> {
    match key.segments() {
        [sources, name, config, field @ ..]
            if sources == "sources" && config == "config" && !field.is_empty() =>
        {
            Some((name.as_str(), field.join(".")))
        }
        _ => None,
    }
}

/// What the plugin of the source called `name` declares as a document-relative path.
///
/// The plugin is read back out of the merge rather than out of a [`Config`](crate::Config),
/// because the rebasing happens before there is one: a `Config` holds the block a plugin
/// will be built from, and building it from an unresolved root is the defect this exists to
/// remove.
fn declared_paths_of(settings: &Merged, name: &str) -> Option<&'static [&'static str]> {
    let key = SettingPath::new(
        vec!["sources".to_owned(), name.to_owned(), "plugin".to_owned()],
        "sources",
    )
    .ok()?;
    let kind = PluginKind::parse(settings.get(&key)?.value.as_str()?)?;
    Some(kind.plugin().document_relative_paths())
}
