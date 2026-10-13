//! Every YAML read and write this source makes, through `serde-saphyr`.
//!
//! A read takes serde-saphyr's own behaviour with three options: `yes`, `no`, `on` and `off`
//! stay strings, because a write leaves them plain; `<<` is an ordinary key; and a refusal is
//! one line, which a message names its file beside. The engine reads with the same three, and
//! `onetaskgraph-core`'s `tests/yaml.rs` holds the two readers to the same readings.
//!
//! A write keeps the bytes the files of a store already hold. A store is a long-lived
//! directory on a user's disk, often under version control, so a record rewritten after the
//! YAML crate moved must not come out re-laid where nothing about it changed. Two options and
//! four passes over serde-saphyr's output hold that, each for a shape real records hold:
//! no long string folded into `>-`; no `%YAML 1.2` header, which a write under YAML 1.2 rules
//! (so that a date-, time- or boolean-shaped string stays plain) begins with; a literal block
//! holding a line that ends in a space written as one double-quoted line; a blank line inside
//! a literal block written empty; a literal block opening with a blank line given its
//! indentation indicator; and a scalar needing quotes but no escape single-quoted.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// What serde-saphyr writes first under [`serde_saphyr::SerializerOptions::yaml_12`].
const YAML_12_HEADER: &str = "%YAML 1.2\n---\n";

/// The indentation indicator a literal block opening with a blank line carries in a file.
const LEADING_BREAK_INDICATOR: char = '2';

/// Why a read refused: what YAML found wrong and where, after the path of the field it was
/// reading when that is not the document itself.
#[derive(Debug)]
pub(crate) struct Error {
    path: Option<String>,
    error: Box<serde_saphyr::Error>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{path}: {}", self.error),
            None => write!(f, "{}", self.error),
        }
    }
}

pub(crate) fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    let options = serde_saphyr::options! {
        strict_booleans: true,
        merge_keys: serde_saphyr::MergeKeyPolicy::AsOrdinary,
        with_snippet: false,
    };
    let mut path = None;
    serde_saphyr::with_deserializer_from_str_with_options(text, options, |deserializer| {
        serde_path_to_error::deserialize(deserializer).map_err(|error| {
            let at = error.path().to_string();
            path = (at != ".").then_some(at);
            error.into_inner()
        })
    })
    .map_err(|error| Error {
        path,
        error: Box::new(error),
    })
}

/// `value` written as the files of a store spell it.
pub(crate) fn to_string<T: Serialize>(value: &T) -> Result<String, serde_saphyr::SerializeError> {
    let written = serde_saphyr::to_string_with_options(value, write_options())?;
    let written = written.strip_prefix(YAML_12_HEADER).unwrap_or(&written);
    let quoted = quote_space_breaks(written)?;
    Ok(single_quote(&indicate_leading_breaks(
        &strip_blank_block_lines(&quoted),
    )))
}

