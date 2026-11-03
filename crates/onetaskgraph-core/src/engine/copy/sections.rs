//! A copied body's references and section links, written in the destination's form.
//!
//! [`BodyLinks`] is what a copy learns about one body before it writes it: the body as its
//! source holds it, the records of its project it names, and each one's headings. It is
//! rendered by [`BodyLinks::render`] against what the copy knows at that moment — each
//! referent's counterpart at the destination, where the body's own record lands, and the
//! heading ids a destination stored — and rendered again whenever the copy knows more,
//! because a counterpart created in this very copy, or a Linear record's heading ids, are
//! only known once it has been written. A link the copy cannot address yet is written as a
//! **placeholder**, the link exactly as the source spelled it, which
//! [`BodyLinks::finished`] replaces once nothing more will be learned.
//!
//! How a link is spelled on each kind is not decided here: every spelling, every parse back
//! and every markup is [`SectionForm`]'s.

use onetaskgraph_plugin_api::{NativeId, StoredHeading};

use super::{Counted, FRAGMENT, Level, stops_a_location};
use crate::GlobalId;
use crate::section_links::{
    RecordHeadings, SectionForm, Written, relative_path, resolve_relative, same_path,
};

/// The section-link figures of one body's rendering, which the copy reports per record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SectionCounts {
    /// Fragments naming no heading of the record they point at, carried verbatim.
    pub(super) unresolved: u64,
    /// Links to a heading the destination's form cannot address, written as the record's
    /// link alone.
    pub(super) unaddressable: u64,
}

impl SectionCounts {
    pub(super) fn add(&mut self, other: Self) {
        self.unresolved += other.unresolved;
        self.unaddressable += other.unaddressable;
    }
}

/// Where one referent's counterpart is, as the copy knows it.
#[derive(Debug, Clone)]
pub(super) enum Counterpart {
    /// At the destination, under `id`, which reports `location` for it.
    At {
        /// Its id there.
        id: NativeId,
        /// The location string the destination reports for it.
        location: String,
    },
    /// Written by this copy, so known only once it has landed: asked of [`Knows`].
    InCopy,
    /// No counterpart: the reference is left as it is, and counted unresolved.
    None,
    /// More than one, or two referents spelled alike: left as it is, and counted ambiguous.
    Ambiguous,
}

/// One record of the body's project that the body names.
#[derive(Debug, Clone)]
pub(super) struct Linked {
    /// Which interface it is read through.
    pub(super) level: Level,
    /// Its qualified id at the source.
    pub(super) id: GlobalId,
    /// The location string its source reports for it.
    pub(super) location: String,
    /// Every spelling of that location in the source kind's own form.
    spellings: Vec<String>,
    /// Its headings at the source.
    headings: RecordHeadings,
    /// The ids its source stored those headings under, where it has any.
    stored: Option<Vec<StoredHeading>>,
    /// Its counterpart.
    pub(super) counterpart: Counterpart,
}

impl Linked {
    /// A record whose source reports `location`, holding `content` and, where the source
    /// stores heading ids, `stored`.
    pub(super) fn new(
        level: Level,
        id: GlobalId,
        location: String,
        from: Option<SectionForm>,
        content: Option<&str>,
        stored: Option<Vec<StoredHeading>>,
    ) -> Self {
        let spellings =
            from.map_or_else(|| vec![location.clone()], |form| form.spellings(&location));
        Self {
            level,
            id,
            location,
            spellings,
            headings: RecordHeadings::of(content.unwrap_or_default()),
            stored,
            counterpart: Counterpart::None,
        }
    }
}

/// What a rendering asks of the copy it is part of.
pub(super) trait Knows {
    /// Where a referent written by this copy landed, once it has: its id and the location
    /// the destination reports for it.
    fn landed(&self, referent: &Linked) -> Option<(NativeId, String)>;
    /// The heading ids the destination stored the record `id`, read through `level`, under.
    fn stored(&self, level: Level, id: &NativeId) -> Option<Vec<StoredHeading>>;
    /// Where the body's own record is at the destination, once it is known.
    fn own(&self) -> Option<(NativeId, String)>;
}

