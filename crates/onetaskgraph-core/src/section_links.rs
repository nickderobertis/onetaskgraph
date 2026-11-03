//! How a section link — a link to one heading of a record — is spelled on each kind of
//! destination, decided once.
//!
//! A copy re-points every link between the records of one project at the destination's own
//! records (see `engine::copy`). A link that names a heading as well as a record has a
//! second half, the fragment, and what a fragment has to say to land on a heading differs
//! by tracker: a Markdown file addresses a heading by its slug, a GitHub issue page only by
//! an anchor the body itself carries, and Linear only by the id its editor gave the heading.
//! So each destination kind has exactly one [`SectionForm`] entry here, and every path that
//! writes, recognises or counts a section link for that kind reads it:
//!
//! - [`SectionForm::cross`] and [`SectionForm::same`] write a link to a heading of another
//!   record, and of the body's own;
//! - [`SectionForm::spellings`], [`SectionForm::heading_of`] and
//!   [`SectionForm::same_record`] parse that kind's own form back into the location a source
//!   reports and a heading, so a body copied *out of* that kind is recognised exactly as an
//!   authored one is;
//! - [`SectionForm::markup`] is the markup a destination's bodies carry for its headings to
//!   be addressable at all, which only GitHub needs.
//!
//! A heading is carried between them as its text and its position among the headings of
//! that text in its record ([`Heading`]), never as a slug, because the slug rules of GitHub
//! and Linear differ. The README's "Section links" is where these forms are stated for a
//! consumer.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use onetaskgraph_plugin_api::{BodyMarkup, SharedBodyMarkup, StoredHeading};

/// One heading of a record: its text, and how many headings of that same text come before
/// it in the record — `0` for the first `## Budgets`, `1` for the second.
// llmlint: ignore[invalid_states_unrepresentable] A heading is carried from one record to its counterpart in another, where it may name nothing — the case a copy counts unresolved or unaddressable — so membership cannot be a property of the value; every form answers it at use, by `RecordHeadings::slug_of` and Linear's stored ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Heading {
    /// Its text, as [`headings`] reads it.
    pub text: String,
    /// Its position among the headings of the same text.
    pub ordinal: usize,
}

/// The headings of one record's content, in order, by [`headings`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordHeadings {
    texts: Vec<String>,
}

impl RecordHeadings {
    /// The headings `content` holds.
    #[must_use]
    pub fn of(content: &str) -> Self {
        Self {
            texts: headings(content),
        }
    }

    /// Each heading's text, in order.
    #[must_use]
    pub fn texts(&self) -> &[String] {
        &self.texts
    }

    /// The heading at `index`, as text and ordinal.
    fn at(&self, index: usize) -> Option<Heading> {
        let text = self.texts.get(index)?;
        let ordinal = self.texts[..index]
            .iter()
            .filter(|earlier| same_text(earlier, text))
            .count();
        Some(Heading {
            text: text.clone(),
            ordinal,
        })
    }

    /// Where `heading` is among these.
    fn index_of(&self, heading: &Heading) -> Option<usize> {
        self.texts
            .iter()
            .enumerate()
            .filter(|(_, text)| same_text(text, &heading.text))
            .nth(heading.ordinal)
            .map(|(index, _)| index)
    }

    /// The heading whose GitHub slug — [`github_slugs`], repeats included — is `slug`.
    #[must_use]
    pub fn by_slug(&self, slug: &str) -> Option<Heading> {
        let index = github_slugs(&self.texts)
            .iter()
            .position(|held| held == slug)?;
        self.at(index)
    }

    /// The GitHub slug of `heading` among these, repeats included.
    #[must_use]
    pub fn slug_of(&self, heading: &Heading) -> Option<String> {
        let index = self.index_of(heading)?;
        github_slugs(&self.texts).into_iter().nth(index)
    }
}

/// Whether two heading texts name the same heading text: compared without the inline
/// Markdown a tracker's own plain text leaves out (code spans and emphasis marks).
fn same_text(a: &str, b: &str) -> bool {
    plain(a) == plain(b)
}

/// A heading's text without inline Markdown marks.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|character| !matches!(character, '`' | '*'))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Every ATX heading's text in `content`, in order — `## Budgets` is `Budgets` — skipping
/// anything inside a fenced code block, which is not a heading.
///
/// The leading `#` marks, up to three spaces before them, and a closing run of `#` are not
/// the text.
#[must_use]
pub fn headings(content: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in content.lines() {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() <= 3
            && let Some((marker, width)) = fence_of(trimmed)
        {
            match fence {
                None => fence = Some((marker, width)),
                Some((open, wide)) if open == marker && width >= wide => fence = None,
                Some(_) => {}
            }
            continue;
        }
        if fence.is_none()
            && let Some(text) = heading_text(line)
        {
            found.push(text);
        }
    }
    found
}

