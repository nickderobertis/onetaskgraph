//! An issue reference, `owner/name#N`: the one spelling a landing's `closes` and the command
//! line share, read by `IssueRef`'s `FromStr` and published to both SDKs as
//! `ISSUE_REF_PATTERN`. The two are one rule in two places, so a matcher built from the
//! constant is held to the parser over a corpus here.

use std::collections::BTreeSet;

use onetaskgraph_plugin_api::{ISSUE_REF_PATTERN, IssueRef};

/// Every character one `[...]` class of the published pattern admits.
fn expand_class(class: &str) -> BTreeSet<char> {
    let chars: Vec<char> = class.chars().collect();
    let mut admitted = BTreeSet::new();
    let mut at = 0;
    while at < chars.len() {
        if at + 2 < chars.len() && chars[at + 1] == '-' {
            for c in chars[at]..=chars[at + 2] {
                admitted.insert(c);
            }
            at += 3;
        } else {
            admitted.insert(chars[at]);
            at += 1;
        }
    }
    admitted
}

/// Match `value` against `^[owner]+/[name]+#[1-9][0-9]{0,N}$`, read out of the published
/// constant rather than restated — this cannot agree with a pattern it did not read.
fn matches_published_pattern(value: &str) -> bool {
    let body = ISSUE_REF_PATTERN
        .strip_prefix("^[")
        .and_then(|rest| rest.strip_suffix('$'))
        .expect("the pattern is anchored and opens with a class");
    let (owner, rest) = body
        .split_once("]+/[")
        .expect("an owner class, then a slash");
    let (name, rest) = rest.split_once("]+#").expect("a name class, then a hash");
    let bound = rest
        .strip_prefix("[1-9][0-9]{0,")
        .and_then(|rest| rest.strip_suffix('}'))
        .and_then(|bound| bound.parse::<usize>().ok())
        .expect("a number with no leading zero and a bounded tail");
    let (owner, name) = (expand_class(owner), expand_class(name));
    let Some((repository, number)) = value.split_once('#') else {
        return false;
    };
    let Some((owner_part, name_part)) = repository.split_once('/') else {
        return false;
    };
    !owner_part.is_empty()
        && owner_part.chars().all(|c| owner.contains(&c))
        && !name_part.is_empty()
        && name_part.chars().all(|c| name.contains(&c))
        && number
            .chars()
            .next()
            .is_some_and(|c| ('1'..='9').contains(&c))
        && number.len() <= bound + 1
        && number.chars().all(|c| c.is_ascii_digit())
}

#[test]
fn the_parser_agrees_with_the_pattern_it_publishes() {
    let mut corpus: Vec<String> = vec![
        String::new(),
        "acme/work#12".to_owned(),
        "Acme-Org/work.site_2#1".to_owned(),
        "acme/work#0".to_owned(),
        "acme/work#012".to_owned(),
        "acme/work#".to_owned(),
        "acme/work".to_owned(),
        "work#3".to_owned(),
        "/work#3".to_owned(),
        "acme/#3".to_owned(),
        "acme/work/extra#3".to_owned(),
        "acme/work#3#4".to_owned(),
        "acme/work#-3".to_owned(),
        "acme/work#3 ".to_owned(),
        "acme/work#9999999999999999999".to_owned(),
        "acme/work#18446744073709551615".to_owned(),
        "acme/work#99999999999999999999".to_owned(),
        format!("acme/work#1{}", "0".repeat(18)),
        format!("acme/work#1{}", "0".repeat(19)),
    ];
    for byte in 0u8..=127 {
        let c = char::from(byte);
        corpus.push(format!("ac{c}me/work#7"));
        corpus.push(format!("acme/wo{c}rk#7"));
        corpus.push(format!("acme/work#7{c}"));
    }
    for value in corpus {
        assert_eq!(
            value.parse::<IssueRef>().is_ok(),
            matches_published_pattern(&value),
            "IssueRef's FromStr and ISSUE_REF_PATTERN ({ISSUE_REF_PATTERN}) disagree about \
             {value:?}. They are one rule in two places — change both together."
        );
    }
}

#[test]
fn a_reference_reads_back_as_it_was_spelled_and_compares_ignoring_case() {
    let read: IssueRef = "Acme/Work#12".parse().expect("a reference");
    assert_eq!(read.to_string(), "Acme/Work#12");
    assert_eq!(
        read,
        "acme/work#12".parse::<IssueRef>().expect("a reference")
    );
    assert_ne!(
        read,
        "acme/work#13".parse::<IssueRef>().expect("a reference")
    );
    assert_eq!(
        serde_json::to_value(&read).expect("serialises"),
        serde_json::json!("Acme/Work#12")
    );
    let refused = "acme/work-12"
        .parse::<IssueRef>()
        .expect_err("not a reference");
    assert!(
        refused.contains("\"acme/work-12\" is not an issue reference"),
        "{refused}"
    );
}
