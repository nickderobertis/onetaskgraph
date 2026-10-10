//! A task's change token in a folder of Markdown: what
//! [`Task::version`](onetaskgraph_plugin_api::Task::version) holds here.
//!
//! The SHA-256 of the task file's bytes. Everything a task is — its front matter, its body and
//! its comments section — is in that one file, so any change to any of them changes the
//! token, and a token read twice off an untouched file is the same. A digest says whether two
//! versions are the same and nothing about which came first, so two different ones are not
//! ordered.

use std::cmp::Ordering;

/// What every token this plugin gives starts with, so a reader can tell it from another
/// plugin's.
pub const PREFIX: &str = "md1|";

/// The token of a task file holding exactly `bytes`.
#[must_use]
pub fn spell(bytes: &[u8]) -> String {
    format!("{PREFIX}{}", onetaskgraph_plugin_api::asset_sha256(bytes))
}

/// `Equal` when `a` and `b` are one token of this plugin, and `None` otherwise: two different
/// digests say the file changed, never which version is the later.
#[must_use]
pub fn order(a: &str, b: &str) -> Option<Ordering> {
    let digest = |token: &str| {
        token
            .strip_prefix(PREFIX)
            .filter(|hex| onetaskgraph_plugin_api::is_sha256(hex))
            .map(str::to_owned)
    };
    (digest(a)? == digest(b)?).then_some(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_file_is_one_token_and_two_files_are_not_ordered() {
        let a = spell(b"---\ntitle: a\n---\n");
        assert_eq!(order(&a, &a), Some(Ordering::Equal));
        assert_eq!(order(&a, &spell(b"---\ntitle: b\n---\n")), None);
        assert_eq!(order("md1|zz", "md1|zz"), None);
        assert_eq!(order("gh1|x", &a), None);
    }
}
