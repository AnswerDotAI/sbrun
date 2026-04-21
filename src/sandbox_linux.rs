use std::{
    ffi::CString,
    fs,
    path::{Path, PathBuf},
    ptr,
};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivilegeMode {
    Unprivileged,
    Privileged,
}

pub struct PreflightPrivilegeGuard;

pub fn temporarily_drop_to_real_user() -> Result<Option<PreflightPrivilegeGuard>> {
    if !needs_preflight_drop(current_ids()) {
        return Ok(None);
    }
    let uid = unsafe { libc::getuid() };
    check(unsafe { libc::seteuid(uid) }, "seteuid(real uid)")?;
    Ok(Some(PreflightPrivilegeGuard))
}

impl PreflightPrivilegeGuard {
    pub fn restore_root(self) -> Result<()> {
        check(unsafe { libc::seteuid(0) }, "seteuid(0)")?;
        Ok(())
    }
}

pub fn apply(workdir: &Path, write_dirs: &[PathBuf], write_files: &[PathBuf]) -> Result<()> {
    match mode_from_ids(current_ids()) {
        PrivilegeMode::Unprivileged => apply_unprivileged(workdir, write_dirs, write_files),
        PrivilegeMode::Privileged => apply_privileged(workdir, write_dirs, write_files),
    }
}

fn apply_unprivileged(
    workdir: &Path,
    write_dirs: &[PathBuf],
    write_files: &[PathBuf],
) -> Result<()> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    check(
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
        "prctl(NO_NEW_PRIVS)",
    )?;
    check(
        unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS) },
        "unshare",
    )?;

    fs::write("/proc/self/setgroups", "deny")
        .map_err(|e| Error::io_path("write", Path::new("/proc/self/setgroups"), e))?;
    fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))
        .map_err(|e| Error::io_path("write", Path::new("/proc/self/uid_map"), e))?;
    fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))
        .map_err(|e| Error::io_path("write", Path::new("/proc/self/gid_map"), e))?;

    check(
        unsafe {
            libc::mount(
                ptr::null(),
                c("/")?.as_ptr(),
                ptr::null(),
                libc::MS_PRIVATE | libc::MS_REC,
                ptr::null(),
            )
        },
        "make-private",
    )?;

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
    let _ = unsafe {
        libc::mount(
            c("proc")?.as_ptr(),
            c("/proc")?.as_ptr(),
            c("proc")?.as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
            ptr::null(),
        )
    };

    let cwd = c_path(workdir)?;
    check(unsafe { libc::chdir(cwd.as_ptr()) }, "chdir")?;

    Ok(())
}

fn apply_privileged(workdir: &Path, write_dirs: &[PathBuf], write_files: &[PathBuf]) -> Result<()> {
    check(unsafe { libc::unshare(libc::CLONE_NEWNS) }, "unshare")?;
    check(
        unsafe {
            libc::mount(
                ptr::null(),
                c("/")?.as_ptr(),
                ptr::null(),
                libc::MS_PRIVATE | libc::MS_REC,
                ptr::null(),
            )
        },
        "make-private",
    )?;

    bind(Path::new("/"), Path::new("/"), true)?;
    tmpfs(Path::new("/tmp"))?;

    for d in std::iter::once(workdir).chain(write_dirs.iter().map(|p| p.as_path())) {
        let _ = fs::create_dir_all(d);
        bind(d, d, false)?;
    }
    for f in write_files {
        bind(f, f, false)?;
    }

    let _ = unsafe {
        libc::mount(
            c("proc")?.as_ptr(),
            c("/proc")?.as_ptr(),
            c("proc")?.as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
            ptr::null(),
        )
    };

    permanently_drop_privileges()?;
    check(
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
        "prctl(NO_NEW_PRIVS)",
    )?;

    let cwd = c_path(workdir)?;
    check(unsafe { libc::chdir(cwd.as_ptr()) }, "chdir")?;

    Ok(())
}

fn permanently_drop_privileges() -> Result<()> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    check(unsafe { libc::setresgid(gid, gid, gid) }, "setresgid")?;
    check(unsafe { libc::setresuid(uid, uid, uid) }, "setresuid")?;

    if unsafe { libc::geteuid() } != uid || unsafe { libc::getegid() } != gid {
        return Err(Error::Sandbox(
            "failed to drop privileges permanently".into(),
        ));
    }
    Ok(())
}

