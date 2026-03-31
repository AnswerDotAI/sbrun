# sbbash

`sbbash` launches commands under macOS `sandbox-exec` and only allows writes
beneath the directory where `sbbash` was started.

## Properties

- reads are broadly allowed, writes are confined to the launch directory tree
- with no arguments, `sbbash` launches your `$SHELL` as an interactive shell
- with arguments, `sbbash` runs that command directly, preserving flags and argv
- if the first argument starts with `-`, `sbbash` passes those flags to your shell
- `-w DIR` or `--writable DIR` adds an extra writable directory; you can repeat it
- `HOME` becomes `./.sbbash-home` when that directory can be created
- `TMPDIR` becomes `./.sbbash-tmp` when that directory can be created
- existing `.sbbash-home` and `.sbbash-tmp` paths must be real directories, not symlinks
- extra file descriptors `>= 3` are closed before entering the sandbox
- on macOS, if stdout or stderr is redirected to a regular file outside the
  allowed writable directories, `sbbash` refuses to start unless you set
  `SBBASH_ALLOW_STDIO_REDIRECTS=1`

## Build

```sh
make
sudo make install
```

No-build Perl variant:

```sh
chmod +x sbbash.pl
./sbbash.pl python3 -c 'print(1)'
sudo make install-perl
```

`sbbash.pl` mirrors the same CLI and config format as `sbbash`, but uses the
system Perl runtime instead of a compiled binary.

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

Global extra writable directories can be set in:

- `$XDG_CONFIG_HOME/sbbash/config`
- `~/.config/sbbash/config` when `XDG_CONFIG_HOME` is unset

Use one `writable_dir=...` entry per line:

```ini
writable_dir=/tmp
writable_dir=~/scratch
```

Configured directories and `-w/--writable` directories are combined.

## Notes

- This relies on Apple `sandbox-exec`, which Apple marks deprecated.
- The sandbox only blocks acquiring new writable resources. If the parent shell
  already gave the process an open writable fd, that fd can still be used.
  `sbbash` mitigates this by closing fds `>= 3` and by rejecting regular-file
  stdout/stderr redirections outside the allowed writable directories on macOS.
