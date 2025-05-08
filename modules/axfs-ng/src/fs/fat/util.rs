use core::time::Duration;

use alloc::string::String;
use axfs_ng_vfs::{Metadata, NodePermission, NodeType, VfsError};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use super::ff;

#[derive(Clone)]
pub struct CaseInsensitiveString(pub String);
impl PartialEq for CaseInsensitiveString {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .bytes()
            .map(|c| c.to_ascii_lowercase())
            .eq(other.0.bytes().map(|c| c.to_ascii_lowercase()))
    }
}
impl Eq for CaseInsensitiveString {}
impl PartialOrd for CaseInsensitiveString {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CaseInsensitiveString {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0
            .bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(other.0.bytes().map(|c| c.to_ascii_lowercase()))
    }
}

pub fn dos_to_unix(date: fatfs::DateTime) -> Duration {
    // let date: NaiveDateTime = date.into();
    let date = NaiveDate::from_ymd_opt(
        date.date.year as _,
        date.date.month as _,
        date.date.day as _,
    )
    .unwrap()
    .and_hms_milli_opt(
        date.time.hour as _,
        date.time.min as _,
        date.time.sec as _,
        date.time.millis as _,
    )
    .unwrap();
    let Some(datetime) = Utc.from_local_datetime(&date).single() else {
        return Duration::default();
    };
    datetime
        .signed_duration_since(DateTime::UNIX_EPOCH)
        .to_std()
        .unwrap_or_default()
}

pub fn file_metadata(file: &ff::File, node_type: NodeType) -> Metadata {
    let size = file.size().unwrap_or(0) as u64;
    Metadata {
        // TODO: inode
        inode: 1,
        device: 0,
        nlink: 1,
        mode: NodePermission::default(),
        node_type,
        uid: 0,
        gid: 0,
        size,
        block_size: 512,
        // TODO: The correct block count should be obtained from
        // `file.extents()`. However it would be costly. This implementation
        // would be enough for now.
        blocks: size / 512,
        atime: dos_to_unix(fatfs::DateTime::new(
            file.accessed(),
            fatfs::Time::new(0, 0, 0, 0),
        )),
        mtime: dos_to_unix(file.modified()),
        ctime: dos_to_unix(file.created()),
    }
}

pub fn into_vfs_err<E>(err: fatfs::Error<E>) -> VfsError {
    use fatfs::Error::*;
    match err {
        AlreadyExists => VfsError::AlreadyExists,
        CorruptedFileSystem => VfsError::InvalidData,
        DirectoryIsNotEmpty => VfsError::DirectoryNotEmpty,
        // TODO: should return ENAMETOOLONG
        InvalidFileNameLength => VfsError::InvalidData,
        InvalidInput | UnsupportedFileNameCharacter => VfsError::InvalidData,
        NotEnoughSpace => VfsError::StorageFull,
        NotFound => VfsError::NotFound,
        UnexpectedEof | WriteZero => VfsError::Io,
        _ => VfsError::Io,
    }
}