/// Which rendering of a body this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pass {
    /// One while the copy may still learn more: a link not addressable yet is left pending.
    Provisional,
    /// The rendering that lands once nothing more will be learned: nothing is left pending,
    /// and a link the form still cannot address is written as the record's link alone.
    Final,
}

/// Where a link was spelled in a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spelled {
    /// As a whole Markdown link destination, `](…)`.
    Destination,
    /// Anywhere else, as a literal location.
    Literal,
}

/// A link written as its source spelled it, because the destination's form cannot address it
/// yet: rendered again once the copy knows more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pending {
    /// The link's text exactly as the source spelled it.
    pub(super) text: String,
    /// Where it was spelled, which is the context it is read again in.
    spelled: Spelled,
}

/// One rendering of a body.
#[derive(Debug, Clone, Default)]
pub(super) struct Rendered {
    /// The body as the destination is sent it.
    pub(super) content: String,
    /// The reference figures the invocation reports.
    pub(super) counts: Counted,
    /// The section-link figures the record reports.
    pub(super) sections: SectionCounts,
    /// The placeholders written.
    pub(super) pending: Vec<Pending>,
}

/// One body's references and section links, as the copy learned them before writing it.
#[derive(Debug, Clone)]
pub(super) struct BodyLinks {
    /// The body as its source holds it.
    pub(super) original: String,
    /// The source kind's form, by which its links are read; `None` reads every fragment as
    /// opaque.
    from: Option<SectionForm>,
    /// The destination kind's form, in which they are written; `None` carries every
    /// fragment byte for byte.
    to: Option<SectionForm>,
    /// Which interface the body's own record is written through.
    pub(super) level: Level,
    /// Where its source says the body's own record is.
    own_location: Option<String>,
    /// The body's own headings.
    own_headings: RecordHeadings,
    /// The ids its source stored the body's own headings under.
    own_stored: Option<Vec<StoredHeading>>,
    /// The records of its project it names, longest spelling first.
    pub(super) referents: Vec<Linked>,
}

/// One recognised occurrence: what it names, and where its text ends.
struct Occurrence<'a> {
    /// The referent, or `None` for the body's own record.
    referent: Option<&'a Linked>,
    /// Where the occurrence's text ends, its fragment included.
    end: usize,
    /// Its fragment, without the `#`.
    fragment: Option<&'a str>,
    /// Whether it was spelled as a path relative to the body's own file.
    relative: bool,
    /// Where it was spelled.
    spelled: Spelled,
}

impl BodyLinks {
    /// What the copy knows of one body before writing it.
    pub(super) fn new(
        original: String,
        from: Option<SectionForm>,
        to: Option<SectionForm>,
        level: Level,
        own_location: Option<String>,
        own_stored: Option<Vec<StoredHeading>>,
        mut referents: Vec<Linked>,
    ) -> Self {
        referents.sort_by_key(|referent| {
            std::cmp::Reverse(
                referent
                    .spellings
                    .iter()
                    .map(String::len)
                    .max()
                    .unwrap_or(0),
            )
        });
        Self {
            own_headings: RecordHeadings::of(&original),
            original,
            from,
            to,
            level,
            own_location,
            own_stored,
            referents,
        }
    }

    /// The same links read in `content` — a comment of the record — whose fragments of the
    /// record's own headings are the record's, as `self`'s are.
    pub(super) fn for_comment(&self, content: String) -> Self {
        Self {
            original: content,
            ..self.clone()
        }
    }

