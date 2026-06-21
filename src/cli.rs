use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};

use crate::{
    ConfigMode, Options, RunTarget,
    error::{Error, Result},
};

pub(crate) const CONFIG_CONFLICT: &str = "--config and --no-config cannot be used together";

pub enum Command {
    Help,
    Version,
    KernelInstall,
    PromptInit(Option<String>),
    Run { target: RunTarget, options: Options },
}

pub fn parse<I>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let _program = args.next();

    let mut write = Vec::new();
    let mut env_dir = Vec::new();
    let mut unset_env = Vec::new();
    let mut config = ConfigMode::Default;
    let mut shell_command = None;
    let mut command: Vec<OsString> = Vec::new();
    let mut kernel_install = false;
    let mut prompt_init = None;

    while let Some(arg) = args.next() {
        if arg == OsStr::new("--") {
            command.extend(args);
            break;
        }
        let Some((flag, inline)) = parse_option(&arg)? else {
            command.push(arg);
            command.extend(args);
            break;
        };
        match flag.as_str() {
            "help" => {
                no_value(&flag, &inline)?;
                return Ok(Command::Help);
            }
            "version" => {
                no_value(&flag, &inline)?;
                return Ok(Command::Version);
            }
            "kernel-install" => {
                no_value(&flag, &inline)?;
                kernel_install = true;
            }
            "no-config" => {
                no_value(&flag, &inline)?;
                set_config(&mut config, ConfigMode::None)?;
            }
            "prompt-init" => {
                if prompt_init.is_some() {
                    return Err(Error::Usage("--prompt-init may only be used once".into()));
                }
                prompt_init = Some(match inline {
                    Some(value) => Some(into_utf8(&flag, value)?),
                    None => None,
                });
            }
            "write" => write.push(PathBuf::from(take_value(&flag, inline, &mut args)?)),
            "env-dir" => env_dir.push(into_utf8(&flag, take_value(&flag, inline, &mut args)?)?),
            "unset-env" => unset_env.push(into_utf8(&flag, take_value(&flag, inline, &mut args)?)?),
            "command" => {
                if shell_command.is_some() {
                    return Err(Error::Usage("--command/-c may only be used once".into()));
                }
                shell_command = Some(into_utf8(&flag, take_value(&flag, inline, &mut args)?)?);
            }
            "config" => {
                let path = PathBuf::from(take_value(&flag, inline, &mut args)?);
                set_config(&mut config, ConfigMode::Explicit(path))?;
            }
            _ => return Err(Error::Usage(format!("unknown option --{flag}"))),
        }
    }

    let has_run_args = !write.is_empty()
        || !env_dir.is_empty()
        || !unset_env.is_empty()
        || shell_command.is_some()
        || !command.is_empty()
        || !matches!(config, ConfigMode::Default);

    if kernel_install {
        if has_run_args || prompt_init.is_some() {
            return Err(Error::Usage(
                "--kernel-install cannot be combined with other options or commands".into(),
            ));
        }
        return Ok(Command::KernelInstall);
    }
    if let Some(shell) = prompt_init {
        if has_run_args {
            return Err(Error::Usage(
                "--prompt-init cannot be combined with other options or commands".into(),
            ));
        }
        return Ok(Command::PromptInit(shell));
    }
    if shell_command.is_some() && !command.is_empty() {
        return Err(Error::Usage(
            "use either --command/-c or a direct command, not both".into(),
        ));
    }

    let target = if let Some(command) = shell_command {
        RunTarget::ShellCommand(command)
    } else if command.is_empty() {
        RunTarget::InteractiveShell
    } else {
        RunTarget::Exec(command)
    };

    Ok(Command::Run {
        target,
        options: Options {
            write,
            env_dir,
            unset_env,
            config,
        },
    })
}