fn write_options() -> serde_saphyr::SerializerOptions {
    serde_saphyr::ser_options! {
        folded_wrap_chars: usize::MAX,
        yaml_12: true,
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn blank(line: &str) -> bool {
    line.trim_end_matches('\n').chars().all(|c| c == ' ')
}

/// A literal block scalar's header on one line: where its `|` token starts, the indentation
/// its node sits at, and its explicit indentation indicator when it has one.
struct Header {
    token: usize,
    parent: usize,
    indicator: Option<usize>,
}

/// The literal block header `line` ends in, if it ends in one: `key: |-`, `- |` and the like.
fn literal_header(line: &str) -> Option<Header> {
    let body = line.trim_end_matches('\n').trim_end();
    // The node's own column: a key's, or for an item of a sequence its dash's.
    let mut start = indent_of(body);
    let mut dash = None;
    while body[start..].starts_with("- ") {
        dash = Some(start);
        start += 2;
    }
    let (token, parent) = match body.rfind(": ") {
        Some(colon) => (colon + 2, start),
        None => (start, dash.unwrap_or(start)),
    };
    let header = &body[token..];
    let rest = header.strip_prefix('|')?;
    if !rest
        .chars()
        .all(|c| c == '-' || c == '+' || c.is_ascii_digit())
    {
        return None;
    }
    let indicator = rest
        .chars()
        .find(char::is_ascii_digit)
        .and_then(|digit| digit.to_digit(10))
        .map(|digit| digit as usize);
    Some(Header {
        token,
        parent,
        indicator,
    })
}

/// The block opened by the header on line `at`: its content indentation and the line after
/// its last.
fn block_after(lines: &[&str], at: usize, header: &Header) -> Option<(usize, usize)> {
    let parent = header.parent;
    let indent = match header.indicator {
        Some(indicator) => parent + indicator,
        None => lines[at + 1..]
            .iter()
            .find(|line| !blank(line))
            .map(|line| indent_of(line))
            .filter(|indent| *indent > parent)?,
    };
    let end = lines[at + 1..]
        .iter()
        .position(|line| !blank(line) && indent_of(line) < indent)
        .map_or(lines.len(), |offset| at + 1 + offset);
    Some((indent, end))
}

/// A literal block one of whose lines ends in a space, written as one double-quoted line.
///
/// A space before a line break is what makes a string's block form unwritable by the
/// emitter the files were written with, which double-quotes it instead.
fn quote_space_breaks(text: &str) -> Result<String, serde_saphyr::SerializeError> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        let block = literal_header(line)
            .filter(|header| header.indicator.is_none())
            .and_then(|header| Some((block_after(&lines, at, &header)?, header)));
        let Some(((indent, end), header)) = block else {
            out.push_str(line);
            at += 1;
            continue;
        };
        let content = &lines[at + 1..end];
        let spaced = content.iter().any(|line| {
            let line = line.trim_end_matches('\n');
            line.ends_with(' ') && (!blank(line) || line.len() > indent)
        });
        if !spaced {
            out.push_str(line);
            out.extend(content.iter().copied());
            at = end;
            continue;
        }
        let token = &line.trim_end_matches('\n')[header.token..];
        let mut alone = format!("v: {}\n", token.trim_end());
        for line in content {
            alone.push_str("  ");
            alone.push_str(line.get(indent..).unwrap_or("\n"));
        }
        let value: std::collections::BTreeMap<String, String> = from_str(&alone)
            .map_err(<serde_saphyr::SerializeError as serde::ser::Error>::custom)?;
        let mut options = write_options();
        options.prefer_block_scalars = false;
        let quoted = serde_saphyr::to_string_with_options(&value["v"], options)?;
        let quoted = quoted.strip_prefix(YAML_12_HEADER).unwrap_or(&quoted);
        out.push_str(&line[..header.token]);
        out.push_str(quoted.trim_end_matches('\n'));
        out.push('\n');
        at = end;
    }
    Ok(out)
}

/// Inside a literal block scalar, a line holding only spaces, no more of them than the
/// block's indentation, is an empty line of the block (YAML 1.2 §8.1.1.2), so it is written
/// empty and reads back the same string. A line with more spaces than the indentation carries
/// content and is left exactly as it is.
fn strip_blank_block_lines(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        out.push_str(line);
        at += 1;
        let Some((indent, end)) =
            literal_header(line).and_then(|header| block_after(&lines, at - 1, &header))
        else {
            continue;
        };
        for line in &lines[at..end] {
            let content = line.trim_end_matches('\n');
            if blank(content) && content.len() <= indent {
                out.push_str(&line[content.len()..]);
            } else {
                out.push_str(line);
            }
        }
        at = end;
    }
    out
}

