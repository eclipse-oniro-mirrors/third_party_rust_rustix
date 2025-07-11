use super::super::c;
use super::super::conv::owned_fd;
#[cfg(not(any(target_os = "haiku", target_os = "illumos", target_os = "solaris")))]
use super::types::FileType;
use crate::fd::{AsFd, BorrowedFd};
use crate::ffi::CStr;
#[cfg(target_os = "wasi")]
use crate::ffi::CString;
use crate::fs::{fcntl_getfl, fstat, openat, Mode, OFlags, Stat};
#[cfg(not(any(
    target_os = "haiku",
    target_os = "illumos",
    target_os = "netbsd",
    target_os = "redox",
    target_os = "solaris",
    target_os = "wasi",
)))]
use crate::fs::{fstatfs, StatFs};
#[cfg(not(any(
    target_os = "haiku",
    target_os = "illumos",
    target_os = "redox",
    target_os = "solaris",
    target_os = "wasi",
)))]
use crate::fs::{fstatvfs, StatVfs};
use crate::io;
#[cfg(not(any(target_os = "fuchsia", target_os = "wasi")))]
use crate::process::fchdir;
#[cfg(target_os = "wasi")]
use alloc::borrow::ToOwned;
#[cfg(not(any(
    target_os = "android",
    target_os = "emscripten",
    target_os = "l4re",
    target_os = "linux",
    target_os = "openbsd",
)))]
use c::dirent as libc_dirent;
#[cfg(not(any(
    target_os = "android",
    target_os = "emscripten",
    target_os = "l4re",
    target_os = "linux",
)))]
use c::readdir as libc_readdir;
#[cfg(any(
    target_os = "android",
    target_os = "emscripten",
    target_os = "l4re",
    target_os = "linux",
))]
use c::{dirent64 as libc_dirent, readdir64 as libc_readdir};
use core::fmt;
use core::mem::zeroed;
use core::ptr::NonNull;
use libc_errno::{errno, set_errno, Errno};

/// `DIR*`
pub struct Dir {
    /// The `libc` `DIR` pointer.
    libc_dir: NonNull<c::DIR>,

    /// Have we seen any errors in this iteration?
    any_errors: bool,
}

impl Dir {
    /// Construct a `Dir` that reads entries from the given directory
    /// file descriptor.
    #[inline]
    pub fn read_from<Fd: AsFd>(fd: Fd) -> io::Result<Self> {
        Self::_read_from(fd.as_fd())
    }

    #[inline]
    #[allow(unused_mut)]
    fn _read_from(fd: BorrowedFd<'_>) -> io::Result<Self> {
        let mut any_errors = false;

        // Given an arbitrary `OwnedFd`, it's impossible to know whether the
        // user holds a `dup`'d copy which could continue to modify the
        // file description state, which would cause Undefined Behavior after
        // our call to `fdopendir`. To prevent this, we obtain an independent
        // `OwnedFd`.
        let flags = fcntl_getfl(fd)?;
        let fd_for_dir = match openat(fd, cstr!("."), flags | OFlags::CLOEXEC, Mode::empty()) {
            Ok(fd) => fd,
            #[cfg(not(target_os = "wasi"))]
            Err(io::Errno::NOENT) => {
                // If "." doesn't exist, it means the directory was removed.
                // We treat that as iterating through a directory with no
                // entries.
                any_errors = true;
                crate::io::dup(fd)?
            }
            Err(err) => return Err(err),
        };

        let raw = owned_fd(fd_for_dir);
        unsafe {
            let libc_dir = c::fdopendir(raw);

            if let Some(libc_dir) = NonNull::new(libc_dir) {
                Ok(Self {
                    libc_dir,
                    any_errors,
                })
            } else {
                let err = io::Errno::last_os_error();
                let _ = c::close(raw);
                Err(err)
            }
        }
    }

    /// `rewinddir(self)`
    #[inline]
    pub fn rewind(&mut self) {
        self.any_errors = false;
        unsafe { c::rewinddir(self.libc_dir.as_ptr()) }
    }

