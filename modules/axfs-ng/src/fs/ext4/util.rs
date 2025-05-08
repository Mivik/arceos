use axerrno::LinuxError;
use axfs_ng_vfs::{NodeType, VfsError};
use lwext4_rust::{DummyHal, Ext4Error, InodeType};

use super::Ext4Disk;

pub type LwExt4Filesystem = lwext4_rust::Ext4Filesystem<DummyHal, Ext4Disk>;

pub fn into_vfs_err(err: Ext4Error) -> VfsError {
    let Ok(err) = LinuxError::try_from(err.code) else {
        return VfsError::Io;
    };
    match err {
        LinuxError::EADDRINUSE => VfsError::AddrInUse,
        LinuxError::EEXIST => VfsError::AlreadyExists,
        LinuxError::EFAULT => VfsError::BadAddress,
        LinuxError::ECONNREFUSED => VfsError::ConnectionRefused,
        LinuxError::ECONNRESET => VfsError::ConnectionReset,
        LinuxError::ENOTEMPTY => VfsError::DirectoryNotEmpty,
        LinuxError::EINVAL => VfsError::InvalidData,
        LinuxError::EIO => VfsError::Io,
        LinuxError::EISDIR => VfsError::IsADirectory,
        LinuxError::ENOMEM => VfsError::NoMemory,
        LinuxError::ENOTDIR => VfsError::NotADirectory,
        LinuxError::ENOTCONN => VfsError::NotConnected,
        LinuxError::ENOENT => VfsError::NotFound,
        LinuxError::EACCES => VfsError::PermissionDenied,
        LinuxError::EBUSY => VfsError::ResourceBusy,
        LinuxError::ENOSPC => VfsError::StorageFull,
        LinuxError::ENOSYS => VfsError::Unsupported,
        LinuxError::EAGAIN => VfsError::WouldBlock,
        _ => VfsError::Io,
    }
}

pub fn into_vfs_type(ty: InodeType) -> NodeType {
    match ty {
        InodeType::RegularFile => NodeType::RegularFile,
        InodeType::Directory => NodeType::Directory,
        InodeType::CharacterDevice => NodeType::CharacterDevice,
        InodeType::BlockDevice => NodeType::BlockDevice,
        InodeType::Fifo => NodeType::Fifo,
        InodeType::Socket => NodeType::Socket,
        InodeType::Symlink => NodeType::Symlink,
        InodeType::Unknown => NodeType::Unknown,
    }
}
