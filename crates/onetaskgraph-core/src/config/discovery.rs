//! The only module here that touches the filesystem.
//!
//! Finding the documents and reading them is all this does; parsing them, merging
//! them and deciding what they mean happens above, on values. Keeping the boundary
//! at one module is what lets every rule this layer has be tested against text
//! rather than against a directory somebody had to build first — and it is why the
//! secrets file is read here too, beside the documents, rather than by the module
//! that parses it.

use std::path::{Path, PathBuf};

use crate::Environment;

use super::ConfigError;

/// The document discovered upward from the working directory.
pub const PROJECT_DOCUMENT_NAME: &str = "onetaskgraph.yaml";

/// The user-level document, under the configuration home.
pub const USER_DOCUMENT_RELATIVE_PATH: &str = "onetaskgraph/config.yaml";

/// The credentials file, under the configuration home.
pub const SECRETS_RELATIVE_PATH: &str = "onetaskgraph/secrets.env";

/// One configuration document, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Where it was read from.
    pub path: PathBuf,
    /// What it holds.
    pub text: String,
}

/// Every configuration document that applies, **lowest precedence first**.
///
/// That is the user-level document, then the nearest `onetaskgraph.yaml` at or above
/// `working_directory`. The nearest one alone: a project's document layers over the
/// user's, and stacking every ancestor as well would make what a command reads depend
/// on how deep in a tree it was run from.
///
/// # Errors
///
/// Returns [`ConfigError::Read`] when a document exists but cannot be read. A
/// document that is not there is not an error — it is the ordinary case.
pub fn documents(
    working_directory: &Path,
    environment: &Environment,
) -> Result<Vec<Document>, ConfigError> {
    let mut found = Vec::new();
    if let Some(path) = user_document_path(environment)
        && let Some(text) = read_optional(&path)?
    {
        found.push(Document { path, text });
    }
    if let Some(path) = nearest_project_document(working_directory)
        && let Some(text) = read_optional(&path)?
    {
        found.push(Document { path, text });
    }
    Ok(found)
}

/// Where the user-level document lives, when this host says where that is.
#[must_use]
pub fn user_document_path(environment: &Environment) -> Option<PathBuf> {
    Some(configuration_home(environment)?.join(USER_DOCUMENT_RELATIVE_PATH))
}

/// Where the credentials file lives, honouring the override variable.
#[must_use]
pub fn secrets_path(environment: &Environment) -> Option<PathBuf> {
    if let Some(override_path) = environment.non_empty(super::SECRETS_FILE_VARIABLE) {
        return Some(PathBuf::from(override_path));
    }
    Some(configuration_home(environment)?.join(SECRETS_RELATIVE_PATH))
}

/// `$XDG_CONFIG_HOME`, or `$HOME/.config`, or nothing when neither is set.
fn configuration_home(environment: &Environment) -> Option<PathBuf> {
    if let Some(xdg) = environment.non_empty("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg));
    }
    Some(PathBuf::from(environment.non_empty("HOME")?).join(".config"))
}

/// The nearest `onetaskgraph.yaml` at or above `working_directory`.
fn nearest_project_document(working_directory: &Path) -> Option<PathBuf> {
    working_directory
        .ancestors()
        .map(|directory| directory.join(PROJECT_DOCUMENT_NAME))
        .find(|candidate| candidate.is_file())
}

/// Read `path`, treating "there is no such file" as "there is nothing here".
///
/// # Errors
///
/// Returns [`ConfigError::Read`] for every other way a read can fail — a directory
/// in the way, or a file this user may not open. Those are worth stopping for:
/// silently continuing would run against a configuration the user believes is loaded.
pub fn read_optional(path: &Path) -> Result<Option<String>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Ok(None),
    }
}
