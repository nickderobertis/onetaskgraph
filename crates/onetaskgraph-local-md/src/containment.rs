//! Whether a path the filesystem resolved lies inside the configured root.
//!
//! The root is resolved once, when the source is built, and every path held to it is resolved
//! later — and Windows does not promise to spell one directory the same way twice. The same
//! folder can come back with the verbatim `\\?\` prefix or without it, with a drive letter in
//! either case, with a component in another case, or with an 8.3 short name such as
//! `RUNNER~1` where the root was resolved with the long one: the hosted Windows runner's
//! temporary directory is spelled that way, and a folder resolved while another process
//! renames a record inside it has been answered with a spelling that is not the root's
//! (onetaskgraph's Linear concurrent-copy journey, refused with `issue-1.md escapes
//! configured root` naming a file inside that very root). A component-wise prefix test calls
//! every one of those an escape.
//!
//! So a path is inside when it is spelled under the root, when it is once the spellings that
//! are the same path by definition are made one — the verbatim prefix and the drive letter's
//! case — or when one of its own ancestors *is* the root's directory, which the filesystem
//! answers by identity rather than by name. The last covers every other spelling, whatever
//! Windows gives, and refuses exactly what lies outside: a directory that is not the root has
//! another identity, and a spelling that names nothing has none. It is asked only when the
//! spelling has already disagreed, so a path spelled under the root costs no call at all.
//!
//! The identity question is kept apart from the calls that answer it, so that every spelling
//! the rule owes is driven wherever the suite runs and not on Windows alone; [`Identities`] is
//! that seam, as [`crate::unlinking::Filesystem`] is the probe's.

use std::fs;
use std::path::Path;

/// The call the containment rule is answered by.
pub(crate) trait Identities {
    /// What tells one directory from another, however it is spelled.
    type Identity: Eq;
    /// The identity of what `path` names, or `None` when it names nothing that will answer.
    fn identity(&self, path: &Path) -> Option<Self::Identity>;
}

/// Whether `path` lies inside `root` on this machine's own filesystem.
pub(crate) fn within(root: &Path, path: &Path) -> bool {
    inside(&Disk, root, path, cfg!(windows))
}

/// Whether `path` lies inside `root`, by the rule this module states, `windows` saying
/// whether the two are spelled by Windows's grammar.
pub(crate) fn inside<F: Identities>(
    filesystem: &F,
    root: &Path,
    path: &Path,
    windows: bool,
) -> bool {
    if path.starts_with(root) {
        return true;
    }
    if windows
        && let (Some(root), Some(path)) = (root.to_str(), path.to_str())
        && spelled_within(root, path)
    {
        return true;
    }
    let Some(root) = filesystem.identity(root) else {
        return false;
    };
    path.ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .any(|ancestor| filesystem.identity(ancestor).as_ref() == Some(&root))
}

/// Whether the Windows path `path` is spelled under `root` once the verbatim prefix is taken
/// off both and their drive letters are put in one case — the two differences that make two
/// spellings of one path by definition rather than by what the filesystem holds.
fn spelled_within(root: &str, path: &str) -> bool {
    let (root, path) = (plain(root), plain(path));
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|rest| rest.starts_with('\\'))
}

/// `spelling` without its verbatim prefix — `\\?\C:\…` as `C:\…`, `\\?\UNC\server\…` as
/// `\\server\…` — its drive letter upper-cased, and no trailing separator. A device or volume
/// spelling is left as written.
fn plain(spelling: &str) -> String {
    let mut plain = if let Some(share) = spelling.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else {
        match spelling.strip_prefix(r"\\?\") {
            Some(disk) if drive(disk) => disk.to_owned(),
            _ => spelling.to_owned(),
        }
    };
    if drive(&plain) {
        plain[..1].make_ascii_uppercase();
    }
    while plain.ends_with('\\') {
        plain.pop();
    }
    plain
}

/// Whether `spelling` starts with a drive letter and its colon.
fn drive(spelling: &str) -> bool {
    let bytes = spelling.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// This machine's filesystem.
struct Disk;

#[cfg(unix)]
impl Identities for Disk {
    type Identity = (u64, u64);

    fn identity(&self, path: &Path) -> Option<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).ok()?;
        Some((metadata.dev(), metadata.ino()))
    }
}

#[cfg(windows)]
impl Identities for Disk {
    type Identity = (u32, u64);

