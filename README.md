# sbrun

`sbrun` launches commands under the macOS sandbox and only allows writes beneath the directory where `sbrun` was started.

## Use

Interactive shell:

```sh
cd /path/to/project
sbrun
```

Run a command directly:

```sh
cd /path/to/project
sbrun ipython --profile-dir=.ipython profile list
```

Allow writes to an extra directory:

```sh
cd /path/to/project
sbrun -w /tmp python3 -c 'open("/tmp/sbrun-demo", "w").write("ok")'
```

Set specific environment variables to project-local directories:

```sh
cd /path/to/project
sbrun -e IPYTHONDIR -e MPLCONFIGDIR ipython
```

Use the long form when you prefer:

```sh
cd /path/to/project
sbrun --envdir=XDG_CACHE_HOME --envdir=XDG_STATE_HOME python3 app.py
```

Run a shell snippet:

```sh
cd /path/to/project
sbrun -lc 'touch ok.txt && echo hello'
```

You can combine `sbrun` options with shell mode:

```sh
cd /path/to/project
sbrun -w /tmp -lc 'echo hi > /tmp/hi.txt'
```

If you need to stop parsing `sbrun` options and force command mode, use `--`:

```sh
cd /path/to/project
sbrun -w /tmp -- ipython --profile-dir=/tmp/ipython
```

The Perl variant is used the same way:

```sh
cd /path/to/project
./sbrun.pl ipython --profile-dir=.ipython profile list
```

Help is available in both variants:

```sh
sbrun --help
sbrun --version
./sbrun.pl --help
./sbrun.pl --version
```

## Properties

- reads are broadly allowed, writes are confined to the launch directory tree
- with no arguments, `sbrun` launches your `$SHELL` as an interactive login shell
- with arguments, `sbrun` runs that command directly, preserving flags and argv
- if the first argument starts with `-`, `sbrun` passes those flags to your shell
- `-w PATH` or `--writable PATH` adds an extra writable file or directory; you can repeat it
- `-e VAR` or `--envdir VAR` sets `VAR` to `.sbrun/VAR`; you can repeat it
- `HOME` stays your real home directory when one is available
- `TMPDIR` is set to `/tmp`
- the shell's normal history file is writable by default
- extra file descriptors `>= 3` are closed before entering the sandbox
- on macOS, if stdout or stderr is redirected to a regular file outside the
  allowed writable paths, `sbrun` refuses to start unless you set
  `SBBASH_ALLOW_STDIO_REDIRECTS=1`

Development, build, test, and release notes live in `DEV.md`.

## Config

Global extra writable paths can be set in:

- `$XDG_CONFIG_DIRS/.../sbrun/config`
- `$XDG_CONFIG_HOME/sbrun/config`
- `~/.config/sbrun/config` when `XDG_CONFIG_HOME` is unset

Use one entry per line:

```ini
writable_path=/tmp
writable_path=~/scratch
optional_writable_path=~/.cache
```

`writable_path=...` is required and errors if the path does not resolve to an
existing regular file or directory.
`optional_writable_path=...` is ignored when the path does not resolve to an
existing regular file or directory, which is useful for shared default configs.
For compatibility, `writable_dir=...` and `optional_writable_dir=...` are also
accepted.

Configured paths and `-w/--writable` paths are combined. System config is
loaded first, then user config, then CLI flags.

`-e/--envdir VAR` is CLI-only. Each requested variable is set to
`.sbrun/VAR`, and those directories are created on demand inside the launch
directory.

## Envdir

`-e VAR`, `--envdir VAR`, and `--envdir=VAR` all mean the same thing.

- `VAR` must be a valid environment variable name: `[A-Za-z_][A-Za-z0-9_]*`
- `sbrun` creates `.sbrun/` only when at least one envdir flag is used
- each requested variable gets a directory at `.sbrun/VAR`
- the child process sees `VAR` set to that directory, even if `VAR` already had a different value
- repeated `-e/--envdir` flags are fine; duplicate names are ignored after the first
- envdir settings are CLI-only and are not read from config files

This is mainly useful for tools that want a writable state or cache directory
without granting broad write access to your real home directory. Typical
examples are `IPYTHONDIR`, `MPLCONFIGDIR`, `XDG_CACHE_HOME`, and
`XDG_STATE_HOME`.

The installed default global config includes a practical allow-list of common
user state/cache locations such as:

- `/tmp`
- `~/.config`
- `~/.cache`
- `~/.local/share`
- `~/.local/state`
- `~/.ipython`
- `~/.jupyter`
- `~/Library/Caches`

Edit the global config or your user config to tighten or extend that list.
