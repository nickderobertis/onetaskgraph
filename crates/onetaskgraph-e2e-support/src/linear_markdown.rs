//! What the loopback Linear stores when it is written Markdown: a stand-in for Linear's own
//! whole-document Markdown serializer, so the journeys that drive the store against it meet a
//! destination that rewrites what it is sent as real Linear does.
//!
//! [`stored`] applies two sets of rewrites, outside a fenced block and outside an inline code
//! span, each of which it keeps byte for byte:
//!
//! - every rewrite real Linear was observed to make that the Linear plugin's earlier ruling
//!   declared — `- ` bullets as `* `, a table's delimiter row as `| -- |` and a table ending the
//!   content ending in its last row's line break, a domain-like token or URL autolinked,
//!   brackets and tildes escaped, `_y_` as `*y*`, a list item's next lines run on under it, a
//!   fence and a lone image set apart by blank lines, a run of blank lines kept as one — so a
//!   table ending the content followed by the metadata slot after two blank lines is stored with
//!   one, as spike `spike-linear-stored-content` measured;
//! - and the five that spike measured beyond them: an ordered item on a lazy-continuation line
//!   inside a list item escaped (`3\. `), a blank line set before a list after a heading or a
//!   paragraph line, an inline code span wrapped across two lines joined, `**bold**` spanning a
//!   line break split into one run per line, and an indented code block turned into a fenced
//!   one.
//!
//! Applying it to its own output changes nothing, which is what the spike measured of Linear: a
//! follow-up write of what Linear stored is stored as it was sent. The samples each rewrite is
//! held to are `crates/onetaskgraph-linear-e2e/tests/fixtures/linear_rewrites/`, whose README
//! names the spike report each comes from. The plugin predicts none of this; it is the loopback's
//! alone, so the store's follow-up path runs on every gate at no Linear quota.

/// The top-level domains a bare domain-like token is autolinked under — the last label of its
/// host, compared without regard to case.
///
/// A list rather than every top-level domain, because a list is what can be declared and held:
/// a token under another one is left as written, and if Linear links it after all the stored
/// content differs from what was sent, which the store reports rather than records.
pub const AUTOLINKED_TOP_LEVEL_DOMAINS: &[&str] = &[
    "ai", "app", "co", "com", "dev", "edu", "gov", "info", "io", "live", "me", "net", "org",
];

/// `content` as the loopback Linear stores it: every rewrite of the module documentation applied,
/// outside a fenced block and outside an inline code span, each of which is kept byte for byte.
///
/// Applying it to its own output changes nothing.
#[must_use]
pub fn stored(content: &str) -> String {
    let content = indented_code_fenced(&bold_split(&code_spans_joined(content)));
    let content = content.as_str();
    let mut stored = String::with_capacity(content.len());
    let mut fence: Option<(char, usize)> = None;
    // The column a list item's text starts at, while the lines that follow continue it.
    let mut item: Option<usize> = None;
    // Whether the line before closed a fence or held an image alone, which the next line that
    // is not blank is set apart from by a blank line.
    let mut apart = false;
    let mut after_blank = true;
    // Whether the line before was blank, which a blank line after it joins rather than adds to.
    let mut blank_run = false;
    // Whether the line before was a heading or a paragraph's line, which a list opening on the
    // next line is set apart from by a blank line.
    let mut after_prose = false;
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
            (after_blank, blank_run) = (false, false);
            continue;
        }
        if body.trim().is_empty() {
            if !blank_run {
                stored.push_str(line);
            }
            (item, apart, after_blank, blank_run, after_prose) = (None, false, true, true, false);
            continue;
        }
        blank_run = false;
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
        let listed = bullet.is_some() || ordered(rest).is_some();
        // A list opening straight after a heading or a paragraph's line is set apart from it.
        if listed && continued.is_none() && after_prose && !stored.ends_with("\n\n") {
            stored.push('\n');
        }
        after_prose = continued.is_none()
            && !listed
            && !image
            && delimiter_row(body).is_none()
            && !(rest.starts_with('|') && rest.ends_with('|'))
            && !rest.starts_with("<!--");
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
        // An ordered item's marker on a line that runs on from a list item is escaped, so it
        // stays the item's text rather than opening a list of its own.
        let rest = match (continued, ordered(rest)) {
            (Some(_), Some(digits)) => {
                stored.push_str(&rest[..digits]);
                stored.push_str("\\");
                &rest[digits..]
            }
            _ => rest,
        };
        stored.push_str(&inline(rest));
        stored.push_str(end);
    }
    if fence.is_none() {
        // The content ends at its last line holding anything: every line break and blank line
        // after it goes.
        loop {
            let last = stored.rfind('\n').map_or(0, |at| at + 1);
            if stored.ends_with('\n') {
                stored.pop();
                if stored.ends_with('\r') {
                    stored.pop();
                }
            } else if last > 0 && stored[last..].trim().is_empty() {
                stored.truncate(last);
            } else {
                break;
            }
        }
        // But for a table's last row, which Linear ends with its line break.
        if item.is_none() && ends_in_table_row(&stored) {
            stored.push('\n');
        }
    }
    stored
}