    /// The volume serial number and file index, read through a handle opened with backup
    /// semantics — which is what lets a directory be opened at all — and every share mode, so
    /// this never blocks a writer it is racing.
    fn identity(&self, path: &Path) -> Option<(u32, u64)> {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;

        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
        };

        let file = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .ok()?;
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: `file` is an open handle that outlives the call, and `information` is the
        // call's one out-parameter, owned here.
        let read = unsafe {
            GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &raw mut information)
        };
        (read != 0).then(|| {
            (
                information.dwVolumeSerialNumber,
                (u64::from(information.nFileIndexHigh) << 32)
                    | u64::from(information.nFileIndexLow),
            )
        })
    }
}

/// A platform with no identity this source reads: a spelling that disagrees is outside.
#[cfg(not(any(unix, windows)))]
impl Identities for Disk {
    type Identity = ();

    fn identity(&self, _: &Path) -> Option<()> {
        None
    }
}

#[cfg(test)]
mod tests {
    //! The rule over the spellings Windows gives one directory. The verbatim prefix and the
    //! drive letter are a matter of grammar and are put to it as Windows spells them; a
    //! component's case and an 8.3 short name are a matter of what the filesystem holds, so
    //! they are put to a stand-in that answers identity as NTFS does — names compared without
    //! regard to case, and a short name the alias of its long one — over paths this host's
    //! own grammar can take apart.

    use std::collections::HashMap;
    use std::path::Path;

    use super::{Identities, inside, spelled_within};

    /// Directories by their long, canonically cased spelling, each with its identity, and the
    /// 8.3 aliases that name the same directory.
    struct Ntfs {
        directories: HashMap<String, u32>,
        aliases: HashMap<String, String>,
    }

    impl Ntfs {
        /// The runner's temporary directory and what a test makes in it: the configured root
        /// `notes`, its `tasks` folder, and a sibling outside the root.
        fn runner() -> Self {
            let directories = [
                "c:/users",
                "c:/users/runneradmin",
                "c:/users/runneradmin/temp",
                "c:/users/runneradmin/temp/project",
                "c:/users/runneradmin/temp/project/notes",
                "c:/users/runneradmin/temp/project/notes/tasks",
                "c:/users/runneradmin/temp/project/notes-elsewhere",
                "c:/users/runneradmin/temp/project/notes-elsewhere/tasks",
            ];
            Self {
                directories: directories
                    .iter()
                    .zip(1..)
                    .map(|(path, id)| ((*path).to_owned(), id))
                    .collect(),
                aliases: HashMap::from([("runner~1".to_owned(), "runneradmin".to_owned())]),
            }
        }
    }

    impl Identities for Ntfs {
        type Identity = u32;

        fn identity(&self, path: &Path) -> Option<u32> {
            let long = path
                .to_str()?
                .to_lowercase()
                .split('/')
                .map(|segment| self.aliases.get(segment).map_or(segment, String::as_str))
                .collect::<Vec<_>>()
                .join("/");
            self.directories.get(&long).copied()
        }
    }

    const ROOT: &str = "C:/Users/runneradmin/Temp/project/notes";

    fn held(path: &str) -> bool {
        inside(&Ntfs::runner(), Path::new(ROOT), Path::new(path), false)
    }

    #[test]
    fn a_folder_spelled_with_the_short_name_of_an_ancestor_is_inside() {
        assert!(held("C:/Users/RUNNER~1/Temp/project/notes/tasks"));
        assert!(held(
            "C:/Users/RUNNER~1/Temp/project/notes/tasks/issue-1.md"
        ));
        assert!(held("C:/Users/RUNNER~1/Temp/project/notes"));
    }

    #[test]
    fn a_folder_spelled_in_another_case_is_inside() {
        assert!(held("c:/users/RunnerAdmin/TEMP/Project/Notes/Tasks"));
        assert!(held(
            "C:/USERS/RUNNERADMIN/TEMP/PROJECT/NOTES/tasks/issue-1.md"
        ));
    }

    #[test]
    fn a_root_resolved_with_the_short_name_holds_a_folder_resolved_with_the_long_one() {
        let root = Path::new("C:/Users/RUNNER~1/Temp/project/notes");
        let path = Path::new("C:/Users/runneradmin/Temp/project/notes/tasks/issue-1.md");
        assert!(inside(&Ntfs::runner(), root, path, false));
    }

