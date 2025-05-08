use core::{cell::OnceCell, marker::PhantomPinned};

use alloc::sync::Arc;
use axdriver::AxBlockDevice;
use axfs_ng_vfs::{DirEntry, Filesystem, FilesystemOps, Reference};
use lock_api::{Mutex, MutexGuard, RawMutex};

use crate::disk::SeekableDisk;

use super::{dir::FatDirNode, ff};

pub struct FatFilesystemInner {
    pub inner: ff::FileSystem,
    _pinned: PhantomPinned,
}

pub struct FatFilesystem<M> {
    inner: Mutex<M, FatFilesystemInner>,
    root_dir: OnceCell<DirEntry<M>>,
}

unsafe impl<M> Send for FatFilesystem<M> {}
unsafe impl<M> Sync for FatFilesystem<M> {}

impl<M: RawMutex + 'static> FatFilesystem<M> {
    pub fn new(dev: AxBlockDevice) -> Filesystem<M> {
        let inner = FatFilesystemInner {
            inner: ff::FileSystem::new(SeekableDisk::new(dev), fatfs::FsOptions::new())
                .expect("failed to initialize FAT filesystem"),
            _pinned: PhantomPinned,
        };
        let result = Arc::new(Self {
            inner: Mutex::new(inner),
            root_dir: OnceCell::new(),
        });

        let root_dir = DirEntry::new_dir(
            |this| FatDirNode::new(result.clone(), result.lock().inner.root_dir(), this),
            Reference::root(),
        );
        let _ = result.root_dir.set(root_dir);
        Filesystem::new(result)
    }

    pub(crate) fn lock(&self) -> MutexGuard<M, FatFilesystemInner> {
        self.inner.lock()
    }
}

impl<M: RawMutex> FilesystemOps<M> for FatFilesystem<M> {
    fn root_dir(&self) -> DirEntry<M> {
        self.root_dir.get().unwrap().clone()
    }
}