    /// `readdir(self)`, where `None` means the end of the directory.
    pub fn read(&mut self) -> Option<io::Result<DirEntry>> {
        // If we've seen errors, don't continue to try to read anyting further.
        if self.any_errors {
            return None;
        }

        set_errno(Errno(0));
        let dirent_ptr = unsafe { libc_readdir(self.libc_dir.as_ptr()) };
        if dirent_ptr.is_null() {
            let curr_errno = errno().0;
            if curr_errno == 0 {
                // We successfully reached the end of the stream.
                None
            } else {
                // `errno` is unknown or non-zero, so an error occurred.
                self.any_errors = true;
                Some(Err(io::Errno(curr_errno)))
            }
        } else {
            // We successfully read an entry.
            unsafe {
                // We have our own copy of OpenBSD's dirent; check that the
                // layout minimally matches libc's.
                #[cfg(target_os = "openbsd")]
                check_dirent_layout(&*dirent_ptr);

                let result = DirEntry {
                    dirent: read_dirent(&*dirent_ptr.cast()),

                    #[cfg(target_os = "wasi")]
                    name: CStr::from_ptr((*dirent_ptr).d_name.as_ptr()).to_owned(),
                };

                Some(Ok(result))
            }
        }
    }

    /// `fstat(self)`
    #[inline]
    pub fn stat(&self) -> io::Result<Stat> {
        fstat(unsafe { BorrowedFd::borrow_raw(c::dirfd(self.libc_dir.as_ptr())) })
    }

    /// `fstatfs(self)`
    #[cfg(not(any(
        target_os = "haiku",
        target_os = "illumos",
        target_os = "netbsd",
        target_os = "redox",
        target_os = "solaris",
        target_os = "wasi",
    )))]
    #[inline]
    pub fn statfs(&self) -> io::Result<StatFs> {
        fstatfs(unsafe { BorrowedFd::borrow_raw(c::dirfd(self.libc_dir.as_ptr())) })
    }

    /// `fstatvfs(self)`
    #[cfg(not(any(
        target_os = "haiku",
        target_os = "illumos",
        target_os = "redox",
        target_os = "solaris",
        target_os = "wasi",
    )))]
    #[inline]
    pub fn statvfs(&self) -> io::Result<StatVfs> {
        fstatvfs(unsafe { BorrowedFd::borrow_raw(c::dirfd(self.libc_dir.as_ptr())) })
    }

    /// `fchdir(self)`
    #[cfg(not(any(target_os = "fuchsia", target_os = "wasi")))]
    #[inline]
    pub fn chdir(&self) -> io::Result<()> {
        fchdir(unsafe { BorrowedFd::borrow_raw(c::dirfd(self.libc_dir.as_ptr())) })
    }
}

