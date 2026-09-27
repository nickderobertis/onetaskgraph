// llmlint: ignore-file[code_lands_in_the_domain_that_owns_it] One part of the `template` module, which sits in this crate for the reason its `mod.rs` states at the head of the file: the task that introduced templates fixes their API at `onetaskgraph-core`'s crate root, and a crate of their own would be a new published sibling that is not this change's to add.
//! A template file's front matter: split off the body, and read strictly by key.
//!
//! Read through `serde_norway::Value` rather than a derived struct, for two reasons a derive
//! cannot serve: a refusal has to name the key it is about (`variables.title.type`, not a
//! serde position), and a file's variables keep the order they were written in, which is the
//! order a prompt asks them in.

use serde_json::Value;
use serde_norway::Value as Yaml;

use super::{ItemType, TemplateError, VariableType, check_value};

/// The front matter's own version key, and the one value it may take.
const VERSION_KEY: &str = "onetaskgraph_template";

/// The only front matter version this build reads.
const VERSION: u64 = 1;

/// Every key front matter may carry, and nothing else: any other is refused by name.
pub const FRONT_MATTER_KEYS: [&str; 3] = [VERSION_KEY, "description", "variables"];

/// Every key one variable's declaration may carry, and nothing else: any other is refused by
/// name.
pub const DECLARATION_KEYS: [&str; 5] = ["description", "type", "items", "required", "default"];

