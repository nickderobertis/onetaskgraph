// llmlint: ignore-file[code_lands_in_the_domain_that_owns_it] This module is where the task that introduced templates fixes their library API: at `onetaskgraph-core`'s crate root, beside the engine every surface shares, so a Rust consumer renders without the binary. A crate of its own would be a new published sibling — a change to the release surface `release-targets.toml` freezes — which is not this change's to make; `crates/onetaskgraph-core/src/lib.rs` carries the same reason where the module is declared.
//! Task templates: a minijinja document with a declared set of variables, rendered from
//! answers.
//!
//! # The format
//!
//! A template file is UTF-8 minijinja source, optionally opened by YAML front matter — a
//! first line reading `---`, closed by the next line reading `---`:
//!
//! ```yaml
//! ---
//! onetaskgraph_template: 1          # required when front matter is present
//! description: <string>             # optional
//! variables:                        # optional; name -> declaration
//!   <name>:                         # ^[a-z][a-z0-9_]*$
//!     description: <string>         # required, non-empty: what a prompt shows
//!     type: string                  # string | text | integer | boolean | list | object
//!     items: string                 # list only: string | object; default string
//!     required: true                # default: true without `default`, false with one
//!     default: <value of `type`>    # optional; `required: true` beside one is refused
//! ---
//! ```
//!
//! Nothing here restricts how the body uses a variable: it is minijinja, whole.
//!
//! # The chain
//!
//! `extends`, `include` and `import` resolve names over a [`TemplateLoader`]'s search path —
//! the directories it was given, in order, then the `(name, source)` pairs registered on it —
//! and never the working directory. The **declared set** is the union of every chain file's
//! front matter. When two files declare one variable, the file nearer the rendered one (fewer
//! references away from it, then loaded first) gives its `description`, `default` and
//! `required`; its `type` and `items` must be the other's, or the chain is refused naming both
//! files.
//!
//! # Rendering
//!
//! Strict: a name that is neither a declared variable nor set by the template fails the
//! render, naming the name and the file. No auto-escaping, trailing newlines kept,
//! `trim_blocks` and `lstrip_blocks` on. An optional variable given no answer and no default
//! renders as `none`.
//!
//! # The digest
//!
//! `sha256:` and the lowercase hex SHA-256 over, for each chain file in first-load order, its
//! resolved name, a NUL, its full bytes — front matter included — and a NUL.
//!
//! No prompting happens here. A caller that prompts asks [`Template::unanswered`] what is
//! left, puts the answers it gathers over the ones it had with [`Answers::overlay`], and
//! renders; the command line is the one caller that does.

mod answers;
mod front_matter;
mod scan;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

pub use answers::Answers;
use answers::Given;
use front_matter::Declaration;
pub use front_matter::{DECLARATION_KEYS, FRONT_MATTER_KEYS, is_variable_name};

/// What one variable holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VariableType {
    /// One line of text.
    String,
    /// Text of any number of lines.
    Text,
    /// A whole number.
    Integer,
    /// `true` or `false`.
    Boolean,
    /// A list, of the variable's `items`.
    List,
    /// A mapping from string keys to values.
    Object,
}

impl VariableType {
    /// Every type, in the order front matter documents them.
    pub const ALL: [Self; 6] = [
        Self::String,
        Self::Text,
        Self::Integer,
        Self::Boolean,
        Self::List,
        Self::Object,
    ];

    /// The type a front matter `type:` spells, if it spells one.
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "string" => Self::String,
            "text" => Self::Text,
            "integer" => Self::Integer,
            "boolean" => Self::Boolean,
            "list" => Self::List,
            "object" => Self::Object,
            _ => return None,
        })
    }

    /// How front matter spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::List => "list",
            Self::Object => "object",
        }
    }
}

impl fmt::Display for VariableType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What each entry of a `list` variable holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ItemType {
    /// Strings.
    String,
    /// Mappings from string keys to values.
    Object,
}

impl ItemType {
    /// Every item type, in the order front matter documents them.
    pub const ALL: [Self; 2] = [Self::String, Self::Object];

    /// The item type a front matter `items:` spells, if it spells one.
    fn parse(text: &str) -> Option<Self> {
        match text {
            "string" => Some(Self::String),
            "object" => Some(Self::Object),
            _ => None,
        }
    }

    /// How front matter spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Object => "object",
        }
    }
}

/// One variable of a template's declared set, merged down its chain.
///
/// Built only by loading a template, so `items` is present exactly when `type` is `list`,
/// a `default` is a value of the variable's type, and `required` is never `true` beside one.
// llmlint: ignore[invalid_states_unrepresentable] Every field is private and the one
// constructor is the chain merge, which copies a declaration the front matter parser already
// refused unless those three rules held; the flat shape is the wire contract C1 fixes
// (`type`, `items`, `required`, `default` side by side), which a nested enum would change.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TemplateVariable {
    /// The variable's name, as the template body uses it.
    name: String,
    /// What the variable is for; what a prompt shows.
    description: String,
    /// What the variable holds.
    #[serde(rename = "type")]
    kind: VariableType,
    /// What each entry holds, for a `list` variable and no other.
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<ItemType>,
    /// Whether rendering without an answer is refused. An optional variable with no answer
    /// and no default renders as `none`.
    required: bool,
    /// The value used when no answer is given.
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<Value>,
    /// The chain file whose declaration this is: the one nearest the rendered template.
    declared_in: String,
}