/// A fence's marker and width, when `line` opens or closes one.
fn fence_of(line: &str) -> Option<(char, usize)> {
    let marker = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let width = line.chars().take_while(|c| *c == marker).count();
    (width >= 3).then_some((marker, width))
}

/// The text of one ATX heading line, when `line` is one.
fn heading_text(line: &str) -> Option<String> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marks = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&marks) {
        return None;
    }
    let rest = &trimmed[marks..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let mut text = rest.trim();
    let closing = text.trim_end_matches('#');
    if closing.len() != text.len() && (closing.is_empty() || closing.ends_with(' ')) {
        text = closing.trim_end();
    }
    Some(text.to_owned())
}

// llmlint: ignore-block[code_lands_in_the_domain_that_owns_it] The task contract this module implements requires each destination kind's form to be "one entry per destination kind" chosen "in one place in onetaskgraph-core", and the slug rules implemented there once; no plugin may depend on the engine (AGENTS.md), so a plugin cannot own a form the engine reads, and GitHub's anchor markup reaches the GitHub plugin only as the `BodyMarkup` this entry hands it.
/// GitHub's slug of one heading text, before repeats are told apart: lowercased, every
/// character that is not a letter, a digit, a space, `-` or `_` removed, and each space
/// replaced by `-`. `### Step 2.b: retry (once)` is `step-2b-retry-once`.
///
/// It is also the authored form's slug: GitHub's file view and VS Code agree on it.
#[must_use]
pub fn github_slug(text: &str) -> String {
    plain(text)
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The GitHub slug of each heading of one record, in order, the n-th repeat of a slug
/// suffixed `-<n>`: the second `## Budgets` is `budgets-1`.
#[must_use]
pub fn github_slugs(texts: &[String]) -> Vec<String> {
    let mut taken: Vec<String> = Vec::with_capacity(texts.len());
    for text in texts {
        let base = github_slug(text);
        let mut slug = base.clone();
        let mut repeat = 0;
        while taken.contains(&slug) {
            repeat += 1;
            slug = format!("{base}-{repeat}");
        }
        taken.push(slug);
    }
    taken
}

/// Linear's slug of one heading text, as the `<slug>` part of its own heading link spells
/// it: lowercased, `&` read as `and`, `_` read as `-`, every character that is not a letter,
/// a digit, a space or `-` removed, and each run of spaces and `-` one `-`, none at either
/// end. `### Step 2.b: retry (once)` is `step-2b-retry-once`, and a repeat is not suffixed:
/// the id beside it tells two headings of one text apart.
///
/// Established from what Linear stores rather than assumed: its one copied heading link in
/// hand, `#heading-budget-demo-quota-seconds-e1544ca7`, and the URL slugs Linear itself gave
/// documents titled `Step 2.b: retry (once)` (`step-2b-retry-once`) and
/// `Under_score & A/B  x--y` (`under-score-and-ab-x-y`). A recognised Linear link is never
/// read by this part: its heading is found by the id.
// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The tracker's form is undocumented and has no authoritative artifact to derive from; by the manager's ruling on node onetaskgraph-section-links its drift check is the committed live check `scripts/live-section-links.py`, run by hand against the real tracker, with a gated live-lane entry point recorded as a follow-up, and the locally gated journeys in `crates/onetaskgraph-e2e/tests/e2e/section_links.rs` hold this entry to the README's statement of it.
#[must_use]
pub fn linear_slug(text: &str) -> String {
    let lowered = plain(text).to_lowercase().replace('&', " and ");
    let mut slug = String::with_capacity(lowered.len());
    let mut gap = false;
    for character in lowered.chars() {
        if character == ' ' || character == '-' || character == '_' {
            gap = true;
        } else if character.is_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.push(character);
        }
    }
    slug
}

/// Which form Linear's section links take — one named decision, so that switching it is
/// one amendment here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearSectionForm {
    /// Linear's own "Copy link" form: `<record URL>#heading-<slug>-<first 8 hex of the
    /// heading's stored id>`.
    HeadingLink,
    /// The record's URL alone, landing at its top.
    RecordUrl,
}

