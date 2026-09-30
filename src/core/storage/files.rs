//! Descriptor-bound file access and short advisory locks for the foundation.
use super::{Publication, StorageError, StorageErrorCode};
use rustix::fs::{self, AtFlags, CWD, Dir, FileType, FlockOperation, Mode, OFlags, RenameFlags};
use std::ffi::{OsStr, OsString};
use std::fs::{File, Metadata};
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Identity {
    pub dev: u64,
    pub ino: u64,
}
impl Identity {
    fn metadata(meta: &Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Source {
    pub raw: Vec<u8>,
    pub identity: Identity,
    pub mode: u32,
    pub size: u64,
    pub mtime: (i64, i64),
    pub ctime: (i64, i64),
}
impl Source {
    fn metadata(raw: Vec<u8>, meta: &Metadata) -> Self {
        Self {
            raw,
            identity: Identity::metadata(meta),
            mode: meta.mode(),
            size: meta.size(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: (meta.ctime(), meta.ctime_nsec()),
        }
    }
    pub fn same_content_identity(&self, other: &Self) -> bool {
        // Rename/exchange itself can change ctime on the retained inode. Keep
        // full Source equality before rename; afterward compare every captured
        // property that the rename preserves, including size and mtime.
        self.raw == other.raw
            && self.identity == other.identity
            && self.mode == other.mode
            && self.size == other.size
            && self.mtime == other.mtime
    }
}
pub(super) struct Directory {
    pub file: File,
    pub path: PathBuf,
    pub identity: Identity,
}
impl Directory {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let canonical = path
            .canonicalize()
            .map_err(|e| StorageError::io(e, path.to_owned()))?;
        if canonical != path {
            return Err(StorageError::new(
                StorageErrorCode::UnsafePath,
                "directory path is not canonical or traverses a symlink",
                Some(path.to_owned()),
            ));
        }
        let fd =
            fs::openat(CWD, path, directory_flags(), Mode::empty()).map_err(|e| map(e, path))?;
        Self::from_file(File::from(fd), path.to_owned())
    }
    fn from_file(file: File, path: PathBuf) -> Result<Self, StorageError> {
        let meta = file
            .metadata()
            .map_err(|e| StorageError::io(e, path.clone()))?;
        if !meta.is_dir() {
            return Err(unsafe_path(&path, "expected a real directory"));
        }
        if meta.mode() & 0o500 != 0o500 {
            return Err(StorageError::new(
                StorageErrorCode::PermissionDenied,
                "directory lacks owner read/search permission",
                Some(path),
            ));
        }
        Ok(Self {
            identity: Identity::metadata(&meta),
            file,
            path,
        })
    }
    pub fn child(&self, name: impl AsRef<OsStr>) -> Result<Self, StorageError> {
        let name = name.as_ref();
        valid_name(name)?;
        self.verify()?;
        let path = self.path.join(name);
        let fd = fs::openat(&self.file, name, directory_flags(), Mode::empty())
            .map_err(|e| map(e, &path))?;
        let dir = Self::from_file(File::from(fd), path)?;
        dir.verify()?;
        Ok(dir)
    }
    pub fn create(&self, name: &str) -> Result<Self, StorageError> {
        valid_name(OsStr::new(name))?;
        self.verify()?;
        let path = self.path.join(name);
        fs::mkdirat(&self.file, name, Mode::from_bits_truncate(0o700)).map_err(|e| {
            let mut error = map(e, &path);
            if e == rustix::io::Errno::EXIST {
                error.errno = Some(e.raw_os_error());
            }
            error
        })?;
        let before =
            fs::statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| map(e, &path))?;
        if FileType::from_raw_mode(before.st_mode) != FileType::Directory {
            return Err(unsafe_path(&path, "new directory was substituted"));
        }
        let expected = Identity {
            dev: before.st_dev as u64,
            ino: before.st_ino as u64,
        };
        // Retain a permission-recovery pin until the ordinary FD and pathname
        // have both been validated, even across the normal reopen.
        let mut _permission_pin = None;
        let fd = match fs::openat(&self.file, name, directory_flags(), Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::ACCESS) => {
                _permission_pin = self.restore_new_directory_mode(name, &expected)?;
                fs::openat(&self.file, name, directory_flags(), Mode::empty())
                    .map_err(|e| map(e, &path))?
            }
            Err(error) => return Err(map(error, &path)),
        };
        let file = File::from(fd);
        let opened = file
            .metadata()
            .map_err(|e| StorageError::io(e, path.clone()))?;
        if !opened.is_dir() || Identity::metadata(&opened) != expected {
            return Err(conflict(&path, "new directory changed while opening"));
        }
        fs::fchmod(&file, Mode::from_bits_truncate(0o700)).map_err(|e| map(e, &path))?;
        let result = Self::from_file(file, path)?;
        result.verify()?;
        result.sync()?;
        self.sync()?;
        self.verify()?;
        Ok(result)
    }
    // Only called for an inaccessible directory this attempt just created.
    // Never follow the mutable storage entry to recover from a restrictive umask.
    #[cfg(target_os = "linux")]
    pub(super) fn restore_new_directory_mode(
        &self,
        name: &str,
        expected: &Identity,
    ) -> Result<Option<File>, StorageError> {
        self.verify()?;
        let path = self.path.join(name);
        #[cfg(target_os = "linux")]
        let held = {
            let fd = fs::openat(
                &self.file,
                name,
                OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| map(e, &path))?;
            let stat = fs::fstat(&fd).map_err(|e| map(e, &path))?;
            if stat.st_dev as u64 != expected.dev || stat.st_ino as u64 != expected.ino {
                return Err(conflict(
                    &path,
                    "new directory changed before permission recovery",
                ));
            }
            // Linux fchmod cannot operate on O_PATH. The kernel's procfs FD
            // reference pins this inode, as in glibc's nofollow fchmodat fallback.
            // Without procfs fail closed: never chmod the storage pathname.
            #[cfg(test)]
            inject("directory_mode", &path)?;
            let held_path = format!("/proc/self/fd/{}", fd.as_raw_fd());
            fs::chmodat(
                CWD,
                held_path.as_str(),
                Mode::from_bits_truncate(0o700),
                AtFlags::empty(),
            )
            .map_err(|e| StorageError::io(std::io::Error::from(e), path.clone()))?;
            Some(File::from(fd))
        };
        self.verify()?;
        let stat =
            fs::statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| map(e, &path))?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
            || stat.st_dev as u64 != expected.dev
            || stat.st_ino as u64 != expected.ino
        {
            return Err(conflict(
                &path,
                "new directory changed during permission recovery",
            ));
        }
        Ok(held)
    }
    #[cfg(not(target_os = "linux"))]
    pub(super) fn restore_new_directory_mode(
        &self,
        name: &str,
        _expected: &Identity,
    ) -> Result<Option<File>, StorageError> {
        // No inode-bound permission recovery is established on these hosts.
        // Leave the inaccessible new directory as explicit recovery context.
        Err(StorageError::io(
            std::io::Error::from_raw_os_error(13),
            self.path.join(name),
        ))
    }
    pub fn ensure(&self, name: &str) -> Result<Self, StorageError> {
        if self.exists(name)? {
            self.child(name)
        } else {
            self.create(name)
        }
    }
    pub fn exists(&self, name: impl AsRef<OsStr>) -> Result<bool, StorageError> {
        let name = name.as_ref();
        valid_name(name)?;
        self.verify()?;
        match fs::statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Ok(true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(e) => Err(map(e, &self.path.join(name))),
        }
    }
    pub fn optional(&self, name: &str) -> Result<Option<Source>, StorageError> {
        if self.exists(name)? {
            self.read(name).map(Some)
        } else {
            Ok(None)
        }
    }
    pub fn read(&self, name: &str) -> Result<Source, StorageError> {
        self.read_open(name).map(|(_, source)| source)
    }
    fn read_open(&self, name: &str) -> Result<(File, Source), StorageError> {
        valid_name(OsStr::new(name))?;
        self.verify()?;
        let path = self.path.join(name);
        let before =
            fs::statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| map(e, &path))?;
        if FileType::from_raw_mode(before.st_mode) != FileType::RegularFile || before.st_nlink != 1
        {
            return Err(unsafe_path(&path, "expected a regular single-link file"));
        }
        if before.st_mode as u32 & 0o400 == 0 {
            return Err(StorageError::new(
                StorageErrorCode::PermissionDenied,
                "file lacks owner-read permission",
                Some(path),
            ));
        }
        #[cfg(test)]
        inject("source_open", &path)?;
        let fd = fs::openat(
            &self.file,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| map(e, &path))?;
        let mut file = File::from(fd);
        let meta = file
            .metadata()
            .map_err(|e| StorageError::io(e, path.clone()))?;
        if !meta.is_file() || meta.nlink() != 1 {
            return Err(unsafe_path(
                &path,
                "opened source is not a single-link regular file",
            ));
        }
        if meta.dev() != before.st_dev as u64 || meta.ino() != before.st_ino as u64 {
            return Err(conflict(&path, "source changed while opening"));
        }
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)
            .map_err(|e| StorageError::io(e, path.clone()))?;
        let after = file
            .metadata()
            .map_err(|e| StorageError::io(e, path.clone()))?;
        let source = Source::metadata(raw, &meta);
        if source != Source::metadata(source.raw.clone(), &after) {
            return Err(conflict(&path, "source changed while reading"));
        }
        self.verify()?;
        Ok((file, source))
    }
    /// Reopen without blocking on a substituted special file, validate the
    /// captured source and sync that held inode before relying on its durability.
    pub fn sync_source(&self, name: &str, expected: &Source) -> Result<File, StorageError> {
        let (file, source) = self.read_open(name)?;
        let path = self.path.join(name);
        if &source != expected {
            return Err(conflict(&path, "source changed before file sync"));
        }
        #[cfg(test)]
        inject(
            if name == "operation.yaml" {
                "receipt_file_sync"
            } else if name.starts_with(".storage-") {
                "retry_stage_sync"
            } else {
                "source_sync"
            },
            &path,
        )?;
        fs::fsync(&file).map_err(|e| map(e, &path))?;
        let after = file
            .metadata()
            .map_err(|e| StorageError::io(e, path.clone()))?;
        if Source::metadata(source.raw.clone(), &after) != source || self.read(name)? != source {
            return Err(conflict(&path, "source changed during file sync"));
        }
        self.verify()?;
        Ok(file)
    }
    pub fn names(&self) -> Result<Vec<OsString>, StorageError> {
        self.verify()?;
        let mut result = Vec::new();
        for entry in Dir::read_from(&self.file).map_err(|e| map(e, &self.path))? {
            let entry = entry.map_err(|e| map(e, &self.path))?;
            let name = entry.file_name().to_bytes();
            if name != b"." && name != b".." {
                result.push(OsString::from_vec(name.to_vec()));
            }
        }
        result.sort();
        self.verify()?;
        Ok(result)
    }
    pub fn verify(&self) -> Result<(), StorageError> {
        let selected = fs::statat(CWD, &self.path, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| {
            conflict(
                &self.path,
                "directory pathname no longer names held directory",
            )
        })?;
        if FileType::from_raw_mode(selected.st_mode) != FileType::Directory
            || selected.st_dev as u64 != self.identity.dev
            || selected.st_ino as u64 != self.identity.ino
        {
            return Err(conflict(&self.path, "directory pathname changed"));
        }
        if self
            .path
            .canonicalize()
            .map_err(|e| StorageError::io(e, self.path.clone()))?
            != self.path
        {
            return Err(conflict(&self.path, "directory ancestor was substituted"));
        }
        Ok(())
    }
    pub fn sync(&self) -> Result<(), StorageError> {
        fs::fsync(&self.file).map_err(|e| map(e, &self.path))
    }
    pub fn sync_receipt_directory(&self) -> Result<(), StorageError> {
        #[cfg(test)]
        inject("receipt_directory_sync", &self.path)?;
        self.sync()?;
        self.verify()
    }
    pub fn stage(&self, name: &str, raw: &[u8]) -> Result<(), StorageError> {
        self.verify()?;
        valid_name(OsStr::new(name))?;
        let path = self.path.join(name);
        let fd = fs::openat(
            &self.file,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )
        .map_err(|e| map(e, &path))?;
        fs::fchmod(&fd, Mode::from_bits_truncate(0o600)).map_err(|e| map(e, &path))?;
        let mut file = File::from(fd);
        #[cfg(test)]
        inject("stage_write", &path)?;
        file.write_all(raw)
            .map_err(|e| StorageError::io(e, path.clone()))?;
        #[cfg(test)]
        inject("stage_sync", &path)?;
        file.sync_all().map_err(|e| StorageError::io(e, path))?;
        self.sync()?;
        self.verify()
    }
    /// Publish one file; every successful replacement retains the former inode.
    pub fn publish(
        &self,
        name: &str,
        raw: &[u8],
        expected: Option<&Source>,
        stage_name: &str,
    ) -> Result<Option<PathBuf>, StorageError> {
        self.verify()?;
        let path = self.path.join(name);
        let stage_path = self.path.join(stage_name);
        if self.optional(name)?.as_ref() != expected {
            return Err(conflict(&path, "source changed before publication"));
        }
        if self.exists(stage_name)? {
            return Err(conflict(&stage_path, "publication stage already exists"));
        }
        self.stage(stage_name, raw)?;
        if self.optional(name)?.as_ref() != expected {
            return Err(conflict(&path, "source changed while staging"));
        }
        self.verify()?;
        #[cfg(test)]
        inject("before_publication", &path)?;
        let flags = if expected.is_some() {
            RenameFlags::EXCHANGE
        } else {
            RenameFlags::NOREPLACE
        };
        fs::renameat_with(&self.file, stage_name, &self.file, name, flags)
            .map_err(|e| map(e, &path))?;
        let result = (|| {
            #[cfg(test)]
            inject("after_publication", &path)?;
            #[cfg(test)]
            inject("publication_sync", &path)?;
            self.sync()?;
            #[cfg(test)]
            inject("after_directory_sync", &path)?;
            self.verify()?;
            let published = self.read(name)?;
            if published.raw != raw {
                return Err(conflict(&path, "source changed after publication"));
            }
            if let Some(before) = expected {
                let retained = self.read(stage_name)?;
                if !before.same_content_identity(&retained) {
                    return Err(conflict(
                        &stage_path,
                        "replaced source changed during publication",
                    ));
                }
            }
            Ok(expected.map(|_| stage_path.clone()))
        })();
        result.map_err(|mut e| {
            e.publication = Publication::Possible;
            e.recovery_paths.push(stage_path);
            e
        })
    }
    pub fn sync_archive_parents(&self, destination: &Self) -> Result<(), StorageError> {
        #[cfg(test)]
        inject("archive_source_sync", &self.path)?;
        self.sync()?;
        #[cfg(test)]
        inject("archive_destination_sync", &destination.path)?;
        destination.sync()?;
        self.verify()?;
        destination.verify()
    }
    pub fn rename_child(
        &self,
        name: &str,
        destination: &Self,
        expected: &Identity,
    ) -> Result<(), StorageError> {
        self.verify()?;
        destination.verify()?;
        let held = self.child(name)?;
        if &held.identity != expected {
            return Err(conflict(
                &held.path,
                "directory identity changed before archival",
            ));
        }
        fs::renameat_with(
            &self.file,
            name,
            &destination.file,
            name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|e| map(e, &destination.path.join(name)))?;
        self.sync_archive_parents(destination)?;
        if &destination.child(name)?.identity != expected {
            return Err(conflict(
                &destination.path.join(name),
                "archived directory was substituted",
            ));
        }
        Ok(())
    }
}
pub(super) struct Locked {
    pub common: Directory,
    pub root: Directory,
    lock: File,
    identity: Identity,
}
impl Locked {
    pub fn open(common: Directory, exclusive: bool, create: bool) -> Result<Self, StorageError> {
        let root = common.child("work")?;
        let path = root.path.join("coordination.lock");
        let existing = root.exists("coordination.lock")?;
        if !existing && !create {
            return Err(StorageError::new(
                StorageErrorCode::StorageMissing,
                "coordination lock is missing; explicit stopped-client recreation required",
                Some(path),
            ));
        }
        let fd = if !existing {
            let fd = fs::openat(
                &root.file,
                "coordination.lock",
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_bits_truncate(0o600),
            )
            .map_err(|e| map(e, &path))?;
            fs::fchmod(&fd, Mode::from_bits_truncate(0o600)).map_err(|e| map(e, &path))?;
            fs::fsync(&fd).map_err(|e| map(e, &path))?;
            root.sync()?;
            fd
        } else {
            let source = root.read("coordination.lock")?;
            let fd = fs::openat(
                &root.file,
                "coordination.lock",
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| map(e, &path))?;
            let stat = fs::fstat(&fd).map_err(|e| map(e, &path))?;
            if stat.st_dev as u64 != source.identity.dev
                || stat.st_ino as u64 != source.identity.ino
            {
                return Err(conflict(&path, "lock changed while opening"));
            }
            fd
        };
        let lock = File::from(fd);
        let meta = lock
            .metadata()
            .map_err(|e| StorageError::io(e, path.clone()))?;
        if !meta.is_file() || meta.nlink() != 1 {
            return Err(unsafe_path(
                &path,
                "coordination lock must be a single-link regular file",
            ));
        }
        let operation = if exclusive {
            FlockOperation::NonBlockingLockExclusive
        } else {
            FlockOperation::NonBlockingLockShared
        };
        fs::flock(&lock, operation).map_err(|e| {
            if e == rustix::io::Errno::WOULDBLOCK {
                StorageError::new(
                    StorageErrorCode::StorageBusy,
                    "shared coordination lock is busy",
                    Some(path.clone()),
                )
            } else {
                map(e, &path)
            }
        })?;
        let result = Self {
            identity: Identity::metadata(&meta),
            common,
            root,
            lock,
        };
        result.verify()?;
        Ok(result)
    }
    pub fn verify(&self) -> Result<(), StorageError> {
        self.common.verify()?;
        self.root.verify()?;
        let path = self.root.path.join("coordination.lock");
        let stat = fs::statat(
            &self.root.file,
            "coordination.lock",
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|_| conflict(&path, "lock pathname disappeared"))?;
        let held = self
            .lock
            .metadata()
            .map_err(|e| StorageError::io(e, path.clone()))?;
        if !held.is_file()
            || held.nlink() != 1
            || FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_dev as u64 != self.identity.dev
            || stat.st_ino as u64 != self.identity.ino
        {
            return Err(conflict(&path, "coordination lock inode changed"));
        }
        Ok(())
    }
}
impl Drop for Locked {
    fn drop(&mut self) {
        // Explicit unlock also prevents an unrelated fork awaiting exec from extending
        // this finished critical section through its transient inherited description.
        let _ = fs::flock(&self.lock, FlockOperation::Unlock);
    }
}
fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}
fn valid_name(name: &OsStr) -> Result<(), StorageError> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes == b"."
        || bytes == b".."
        || bytes.contains(&b'/')
        || bytes.contains(&0)
    {
        return Err(StorageError::new(
            StorageErrorCode::UnsafePath,
            "unsafe single directory entry name",
            None,
        ));
    }
    Ok(())
}
fn unsafe_path(path: &Path, message: &str) -> StorageError {
    StorageError::new(StorageErrorCode::UnsafePath, message, Some(path.to_owned()))
}
pub(super) fn conflict(path: &Path, message: &str) -> StorageError {
    StorageError::new(StorageErrorCode::Conflict, message, Some(path.to_owned()))
}
fn map(error: rustix::io::Errno, path: &Path) -> StorageError {
    match error {
        rustix::io::Errno::NOENT => StorageError::new(
            StorageErrorCode::StorageMissing,
            "required storage entry is missing",
            Some(path.to_owned()),
        ),
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => {
            unsafe_path(path, "symlink or non-directory storage path")
        }
        rustix::io::Errno::EXIST => conflict(path, "entry already exists"),
        _ => StorageError::io(std::io::Error::from(error), path.to_owned()),
    }
}

