use std::{fs, path::Path, process::Command};

use crate::error::{Error, Result};

#[cfg(target_os = "linux")]
const SYSCTL_CONFIG_PATH: &str = "/etc/sysctl.d/90-sbrun.conf";
#[cfg(target_os = "linux")]
const SYSCTL_CONFIG: &str =
    "kernel.unprivileged_userns_clone=1\nkernel.apparmor_restrict_unprivileged_userns=0\n";

pub fn kernel_install() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        ensure_kernel_install_allowed()?;
        let config_path = Path::new(SYSCTL_CONFIG_PATH);
        fs::write(config_path, SYSCTL_CONFIG)
            .map_err(|err| Error::io_path("write", config_path, err))?;

        let status = Command::new(sysctl_program())
            .arg("--system")
            .status()
            .map_err(|err| Error::io("run sysctl --system", err))?;
        if !status.success() {
            return Err(Error::Usage(format!("sysctl --system failed: {status}")));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error::Usage(
            "--kernel-install is only available on Linux".into(),
        ))
    }
}

#[cfg(target_os = "linux")]
fn ensure_kernel_install_allowed() -> Result<()> {
    if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
        return Err(Error::Usage(
            "--kernel-install requires running as root (for example via sudo)".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn sysctl_program() -> &'static str {
    ["/usr/sbin/sysctl", "/sbin/sysctl", "sysctl"]
        .into_iter()
        .find(|candidate| !candidate.starts_with('/') || Path::new(candidate).exists())
        .unwrap_or("sysctl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn kernel_install_requires_root_on_linux() {
        if unsafe { libc::getuid() } == 0 && unsafe { libc::geteuid() } == 0 {
            return;
        }
        let err = ensure_kernel_install_allowed().unwrap_err();
        assert!(
            err.to_string()
                .contains("--kernel-install requires running as root")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kernel_install_sysctl_config_has_required_keys() {
        assert!(SYSCTL_CONFIG.contains("kernel.unprivileged_userns_clone=1"));
        assert!(SYSCTL_CONFIG.contains("kernel.apparmor_restrict_unprivileged_userns=0"));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn kernel_install_is_linux_only() {
        let err = kernel_install().unwrap_err();
        assert!(err.to_string().contains("only available on Linux"));
    }
}