/// The variable a test or a diagnostic sets to `record-url` to have Linear section links
/// written as the record's URL alone; anything else, or nothing, is
/// [`LINEAR_SECTION_FORM`]. Outside the `ONETASKGRAPH_` prefix on purpose: every variable under
/// it is read as a setting of the configuration's environment layer.
pub const LINEAR_SECTION_FORM_VARIABLE: &str = "OTG_LINEAR_SECTION_FORM";

/// `linear_section_form`: the form Linear section links are written in.
pub const LINEAR_SECTION_FORM: LinearSectionForm = LinearSectionForm::HeadingLink;

/// [`LINEAR_SECTION_FORM`], unless [`LINEAR_SECTION_FORM_VARIABLE`] says `record-url`.
#[must_use]
pub fn linear_section_form() -> LinearSectionForm {
    match std::env::var(LINEAR_SECTION_FORM_VARIABLE).as_deref() {
        Ok("record-url") => LinearSectionForm::RecordUrl,
        _ => LINEAR_SECTION_FORM,
    }
}

/// One destination kind's section form: the one entry each kind has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionForm {
    /// A folder of Markdown: the path relative to the linking record's own file, then
    /// `#<slug>`.
    LocalMd,
    /// A GitHub Projects board's issues: `https://www.github.com/<owner>/<repo>/issues/<n>`
    /// then `#user-content-<slug>`, against an `<a id="<slug>"></a>` the body carries on the
    /// line before each heading.
    GithubProjects,
    /// Linear issues, projects and documents: see [`LinearSectionForm`].
    Linear(LinearSectionForm),
}

/// What writing one section link came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// The link, in the destination's form.
    Link(String),
    /// The record's link with no fragment: the destination holds no heading this form can
    /// address — counted unaddressable.
    Unaddressable(String),
    /// What the form needs to address the heading is not known yet: the counterpart's
    /// stored heading ids, or where the linking record is.
    Pending,
}

/// The canonical GitHub host prefix: the one a GitHub board reports its issues' locations
/// under, and recognised in a link written by hand.
const GITHUB: &str = "https://github.com/";
/// The prefix [`SectionForm::GithubProjects`] writes a cross-record link under: GitHub answers
/// it with a redirect that keeps the fragment, so the link lands when clicked inside GitHub.
const GITHUB_WWW: &str = "https://www.github.com/";
/// The prefix GitHub gives every `id` in a body it renders.
const USER_CONTENT: &str = "user-content-";
/// The prefix of Linear's heading fragments.
const LINEAR_HEADING: &str = "heading-";

impl SectionForm {
    /// The entry for a source of plugin kind `kind`, or `None` for a kind whose section
    /// links are carried as they are — every fragment byte for byte, as before there were
    /// forms.
    #[must_use]
    pub fn for_kind(kind: &str) -> Option<Self> {
        match kind {
            "local-md" => Some(Self::LocalMd),
            "github-projects" => Some(Self::GithubProjects),
            "linear" => Some(Self::Linear(linear_section_form())),
            _ => None,
        }
    }

    /// Every spelling, in this kind's own form, of a record whose source reports `location`:
    /// the location itself, and for a GitHub issue the `www.github.com` spelling this form
    /// writes.
    #[must_use]
    pub fn spellings(self, location: &str) -> Vec<String> {
        let mut spelled = vec![location.to_owned()];
        if self == Self::GithubProjects
            && let Some(rest) = location.strip_prefix(GITHUB)
        {
            spelled.push(format!("{GITHUB_WWW}{rest}"));
        }
        spelled
    }

    /// The heading `fragment` names in a record with `headings` — and, for Linear, the ids
    /// its editor stored them under — read in this kind's own form.
    #[must_use]
    pub fn heading_of(
        self,
        fragment: &str,
        headings: &RecordHeadings,
        stored: Option<&[StoredHeading]>,
    ) -> Option<Heading> {
        match self {
            Self::LocalMd => headings.by_slug(fragment),
            Self::GithubProjects => headings.by_slug(
                fragment
                    .strip_prefix(USER_CONTENT)
                    .filter(|slug| !slug.is_empty())
                    .unwrap_or(fragment),
            ),
            Self::Linear(_) => {
                let id8 = linear_id8(fragment)?;
                let stored = stored?;
                let index = stored.iter().position(|held| held.id.starts_with(id8))?;
                let text = &stored[index].text;
                let ordinal = stored[..index]
                    .iter()
                    .filter(|earlier| same_text(&earlier.text, text))
                    .count();
                Some(Heading {
                    text: text.clone(),
                    ordinal,
                })
            }
        }
    }