impl TemplateVariable {
    /// The variable's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the variable is for.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// What the variable holds.
    #[must_use]
    pub fn kind(&self) -> VariableType {
        self.kind
    }

    /// What each entry holds, for a `list` variable.
    #[must_use]
    pub fn items(&self) -> Option<ItemType> {
        self.items
    }

    /// Whether rendering without an answer is refused.
    #[must_use]
    pub fn required(&self) -> bool {
        self.required
    }

    /// The value used when no answer is given.
    #[must_use]
    pub fn default(&self) -> Option<&Value> {
        self.default.as_ref()
    }

    /// The chain file whose declaration this is.
    #[must_use]
    pub fn declared_in(&self) -> &str {
        &self.declared_in
    }

    /// Whether a value of this variable is written over several lines: `text`, `list` and
    /// `object`.
    #[must_use]
    pub fn multi_line(&self) -> bool {
        matches!(
            self.kind,
            VariableType::Text | VariableType::List | VariableType::Object
        )
    }

    /// Read text as this variable's value: literally for `string` and `text`, as YAML for
    /// every other type — the rule `--var` and a prompt both follow.
    ///
    /// # Errors
    ///
    /// Why the text is not a value of this variable's type.
    pub fn parse(&self, text: &str) -> Result<Value, String> {
        let value = match self.kind {
            VariableType::String | VariableType::Text => Value::String(text.to_owned()),
            _ => {
                serde_norway::from_str(text).map_err(|error| format!("it is not YAML: {error}"))?
            }
        };
        self.check(&value)?;
        Ok(value)
    }

    /// Whether `value` is a value of this variable's type.
    ///
    /// # Errors
    ///
    /// Why it is not.
    pub fn check(&self, value: &Value) -> Result<(), String> {
        check_value(self.kind, self.items, value)
    }
}

/// Whether `value` is a value of `kind` (with `items`, for a list).
fn check_value(kind: VariableType, items: Option<ItemType>, value: &Value) -> Result<(), String> {
    let fits = match kind {
        VariableType::String => {
            if let Value::String(text) = value
                && text.contains(['\n', '\r'])
            {
                return Err("a `string` is one line; declare the variable `text` for more".into());
            }
            value.is_string()
        }
        VariableType::Text => value.is_string(),
        VariableType::Integer => value.is_i64() || value.is_u64(),
        VariableType::Boolean => value.is_boolean(),
        VariableType::Object => value.is_object(),
        VariableType::List => {
            let Value::Array(entries) = value else {
                return Err(format!("expected a list, got {}", describe(value)));
            };
            let items = items.unwrap_or(ItemType::String);
            if let Some((index, entry)) =
                entries.iter().enumerate().find(|(_, entry)| match items {
                    ItemType::String => !entry.is_string(),
                    ItemType::Object => !entry.is_object(),
                })
            {
                return Err(format!(
                    "entry {index} of the list is {}, and its items are {}s",
                    describe(entry),
                    items.as_str()
                ));
            }
            true
        }
    };
    if fits {
        Ok(())
    } else {
        Err(format!(
            "expected {}, got {}",
            article(kind),
            describe(value)
        ))
    }
}

/// A type with its article, for a message.
fn article(kind: VariableType) -> String {
    match kind {
        VariableType::Integer | VariableType::Object => format!("an {kind}"),
        _ => format!("a {kind}"),
    }
}

/// What kind of JSON value `value` is, for a message.
fn describe(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => format!("the boolean {value}"),
        Value::Number(number) => format!("the number {number}"),
        Value::String(text) => format!("the string {text:?}"),
        Value::Array(_) => "a list".to_owned(),
        Value::Object(_) => "a mapping".to_owned(),
    }
}

/// Which field of a declaration two chain files disagree about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainField {
    /// The variable's `type`.
    Type,
    /// A `list` variable's `items`.
    Items,
}

impl fmt::Display for ChainField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Type => "type",
            Self::Items => "items",
        })
    }
}

