use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub fn build(
    _workdir: &Path,
    dirs: &[PathBuf],
    files: &[PathBuf],
    histfile: Option<&Path>,
) -> Result<String> {
    let mut out = String::from(concat!(
        "(version 1)\n",
        "(deny default)\n",
        "(import \"system.sb\")\n",
        "\n",
        "; behave like a normal shell, but only allow writes inside WORKDIR\n",
        "(allow process*)\n",
        "(allow network*)\n",
        "(allow sysctl-read)\n",
        "(allow file-read*)\n",
        "\n",
        "; common special files and tty ioctls for interactive tools\n",
        "(allow file-read-data\n",
        "    (literal \"/dev/random\")\n",
        "    (literal \"/dev/urandom\"))\n",
        "(allow file-read-data file-write-data file-ioctl\n",
        "    (literal \"/dev/null\")\n",
        "    (literal \"/dev/tty\")\n",
        "    (literal (param \"TTY\")))\n",
        "\n",
        "; the writable places are rooted under the launch directory and any configured extras\n",
        "(allow file-write*\n",
        "    (subpath (param \"WORKDIR\"))\n",
    ));
    for dir in dirs {
        out.push_str(&format!("    (subpath \"{}\")\n", escape(dir)?));
    }
    out.push_str(")\n");

    if histfile.is_some() || !files.is_empty() {
        out.push_str("(allow file-write*\n");
        if histfile.is_some() {
            out.push_str("    (literal (param \"HISTFILE\"))\n");
        }
        for file in files {
            out.push_str(&format!("    (literal \"{}\")\n", escape(file)?));
        }
        out.push_str(")\n");
    }

    Ok(out)
}

fn escape(path: &Path) -> Result<String> {
    let text = path.to_string_lossy();
    if text.contains('\n') || text.contains('\r') {
        return Err(Error::PathContainsNewline(text.into_owned()));
    }
    Ok(text.replace('\\', "\\\\").replace('"', "\\\""))
}