    /// The heading a same-record link — a Markdown link destination of `#<fragment>` alone —
    /// names in this kind's own form, when it is one. Linear's same-record link is the record's
    /// own URL with a fragment, which [`Self::heading_of`] reads.
    #[must_use]
    pub fn same_record(self, fragment: &str, headings: &RecordHeadings) -> Option<Heading> {
        match self {
            Self::LocalMd | Self::GithubProjects => self.heading_of(fragment, headings, None),
            Self::Linear(_) => None,
        }
    }

    /// A link from a record at `from` to the `heading` of the record at `location`, whose
    /// headings are `headings` — and, where this form needs them, `stored` at its backend.
    #[must_use]
    pub fn cross(
        self,
        location: &str,
        from: Option<&str>,
        heading: &Heading,
        headings: &RecordHeadings,
        stored: Option<&[StoredHeading]>,
    ) -> Written {
        match self {
            Self::LocalMd => {
                let Some(from) = from else {
                    return Written::Pending;
                };
                let Some(slug) = headings.slug_of(heading) else {
                    return Written::Unaddressable(relative_path(from, location));
                };
                Written::Link(format!("{}#{slug}", relative_path(from, location)))
            }
            // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The tracker's form is undocumented and has no authoritative artifact to derive from; by the manager's ruling on node onetaskgraph-section-links its drift check is the committed live check `scripts/live-section-links.py`, run by hand against the real tracker, with a gated live-lane entry point recorded as a follow-up, and the locally gated journeys in `crates/onetaskgraph-e2e/tests/e2e/section_links.rs` hold this entry to the README's statement of it.
            Self::GithubProjects => {
                let at = location
                    .strip_prefix(GITHUB)
                    .map_or_else(|| location.to_owned(), |rest| format!("{GITHUB_WWW}{rest}"));
                match headings.slug_of(heading) {
                    Some(slug) => Written::Link(format!("{at}#{USER_CONTENT}{slug}")),
                    None => Written::Unaddressable(at),
                }
            }
            Self::Linear(LinearSectionForm::RecordUrl) => Written::Link(location.to_owned()),
            Self::Linear(LinearSectionForm::HeadingLink) => {
                let Some(stored) = stored else {
                    return Written::Pending;
                };
                match linear_fragment(heading, stored) {
                    Some(fragment) => Written::Link(format!("{location}#{fragment}")),
                    None => Written::Unaddressable(location.to_owned()),
                }
            }
        }
    }

    /// A link from a record to its own `heading`: `own` is where the destination says the
    /// record is, which only Linear's form needs.
    #[must_use]
    pub fn same(
        self,
        own: Option<&str>,
        heading: &Heading,
        headings: &RecordHeadings,
        stored: Option<&[StoredHeading]>,
    ) -> Written {
        match self {
            Self::LocalMd => match headings.slug_of(heading) {
                Some(slug) => Written::Link(format!("#{slug}")),
                None => Written::Unaddressable(String::new()),
            },
            Self::GithubProjects => match headings.slug_of(heading) {
                Some(slug) => Written::Link(format!("#{USER_CONTENT}{slug}")),
                None => Written::Unaddressable(String::new()),
            },
            Self::Linear(_) => match own {
                Some(own) => self.cross(own, None, heading, headings, stored),
                None => Written::Pending,
            },
        }
    }

    /// The markup this kind's bodies carry for their headings to be addressable, when it
    /// needs any: GitHub's issue page gives a heading no `id`, so its bodies carry one.
    #[must_use]
    pub fn markup(self) -> Option<SharedBodyMarkup> {
        match self {
            Self::GithubProjects => Some(Arc::new(GithubAnchors)),
            Self::LocalMd | Self::Linear(_) => None,
        }
    }
}

/// The 8 hex characters ending a Linear heading fragment `heading-<slug>-<id8>`.
fn linear_id8(fragment: &str) -> Option<&str> {
    let rest = fragment.strip_prefix(LINEAR_HEADING)?;
    let (_, id8) = rest.rsplit_once('-')?;
    (id8.len() == 8 && id8.chars().all(|c| c.is_ascii_hexdigit())).then_some(id8)
}

/// `heading-<slug>-<id8>` for `heading` among the headings `stored` holds, found by its
/// position among the headings of its text.
fn linear_fragment(heading: &Heading, stored: &[StoredHeading]) -> Option<String> {
    let held = stored
        .iter()
        .filter(|held| same_text(&held.text, &heading.text))
        .nth(heading.ordinal)?;
    let id8: String = held.id.chars().filter(|c| *c != '-').take(8).collect();
    (id8.len() == 8 && id8.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("{LINEAR_HEADING}{}-{id8}", linear_slug(&heading.text)))
}

