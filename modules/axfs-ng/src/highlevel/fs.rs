use alloc::{
    borrow::{Cow, ToOwned},
    collections::vec_deque::VecDeque,
    string::String,
    vec::Vec,
};
use axio::{Read, Write};
use lock_api::RawMutex;

use axfs_ng_vfs::{
    Location, Metadata, NodePermission, NodeType, VfsError, VfsResult,
    path::{Component, Path, PathBuf},
};

use super::{File, FileFlags};

#[cfg(feature = "thread-local")]
axns::def_resource! {
    pub static FS_CONTEXT: axns::ResArc<axsync::Mutex<FsContext<axsync::RawMutex>>> = axns::ResArc::new();
}
#[cfg(feature = "thread-local")]
impl FS_CONTEXT {
    pub fn copy_inner(&self) -> axsync::Mutex<FsContext<axsync::RawMutex>> {
        axsync::Mutex::new(self.lock().clone())
    }
}

pub struct ReadDirEntry {
    pub name: String,
    pub ino: u64,
    pub node_type: NodeType,
    pub offset: u64,
}

/// Provides `std::fs`-like interface.
pub struct FsContext<M> {
    root_dir: Location<M>,
    current_dir: Location<M>,
}
impl<M> Clone for FsContext<M> {
    fn clone(&self) -> Self {
        Self {
            root_dir: self.root_dir.clone(),
            current_dir: self.current_dir.clone(),
        }
    }
}
impl<M: RawMutex> FsContext<M> {
    pub fn new(root_dir: Location<M>) -> Self {
        Self {
            root_dir: root_dir.clone(),
            current_dir: root_dir,
        }
    }

    pub fn root_dir(&self) -> &Location<M> {
        &self.root_dir
    }
    pub fn current_dir(&self) -> &Location<M> {
        &self.current_dir
    }

    pub fn set_current_dir(&mut self, current_dir: Location<M>) -> VfsResult<()> {
        current_dir.check_is_dir()?;
        self.current_dir = current_dir;
        Ok(())
    }

    pub fn with_current_dir(&self, current_dir: Location<M>) -> VfsResult<Self> {
        current_dir.check_is_dir()?;
        Ok(Self {
            root_dir: self.root_dir.clone(),
            current_dir,
        })
    }

