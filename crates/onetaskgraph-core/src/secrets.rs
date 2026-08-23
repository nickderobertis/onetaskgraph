//! The credentials file, and the resolver a plugin reads its credential through.
//!
//! A configuration document never carries a credential value — it names the
//! environment variable holding one (`api_key_env: LINEAR_API_KEY`). This is the
//! layer that answers those names: the process environment first, and a file under
//! the configuration home for whatever the process environment does not define.
//!
//! Parsing here is pure; the read is [`discovery`](crate::config::read_optional)'s,
//! like every other read this crate makes.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use onetaskgraph_plugin_api::SecretResolver;
use schemars::JsonSchema;
use secrecy::SecretString;
use serde::Serialize;

use crate::Environment;
use crate::config::{ConfigError, read_optional, secrets_path};

/// Where a plugin's named credential is looked up.
///
/// # Debug redacts
///
/// A credential must never reach standard output, an error message, a log line, or a
/// `Debug` rendering. `SecretString` already refuses to print itself; the
/// implementation below goes further and prints only the *names* this resolver can
/// answer, so even a `{:#?}` of the whole resolver carries nothing to leak.
#[derive(Clone)]
pub struct Secrets {
    environment: Environment,
    file: BTreeMap<String, SecretString>,
    path: Option<PathBuf>,
}

impl Secrets {
    /// Read the credentials file this environment points at.
    ///
    /// The path is `$ONETASKGRAPH_SECRETS_FILE`, or
    /// `$XDG_CONFIG_HOME/onetaskgraph/secrets.env`, or
    /// `$HOME/.config/onetaskgraph/secrets.env`. A file that is not there is not an
    /// error: a host with both credentials exported and no file is a configured host.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Read`] when the file exists and cannot be read, and
    /// [`ConfigError::Setting`] when a line of it is not `KEY=VALUE`.
    pub fn load(environment: Environment) -> Result<Self, ConfigError> {
        let path = secrets_path(&environment);
        let file = match &path {
            Some(path) => read_optional(path)?
                .map(|text| parse(&text, path))
                .transpose()?
                .unwrap_or_default(),
            None => BTreeMap::new(),
        };
        Ok(Self {
            environment,
            file,
            path,
        })
    }

    /// What the credentials file supplied and which layer each name resolves from.
    ///
    /// Names and layers only — never a value. This is what lets a user check that
    /// their key was picked up, and which of the two layers is answering, without the
    /// key itself ever reaching a terminal or a log.
    #[must_use]
    pub fn report(&self) -> SecretsReport {
        SecretsReport {
            path: self.path.clone(),
            variables: self
                .file
                .keys()
                .map(|variable| ResolvedCredential {
                    variable: variable.clone(),
                    resolved_from: if self.environment.non_empty(variable).is_some() {
                        CredentialLayer::Environment
                    } else {
                        CredentialLayer::SecretsFile
                    },
                })
                .collect(),
        }
    }
}

impl SecretResolver for Secrets {
    /// The process environment wins.
    ///
    /// A variable someone exported deliberately for this one command has to beat a
    /// file that was written once and forgotten, or the file becomes impossible to
    /// override without editing it.
    fn get(&self, var: &str) -> Option<SecretString> {
        if let Some(exported) = self.environment.non_empty(var) {
            return Some(SecretString::from(exported.to_owned()));
        }
        self.file.get(var).cloned()
    }
}

impl fmt::Debug for Secrets {
    /// Names only. See the type's own note: every value here is a credential.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secrets")
            .field("path", &self.path)
            .field("file_variables", &self.file.keys().collect::<Vec<_>>())
            .field("values", &"<redacted>")
            .finish()
    }
}

/// Which layer answered for one credential name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialLayer {
    /// The process environment already defined it, so the file's value is unused.
    Environment,
    /// The credentials file supplied it.
    SecretsFile,
}

impl fmt::Display for CredentialLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment => f.write_str("environment"),
            Self::SecretsFile => f.write_str("secrets file"),
        }
    }
}

/// One credential name the file supplied, and where its value comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ResolvedCredential {
    /// The variable's name. Never its value.
    pub variable: String,
    /// The layer whose value a plugin would receive.
    pub resolved_from: CredentialLayer,
}

/// What the credentials file supplied, without supplying it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SecretsReport {
    /// The file that was looked for, whether or not it was there.
    pub path: Option<PathBuf>,
    /// Every name the file defines, in order.
    pub variables: Vec<ResolvedCredential>,
}

/// Read a `KEY=VALUE` credentials file.
///
/// Blank lines are skipped and a line whose first non-blank character is `#` is a
/// comment. A value may be wrapped in matching single or double quotes, which are
/// stripped; otherwise it is the rest of the line with surrounding blanks removed,
/// `#` included — a credential may contain one, and guessing where a comment starts
/// inside a value would silently truncate a key.
///
/// # Errors
///
/// Returns [`ConfigError::Setting`] naming the file and the line number when a line
/// is not `KEY=VALUE` or its key is not a usable variable name. Neither message
/// carries any part of the line's value.
fn parse(text: &str, path: &Path) -> Result<BTreeMap<String, SecretString>, ConfigError> {
    let mut values = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);

        let Some((name, value)) = line.split_once('=') else {
            return Err(ConfigError::setting(
                format!("{}:{number}", path.display()),
                "this line is not `KEY=VALUE`",
                "write it as `NAME=value`, comment it out with `#`, or delete it.",
            ));
        };
        let name = name.trim();
        if !is_variable_name(name) {
            return Err(ConfigError::setting(
                format!("{}:{number}", path.display()),
                format!("{name:?} is not a usable environment variable name"),
                "use letters, digits and underscores, starting with a letter or an \
                 underscore — the names this product reads are LINEAR_API_KEY and \
                 GH_PROJECTS_TOKEN.",
            ));
        }
        values.insert(name.to_owned(), SecretString::from(unquote(value.trim())));
    }
    Ok(values)
}

/// Whether `name` is something a shell could have exported.
fn is_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Strip one matching pair of surrounding quotes, if there is one.
fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return value[1..value.len() - 1].to_owned();
        }
    }
    value.to_owned()
}