    /// Whether `content` names a record whose source reports `location`, in any spelling of
    /// the source kind's form, as a whole reference — or, read from `own` in a folder of
    /// Markdown, as a Markdown link to a path relative to it.
    pub(super) fn names(
        from: Option<SectionForm>,
        content: &str,
        own: Option<&str>,
        location: &str,
    ) -> bool {
        let spellings = from.map_or_else(
            || vec![location.to_owned()],
            |form| form.spellings(location),
        );
        if spellings
            .iter()
            .any(|spelling| (0..content.len()).any(|at| delimited(content, at, spelling)))
        {
            return true;
        }
        from == Some(SectionForm::LocalMd)
            && own.is_some_and(|own| {
                destinations(content).any(|(_, target)| {
                    let path = target.split(FRAGMENT).next().unwrap_or_default();
                    resolve_relative(own, path).is_some_and(|at| same_path(&at, location))
                })
            })
    }

    /// Whether this body holds anything a rendering could change: a referent, or a link to
    /// its own record a destination form writes otherwise.
    pub(super) fn matters(&self) -> bool {
        !self.referents.is_empty()
            || (self.from.is_some()
                && self.to.is_some()
                && destinations(&self.original).any(|(_, target)| target.starts_with(FRAGMENT)))
            || self.own_location.as_deref().is_some_and(|own| {
                (0..self.original.len()).any(|at| delimited(&self.original, at, own))
                    || (self.from == Some(SectionForm::LocalMd)
                        && destinations(&self.original).any(|(_, target)| {
                            let path = target.split(FRAGMENT).next().unwrap_or_default();
                            resolve_relative(own, path).is_some_and(|at| same_path(&at, own))
                        }))
            })
    }

    /// The body rendered against what `knows` says now, on the terms `pass` names.
    pub(super) fn render(&self, knows: &dyn Knows, pass: Pass) -> Rendered {
        let content = self.original.as_str();
        let mut rendered = Rendered {
            content: String::with_capacity(content.len()),
            ..Rendered::default()
        };
        let mut at = 0;
        while at < content.len() {
            if content[at..].starts_with("](") {
                // A destination may be written in angle brackets, as Linear stores every one.
                let opening = if content[at..].starts_with("](<") {
                    "](<"
                } else {
                    "]("
                };
                rendered.content.push_str(opening);
                at += opening.len();
                if let Some(occurrence) = self.in_destination(content, at, opening.len() == 3) {
                    at = self.write(&occurrence, content, at, knows, pass, &mut rendered);
                }
                continue;
            }
            if let Some(occurrence) = self.spelled(content, at) {
                at = self.write(&occurrence, content, at, knows, pass, &mut rendered);
                continue;
            }
            let character = content[at..]
                .chars()
                .next()
                .expect("a character at a boundary this walk only ever lands on");
            rendered.content.push(character);
            at += character.len_utf8();
        }
        rendered
    }