/// Why a template could not be loaded, answered or rendered.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum TemplateError {
    /// A name or path the chain needs resolved to nothing.
    #[error(
        "template {name:?} was not found{}\nnext: {}",
        referenced_from.as_ref().map(|from| format!(" (named by {from})")).unwrap_or_default(),
        if searched.is_empty() {
            "give the directory holding it as a search path, or register it by name.".to_owned()
        } else {
            format!("it was looked for in {}; add the directory holding it as a search path.", searched.join(", "))
        }
    )]
    NotFound {
        /// The name or path.
        name: String,
        /// The chain file that named it, when one did.
        referenced_from: Option<String>,
        /// The search path directories it was looked for in.
        searched: Vec<String>,
    },
    /// A template that is there and could not be read — a permission, or a directory where a
    /// file was named.
    #[error(
        "template {name} could not be read: {message}\n\
         next: name a readable file, or give it the permissions this process reads with."
    )]
    Unreadable {
        /// The path it was read at.
        name: String,
        /// Why the read failed.
        message: String,
    },
    /// A chain file that is not a template this format reads.
    #[error(
        "template {file}: {}{message}\nnext: {}",
        key.as_ref().map(|key| format!("front matter key `{key}` ")).unwrap_or_default(),
        "correct that file; a template's front matter is described in the README under \"Task templates\"."
    )]
    Malformed {
        /// The chain file.
        file: String,
        /// The front matter key the refusal is about, when it is about one.
        key: Option<String>,
        /// What is wrong.
        message: String,
    },
    /// Two chain files declaring one variable with different `type`s or `items`.
    #[error(
        "template variable {variable:?} is declared with {field} {nearer_value} in {nearer} and \
         with {field} {farther_value} in {farther}; a redeclaration may change `description`, \
         `default` and `required`, never `type` or `items`\n\
         next: declare it with the same {field} in both files, or rename one of them."
    )]
    ChainConflict {
        /// The variable.
        variable: String,
        /// Which field differs.
        field: ChainField,
        /// The file nearer the rendered template.
        nearer: String,
        /// What that file declares.
        nearer_value: &'static str,
        /// The file farther from it.
        farther: String,
        /// What that file declares.
        farther_value: &'static str,
    },
    /// An answers document that is not a mapping of answers.
    #[error(
        "the answers document is refused: {message}\n\
         next: write the answers as a YAML mapping from variable name to value."
    )]
    MalformedAnswers {
        /// What is wrong with it.
        message: String,
    },
    /// Answers naming no declared variable.
    #[error(
        "{} no variable the template declares: {}\n\
         next: remove {}, or declare {} in the template's front matter — `onetaskgraph template \
         variables` lists what it declares.",
        if names.len() == 1 { "an answer names" } else { "answers name" },
        names.join(", "),
        if names.len() == 1 { "that answer" } else { "those answers" },
        if names.len() == 1 { "it" } else { "them" }
    )]
    UnknownAnswer {
        /// Every undeclared name answered, in name order.
        names: Vec<String>,
    },
    /// An answer that is not a value of its variable's type.
    #[error(
        "the answer to {name:?} is not {expected}: {problem}\n\
         next: give {name} a value of type {kind}."
    )]
    MistypedAnswer {
        /// The variable.
        name: String,
        /// Its type.
        kind: VariableType,
        /// Its type with an article, for the message.
        expected: String,
        /// Why the answer is not one.
        problem: String,
    },
    /// Required variables no answer and no default covers.
    #[error(
        "{} unanswered: {}\n\
         next: answer {} with --var NAME=VALUE or in an answers file (--answers FILE), or run \
         interactively to be asked.",
        if names.len() == 1 { "a required variable is" } else { "required variables are" },
        names.join(", "),
        if names.len() == 1 { "it" } else { "each" }
    )]
    MissingRequired {
        /// Every one, in declaration order.
        names: Vec<String>,
    },
    /// The render itself failed: an undefined name, or an error a filter or tag raised.
    #[error(
        "template {}{}: {message}\n\
         next: {}",
        file.as_deref().unwrap_or("?"),
        line.map(|line| format!(" line {line}")).unwrap_or_default(),
        match name {
            Some(name) => format!(
                "declare {name} in the front matter of a file in the chain, or set it in the \
                 template before it is used."
            ),
            None => "correct the template at that line.".to_owned(),
        }
    )]
    Render {
        /// The chain file the render failed in.
        file: Option<String>,
        /// The file's own line, front matter counted.
        line: Option<usize>,
        /// The undefined name, when that is what failed.
        name: Option<String>,
        /// What failed.
        message: String,
    },
}

impl TemplateError {
    /// This product's kebab-case name for the failure.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "template-not-found",
            Self::Unreadable { .. } => "template-unreadable",
            Self::Malformed { .. } => "template-malformed",
            Self::ChainConflict { .. } => "template-chain-conflict",
            Self::MalformedAnswers { .. } => "template-answers-malformed",
            Self::UnknownAnswer { .. } => "template-unknown-answer",
            Self::MistypedAnswer { .. } => "template-mistyped-answer",
            Self::MissingRequired { .. } => "template-missing-required",
            Self::Render { .. } => "template-render",
        }
    }

    /// Whether this refuses the answers rather than the template: an answers document that
    /// is not one, an answer to no declared variable or of the wrong type, and required
    /// variables left unanswered. The command line exits `2` for these.
    #[must_use]
    pub fn refuses_answers(&self) -> bool {
        matches!(
            self,
            Self::MalformedAnswers { .. }
                | Self::UnknownAnswer { .. }
                | Self::MistypedAnswer { .. }
                | Self::MissingRequired { .. }
        )
    }

    fn malformed(file: &str, key: Option<&str>, message: impl Into<String>) -> Self {
        Self::Malformed {
            file: file.to_owned(),
            key: key.map(str::to_owned),
            message: message.into(),
        }
    }
}

