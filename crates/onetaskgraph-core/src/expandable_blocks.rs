//! How an expandable (collapsed-by-default) block is spelled on each kind of destination,
//! decided once.
//!
//! A record held in a folder of Markdown carries an expandable block in **the authored form**:
//!
//! ```text
//! <details>
//! <summary>SUMMARY</summary>
//!
//! BODY
//!
//! </details>
//! ```
//!
//! outside any fenced code block and not indented, `<details>` with no attribute, SUMMARY on one
//! line with `<code>`, `<strong>` and the entities `&amp;`, `&lt;` and `&gt;` as its only markup,
//! one blank line on either side of a BODY of one or more lines. A BODY may hold further blocks in
//! the authored form, each meeting every one of those rules, to any depth. The README's
//! "Expandable blocks" states it for a consumer, and [`blocks`] is what recognises it.
//!
//! Each destination kind has exactly one [`ExpandableForm`] entry, and every path that writes or
//! reads a block for that kind reads it: [`ExpandableForm::written`] writes the destination's form
//! from the authored one, [`ExpandableForm::read`] reads it back, and [`ExpandableForm::markup`]
//! hands a plugin both as the [`BodyMarkup`] it applies to every body it sends and receives, so
//! no plugin holds a copy of any form. A folder of Markdown and a GitHub issue keep the authored
//! form byte for byte; Linear keeps a collapsible section, `+++ <summary>` … `+++`, and a block
//! nested in another as a section inside that block's section, every section closed by a bare
//! `+++` line.
//!
//! A shape that is not the authored form — an attribute on `<details>`, a missing blank line, an
//! indented or fenced block, a summary holding another tag — is never translated, and a block
//! holding such a shape anywhere inside it is carried whole as it stands. Neither is a block whose
//! Linear form would not read back as itself — one written with carriage returns among them: see
//! [`ExpandableForm::written`].

use std::sync::Arc;

use onetaskgraph_plugin_api::{BodyMarkup, SharedBodyMarkup};

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] The task contract this module implements requires each destination kind's expandable form to be one entry of `ExpandableForm` in `onetaskgraph-core`, the one place that kind's form is written and read back; no plugin may depend on the engine (AGENTS.md), so a plugin cannot own a form the engine reads, and Linear's form — with what it remembers of blocks it read back as literal text, which is part of reading that form back — reaches the Linear plugin only as the `BodyMarkup` this entry hands it, as GitHub's anchors do in `section_links`.
/// One destination kind's expandable form: the one entry each kind has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpandableForm {
    /// A folder of Markdown: the authored form, byte for byte.
    LocalMd,
    /// A GitHub Projects board's issues: the authored form, byte for byte. The anchor line the
    /// GitHub source writes before each heading (see `section_links`) reaches a heading inside a
    /// block's body as it reaches any other.
    GithubProjects,
    /// Linear issues, projects, documents and comments: a collapsible section, `+++ LSUMMARY`
    /// alone on its line, a blank line, the body, a blank line, and `+++` alone on its line. A
    /// block nested in the body is a section of its own inside it, written the same way — never
    /// with `>>>`, whose first bare line Linear reads as closing every section open.
    /// LSUMMARY is SUMMARY with `<code>TEXT</code>` as `` `TEXT` ``, `<strong>TEXT</strong>` as
    /// `**TEXT**`, and each entity as the character it stands for, which is how Linear stores it.
    Linear,
}

impl ExpandableForm {
    /// The entry for a source of plugin kind `kind`, or `None` for a kind with no expandable
    /// form — a plugin served over stdio among them — which carries the authored form unchanged.
    #[must_use]
    pub fn for_kind(kind: &str) -> Option<Self> {
        match kind {
            "local-md" => Some(Self::LocalMd),
            "github-projects" => Some(Self::GithubProjects),
            "linear" => Some(Self::Linear),
            _ => None,
        }
    }

    /// `authored` with every authored block written in this kind's form.
    ///
    /// For Linear a block is written as a collapsible section only when that section reads back,
    /// by [`Self::read`], as the very block it was written from. A summary whose plain text holds
    /// a backtick or a `*`, which Linear would read as a code or a strong mark, or a raw `&`, `<`
    /// or `>`, which Linear's stored text cannot tell from an escaped one, fails that test, and
    /// its block is carried as it stands. A block nested in another is written inside its outer
    /// section by the same test, so an inner block failing it is carried as it stands inside a
    /// section that reads back as the outer block.
    #[must_use]
    pub fn written(self, authored: &str) -> String {
        match self {
            Self::LocalMd | Self::GithubProjects => authored.to_owned(),
            Self::Linear => linear_written(authored, &std::collections::HashSet::new(), true),
        }
    }

    /// `stored`, as this kind's backend holds it, with every block in this kind's form read back
    /// into the authored form: exactly what [`Self::written`] translates, and a section a person
    /// made in the tracker whose authored block would be written as that same section. A section
    /// inside a section is read back as a block inside the outer one's block.
    #[must_use]
    pub fn read(self, stored: &str) -> String {
        match self {
            Self::LocalMd | Self::GithubProjects => stored.to_owned(),
            Self::Linear => linear_read(stored),
        }
    }

