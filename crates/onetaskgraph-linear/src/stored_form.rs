//! What Linear stores when it is written Markdown, as this source declares it.
//!
//! [`stored_form`] is the rewrite the crate documentation's ruling *what Linear does to Markdown
//! on write, declared* states, and that ruling is its one authoritative statement: every rule
//! there is implemented here, each with the example the ruling gives, and
//! `every_example_the_ruling_states_is_what_the_rewrite_stores` reads the examples out of the
//! ruling itself, so the two cannot part. The engine applies it before a content write, so what
//! it sends is what Linear keeps.

/// The top-level domains a bare domain-like token is autolinked under — the last label of its
/// host, compared without regard to case.
///
/// A list rather than every top-level domain, because a list is what can be declared and held:
/// a token under another one is left as written, and if Linear links it after all the stored
/// content differs from what was sent, which the store reports rather than records.
pub const AUTOLINKED_TOP_LEVEL_DOMAINS: &[&str] = &[
    "ai", "app", "co", "com", "dev", "edu", "gov", "info", "io", "live", "me", "net", "org",
];

/// `content` as Linear stores it: every rule of the ruling applied, outside a fenced block and
/// outside an inline code span, each of which is kept byte for byte.
///
/// Applying it to its own output changes nothing, which is what the ruling records of Linear.
#[must_use]
pub fn stored_form(content: &str) -> String {
    let mut stored = String::with_capacity(content.len());
    let mut fence: Option<(char, usize)> = None;
    // The column a list item's text starts at, while the lines that follow continue it.
    let mut item: Option<usize> = None;
    // Whether the line before closed a fence or held an image alone, which the next line that
    // is not blank is set apart from by a blank line.
    let mut apart = false;
    let mut after_blank = true;
    for line in content.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let end = &line[body.len()..];
        if let Some((mark, length)) = fence {
            if fence_mark(body).is_some_and(|(closing, run, rest)| {
                closing == mark && run >= length && rest.trim().is_empty()
            }) {
                fence = None;
                apart = true;
            }
            stored.push_str(line);
            after_blank = false;
            continue;
        }
        if body.trim().is_empty() {
            stored.push_str(line);
            (item, apart, after_blank) = (None, false, true);
            continue;
        }
        let opens = fence_mark(body);
        let image = lone_image(body);
        if !after_blank && (apart || opens.is_some() || image) {
            stored.push('\n');
        }
        (apart, after_blank) = (image, false);
        if let Some((mark, run, _)) = opens {
            fence = Some((mark, run));
            item = None;
            stored.push_str(line);
            continue;
        }
        let indent = &body[..body.len() - body.trim_start_matches([' ', '\t']).len()];
        let rest = &body[indent.len()..];
        let bullet = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* "));
        let continued = item.filter(|_| bullet.is_none() && !image && !opens_block(rest));
        if let Some(width) = continued {
            stored.push_str(indent);
            stored.push_str(&" ".repeat(width.saturating_sub(indent.len())));
        } else if let Some(row) = delimiter_row(body).filter(|_| !image) {
            item = None;
            stored.push_str(&row);
            stored.push_str(end);
            continue;
        } else {
            stored.push_str(indent);
        }
        let rest = match bullet {
            Some(text) => {
                stored.push_str("* ");
                item = Some(indent.len() + 2);
                text
            }
            None => {
                if continued.is_none() {
                    item = None;
                }
                rest
            }
        };
        if rest.starts_with("-->") {
            stored.push('\\');
        }
        stored.push_str(&inline(rest));
        stored.push_str(end);
    }
    stored
}

/// Whether `line`, its indentation taken off, holds one image and nothing else:
/// `![alt](target)`.
fn lone_image(line: &str) -> bool {
    let text = line.trim();
    text.starts_with("![") && link_end(text, 1) == Some(text.len())
}

