# sbrun

`sbrun` launches commands in a sandbox that only allows writes beneath the
current directory tree plus paths you explicitly opt into.

- **macOS**: uses the Seatbelt sandbox via `libsandbox`
- **Linux**: uses unprivileged user namespaces + mount namespaces (inspired by
  [bubblewrap](https://github.com/containers/bubblewrap)) by default; when the
  native `sbrun` binary is installed setuid root, it automatically switches to
  a privileged mount-namespace backend

The implementation is a single Rust crate:

- the `sbrun` binary is the CLI
- the same crate also exposes a Python `sbrun.exec(...)` API via PyO3
- platform-specific sandboxing is selected at compile time

## Install

Install the latest release:

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

Install the persistent Linux sysctl fix and apply it:

```sh
sudo sbrun --kernel-install
```

## CLI

- `-w, --write PATH`: allow writes to a regular file or directory; repeatable
- `-d, --env-dir VAR`: set `VAR` to `.sbrun/VAR`; repeatable
- `-u, --unset-env VAR`: remove `VAR` from the child environment; repeatable
- `-c, --command STRING`: run `$SHELL -lc STRING`
- `--kernel-install`: install `/etc/sysctl.d/90-sbrun.conf` and run `sysctl --system` (Linux only; must be root, e.g. via `sudo`)
- `--config PATH`: load that TOML file and ignore the standard config locations
- `--no-config`: ignore config files entirely
- `--`: stop parsing `sbrun` options

Behavior:

- with no command, `sbrun` launches your `$SHELL` as an interactive login shell
- with `-c/--command`, `sbrun` runs `$SHELL -lc STRING`
- with `--kernel-install`, `sbrun` installs the persistent Linux sysctl config and runs `sysctl --system`
- otherwise `sbrun` `exec`s the given command directly
- `SBRUN_ACTIVE=1` is exported in the child environment
- `HOME` stays your real home directory when one is available
- `TMPDIR` is set to `/tmp`
- the shell history file is writable by default
- stdout/stderr redirected to regular files outside allowed writable paths are rejected unless `SBRUN_ALLOW_STDIO_REDIRECTS=1`

To add a lock icon to sandboxed bash or zsh prompts, put this in your
`~/.bashrc` or `~/.zshrc`:

```sh
eval "$(sbrun --prompt-init)"
```

If shell autodetection gets the wrong shell, use one of:

```sh
eval "$(sbrun --prompt-init=bash)"
eval "$(sbrun --prompt-init=zsh)"
```

The generated hook uses `SBRUN_ACTIVE` and preserves existing bash
`PROMPT_COMMAND` and zsh `precmd_functions` hooks.

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

On first run, if no config file exists, `sbrun` auto-creates
`~/.config/sbrun/config.toml` with sensible platform defaults (writable
`/tmp`, `~/.cache`, `~/.config`, etc). The defaults are also shipped in the
repo as `sbrun.default.macos.toml` and `sbrun.default.linux.toml`.

## Platform notes

### macOS

The sandbox is applied via the Seatbelt profile language and `libsandbox`.
All reads are allowed; writes are confined to the working directory and
configured paths.

### Linux

The sandbox uses unprivileged user namespaces (`CLONE_NEWUSER`) and mount
namespaces (`CLONE_NEWNS`), the same approach used by
[bubblewrap](https://github.com/containers/bubblewrap). The root filesystem
is bind-mounted read-only, then writable paths are bind-mounted back on top.
Default installs require neither root nor setuid.

If the native `sbrun` binary is installed root-owned and setuid, `sbrun`
automatically switches to a privileged Linux backend. In that mode it skips
`CLONE_NEWUSER`, sets up the mount namespace as root, then drops back to the
calling user before `exec()`. That avoids AppArmor's unprivileged user
namespace restriction without changing kernel settings.

Example optional install:

```sh
sudo install -o root -g root -m 4755 ./target/release/sbrun /usr/local/bin/sbrun
```

The setuid mode only applies to the native binary, not a Python console-script
wrapper.

Requires `kernel.unprivileged_userns_clone=1` (the default on most distros).

On Ubuntu 24.04, the most common failure is AppArmor blocking unprivileged user
namespaces. The usual symptom is that `sbrun` fails before starting your
command with an error like:

- `write /proc/self/setgroups: Permission denied`
- `write /proc/self/uid_map: Operation not permitted`

You can confirm the host setup with:

```sh
unshare --user --map-root-user --mount sh -c 'id -u; mount | head -1'
```

If that fails, `sbrun` will fail too.

Two ways to make `sbrun` work on affected Ubuntu systems:

1. keep the default unprivileged install and let `sbrun` install the persistent host setting:

```sh
sudo sbrun --kernel-install
```

That writes:

```sh
cat <<'EOF'
kernel.unprivileged_userns_clone=1
kernel.apparmor_restrict_unprivileged_userns=0
EOF
```

and then runs `sysctl --system`.

2. install the native `sbrun` binary setuid root instead, which needs no
kernel setting change:

```sh
sudo install -o root -g root -m 4755 sbrun /usr/local/bin/sbrun
```

GitHub-hosted Linux runners currently hit this restriction too, so this repo
only runs full sandbox integration tests on macOS in GitHub Actions.

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

Build, test, and release notes live in [`DEV.md`](DEV.md).
