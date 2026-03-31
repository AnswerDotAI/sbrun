# sbbash

`sbbash` launches commands under the macOS sandbox and only allows writes
beneath the directory where `sbbash` was started.

## Properties

- reads are broadly allowed, writes are confined to the launch directory tree
- with no arguments, `sbbash` launches your `$SHELL` as an interactive login shell
- with arguments, `sbbash` runs that command directly, preserving flags and argv
- if the first argument starts with `-`, `sbbash` passes those flags to your shell
- `-w PATH` or `--writable PATH` adds an extra writable file or directory; you can repeat it
- `HOME` stays your real home directory when one is available
- `TMPDIR` is set to `/tmp`
- the shell's normal history file is writable by default
- extra file descriptors `>= 3` are closed before entering the sandbox
- on macOS, if stdout or stderr is redirected to a regular file outside the
  allowed writable directories, `sbbash` refuses to start unless you set
  `SBBASH_ALLOW_STDIO_REDIRECTS=1`

## Build

```sh
make
sudo make install
```

Run the local verification suite with:

```sh
pytest -q
```

The compiled `sbbash` binary applies the sandbox profile directly through
`libsandbox`.
By default the Makefile targets macOS 13.0 on the build architecture; override
`MACOSX_DEPLOYMENT_TARGET` if you need a different minimum version.
`make install` also installs a default global allow-list config to
`$(PREFIX)/etc/xdg/sbbash/config` if one does not already exist.

No-build Perl variant:

```sh
chmod +x sbbash.pl
./sbbash.pl python3 -c 'print(1)'
sudo make install-perl
```

`sbbash.pl` mirrors the same CLI and config format as `sbbash`, but uses the
system Perl runtime and `/usr/bin/sandbox-exec` instead of a compiled binary.

## Use

Interactive shell:

```sh
cd /path/to/project
sbbash
```

Run a command directly:

```sh
cd /path/to/project
sbbash ipython --profile-dir=.ipython profile list
```

Allow writes to an extra directory:

```sh
cd /path/to/project
sbbash -w /tmp python3 -c 'open("/tmp/sbbash-demo", "w").write("ok")'
```

Run a shell snippet:

```sh
cd /path/to/project
sbbash -lc 'touch ok.txt && echo hello'
```

You can combine `sbbash` options with shell mode:

```sh
cd /path/to/project
sbbash -w /tmp -lc 'echo hi > /tmp/hi.txt'
```

If you need to stop parsing `sbbash` options and force command mode, use `--`:

```sh
cd /path/to/project
sbbash -w /tmp -- ipython --profile-dir=/tmp/ipython
```

The Perl variant is used the same way:

```sh
cd /path/to/project
./sbbash.pl ipython --profile-dir=.ipython profile list
```

Help is available in both variants:

```sh
sbbash --help
./sbbash.pl --help
```

## Config

Global extra writable paths can be set in:

- `$XDG_CONFIG_DIRS/.../sbbash/config`
- `$XDG_CONFIG_HOME/sbbash/config`
- `~/.config/sbbash/config` when `XDG_CONFIG_HOME` is unset

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

## Notes

- The compiled `sbbash` binary applies the sandbox profile directly through
  `libsandbox`; `sbbash.pl` shells out through `/usr/bin/sandbox-exec`.
- `sbbash` is now optimized for "real home, restricted writes" rather than
  "fake home". That means shell startup and config discovery behave normally,
  but writes still need to land in the work tree, the installed allow-list,
  the shell's normal history file, or paths explicitly added with `-w`.
- The sandbox only blocks acquiring new writable resources. If the parent shell
  already gave the process an open writable fd, that fd can still be used.
  `sbbash` mitigates this by closing fds `>= 3` and by rejecting regular-file
  stdout/stderr redirections outside the allowed writable directories on macOS.