/// Where a template and the templates it names are found.
///
/// Directories are searched first, in the order given, then the registered pairs — which
/// is how a Rust consumer supplies templates embedded in its own binary. The working
/// directory is never searched unless it is given.
#[derive(Debug, Clone, Default)]
pub struct TemplateLoader {
    directories: Vec<PathBuf>,
    registered: Vec<(String, String)>,
}

impl TemplateLoader {
    /// A loader with an empty search path.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `directory` to the end of the search path's directories.
    #[must_use]
    pub fn with_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directories.push(directory.into());
        self
    }

    /// Register a template's source under `name`, searched after every directory.
    #[must_use]
    pub fn with_template(mut self, name: impl Into<String>, source: impl Into<String>) -> Self {
        self.registered.push((name.into(), source.into()));
        self
    }

    /// Load the template at `path` and every template its chain names.
    ///
    /// The file is known to its chain — and in its digest — by its file name.
    ///
    /// # Errors
    ///
    /// [`TemplateError::NotFound`] for a file that cannot be read or a chain name nothing
    /// resolves; [`TemplateError::Malformed`] for a chain file that is not UTF-8, whose front
    /// matter is refused, or whose body does not parse; [`TemplateError::ChainConflict`] for
    /// a variable two chain files type differently.
    pub fn load_path(&self, path: &Path) -> Result<Template, TemplateError> {
        let shown = path.display().to_string();
        let bytes = std::fs::read(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                TemplateError::NotFound {
                    name: shown.clone(),
                    referenced_from: None,
                    searched: Vec::new(),
                }
            } else {
                TemplateError::Unreadable {
                    name: shown.clone(),
                    message: error.to_string(),
                }
            }
        })?;
        let source = String::from_utf8(bytes)
            .map_err(|_| TemplateError::malformed(&shown, None, "it is not UTF-8 text"))?;
        let name = path
            .file_name()
            .map_or_else(|| shown.clone(), |name| name.to_string_lossy().into_owned());
        self.load_chain(name, source)
    }

    /// Load the template the search path resolves `name` to, and every template its chain
    /// names.
    ///
    /// # Errors
    ///
    /// As [`TemplateLoader::load_path`], and [`TemplateError::NotFound`] when the search
    /// path resolves `name` to nothing.
    pub fn load_name(&self, name: &str) -> Result<Template, TemplateError> {
        let source = self.find(name)?.ok_or_else(|| self.not_found(name, None))?;
        self.load_chain(name.to_owned(), source)
    }

    /// The source the search path resolves `name` to, or `None`.
    ///
    /// A name spelled to climb out of the search path (`..`), or an absolute one, is looked
    /// for among the registered pairs alone, so a template cannot name its way to a file
    /// outside the directories it was given. A symbolic link inside one of those directories
    /// is followed like any other file there: the check is on how the name is spelled, and
    /// whoever put the link in a search directory chose what it reaches.
    fn find(&self, name: &str) -> Result<Option<String>, TemplateError> {
        let path = Path::new(name);
        let spelled_within = !name.is_empty()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if spelled_within {
            for directory in &self.directories {
                let candidate = directory.join(path);
                if candidate.is_file() {
                    let bytes =
                        std::fs::read(&candidate).map_err(|error| TemplateError::Unreadable {
                            name: candidate.display().to_string(),
                            message: error.to_string(),
                        })?;
                    return String::from_utf8(bytes)
                        .map(Some)
                        .map_err(|_| TemplateError::malformed(name, None, "it is not UTF-8 text"));
                }
            }
        }
        Ok(self
            .registered
            .iter()
            .find(|(registered, _)| registered == name)
            .map(|(_, source)| source.clone()))
    }

    fn not_found(&self, name: &str, referenced_from: Option<&str>) -> TemplateError {
        TemplateError::NotFound {
            name: name.to_owned(),
            referenced_from: referenced_from.map(str::to_owned),
            searched: self
                .directories
                .iter()
                .map(|directory| directory.display().to_string())
                .collect(),
        }
    }

    /// Read the whole chain from its root, depth first, and merge its declarations.
    fn load_chain(&self, root: String, source: String) -> Result<Template, TemplateError> {
        let mut files = Vec::new();
        let mut edges = Vec::new();
        self.extend(&mut files, &mut edges, root.clone(), source)?;
        let variables = merge(&files, &edges)?;
        Ok(Template::assemble(
            root,
            files,
            edges,
            variables,
            self.clone(),
        ))
    }

    /// Add the file `name` and every file its literal references reach to `files`, depth
    /// first in the order each file names the next, skipping any already there.
    fn extend(
        &self,
        files: &mut Vec<ChainFile>,
        edges: &mut Vec<(usize, String)>,
        name: String,
        source: String,
    ) -> Result<(), TemplateError> {
        let mut seen: HashSet<String> = files.iter().map(|file| file.name.clone()).collect();
        let mut pending = vec![(name, source, None::<usize>)];
        // A stack, pushed in reverse, so the first name a file spells is the next one read.
        while let Some((name, source, parent)) = pending.pop() {
            if let Some(parent) = parent {
                edges.push((parent, name.clone()));
            }
            if !seen.insert(name.clone()) {
                continue;
            }
            let split = front_matter::split(&name, &source)?;
            compile_check(&name, &split.body, split.offset_lines)?;
            let index = files.len();
            let mut children = Vec::new();
            for reference in scan::references(&split.body) {
                // The first candidate that resolves is the one the render loads, so it is the
                // one that belongs to the chain; the rest are not read.
                let mut resolved = false;
                for candidate in &reference.candidates {
                    if seen.contains(candidate) || *candidate == name {
                        edges.push((index, candidate.clone()));
                        resolved = true;
                        break;
                    }
                    if let Some(source) = self.find(candidate)? {
                        children.push((candidate.clone(), source, Some(index)));
                        resolved = true;
                        break;
                    }
                }
                if !resolved && !reference.optional {
                    return Err(self.not_found(&reference.candidates.join(" or "), Some(&name)));
                }
            }
            pending.extend(children.into_iter().rev());
            files.push(ChainFile {
                name,
                source,
                body: split.body,
                offset_lines: split.offset_lines,
                declarations: split.declarations,
            });
        }
        Ok(())
    }
}