    fn resolve_inner<'a>(&self, path: &'a Path) -> VfsResult<(Location<M>, Option<&'a str>)> {
        let mut dir = self.current_dir.clone();

        let entry_name = path.file_name();
        let mut components = path.components();
        if entry_name.is_some() {
            components.next_back();
        }
        for comp in components {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    dir = dir.parent().unwrap_or_else(|| self.root_dir.clone());
                }
                Component::RootDir => {
                    dir = self.root_dir.clone();
                }
                Component::Normal(name) => {
                    dir = dir.lookup(name)?;
                }
            }
        }
        dir.check_is_dir()?;
        Ok((dir, entry_name))
    }

    /// Taking current node as root directory, resolves a path starting from
    /// `current_dir`.
    pub fn resolve(&self, path: impl AsRef<Path>) -> VfsResult<Location<M>> {
        let (dir, name) = self.resolve_inner(path.as_ref())?;
        match name {
            Some(name) => dir.lookup(name),
            None => Ok(dir),
        }
    }

    /// Taking current node as root directory, resolves a path starting from
    /// `current_dir`.
    ///
    /// Returns `(parent_dir, entry_name)`, where `entry_name` is the name of
    /// the entry.
    pub fn resolve_parent<'a>(&self, path: &'a Path) -> VfsResult<(Location<M>, Cow<'a, str>)> {
        let (dir, name) = self.resolve_inner(path)?;
        if let Some(name) = name {
            Ok((dir, Cow::Borrowed(name)))
        } else if let Some(parent) = dir.parent() {
            Ok((parent, Cow::Owned(dir.name().to_owned())))
        } else {
            Err(VfsError::EINVAL)
        }
    }

    /// Resolves a path starting from `current_dir`, returning the parent
    /// directory and the name of the entry.
    ///
    /// This function requires that the entry does not exist and the parent
    /// exists. Note that, it does not perform an actual check to ensure the
    /// entry's non-existence. It simply raises an error if the entry name is
    /// not present in the path.
    pub fn resolve_nonexistent<'a>(&self, path: &'a Path) -> VfsResult<(Location<M>, &'a str)> {
        let (dir, name) = self.resolve_inner(path)?;
        if let Some(name) = name {
            Ok((dir, name))
        } else {
            Err(VfsError::EEXIST)
        }
    }

    /// Reads the entire contents of a file into a bytes vector.
    pub fn read(&self, path: impl AsRef<Path>) -> VfsResult<Vec<u8>> {
        let file = self.resolve(path.as_ref())?;
        let mut buf = Vec::new();
        File::new(file, FileFlags::READ).read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Reads the entire contents of a file into a string.
    pub fn read_to_string(&self, path: impl AsRef<Path>) -> VfsResult<String> {
        String::from_utf8(self.read(path)?).map_err(|_| VfsError::EINVAL)
    }

    /// Writes the entire contents of a bytes vector into a file.
    pub fn write(&self, path: impl AsRef<Path>, data: impl AsRef<[u8]>) -> VfsResult<()> {
        File::create(self, path.as_ref())?.write_all(data.as_ref())?;
        Ok(())
    }

    /// Retrieves metadata for the file.
    pub fn metadata(&self, path: impl AsRef<Path>) -> VfsResult<Metadata> {
        self.resolve(path)?.metadata()
    }

    /// Returns an iterator over the entries in a directory.
    pub fn read_dir(&self, path: impl AsRef<Path>) -> VfsResult<ReadDir<M>> {
        let dir = self.resolve(path)?;
        Ok(ReadDir {
            dir,
            buf: VecDeque::new(),
            offset: 0,
            ended: false,
        })
    }

    /// Removes a file from the filesystem.
    pub fn remove_file(&self, path: impl AsRef<Path>) -> VfsResult<()> {
        let entry = self.resolve(path.as_ref())?;
        entry
            .parent()
            .ok_or(VfsError::EISDIR)?
            .unlink(entry.name(), false)
    }

    /// Removes a directory from the filesystem.
    pub fn remove_dir(&self, path: impl AsRef<Path>) -> VfsResult<()> {
        let entry = self.resolve(path.as_ref())?;
        entry
            .parent()
            .ok_or(VfsError::EBUSY)?
            .unlink(entry.name(), true)
    }

    /// Renames a file or directory to a new name, replacing the original file if `to` already exists.
    pub fn rename(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> VfsResult<()> {
        let (src_dir, src_name) = self.resolve_parent(from.as_ref())?;
        let (dst_dir, dst_name) = self.resolve_parent(to.as_ref())?;
        src_dir.rename(&src_name, &dst_dir, &dst_name)
    }

    /// Creates a new, empty directory at the provided path.
    pub fn create_dir(
        &self,
        path: impl AsRef<Path>,
        mode: NodePermission,
    ) -> VfsResult<Location<M>> {
        let (dir, name) = self.resolve_nonexistent(path.as_ref())?;
        dir.create(name, NodeType::Directory, mode)
    }

    /// Creates a new hard link on the filesystem.
    pub fn link(
        &self,
        old_path: impl AsRef<Path>,
        new_path: impl AsRef<Path>,
    ) -> VfsResult<Location<M>> {
        let old = self.resolve(old_path.as_ref())?;
        let (new_dir, new_name) = self.resolve_nonexistent(new_path.as_ref())?;
        new_dir.link(new_name, &old)
    }

    /// Returns the canonical, absolute form of a path.
    pub fn canonicalize(&self, path: impl AsRef<Path>) -> VfsResult<PathBuf> {
        self.resolve(path.as_ref())?.absolute_path()
    }
}

/// Iterator returned by [`FsContext::read_dir`].
pub struct ReadDir<M> {
    dir: Location<M>,
    buf: VecDeque<ReadDirEntry>,
    offset: u64,
    ended: bool,
}
impl<M> ReadDir<M> {
    // TODO: tune this
    pub const BUF_SIZE: usize = 128;
}
impl<M: RawMutex> Iterator for ReadDir<M> {
    type Item = VfsResult<ReadDirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ended {
            return None;
        }

        if self.buf.is_empty() {
            self.buf.clear();
            let result = self.dir.read_dir(
                self.offset,
                &mut |name: &str, ino: u64, node_type: NodeType, offset: u64| {
                    self.buf.push_back(ReadDirEntry {
                        name: name.to_owned(),
                        ino,
                        node_type,
                        offset,
                    });
                    self.offset = offset;
                    self.buf.len() < Self::BUF_SIZE
                },
            );

            // We handle errors only if we didn't get any entries
            if self.buf.is_empty() {
                if let Err(err) = result {
                    return Some(Err(err));
                }
                self.ended = true;
                return None;
            }
        }

        self.buf.pop_front().map(Ok)
    }
}
