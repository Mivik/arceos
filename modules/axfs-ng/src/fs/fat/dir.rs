use core::{any::Any, mem, ops::Deref, time::Duration};

use alloc::{string::String, sync::Arc};
use axfs_ng_vfs::{
    DirEntry, DirEntryVisitor, DirNode, DirNodeOps, FilesystemOps, Metadata, NodeOps,
    NodePermission, NodeType, Reference, VfsError, VfsResult, WeakDirEntry,
};
use lock_api::RawMutex;

use super::{
    FsRef, ff,
    file::FatFileNode,
    fs::FatFilesystem,
    util::{file_metadata, into_vfs_err},
};

pub struct FatDirNode<M> {
    fs: Arc<FatFilesystem<M>>,
    pub(crate) inner: FsRef<ff::Dir<'static>>,
    this: WeakDirEntry<M>,
}
impl<M: RawMutex + 'static> FatDirNode<M> {
    pub fn new(fs: Arc<FatFilesystem<M>>, dir: ff::Dir, this: WeakDirEntry<M>) -> DirNode<M> {
        DirNode::new(Arc::new(Self {
            fs,
            // SAFETY: FsRef guarantees correct lifetime
            inner: FsRef::new(unsafe { mem::transmute(dir) }),
            this,
        }))
    }

    fn create_entry(&self, entry: ff::DirEntry, name: impl Into<String>) -> DirEntry<M> {
        let reference = Reference::new(Some(self.this.clone()), name.into());
        if entry.is_file() {
            DirEntry::new_file(
                FatFileNode::new(self.fs.clone(), entry.to_file()),
                NodeType::RegularFile,
                reference,
            )
        } else {
            DirEntry::new_dir(
                |this| FatDirNode::new(self.fs.clone(), entry.to_dir(), this),
                reference,
            )
        }
    }
}

unsafe impl<M> Send for FatDirNode<M> {}
unsafe impl<M> Sync for FatDirNode<M> {}

impl<M: RawMutex + 'static> NodeOps<M> for FatDirNode<M> {
    fn inode(&self) -> u64 {
        // TODO: implement this
        1
    }

    /// Get the metadata of the file.
    fn metadata(&self) -> VfsResult<Metadata> {
        let fs = self.fs.lock();
        let dir = self.inner.borrow(&fs);
        if let Some(file) = dir.as_file() {
            return Ok(file_metadata(file, NodeType::Directory));
        }

        // root directory
        let block_size = fs.inner.bytes_per_sector() as u64;
        Ok(Metadata {
            // TODO: inode
            inode: self.inode(),
            device: 0,
            nlink: 1,
            mode: NodePermission::default(),
            node_type: NodeType::Directory,
            uid: 0,
            gid: 0,
            size: block_size,
            block_size: block_size,
            blocks: 1,
            atime: Duration::default(),
            mtime: Duration::default(),
            ctime: Duration::default(),
        })
    }

    fn filesystem(&self) -> &dyn FilesystemOps<M> {
        self.fs.deref()
    }

    fn sync(&self, _data_only: bool) -> VfsResult<()> {
        Ok(())
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}
impl<M: RawMutex + 'static> DirNodeOps<M> for FatDirNode<M> {
    fn read_dir(&self, offset: u64, mut visitor: DirEntryVisitor<'_, M>) -> VfsResult<usize> {
        let fs = self.fs.lock();
        let dir = self.inner.borrow(&fs);
        let mut count = 0;
        for entry in dir.iter().skip(offset as usize) {
            let entry = entry.map_err(into_vfs_err)?;
            let name = entry.file_name().to_ascii_lowercase();
            if !visitor.accept_with(name, offset + count + 1, |name| self.create_entry(entry, name)) {
                break;
            }
            count += 1;
        }
        Ok(count as usize)
    }

    fn lookup(&self, name: &str) -> VfsResult<DirEntry<M>> {
        let fs = self.fs.lock();
        let dir = self.inner.borrow(&fs);
        dir.iter()
            .find_map(|entry| entry.ok().filter(|it| it.eq_name(name)))
            .map(|entry| self.create_entry(entry, name.to_ascii_lowercase()))
            .ok_or(VfsError::NotFound)
    }

    fn create(
        &self,
        name: &str,
        node_type: NodeType,
        _permission: NodePermission,
    ) -> VfsResult<DirEntry<M>> {
        let fs = self.fs.lock();
        let dir = self.inner.borrow(&fs);
        let reference = Reference::new(Some(self.this.clone()), name.to_ascii_lowercase());
        match node_type {
            NodeType::RegularFile => dir
                .create_file(name)
                .map(|file| {
                    DirEntry::new_file(
                        FatFileNode::new(self.fs.clone(), file),
                        NodeType::RegularFile,
                        reference,
                    )
                })
                .map_err(into_vfs_err),
            NodeType::Directory => dir
                .create_dir(name)
                .map(|dir| {
                    DirEntry::new_dir(
                        |this| FatDirNode::new(self.fs.clone(), dir, this),
                        reference,
                    )
                })
                .map_err(into_vfs_err),
            _ => Err(VfsError::InvalidInput),
        }
    }

    fn link(&self, _name: &str, _node: &DirEntry<M>) -> VfsResult<DirEntry<M>> {
        //  EPERM  The filesystem containing oldpath and newpath does not
        //         support the creation of hard links.
        Err(VfsError::PermissionDenied)
    }

    fn unlink(&self, name: &str) -> VfsResult<()> {
        let fs = self.fs.lock();
        let dir = self.inner.borrow(&fs);
        dir.remove(name).map_err(into_vfs_err)
    }

    fn rename(&self, src_name: &str, dst_dir: &DirNode<M>, dst_name: &str) -> VfsResult<()> {
        let fs = self.fs.lock();
        let dst_dir: Arc<Self> = dst_dir.downcast().map_err(|_| VfsError::InvalidInput)?;

        let dir = self.inner.borrow(&fs);

        // The default implementation throws EEXIST if dst exists, so we need to
        // handle it
        match dst_dir.inner.borrow(&fs).remove(dst_name) {
            Ok(_) => {}
            Err(fatfs::Error::NotFound) => {}
            Err(err) => return Err(into_vfs_err(err)),
        }

        dir.rename(src_name, dst_dir.inner.borrow(&fs), dst_name)
            .map_err(into_vfs_err)
    }
}
