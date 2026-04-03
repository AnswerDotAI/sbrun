use std::env;

use crate::error::{Error, Result};

const LOCK_PREFIX: &str = "\u{1F512} ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptShell {
    Bash,
    Zsh,
}

pub fn init_script(shell: Option<&str>) -> Result<String> {
    let shell = match shell {
        Some(name) => parse_shell(name)?,
        None => detect_shell()?,
    };
    Ok(match shell {
        PromptShell::Bash => bash_script(),
        PromptShell::Zsh => zsh_script(),
    })
}

fn detect_shell() -> Result<PromptShell> {
    let Some(shell) = env::var_os("SHELL") else {
        return Err(prompt_init_shell_error());
    };
    let shell = shell.to_string_lossy();
    parse_shell(&shell).map_err(|_| prompt_init_shell_error())
}

fn parse_shell(value: &str) -> Result<PromptShell> {
    match value.rsplit('/').next().unwrap_or(value) {
        "bash" => Ok(PromptShell::Bash),
        "zsh" => Ok(PromptShell::Zsh),
        _ => Err(Error::Usage(
            "--prompt-init only supports bash and zsh".into(),
        )),
    }
}

fn prompt_init_shell_error() -> Error {
    Error::Usage(
        "could not infer shell for --prompt-init; use --prompt-init=bash or --prompt-init=zsh"
            .into(),
    )
}

fn bash_script() -> String {
    format!(
        "sbrun_prompt_prefix() {{\n\
  [[ ${{SBRUN_ACTIVE:-}} == 1 ]] || return\n\
  case $PS1 in\n\
    '{LOCK_PREFIX}'*) ;;\n\
    *) PS1=\"{LOCK_PREFIX}$PS1\" ;;\n\
  esac\n\
}}\n\
\n\
case \"$(declare -p PROMPT_COMMAND 2>/dev/null)\" in\n\
  \"declare -a \"*)\n\
    case \" ${{PROMPT_COMMAND[*]}} \" in\n\
      *\" sbrun_prompt_prefix \"*) ;;\n\
      *) PROMPT_COMMAND+=(sbrun_prompt_prefix) ;;\n\
    esac\n\
    ;;\n\
  *)\n\
    case \";${{PROMPT_COMMAND:-}};\" in\n\
      *\";sbrun_prompt_prefix;\"*) ;;\n\
      *) PROMPT_COMMAND=\"${{PROMPT_COMMAND:+$PROMPT_COMMAND; }}sbrun_prompt_prefix\" ;;\n\
    esac\n\
    ;;\n\
esac\n"
    )
}

fn zsh_script() -> String {
    format!(
        "sbrun_prompt_prefix() {{\n\
  [[ ${{SBRUN_ACTIVE:-}} == 1 ]] || return\n\
  case $PROMPT in\n\
    '{LOCK_PREFIX}'*) ;;\n\
    *) PROMPT=\"{LOCK_PREFIX}$PROMPT\" ;;\n\
  esac\n\
}}\n\
\n\
typeset -ga precmd_functions\n\
case \" ${{precmd_functions[*]}} \" in\n\
  *\" sbrun_prompt_prefix \"*) ;;\n\
  *) precmd_functions+=(sbrun_prompt_prefix) ;;\n\
esac\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_shell_names() {
        assert_eq!(parse_shell("bash").unwrap(), PromptShell::Bash);
        assert_eq!(parse_shell("/bin/zsh").unwrap(), PromptShell::Zsh);
    }

    #[test]
    fn rejects_unsupported_shells() {
        let err = parse_shell("fish").unwrap_err();
        assert!(err.to_string().contains("only supports bash and zsh"));
    }

    #[test]
    fn bash_script_uses_prompt_command_hook() {
        let script = init_script(Some("bash")).unwrap();
        assert!(script.contains("PROMPT_COMMAND"));
        assert!(script.contains(LOCK_PREFIX));
    }

    #[test]
    fn zsh_script_uses_precmd_hook() {
        let script = init_script(Some("zsh")).unwrap();
        assert!(script.contains("precmd_functions"));
        assert!(script.contains(LOCK_PREFIX));
    }
}
