//! What the harness found, kind by kind, and how it prints.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One difference between what `serde_norway` and `serde-saphyr` did with one case.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Diff {
    /// The input this is about: a file, or a named synthetic case.
    pub case: String,
    /// `read`, `write` or `error`, and which path.
    pub aspect: String,
    /// What groups this difference with others of the same cause.
    pub class: String,
    /// What `serde_norway` gave.
    pub before: String,
    /// What `serde-saphyr` gave.
    pub after: String,
}

/// Every case of one document kind, and its differences.
#[derive(Debug, Default, serde::Serialize)]
pub struct Kind {
    /// The document kind, as the task names it.
    pub name: String,
    /// How many cases were run, by input class (`real …`, `synthetic`).
    pub cases: BTreeMap<String, usize>,
    /// Every difference found.
    pub diffs: Vec<Diff>,
    /// Notes the kind wants printed beside its differences.
    pub notes: Vec<String>,
}

impl Kind {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    pub fn count(&mut self, class: &str) {
        *self.cases.entry(class.to_owned()).or_default() += 1;
    }

    pub fn diff(
        &mut self,
        case: impl Into<String>,
        aspect: &str,
        class: impl Into<String>,
        before: impl Into<String>,
        after: impl Into<String>,
    ) {
        self.diffs.push(Diff {
            case: case.into(),
            aspect: aspect.to_owned(),
            class: class.into(),
            before: before.into(),
            after: after.into(),
        });
    }

    /// The kind as the harness prints it: every difference, grouped by class, with the whole
    /// before and after of the first of each class and the case and summary line of the rest.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let total: usize = self.cases.values().sum();
        let cases: Vec<String> = self.cases.iter().map(|(k, v)| format!("{v} {k}")).collect();
        let _ = writeln!(
            out,
            "\n=== {} — {} cases ({}), {} differences",
            self.name,
            total,
            cases.join(", "),
            self.diffs.len()
        );
        for note in &self.notes {
            let _ = writeln!(out, "  note: {note}");
        }
        if self.diffs.is_empty() {
            let _ = writeln!(out, "  no difference");
            return out;
        }
        let mut classes: BTreeMap<(&str, &str), Vec<&Diff>> = BTreeMap::new();
        for diff in &self.diffs {
            classes
                .entry((diff.aspect.as_str(), diff.class.as_str()))
                .or_default()
                .push(diff);
        }
        for ((aspect, class), diffs) in classes {
            let _ = writeln!(out, "  --- [{aspect}] {class} ({} cases)", diffs.len());
            let first = diffs[0];
            let _ = writeln!(out, "    e.g. {}", first.case);
            let _ = writeln!(out, "    serde_norway:\n{}", indent(&first.before));
            let _ = writeln!(out, "    serde-saphyr:\n{}", indent(&first.after));
            for diff in &diffs[1..] {
                let _ = writeln!(out, "    also {}", diff.case);
            }
        }
        out
    }
}

fn indent(text: &str) -> String {
    if text.is_empty() {
        return "      ⟨empty⟩".to_owned();
    }
    text.lines()
        .map(|line| format!("      │{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The lines in which `before` and `after` differ, as `-`/`+` lines of a longest-common-
/// subsequence diff (each hunk prefixed by `@ line N`); empty when they are equal.
pub fn line_diff(before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }
    let a: Vec<&str> = before.split_inclusive('\n').collect();
    let b: Vec<&str> = after.split_inclusive('\n').collect();
    // Common ends first, so the table covers only the middle.
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (ma, mb) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut out = String::new();
    if ma.len() * mb.len() > 25_000_000 {
        let _ = writeln!(out, "@ line {}", prefix + 1);
        for line in ma {
            let _ = writeln!(out, "-{}", visible(line));
        }
        for line in mb {
            let _ = writeln!(out, "+{}", visible(line));
        }
        return out;
    }
    let (n, m) = (ma.len(), mb.len());
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * (m + 1) + j] = if ma[i] == mb[j] {
                table[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                table[(i + 1) * (m + 1) + j].max(table[i * (m + 1) + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut in_hunk = false;
    while i < n || j < m {
        if i < n && j < m && ma[i] == mb[j] {
            i += 1;
            j += 1;
            in_hunk = false;
            continue;
        }
        if !in_hunk {
            let _ = writeln!(out, "@ line {}", prefix + i + 1);
            in_hunk = true;
        }
        if j < m && (i == n || table[i * (m + 1) + j + 1] >= table[(i + 1) * (m + 1) + j]) {
            let _ = writeln!(out, "+{}", visible(mb[j]));
            j += 1;
        } else {
            let _ = writeln!(out, "-{}", visible(ma[i]));
            i += 1;
        }
    }
    out
}

/// A line with its ending and any trailing whitespace made visible.
fn visible(line: &str) -> String {
    let body = line.trim_end_matches('\n');
    let ending = if line.ends_with('\n') { "" } else { "⟨no newline⟩" };
    format!("{body}{ending}")
}

/// A short class for an emitted difference: the changed lines with every run of word
/// characters reduced to `w`, so the same cause over different values lands in one class.
pub fn shape(diff: &str) -> String {
    let mut out = String::new();
    for line in diff.lines().filter(|l| !l.starts_with('@')).take(6) {
        let mut previous_word = false;
        for c in line.chars() {
            let word = c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/';
            if word {
                if !previous_word {
                    out.push('w');
                }
            } else {
                out.push(c);
            }
            previous_word = word;
        }
        out.push_str(" ⏎ ");
    }
    out
}