    /// The markup this kind's bodies carry for their blocks to be expandable there, when the kind
    /// spells a block otherwise than the authored form: only Linear does.
    #[must_use]
    pub fn markup(self) -> Option<SharedBodyMarkup> {
        match self {
            Self::Linear => Some(Arc::new(LinearSections::default())),
            Self::LocalMd | Self::GithubProjects => None,
        }
    }
}

// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

/// One expandable block in the authored form: made only by [`authored_parts`], which holds it to
/// the form, and by [`authored_block`], which reads it out of a Linear section. Its body holds any
/// block nested in it in the authored form.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Block {
    /// SUMMARY, as written between `<summary>` and `</summary>`.
    summary: String,
    /// BODY: every line between the blank line after the summary and the blank line before
    /// `</details>`, joined by line breaks.
    body: String,
}

impl Block {
    /// The block in the authored form, its last line ended by `ending`.
    fn render(&self, ending: &str) -> String {
        format!(
            "<details>\n<summary>{}</summary>\n\n{}\n\n</details>{ending}",
            self.summary, self.body
        )
    }
}

/// How many outermost blocks of `content` are in the authored form, a block nested in one counted
/// with it. A block-like shape that is not the form — see the module documentation — is not one,
/// nor is anything inside it, nor a block holding it.
#[must_use]
pub fn blocks(content: &str) -> usize {
    authored_regions(content)
        .iter()
        .filter(|region| matches!(region.shape, Shape::Block(_)))
        .count()
}

/// The lines from one opening line to its closing one, and what they hold when they are
/// exactly one block of a form.
struct Region {
    /// The first and last line, by index.
    lines: (usize, usize),
    /// The last line's own line break.
    ending: &'static str,
    /// What they are.
    shape: Shape,
}

/// What the lines of one [`Region`] are.
enum Shape {
    /// Exactly one block in the authored form.
    Block(Block),
    /// Exactly one section in Linear's form.
    Section(Section),
    /// Neither: a shape that is not a form, carried as it stands.
    Other,
}

/// One line of a body: its text without its line break, and the break.
struct Line<'a> {
    text: &'a str,
    ending: &'static str,
}

/// `content` cut into lines, each with its own line break.
fn lines(content: &str) -> Vec<Line<'_>> {
    content
        .split_inclusive('\n')
        .map(|line| {
            let text = line.trim_end_matches(['\n', '\r']);
            let ending = match &line[text.len()..] {
                "\r\n" => "\r\n",
                "\n" => "\n",
                "\r" => "\r",
                _ => "",
            };
            Line { text, ending }
        })
        .collect()
}

/// A fence's marker and width, when `line` — indented at most three spaces — opens or closes
/// one.
fn fence_of(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let width = trimmed.chars().take_while(|c| *c == marker).count();
    (width >= 3).then_some((marker, width))
}

/// Which lines of `lines` sit inside a fenced code block, the fence lines themselves included.
fn fenced(lines: &[Line<'_>]) -> Vec<bool> {
    let mut fence: Option<(char, usize)> = None;
    lines
        .iter()
        .map(|line| match (fence, fence_of(line.text)) {
            (None, Some(opened)) => {
                fence = Some(opened);
                true
            }
            (Some((open, wide)), Some((marker, width))) if open == marker && width >= wide => {
                fence = None;
                true
            }
            (Some(_), _) => true,
            (None, None) => false,
        })
        .collect()
}

/// Every region of `content` an unindented `<details` line outside fenced code opens, through
/// the `</details>` line that closes it, a `<details` line between them nesting one level deeper.
/// A region that is exactly one authored block, every region of its body one too, carries it; any
/// other carries none, and nothing inside it is a block either.
fn authored_regions(content: &str) -> Vec<Region> {
    let lines = lines(content);
    let fenced = fenced(&lines);
    let opens = |index: usize| {
        !fenced[index]
            && (lines[index].text == "<details>" || lines[index].text.starts_with("<details "))
    };
    let closes = |index: usize| !fenced[index] && lines[index].text == "</details>";
    let mut regions = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !opens(index) {
            index += 1;
            continue;
        }
        let mut depth = 0_usize;
        let mut close = None;
        for at in index..lines.len() {
            if opens(at) {
                depth += 1;
            } else if closes(at) {
                depth -= 1;
                if depth == 0 {
                    close = Some(at);
                    break;
                }
            }
        }
        let Some(close) = close else {
            index += 1;
            continue;
        };
        // A block written with carriage returns would not read back from Linear byte for byte —
        // Linear keeps none — so it is no block to translate.
        let returned = lines[index..=close]
            .iter()
            .any(|line| line.ending != "\n" && !line.ending.is_empty());
        let block = (!returned && lines[index].text == "<details>")
            .then(|| authored_parts(&lines[index..=close]))
            .flatten()
            .filter(|block| summary_markup(&block.summary).is_some())
            .filter(|block| {
                authored_regions(&block.body)
                    .iter()
                    .all(|inner| matches!(inner.shape, Shape::Block(_)))
            });
        regions.push(Region {
            lines: (index, close),
            ending: lines[close].ending,
            shape: block.map_or(Shape::Other, Shape::Block),
        });
        index = close + 1;
    }
    regions
}