/// Refuse a body that does not parse, before anything renders it.
fn compile_check(name: &str, body: &str, offset_lines: usize) -> Result<(), TemplateError> {
    let environment = environment();
    environment
        .template_from_named_str(name, body)
        .map(|_| ())
        .map_err(|error| TemplateError::Malformed {
            file: name.to_owned(),
            key: None,
            message: format!(
                "its body does not parse{}: {}",
                error
                    .line()
                    .map(|line| format!(" at line {}", line + offset_lines))
                    .unwrap_or_default(),
                error.detail().unwrap_or("a syntax error")
            ),
        })
}

/// One file of a loaded chain.
#[derive(Debug, Clone)]
struct ChainFile {
    name: String,
    source: String,
    body: String,
    offset_lines: usize,
    declarations: Vec<Declaration>,
}

/// Merge every chain file's declarations into the declared set.
///
/// Nearness is the fewest references from the rendered file, ties going to the file loaded
/// first; the order is the files' first-load order, each variable where it was first
/// declared.
fn merge(
    files: &[ChainFile],
    edges: &[(usize, String)],
) -> Result<Vec<TemplateVariable>, TemplateError> {
    let index_of: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(index, file)| (file.name.as_str(), index))
        .collect();
    let mut depth = vec![usize::MAX; files.len()];
    if !files.is_empty() {
        depth[0] = 0;
    }
    let mut queue = VecDeque::from([0usize]);
    while let Some(at) = queue.pop_front() {
        for (_, child) in edges.iter().filter(|(parent, _)| *parent == at) {
            if let Some(&child) = index_of.get(child.as_str())
                && depth[child] == usize::MAX
            {
                depth[child] = depth[at] + 1;
                queue.push_back(child);
            }
        }
    }
    let mut nearness: Vec<usize> = (0..files.len()).collect();
    nearness.sort_by_key(|&index| (depth[index], index));

    let mut merged: BTreeMap<&str, TemplateVariable> = BTreeMap::new();
    for &index in &nearness {
        let file = &files[index];
        for declaration in &file.declarations {
            match merged.get(declaration.name.as_str()) {
                None => {
                    merged.insert(
                        &declaration.name,
                        TemplateVariable {
                            name: declaration.name.clone(),
                            description: declaration.description.clone(),
                            kind: declaration.kind,
                            items: declaration.items,
                            required: declaration.required,
                            default: declaration.default.clone(),
                            declared_in: file.name.clone(),
                        },
                    );
                }
                Some(nearer) => {
                    if nearer.kind != declaration.kind {
                        return Err(conflict(
                            nearer,
                            ChainField::Type,
                            nearer.kind.as_str(),
                            &file.name,
                            declaration.kind.as_str(),
                            &declaration.name,
                        ));
                    }
                    if nearer.items != declaration.items {
                        let spell =
                            |items: Option<ItemType>| items.map_or("none", ItemType::as_str);
                        return Err(conflict(
                            nearer,
                            ChainField::Items,
                            spell(nearer.items),
                            &file.name,
                            spell(declaration.items),
                            &declaration.name,
                        ));
                    }
                }
            }
        }
    }

    let mut ordered = Vec::with_capacity(merged.len());
    for file in files {
        for declaration in &file.declarations {
            if let Some(variable) = merged.remove(declaration.name.as_str()) {
                ordered.push(variable);
            }
        }
    }
    Ok(ordered)
}

fn conflict(
    nearer: &TemplateVariable,
    field: ChainField,
    nearer_value: &'static str,
    farther: &str,
    farther_value: &'static str,
    variable: &str,
) -> TemplateError {
    TemplateError::ChainConflict {
        variable: variable.to_owned(),
        field,
        nearer: nearer.declared_in.clone(),
        nearer_value,
        farther: farther.to_owned(),
        farther_value,
    }
}

