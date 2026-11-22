pub mod admin;
mod cli;
pub mod config;
mod error;
pub mod host;
pub mod pathutil;
#[cfg(target_os = "macos")]
pub mod profile;
pub mod prompt;
pub mod sandbox;

use std::{collections::HashSet, convert::Infallible, env, ffi::{OsStr, OsString}, os::unix::process::CommandExt, path::{Path, PathBuf}, process::Command};

pub use cli::{Command as CliCommand, help_text, parse as parse_cli};
pub use config::ConfigMode;
pub use error::{Error, Result};

pub fn cli_main_with_args<I: IntoIterator<Item = OsString>>(args: I) {
    let args: Vec<OsString> = args.into_iter().collect();
    let program = args.first().map_or_else(|| OsString::from("sbrun"), |a| a.clone());
    let program = program.to_string_lossy();
    let err = match parse_cli(args) {
        Ok(CliCommand::Help) => { print!("{}", help_text(&program)); return; }
        Ok(CliCommand::Version) => { println!("sbrun {}", env!("CARGO_PKG_VERSION")); return; }
        Ok(CliCommand::KernelInstall) => match admin::kernel_install() { Ok(()) => return, Err(err) => err },
        Ok(CliCommand::PromptInit(shell)) => match prompt::init_script(shell.as_deref()) { Ok(script) => { print!("{script}"); return; } Err(err) => err },
        Ok(CliCommand::Run { target, options }) => run(target, options).unwrap_err(),
        Err(err) => {
            eprintln!("sbrun: {err}");
            eprintln!();
            eprint!("{}", help_text(&program));
            std::process::exit(111);
        }
    };
    eprintln!("sbrun: {err}");
    std::process::exit(exit_code(&err));
}

