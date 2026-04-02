# sbrun

`sbrun` launches commands under the macOS sandbox and only allows writes beneath
the current directory tree plus paths you explicitly opt into.

The implementation is a single Rust crate:

- the `sbrun` binary is the CLI
- the same crate also exposes a Python `sbrun.exec(...)` API via PyO3
- the binary applies the sandbox directly through `libsandbox`

## Install

Install the latest macOS arm64 release:

```sh
curl -fsSL https://raw.githubusercontent.com/AnswerDotAI/sbrun/main/install.sh | bash
```

Build locally:

```sh
cargo build --release
```

Install the Python extension into an active virtualenv:

```sh
maturin develop --release
```

## Use

Start an interactive login shell:

```sh
cd /path/to/project
sbrun
```

Run a command directly:

```sh
cd /path/to/project
sbrun python3 app.py
```

Run a shell snippet with your current `$SHELL`:

```sh
cd /path/to/project
sbrun -c 'touch ok.txt && echo hello'
```

Allow writes to an extra directory:

```sh
cd /path/to/project
sbrun --write /tmp -- python3 -c 'open("/tmp/sbrun-demo", "w").write("ok")'
```

Set environment variables to project-local directories:

```sh
cd /path/to/project
sbrun --env-dir IPYTHONDIR --env-dir MPLCONFIGDIR -- ipython
```

Remove selected variables from the child environment:

```sh
cd /path/to/project
sbrun --unset-env GITHUB_API_KEY --unset-env OPENAI_API_KEY -- python3 app.py
```

If the command itself starts with `-`, use `--` to stop option parsing:

```sh
cd /path/to/project
sbrun -- -lc 'printf hello\n'
```

Help and version:

```sh
sbrun --help
sbrun --version
```

## CLI

- `-w, --write PATH`: allow writes to a regular file or directory; repeatable
- `-d, --env-dir VAR`: set `VAR` to `.sbrun/VAR`; repeatable
- `-u, --unset-env VAR`: remove `VAR` from the child environment; repeatable
- `-c, --command STRING`: run `$SHELL -lc STRING`
- `--config PATH`: load that TOML file and ignore the standard config locations
- `--no-config`: ignore config files entirely
- `--`: stop parsing `sbrun` options

Behavior:

- with no command, `sbrun` launches your `$SHELL` as an interactive login shell
- with `-c/--command`, `sbrun` runs `$SHELL -lc STRING`
- otherwise `sbrun` `exec`s the given command directly
- `SBRUN_ACTIVE=1` is exported in the child environment
- `HOME` stays your real home directory when one is available
- `TMPDIR` is set to `/tmp`
- the shell history file is writable by default
- stdout/stderr redirected to regular files outside allowed writable paths are rejected unless `SBRUN_ALLOW_STDIO_REDIRECTS=1`

For bash prompt logic, you can use `SBRUN_ACTIVE` without replacing an existing
`PROMPT_COMMAND` or `PS1`. Put this in `~/.bashrc`:

```bash
sbrun_prompt_prefix() {
  [[ ${SBRUN_ACTIVE:-} == 1 ]] || return
  case $PS1 in
    '🔒 '*) ;;
    *) PS1="🔒 $PS1" ;;
  esac
}

case "$(declare -p PROMPT_COMMAND 2>/dev/null)" in
  "declare -a "*)
    case " ${PROMPT_COMMAND[*]} " in
      *" sbrun_prompt_prefix "*) ;;
      *) PROMPT_COMMAND+=(sbrun_prompt_prefix) ;;
    esac
    ;;
  *)
    case ";${PROMPT_COMMAND:-};" in
      *";sbrun_prompt_prefix;"*) ;;
      *) PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }sbrun_prompt_prefix" ;;
    esac
    ;;
esac
```

If your login shell does not source `~/.bashrc`, put the same snippet in
`~/.bash_profile`.

## Config

`sbrun` reads TOML config from:

- `$XDG_CONFIG_DIRS/.../sbrun/config.toml`
- `$XDG_CONFIG_HOME/sbrun/config.toml`
- `~/.config/sbrun/config.toml` when `XDG_CONFIG_HOME` is unset

`--config PATH` replaces those defaults with one explicit file. `--no-config`
skips config loading entirely.

Example:

```toml
version = 1

write = ["/tmp", "/Volumes/scratch"]
optional_write = [
  "~/.cache",
  "~/Library/Caches",
]
```

Rules:

- `version` must be `1` when present
- `write` entries are required and error if they do not resolve
- `optional_write` entries are ignored when they do not resolve
- config paths must be absolute or start with `~/`
- `env_dir` and `unset_env` are CLI-only

The repo ships a practical default config in [`sbrun.default.toml`](/Users/jhoward/git/sbrun/sbrun.default.toml).

## Python

The Python API is intentionally minimal and follows the same `exec` model as the
CLI:

```python
import sbrun

sbrun.exec(
    ["python3", "app.py"],
    write=["/tmp"],
    env_dir=["IPYTHONDIR"],
    unset_env=["GITHUB_API_KEY"],
)
```

On success, `sbrun.exec(...)` does not return because it replaces the current
process image. On failure, it raises a Python exception.

## Development

Build, test, and release notes live in [`DEV.md`](/Users/jhoward/git/sbrun/DEV.md).
