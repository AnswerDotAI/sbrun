use std::{
    ffi::{CString, OsString},
    fs,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    ptr,
};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivilegeMode {
    Unprivileged,
    Privileged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MountEntry {
    mountpoint: PathBuf,
    options: libc::c_ulong,
    filesystem_type: String,
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

fn bind(src: &Path, dest: &Path, readonly: bool) -> Result<()> {
    let submounts = if readonly {
        parse_mountinfo()?
            .into_iter()
            .filter(|m| is_submount_of(&m.mountpoint, dest))
            .collect()
    } else {
        Vec::new()
    };
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
    let mut flags =
        libc::MS_BIND | libc::MS_REMOUNT | libc::MS_REC | libc::MS_NOSUID | libc::MS_NODEV;
    if readonly {
        flags |= libc::MS_RDONLY;
    }
    if unsafe { libc::mount(ptr::null(), d.as_ptr(), ptr::null(), flags, ptr::null()) } != 0 {
        return Err(Error::Sandbox(format!(
            "remount {}: {}",
            dest.display(),
            std::io::Error::last_os_error()
        )));
    }
    if readonly {
        remount_submounts_readonly(&submounts)?;
    }
    Ok(())
}

fn parse_mountinfo() -> Result<Vec<MountEntry>> {
    let data = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|e| Error::io_path("read", Path::new("/proc/self/mountinfo"), e))?;
    Ok(parse_mountinfo_str(&data))
}

fn parse_mountinfo_str(data: &str) -> Vec<MountEntry> {
    data.lines().filter_map(parse_mountinfo_line).collect()
}

fn parse_mountinfo_line(line: &str) -> Option<MountEntry> {
    let (fields, mount_source) = line.split_once(" - ")?;
    let mut fields = fields.split_whitespace();
    fields.next()?;
    fields.next()?;
    fields.next()?;
    fields.next()?;
    let mountpoint = unescape_mountinfo_path(fields.next()?);
    let options = vfs_options_to_mount_flags(fields.next()?);
    let filesystem_type = mount_source.split_whitespace().next()?.to_owned();
    Some(MountEntry {
        mountpoint,
        options,
        filesystem_type,
    })
}

fn unescape_mountinfo_path(raw: &str) -> PathBuf {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            if let (Some(a), Some(b), Some(c)) = (
                octal_digit(bytes[i + 1]),
                octal_digit(bytes[i + 2]),
                octal_digit(bytes[i + 3]),
            ) {
                out.push((a << 6) | (b << 3) | c);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(OsString::from_vec(out))
}

fn octal_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'7' => Some(byte - b'0'),
        _ => None,
    }
}

fn vfs_options_to_mount_flags(options: &str) -> libc::c_ulong {
    let mut flags = 0;
    for option in options.split(',') {
        match option {
            "ro" => flags |= libc::MS_RDONLY,
            "nosuid" => flags |= libc::MS_NOSUID,
            "nodev" => flags |= libc::MS_NODEV,
            "noexec" => flags |= libc::MS_NOEXEC,
            "noatime" => flags |= libc::MS_NOATIME,
            "relatime" => flags |= libc::MS_RELATIME,
            _ => {}
        }
    }
    flags
}

fn is_submount_of(mountpoint: &Path, root: &Path) -> bool {
    mountpoint != root && mountpoint.starts_with(root)
}

fn readonly_submount_remount_flags(submount: &MountEntry) -> Option<libc::c_ulong> {
    let mut required_flags = libc::MS_RDONLY | libc::MS_NOSUID;
    if !is_device_mount(submount) {
        required_flags |= libc::MS_NODEV;
    }

    let new_flags = submount.options | required_flags;
    if new_flags == submount.options {
        None
    } else {
        Some(libc::MS_BIND | libc::MS_REMOUNT | new_flags)
    }
}

fn is_device_mount(submount: &MountEntry) -> bool {
    matches!(submount.filesystem_type.as_str(), "devtmpfs" | "devpts")
        || submount.mountpoint == Path::new("/dev")
        || submount.mountpoint.starts_with("/dev/")
}

fn remount_submounts_readonly(submounts: &[MountEntry]) -> Result<()> {
    for submount in submounts {
        let Some(flags) = readonly_submount_remount_flags(submount) else {
            continue;
        };
        let mountpoint = c_path(&submount.mountpoint)?;
        if unsafe {
            libc::mount(
                ptr::null(),
                mountpoint.as_ptr(),
                ptr::null(),
                flags,
                ptr::null(),
            )
        } != 0
        {
            let err = std::io::Error::last_os_error();
            if should_ignore_submount_remount_errno(err.raw_os_error(), submount) {
                continue;
            }
            return Err(Error::Sandbox(format!(
                "remount submount {}: {err}",
                submount.mountpoint.display()
            )));
        }
    }
    Ok(())
}

fn should_ignore_submount_remount_errno(errno: Option<i32>, submount: &MountEntry) -> bool {
    errno == Some(libc::EACCES) || (errno == Some(libc::EPERM) && is_kernel_api_mount(submount))
}

