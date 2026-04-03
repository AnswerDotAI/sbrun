use std::{
    ffi::{CString, OsStr, OsString},
    fs, io,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use crate::error::{Error, Result};

#[derive(Debug, Default)]
pub struct AllowedWrites {
    pub dirs: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

pub fn ensure_real_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(Error::Usage(format!(
                    "{} exists and is a symlink; refusing to use it",
                    path.display()
                )));
            }
            if !meta.is_dir() {
                return Err(Error::Usage(format!(
                    "{} exists and is not a directory",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => ensure_real_directory(path),
            Err(err) => Err(Error::io_path("create directory", path, err)),
        },
        Err(err) => Err(Error::io_path("stat directory", path, err)),
    }
}

pub fn expand_home(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let bytes = path.as_os_str().as_bytes();
    if !bytes.starts_with(b"~") {
        return Ok(path.to_path_buf());
    }
    let Some(home) = home else {
        return Err(Error::MissingHomeDirectory {
            path: path.display().to_string(),
        });
    };
    if bytes == b"~" {
        return Ok(home.to_path_buf());
    }
    if !bytes.starts_with(b"~/") {
        return Err(Error::UnsupportedHomeExpansion(path.display().to_string()));
    }
    let mut out = home.to_path_buf();
    out.push(OsString::from_vec(bytes[2..].to_vec()));
    Ok(out)
}

pub fn resolve_writes(
    required: &[PathBuf],
    optional: &[PathBuf],
    home: Option<&Path>,
) -> Result<AllowedWrites> {
    let mut out = AllowedWrites::default();
    for path in required {
        if let Some(target) = resolve_write_path(path, home, false)? {
            push_unique(&mut out, target);
        }
    }
    for path in optional {
        if let Some(target) = resolve_write_path(path, home, true)? {
            push_unique(&mut out, target);
        }
    }
    Ok(out)
}

pub fn refuse_redirected_regular_stdio(workdir: &Path, allowed: &AllowedWrites) -> Result<()> {
    let override_enabled =
        std::env::var_os("SBRUN_ALLOW_STDIO_REDIRECTS").as_deref() == Some(OsStr::new("1"));
    if override_enabled {
        return Ok(());
    }

    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
            continue;
        }
        let stat = unsafe { stat.assume_init() };
        if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
            continue;
        }

        let final_path = match fd_path(fd) {
            Some(path) => path,
            None => return Err(Error::Usage(format!(
                "fd {fd} is redirected to a regular file outside the sandbox check path; refusing to start"
            ))),
        };
        if !path_is_allowed(&final_path, workdir, allowed) {
            return Err(Error::Usage(format!(
                "fd {fd} is redirected to {} outside allowed writable paths; refusing to start (set SBRUN_ALLOW_STDIO_REDIRECTS=1 to override)",
                final_path.display()
            )));
        }
    }
    Ok(())
}

pub fn path_to_cstring(path: &Path) -> Result<CString> {
    os_str_to_cstring(path.as_os_str())
}

pub fn os_str_to_cstring(value: &OsStr) -> Result<CString> {
    CString::new(value.as_bytes().to_vec())
        .map_err(|_| Error::Usage("paths and arguments cannot contain NUL bytes".into()))
}