pub fn help_text(program: &str) -> String {
    format!(
        "Usage: {program} [options] [--] [command [args...]]\n\
\n\
Run commands in a sandbox with writes confined to the current directory\n\
tree plus explicitly allowed paths.\n\
\n\
Options:\n\
  -h, --help             Show this help and exit\n\
      --version          Show version and exit\n\
      --kernel-install   Install Linux sysctl settings and run sysctl --system\n\
      --prompt-init      Print shell code to show a lock icon in sandboxed prompts\n\
  -w, --write PATH       Allow writes to PATH; may be repeated\n\
  -d, --env-dir VAR      Set VAR to .sbrun/VAR; may be repeated\n\
  -u, --unset-env VAR    Remove VAR from the child environment; may be repeated\n\
  -c, --command STRING   Run STRING with $SHELL -lc\n\
      --config PATH      Load PATH as the only config file\n\
      --no-config        Ignore config files\n\
  --                     Stop option parsing\n\
\n\
Behavior:\n\
  With --kernel-install, install /etc/sysctl.d/90-sbrun.conf and apply it (Linux only; run as root).\n\
  With --prompt-init[=bash|zsh], print shell init code for the lock icon prompt hook.\n\
  With no command, start $SHELL as an interactive login shell.\n\
  With -c/--command, run $SHELL -lc STRING.\n\
  Otherwise run the given command directly.\n\
\n\
Config:\n\
  System config: $XDG_CONFIG_DIRS/.../sbrun/config.toml\n\
  User config:   $XDG_CONFIG_HOME/sbrun/config.toml or ~/.config/sbrun/config.toml\n\
  Format: version = 1, write = [\"/path\"], optional_write = [\"/path\"]\n"
    )
}

/// Normalize one argument into a canonical long flag name plus optional
/// inline (`--flag=value`) value. `Ok(None)` means the argument is not an
/// option and starts the command. Unknown options are errors here (short
/// form) or in the caller's match (long form).
fn parse_option(arg: &OsStr) -> Result<Option<(String, Option<OsString>)>> {
    let bytes = arg.as_encoded_bytes();
    if !bytes.starts_with(b"-") || bytes == b"-" {
        return Ok(None);
    }
    if let Some(body) = bytes.strip_prefix(b"--") {
        let (name, value) = match body.iter().position(|&b| b == b'=') {
            Some(eq) => (
                &body[..eq],
                Some(OsString::from_vec(body[eq + 1..].to_vec())),
            ),
            None => (body, None),
        };
        let name = std::str::from_utf8(name)
            .map_err(|_| Error::Usage("option names must be utf-8".into()))?;
        return Ok(Some((name.to_string(), value)));
    }
    let text = arg.to_string_lossy();
    let mut chars = text[1..].chars();
    let (Some(short), None) = (chars.next(), chars.next()) else {
        return Err(Error::Usage(format!("unknown option {text}")));
    };
    let long = match short {
        'h' => "help",
        'w' => "write",
        'd' => "env-dir",
        'u' => "unset-env",
        'c' => "command",
        _ => return Err(Error::Usage(format!("unknown option -{short}"))),
    };
    Ok(Some((long.to_string(), None)))
}

fn take_value<I>(flag: &str, inline: Option<OsString>, args: &mut I) -> Result<OsString>
where
    I: Iterator<Item = OsString>,
{
    inline
        .or_else(|| args.next())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Usage(format!("--{flag} requires a value")))
}

fn no_value(flag: &str, inline: &Option<OsString>) -> Result<()> {
    if inline.is_some() {
        return Err(Error::Usage(format!("--{flag} does not take a value")));
    }
    Ok(())
}

fn set_config(slot: &mut ConfigMode, new: ConfigMode) -> Result<()> {
    if matches!(
        (&*slot, &new),
        (ConfigMode::Explicit(_), ConfigMode::None) | (ConfigMode::None, ConfigMode::Explicit(_))
    ) {
        return Err(Error::Usage(CONFIG_CONFLICT.into()));
    }
    *slot = new;
    Ok(())
}

fn into_utf8(flag: &str, value: OsString) -> Result<String> {
    value
        .into_string()
        .map_err(|_| Error::Usage(format!("--{flag} value must be utf-8")))
}