/// A literal block whose first line is empty given the indentation indicator the files carry
/// for it, so its leading line break is unmistakably content.
fn indicate_leading_breaks(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    for (at, line) in lines.iter().enumerate() {
        let opens_blank = lines.get(at + 1).is_some_and(|next| blank(next));
        match literal_header(line) {
            Some(header) if header.indicator.is_none() && opens_blank => {
                out.push_str(&line[..=header.token]);
                out.push(LEADING_BREAK_INDICATOR);
                out.push_str(&line[header.token + 1..]);
            }
            _ => out.push_str(line),
        }
    }
    out
}

/// A double-quoted scalar that starts a key or value position (line start, after `- `, or
/// after `: `), closes on the same line and holds no escape but `\"` and `\\`, rewritten
/// single-quoted with its `'` doubled — the style the files carry for a string that needs
/// quoting but no escape. Lines inside block scalars are left alone.
fn single_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut block: Option<usize> = None;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches('\n');
        if let Some(parent) = block {
            if content.trim().is_empty() || indent_of(content) > parent {
                out.push_str(line);
                continue;
            }
            block = None;
        }
        let rewritten = single_quoted_line(content);
        if let Some(header) = literal_header(&rewritten) {
            block = Some(header.parent);
        }
        out.push_str(&rewritten);
        out.push_str(&line[content.len()..]);
    }
    out
}

fn single_quoted_line(content: &str) -> String {
    let bytes = content.as_bytes();
    let mut rewritten = String::with_capacity(content.len());
    let mut at = 0;
    let mut at_token = true;
    while at < bytes.len() {
        let byte = bytes[at];
        if at_token && byte == b' ' {
            rewritten.push(' ');
            at += 1;
        } else if at_token && byte == b'-' && bytes.get(at + 1) == Some(&b' ') {
            rewritten.push_str("- ");
            at += 2;
        } else if at_token && byte == b'"' {
            let close = closing_double_quote(bytes, at);
            match close.and_then(|close| unescaped(&content[at + 1..close])) {
                Some(body) => {
                    rewritten.push('\'');
                    rewritten.push_str(&body.replace('\'', "''"));
                    rewritten.push('\'');
                }
                None => {
                    let end = close.map_or(bytes.len(), |close| close + 1);
                    rewritten.push_str(&content[at..end]);
                }
            }
            at = close.map_or(bytes.len(), |close| close + 1);
            at_token = false;
        } else if !at_token && byte == b':' && bytes.get(at + 1) == Some(&b' ') {
            rewritten.push_str(": ");
            at += 2;
            at_token = true;
        } else if at_token && byte == b'\'' {
            let end = closing_single_quote(bytes, at);
            rewritten.push_str(&content[at..end]);
            at = end;
            at_token = false;
        } else {
            let character = content[at..].chars().next().expect("inside the line");
            rewritten.push(character);
            at += character.len_utf8();
            at_token = false;
        }
    }
    rewritten
}

/// The index of the `"` closing the double-quoted scalar opening at `open`, if the line
/// closes it.
fn closing_double_quote(bytes: &[u8], open: usize) -> Option<usize> {
    let mut at = open + 1;
    while at < bytes.len() && bytes[at] != b'"' {
        at += if bytes[at] == b'\\' { 2 } else { 1 };
    }
    (at < bytes.len()).then_some(at)
}

/// The index after the single-quoted scalar opening at `open`, `''` included in it.
fn closing_single_quote(bytes: &[u8], open: usize) -> usize {
    let mut at = open + 1;
    while at < bytes.len() {
        if bytes[at] == b'\'' {
            if bytes.get(at + 1) == Some(&b'\'') {
                at += 2;
                continue;
            }
            return at + 1;
        }
        at += 1;
    }
    bytes.len()
}

/// A double-quoted scalar's body unescaped, when its only escapes are `\"` and `\\` — the
/// two a single-quoted scalar carries as themselves.
fn unescaped(body: &str) -> Option<String> {
    let mut plain = String::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next() {
                Some(escaped @ ('"' | '\\')) => plain.push(escaped),
                _ => return None,
            }
        } else {
            plain.push(character);
        }
    }
    Some(plain)
}
