mod cli;
mod config;
mod error;
mod host;
mod pathutil;
mod profile;
mod sandbox;

use std::{
    collections::HashSet,
    convert::Infallible,
    env,
    ffi::{OsStr, OsString},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyModule};

pub use cli::{Command as CliCommand, help_text, parse as parse_cli};
pub use config::ConfigMode;
pub use error::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub write: Vec<PathBuf>,
    pub env_dir: Vec<String>,
    pub unset_env: Vec<String>,
    pub config: ConfigMode,
}

#[derive(Debug, Clone)]
pub enum RunTarget {
    InteractiveShell,
    ShellCommand(String),
    Exec(Vec<OsString>),
}

pub fn exec<I, S>(argv: I, options: Options) -> Result<Infallible>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    run(
        RunTarget::Exec(argv.into_iter().map(Into::into).collect()),
        options,
    )
}

pub fn run(target: RunTarget, mut options: Options) -> Result<Infallible> {
    let host = host::current()?;
    let workdir = env::current_dir().map_err(|err| Error::io("get current directory", err))?;
    let workdir = workdir
        .canonicalize()
        .map_err(|err| Error::io_path("resolve current directory", &workdir, err))?;

    dedup_validate_env_names(&mut options.env_dir, &mut options.unset_env)?;
    let config = config::load(&options.config, host.home.as_deref())?;

    let mut required = config.required;
    required.extend(options.write);
    let allowed = pathutil::resolve_writes(&required, &config.optional, host.home.as_deref())?;
    pathutil::refuse_redirected_regular_stdio(&workdir, &allowed)?;

    let histfile = host
        .home
        .as_ref()
        .map(|home| home.join(host::history_file_name(&host.shell)));
    let envdir_root = prepare_env_dirs(&workdir, host.home.as_deref(), &options.env_dir)?;
    let env_map = build_child_env(
        &host,
        &workdir,
        histfile.as_deref(),
        envdir_root.as_deref(),
        &options.env_dir,
        &options.unset_env,
    );
    let tty_path = host::tty_path();
    let profile = profile::build(&workdir, &allowed.dirs, &allowed.files, histfile.as_deref())?;

    let mut command = build_command(target, &host.shell, &workdir, &env_map)?;
    host::close_extra_fds();
    sandbox::apply(&profile, &workdir, &tty_path, histfile.as_deref())?;

    Err(Error::io("exec", command.exec()))
}

fn dedup_validate_env_names(env_dir: &mut Vec<String>, unset_env: &mut Vec<String>) -> Result<()> {
    dedup(env_dir);
    dedup(unset_env);

    for name in env_dir.iter().chain(unset_env.iter()) {
        if !valid_env_name(name) {
            return Err(Error::InvalidEnvName(name.clone()));
        }
    }
    for name in unset_env.iter() {
        if reserved_unset_env(name) {
            return Err(Error::ReservedUnsetEnv(name.clone()));
        }
        if env_dir.contains(name) {
            return Err(Error::ConflictingEnv(name.clone()));
        }
    }
    Ok(())
}

fn prepare_env_dirs(
    workdir: &Path,
    home: Option<&Path>,
    env_dir: &[String],
) -> Result<Option<PathBuf>> {
    if env_dir.is_empty() {
        return Ok(None);
    }
    let root = pathutil::expand_home(&workdir.join(".sbrun"), home)?;
    pathutil::ensure_real_directory(&root)?;
    for name in env_dir {
        pathutil::ensure_real_directory(&root.join(name))?;
    }
    Ok(Some(root))
}