/// Whether `text`, a line with its indentation taken off, opens a block of its own that a list
/// item's text does not run on into — a heading, a quotation, an HTML block or an ordered list's
/// first item — so it is left where it is rather than indented under the item.
fn opens_block(text: &str) -> bool {
    text.starts_with(['#', '>', '<'])
        || text.starts_with("+ ")
        || text.starts_with("1. ")
        || text.starts_with("1) ")
}

/// The fence a line opens or closes with — its character, how many of it, and what follows —
/// or `None` when it is no fence line: at most three spaces, then three or more backticks or
/// tildes.
fn fence_mark(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let mark = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = trimmed.len() - trimmed.trim_start_matches(mark).len();
    (run >= 3).then(|| (mark, run, &trimmed[run..]))
}

/// A table's delimiter row as Linear stores it — every cell `--`, its alignment colons kept,
/// one space either side of every pipe — or `None` when `line` is not one.
fn delimiter_row(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('|')?.strip_suffix('|')?;
    let mut cells = Vec::new();
    for cell in inner.split('|') {
        let cell = cell.trim();
        let dashes = cell.trim_start_matches(':').trim_end_matches(':');
        if dashes.is_empty() || !dashes.chars().all(|c| c == '-') || cell.len() - dashes.len() > 2 {
            return None;
        }
        cells.push(format!(
            "{}--{}",
            if cell.starts_with(':') { ":" } else { "" },
            if cell.len() > 1 && cell.ends_with(':') {
                ":"
            } else {
                ""
            }
        ));
    }
    let indent = &line[..line.len() - line.trim_start().len()];
    Some(format!("{indent}| {} |", cells.join(" | ")))
}

/// One line's text outside a fence, its prose rewritten and its code spans and links kept.
fn inline(text: &str) -> String {
    let mut stored = String::with_capacity(text.len());
    let mut previous: Option<char> = None;
    let mut at = 0;
    while let Some(character) = text[at..].chars().next() {
        let taken = match character {
            '`' => {
                let run = text[at..].len() - text[at..].trim_start_matches('`').len();
                let end = code_span_end(text, at, run).unwrap_or(at + run);
                stored.push_str(&text[at..end]);
                end - at
            }
            '\\' => match text[at + 1..].chars().next() {
                Some('"') => {
                    stored.push('"');
                    2
                }
                Some(letter) if letter.is_ascii_alphabetic() => {
                    stored.push_str("\\\\");
                    stored.push(letter);
                    2
                }
                escaped => {
                    let escaped = escaped.map_or(0, char::len_utf8);
                    stored.push_str(&text[at..at + 1 + escaped]);
                    1 + escaped
                }
            },
            '!' if text[at + 1..].starts_with('[') && link_end(text, at + 1).is_some() => {
                let end = link_end(text, at + 1).expect("checked just now");
                stored.push_str(&text[at..end]);
                end - at
            }
            '[' => {
                if let Some(end) = link_end(text, at) {
                    stored.push_str(&text[at..end]);
                    end - at
                } else {
                    stored.push_str("\\[");
                    1
                }
            }
            ']' => {
                stored.push_str("\\]");
                1
            }
            '~' => {
                stored.push_str("\\~");
                1
            }
            '_' => {
                if let Some(close) = emphasis_close(text, at, previous) {
                    stored.push('*');
                    stored.push_str(&text[at + 1..close]);
                    stored.push('*');
                    close + 1 - at
                } else {
                    stored.push('_');
                    1
                }
            }
            _ => match autolink(text, at, previous) {
                Some((token, target)) => {
                    stored.push_str(&format!("[{token}](<{target}>)"));
                    token.len()
                }
                None => {
                    stored.push(character);
                    character.len_utf8()
                }
            },
        };
        previous = text[..at + taken].chars().next_back();
        at += taken;
    }
    stored
}

/// Where the code span opened by the `run` backticks at `at` ends — just past the next run of
/// exactly that many — or `None` when nothing on the line closes it.
fn code_span_end(text: &str, at: usize, run: usize) -> Option<usize> {
    let mut from = at + run;
    while let Some(found) = text[from..].find('`') {
        let start = from + found;
        let length = text[start..].len() - text[start..].trim_start_matches('`').len();
        if length == run {
            return Some(start + length);
        }
        from = start + length;
    }
    None
}