/// The digest over `files`, each a resolved name and its full source, in first-load order.
fn digest<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut hasher = Sha256::new();
    for (name, source) in files {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(source.as_bytes());
        hasher.update([0]);
    }
    let hash = hasher.finalize();
    let mut rendered = String::with_capacity(7 + hash.len() * 2);
    rendered.push_str("sha256:");
    for byte in hash {
        rendered.push_str(&format!("{byte:02x}"));
    }
    rendered
}

/// The renderer every template is compiled and rendered in.
fn environment() -> minijinja::Environment<'static> {
    let mut environment = minijinja::Environment::new();
    environment.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    environment.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    environment.set_keep_trailing_newline(true);
    environment.set_trim_blocks(true);
    environment.set_lstrip_blocks(true);
    environment
}

/// A loaded template: its chain, its declared set and its digest.
#[derive(Debug, Clone)]
pub struct Template {
    name: String,
    files: Vec<ChainFile>,
    /// Which file names which, by index into `files`: what nearness is measured over.
    edges: Vec<(usize, String)>,
    variables: Vec<TemplateVariable>,
    digest: String,
    loader: TemplateLoader,
}

/// What `onetaskgraph template variables` answers with: a template's declared set.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TemplateVariables {
    /// The rendered template's resolved name.
    pub template: String,
    /// The chain's digest: `sha256:` and 64 lowercase hex digits, over every file it reads
    /// in first-load order — each template an expression names counted as it names one when
    /// every variable takes its default.
    pub digest: String,
    /// Every declared variable, in declaration order along the chain.
    pub variables: Vec<TemplateVariable>,
}

/// A rendered template: what `onetaskgraph template render` answers with.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct RenderedTemplate {
    /// The rendered text.
    pub body: String,
    /// The digest of every file the render read, in first-load order: `sha256:` and 64
    /// lowercase hex digits. The files the chain names by a literal come first, then each
    /// template an expression named for these answers, in the order the render reached it.
    pub digest: String,
    /// Every declared variable and the value it rendered with — an answer, a default, or
    /// `null` for an optional variable given neither.
    pub answers: BTreeMap<String, Value>,
}

impl Template {
    fn assemble(
        name: String,
        files: Vec<ChainFile>,
        edges: Vec<(usize, String)>,
        variables: Vec<TemplateVariable>,
        loader: TemplateLoader,
    ) -> Self {
        let digest = digest(
            files
                .iter()
                .map(|file| (file.name.as_str(), file.source.as_str())),
        );
        Self {
            name,
            files,
            edges,
            variables,
            digest,
            loader,
        }
    }

    /// This template with every template an expression names for `answers` added to its
    /// chain — each with the files its own literals name — and their front matter merged into
    /// the declared set under the same rules as the rest of the chain.
    ///
    /// Which file an expression names can depend on the answers, so it is found by rendering:
    /// leniently, from the answers that fit their declarations and the defaults of the rest,
    /// with every file the render asks for that the chain does not hold read through the
    /// search path. A file found that way may declare a variable that decides the next one,
    /// so it repeats until a render asks for nothing new. The files so found are farther from
    /// the rendered template than any its literals name, so a redeclaration there yields to
    /// theirs, and they follow them in first-load order, in the order the renders reached
    /// them. A template named by no expression expands to itself.
    ///
    /// [`Template::unanswered`], [`Template::resolve`] and [`Template::render`] each expand
    /// first, so a caller need not.
    ///
    /// # Errors
    ///
    /// [`TemplateError::Malformed`], [`TemplateError::Unreadable`] and
    /// [`TemplateError::NotFound`] for a file found this way, or one its literals name, as for
    /// any chain file; and [`TemplateError::ChainConflict`] for a declaration of it whose
    /// `type` or `items` another chain file's contradicts.
    pub fn expand(&self, answers: &Answers) -> Result<Self, TemplateError> {
        let mut files = self.files.clone();
        let mut edges = self.edges.clone();
        let mut variables = self.variables.clone();
        loop {
            let found: Vec<(String, String)> =
                discover(&self.name, &files, &variables, &self.loader, answers)?
                    .into_iter()
                    .filter(|(name, _)| files.iter().all(|file| file.name != *name))
                    .collect();
            if found.is_empty() {
                break;
            }
            for (name, source) in found {
                self.loader.extend(&mut files, &mut edges, name, source)?;
            }
            variables = merge(&files, &edges)?;
        }
        Ok(Self::assemble(
            self.name.clone(),
            files,
            edges,
            variables,
            self.loader.clone(),
        ))
    }

    /// The rendered template's resolved name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The chain's digest.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The declared set, in declaration order along the chain.
    #[must_use]
    pub fn variables(&self) -> &[TemplateVariable] {
        &self.variables
    }