    /// `written` — a rendering whose placeholders were `pending`, as it was sent or as the
    /// destination stored it — with each placeholder rendered again against what `knows` says
    /// now, answering it and the placeholders still pending. Every other byte is left as it is.
    pub(super) fn finished(
        &self,
        written: &str,
        pending: &[Pending],
        knows: &dyn Knows,
        pass: Pass,
    ) -> (String, Vec<Pending>) {
        let mut replacements: Vec<(Pending, Option<String>)> = Vec::new();
        for placeholder in pending {
            if replacements.iter().any(|(held, _)| held == placeholder) {
                continue;
            }
            let original = if placeholder.spelled == Spelled::Destination {
                format!("]({})", placeholder.text)
            } else {
                placeholder.text.clone()
            };
            let lone = Self {
                original,
                ..self.clone()
            }
            .render(knows, pass);
            let text = if !lone.pending.is_empty() {
                None
            } else if placeholder.spelled == Spelled::Destination {
                lone.content
                    .strip_prefix("](")
                    .and_then(|rest| rest.strip_suffix(')'))
                    .map(str::to_owned)
            } else {
                Some(lone.content)
            };
            replacements.push((placeholder.clone(), text));
        }
        replacements.sort_by_key(|(placeholder, _)| std::cmp::Reverse(placeholder.text.len()));
        let mut still = Vec::new();
        let mut out = String::with_capacity(written.len());
        let mut at = 0;
        while at < written.len() {
            let found = replacements.iter().find(|(placeholder, _)| {
                if placeholder.spelled == Spelled::Destination {
                    // As it was sent, or as a destination that writes every link destination in
                    // angle brackets stores it.
                    written[at..].starts_with(&placeholder.text) && {
                        let next = written[at + placeholder.text.len()..].chars().next();
                        (written[..at].ends_with("](")
                            && next.is_none_or(|c| c == ')' || c.is_whitespace()))
                            || (written[..at].ends_with("](<") && next == Some('>'))
                    }
                } else {
                    delimited(written, at, &placeholder.text)
                }
            });
            if let Some((placeholder, text)) = found {
                match text {
                    Some(text) => out.push_str(text),
                    None => {
                        out.push_str(&placeholder.text);
                        if !still.contains(placeholder) {
                            still.push(placeholder.clone());
                        }
                    }
                }
                at += placeholder.text.len();
                continue;
            }
            let character = written[at..].chars().next().expect("a character boundary");
            out.push(character);
            at += character.len_utf8();
        }
        (out, still)
    }

