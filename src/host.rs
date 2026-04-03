use std::{
    env,
    ffi::{CStr, OsStr, OsString},
    io,
    mem::MaybeUninit,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Host {
    pub home: Option<PathBuf>,
    pub shell: PathBuf,
    pub user: Option<OsString>,
}

pub fn current() -> Result<Host> {
    let uid = unsafe { libc::getuid() };
    let mut pwd = MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0_u8; 4096];
    let mut result = std::ptr::null_mut();

    loop {
        let err = unsafe {
            libc::getpwuid_r(
                uid,
                pwd.as_mut_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
        if err == 0 {
            break;
        }
        if err == libc::ERANGE {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        return Err(Error::io("getpwuid_r", io::Error::from_raw_os_error(err)));
    }

    let (home, passwd_shell, user) = if result.is_null() {
        (None, None, None)
    } else {
        let pwd = unsafe { pwd.assume_init() };
        (
            c_path(pwd.pw_dir),
            c_path(pwd.pw_shell),
            c_os_string(pwd.pw_name),
        )
    };

    let shell = pick_shell(env::var_os("SHELL").map(PathBuf::from), passwd_shell)?;
    Ok(Host { home, shell, user })
}

pub fn history_file_name(shell: &Path) -> &'static str {
    match shell.file_name().and_then(OsStr::to_str) {
        Some("bash") => ".bash_history",
        Some("zsh") => ".zsh_history",
        _ => ".sh_history",
    }
}

pub fn shell_is_bash(shell: &Path) -> bool {
    shell.file_name().and_then(OsStr::to_str) == Some("bash")
}

pub fn login_arg0(shell: &Path) -> OsString {
    let name = shell
        .file_name()
        .unwrap_or_else(|| OsStr::new("sh"))
        .as_bytes();
    let mut bytes = Vec::with_capacity(name.len() + 1);
    bytes.push(b'-');
    bytes.extend_from_slice(name);
    OsString::from_vec(bytes)
}

#[cfg(target_os = "macos")]
pub fn tty_path() -> PathBuf {
    for fd in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        if let Some(path) = tty_path_for_fd(fd) {
            return path;
        }
    }
    PathBuf::from("/dev/tty")
}

pub fn close_extra_fds() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return;
    }
    let maxfd = limit.rlim_cur.min(65_536);
    for fd in 3..maxfd as i32 {
        unsafe {
            libc::close(fd);
        }
    }
}

fn pick_shell(env_shell: Option<PathBuf>, passwd_shell: Option<PathBuf>) -> Result<PathBuf> {
    for candidate in [env_shell, passwd_shell, Some(PathBuf::from("/bin/bash"))]
        .into_iter()
        .flatten()
    {
        if candidate.is_absolute() && is_executable(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(Error::Usage(
        "could not find an executable shell from $SHELL, passwd entry, or /bin/bash".into(),
    ))
}

fn is_executable(path: &Path) -> Result<bool> {
    let c_path = crate::pathutil::path_to_cstring(path)?;
    Ok(unsafe { libc::access(c_path.as_ptr(), libc::X_OK) } == 0)
}

#[cfg(target_os = "macos")]
fn tty_path_for_fd(fd: i32) -> Option<PathBuf> {
    let mut buf = vec![0 as libc::c_char; 1024];
    if unsafe { libc::ttyname_r(fd, buf.as_mut_ptr(), buf.len()) } != 0 || buf[0] == 0 {
        return None;
    }
    let bytes = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_bytes().to_vec();
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

fn c_path(ptr: *const libc::c_char) -> Option<PathBuf> {
    c_os_string(ptr).map(PathBuf::from)
}

fn c_os_string(ptr: *const libc::c_char) -> Option<OsString> {
    if ptr.is_null() {
        return None;
    }
    let bytes = unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec();
    Some(OsString::from_vec(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn history_bash() { assert_eq!(history_file_name(Path::new("/bin/bash")), ".bash_history"); }

    #[test]
    fn history_zsh() { assert_eq!(history_file_name(Path::new("/bin/zsh")), ".zsh_history"); }

    #[test]
    fn history_other() { assert_eq!(history_file_name(Path::new("/bin/fish")), ".sh_history"); }

    #[test]
    fn bash_detection() {
        assert!(shell_is_bash(Path::new("/bin/bash")));
        assert!(!shell_is_bash(Path::new("/bin/zsh")));
    }

    #[test]
    fn login_arg0_prefix() {
        assert_eq!(login_arg0(Path::new("/bin/bash")), OsString::from("-bash"));
        assert_eq!(login_arg0(Path::new("/usr/bin/zsh")), OsString::from("-zsh"));
    }

    #[test]
    fn current_host_succeeds() {
        let host = current().unwrap();
        assert!(host.shell.is_absolute());
    }
}