fn is_kernel_api_mount(submount: &MountEntry) -> bool {
    matches!(
        submount.filesystem_type.as_str(),
        "autofs"
            | "binfmt_misc"
            | "bpf"
            | "cgroup"
            | "cgroup2"
            | "configfs"
            | "debugfs"
            | "devpts"
            | "devtmpfs"
            | "efivarfs"
            | "fusectl"
            | "mqueue"
            | "proc"
            | "pstore"
            | "securityfs"
            | "sysfs"
            | "tracefs"
    ) || submount.mountpoint.starts_with("/proc")
        || submount.mountpoint.starts_with("/sys")
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

    #[test]
    fn parses_mountinfo_mountpoints_and_vfs_options() {
        let mounts = parse_mountinfo_str(
            "36 35 0:27 / / rw,relatime shared:1 - btrfs /dev/sda rw,subvol=/@\n\
             37 36 0:28 / /home rw,nosuid,nodev,noatime shared:1 - btrfs /dev/sda rw,subvol=/@home\n\
             38 36 0:29 / /space\\040dir ro,nosuid,nodev,noexec,relatime - ext4 /dev/sdb rw\n",
        );

        assert_eq!(mounts.len(), 3);
        assert_eq!(mounts[0].mountpoint, PathBuf::from("/"));
        assert_eq!(mounts[0].filesystem_type, "btrfs");
        assert_eq!(mounts[0].options & libc::MS_RDONLY, 0);
        assert_ne!(mounts[0].options & libc::MS_RELATIME, 0);

        assert_eq!(mounts[1].mountpoint, PathBuf::from("/home"));
        assert_eq!(mounts[1].filesystem_type, "btrfs");
        assert_ne!(mounts[1].options & libc::MS_NOSUID, 0);
        assert_ne!(mounts[1].options & libc::MS_NODEV, 0);
        assert_ne!(mounts[1].options & libc::MS_NOATIME, 0);

        assert_eq!(mounts[2].mountpoint, PathBuf::from("/space dir"));
        assert_eq!(mounts[2].filesystem_type, "ext4");
        assert_ne!(mounts[2].options & libc::MS_RDONLY, 0);
        assert_ne!(mounts[2].options & libc::MS_NOEXEC, 0);
    }

    #[test]
    fn submount_matching_uses_path_boundaries() {
        assert!(is_submount_of(
            Path::new("/home/natedawg"),
            Path::new("/home")
        ));
        assert!(is_submount_of(Path::new("/home"), Path::new("/")));
        assert!(!is_submount_of(Path::new("/home"), Path::new("/home")));
        assert!(!is_submount_of(Path::new("/home2"), Path::new("/home")));
        assert!(!is_submount_of(Path::new("/var/log"), Path::new("/home")));
    }

    #[test]
    fn readonly_submount_flags_only_add_required_flags() {
        let storage = MountEntry {
            mountpoint: PathBuf::from("/home"),
            options: libc::MS_RELATIME | libc::MS_NOEXEC,
            filesystem_type: "btrfs".to_owned(),
        };
        let flags = readonly_submount_remount_flags(&storage)
            .expect("readonly/nosuid/nodev should be added");

        assert_ne!(flags & libc::MS_BIND, 0);
        assert_ne!(flags & libc::MS_REMOUNT, 0);
        assert_ne!(flags & libc::MS_RDONLY, 0);
        assert_ne!(flags & libc::MS_NOSUID, 0);
        assert_ne!(flags & libc::MS_NODEV, 0);
        assert_ne!(flags & libc::MS_RELATIME, 0);
        assert_ne!(flags & libc::MS_NOEXEC, 0);

        let already_readonly = MountEntry {
            mountpoint: PathBuf::from("/home"),
            options: libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
            filesystem_type: "btrfs".to_owned(),
        };
        assert_eq!(readonly_submount_remount_flags(&already_readonly), None);
    }

    #[test]
    fn device_submount_flags_preserve_device_node_access() {
        let dev = MountEntry {
            mountpoint: PathBuf::from("/dev"),
            options: libc::MS_NOSUID | libc::MS_RELATIME,
            filesystem_type: "devtmpfs".to_owned(),
        };
        let flags =
            readonly_submount_remount_flags(&dev).expect("readonly should be added to /dev");

        assert_ne!(flags & libc::MS_RDONLY, 0);
        assert_ne!(flags & libc::MS_NOSUID, 0);
        assert_eq!(flags & libc::MS_NODEV, 0);
    }

    #[test]
    fn ignores_permission_errors_only_for_expected_submounts() {
        let home = MountEntry {
            mountpoint: PathBuf::from("/home"),
            options: 0,
            filesystem_type: "btrfs".to_owned(),
        };
        let binfmt_misc = MountEntry {
            mountpoint: PathBuf::from("/proc/sys/fs/binfmt_misc"),
            options: 0,
            filesystem_type: "binfmt_misc".to_owned(),
        };
        let proc_autofs = MountEntry {
            mountpoint: PathBuf::from("/proc/sys/fs/binfmt_misc"),
            options: 0,
            filesystem_type: "autofs".to_owned(),
        };

        assert!(should_ignore_submount_remount_errno(
            Some(libc::EACCES),
            &home
        ));
        assert!(!should_ignore_submount_remount_errno(
            Some(libc::EPERM),
            &home
        ));
        assert!(should_ignore_submount_remount_errno(
            Some(libc::EPERM),
            &binfmt_misc
        ));
        assert!(should_ignore_submount_remount_errno(
            Some(libc::EPERM),
            &proc_autofs
        ));
    }
}