fn resolve_write_path(
    path: &Path,
    home: Option<&Path>,
    optional: bool,
) -> Result<Option<ResolvedTarget>> {
    let expanded = expand_home(path, home)?;
    let resolved = match fs::canonicalize(&expanded) {
        Ok(path) => path,
        Err(err)
            if optional && matches!(err.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(Error::io_path("resolve writable path", &expanded, err)),
    };

    let meta = match fs::metadata(&resolved) {
        Ok(meta) => meta,
        Err(err) if optional && err.raw_os_error() == Some(libc::ENOENT) => return Ok(None),
        Err(err) => return Err(Error::io_path("stat writable path", &resolved, err)),
    };
    if meta.is_dir() {
        return Ok(Some(ResolvedTarget::Dir(resolved)));
    }
    if meta.is_file() {
        return Ok(Some(ResolvedTarget::File(resolved)));
    }
    if optional {
        return Ok(None);
    }
    Err(Error::Usage(format!(
        "writable path {} resolves to {}, which is not a regular file or directory",
        path.display(),
        resolved.display()
    )))
}

fn push_unique(allowed: &mut AllowedWrites, target: ResolvedTarget) {
    match target {
        ResolvedTarget::Dir(path) => {
            if !allowed.dirs.iter().any(|existing| existing == &path) {
                allowed.dirs.push(path);
            }
        }
        ResolvedTarget::File(path) => {
            if !allowed.files.iter().any(|existing| existing == &path) {
                allowed.files.push(path);
            }
        }
    }
}

fn path_is_allowed(target: &Path, workdir: &Path, allowed: &AllowedWrites) -> bool {
    target.starts_with(workdir)
        || allowed.dirs.iter().any(|dir| target.starts_with(dir))
        || allowed.files.iter().any(|file| target == file)
}

enum ResolvedTarget {
    Dir(PathBuf),
    File(PathBuf),
}

#[cfg(target_os = "macos")]
fn fd_path(fd: i32) -> Option<PathBuf> {
    let mut raw = vec![0_i8; libc::PATH_MAX as usize];
    if unsafe { libc::fcntl(fd, libc::F_GETPATH, raw.as_mut_ptr()) } == -1 || raw[0] == 0 {
        return None;
    }
    let raw_path = unsafe { std::ffi::CStr::from_ptr(raw.as_ptr()) }.to_bytes().to_vec();
    Some(fs::canonicalize(PathBuf::from(OsString::from_vec(raw_path.clone())))
        .unwrap_or_else(|_| PathBuf::from(OsString::from_vec(raw_path))))
}

#[cfg(target_os = "linux")]
fn fd_path(fd: i32) -> Option<PathBuf> {
    let link = format!("/proc/self/fd/{fd}");
    fs::read_link(&link).ok().and_then(|p| fs::canonicalize(&p).ok().or(Some(p)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn expand_home_tilde() {
        let home = Path::new("/home/test");
        assert_eq!(expand_home(Path::new("~/foo"), Some(home)).unwrap(), PathBuf::from("/home/test/foo"));
        assert_eq!(expand_home(Path::new("~"), Some(home)).unwrap(), PathBuf::from("/home/test"));
    }

    #[test]
    fn expand_home_absolute_unchanged() {
        assert_eq!(expand_home(Path::new("/tmp/foo"), Some(Path::new("/home/x"))).unwrap(), PathBuf::from("/tmp/foo"));
    }

    #[test]
    fn expand_home_no_home_errors() {
        assert!(expand_home(Path::new("~/foo"), None).is_err());
    }

    #[test]
    fn expand_home_other_user_errors() {
        assert!(expand_home(Path::new("~other/foo"), Some(Path::new("/home/me"))).is_err());
    }

    #[test]
    fn path_allowed_in_workdir() {
        let allowed = AllowedWrites::default();
        assert!(path_is_allowed(Path::new("/work/sub/file"), Path::new("/work"), &allowed));
    }

    #[test]
    fn path_allowed_in_extra_dir() {
        let allowed = AllowedWrites { dirs: vec![PathBuf::from("/extra")], files: vec![] };
        assert!(path_is_allowed(Path::new("/extra/sub"), Path::new("/work"), &allowed));
    }

    #[test]
    fn path_allowed_exact_file() {
        let allowed = AllowedWrites { dirs: vec![], files: vec![PathBuf::from("/etc/hosts")] };
        assert!(path_is_allowed(Path::new("/etc/hosts"), Path::new("/work"), &allowed));
        assert!(!path_is_allowed(Path::new("/etc/passwd"), Path::new("/work"), &allowed));
    }

    #[test]
    fn path_outside_not_allowed() {
        let allowed = AllowedWrites::default();
        assert!(!path_is_allowed(Path::new("/etc/passwd"), Path::new("/work"), &allowed));
    }

    #[test]
    fn push_unique_deduplicates() {
        let mut allowed = AllowedWrites::default();
        push_unique(&mut allowed, ResolvedTarget::Dir(PathBuf::from("/tmp")));
        push_unique(&mut allowed, ResolvedTarget::Dir(PathBuf::from("/tmp")));
        assert_eq!(allowed.dirs.len(), 1);
        push_unique(&mut allowed, ResolvedTarget::File(PathBuf::from("/f")));
        push_unique(&mut allowed, ResolvedTarget::File(PathBuf::from("/f")));
        assert_eq!(allowed.files.len(), 1);
    }
}
