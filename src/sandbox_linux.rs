use std::{
    ffi::CString,
    fs,
    path::{Path, PathBuf},
    ptr,
};

use crate::error::{Error, Result};

pub fn apply(
    workdir: &Path,
    write_dirs: &[PathBuf],
    write_files: &[PathBuf],
) -> Result<()> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    check(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) }, "prctl(NO_NEW_PRIVS)")?;
    check(unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS) }, "unshare")?;

    fs::write("/proc/self/setgroups", "deny")
        .map_err(|e| Error::io_path("write", Path::new("/proc/self/setgroups"), e))?;
    fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))
        .map_err(|e| Error::io_path("write", Path::new("/proc/self/uid_map"), e))?;
    fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))
        .map_err(|e| Error::io_path("write", Path::new("/proc/self/gid_map"), e))?;

    check(unsafe { libc::mount(ptr::null(), c("/")?.as_ptr(), ptr::null(),
        libc::MS_PRIVATE | libc::MS_REC, ptr::null()) }, "make-private")?;

    bind(Path::new("/"), Path::new("/"), true)?;
    tmpfs(Path::new("/tmp"))?;

    for d in std::iter::once(workdir).chain(write_dirs.iter().map(|p| p.as_path())) {
        let _ = fs::create_dir_all(d);
        bind(d, d, false)?;
    }
    for f in write_files {
        bind(f, f, false)?;
    }

    // Try to remount /proc for the new namespace (non-fatal)
    let _ = unsafe { libc::mount(
        c("proc")?.as_ptr(), c("/proc")?.as_ptr(), c("proc")?.as_ptr(),
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC, ptr::null()) };

    let cwd = c_path(workdir)?;
    check(unsafe { libc::chdir(cwd.as_ptr()) }, "chdir")?;

    Ok(())
}

fn bind(src: &Path, dest: &Path, readonly: bool) -> Result<()> {
    let s = c_path(src)?;
    let d = c_path(dest)?;
    if unsafe { libc::mount(s.as_ptr(), d.as_ptr(), ptr::null(),
        libc::MS_BIND | libc::MS_REC, ptr::null()) } != 0 {
        return Err(Error::Sandbox(format!("bind-mount {}: {}", src.display(), std::io::Error::last_os_error())));
    }
    let mut flags = libc::MS_BIND | libc::MS_REMOUNT | libc::MS_REC | libc::MS_NOSUID | libc::MS_NODEV;
    if readonly { flags |= libc::MS_RDONLY; }
    if unsafe { libc::mount(ptr::null(), d.as_ptr(), ptr::null(), flags, ptr::null()) } != 0 {
        return Err(Error::Sandbox(format!("remount {}: {}", dest.display(), std::io::Error::last_os_error())));
    }
    Ok(())
}

fn tmpfs(dest: &Path) -> Result<()> {
    let d = c_path(dest)?;
    check(unsafe { libc::mount(
        c("tmpfs")?.as_ptr(), d.as_ptr(), c("tmpfs")?.as_ptr(),
        libc::MS_NOSUID | libc::MS_NODEV, c("size=256M")?.as_ptr().cast()) },
        "tmpfs")
}

fn check(ret: libc::c_int, action: &'static str) -> Result<()> {
    if ret != 0 { return Err(Error::io(action, std::io::Error::last_os_error())); }
    Ok(())
}

fn c(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::Usage("path contains NUL".into()))
}

fn c_path(p: &Path) -> Result<CString> {
    CString::new(p.as_os_str().as_encoded_bytes())
        .map_err(|_| Error::Usage("path contains NUL".into()))
}
