use alloc::{
    borrow::{Cow, ToOwned},
    collections::vec_deque::VecDeque,
    string::String,
    vec::Vec,
};
use axio::{Read, Write};
use lock_api::RawMutex;

use axfs_ng_vfs::{
    Component, DirEntry, Filesystem, Metadata, NodePermission, NodeType, Path, PathBuf, VfsError,
    VfsResult,
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

/// Provides `std::fs`-like interface.
pub struct FsContext<M> {
    fs: Filesystem<M>,
    root_dir: DirEntry<M>,
    current_dir: DirEntry<M>,
}
impl<M> Clone for FsContext<M> {
    fn clone(&self) -> Self {
        Self {
            fs: self.fs.clone(),
            root_dir: self.root_dir.clone(),
            current_dir: self.current_dir.clone(),
        }
    }
}
impl<M: RawMutex> FsContext<M> {
    pub fn new(fs: Filesystem<M>, root_dir: DirEntry<M>) -> Self {
        Self {
            fs,
            root_dir: root_dir.clone(),
            current_dir: root_dir,
        }
    }

    pub fn new_root(fs: Filesystem<M>) -> Self {
        let root_dir = fs.root_dir();
        Self::new(fs, root_dir)
    }

    pub fn filesystem(&self) -> &Filesystem<M> {
        &self.fs
    }

    pub fn root_dir(&self) -> &DirEntry<M> {
        &self.root_dir
    }
    pub fn current_dir(&self) -> &DirEntry<M> {
        &self.current_dir
    }

    pub fn set_current_dir(&mut self, current_dir: DirEntry<M>) -> VfsResult<()> {
        current_dir.as_dir()?;
        self.current_dir = current_dir;
        Ok(())
    }

    pub fn with_current_dir(&self, current_dir: DirEntry<M>) -> VfsResult<Self> {
        current_dir.as_dir()?;
        Ok(Self {
            fs: self.fs.clone(),
            root_dir: self.root_dir.clone(),
            current_dir,
        })
    }

    fn resolve_inner<'a>(&self, path: &'a Path) -> VfsResult<(DirEntry<M>, Option<&'a str>)> {
        let mut dir = self.current_dir.clone();
        let mut stack = Vec::new();

        let entry_name = path.file_name();
        let mut components = path.components();
        if entry_name.is_some() {
            components.next_back();
        }
        for comp in components {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    dir = stack.pop().unwrap_or_else(|| self.root_dir.clone());
                }
                Component::RootDir => {
                    dir = self.root_dir.clone();
                }
                Component::Normal(name) => {
                    dir = dir.as_dir()?.lookup(name)?;
                }
            }
        }
        dir.as_dir()?;
        Ok((dir, entry_name))
    }

    /// Taking current node as root directory, resolves a path starting from
    /// `current_dir`.
    pub fn resolve(&self, path: impl AsRef<Path>) -> VfsResult<DirEntry<M>> {
        let (dir, name) = self.resolve_inner(path.as_ref())?;
        Ok(match name {
            Some(name) => dir.as_dir()?.lookup(name)?,
            None => dir,
        })
    }

    /// Taking current node as root directory, resolves a path starting from
    /// `current_dir`.
    ///
    /// Returns `(parent_dir, entry_name)`, where `entry_name` is the name of
    /// the entry.
    pub fn resolve_parent<'a>(&self, path: &'a Path) -> VfsResult<(DirEntry<M>, Cow<'a, str>)> {
        let (dir, name) = self.resolve_inner(path)?;
        if let Some(name) = name {
            Ok((dir, Cow::Borrowed(name)))
        } else if let Some(parent) = dir.parent()? {
            Ok((parent, Cow::Owned(dir.name().to_owned())))
        } else {
            Err(VfsError::InvalidInput)
        }
    }

    /// Resolves a path starting from `current_dir`, returning the parent
    /// directory and the name of the entry.
    ///
    /// This function requires that the entry does not exist and the parent
    /// exists. Note that, it does not perform an actual check to ensure the
    /// entry's non-existence. It simply raises an error if the entry name is
    /// not present in the path.
    pub fn resolve_nonexistent<'a>(&self, path: &'a Path) -> VfsResult<(DirEntry<M>, &'a str)> {
        let (dir, name) = self.resolve_inner(path)?;
        if let Some(name) = name {
            Ok((dir, name))
        } else {
            Err(VfsError::AlreadyExists)
        }
    }

    /// Reads the entire contents of a file into a bytes vector.
    pub fn read(&self, path: impl AsRef<Path>) -> VfsResult<Vec<u8>> {
        let file = self.resolve(path.as_ref())?;
        let mut buf = Vec::new();
        File::new(file.clone(), FileFlags::READ).read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Reads the entire contents of a file into a string.
    pub fn read_to_string(&self, path: impl AsRef<Path>) -> VfsResult<String> {
        String::from_utf8(self.read(path)?).map_err(|_| VfsError::InvalidData)
    }

    /// Writes the entire contents of a bytes vector into a file.
    pub fn write(&self, path: impl AsRef<Path>, data: impl AsRef<[u8]>) -> VfsResult<()> {
        File::new(self.resolve(path.as_ref())?, FileFlags::WRITE).write_all(data.as_ref())?;
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
            .parent()?
            .ok_or(VfsError::IsADirectory)?
            .as_dir()?
            .unlink(entry.name(), false)
    }

    /// Removes a directory from the filesystem.
    pub fn remove_dir(&self, path: impl AsRef<Path>) -> VfsResult<()> {
        let entry = self.resolve(path.as_ref())?;
        entry
            .parent()?
            .ok_or(VfsError::ResourceBusy)?
            .as_dir()?
            .unlink(entry.name(), true)
    }

    /// Renames a file or directory to a new name, replacing the original file if `to` already exists.
    pub fn rename(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> VfsResult<()> {
        let (src_dir, src_name) = self.resolve_parent(from.as_ref())?;
        let (dst_dir, dst_name) = self.resolve_parent(to.as_ref())?;
        if !src_dir.ptr_eq(&dst_dir) && src_dir.is_ancestor_of(&dst_dir)? {
            return Err(VfsError::InvalidInput);
        }
        src_dir
            .as_dir()?
            .rename(&src_name, dst_dir.as_dir()?, &dst_name)
    }

    /// Creates a new, empty directory at the provided path.
    pub fn create_dir(
        &self,
        path: impl AsRef<Path>,
        mode: NodePermission,
    ) -> VfsResult<DirEntry<M>> {
        let (dir, name) = self.resolve_nonexistent(path.as_ref())?;
        dir.as_dir()?.create(name, NodeType::Directory, mode)
    }

    /// Creates a new hard link on the filesystem.
    pub fn link(
        &self,
        old_path: impl AsRef<Path>,
        new_path: impl AsRef<Path>,
    ) -> VfsResult<DirEntry<M>> {
        let old = self.resolve(old_path.as_ref())?;
        let (new_dir, new_name) = self.resolve_nonexistent(new_path.as_ref())?;
        new_dir.as_dir()?.link(new_name, &old)
    }

    /// Returns the canonical, absolute form of a path.
    pub fn canonicalize(&self, path: impl AsRef<Path>) -> VfsResult<PathBuf> {
        self.resolve(path.as_ref())?.absolute_path()
    }
}

/// Iterator returned by [`FsContext::read_dir`].
pub struct ReadDir<M> {
    dir: DirEntry<M>,
    buf: VecDeque<DirEntry<M>>,
    offset: u64,
    ended: bool,
}
impl<M> ReadDir<M> {
    // TODO: tune this
    pub const BUF_SIZE: usize = 128;
}
impl<M: RawMutex> Iterator for ReadDir<M> {
    type Item = VfsResult<DirEntry<M>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ended {
            return None;
        }

        if self.buf.is_empty() {
            self.buf.clear();
            let result = self.dir.as_dir().unwrap().read_dir(
                self.offset,
                &mut |entry: DirEntry<M>, offset| {
                    self.buf.push_back(entry);
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