    /// The occurrence a Markdown link destination starting at `at` holds, when it is a
    /// fragment of the body's own record or a path relative to the body's own file. Every
    /// other destination is left to [`Self::spelled`].
    fn in_destination<'a>(
        &'a self,
        content: &'a str,
        at: usize,
        angled: bool,
    ) -> Option<Occurrence<'a>> {
        let end = content[at..]
            .find(|c: char| {
                if angled {
                    c == '>'
                } else {
                    c == ')' || c.is_whitespace()
                }
            })
            .map_or(content.len(), |found| at + found);
        let target = &content[at..end];
        if let Some(fragment) = target.strip_prefix(FRAGMENT) {
            return Some(Occurrence {
                referent: None,
                end,
                fragment: Some(fragment),
                relative: false,
                spelled: Spelled::Destination,
            });
        }
        if self.from != Some(SectionForm::LocalMd) {
            return None;
        }
        let (path, fragment) = match target.split_once(FRAGMENT) {
            Some((path, fragment)) => (path, Some(fragment)),
            None => (target, None),
        };
        let resolved = resolve_relative(self.own_location.as_deref()?, path)?;
        let referent = self
            .referents
            .iter()
            .find(|referent| same_path(&referent.location, &resolved));
        if referent.is_none()
            && !self
                .own_location
                .as_deref()
                .is_some_and(|own| same_path(own, &resolved))
        {
            return None;
        }
        Some(Occurrence {
            referent,
            end,
            fragment,
            relative: true,
            spelled: Spelled::Destination,
        })
    }

    /// The whole reference spelled at `at`, with its fragment, when one is: a referent's, or
    /// the body's own record's.
    fn spelled<'a>(&'a self, content: &'a str, at: usize) -> Option<Occurrence<'a>> {
        let own = self.own_location.as_ref().map(|own| (None, own));
        let (referent, spelling) = self
            .referents
            .iter()
            .flat_map(|referent| {
                referent
                    .spellings
                    .iter()
                    .map(move |spelling| (Some(referent), spelling))
            })
            .chain(own)
            .find(|(_, spelling)| delimited(content, at, spelling))?;
        let after = at + spelling.len();
        let (end, fragment) = if content[after..].starts_with(FRAGMENT) {
            let start = after + FRAGMENT.len_utf8();
            let end = content[start..]
                .find(|c: char| stops_a_location(Some(c)))
                .map_or(content.len(), |found| start + found);
            (end, Some(&content[start..end]))
        } else {
            (after, None)
        };
        Some(Occurrence {
            referent,
            end,
            fragment,
            relative: false,
            spelled: Spelled::Literal,
        })
    }

    /// Write one occurrence starting at `at`, answering where the walk goes on from.
    fn write(
        &self,
        occurrence: &Occurrence<'_>,
        content: &str,
        at: usize,
        knows: &dyn Knows,
        pass: Pass,
        rendered: &mut Rendered,
    ) -> usize {
        let verbatim = &content[at..occurrence.end];
        let written = match occurrence.referent {
            None => self.own_link(occurrence, verbatim, knows, pass, rendered),
            Some(referent) => {
                self.cross_link(referent, occurrence, verbatim, knows, pass, rendered)
            }
        };
        match written {
            Some(written) => rendered.content.push_str(&written),
            None => {
                rendered.pending.push(Pending {
                    text: verbatim.to_owned(),
                    spelled: occurrence.spelled,
                });
                rendered.content.push_str(verbatim);
            }
        }
        occurrence.end
    }

    /// A link to the body's own record: a fragment of it, or its own location. `None` is a
    /// link not addressable yet.
    fn own_link(
        &self,
        occurrence: &Occurrence<'_>,
        verbatim: &str,
        knows: &dyn Knows,
        pass: Pass,
        rendered: &mut Rendered,
    ) -> Option<String> {
        let bare = verbatim.starts_with(FRAGMENT);
        let (Some(from), Some(to), Some(fragment)) = (self.from, self.to, occurrence.fragment)
        else {
            // A plain link to itself, or a source or a destination with no form: its location
            // rewritten as any reference's is, its fragment carried.
            if bare {
                return Some(verbatim.to_owned());
            }
            return match knows.own() {
                Some((_, location)) => {
                    rendered.counts.rewritten += 1;
                    self.plain(&location, occurrence.relative, knows)
                        .map(|link| with_fragment(link, occurrence.fragment))
                        .or_else(|| (pass == Pass::Final).then(|| location.clone()))
                }
                None if pass == Pass::Final => Some(verbatim.to_owned()),
                None => None,
            };
        };
        let heading = if bare {
            from.same_record(fragment, &self.own_headings)
        } else {
            from.heading_of(fragment, &self.own_headings, self.own_stored.as_deref())
        };
        let Some(heading) = heading else {
            rendered.sections.unresolved += 1;
            return Some(verbatim.to_owned());
        };
        let own = knows.own();
        let stored = own
            .as_ref()
            .and_then(|(id, _)| knows.stored(self.level, id));
        let written = to.same(
            own.as_ref().map(|(_, location)| location.as_str()),
            &heading,
            &self.own_headings,
            stored.as_deref(),
        );
        let fallback = own.map_or_else(|| verbatim.to_owned(), |(_, location)| location);
        settle(written, fallback, pass, rendered)
    }

    /// A link to another record of the body's project. `None` is a link not addressable yet.
    fn cross_link(
        &self,
        referent: &Linked,
        occurrence: &Occurrence<'_>,
        verbatim: &str,
        knows: &dyn Knows,
        pass: Pass,
        rendered: &mut Rendered,
    ) -> Option<String> {
        let (id, location) = match &referent.counterpart {
            Counterpart::At { id, location } => (id.clone(), location.clone()),
            Counterpart::InCopy => match knows.landed(referent) {
                Some(landed) => landed,
                None if pass == Pass::Final => {
                    rendered.counts.unresolved += 1;
                    return Some(verbatim.to_owned());
                }
                None => return None,
            },
            Counterpart::None => {
                rendered.counts.unresolved += 1;
                return Some(verbatim.to_owned());
            }
            Counterpart::Ambiguous => {
                rendered.counts.unresolved += 1;
                rendered.counts.ambiguous += 1;
                return Some(verbatim.to_owned());
            }
        };
        rendered.counts.rewritten += 1;
        let Some(fragment) = occurrence.fragment else {
            return self
                .plain(&location, occurrence.relative, knows)
                .or_else(|| (pass == Pass::Final).then(|| location.clone()));
        };
        let heading = self.from.and_then(|from| {
            from.heading_of(fragment, &referent.headings, referent.stored.as_deref())
        });
        let (Some(to), Some(heading)) = (self.to, heading) else {
            if self.to.is_some() && self.from.is_some() {
                rendered.sections.unresolved += 1;
            }
            // A fragment naming no heading, or a destination with no form: the record's link,
            // its fragment carried byte for byte — into a folder of Markdown as the path relative
            // to the linking file a section link there is.
            // llmlint: ignore[changed_behavior_has_e2e] Held by `a_reference_carrying_a_fragment_is_rewritten_and_keeps_the_fragment` and `a_fragment_reference_is_left_and_counted_as_any_other_reference_without_a_counterpart` in `crates/onetaskgraph-core/tests/copy.rs`, which drive the real engine's copy into an in-memory destination that reports a location for every record and holds no section form, and by `a_reference_carrying_a_fragment_lands_on_the_copied_tasks_destination_location` and the section-link journeys in `crates/onetaskgraph-e2e` for every kind with one.
            let relative = occurrence.relative || self.to == Some(SectionForm::LocalMd);
            return self
                .plain(&location, relative, knows)
                .or_else(|| (pass == Pass::Final).then(|| location.clone()))
                .map(|link| format!("{link}#{fragment}"));
        };
        let own = knows.own().map(|(_, location)| location);
        let stored = knows.stored(referent.level, &id);
        let written = to.cross(
            &location,
            own.as_deref(),
            &heading,
            &referent.headings,
            stored.as_deref(),
        );
        settle(written, location, pass, rendered)
    }

    /// A plain reference to `location`: relative to the body's own file when it was spelled
    /// so and the destination is a folder of Markdown, and the location itself otherwise.
    /// `None` while the body's own file is not known yet.
    fn plain(&self, location: &str, relative: bool, knows: &dyn Knows) -> Option<String> {
        if relative && self.to == Some(SectionForm::LocalMd) {
            return knows.own().map(|(_, own)| relative_path(&own, location));
        }
        Some(location.to_owned())
    }
}