/// Whether the last line of `text` is a table row: opening and closing with a pipe.
#[must_use]
pub fn ends_in_table_row(text: &str) -> bool {
    let last = text.rsplit('\n').next().unwrap_or_default().trim();
    last.len() > 1 && last.starts_with('|') && last.ends_with('|')
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

/// Where an ordered list item's number ends in `text`, a line with its indentation taken off —
/// one to nine digits, then `.` and a space — or `None` when it opens no such item.
fn ordered(text: &str) -> Option<usize> {
    let digits = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    ((1..=9).contains(&digits) && text[digits..].starts_with(". ")).then_some(digits)
}

/// Each line of `content` outside a fence, `f` applied to it with the line after it, and the
/// fences themselves kept byte for byte.
fn outside_fences(content: &str, mut f: impl FnMut(&mut Vec<String>, usize)) -> String {
    let mut lines: Vec<String> = content.split('\n').map(str::to_owned).collect();
    let mut fence: Option<(char, usize)> = None;
    let mut at = 0;
    while at < lines.len() {
        let mark =
            fence_mark(&lines[at]).map(|(mark, run, rest)| (mark, run, rest.trim().is_empty()));
        match (fence, mark) {
            (Some((open, length)), Some((closing, run, true)))
                if closing == open && run >= length =>
            {
                fence = None;
            }
            (Some(_), _) => {}
            (None, Some((mark, run, _))) => fence = Some((mark, run)),
            (None, None) => f(&mut lines, at),
        }
        at += 1;
    }
    lines.join("\n")
}

/// The byte ranges of `line`'s inline code spans, each opened and closed on it.
fn code_spans(line: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(found) = line[at..].find('`') {
        let start = at + found;
        let run = line[start..].len() - line[start..].trim_start_matches('`').len();
        match code_span_end(line, start, run) {
            Some(end) => {
                spans.push(start..end);
                at = end;
            }
            None => at = start + run,
        }
    }
    spans
}

/// Whether `line` opens a code span it does not close: a backtick run nothing after it on the
/// line closes — the run's length, or `None`.
fn unclosed_span(line: &str) -> Option<usize> {
    let spans = code_spans(line);
    let mut at = 0;
    while let Some(found) = line[at..].find('`') {
        let start = at + found;
        let run = line[start..].len() - line[start..].trim_start_matches('`').len();
        if let Some(span) = spans.iter().find(|span| span.start == start) {
            at = span.end;
            continue;
        }
        return Some(run);
    }
    None
}

/// `content` with each inline code span wrapped across two lines joined onto the first, as
/// Linear stores a span it reads as one: the line break becomes a space.
fn code_spans_joined(content: &str) -> String {
    outside_fences(content, |lines, at| {
        while at + 1 < lines.len() {
            let Some(run) = unclosed_span(&lines[at]) else {
                return;
            };
            let next = lines[at + 1].trim_start().to_owned();
            let closes = next.contains(&"`".repeat(run));
            if next.is_empty() || fence_mark(&next).is_some() || !closes {
                return;
            }
            lines[at].push(' ');
            lines[at].push_str(&next);
            lines.remove(at + 1);
        }
    })
}

/// How many `**` runs `line` holds outside its code spans.
fn bold_runs(line: &str) -> usize {
    let spans = code_spans(line);
    let mut count = 0;
    let mut at = 0;
    while let Some(found) = line[at..].find("**") {
        let start = at + found;
        if !spans.iter().any(|span| span.contains(&start)) {
            count += 1;
        }
        at = start + 2;
    }
    count
}

/// `content` with each `**bold**` run that spans a line break closed at the end of its first
/// line and opened again at the start of the next, one run per line, as Linear stores it.
fn bold_split(content: &str) -> String {
    outside_fences(content, |lines, at| {
        if at + 1 >= lines.len() || bold_runs(&lines[at]) % 2 == 0 {
            return;
        }
        let next = &lines[at + 1];
        if next.trim().is_empty() || bold_runs(next) % 2 == 0 {
            return;
        }
        let indent = next.len() - next.trim_start().len();
        let reopened = format!("{}**{}", &next[..indent], &next[indent..]);
        lines[at].push_str("**");
        lines[at + 1] = reopened;
    })
}

/// `content` with each indented code block — lines indented four spaces or more after a blank
/// line, below a line that is no list item's — turned into a fenced block of the same lines,
/// unindented, as Linear stores one.
fn indented_code_fenced(content: &str) -> String {
    let indented = |line: &str| line.starts_with("    ") && !line.trim().is_empty();
    let mut lines: Vec<String> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut source = content.split('\n').peekable();
    // The last line holding anything, which says whether an indented line is code or a list
    // item's own continuation.
    let mut last_text: Option<String> = None;
    while let Some(line) = source.next() {
        if let Some((open, length)) = fence {
            if fence_mark(line).is_some_and(|(mark, run, rest)| {
                mark == open && run >= length && rest.trim().is_empty()
            }) {
                fence = None;
            }
            lines.push(line.to_owned());
            continue;
        }
        if let Some((mark, run, _)) = fence_mark(line) {
            fence = Some((mark, run));
            lines.push(line.to_owned());
            last_text = Some(line.to_owned());
            continue;
        }
        let after_blank = lines.last().is_none_or(|before| before.trim().is_empty());
        let under_item = last_text.as_deref().is_some_and(|text| {
            let rest = text.trim_start();
            text.starts_with(' ')
                || rest.starts_with("- ")
                || rest.starts_with("* ")
                || ordered(rest).is_some()
        });
        if indented(line) && after_blank && !under_item {
            lines.push("```".to_owned());
            lines.push(line[4..].to_owned());
            while let Some(next) = source.peek() {
                if indented(next) {
                    lines.push(next[4..].to_owned());
                    source.next();
                } else {
                    break;
                }
            }
            lines.push("```".to_owned());
            last_text = Some("```".to_owned());
            continue;
        }
        if !line.trim().is_empty() {
            last_text = Some(line.to_owned());
        }
        lines.push(line.to_owned());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::stored;

    /// Each rewrite, as the spike's own probe of it was stored: the line the spike report names
    /// as where the two first part, here as the loopback stores it.
    #[test]
    fn each_measured_rewrite_is_stored_and_storing_it_again_changes_nothing() {
        for (sent, wanted) in [
            (
                "| A | B |\n| --- | --- |\n| one | two |",
                "| A | B |\n| -- | -- |\n| one | two |\n",
            ),
            (
                "- first\n  - nested\n- third",
                "* first\n  * nested\n* third",
            ),
            (
                "Before.\n\n    one\n    two\n\nAfter.",
                "Before.\n\n```\none\ntwo\n```\n\nAfter.",
            ),
            (
                "See `a.rs` and a `span that\nwraps across` in prose.",
                "See `a.rs` and a `span that wraps across` in prose.",
            ),
            (
                "This has **bold that spans\na line break** in it.",
                "This has **bold that spans**\n**a line break** in it.",
            ),
            (
                "- a list item whose text wraps onto\n3. a lazy line\n- next",
                "* a list item whose text wraps onto\n  3\\. a lazy line\n* next",
            ),
            (
                "## A heading\n- first\n- second",
                "## A heading\n\n* first\n* second",
            ),
            ("A paragraph line.\n- first", "A paragraph line.\n\n* first"),
            (
                "![a](https://uploads.linear.app/a)\n![b](https://uploads.linear.app/b)",
                "![a](https://uploads.linear.app/a)\n\n![b](https://uploads.linear.app/b)",
            ),
            ("# Top\n\nText.", "# Top\n\nText."),
            ("1. one\n2. two\n3. three", "1. one\n2. two\n3. three"),
            (
                "```rust\n- [x] _y_ ~\n    kept\n```",
                "```rust\n- [x] _y_ ~\n    kept\n```",
            ),
            // A table ending the content and the metadata slot two blank lines after its row's
            // line break: one blank line between them, as the spike measured.
            (
                "| a |\n| -- |\n\n\n<!-- onetaskgraph.metadata `{}` -->",
                "| a |\n| -- |\n\n<!-- onetaskgraph.metadata `{}` -->",
            ),
        ] {
            assert_eq!(stored(sent), wanted, "{sent:?}");
            assert_eq!(stored(wanted), wanted, "storing {wanted:?} again");
        }
    }

    #[test]
    fn a_slot_a_fence_and_a_code_span_are_kept_byte_for_byte() {
        for kept in [
            "<!-- onetaskgraph.metadata `{\"k\":\"[x] ~ _y_ ** \\u0060\"}` -->",
            "`- [x] _y_ example.com ~ **`",
            "```\n- a **b\n    c\n```",
        ] {
            assert_eq!(stored(kept), kept, "{kept:?}");
        }
    }
}
