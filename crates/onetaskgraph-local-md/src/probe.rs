//! The Windows probe that tells a file being deleted from one this reader may not open.
//!
//! Windows does not remove a file from its folder while another process still holds a
//! handle to it: `DeleteFile` marks it and the entry stays until the last handle closes.
//! Through that interval every open of it is refused with *access is denied* — the very
//! answer an access-control entry that denies this reader gives — and `std`'s metadata read
//! falls back to the folder listing, which still names the file. So nothing in `std` tells
//! the two apart, and a walk that guesses either way is wrong half the time: guess deleted
//! and a genuinely unreadable record is passed over in silence, guess denied and a record
//! that is merely being deleted is reported malformed.
//!
//! The NT layer does tell them apart. `NtOpenFile` answers `STATUS_DELETE_PENDING` for the
//! first and `STATUS_ACCESS_DENIED` for the second, and it checks for the pending deletion
//! before it checks access, so a file that is both answers that it is going. The entry is
//! opened by name relative to an open handle on its own folder, which is what saves this
//! from having to spell a DOS path as an NT object path — every form of one, including the
//! UNC and verbatim spellings a configured root may be given in.

use std::fs::OpenOptions;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{FILE_OPEN_REPARSE_POINT, NtOpenFile};
use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, OBJ_CASE_INSENSITIVE, STATUS_DELETE_PENDING, STATUS_OBJECT_NAME_NOT_FOUND,
    STATUS_OBJECT_PATH_NOT_FOUND, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

/// Whether the filesystem has already marked the entry at `path` for deletion.
///
/// A question this cannot put — a path with no folder to open it relative to, a folder that
/// will not open, a name that is not valid UTF-16 — is answered `false`, which is the read
/// path reporting the failure it already had rather than passing a record over on a probe
/// that never ran. An entry the probe finds is no longer there at all answers `true`: it is
/// gone, which is the same fate as going.
pub(crate) fn delete_pending(path: &Path) -> bool {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    // The folder handle is what the entry is named relative to, so nothing here spells an NT
    // object path. Backup semantics is what lets a directory be opened at all, and every
    // share mode is granted so this probe never itself blocks the deletion it is asking about.
    let Ok(folder) = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(folder)
    else {
        return false;
    };
    let mut wide: Vec<u16> = name.encode_wide().collect();
    let Ok(bytes) = u16::try_from(wide.len() * size_of::<u16>()) else {
        return false;
    };
    let name = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: folder.as_raw_handle() as HANDLE,
        ObjectName: &raw const name,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut opened: HANDLE = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    // SAFETY: `attributes` names `name`, which names `wide`, and all three outlive the call;
    // `opened` and `status_block` are the call's two out-parameters and are owned here. The
    // access asked for is what the read that failed wanted, so a denial is a denial of the
    // same thing; the entry itself is opened rather than anything it links to.
    let status = unsafe {
        NtOpenFile(
            &raw mut opened,
            FILE_READ_DATA | FILE_READ_ATTRIBUTES,
            &raw const attributes,
            &raw mut status_block,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN_REPARSE_POINT,
        )
    };
    if status >= 0 {
        // The entry opened, so it is neither going nor denied: the refusal this was asked
        // about is the caller's to report. Windows leaks a handle nobody closes.
        // SAFETY: `opened` is the handle the successful call just returned and is closed once.
        unsafe { CloseHandle(opened) };
        return false;
    }
    matches!(
        status,
        STATUS_DELETE_PENDING | STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND
    )
}