#[cfg(test)]
thread_local! {
    static FAILURE: std::cell::Cell<Option<&'static str>>=const {std::cell::Cell::new(None)};
    static ACTION: std::cell::RefCell<Option<TestAction>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
struct TestAction {
    point: &'static str,
    remaining: usize,
    action: Box<dyn FnOnce()>,
}
#[cfg(test)]
pub(super) fn on_next(point: &'static str, action: impl FnOnce() + 'static) -> FailureGuard {
    on_nth(point, 1, action)
}
#[cfg(test)]
pub(super) fn on_nth(
    point: &'static str,
    remaining: usize,
    action: impl FnOnce() + 'static,
) -> FailureGuard {
    ACTION.with(|slot| {
        *slot.borrow_mut() = Some(TestAction {
            point,
            remaining,
            action: Box::new(action),
        })
    });
    FailureGuard
}
#[cfg(test)]
pub(super) struct FailureGuard;
#[cfg(test)]
impl Drop for FailureGuard {
    fn drop(&mut self) {
        FAILURE.set(None);
        ACTION.with(|slot| *slot.borrow_mut() = None);
    }
}
#[cfg(test)]
pub(super) fn fail_next(point: &'static str) -> FailureGuard {
    FAILURE.set(Some(point));
    FailureGuard
}
#[cfg(test)]
pub(super) fn inject(point: &str, path: &Path) -> Result<(), StorageError> {
    let action = ACTION.with(|slot| {
        let mut action = slot.borrow_mut();
        if let Some(selected) = action.as_mut()
            && selected.point == point
        {
            selected.remaining -= 1;
            if selected.remaining == 0 {
                return action.take();
            }
        }
        None
    });
    if let Some(action) = action {
        (action.action)();
    }
    if FAILURE.get() == Some(point) {
        FAILURE.set(None);
        return Err(StorageError::io(
            std::io::Error::from_raw_os_error(5),
            path.to_owned(),
        ));
    }
    Ok(())
}