    /// The resolved name of every file of the chain, in first-load order: the files
    /// [`Template::digest`] is taken over. For a loaded template these are the rendered one
    /// and each `extends`, `include` and `import` its literals reach; a template an expression
    /// names joins them in what [`Template::expand`] answers.
    pub fn chain(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|file| file.name.as_str())
    }

    /// The declared set as `template variables` reports it.
    #[must_use]
    pub fn describe(&self) -> TemplateVariables {
        TemplateVariables {
            template: self.name.clone(),
            digest: self.digest.clone(),
            variables: self.variables.clone(),
        }
    }

    /// The declared variables `answers` leaves unanswered, in declaration order — whether or
    /// not a default covers them — over the chain [`Template::expand`] reaches for `answers`.
    /// What a prompting caller asks for; answering one can name a further template, so a
    /// caller asks again until nothing new is left.
    ///
    /// # Errors
    ///
    /// [`TemplateError::UnknownAnswer`] and [`TemplateError::MistypedAnswer`], as
    /// [`Template::render`] refuses them.
    pub fn unanswered(&self, answers: &Answers) -> Result<Vec<TemplateVariable>, TemplateError> {
        let expanded = self.expand(answers)?;
        let typed = expanded.typed(answers)?;
        Ok(expanded
            .variables
            .into_iter()
            .filter(|variable| !typed.contains_key(&variable.name))
            .collect())
    }

    /// Every declared variable's value — over the chain [`Template::expand`] reaches for
    /// `answers` — its answer, else its default, else `null` for an optional one.
    ///
    /// # Errors
    ///
    /// [`TemplateError::UnknownAnswer`] naming every answer to no declared variable,
    /// [`TemplateError::MistypedAnswer`] for an answer not of its variable's type, and
    /// [`TemplateError::MissingRequired`] naming every required variable left unanswered.
    pub fn resolve(&self, answers: &Answers) -> Result<BTreeMap<String, Value>, TemplateError> {
        self.expand(answers)?.resolve_here(answers)
    }

    /// [`Template::resolve`] over this chain as it stands.
    fn resolve_here(&self, answers: &Answers) -> Result<BTreeMap<String, Value>, TemplateError> {
        let mut typed = self.typed(answers)?;
        let missing: Vec<String> = self
            .variables
            .iter()
            .filter(|variable| {
                variable.required
                    && variable.default.is_none()
                    && !typed.contains_key(&variable.name)
            })
            .map(|variable| variable.name.clone())
            .collect();
        if !missing.is_empty() {
            return Err(TemplateError::MissingRequired { names: missing });
        }
        for variable in &self.variables {
            typed
                .entry(variable.name.clone())
                .or_insert_with(|| variable.default.clone().unwrap_or(Value::Null));
        }
        Ok(typed)
    }

    /// Render the template from `answers`, over the chain [`Template::expand`] reaches for
    /// them: a template an expression names is part of the chain, its declarations part of
    /// the declared set the answers are held to, and its bytes part of the digest.
    ///
    /// # Errors
    ///
    /// Every refusal of [`Template::expand`] and [`Template::resolve`], and
    /// [`TemplateError::Render`] for a render that fails — an undefined name among them.
    pub fn render(&self, answers: &Answers) -> Result<RenderedTemplate, TemplateError> {
        self.expand(answers)?.render_here(answers)
    }

    /// [`Template::render`] over this chain as it stands.
    ///
    /// A template the strict render asks for that the chain does not hold — one expansion's
    /// lenient render could not reach, because it failed before getting there — is read
    /// through the search path as it is asked for, counted into the digest after the chain,
    /// and refused if it declares a variable, since answers were already held to a declared
    /// set without it.
    fn render_here(&self, answers: &Answers) -> Result<RenderedTemplate, TemplateError> {
        let resolved = self.resolve_here(answers)?;

        let offsets: HashMap<String, usize> = self
            .files
            .iter()
            .map(|file| (file.name.clone(), file.offset_lines))
            .collect();
        let bodies: Arc<HashMap<String, String>> = Arc::new(
            self.files
                .iter()
                .map(|file| (file.name.clone(), file.body.clone()))
                .collect(),
        );
        let loader = self.loader.clone();
        let loaded: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let recorded = Arc::clone(&loaded);
        let mut environment = environment();
        environment.set_loader(move |name| {
            if let Some(body) = bodies.get(name) {
                return Ok(Some(body.clone()));
            }
            // A file that is there and cannot be read is that failure, not a missing file.
            let Some(source) = loader.find(name).map_err(|error| {
                minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, error.to_string())
            })?
            else {
                return Ok(None);
            };
            let split = front_matter::split(name, &source).map_err(|error| {
                minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, error.to_string())
            })?;
            if !split.declarations.is_empty() {
                return Err(minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    format!(
                        "{name} is named by an expression the variables' defaults and answers \
                         did not reach before rendering, and declares variables; name it with \
                         a string literal"
                    ),
                ));
            }
            recorded
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((name.to_owned(), source));
            Ok(Some(split.body))
        });

        let rendered = environment
            .get_template(&self.name)
            .and_then(|template| template.render(&resolved))
            .map_err(|error| render_error(&error, &offsets))?;
        let loaded = loaded.lock().unwrap_or_else(PoisonError::into_inner);
        let digest = if loaded.is_empty() {
            self.digest.clone()
        } else {
            digest(
                self.files
                    .iter()
                    .map(|file| (file.name.as_str(), file.source.as_str()))
                    .chain(
                        loaded
                            .iter()
                            .map(|(name, source)| (name.as_str(), source.as_str())),
                    ),
            )
        };
        Ok(RenderedTemplate {
            body: rendered,
            digest,
            answers: resolved,
        })
    }

    /// Check `answers` against the declared set and type each one.
    fn typed(&self, answers: &Answers) -> Result<BTreeMap<String, Value>, TemplateError> {
        let declared: HashMap<&str, &TemplateVariable> = self
            .variables
            .iter()
            .map(|variable| (variable.name.as_str(), variable))
            .collect();
        let unknown: Vec<String> = answers
            .names()
            .filter(|name| !declared.contains_key(name))
            .map(str::to_owned)
            .collect();
        if !unknown.is_empty() {
            return Err(TemplateError::UnknownAnswer { names: unknown });
        }

        let mut typed = BTreeMap::new();
        for variable in &self.variables {
            let value = match answers.given(&variable.name) {
                None => continue,
                Some(Given::Value(value)) => variable.check(value).map(|()| value.clone()),
                Some(Given::Text(text)) => variable.parse(text),
            };
            let value = value.map_err(|problem| TemplateError::MistypedAnswer {
                name: variable.name.clone(),
                kind: variable.kind,
                expected: article(variable.kind),
                problem,
            })?;
            typed.insert(variable.name.clone(), value);
        }
        Ok(typed)
    }
}

