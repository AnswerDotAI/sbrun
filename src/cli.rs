use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};

use crate::{
    ConfigMode, Options, RunTarget,
    error::{Error, Result},
};

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
    let mut command = Vec::new();
    let mut kernel_install = false;
    let mut prompt_init = None;
    let mut stop_parsing = false;

    while let Some(arg) = args.next() {
        if stop_parsing {
            command.push(arg);
            command.extend(args);
            break;
        }

        if arg == OsStr::new("--") {
            stop_parsing = true;
            continue;
        }
        if arg == OsStr::new("--help") || arg == OsStr::new("-h") {
            return Ok(Command::Help);
        }
        if arg == OsStr::new("--version") {
            return Ok(Command::Version);
        }
        if arg == OsStr::new("--kernel-install") {
            kernel_install = true;
            continue;
        }
        if arg == OsStr::new("--prompt-init") {
            if prompt_init.is_some() {
                return Err(Error::Usage("--prompt-init may only be used once".into()));
            }
            prompt_init = Some(None);
            continue;
        }
        if arg == OsStr::new("--no-config") {
            if matches!(config, ConfigMode::Explicit(_)) {
                return Err(Error::Usage(
                    "--config and --no-config cannot be used together".into(),
                ));
            }
            config = ConfigMode::None;
            continue;
        }
        if arg == OsStr::new("--write") {
            write.push(PathBuf::from(next_value("--write", &mut args)?));
            continue;
        }
        if arg == OsStr::new("--env-dir") {
            env_dir.push(parse_env_name(next_value("--env-dir", &mut args)?)?);
            continue;
        }
        if arg == OsStr::new("--unset-env") {
            unset_env.push(parse_env_name(next_value("--unset-env", &mut args)?)?);
            continue;
        }
        if arg == OsStr::new("--command") {
            set_shell_command(&mut shell_command, next_value("--command", &mut args)?)?;
            continue;
        }
        if arg == OsStr::new("--config") {
            if matches!(config, ConfigMode::None) {
                return Err(Error::Usage(
                    "--config and --no-config cannot be used together".into(),
                ));
            }
            config = ConfigMode::Explicit(PathBuf::from(next_value("--config", &mut args)?));
            continue;
        }

        if let Some((flag, value)) = split_long_option(&arg)? {
            match flag.as_str() {
                "write" => write.push(PathBuf::from(value)),
                "env-dir" => env_dir.push(parse_env_name(value)?),
                "unset-env" => unset_env.push(parse_env_name(value)?),
                "command" => set_shell_command(&mut shell_command, value)?,
                "config" => {
                    if matches!(config, ConfigMode::None) {
                        return Err(Error::Usage(
                            "--config and --no-config cannot be used together".into(),
                        ));
                    }
                    config = ConfigMode::Explicit(PathBuf::from(value));
                }
                "prompt-init" => {
                    if prompt_init.is_some() {
                        return Err(Error::Usage("--prompt-init may only be used once".into()));
                    }
                    let shell = value
                        .into_string()
                        .map_err(|_| Error::Usage("prompt-init shell must be utf-8".into()))?;
                    prompt_init = Some(Some(shell));
                }
                _ => return Err(Error::Usage(format!("unknown option --{flag}"))),
            }
            continue;
        }

        if let Some(flag) = short_flag(&arg) {
            match flag {
                'w' => write.push(PathBuf::from(next_value("--write", &mut args)?)),
                'd' => env_dir.push(parse_env_name(next_value("--env-dir", &mut args)?)?),
                'u' => unset_env.push(parse_env_name(next_value("--unset-env", &mut args)?)?),
                'c' => set_shell_command(&mut shell_command, next_value("--command", &mut args)?)?,
                _ => return Err(Error::Usage(format!("unknown option -{flag}"))),
            }
            continue;
        }

        command.push(arg);
        command.extend(args);
        break;
    }

    if shell_command.is_some() && !command.is_empty() {
        return Err(Error::Usage(
            "use either --command/-c or a direct command, not both".into(),
        ));
    }
    if kernel_install {
        if prompt_init.is_some()
            || shell_command.is_some()
            || !command.is_empty()
            || !write.is_empty()
            || !env_dir.is_empty()
            || !unset_env.is_empty()
            || !matches!(config, ConfigMode::Default)
        {
            return Err(Error::Usage(
                "--kernel-install cannot be combined with other options or commands".into(),
            ));
        }
        return Ok(Command::KernelInstall);
    }
    if let Some(shell) = prompt_init {
        if shell_command.is_some()
            || !command.is_empty()
            || !write.is_empty()
            || !env_dir.is_empty()
            || !unset_env.is_empty()
            || !matches!(config, ConfigMode::Default)
        {
            return Err(Error::Usage(
                "--prompt-init cannot be combined with other options or commands".into(),
            ));
        }
        return Ok(Command::PromptInit(shell));
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

fn split_long_option(arg: &OsStr) -> Result<Option<(String, OsString)>> {
    let bytes = arg.as_encoded_bytes();
    if !bytes.starts_with(b"--") || bytes == b"--" {
        return Ok(None);
    }

    let body = &bytes[2..];
    let Some(eq) = body.iter().position(|b| *b == b'=') else {
        return Ok(None);
    };
    let flag = String::from_utf8(body[..eq].to_vec())
        .map_err(|_| Error::Usage("option names must be utf-8".into()))?;
    let value = OsString::from_vec(body[eq + 1..].to_vec());
    if value.is_empty() {
        return Err(Error::Usage(format!("--{flag} requires a value")));
    }
    Ok(Some((flag, value)))
}

fn short_flag(arg: &OsStr) -> Option<char> {
    let text = arg.to_str()?;
    if text.len() == 2 && text.starts_with('-') && text != "--" {
        text.chars().nth(1)
    } else {
        None
    }
}

fn next_value<I>(flag: &str, args: &mut I) -> Result<OsString>
where
    I: Iterator<Item = OsString>,
{
    args.next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Usage(format!("{flag} requires a value")))
}

fn parse_env_name(value: OsString) -> Result<String> {
    let name = value
        .into_string()
        .map_err(|_| Error::Usage("environment variable names must be utf-8".into()))?;
    Ok(name)
}

fn set_shell_command(slot: &mut Option<String>, value: OsString) -> Result<()> {
    if slot.is_some() {
        return Err(Error::Usage("--command/-c may only be used once".into()));
    }
    let command = value
        .into_string()
        .map_err(|_| Error::Usage("shell command must be utf-8".into()))?;
    *slot = Some(command);
    Ok(())
}