/// The path of `to` written relative to the directory of the file `from`, with `/` between
/// its parts: `../tasks/t.md` from `documents/d.md`, `t.md` beside it.
#[must_use]
pub fn relative_path(from: &str, to: &str) -> String {
    let from_dir: Vec<Component<'_>> = Path::new(from)
        .parent()
        .map(|dir| dir.components().collect())
        .unwrap_or_default();
    let to_parts: Vec<Component<'_>> = Path::new(to).components().collect();
    let common = from_dir
        .iter()
        .zip(&to_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = vec!["..".to_owned(); from_dir.len() - common];
    parts.extend(
        to_parts[common..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// The absolute path a Markdown link destination `target` names, read from the file
/// `from`, when it is a relative path at all — not absolute, not a URL, not a fragment.
#[must_use]
pub fn resolve_relative(from: &str, target: &str) -> Option<String> {
    if target.is_empty()
        || target.starts_with('#')
        || target.starts_with('/')
        || target.starts_with('\\')
        || target.contains("://")
        || target.starts_with("mailto:")
        || Path::new(target).is_absolute()
    {
        return None;
    }
    let decoded = percent_decoded(target)?;
    let mut path = PathBuf::new();
    for component in Path::new(from).parent()?.join(decoded).components() {
        match component {
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            other => path.push(other.as_os_str()),
        }
    }
    path.to_str().map(str::to_owned)
}

/// `text` with each `%XX` escape decoded, when every escape is one and the result is UTF-8.
fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// The `<a id="<slug>"></a>` line GitHub's form writes before every heading, and strips on
/// reading a body back.
struct GithubAnchors;

/// The anchor line for one slug.
fn anchor_line(slug: &str) -> String {
    format!("<a id=\"{slug}\"></a>")
}

/// The slug an anchor line names, when `line` is exactly one.
fn anchored(line: &str) -> Option<&str> {
    line.strip_prefix("<a id=\"")?.strip_suffix("\"></a>")
}

impl BodyMarkup for GithubAnchors {
    fn written(&self, content: &str) -> String {
        let texts = headings(content);
        if texts.is_empty() {
            return content.to_owned();
        }
        let slugs = github_slugs(&texts);
        let mut next = slugs.iter();
        let mut written = String::with_capacity(content.len() + slugs.len() * 24);
        let mut fence: Option<(char, usize)> = None;
        for line in content.split_inclusive('\n') {
            let bare = line.trim_end_matches(['\n', '\r']);
            let trimmed = bare.trim_start_matches(' ');
            if bare.len() - trimmed.len() <= 3
                && let Some((marker, width)) = fence_of(trimmed)
            {
                match fence {
                    None => fence = Some((marker, width)),
                    Some((open, wide)) if open == marker && width >= wide => fence = None,
                    Some(_) => {}
                }
            } else if fence.is_none()
                && heading_text(bare).is_some()
                && let Some(slug) = next.next()
            {
                written.push_str(&anchor_line(slug));
                written.push('\n');
            }
            written.push_str(line);
        }
        written
    }

    /// Exactly the lines [`Self::written`] adds: an anchor line outside fenced code, straight
    /// before a heading, naming that heading's own slug. An anchor a person wrote — another id,
    /// or one inside a fenced example — is part of the body and is read as it is.
    fn read(&self, content: &str) -> String {
        let slugs = github_slugs(&headings(content));
        let lines: Vec<&str> = content.split_inclusive('\n').collect();
        let mut read = String::with_capacity(content.len());
        let mut fence: Option<(char, usize)> = None;
        let mut seen = 0;
        for (index, line) in lines.iter().enumerate() {
            let bare = line.trim_end_matches(['\n', '\r']);
            let trimmed = bare.trim_start_matches(' ');
            if bare.len() - trimmed.len() <= 3
                && let Some((marker, width)) = fence_of(trimmed)
            {
                match fence {
                    None => fence = Some((marker, width)),
                    Some((open, wide)) if open == marker && width >= wide => fence = None,
                    Some(_) => {}
                }
            } else if fence.is_none() {
                if heading_text(bare).is_some() {
                    seen += 1;
                } else if let Some(slug) = anchored(bare)
                    && lines.get(index + 1).is_some_and(|next| {
                        heading_text(next.trim_end_matches(['\n', '\r'])).is_some()
                    })
                    && slugs.get(seen).is_some_and(|expected| expected == slug)
                {
                    continue;
                }
            }
            read.push_str(line);
        }
        read
    }
}
// llmlint: ignore-end[code_lands_in_the_domain_that_owns_it]