fn current_ids() -> (libc::uid_t, libc::uid_t) {
    (unsafe { libc::getuid() }, unsafe { libc::geteuid() })
}

fn mode_from_ids((_, euid): (libc::uid_t, libc::uid_t)) -> PrivilegeMode {
    if euid == 0 {
        PrivilegeMode::Privileged
    } else {
        PrivilegeMode::Unprivileged
    }
}

fn needs_preflight_drop((uid, euid): (libc::uid_t, libc::uid_t)) -> bool {
    mode_from_ids((uid, euid)) == PrivilegeMode::Privileged && uid != 0
}

const AT_RECURSIVE: libc::c_int = 0x8000;
const MOUNT_ATTR_RDONLY: u64 = 0x1;
const MOUNT_ATTR_NOSUID: u64 = 0x2;

#[repr(C)]
struct MountAttr {
    attr_set: u64,
    attr_clr: u64,
    propagation: u64,
    userns_fd: u64,
}

fn mount_setattr_ro_rec(path: &Path) -> Result<()> {
    let p = c_path(path)?;
    let attr = MountAttr {
        attr_set: MOUNT_ATTR_RDONLY | MOUNT_ATTR_NOSUID,
        attr_clr: 0,
        propagation: 0,
        userns_fd: 0,
    };
    let rc = unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            libc::AT_FDCWD,
            p.as_ptr(),
            AT_RECURSIVE,
            &attr as *const _,
            std::mem::size_of::<MountAttr>(),
        )
    };
    if rc != 0 {
        return Err(Error::Sandbox(format!(
            "mount_setattr {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

fn bind(src: &Path, dest: &Path, readonly: bool) -> Result<()> {
    let s = c_path(src)?;
    let d = c_path(dest)?;
    if unsafe {
        libc::mount(
            s.as_ptr(),
            d.as_ptr(),
            ptr::null(),
            libc::MS_BIND | libc::MS_REC,
            ptr::null(),
        )
    } != 0
    {
        return Err(Error::Sandbox(format!(
            "bind-mount {}: {}",
            src.display(),
            std::io::Error::last_os_error()
        )));
    }
    if readonly {
        return mount_setattr_ro_rec(dest);
    }
    let flags = libc::MS_BIND | libc::MS_REMOUNT | libc::MS_REC | libc::MS_NOSUID | libc::MS_NODEV;
    if unsafe { libc::mount(ptr::null(), d.as_ptr(), ptr::null(), flags, ptr::null()) } != 0 {
        return Err(Error::Sandbox(format!(
            "remount {}: {}",
            dest.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

fn tmpfs(dest: &Path) -> Result<()> {
    let d = c_path(dest)?;
    check(
        unsafe {
            libc::mount(
                c("tmpfs")?.as_ptr(),
                d.as_ptr(),
                c("tmpfs")?.as_ptr(),
                libc::MS_NOSUID | libc::MS_NODEV,
                c("size=256M")?.as_ptr().cast(),
            )
        },
        "tmpfs",
    )
}

fn check(ret: libc::c_int, action: &'static str) -> Result<()> {
    if ret != 0 {
        return Err(Error::io(action, std::io::Error::last_os_error()));
    }
    Ok(())
}

fn c(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::Usage("path contains NUL".into()))
}

fn c_path(p: &Path) -> Result<CString> {
    CString::new(p.as_os_str().as_encoded_bytes())
        .map_err(|_| Error::Usage("path contains NUL".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_root_uses_unprivileged_backend() {
        assert_eq!(mode_from_ids((1000, 1000)), PrivilegeMode::Unprivileged);
        assert!(!needs_preflight_drop((1000, 1000)));
    }

    #[test]
    fn setuid_root_uses_privileged_backend() {
        assert_eq!(mode_from_ids((1000, 0)), PrivilegeMode::Privileged);
        assert!(needs_preflight_drop((1000, 0)));
    }

    #[test]
    fn root_uses_privileged_backend_without_preflight_drop() {
        assert_eq!(mode_from_ids((0, 0)), PrivilegeMode::Privileged);
        assert!(!needs_preflight_drop((0, 0)));
    }
}
