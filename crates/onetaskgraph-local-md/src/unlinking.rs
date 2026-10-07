//! How the Windows probe reads what the filesystem tells it into one answer: has this entry
//! been unlinked, or is it still there?
//!
//! The probe opens the entry by name relative to an open handle on its folder, which is what
//! saves it from spelling an NT object path. That leaves one question the entry's own answer
//! cannot settle: the folder itself will not open. Windows refuses to open a folder that is
//! marked for deletion with the same *access is denied* it gives a folder this reader may not
//! have, and a folder already gone does not open either — and a walk racing a deletion meets
//! exactly that, a task vanishing with the folder it was in. So a folder that will not open is
//! put the same question in turn, by name in *its* folder: the entry is unlinked when the
//! folder is, and still there when the folder is (onetaskgraph#3044).
//!
//! The classification is kept apart from the calls that answer it so that every case it owes
//! is driven wherever the suite runs, not on Windows alone; [`Filesystem`] is that seam.

use std::ffi::OsStr;
use std::path::Path;

/// What opening an entry by name in an open folder answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opened {
    /// The entry opened.
    Yes,
    /// The entry is marked for deletion and waiting on a handle that still holds it.
    DeletePending,
    /// The folder no longer names the entry.
    NotFound,
    /// The entry is there and the open was refused for a reason of its own — an
    /// access-control entry that denies this reader, among others.
    Refused,
}

/// The two calls the classification is answered by.
pub(crate) trait Filesystem {
    /// An open handle on a folder.
    type Folder;
    /// Open the folder at `path`, or `None` when it will not open, for whatever reason.
    fn open_folder(&self, path: &Path) -> Option<Self::Folder>;
    /// Open the entry `name` relative to `folder`.
    fn open_entry(&self, folder: &Self::Folder, name: &OsStr) -> Opened;
}

/// Whether `filesystem` says the entry at `path` has been unlinked: marked for deletion,
/// gone from its folder, or in a folder that is itself unlinked.
///
/// A question this cannot put — a path with no folder or no name, as a filesystem root has
/// neither — is answered `false`, which is the read path reporting the failure it already had
/// rather than passing a record over on a probe that never ran. So is a folder that will not
/// open while it is still there: a folder this reader may not have is the author's to mend.
pub(crate) fn unlinked<F: Filesystem>(filesystem: &F, path: &Path) -> bool {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let Some(open) = filesystem.open_folder(folder) else {
        return unlinked(filesystem, folder);
    };
    matches!(
        filesystem.open_entry(&open, name),
        Opened::DeletePending | Opened::NotFound
    )
}

#[cfg(test)]
mod tests {
    //! The classification over a stand-in filesystem that behaves as Windows does: a folder
    //! that is missing, marked for deletion or denied to this reader will not open, nor will
    //! one whose own folder will not, and an entry answers by its own state.

    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use super::{Filesystem, Opened, unlinked};

    #[derive(Clone, Copy)]
    enum State {
        DeletePending,
        Denied,
    }

    /// Every path is present unless named here, or missing when `missing` names it or a folder
    /// above it.
    #[derive(Default)]
    struct StandIn {
        states: HashMap<PathBuf, State>,
        missing: Vec<PathBuf>,
    }

    impl StandIn {
        fn missing(&self, path: &Path) -> bool {
            self.missing.iter().any(|gone| path.starts_with(gone))
        }

        fn state(&self, path: &Path) -> Option<State> {
            self.states.get(path).copied()
        }
    }

    impl Filesystem for StandIn {
        type Folder = PathBuf;

        fn open_folder(&self, path: &Path) -> Option<PathBuf> {
            let opens = path
                .ancestors()
                .all(|at| !self.missing(at) && self.state(at).is_none());
            opens.then(|| path.to_path_buf())
        }

        fn open_entry(&self, folder: &PathBuf, name: &OsStr) -> Opened {
            let path = folder.join(name);
            if self.missing(&path) {
                return Opened::NotFound;
            }
            match self.state(&path) {
                None => Opened::Yes,
                Some(State::DeletePending) => Opened::DeletePending,
                Some(State::Denied) => Opened::Refused,
            }
        }
    }

    const ENTRY: &str = "/notes/tasks/theirs/deep-2/39.md";
    const PARENT: &str = "/notes/tasks/theirs/deep-2";

    fn with(state: State, at: &str) -> StandIn {
        StandIn {
            states: HashMap::from([(PathBuf::from(at), state)]),
            ..StandIn::default()
        }
    }

    fn without(at: &str) -> StandIn {
        StandIn {
            missing: vec![PathBuf::from(at)],
            ..StandIn::default()
        }
    }

    #[test]
    fn an_entry_whose_folder_has_gone_is_unlinked() {
        assert!(unlinked(&without(PARENT), Path::new(ENTRY)));
        // And whose folder's folder has: the question climbs until a folder opens.
        assert!(unlinked(&without("/notes/tasks/theirs"), Path::new(ENTRY)));
    }

    #[test]
    fn an_entry_whose_folder_is_marked_for_deletion_is_unlinked() {
        assert!(unlinked(
            &with(State::DeletePending, PARENT),
            Path::new(ENTRY)
        ));
    }

    #[test]
    fn an_entry_that_is_there_but_refused_is_not_unlinked() {
        assert!(!unlinked(&with(State::Denied, ENTRY), Path::new(ENTRY)));
        // Nor is one in a folder this reader may not open, which is the author's to mend.
        assert!(!unlinked(&with(State::Denied, PARENT), Path::new(ENTRY)));
        // Nor one that opens.
        assert!(!unlinked(&StandIn::default(), Path::new(ENTRY)));
    }

    #[test]
    fn an_entry_marked_or_gone_in_a_folder_that_opens_is_unlinked() {
        assert!(unlinked(
            &with(State::DeletePending, ENTRY),
            Path::new(ENTRY)
        ));
        assert!(unlinked(&without(ENTRY), Path::new(ENTRY)));
    }

    #[test]
    fn a_path_with_no_folder_to_ask_in_is_not_unlinked() {
        assert!(!unlinked(&without("/"), Path::new("/")));
    }
}