/// `keys` as a message lists them: `` `a`, `b` and `c` ``.
fn listed(keys: &[&str]) -> String {
    let quoted: Vec<String> = keys.iter().map(|key| format!("`{key}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}

/// A file with its front matter taken off.
pub(super) struct Split {
    /// The declarations the front matter made, in the order it made them.
    pub declarations: Vec<Declaration>,
    /// What the renderer compiles: everything after the closing `---` line.
    pub body: String,
    /// How many lines the front matter took, so a body line number can be reported as the
    /// file's own.
    pub offset_lines: usize,
}

/// One variable as one file declares it, before the chain is merged.
#[derive(Debug, Clone)]
pub(super) struct Declaration {
    pub name: String,
    pub description: String,
    pub kind: VariableType,
    pub items: Option<ItemType>,
    pub required: bool,
    pub default: Option<Value>,
}

/// Split `source` into its declarations and its body.
///
/// A file whose first line is not `---` has no front matter and declares nothing. One whose
/// first line is `---` must close it with a later line reading `---`.
pub(super) fn split(file: &str, source: &str) -> Result<Split, TemplateError> {
    let mut lines = source.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return Ok(unfronted(source));
    };
    if trim_line_end(first) != "---" {
        return Ok(unfronted(source));
    }

    let mut consumed = first.len();
    let mut offset_lines = 1;
    let mut matter = String::new();
    let mut closed = false;
    for line in lines {
        consumed += line.len();
        offset_lines += 1;
        if trim_line_end(line) == "---" {
            closed = true;
            break;
        }
        matter.push_str(line);
    }
    if !closed {
        return Err(TemplateError::malformed(
            file,
            None,
            "its first line opens front matter with `---`, and no later line reading `---` \
             closes it",
        ));
    }

    let parsed: Yaml = serde_norway::from_str(&matter).map_err(|error| {
        TemplateError::malformed(file, None, format!("its front matter is not YAML: {error}"))
    })?;
    Ok(Split {
        declarations: read(file, &parsed)?,
        body: source[consumed..].to_owned(),
        offset_lines,
    })
}

/// A file with no front matter: its whole source is the body.
fn unfronted(source: &str) -> Split {
    Split {
        declarations: Vec::new(),
        body: source.to_owned(),
        offset_lines: 0,
    }
}

/// A line without its line ending, so a file written with CRLF reads the same.
fn trim_line_end(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

/// Read the front matter mapping, refusing any key C1 does not define.
fn read(file: &str, matter: &Yaml) -> Result<Vec<Declaration>, TemplateError> {
    let Yaml::Mapping(mapping) = matter else {
        return Err(TemplateError::malformed(
            file,
            None,
            "its front matter is not a mapping; it opens with `onetaskgraph_template: 1`",
        ));
    };

    let mut version = None;
    let mut variables = None;
    for (key, value) in mapping {
        match key_name(file, key, "")?.as_str() {
            VERSION_KEY => version = Some(value),
            "description" => {
                if !matches!(value, Yaml::String(_)) {
                    return Err(TemplateError::malformed(
                        file,
                        Some("description"),
                        "is not a string",
                    ));
                }
            }
            "variables" => variables = Some(value),
            other => {
                return Err(TemplateError::malformed(
                    file,
                    Some(other),
                    format!(
                        "is not a front matter key; the keys are {}",
                        listed(&FRONT_MATTER_KEYS)
                    ),
                ));
            }
        }
    }

    match version {
        None => {
            return Err(TemplateError::malformed(
                file,
                Some(VERSION_KEY),
                "is missing; front matter declares `onetaskgraph_template: 1`",
            ));
        }
        Some(Yaml::Number(number)) if number.as_u64() == Some(VERSION) => {}
        Some(_) => {
            return Err(TemplateError::malformed(
                file,
                Some(VERSION_KEY),
                "is not 1, the only template format version this build reads",
            ));
        }
    }

    let Some(variables) = variables else {
        return Ok(Vec::new());
    };
    let variables = match variables {
        Yaml::Null => return Ok(Vec::new()),
        Yaml::Mapping(variables) => variables,
        _ => {
            return Err(TemplateError::malformed(
                file,
                Some("variables"),
                "is not a mapping from variable name to declaration",
            ));
        }
    };

    variables
        .iter()
        .map(|(name, declaration)| {
            let name = key_name(file, name, "variables.")?;
            declaration_of(file, name, declaration)
        })
        .collect()
}

/// A mapping key as the string it has to be.
fn key_name(file: &str, key: &Yaml, prefix: &str) -> Result<String, TemplateError> {
    match key {
        Yaml::String(key) => Ok(key.clone()),
        other => Err(TemplateError::malformed(
            file,
            Some(&format!("{prefix}{}", render_yaml(other))),
            "is not a string key",
        )),
    }
}

/// A YAML value, compactly, for a message.
fn render_yaml(value: &Yaml) -> String {
    serde_norway::to_string(value)
        .map(|rendered| rendered.trim_end().to_owned())
        .unwrap_or_else(|_| "?".to_owned())
}

/// Whether `name` is a variable name C1 admits: `^[a-z][a-z0-9_]*$`.
fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|rest| rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '_')
}

/// One variable's declaration, refused by key.
fn declaration_of(file: &str, name: String, value: &Yaml) -> Result<Declaration, TemplateError> {
    let base = format!("variables.{name}");
    if !valid_name(&name) {
        return Err(TemplateError::malformed(
            file,
            Some(&base),
            "is not a variable name; a name matches ^[a-z][a-z0-9_]*$",
        ));
    }
    let Yaml::Mapping(fields) = value else {
        return Err(TemplateError::malformed(
            file,
            Some(&base),
            "is not a mapping; a declaration carries at least a `description`",
        ));
    };

    let mut description = None;
    let mut kind = VariableType::String;
    let mut items = None;
    let mut required = None;
    let mut default = None;
    for (key, field) in fields {
        let key = key_name(file, key, &format!("{base}."))?;
        let path = format!("{base}.{key}");
        match key.as_str() {
            "description" => match field {
                Yaml::String(text) if !text.trim().is_empty() => description = Some(text.clone()),
                _ => {
                    return Err(TemplateError::malformed(
                        file,
                        Some(&path),
                        "is not a non-empty string; it is what a prompt shows",
                    ));
                }
            },
            "type" => {
                kind = match field.as_str().and_then(VariableType::parse) {
                    Some(kind) => kind,
                    None => {
                        return Err(TemplateError::malformed(
                            file,
                            Some(&path),
                            "is not one of string, text, integer, boolean, list, object",
                        ));
                    }
                };
            }
            "items" => {
                items = match field.as_str().and_then(ItemType::parse) {
                    Some(items) => Some(items),
                    None => {
                        return Err(TemplateError::malformed(
                            file,
                            Some(&path),
                            "is not one of string, object",
                        ));
                    }
                };
            }
            "required" => match field {
                Yaml::Bool(value) => required = Some(*value),
                _ => {
                    return Err(TemplateError::malformed(
                        file,
                        Some(&path),
                        "is not a boolean",
                    ));
                }
            },
            "default" => {
                default = Some(serde_json::to_value(field).map_err(|error| {
                    TemplateError::malformed(
                        file,
                        Some(&path),
                        format!("cannot be read as a value: {error}"),
                    )
                })?);
            }
            _ => {
                return Err(TemplateError::malformed(
                    file,
                    Some(&path),
                    format!(
                        "is not a declaration key; the keys are {}",
                        listed(&DECLARATION_KEYS)
                    ),
                ));
            }
        }
    }

    let Some(description) = description else {
        return Err(TemplateError::malformed(
            file,
            Some(&format!("{base}.description")),
            "is missing; every variable says what it is for",
        ));
    };
    if items.is_some() && kind != VariableType::List {
        return Err(TemplateError::malformed(
            file,
            Some(&format!("{base}.items")),
            "is given for a variable whose type is not `list`",
        ));
    }
    let items = (kind == VariableType::List).then(|| items.unwrap_or(ItemType::String));
    if required == Some(true) && default.is_some() {
        return Err(TemplateError::malformed(
            file,
            Some(&format!("{base}.required")),
            "is `true` beside a `default`; a variable with a default is optional",
        ));
    }
    if let Some(default) = &default
        && let Err(problem) = check_value(kind, items, default)
    {
        return Err(TemplateError::malformed(
            file,
            Some(&format!("{base}.default")),
            format!("is not a value of the variable's type: {problem}"),
        ));
    }

    Ok(Declaration {
        required: required.unwrap_or(default.is_none()),
        name,
        description,
        kind,
        items,
        default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published key lists are the parser's: every listed key is read, and a key beside
    /// them is refused naming the whole list.
    #[test]
    fn the_published_keys_are_exactly_the_ones_read() {
        let every_key = "---\n\
onetaskgraph_template: 1\n\
description: all of it\n\
variables:\n  \
  steps:\n    \
    description: d\n    \
    type: list\n    \
    items: string\n    \
    required: false\n    \
    default: [a]\n\
---\n";
        let read = split("t.md", every_key).expect("every published key is read");
        assert_eq!(read.declarations.len(), 1);
        assert_eq!(
            FRONT_MATTER_KEYS.len(),
            every_key
                .lines()
                .filter(|line| line.contains(':') && !line.starts_with(' '))
                .count()
        );
        assert_eq!(
            DECLARATION_KEYS.len(),
            every_key
                .lines()
                .filter(|line| line.starts_with("    "))
                .count()
        );

        let refused = split("t.md", "---\nonetaskgraph_template: 1\nother: x\n---\n")
            .err()
            .expect("an unlisted key");
        assert!(
            refused.to_string().contains(&listed(&FRONT_MATTER_KEYS)),
            "{refused}"
        );
        let refused = split(
            "t.md",
            "---\nonetaskgraph_template: 1\nvariables:\n  x: {description: d, other: y}\n---\n",
        )
        .err()
        .expect("an unlisted declaration key");
        assert!(
            refused.to_string().contains(&listed(&DECLARATION_KEYS)),
            "{refused}"
        );
    }
}