// A `dirent` pointer returned from `readdir` may not point to a full `dirent`
// struct, as the name is NUL-terminated and memory may not be allocated for
// the full extent of the struct. Copy the fields one at a time.
unsafe fn read_dirent(input: &libc_dirent) -> libc_dirent {
    #[cfg(not(any(
        target_os = "aix",
        target_os = "haiku",
        target_os = "illumos",
        target_os = "solaris"
    )))]
    let d_type = input.d_type;

    #[cfg(not(any(
        target_os = "aix",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "haiku",
        target_os = "ios",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "wasi",
    )))]
    let d_off = input.d_off;

    #[cfg(target_os = "aix")]
    let d_offset = input.d_offset;

    #[cfg(not(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    )))]
    let d_ino = input.d_ino;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    let d_fileno = input.d_fileno;

    #[cfg(not(any(target_os = "dragonfly", target_os = "wasi")))]
    let d_reclen = input.d_reclen;

    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "ios",
        target_os = "macos",
    ))]
    let d_namlen = input.d_namlen;

    #[cfg(any(target_os = "ios", target_os = "macos"))]
    let d_seekoff = input.d_seekoff;

    #[cfg(target_os = "haiku")]
    let d_dev = input.d_dev;
    #[cfg(target_os = "haiku")]
    let d_pdev = input.d_pdev;
    #[cfg(target_os = "haiku")]
    let d_pino = input.d_pino;

    // Construct the input. Rust will give us an error if any OS has a input
    // with a field that we missed here. And we can avoid blindly copying the
    // whole `d_name` field, which may not be entirely allocated.
    #[cfg_attr(target_os = "wasi", allow(unused_mut))]
    #[cfg(not(any(target_os = "freebsd", target_os = "dragonfly")))]
    let mut dirent = libc_dirent {
        #[cfg(not(any(
            target_os = "aix",
            target_os = "haiku",
            target_os = "illumos",
            target_os = "solaris"
        )))]
        d_type,
        #[cfg(not(any(
            target_os = "aix",
            target_os = "freebsd",  // Until FreeBSD 12
            target_os = "haiku",
            target_os = "ios",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "wasi",
        )))]
        d_off,
        #[cfg(target_os = "aix")]
        d_offset,
        #[cfg(not(any(target_os = "freebsd", target_os = "netbsd", target_os = "openbsd")))]
        d_ino,
        #[cfg(any(target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"))]
        d_fileno,
        #[cfg(not(target_os = "wasi"))]
        d_reclen,
        #[cfg(any(
            target_os = "aix",
            target_os = "freebsd",
            target_os = "ios",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "openbsd",
        ))]
        d_namlen,
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        d_seekoff,
        // The `d_name` field is NUL-terminated, and we need to be careful not
        // to read bytes past the NUL, even though they're within the nominal
        // extent of the `struct dirent`, because they may not be allocated. So
        // don't read it from `dirent_ptr`.
        //
        // In theory this could use `MaybeUninit::uninit().assume_init()`, but
        // that [invokes undefined behavior].
        //
        // [invokes undefined behavior]: https://doc.rust-lang.org/stable/core/mem/union.MaybeUninit.html#initialization-invariant
        d_name: zeroed(),
        #[cfg(target_os = "openbsd")]
        __d_padding: zeroed(),
        #[cfg(target_os = "haiku")]
        d_dev,
        #[cfg(target_os = "haiku")]
        d_pdev,
        #[cfg(target_os = "haiku")]
        d_pino,
    };
    /*
    pub d_ino: ino_t,
    pub d_pino: i64,
    pub d_reclen: ::c_ushort,
    pub d_name: [::c_char; 1024], // Max length is _POSIX_PATH_MAX
                                  // */

    // On dragonfly and FreeBSD 12, `dirent` has some non-public padding fields
    // so we can't directly initialize it.
    #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
    let mut dirent = {
        let mut dirent: libc_dirent = zeroed();
        dirent.d_fileno = d_fileno;
        dirent.d_namlen = d_namlen;
        dirent.d_type = d_type;
        #[cfg(target_os = "freebsd")]
        {
            dirent.d_reclen = d_reclen;
        }
        dirent
    };

    // Copy from d_name, reading up to and including the first NUL.
    #[cfg(not(target_os = "wasi"))]
    {
        let name_len = CStr::from_ptr(input.d_name.as_ptr())
            .to_bytes_with_nul()
            .len();
        dirent.d_name[..name_len].copy_from_slice(&input.d_name[..name_len]);
    }

    dirent
}

/// `Dir` implements `Send` but not `Sync`, because we use `readdir` which is
/// not guaranteed to be thread-safe. Users can wrap this in a `Mutex` if they
/// need `Sync`, which is effectively what'd need to do to implement `Sync`
/// ourselves.
unsafe impl Send for Dir {}

impl Drop for Dir {
    #[inline]
    fn drop(&mut self) {
        unsafe { c::closedir(self.libc_dir.as_ptr()) };
    }
}

impl Iterator for Dir {
    type Item = io::Result<DirEntry>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        Self::read(self)
    }
}

impl fmt::Debug for Dir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dir")
            .field("fd", unsafe { &c::dirfd(self.libc_dir.as_ptr()) })
            .finish()
    }
}

/// `struct dirent`
#[derive(Debug)]
pub struct DirEntry {
    dirent: libc_dirent,

    #[cfg(target_os = "wasi")]
    name: CString,
}

impl DirEntry {
    /// Returns the file name of this directory entry.
    #[inline]
    pub fn file_name(&self) -> &CStr {
        #[cfg(not(target_os = "wasi"))]
        unsafe {
            CStr::from_ptr(self.dirent.d_name.as_ptr())
        }

        #[cfg(target_os = "wasi")]
        &self.name
    }