fn exit_code(err: &Error) -> i32 {
    match err {
        Error::Io { action: "exec", source } => match source.raw_os_error() { Some(libc::ENOENT) => 127, Some(libc::EACCES | libc::ENOEXEC) => 126, _ => 111 },
        _ => 111,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options { pub write: Vec<PathBuf>, pub env_dir: Vec<String>, pub unset_env: Vec<String>, pub config: ConfigMode }

#[derive(Debug, Clone)]
pub enum RunTarget { InteractiveShell, ShellCommand(String), Exec(Vec<OsString>) }

pub fn run(target: RunTarget, mut options: Options) -> Result<Infallible> {
    #[cfg(target_os = "linux")]
    let privilege_guard = sandbox::temporarily_drop_to_real_user()?;

    let host = host::current()?;
    let workdir = env::current_dir().map_err(|err| Error::io("get current directory", err))?;
    let workdir = workdir.canonicalize().map_err(|err| Error::io_path("resolve current directory", &workdir, err))?;

    dedup_validate_env_names(&mut options.env_dir, &mut options.unset_env)?;
    let config = config::load(&options.config, host.home.as_deref())?;

    let mut required = config.required;
    required.extend(options.write);
    let allowed = pathutil::resolve_writes(&required, &config.optional, host.home.as_deref())?;
    pathutil::refuse_redirected_regular_stdio(&workdir, &allowed)?;

    let histfile = host.home.as_ref().map(|home| home.join(host::history_file_name(&host.shell)));
    #[cfg(target_os = "linux")]
    let histfile = histfile.filter(|path| ensure_file(path));
    let envdir_root = prepare_env_dirs(&workdir, &options.env_dir)?;
    let env_map = build_child_env(&host, &workdir, histfile.as_deref(), envdir_root.as_deref(), &options.env_dir, &options.unset_env);
    let mut command = build_command(target, &host.shell, &workdir, &env_map)?;
    host::close_extra_fds();

    #[cfg(target_os = "linux")]
    if let Some(guard) = privilege_guard { guard.restore_root()?; }

    #[cfg(target_os = "macos")]
    {
        let tty_path = host::tty_path();
        let profile = profile::build(&workdir, &allowed.dirs, &allowed.files, histfile.as_deref())?;
        sandbox::apply(&profile, &workdir, &tty_path, histfile.as_deref())?;
    }
    #[cfg(target_os = "linux")]
    sandbox::apply(&workdir, &allowed.dirs, &allowed.files, histfile.as_deref())?;

    Err(Error::io("exec", command.exec()))
}

pub fn dedup_validate_env_names(env_dir: &mut Vec<String>, unset_env: &mut Vec<String>) -> Result<()> {
    dedup(env_dir);
    dedup(unset_env);

    for name in env_dir.iter().chain(unset_env.iter()) { if !valid_env_name(name) { return Err(Error::InvalidEnvName(name.clone())); } }
    for name in unset_env.iter() {
        if reserved_unset_env(name) { return Err(Error::ReservedUnsetEnv(name.clone())); }
        if env_dir.contains(name) { return Err(Error::ConflictingEnv(name.clone())); }
    }
    Ok(())
}

fn prepare_env_dirs(workdir: &Path, env_dir: &[String]) -> Result<Option<PathBuf>> {
    if env_dir.is_empty() { return Ok(None); }
    let root = workdir.join(".sbrun");
    pathutil::ensure_real_directory(&root)?;
    for name in env_dir { pathutil::ensure_real_directory(&root.join(name))?; }
    Ok(Some(root))
}

/// The Linux sandbox can only bind-mount existing files, so the history file
/// must exist before entering the sandbox. Failure just means no history.
#[cfg(target_os = "linux")]
fn ensure_file(path: &Path) -> bool { use std::fs::OpenOptions; OpenOptions::new().append(true).create(true).open(path).is_ok() }

fn build_child_env(
    host: &host::Host, workdir: &Path, histfile: Option<&Path>, envdir_root: Option<&Path>, env_dir: &[String], unset_env: &[String]
) -> Vec<(OsString, OsString)> {
    let mut env_map: Vec<(OsString, OsString)> = env::vars_os().collect();
    remove_env(&mut env_map, "BASH_ENV");
    remove_env(&mut env_map, "ENV");
    remove_env(&mut env_map, "SBRUN_ALLOW_STDIO_REDIRECTS");
    #[cfg(target_os = "macos")]
    {
        remove_env(&mut env_map, "DYLD_INSERT_LIBRARIES");
        remove_env(&mut env_map, "DYLD_LIBRARY_PATH");
        remove_env(&mut env_map, "DYLD_FRAMEWORK_PATH");
    }
    remove_env(&mut env_map, "LD_LIBRARY_PATH");
    #[cfg(target_os = "linux")]
    remove_env(&mut env_map, "LD_PRELOAD");

    for name in unset_env { remove_env(&mut env_map, name); }

    let path = env::var_os("PATH").unwrap_or_else(|| {
        if cfg!(target_os = "macos") { OsString::from("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin") }
        else { OsString::from("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin") }
    });
    set_env(&mut env_map, "PATH", &path);
    set_env(&mut env_map, "PWD", workdir.as_os_str());
    if let Some(home) = &host.home { set_env(&mut env_map, "HOME", home.as_os_str()); }
    set_env(&mut env_map, "TMPDIR", OsStr::new("/tmp"));
    if let Some(histfile) = histfile { set_env(&mut env_map, "HISTFILE", histfile.as_os_str()); }
    else { remove_env(&mut env_map, "HISTFILE"); }
    set_env(&mut env_map, "SHELL", host.shell.as_os_str());
    set_env(&mut env_map, "SBRUN_ACTIVE", OsStr::new("1"));

    if host::shell_is_bash(&host.shell) { set_env(&mut env_map, "BASH_SILENCE_DEPRECATION_WARNING", OsStr::new("1")); }
    else { remove_env(&mut env_map, "BASH_SILENCE_DEPRECATION_WARNING"); }

    if let Some(user) = &host.user {
        set_env(&mut env_map, "USER", user);
        set_env(&mut env_map, "LOGNAME", user);
    }

    for key in ["TERM", "LANG", "LC_ALL", "LC_CTYPE"] { if let Some(value) = env::var_os(key) { set_env(&mut env_map, key, &value); } }

    if let Some(root) = envdir_root { for name in env_dir { set_env(&mut env_map, name, root.join(name).as_os_str()); } }

    env_map
}

fn build_command(target: RunTarget, shell: &Path, workdir: &Path, env_map: &[(OsString, OsString)]) -> Result<Command> {
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
            if argv.is_empty() { return Err(Error::Usage("direct command cannot be empty".into())); }
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
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic()) && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn reserved_unset_env(name: &str) -> bool {
    matches!(
        name, "PATH" | "PWD" | "HOME" | "TMPDIR" | "HISTFILE" | "SHELL" | "SBRUN_ACTIVE" | "USER" | "LOGNAME" | "TERM" | "LANG" | "LC_ALL" | "LC_CTYPE"
            | "BASH_SILENCE_DEPRECATION_WARNING"
    )
}

fn remove_env(env_map: &mut Vec<(OsString, OsString)>, key: &str) { env_map.retain(|(existing, _)| existing != OsStr::new(key)); }

fn set_env(env_map: &mut Vec<(OsString, OsString)>, key: &str, value: &OsStr) {
    remove_env(env_map, key);
    env_map.push((OsString::from(key), value.to_os_string()));
}

pub fn cli_main() { cli_main_with_args(env::args_os()) }