    #[test]
    fn a_sibling_is_outside_however_it_is_spelled() {
        assert!(!held(
            "C:/Users/runneradmin/Temp/project/notes-elsewhere/tasks"
        ));
        assert!(!held(
            "C:/Users/RUNNER~1/Temp/project/notes-elsewhere/tasks/issue-1.md"
        ));
        assert!(!held(
            "c:/users/RUNNERADMIN/temp/project/NOTES-ELSEWHERE/tasks"
        ));
        assert!(!held("C:/Users/runneradmin/Temp/project"));
    }

    #[test]
    fn a_spelling_that_names_nothing_is_outside() {
        assert!(!held("C:/Users/RUNNER~2/Temp/project/notes/tasks"));
        assert!(!held("Z:/notes/tasks"));
        let gone = Path::new("C:/Users/runneradmin/Temp/gone");
        let path = Path::new("C:/Users/RUNNER~1/Temp/gone/tasks/issue-1.md");
        assert!(!inside(&Ntfs::runner(), gone, path, false));
    }

    const VERBATIM_ROOT: &str =
        r"\\?\C:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes";

    #[test]
    fn a_path_with_or_without_the_verbatim_prefix_is_inside() {
        for (root, path) in [
            (
                VERBATIM_ROOT,
                r"C:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes\tasks\issue-1.md",
            ),
            (
                r"C:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes",
                r"\\?\C:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes\tasks",
            ),
            (
                VERBATIM_ROOT,
                r"\\?\c:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes\tasks",
            ),
            (
                VERBATIM_ROOT,
                r"c:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes",
            ),
            (
                r"\\?\UNC\server\share\notes",
                r"\\server\share\notes\tasks\issue-1.md",
            ),
            (
                r"\\server\share\notes\",
                r"\\?\UNC\server\share\notes\tasks",
            ),
            (r"\\?\C:\", r"C:\notes"),
        ] {
            assert!(spelled_within(root, path), "{path} under {root}");
        }
    }

    #[test]
    fn a_path_outside_the_root_is_outside_with_or_without_the_verbatim_prefix() {
        for (root, path) in [
            (
                VERBATIM_ROOT,
                r"C:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes-elsewhere\tasks",
            ),
            (
                VERBATIM_ROOT,
                r"\\?\C:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project",
            ),
            (
                VERBATIM_ROOT,
                r"\\?\D:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes\tasks",
            ),
            (r"\\?\UNC\server\share\notes", r"\\other\share\notes\tasks"),
            (
                r"\\?\UNC\server\share\notes",
                r"C:\server\share\notes\tasks",
            ),
            (r"\\?\Volume{1}\notes", r"\\?\Volume{2}\notes\tasks"),
        ] {
            assert!(!spelled_within(root, path), "{path} under {root}");
        }
    }

    #[test]
    fn grammar_alone_decides_only_where_windows_spells_the_paths() {
        let filesystem = Ntfs::runner();
        let root = Path::new(VERBATIM_ROOT);
        let path = Path::new(r"C:\Users\runneradmin\AppData\Local\Temp\.tmpXXXX\project\notes\x");
        assert!(inside(&filesystem, root, path, true));
        // A backslash is an ordinary character in a name elsewhere, so the same two strings
        // name two unrelated files there.
        assert!(!inside(&filesystem, root, path, false));
    }

    /// On this machine's own filesystem: a second spelling of the root the host can produce —
    /// through a link to it — holds what is inside, and a sibling stays outside.
    #[cfg(unix)]
    #[test]
    fn this_filesystem_answers_identity_for_another_spelling_of_the_root() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("notes");
        let sibling = scratch.path().join("notes-elsewhere");
        std::fs::create_dir_all(root.join("tasks")).unwrap();
        std::fs::create_dir_all(sibling.join("tasks")).unwrap();
        let alias = scratch.path().join("RUNNER~1");
        std::os::unix::fs::symlink(scratch.path(), &alias).unwrap();
        use super::within;

        let spelled = alias.join("notes/tasks");
        assert!(within(&root, &spelled));
        assert!(within(&root, &spelled.join("issue-1.md")));
        assert!(!within(&root, &alias.join("notes-elsewhere/tasks")));
        assert!(!within(&root, &alias.join("missing/tasks")));
    }
}
