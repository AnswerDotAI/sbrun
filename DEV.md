# Development

Developer workflow and release notes for `sbrun`.

## Layout

- `src/main.rs`: CLI entrypoint
- `src/lib.rs`: shared runtime (CLI dispatch, sandbox orchestration, env setup)
- `src/cli.rs`: argument parsing and help text
- `src/admin.rs`: `--kernel-install` implementation
- `src/prompt.rs`: `--prompt-init` shell hook generation
- `src/sandbox.rs`: platform dispatcher (`#[cfg]` selects backend)
- `src/sandbox_macos.rs`: macOS Seatbelt FFI bridge
- `src/sandbox_linux.rs`: Linux user/mount namespace sandbox
- `src/profile.rs`: Seatbelt profile generation (macOS only)
- `src/config.rs`: TOML config loading
- `src/pathutil.rs`: path resolution and validation
- `src/host.rs`: host info detection (shell, home, user)
- `src/error.rs`: error types
- `tests/test_sbrun.py`: pytest integration tests
- `tools/test.sh`: standard local verification entrypoint

## Local Build

Build a debug binary and install the `sbrun` CLI into the active venv:

```sh
./tools/local-build.sh
```

Or manually:

```sh
python -m pip install -e '.[dev]'   # dev deps (pytest, maturin)
cargo build                          # produces target/debug/sbrun (used by the tests)
maturin develop                      # installs the sbrun binary onto the venv PATH
```

## Testing

Run the full local verification suite with:

```sh
tools/test.sh
```

That runs:

- `cargo test` — Rust unit tests (CLI parsing, env helpers, config, path utils, host detection)
- `cargo build`
- `pytest -q tests/test_sbrun.py` — integration tests (sandbox enforcement, config, environment)

Tests run on both macOS and Linux.
GitHub Actions runs the full suite on macOS from `.github/workflows/test.yml` on pushes to `main`.
On GitHub-hosted Linux it only runs `cargo test` and `cargo build`, because the hosted environment blocks the
user-namespace setup needed for the Linux sandbox integration tests. Use a self-hosted Linux runner, or any Linux
environment whose policy allows unprivileged user and mount namespaces, for full Linux integration coverage.

## Versioning

The canonical version lives in `Cargo.toml`.

Bump the patch version with:

```sh
ship-rs-bump
```

## Release

Push a tag like `v0.0.3` to trigger the GitHub Actions release workflow in
`.github/workflows/release.yml`.

The workflow builds on both macOS and Linux in parallel:

- installs Rust and Python
- builds `target/release/sbrun`
- builds the wheel (the `sbrun` binary packaged for `pip install`, via maturin `bin` bindings) with `maturin build --release --strip`
- packages platform-specific tarballs (e.g. `sbrun-v0.0.3-macos-arm64.tar.gz`,
  `sbrun-v0.0.3-linux-x86_64.tar.gz`)
- uploads all assets and wheels to a single GitHub release

For the local release flow:

1. run `ship-rs-bump`
2. review and commit the version change
3. run `ship-rs-release`

## PyPI

Publish the Python package with:

```sh
maturin publish --release --strip
```

The CI workflow publishes both macOS and Linux wheels to PyPI automatically.

## Platform notes

**macOS**: sandbox is applied via Seatbelt (`libsandbox`). Requires macOS.

**Linux**: default sandbox uses unprivileged user namespaces + mount
namespaces (inspired by [bubblewrap](https://github.com/containers/bubblewrap)).
When the native `sbrun` binary is installed setuid root, the same binary
automatically switches to a privileged mount-namespace backend instead and
drops back to the caller before `exec()`. Default unprivileged mode still
requires `kernel.unprivileged_userns_clone=1` (default on most distros). The
CLI also supports `sudo sbrun --kernel-install`, which writes
`/etc/sysctl.d/90-sbrun.conf` and runs `sysctl --system` on Linux.
