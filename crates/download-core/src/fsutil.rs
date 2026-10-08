//! Platform file-system helpers: positional writes, sparse pre-allocation,
//! free-space queries and cross-volume moves.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Write the whole buffer at `offset` without moving a shared cursor.
pub fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.write_all_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let (mut buf, mut offset) = (buf, offset);
        while !buf.is_empty() {
            match file.seek_write(buf, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write whole buffer",
                    ))
                }
                Ok(n) => {
                    buf = &buf[n..];
                    offset += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Read exactly `buf.len()` bytes at `offset`.
pub fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0usize;
        while done < buf.len() {
            match file.seek_read(&mut buf[done..], offset + done as u64) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read")),
                Ok(n) => done += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Open (creating if needed) the partial file for random-access writing.
pub fn open_partial(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Size the file to `len` bytes.
///
/// On Windows the file is first marked sparse: NTFS otherwise zero-fills
/// the gap synchronously when a segment writes far beyond the current valid
/// data length, which stalls multi-connection downloads of large files.
/// On Unix `set_len` already creates a sparse file.
pub fn preallocate(file: &File, len: u64) -> io::Result<()> {
    #[cfg(windows)]
    {
        let _ = set_sparse(file);
    }
    if file.metadata()?.len() != len {
        file.set_len(len)?;
    }
    Ok(())
}

#[cfg(windows)]
fn set_sparse(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    let mut returned: u32 = 0;
    // SAFETY: valid file handle, no input/output buffers.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Free bytes available to the current user on the volume containing `path`
/// (or its nearest existing ancestor).
pub fn available_space(path: &Path) -> io::Result<u64> {
    let mut p = path;
    while !p.exists() {
        p = match p.parent() {
            Some(parent) => parent,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "no existing ancestor",
                ))
            }
        };
    }
    available_space_impl(p)
}

#[cfg(unix)]
fn available_space_impl(path: &Path) -> io::Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid C string and out-pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    #[allow(clippy::unnecessary_cast)]
    Ok(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(windows)]
fn available_space_impl(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_to_caller: u64 = 0;
    // SAFETY: wide is NUL-terminated; the other out-params may be null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_to_caller,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(free_to_caller)
    }
}

#[cfg(not(any(unix, windows)))]
fn available_space_impl(_path: &Path) -> io::Result<u64> {
    Ok(u64::MAX)
}

/// Rename, falling back to copy + delete across volumes.
pub fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            std::fs::copy(from, to)?;
            File::open(to)?.sync_all()?;
            std::fs::remove_file(from)
        }
        Err(e) => Err(e),
    }
}

fn is_cross_device(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::CrossesDevices {
        return true;
    }
    #[cfg(unix)]
    {
        e.raw_os_error() == Some(libc::EXDEV)
    }
    #[cfg(windows)]
    {
        e.raw_os_error() == Some(17) // ERROR_NOT_SAME_DEVICE
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Make a rename durable by syncing the containing directory (Unix only;
/// NTFS journals metadata itself).
pub fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positional_writes_and_preallocation() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("f.vdpart");
        let f = open_partial(&p).unwrap();
        preallocate(&f, 1000).unwrap();
        write_all_at(&f, b"world", 995).unwrap();
        write_all_at(&f, b"hello", 0).unwrap();
        let mut buf = [0u8; 5];
        read_exact_at(&f, &mut buf, 995).unwrap();
        assert_eq!(&buf, b"world");
        assert_eq!(f.metadata().unwrap().len(), 1000);
        assert!(available_space(&p).unwrap() > 0);
        let dest = dir.path().join("moved").join("f.bin");
        drop(f);
        move_file(&p, &dest).unwrap();
        assert!(dest.exists() && !p.exists());
    }
}