/// Where the Markdown link whose `[` is at `at` ends — `[text](target)`, the target bare or in
/// angle brackets — or `None` when no link starts there.
fn link_end(text: &str, at: usize) -> Option<usize> {
    let close = at + 1 + text[at + 1..].find(']')?;
    if text[at + 1..close].contains(['[', '`', '\\']) {
        return None;
    }
    let target = close + 1;
    let rest = text[target..].strip_prefix('(')?;
    let length = if rest.starts_with('<') {
        rest.find(">)")? + 2
    } else {
        let end = rest.find(')')?;
        if rest[..end].contains(char::is_whitespace) {
            return None;
        }
        end + 1
    };
    Some(target + 1 + length)
}

/// Where the `_` closing a single-underscore emphasis opened at `at` is, or `None`: the opener
/// follows no letter, digit or underscore, the text between holds no underscore, backtick,
/// bracket, tilde, asterisk, backslash or angle bracket and neither begins nor ends with
/// whitespace, and the closer is followed by no letter, digit or underscore.
fn emphasis_close(text: &str, at: usize, previous: Option<char>) -> Option<usize> {
    if previous.is_some_and(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let inner_start = at + 1;
    let close = inner_start + text[inner_start..].find('_')?;
    let inner = &text[inner_start..close];
    let after = text[close + 1..].chars().next();
    let clean = !inner.is_empty()
        && !inner.contains(['`', '[', ']', '~', '*', '\\', '<', '>'])
        && inner.trim() == inner;
    (clean && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')).then_some(close)
}

/// The token an autolink starting at `at` covers and the target Linear links it to, or `None`:
/// a URL opening `http://` or `https://`, or a domain-like token whose last label is one of
/// [`AUTOLINKED_TOP_LEVEL_DOMAINS`], each starting at the start of the text or after a character
/// that cannot continue a word, a path, an address or an escape — not after a letter, a digit or
/// one of `@ / . - _ : \ # = + %` — and ending before whitespace, with any trailing punctuation
/// left outside it.
fn autolink(text: &str, at: usize, previous: Option<char>) -> Option<(&str, String)> {
    if previous.is_some_and(|c| {
        c.is_alphanumeric()
            || matches!(
                c,
                '@' | '/' | '.' | '-' | '_' | ':' | '\\' | '#' | '=' | '+' | '%'
            )
    }) {
        return None;
    }
    let rest = &text[at..];
    let stops = |c: char| c.is_whitespace() || matches!(c, '<' | '>' | '`' | '[' | ']');
    let trailing: &[char] = &['.', ',', ':', ';', '!', '?', '\'', '"', ')', '*', '_', '~'];
    if rest.starts_with("http://") || rest.starts_with("https://") {
        let scheme = rest.find("://").expect("checked just now") + 3;
        let end = rest.find(stops).unwrap_or(rest.len());
        let token = rest[..end].trim_end_matches(trailing);
        return (token.len() > scheme).then(|| (token, token.to_owned()));
    }
    let host_end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.'))
        .unwrap_or(rest.len());
    let host = rest[..host_end].trim_end_matches(['.', '-']);
    let labels: Vec<&str> = host.split('.').collect();
    let top = labels.last()?.to_ascii_lowercase();
    if labels.len() < 2
        || labels.iter().any(|label| label.is_empty())
        || !AUTOLINKED_TOP_LEVEL_DOMAINS.contains(&top.as_str())
    {
        return None;
    }
    let after_host = rest[host.len()..].chars().next();
    let token = if host.len() == host_end && after_host == Some('/') {
        let end = rest.find(stops).unwrap_or(rest.len());
        rest[..end].trim_end_matches(trailing)
    } else {
        if after_host.is_some_and(|c| c == '@' || c == '_') {
            return None;
        }
        host
    };
    Some((token, format!("http://{token}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `input` → `stored` example the ruling in the crate documentation states, read out
    /// of the ruling itself: a row of its table whose two cells are code spans.
    fn ruled_examples() -> Vec<(String, String)> {
        let source = include_str!("lib.rs");
        let start = source
            .find("//! ## Ruling: what Linear does to Markdown on write, declared")
            .expect("the ruling is in the crate documentation");
        let section = &source[start..];
        let end = section[3..]
            .find("//! ## ")
            .map_or(section.len(), |at| at + 3);
        section[..end]
            .lines()
            .filter_map(|line| {
                let row = line.strip_prefix("//! | ")?;
                let cells: Vec<&str> = row.trim_end_matches(" |").split(" | ").collect();
                let [_, input, stored] = cells[..] else {
                    return None;
                };
                let code = |cell: &str| {
                    let cell = cell.strip_prefix("``")?.strip_suffix("``")?;
                    Some(cell.trim().replace("\\n", "\n").replace("\\|", "|"))
                };
                Some((code(input)?, code(stored)?))
            })
            .collect()
    }

    #[test]
    fn every_example_the_ruling_states_is_what_the_rewrite_stores() {
        let examples = ruled_examples();
        assert!(
            examples.len() >= 12,
            "the ruling states an example for every rule: {examples:#?}"
        );
        for (input, stored) in examples {
            assert_eq!(
                stored_form(&input),
                stored,
                "the ruling's example {input:?}"
            );
            assert_eq!(
                stored_form(&stored),
                stored,
                "Linear rewriting its own {stored:?} changes nothing more"
            );
        }
    }

    /// One string constant of `tests/live.rs`, by name: its literal's escapes are JSON's.
    fn live_constant(name: &str) -> String {
        let live = include_str!("../tests/live.rs");
        let opening = format!("const {name}: &str = ");
        let start = live.find(&opening).expect("the live journey declares it") + opening.len();
        let literal = &live[start..start + live[start..].find(";\n").expect("one line")];
        serde_json::from_str(literal).expect("a string literal")
    }

    #[test]
    fn the_probe_linear_was_written_is_stored_as_linear_handed_it_back() {
        assert_eq!(
            stored_form(&live_constant("PROBE")),
            live_constant("PROBE_STORED")
        );
        assert_eq!(
            stored_form(&live_constant("MARKDOWN_PROBE")),
            live_constant("MARKDOWN_PROBE_STORED")
        );
    }

    #[test]
    fn a_fence_and_a_code_span_keep_what_the_prose_rules_rewrite() {
        let held = "```\n- a [x] _y_ example.com ~\n| --- |\n```\n";
        assert_eq!(stored_form(held), held);
        let span = "`- [x] _y_ example.com ~`";
        assert_eq!(stored_form(span), span);
        let tilde = "~~~ text\n- kept\n~~~\n- moved";
        assert_eq!(stored_form(tilde), "~~~ text\n- kept\n~~~\n\n* moved");
    }

    #[test]
    fn what_no_rule_names_comes_back_as_written() {
        for kept in [
            "snake_case_name and file_name.md",
            "caller.key, README.md, v0.8.0 and e.g. this",
            "a@example.com",
            "<!-- onetaskgraph.metadata `{\"k\":\"[x] ~ _y_\"}` -->",
            "**bold**, 1. one, # heading, > quoted - dash",
            "| a | b |",
            "\\[already\\] \\~ escaped",
        ] {
            assert_eq!(stored_form(kept), kept, "{kept:?}");
        }
    }

    #[test]
    fn a_link_and_a_url_ending_a_sentence_are_linked_without_the_full_stop() {
        assert_eq!(
            stored_form("See https://example.com/a. Or example.com."),
            "See [https://example.com/a](<https://example.com/a>). Or \
             [example.com](<http://example.com>)."
        );
        let linked = "[named](https://example.com) and ![shot](./shot.png)";
        assert_eq!(stored_form(linked), linked);
    }
}