/// The block `lines` — `<details>` through `</details>` — are, when the lines between hold the
/// summary line, a blank line, a body whose first and last lines hold something, and a blank line.
fn authored_parts(lines: &[Line<'_>]) -> Option<Block> {
    let [_, summary, blank, body @ .., after, _] = lines else {
        return None;
    };
    let summary = summary
        .text
        .strip_prefix("<summary>")?
        .strip_suffix("</summary>")?;
    let (first, last) = (body.first()?, body.last()?);
    (blank.text.is_empty()
        && after.text.is_empty()
        && !first.text.trim().is_empty()
        && !last.text.trim().is_empty()
        && !summary.contains("</summary>"))
    .then(|| Block {
        summary: summary.to_owned(),
        body: joined(body),
    })
}

/// `lines` joined by their own line breaks, the last one's left off.
fn joined(lines: &[Line<'_>]) -> String {
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate() {
        text.push_str(line.text);
        if index + 1 < lines.len() {
            text.push_str(if line.ending.is_empty() {
                "\n"
            } else {
                line.ending
            });
        }
    }
    text
}

/// `content` with each region `replace` answers for put in that answer's place, every other line
/// as it was.
fn rewritten(
    content: &str,
    regions: &[Region],
    replace: impl Fn(&Region) -> Option<String>,
) -> String {
    let lines = lines(content);
    let mut text = String::with_capacity(content.len());
    let mut regions = regions.iter().peekable();
    let mut index = 0;
    while index < lines.len() {
        if let Some(region) = regions.next_if(|region| region.lines.0 == index)
            && let Some(replaced) = replace(region)
        {
            text.push_str(&replaced);
            index = region.lines.1 + 1;
            continue;
        }
        text.push_str(lines[index].text);
        text.push_str(lines[index].ending);
        index += 1;
    }
    text
}

/// The text of `region` of `lines`, its last line's break left off.
fn region_text(lines: &[Line<'_>], region: &Region) -> String {
    joined(&lines[region.lines.0..=region.lines.1])
}

/// One run of a summary's markup.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Run {
    /// Text, its entities decoded.
    Text(String),
    /// `<code>TEXT</code>`, TEXT decoded.
    Code(String),
    /// `<strong>TEXT</strong>`, TEXT decoded.
    Strong(String),
}

/// SUMMARY's runs, when it is the authored form's summary: non-empty, with `<code>` and
/// `<strong>` — neither nested in the other, their TEXT holding no tag, no backtick and no `*` —
/// as its only tags, and `&` only in `&amp;`, `&lt;` and `&gt;`.
fn summary_markup(summary: &str) -> Option<Vec<Run>> {
    if summary.trim().is_empty() {
        return None;
    }
    let mut runs = Vec::new();
    let mut rest = summary;
    let mut text = String::new();
    while let Some(character) = rest.chars().next() {
        let tagged = [("<code>", "</code>"), ("<strong>", "</strong>")]
            .into_iter()
            .find(|(open, _)| rest.starts_with(open));
        if let Some((open, close)) = tagged {
            let inner_end = rest[open.len()..].find(close)?;
            let inner = &rest[open.len()..open.len() + inner_end];
            if inner.is_empty() || inner.contains(['<', '>', '`', '*']) {
                return None;
            }
            let inner = decoded(inner)?;
            if !text.is_empty() {
                runs.push(Run::Text(std::mem::take(&mut text)));
            }
            runs.push(if open == "<code>" {
                Run::Code(inner)
            } else {
                Run::Strong(inner)
            });
            rest = &rest[open.len() + inner_end + close.len()..];
            continue;
        }
        if matches!(character, '<' | '>') {
            return None;
        }
        if character == '&' {
            let (entity, stands_for) = ENTITIES
                .into_iter()
                .find(|(entity, _)| rest.starts_with(entity))?;
            text.push(stands_for);
            rest = &rest[entity.len()..];
            continue;
        }
        text.push(character);
        rest = &rest[character.len_utf8()..];
    }
    if !text.is_empty() {
        runs.push(Run::Text(text));
    }
    Some(runs)
}

/// The three entities a summary writes, and the character each stands for.
const ENTITIES: [(&str, char); 3] = [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>')];

/// `text` with each of [`ENTITIES`] decoded, when every `&` in it begins one.
fn decoded(text: &str) -> Option<String> {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(character) = rest.chars().next() {
        if character == '&' {
            let (entity, stands_for) = ENTITIES
                .into_iter()
                .find(|(entity, _)| rest.starts_with(entity))?;
            plain.push(stands_for);
            rest = &rest[entity.len()..];
        } else {
            plain.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    Some(plain)
}

/// `text` with `&`, `<` and `>` escaped as the authored summary writes them.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] This region is Linear's form alone — its section, how it is parsed back, how it is written, and the `BodyMarkup` that hands it to the Linear plugin. The task contract this module implements requires each destination kind's expandable form to be one entry of `ExpandableForm` in `onetaskgraph-core`, the one place that kind's form is written and read back; no plugin may depend on the engine (AGENTS.md), so a plugin cannot own a form the engine reads, and Linear's form — with what it remembers of blocks it read back as literal text, which is part of reading that form back — reaches the Linear plugin only as the `BodyMarkup` this entry hands it, as GitHub's anchors do in `section_links`.
/// A Linear collapsible section: its summary as Linear stores it, and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    summary: String,
    body: String,
}

impl Section {
    /// The section as Linear's Markdown spells it, its last line ended by `ending`.
    fn render(&self, ending: &str) -> String {
        format!("+++ {}\n\n{}\n\n+++{ending}", self.summary, self.body)
    }
}

/// Every region of `stored` a `+++ <summary>` line outside fenced code opens, through the `+++`
/// line that closes it, an inner `+++ ` line nesting one level deeper. A region that is exactly
/// one section, its first and last body lines holding something, carries it.
fn linear_regions(stored: &str) -> Vec<Region> {
    let lines = lines(stored);
    let fenced = fenced(&lines);
    let opens = |index: usize| !fenced[index] && lines[index].text.starts_with("+++ ");
    let closes = |index: usize| !fenced[index] && lines[index].text == "+++";
    let mut regions = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !opens(index) {
            index += 1;
            continue;
        }
        let mut depth = 0_usize;
        let mut close = None;
        for at in index..lines.len() {
            if opens(at) {
                depth += 1;
            } else if closes(at) {
                depth -= 1;
                if depth == 0 {
                    close = Some(at);
                    break;
                }
            }
        }
        let Some(close) = close else {
            index += 1;
            continue;
        };
        let section = match &lines[index..=close] {
            [header, blank, body @ .., after, _]
                if blank.text.is_empty()
                    && after.text.is_empty()
                    && body
                        .first()
                        .is_some_and(|line| !line.text.trim().is_empty())
                    && body.last().is_some_and(|line| !line.text.trim().is_empty()) =>
            {
                let summary = &header.text["+++ ".len()..];
                (!summary.trim().is_empty()).then(|| Section {
                    summary: summary.to_owned(),
                    body: joined(body),
                })
            }
            _ => None,
        };
        regions.push(Region {
            lines: (index, close),
            ending: lines[close].ending,
            shape: section.map_or(Shape::Other, Shape::Section),
        });
        index = close + 1;
    }
    regions
}

/// `block` as a Linear section, when its summary is the authored form's and its Linear summary
/// can be spelled — its text holding no backtick and no `*`, which Linear reads as marks, and
/// no space at either end.
fn linear_section(block: &Block) -> Option<Section> {
    // Linear keeps no space at either end of a section's summary.
    if block.summary.trim() != block.summary {
        return None;
    }
    let mut summary = String::new();
    for run in summary_markup(&block.summary)? {
        match run {
            Run::Text(text) if text.contains(['`', '*']) => return None,
            Run::Text(text) => summary.push_str(&text),
            Run::Code(text) => {
                summary.push('`');
                summary.push_str(&text);
                summary.push('`');
            }
            Run::Strong(text) => {
                summary.push_str("**");
                summary.push_str(&text);
                summary.push_str("**");
            }
        }
    }
    Some(Section {
        summary,
        body: block.body.clone(),
    })
}

/// The authored block a Linear section reads back as: its summary's backtick spans as `<code>`,
/// its `**` spans as `<strong>`, and every `&`, `<` and `>` of its text escaped again, because
/// Linear stores entities decoded.
fn authored_block(section: &Section) -> Option<Block> {
    let mut summary = String::new();
    let mut rest = section.summary.as_str();
    while let Some(character) = rest.chars().next() {
        let marked = [("`", "<code>", "</code>"), ("**", "<strong>", "</strong>")]
            .into_iter()
            .find_map(|(mark, open, close)| {
                let inner = rest.strip_prefix(mark)?;
                let end = inner.find(mark)?;
                (end > 0).then(|| (mark.len() * 2 + end, open, &inner[..end], close))
            });
        if let Some((taken, open, inner, close)) = marked {
            summary.push_str(open);
            summary.push_str(&escaped(inner));
            summary.push_str(close);
            rest = &rest[taken..];
            continue;
        }
        summary.push_str(&escaped(&character.to_string()));
        rest = &rest[character.len_utf8()..];
    }
    let block = Block {
        summary,
        body: section.body.clone(),
    };
    (summary_markup(&block.summary).is_some()).then_some(block)
}

/// `authored` with every authored block written as a Linear section, each block nested in one
/// as a section inside its section — but for a block whose whole text is in `literal`, which is
/// carried as it stands.
///
/// `checked` holds each section to reading back, by [`linear_read`], as the very block it was
/// written from, and carries a block failing that as it stands. [`linear_read`] asks for none:
/// it holds a section to being what its block is written as, which a check of its own would
/// make each ask the other without end.
fn linear_written(
    authored: &str,
    literal: &std::collections::HashSet<String>,
    checked: bool,
) -> String {
    let lines = lines(authored);
    rewritten(authored, &authored_regions(authored), |region| {
        let Shape::Block(block) = &region.shape else {
            return None;
        };
        let text = region_text(&lines, region);
        if literal.contains(&text) {
            return None;
        }
        let mut section = linear_section(block)?;
        section.body = linear_written(&block.body, literal, checked);
        (!checked || linear_read(&section.render("")) == text)
            .then(|| section.render(region.ending))
    })
}

/// `stored` with every Linear section read back as the authored block it is written from, each
/// section inside one as a block inside its block. A section is read back only when that block,
/// written again, is the section byte for byte — each block of its text Linear held as literal
/// text carried as it stands, as a write-back carries it — and is otherwise left as it is stored.
fn linear_read(stored: &str) -> String {
    let lines = lines(stored);
    rewritten(stored, &linear_regions(stored), |region| {
        let Shape::Section(section) = &region.shape else {
            return None;
        };
        let mut block = authored_block(section)?;
        block.body = linear_read(&section.body);
        let text = region_text(&lines, region);
        let literal = block_texts(&text).into_iter().collect();
        (linear_written(&block.render(""), &literal, false) == text)
            .then(|| block.render(region.ending))
    })
}

tokio::task_local! {
    /// Set while the engine writes content a caller authored: see [`authored`].
    static AUTHORED: ();
}

/// Run `call` — one write the engine sends a source — as a write of content a caller authored:
/// a create, a replace, a render, `task content set`, `task update`, a comment added or edited,
/// and a copy's write of its source's content. Every block in the authored form in what it sends
/// is written in the destination's expandable form. A write sent outside it is the engine writing
/// back what it read, as a copy's follow-up does.
pub(crate) async fn authored<F: Future>(call: F) -> F::Output {
    AUTHORED.scope((), call).await
}

/// Linear's collapsible sections, as the Linear source applies them to every body it sends and
/// receives.
///
/// What Linear stores of a body it was sent can hold a block in the authored form as literal text:
/// a shape the authored form does not recognise — `<details>` straight followed by a list, say —
/// that Linear's own rewrite sets apart with the blank line the form wants, so it reads back as an
/// authored block though Linear shows it as paragraphs. Written back as it reads, as a copy's
/// follow-up writes what the destination stored, it would be translated into a section the author
/// never wrote. So the markup remembers the latest body read through it, and how that read found
/// each of its blocks held; the Linear source reads each item's own last stored body through it
/// again immediately before writing that item, or reads nothing — an empty body — for an item it
/// has not read, so the latest read is always the written item's own.
///
/// A write of content a caller authored ([`authored`]) is written with every block as a section —
/// one whose text Linear holds as literal text included — unless the body is, byte for byte, the
/// latest body read: that content set back as it reads. Any other write, and that one, is the
/// engine writing back what it read: each block the latest read found held as literal text is
/// carried as it stands, and every other block is written as a section.
#[derive(Default)]
struct LinearSections {
    latest: std::sync::Mutex<LatestRead>,
}

/// The latest body read through [`LinearSections`].
#[derive(Default)]
struct LatestRead {
    /// The body, in the authored form, as the read gave it back.
    body: String,
    /// The whole text of each of its blocks Linear held as literal text.
    literal: std::collections::HashSet<String>,
}

/// The whole text of every block of `content` in the authored form, in order.
fn block_texts(content: &str) -> Vec<String> {
    let lines = lines(content);
    authored_regions(content)
        .iter()
        .filter(|region| matches!(region.shape, Shape::Block(_)))
        .map(|region| region_text(&lines, region))
        .collect()
}

impl BodyMarkup for LinearSections {
    fn written(&self, content: &str) -> String {
        let latest = self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if AUTHORED.try_with(|()| ()).is_ok() && latest.body != content {
            return linear_written(content, &std::collections::HashSet::new(), true);
        }
        linear_written(content, &latest.literal, true)
    }

    fn read(&self, content: &str) -> String {
        let read = ExpandableForm::Linear.read(content);
        // A block Linear held as literal text reads back as it is stored, so it is one of the
        // stored body's blocks still; one read out of a section is not.
        let literal = block_texts(content).into_iter().collect();
        *self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = LatestRead {
            body: read.clone(),
            literal,
        };
        read
    }
}
// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]

#[cfg(test)]
mod tests {
    use super::{ExpandableForm, blocks};

    const AUTHORED: &str = "Intro.\n\n<details>\n<summary>Run <code>copy --dry-run</code> &amp; \
                            <strong>read</strong> &lt;it&gt;</summary>\n\n- one\n- two\n\n\
                            </details>\n\nAfter.\n";
    const LINEAR: &str =
        "Intro.\n\n+++ Run `copy --dry-run` & **read** <it>\n\n- one\n- two\n\n+++\n\nAfter.\n";

    #[test]
    fn the_authored_form_is_written_as_a_linear_section_and_read_back_byte_for_byte() {
        assert_eq!(ExpandableForm::Linear.written(AUTHORED), LINEAR);
        assert_eq!(ExpandableForm::Linear.read(LINEAR), AUTHORED);
        for kind in [ExpandableForm::LocalMd, ExpandableForm::GithubProjects] {
            assert_eq!(kind.written(AUTHORED), AUTHORED);
            assert_eq!(kind.read(LINEAR), LINEAR);
        }
        assert_eq!(blocks(AUTHORED), 1);
    }

    #[test]
    fn a_shape_that_is_not_the_authored_form_is_carried_as_it_stands() {
        for shape in [
            "<details open>\n<summary>S</summary>\n\nBody.\n\n</details>\n",
            "<details>\n<summary>S</summary>\nBody.\n\n</details>\n",
            "```\n<details>\n<summary>S</summary>\n\nBody.\n\n</details>\n```\n",
            "  <details>\n  <summary>S</summary>\n\n  Body.\n\n  </details>\n",
            // A nested block breaking a rule carries its outermost block as it stands.
            "<details>\n<summary>S</summary>\n\nOuter.\n\n<details>\n<summary>I</summary>\n\
             Inner.\n\n</details>\n\n</details>\n",
            "<details>\n<summary>S</summary>\n\nOuter.\n\n<details>\n<summary>I <em>x</em>\
             </summary>\n\nInner.\n\n</details>\n\n</details>\n",
            "<details>\n<summary>S</summary>\n\n<details open>\n<summary>I</summary>\n\n\
             Inner.\n\n</details>\n\n</details>\n",
            "<details>\n<summary>S</summary>\n\nOuter.\n\n<details>\n<summary>I</summary>\n\n\
             <details>\n<summary>J</summary>\nDeepest.\n\n</details>\n\n</details>\n\n\
             </details>\n",
            "<details>\n<summary>S <em>x</em></summary>\n\nBody.\n\n</details>\n",
            "<details>\n<summary>A `tick`</summary>\n\nBody.\n\n</details>\n",
            "<details>\n<summary>A * star</summary>\n\nBody.\n\n</details>\n",
            "<details>\n<summary>Q & A</summary>\n\nBody.\n\n</details>\n",
            "<details>\n<summary> S</summary>\n\nBody.\n\n</details>\n",
            "<details>\r\n<summary>S</summary>\r\n\r\nBody.\r\n\r\n</details>\r\n",
        ] {
            assert_eq!(ExpandableForm::Linear.written(shape), shape, "{shape}");
        }
    }

    #[test]
    fn a_section_made_in_linear_reads_back_as_an_authored_block_and_its_end_is_kept() {
        assert_eq!(
            ExpandableForm::Linear.read("+++ Made in Linear\n\nBody.\n\n+++"),
            "<details>\n<summary>Made in Linear</summary>\n\nBody.\n\n</details>"
        );
        // A section made inside another in Linear reads back as a block inside its block.
        let nested = "+++ Outer\n\nOuter.\n\n+++ Inner\n\nInner.\n\n+++\n\n+++\n";
        let read = ExpandableForm::Linear.read(nested);
        assert_eq!(
            read,
            "<details>\n<summary>Outer</summary>\n\nOuter.\n\n<details>\n<summary>Inner\
             </summary>\n\nInner.\n\n</details>\n\n</details>\n"
        );
        assert_eq!(ExpandableForm::Linear.written(&read), nested);
        // An inner section that is no block — its summary empty — stays as it is stored inside
        // the block the outer one reads back as.
        let unread = "+++ Outer\n\nOuter.\n\n+++ \n\nInner.\n\n+++\n\n+++\n";
        let read = ExpandableForm::Linear.read(unread);
        assert_eq!(
            read,
            "<details>\n<summary>Outer</summary>\n\nOuter.\n\n+++ \n\nInner.\n\n+++\n\n\
             </details>\n"
        );
        // An empty summary, as Linear writes after a stray `>>>`, is no block.
        let empty = "+++ \n\nBody.\n\n+++";
        assert_eq!(ExpandableForm::Linear.read(empty), empty);
    }

    /// The engine hands a source one markup, so no kind may need a section form's and an
    /// expandable form's both.
    #[test]
    fn no_kind_carries_both_a_section_markup_and_an_expandable_one() {
        for kind in [
            "local-md",
            "github-projects",
            "linear",
            "in-memory",
            "stdio",
        ] {
            let section = crate::section_links::SectionForm::for_kind(kind)
                .and_then(crate::section_links::SectionForm::markup);
            let expandable = ExpandableForm::for_kind(kind).and_then(ExpandableForm::markup);
            assert!(section.is_none() || expandable.is_none(), "{kind}");
        }
    }

    /// A block Linear holds as literal text reads back as it is stored, and is written back so,
    /// though its text is in the authored form: Linear rewrote a shape that was not.
    #[test]
    fn a_block_linear_holds_as_literal_text_is_written_back_as_it_was_stored() {
        let markup = ExpandableForm::Linear.markup().expect("Linear's markup");
        let literal =
            "Before.\n\n<details>\n<summary>Held as text</summary>\n\n* a list\n\n</details>";
        let read = markup.read(literal);
        assert_eq!(read, literal);
        assert_eq!(
            markup.written(&format!("{read}\n\nA slot.")),
            format!("{literal}\n\nA slot.")
        );
        // Another block, never read as text, is written as a section still.
        assert_eq!(
            markup.written("<details>\n<summary>New</summary>\n\nBody.\n\n</details>"),
            "+++ New\n\nBody.\n\n+++"
        );
    }

    const FENCED: &str = "<details>\n<summary>Reproduction</summary>\n\nRun:\n\n```\nsome command\n```\n\n</details>";
    const FENCED_SECTION: &str = "+++ Reproduction\n\nRun:\n\n```\nsome command\n```\n\n+++";
    const LISTED: &str = "<details>\n<summary>Points</summary>\n\n- a point\n\n</details>";
    const LISTED_SECTION: &str = "+++ Points\n\n- a point\n\n+++";

    /// `write` as a write of content a caller authored, as [`super::authored`] marks one.
    fn authored<T>(write: impl FnOnce() -> T) -> T {
        super::AUTHORED.sync_scope((), write)
    }

    /// A markup that has read an item holding a list block and the fenced block as literal text,
    /// as Linear stores them sent as HTML, and that body.
    fn after_reading_them_as_text() -> (onetaskgraph_plugin_api::SharedBodyMarkup, String) {
        let markup = ExpandableForm::Linear.markup().expect("Linear's markup");
        let stored = format!("Held.\n\n{LISTED}\n\n{FENCED}").replace("- a", "* a");
        assert_eq!(
            markup.read(&stored),
            stored,
            "held as literal text reads as stored"
        );
        (markup, stored)
    }

    /// The caller's own `body`, written after that read, lands every block as a section and reads
    /// back as itself.
    fn every_block_is_a_section(body: &str) {
        let (markup, _) = after_reading_them_as_text();
        let expected = body
            .replace(FENCED, FENCED_SECTION)
            .replace(LISTED, LISTED_SECTION);
        assert_eq!(authored(|| markup.written(body)), expected, "{body}");
        assert_eq!(ExpandableForm::Linear.read(&expected), body, "{body}");
    }

    #[test]
    fn a_callers_block_held_as_text_elsewhere_is_a_section_first() {
        every_block_is_a_section(&format!("{FENCED}\n\n{LISTED}\n"));
    }

    #[test]
    fn a_callers_block_held_as_text_elsewhere_is_a_section_in_the_middle() {
        every_block_is_a_section(&format!("{LISTED}\n\n{FENCED}\n\n{LISTED}\n"));
    }

    #[test]
    fn a_callers_block_held_as_text_elsewhere_is_a_section_last() {
        every_block_is_a_section(&format!("{LISTED}\n\n{FENCED}\n"));
        every_block_is_a_section(&format!("New.\n\n{LISTED}\n\n{FENCED}"));
    }

    #[test]
    fn a_callers_block_held_as_text_elsewhere_is_a_section_alone() {
        every_block_is_a_section(&format!("{FENCED}\n"));
        every_block_is_a_section(&format!("New.\n\n{FENCED}"));
    }

    /// Prose is what tells a caller's body from the one read: the item held the fenced block
    /// alone, and the caller's body is a line of prose and that same block.
    #[test]
    fn a_callers_body_differing_from_the_read_only_in_prose_is_the_callers_own() {
        let markup = ExpandableForm::Linear.markup().expect("Linear's markup");
        assert_eq!(markup.read(FENCED), FENCED);
        let body = format!("New.\n\n{FENCED}");
        assert_eq!(
            authored(|| markup.written(&body)),
            format!("New.\n\n{FENCED_SECTION}")
        );
        // Set back byte for byte as it reads, it is written back as Linear stored it; one line
        // break more is no longer that body.
        assert_eq!(markup.read(FENCED), FENCED);
        assert_eq!(authored(|| markup.written(FENCED)), FENCED);
        assert_eq!(
            authored(|| markup.written(&format!("{FENCED}\n"))),
            format!("{FENCED_SECTION}\n")
        );
    }

    /// Content a read gave, set back byte for byte, keeps every block Linear held as literal
    /// text; with its prose edited it is the caller's own, and every block is a section.
    #[test]
    fn a_genuine_read_back_keeps_every_block_linear_held_as_text() {
        let (markup, stored) = after_reading_them_as_text();
        assert_eq!(authored(|| markup.written(&stored)), stored);
        let edited = stored.replace("Held.", "Edited prose.");
        assert_eq!(
            authored(|| markup.written(&edited)),
            edited.replace(FENCED, FENCED_SECTION).replace(
                &LISTED.replace("- a", "* a"),
                &LISTED_SECTION.replace("- a", "* a")
            )
        );
    }

    /// An engine write-back that changed another block — a link finished in it — keeps the
    /// block Linear held as literal text as it was stored, block by block.
    #[test]
    fn a_write_back_editing_another_block_keeps_the_literal_one() {
        let (markup, stored) = after_reading_them_as_text();
        let listed = LISTED.replace("- a", "* a");
        let edited = stored.replace(
            &listed,
            &listed.replace("a point", "a [point](https://x.test)"),
        );
        assert_eq!(
            markup.written(&edited),
            edited.replace(
                &listed.replace("a point", "a [point](https://x.test)"),
                &LISTED_SECTION.replace("- a point", "* a [point](https://x.test)")
            )
        );
    }

    /// The same blocks read as literal text and then, from a write's own answer, as sections are
    /// written back as the latest read found them: a copy's follow-up keeps the sections it wrote.
    #[test]
    fn a_body_written_back_carries_each_block_as_the_latest_read_found_it() {
        let markup = ExpandableForm::Linear.markup().expect("Linear's markup");
        let block =
            "<details>\n<summary>Reproduction</summary>\n\nRun:\n\n```\nrun\n```\n\n</details>";
        let section = "+++ Reproduction\n\nRun:\n\n```\nrun\n```\n\n+++";
        assert_eq!(markup.read(block), block);
        assert_eq!(markup.written(block), block, "held as literal text");
        assert_eq!(markup.read(section), block);
        assert_eq!(
            markup.written(block),
            section,
            "the latest read held a section"
        );
        assert_eq!(markup.read(block), block);
        assert_eq!(markup.written(block), block, "and literal text again");
    }

    /// The design document's case, as spike `spike-linear-nested-blocks` sent it to Linear: an
    /// outer block holding a paragraph, an image and an inner block holding an image.
    const NESTED: &str = "Nested.\n\n<details>\n<summary>Visual <strong>change</strong></summary>\
                          \n\nParagraph.\n\n![After](./after.png)\n\n<details>\n<summary>\
                          Before</summary>\n\n![Before](./before.png)\n\n</details>\n\n\
                          </details>\n\nAfter both.\n";
    const NESTED_SECTIONS: &str = "Nested.\n\n+++ Visual **change**\n\nParagraph.\n\n\
                                   ![After](./after.png)\n\n+++ Before\n\n![Before](./before.png)\
                                   \n\n+++\n\n+++\n\nAfter both.\n";

    #[test]
    fn a_nested_block_is_a_section_inside_a_section_and_reads_back_byte_for_byte() {
        assert_eq!(ExpandableForm::Linear.written(NESTED), NESTED_SECTIONS);
        assert_eq!(ExpandableForm::Linear.read(NESTED_SECTIONS), NESTED);
        for kind in [ExpandableForm::LocalMd, ExpandableForm::GithubProjects] {
            assert_eq!(kind.written(NESTED), NESTED);
        }
        assert_eq!(blocks(NESTED), 1);
        // Three levels, and two blocks side by side inside one.
        let deep = "<details>\n<summary>A</summary>\n\n<details>\n<summary>B</summary>\n\n\
                    <details>\n<summary>C</summary>\n\nC.\n\n</details>\n\n</details>\n\n\
                    <details>\n<summary>D</summary>\n\nD.\n\n</details>\n\n</details>";
        let sections = "+++ A\n\n+++ B\n\n+++ C\n\nC.\n\n+++\n\n+++\n\n+++ D\n\nD.\n\n\
                        +++\n\n+++";
        assert_eq!(ExpandableForm::Linear.written(deep), sections);
        assert_eq!(ExpandableForm::Linear.read(sections), deep);
        assert!(!ExpandableForm::Linear.written(deep).contains(">>>"));
    }

    /// An inner block whose Linear form would not read back as itself — a backtick in its
    /// summary's text — is carried as it stands inside the outer section, and the whole reads back.
    #[test]
    fn an_inner_block_linear_cannot_spell_is_carried_inside_its_outer_section() {
        let inner = "<details>\n<summary>A `tick`</summary>\n\nInner.\n\n</details>";
        let authored = format!("<details>\n<summary>Outer</summary>\n\n{inner}\n\n</details>\n");
        let linear = ExpandableForm::Linear.written(&authored);
        assert_eq!(linear, format!("+++ Outer\n\n{inner}\n\n+++\n"));
        assert_eq!(ExpandableForm::Linear.read(&linear), authored);
        // A body holding Linear's own section lines as text would read back as a block, so its
        // block is carried as it stands.
        let lines = "<details>\n<summary>Outer</summary>\n\n+++ X\n\nY.\n\n+++\n\n</details>\n";
        assert_eq!(ExpandableForm::Linear.written(lines), lines);
    }

    /// A nested block Linear held as literal text inside a section is written back as it was
    /// stored, the section around it written as a section.
    #[test]
    fn a_nested_block_linear_holds_as_literal_text_is_written_back_as_it_was_stored() {
        let markup = ExpandableForm::Linear.markup().expect("Linear's markup");
        let literal = "<details>\n<summary>Held as text</summary>\n\n* a list\n\n</details>";
        let stored = format!("+++ Outer\n\nOuter.\n\n{literal}\n\n+++");
        let read = markup.read(&stored);
        assert_eq!(
            read,
            format!("<details>\n<summary>Outer</summary>\n\nOuter.\n\n{literal}\n\n</details>")
        );
        assert_eq!(markup.written(&read), stored);
        // Authored by a caller, the same text is a section inside a section.
        assert_eq!(
            authored(|| markup.written(&format!("{read}\n"))),
            "+++ Outer\n\nOuter.\n\n+++ Held as text\n\n* a list\n\n+++\n\n+++\n"
        );
    }

    #[test]
    fn a_closing_line_inside_a_fenced_body_does_not_end_the_block() {
        let authored = "<details>\n<summary>Code</summary>\n\n```text\n</details>\n+++\n```\n\n\
                        </details>\n";
        let linear = ExpandableForm::Linear.written(authored);
        assert_eq!(linear, "+++ Code\n\n```text\n</details>\n+++\n```\n\n+++\n");
        assert_eq!(ExpandableForm::Linear.read(&linear), authored);
    }
}