fn build_child_env(
    host: &host::Host,
    workdir: &Path,
    histfile: Option<&Path>,
    envdir_root: Option<&Path>,
    env_dir: &[String],
    unset_env: &[String],
) -> Vec<(OsString, OsString)> {
    let mut env_map: Vec<(OsString, OsString)> = env::vars_os().collect();
    remove_env(&mut env_map, "BASH_ENV");
    remove_env(&mut env_map, "ENV");
    remove_env(&mut env_map, "DYLD_INSERT_LIBRARIES");
    remove_env(&mut env_map, "DYLD_LIBRARY_PATH");
    remove_env(&mut env_map, "DYLD_FRAMEWORK_PATH");
    remove_env(&mut env_map, "LD_LIBRARY_PATH");
    remove_env(&mut env_map, "SBRUN_ALLOW_STDIO_REDIRECTS");

    for name in unset_env {
        remove_env(&mut env_map, name);
    }

    let path = env::var_os("PATH").unwrap_or_else(|| {
        OsString::from("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
    });
    set_env(&mut env_map, "PATH", &path);
    set_env(&mut env_map, "PWD", workdir.as_os_str());
    if let Some(home) = &host.home {
        set_env(&mut env_map, "HOME", home.as_os_str());
    }
    set_env(&mut env_map, "TMPDIR", OsStr::new("/tmp"));
    if let Some(histfile) = histfile {
        set_env(&mut env_map, "HISTFILE", histfile.as_os_str());
    } else {
        remove_env(&mut env_map, "HISTFILE");
    }
    set_env(&mut env_map, "SHELL", host.shell.as_os_str());
    set_env(&mut env_map, "SBRUN_ACTIVE", OsStr::new("1"));

    if host::shell_is_bash(&host.shell) {
        set_env(
            &mut env_map,
            "BASH_SILENCE_DEPRECATION_WARNING",
            OsStr::new("1"),
        );
    } else {
        remove_env(&mut env_map, "BASH_SILENCE_DEPRECATION_WARNING");
    }

    if let Some(user) = &host.user {
        set_env(&mut env_map, "USER", user);
        set_env(&mut env_map, "LOGNAME", user);
    }

    for key in ["TERM", "LANG", "LC_ALL", "LC_CTYPE"] {
        if let Some(value) = env::var_os(key) {
            set_env(&mut env_map, key, &value);
        }
    }

    if let Some(root) = envdir_root {
        for name in env_dir {
            set_env(&mut env_map, name, root.join(name).as_os_str());
        }
    }

    env_map
}

fn build_command(
    target: RunTarget,
    shell: &Path,
    workdir: &Path,
    env_map: &[(OsString, OsString)],
) -> Result<Command> {
    let mut command = match target {
        RunTarget::InteractiveShell => {
            let mut command = Command::new(shell);
            command.arg("-i");
            command.arg0(host::login_arg0(shell));
            command
        }
        RunTarget::ShellCommand(text) => {
            let mut command = Command::new(shell);
            command.arg("-lc").arg(text);
            command
        }
        RunTarget::Exec(argv) => {
            if argv.is_empty() {
                return Err(Error::Usage("direct command cannot be empty".into()));
            }
            let mut command = Command::new(&argv[0]);
            command.args(&argv[1..]);
            command
        }
    };

    command.current_dir(workdir);
    command.env_clear();
    command.envs(env_map.iter().cloned());
    Ok(command)
}

fn dedup(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn reserved_unset_env(name: &str) -> bool {
    matches!(
        name,
        "PATH"
            | "PWD"
            | "HOME"
            | "TMPDIR"
            | "HISTFILE"
            | "SHELL"
            | "SBRUN_ACTIVE"
            | "USER"
            | "LOGNAME"
            | "TERM"
            | "LANG"
            | "LC_ALL"
            | "LC_CTYPE"
            | "BASH_SILENCE_DEPRECATION_WARNING"
    )
}

fn remove_env(env_map: &mut Vec<(OsString, OsString)>, key: &str) {
    env_map.retain(|(existing, _)| existing != OsStr::new(key));
}

fn set_env(env_map: &mut Vec<(OsString, OsString)>, key: &str, value: &OsStr) {
    remove_env(env_map, key);
    env_map.push((OsString::from(key), value.to_os_string()));
}

#[pyfunction(name = "exec", signature=(argv, *, write=None, env_dir=None, unset_env=None, config=None, no_config=false))]
fn py_exec(
    argv: Vec<String>,
    write: Option<Vec<String>>,
    env_dir: Option<Vec<String>>,
    unset_env: Option<Vec<String>>,
    config: Option<String>,
    no_config: bool,
) -> PyResult<()> {
    let config = match (config, no_config) {
        (Some(_), true) => {
            return Err(PyRuntimeError::new_err(
                "--config and --no-config cannot be used together",
            ));
        }
        (Some(path), false) => ConfigMode::Explicit(PathBuf::from(path)),
        (None, true) => ConfigMode::None,
        (None, false) => ConfigMode::Default,
    };
    let options = Options {
        write: write
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        env_dir: env_dir.unwrap_or_default(),
        unset_env: unset_env.unwrap_or_default(),
        config,
    };
    exec(argv.into_iter().map(OsString::from), options)
        .map(|_| ())
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[pymodule]
fn sbrun(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(py_exec, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use crate::{CliCommand, ConfigMode, RunTarget, parse_cli};

    #[test]
    fn parse_direct_command() {
        let parsed = parse_cli([
            OsString::from("sbrun"),
            OsString::from("python3"),
            OsString::from("-c"),
            OsString::from("print(1)"),
        ])
        .unwrap();
        let CliCommand::Run { target, options } = parsed else {
            panic!("expected run command")
        };
        assert!(matches!(options.config, ConfigMode::Default));
        let RunTarget::Exec(argv) = target else {
            panic!("expected direct command")
        };
        assert_eq!(
            argv,
            vec![
                OsString::from("python3"),
                OsString::from("-c"),
                OsString::from("print(1)")
            ]
        );
    }

    #[test]
    fn parse_shell_command() {
        let parsed = parse_cli([
            OsString::from("sbrun"),
            OsString::from("-c"),
            OsString::from("echo hi"),
        ])
        .unwrap();
        let CliCommand::Run { target, .. } = parsed else {
            panic!("expected run command")
        };
        let RunTarget::ShellCommand(text) = target else {
            panic!("expected shell command")
        };
        assert_eq!(text, "echo hi");
    }
}
