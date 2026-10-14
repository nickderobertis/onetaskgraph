//! The shape of the write context this source hands the store's cache, held to the version
//! every context carries.
//!
//! A context is kept on a host and read back by whichever release runs next, which takes one of
//! its own version as one it can read. A change to what a context holds that keeps the version
//! would have a release misread another's — so each version is recorded beside a digest of the
//! shapes it publishes, append-only, and a shape changed without a new row fails here.

use onetaskgraph_github_projects::{CONTEXT_VERSION, WHOLE_MEMBERS, context_shapes, graphql};

/// Every context version with the digest of the task and board context shapes it publishes.
const PUBLISHED_CONTEXTS: &[(u64, u64)] = &[(1, 0x8eab_f0e5_96fd_b4b8), (2, 0x1d08_0c96_c984_edd5)];

/// FNV-1a over `value`'s JSON, whose object keys serde_json keeps sorted.
fn digest(value: &serde_json::Value) -> u64 {
    serde_json::to_string(value)
        .expect("a schema renders")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

#[test]
fn the_context_version_moves_whenever_the_context_shape_does() {
    for (index, (version, recorded)) in PUBLISHED_CONTEXTS.iter().enumerate() {
        assert_eq!(*version, index as u64 + 1, "rows are the versions in order");
        assert!(
            !PUBLISHED_CONTEXTS[..index]
                .iter()
                .any(|(_, earlier)| earlier == recorded),
            "version {version} republishes an earlier shape"
        );
    }
    let (version, recorded) = PUBLISHED_CONTEXTS.last().expect("one row at least");
    assert_eq!(
        *version, CONTEXT_VERSION,
        "append a row for CONTEXT_VERSION"
    );
    let emitted = digest(&context_shapes());
    assert_eq!(
        emitted,
        *recorded,
        "the context shape changed: bump CONTEXT_VERSION and append ({}, {emitted:#018x}), or \
         put the shape back",
        CONTEXT_VERSION + 1
    );
}

/// The members the issue fragment every read of an issue selects names at its top level, each
/// by the key its answer carries — an alias where it gives one.
fn fragment_members() -> Vec<String> {
    let document = graphql::ISSUE_DETAIL;
    let fragment = &document[document
        .find("fragment BoardIssue on Issue{")
        .expect("the fragment")
        + "fragment BoardIssue on Issue{".len()..];
    let mut members = Vec::new();
    let (mut depth, mut word) = (0_i32, String::new());
    let take = |word: &mut String, members: &mut Vec<String>| {
        if !word.is_empty() {
            members.push(word.split(':').next().unwrap_or_default().to_owned());
            word.clear();
        }
    };
    let mut parens = 0_i32;
    for character in fragment.chars() {
        match character {
            '(' => parens += 1,
            ')' => parens -= 1,
            _ if parens > 0 => {}
            '{' => {
                if depth == 0 {
                    take(&mut word, &mut members);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    take(&mut word, &mut members);
                    break;
                }
                depth -= 1;
            }
            character if character.is_whitespace() => {
                if depth == 0 {
                    take(&mut word, &mut members);
                }
            }
            character if depth == 0 => word.push(character),
            _ => {}
        }
    }
    members
}

#[test]
fn a_response_is_whole_only_when_it_carries_every_member_the_issue_fragment_selects() {
    let members = fragment_members();
    assert!(members.contains(&"labels".to_owned()), "{members:?}");
    // The members the change token is read from are checked where the token is: a response
    // without any of them has no version, and so no whole task.
    let token = [
        "commentCount",
        "relationshipEvents",
        "updatedAt",
        "projectItems",
    ];
    // The reader decides on this before any member, and a task reports nothing from it.
    let unread = ["__typename"];
    let required: Vec<&str> = WHOLE_MEMBERS
        .iter()
        .map(|member| {
            member
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or_default()
        })
        .collect();
    for member in &members {
        assert!(
            required.contains(&member.as_str())
                || token.contains(&member.as_str())
                || unread.contains(&member.as_str()),
            "the issue fragment selects {member}, which WHOLE_MEMBERS does not require"
        );
    }
    for member in required {
        assert!(
            member == "comments" || members.iter().any(|selected| selected == member),
            "WHOLE_MEMBERS requires {member}, which the fragment does not select"
        );
    }
}