/// What one form's answer is written as: `fallback` when the form still cannot address the
/// heading on the last rendering, and `None` — not addressable yet — before it.
fn settle(
    written: Written,
    fallback: String,
    pass: Pass,
    rendered: &mut Rendered,
) -> Option<String> {
    match written {
        Written::Link(link) => Some(link),
        Written::Unaddressable(link) => {
            rendered.sections.unaddressable += 1;
            Some(link)
        }
        Written::Pending if pass == Pass::Final => {
            rendered.sections.unaddressable += 1;
            Some(fallback)
        }
        Written::Pending => None,
    }
}

/// `link` with `#fragment` after it, when there is one.
fn with_fragment(link: String, fragment: Option<&str>) -> String {
    match fragment {
        Some(fragment) => format!("{link}#{fragment}"),
        None => link,
    }
}

/// Whether `location` occurs at `at` as a whole reference, as `delimited_at` reads one.
fn delimited(content: &str, at: usize, location: &str) -> bool {
    super::delimited_at(content, at, location)
}

/// Every Markdown link destination `content` holds, with where it starts — one written in
/// angle brackets without them, as [`BodyLinks::render`] reads it.
fn destinations(content: &str) -> impl Iterator<Item = (usize, &str)> {
    content.match_indices("](").map(move |(at, _)| {
        let angled = content[at + 2..].starts_with('<');
        let start = at + 2 + usize::from(angled);
        let end = content[start..]
            .find(|c: char| {
                if angled {
                    c == '>'
                } else {
                    c == ')' || c.is_whitespace()
                }
            })
            .map_or(content.len(), |found| start + found);
        (start, &content[start..end])
    })
}
