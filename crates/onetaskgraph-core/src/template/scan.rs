//! The templates one template names, read off its source before anything renders.
//!
//! minijinja resolves `extends`, `include` and `import` lazily, as the render reaches them,
//! and the declared set has to be known before that: it is what `template variables` reports
//! and what a prompt asks for. So the chain is read here, statically — every tag naming a
//! template by a string literal, or by a list of them, whichever branch it sits in. A tag
//! naming one by an expression cannot be read before rendering; the renderer still loads
//! what it names, as [`super::Template::render`] says.

/// One template a tag names: one name, or a list of candidates of which the first that
/// resolves is the one loaded — minijinja's own rule for `{% include [a, b] %}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reference {
    /// The names as written, in the order they are tried.
    pub candidates: Vec<String>,
    /// Written with `ignore missing`, so candidates nothing resolves are not an error.
    pub optional: bool,
}

/// Every template `source` names by a literal, in the order its tags name them.
pub(super) fn references(source: &str) -> Vec<Reference> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find('{') {
        let after = &rest[start..];
        if let Some(inner) = after.strip_prefix("{#") {
            rest = skip_past(inner, "#}");
        } else if let Some(inner) = after.strip_prefix("{{") {
            rest = skip_past_quoted(inner, "}}");
        } else if let Some(inner) = after.strip_prefix("{%") {
            let (tag, remaining) = tag_body(inner);
            rest = remaining;
            let tag = tag
                .trim_start_matches(['-', '+'])
                .trim_end_matches(['-', '+']);
            let (word, arguments) = split_word(tag);
            match word {
                "raw" => rest = skip_raw(rest),
                "extends" | "import" | "from" | "include" => {
                    let candidates = literal_names(arguments);
                    if !candidates.is_empty() {
                        found.push(Reference {
                            candidates,
                            optional: word == "include" && arguments.contains("ignore missing"),
                        });
                    }
                }
                _ => {}
            }
        } else {
            rest = &after[1..];
        }
    }
    found
}

/// What follows the first `end` in `text`, or nothing when there is none.
fn skip_past<'a>(text: &'a str, end: &str) -> &'a str {
    text.find(end).map_or("", |at| &text[at + end.len()..])
}

/// What follows the first `end` outside a string literal.
fn skip_past_quoted<'a>(text: &'a str, end: &str) -> &'a str {
    let (_, rest) = until_unquoted(text, end);
    rest
}

/// A tag's body up to its closing `%}`, and what follows it.
fn tag_body(text: &str) -> (&str, &str) {
    until_unquoted(text, "%}")
}

/// Split `text` at the first `end` outside a string literal.
fn until_unquoted<'a>(text: &'a str, end: &str) -> (&'a str, &'a str) {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        match quote {
            Some(open) => {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == open {
                    quote = None;
                }
            }
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                } else if text[index..].starts_with(end) {
                    return (&text[..index], &text[index + end.len()..]);
                }
            }
        }
    }
    (text, "")
}

/// Skip a `{% raw %}` block to just past its `{% endraw %}`.
fn skip_raw(text: &str) -> &str {
    let mut rest = text;
    while let Some(start) = rest.find("{%") {
        let (tag, remaining) = tag_body(&rest[start + 2..]);
        let tag = tag
            .trim_start_matches(['-', '+'])
            .trim_end_matches(['-', '+']);
        if split_word(tag).0 == "endraw" {
            return remaining;
        }
        rest = remaining;
    }
    ""
}

/// A tag's first word and the rest of it.
fn split_word(tag: &str) -> (&str, &str) {
    let tag = tag.trim();
    match tag.find(char::is_whitespace) {
        Some(at) => (&tag[..at], tag[at..].trim_start()),
        None => (tag, ""),
    }
}

/// The names an argument spells as a string literal or a list of them, and none when it
/// is any other expression.
fn literal_names(arguments: &str) -> Vec<String> {
    let arguments = arguments.trim_start();
    if let Some(list) = arguments.strip_prefix('[') {
        let mut names = Vec::new();
        let mut rest = list;
        loop {
            rest = rest.trim_start();
            if rest.starts_with(']') {
                return names;
            }
            let Some((name, after)) = string_literal(rest) else {
                return Vec::new();
            };
            names.push(name);
            rest = after.trim_start();
            if let Some(after) = rest.strip_prefix(',') {
                rest = after;
            } else if rest.starts_with(']') {
                return names;
            } else {
                return Vec::new();
            }
        }
    }
    match string_literal(arguments) {
        // A literal followed by `~` or `+` is the start of an expression, not a name.
        Some((name, after)) if !after.trim_start().starts_with(['~', '+', '|', '.', '[']) => {
            vec![name]
        }
        _ => Vec::new(),
    }
}

/// One string literal at the start of `text`, unescaped, and what follows it.
fn string_literal(text: &str) -> Option<(String, &str)> {
    let mut characters = text.char_indices();
    let (_, open) = characters.next()?;
    if open != '"' && open != '\'' {
        return None;
    }
    let mut value = String::new();
    let mut escaped = false;
    for (index, character) in characters {
        if escaped {
            value.push(match character {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == open {
            return Some((value, &text[index + 1..]));
        } else {
            value.push(character);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str) -> Vec<(Vec<String>, bool)> {
        references(source)
            .into_iter()
            .map(|reference| (reference.candidates, reference.optional))
            .collect()
    }

    fn one(name: &str) -> (Vec<String>, bool) {
        (vec![name.to_owned()], false)
    }

    #[test]
    fn every_kind_of_tag_is_read_in_the_order_it_is_written() {
        let source = "{% extends \"base.md\" %}\n\
                      {%- include 'part.md' -%}\n\
                      {% import \"macros.md\" as m %}\n\
                      {% from 'helpers.md' import one, two %}\n\
                      {% include ['a.md', \"b.md\"] ignore missing %}";
        assert_eq!(
            names(source),
            [
                one("base.md"),
                one("part.md"),
                one("macros.md"),
                one("helpers.md"),
                (vec!["a.md".to_owned(), "b.md".to_owned()], true),
            ]
        );
    }

    #[test]
    fn comments_raw_blocks_expressions_and_strings_name_nothing() {
        let source = "{# {% include 'commented.md' %} #}\n\
                      {% raw %}{% include 'raw.md' %}{% endraw %}\n\
                      {{ '{% include \"quoted.md\" %}' }}\n\
                      {% include kind ~ '.md' %}\n\
                      {% include 'prefix-' ~ kind %}\n\
                      {% if x %}{% include 'branch.md' %}{% endif %}";
        assert_eq!(names(source), [one("branch.md")]);
    }
}