    /// Returns the type of this directory entry.
    #[cfg(not(any(
        target_os = "aix",
        target_os = "haiku",
        target_os = "illumos",
        target_os = "solaris"
    )))]
    #[inline]
    pub fn file_type(&self) -> FileType {
        FileType::from_dirent_d_type(self.dirent.d_type)
    }

    /// Return the inode number of this directory entry.
    #[cfg(not(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    )))]
    #[inline]
    pub fn ino(&self) -> u64 {
        self.dirent.d_ino as u64
    }

    /// Return the inode number of this directory entry.
    #[cfg(any(
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    #[inline]
    pub fn ino(&self) -> u64 {
        #[allow(clippy::useless_conversion)]
        self.dirent.d_fileno.into()
    }
}

/// libc's OpenBSD `dirent` has a private field so we can't construct it
/// directly, so we declare it ourselves to make all fields accessible.
#[cfg(target_os = "openbsd")]
#[repr(C)]
#[derive(Debug)]
struct libc_dirent {
    d_fileno: c::ino_t,
    d_off: c::off_t,
    d_reclen: u16,
    d_type: u8,
    d_namlen: u8,
    __d_padding: [u8; 4],
    d_name: [c::c_char; 256],
}

/// We have our own copy of OpenBSD's dirent; check that the layout
/// minimally matches libc's.
#[cfg(target_os = "openbsd")]
fn check_dirent_layout(dirent: &c::dirent) {
    use crate::utils::as_ptr;
    use core::mem::{align_of, size_of};

    // Check that the basic layouts match.
    assert_eq!(size_of::<libc_dirent>(), size_of::<c::dirent>());
    assert_eq!(align_of::<libc_dirent>(), align_of::<c::dirent>());

    // Check that the field offsets match.
    assert_eq!(
        {
            let z = libc_dirent {
                d_fileno: 0_u64,
                d_off: 0_i64,
                d_reclen: 0_u16,
                d_type: 0_u8,
                d_namlen: 0_u8,
                __d_padding: [0_u8; 4],
                d_name: [0 as c::c_char; 256],
            };
            let base = as_ptr(&z) as usize;
            (
                (as_ptr(&z.d_fileno) as usize) - base,
                (as_ptr(&z.d_off) as usize) - base,
                (as_ptr(&z.d_reclen) as usize) - base,
                (as_ptr(&z.d_type) as usize) - base,
                (as_ptr(&z.d_namlen) as usize) - base,
                (as_ptr(&z.d_name) as usize) - base,
            )
        },
        {
            let z = dirent;
            let base = as_ptr(z) as usize;
            (
                (as_ptr(&z.d_fileno) as usize) - base,
                (as_ptr(&z.d_off) as usize) - base,
                (as_ptr(&z.d_reclen) as usize) - base,
                (as_ptr(&z.d_type) as usize) - base,
                (as_ptr(&z.d_namlen) as usize) - base,
                (as_ptr(&z.d_name) as usize) - base,
            )
        }
    );
}

#[test]
fn dir_iterator_handles_io_errors() {
    // create a dir, keep the FD, then delete the dir
    let tmp = tempfile::tempdir().unwrap();
    let fd = crate::fs::openat(
        crate::fs::cwd(),
        tmp.path(),
        crate::fs::OFlags::RDONLY | crate::fs::OFlags::CLOEXEC,
        crate::fs::Mode::empty(),
    )
    .unwrap();

    let file_fd = crate::fs::openat(
        &fd,
        tmp.path().join("test.txt"),
        crate::fs::OFlags::WRONLY | crate::fs::OFlags::CREATE,
        crate::fs::Mode::RWXU,
    )
    .unwrap();

    let mut dir = Dir::read_from(&fd).unwrap();

    // Reach inside the `Dir` and replace its directory with a file, which
    // will cause the subsequent `readdir` to fail.
    unsafe {
        let raw_fd = c::dirfd(dir.libc_dir.as_ptr());
        let mut owned_fd: crate::fd::OwnedFd = crate::fd::FromRawFd::from_raw_fd(raw_fd);
        crate::io::dup2(&file_fd, &mut owned_fd).unwrap();
        core::mem::forget(owned_fd);
    }

    // FreeBSD and macOS seem to read some directory entries before we call
    // `.next()`.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "watchos",
        target_os = "tvos",
        target_os = "freebsd",
        target_os = "dragonflybsd"
    ))]
    {
        dir.rewind();
    }

    assert!(matches!(dir.next(), Some(Err(_))));
    assert!(matches!(dir.next(), None));
}