/// Every file a lenient render of `files` asks for that they do not hold, with its source, in
/// the order it asked.
///
/// The render is only a way to learn what an expression names: its output is dropped, and
/// so is any failure of the render itself — a strict render reports those. A file it asks
/// for that is there and cannot be read is that file's failure, and is answered as one. Each variable takes the answer given
/// for it when that is one of its type, else its default; one with neither is left undefined,
/// which a lenient render reads as empty.
fn discover(
    root: &str,
    files: &[ChainFile],
    variables: &[TemplateVariable],
    loader: &TemplateLoader,
    answers: &Answers,
) -> Result<Vec<(String, String)>, TemplateError> {
    let mut context = BTreeMap::new();
    for variable in variables {
        let answered = match answers.given(&variable.name) {
            Some(Given::Value(value)) => variable.check(value).ok().map(|()| value.clone()),
            Some(Given::Text(text)) => variable.parse(text).ok(),
            None => None,
        };
        if let Some(value) = answered.or_else(|| variable.default.clone()) {
            context.insert(variable.name.clone(), value);
        }
    }

    let bodies: HashMap<String, String> = files
        .iter()
        .map(|file| (file.name.clone(), file.body.clone()))
        .collect();
    let loader = loader.clone();
    let found: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let recorded = Arc::clone(&found);
    let failed: Arc<Mutex<Option<TemplateError>>> = Arc::default();
    let failure = Arc::clone(&failed);
    let mut environment = environment();
    environment.set_undefined_behavior(minijinja::UndefinedBehavior::Lenient);
    environment.set_loader(move |name| {
        if let Some(body) = bodies.get(name) {
            return Ok(Some(body.clone()));
        }
        let source = match loader.find(name) {
            Ok(Some(source)) => source,
            Ok(None) => return Ok(None),
            Err(error) => {
                failure
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get_or_insert(error);
                return Ok(None);
            }
        };
        let body = front_matter::split(name, &source)
            .map(|split| split.body)
            .unwrap_or_default();
        recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((name.to_owned(), source));
        Ok(Some(body))
    });
    let _ = environment
        .get_template(root)
        .and_then(|template| template.render(&context));
    if let Some(error) = failed.lock().unwrap_or_else(PoisonError::into_inner).take() {
        return Err(error);
    }
    let found = found.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(found.clone())
}

/// A minijinja failure as the template failure it is, located in the file's own lines.
///
/// A failure inside an included or imported file reaches here wrapped in one per template
/// it passed through; the innermost is where it happened.
fn render_error(error: &minijinja::Error, offsets: &HashMap<String, usize>) -> TemplateError {
    let mut error = error;
    while let Some(inner) = std::error::Error::source(error)
        .and_then(|source| source.downcast_ref::<minijinja::Error>())
    {
        error = inner;
    }
    let file = error.name().map(str::to_owned);
    let offset = file
        .as_deref()
        .and_then(|file| offsets.get(file))
        .copied()
        .unwrap_or(0);
    let line = error.line().map(|line| line + offset);
    if error.kind() == minijinja::ErrorKind::UndefinedError {
        let name = error
            .template_source()
            .zip(error.range())
            .and_then(|(source, range)| source.get(range))
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        return TemplateError::Render {
            message: match &name {
                Some(name) => format!(
                    "{name} is undefined: it is neither a declared variable nor set by the template"
                ),
                None => "a value there is undefined".to_owned(),
            },
            file,
            line,
            name,
        };
    }
    TemplateError::Render {
        file,
        line,
        name: None,
        message: error
            .detail()
            .map_or_else(|| error.kind().to_string(), str::to_owned),
    }
}
