# Development

Developer workflow and release notes for `sbrun`.

## Layout

- `src/main.rs`: CLI entrypoint
- `src/lib.rs`: shared runtime plus PyO3 module
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

Build and install a debug binary plus Python extension:

```sh
./tools/local-build.sh
```

Or manually:

```sh
python -m pip install -e '.[dev]'
cargo build
maturin develop
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
GitHub Actions runs the same suite from `.github/workflows/test.yml` on pushes to `main`.

## Versioning

The canonical version lives in `Cargo.toml`.

Bump the patch version with:

```sh
tools/bump.sh
```

## Release

Push a tag like `v0.0.3` to trigger the GitHub Actions release workflow in
`.github/workflows/release.yml`.

The workflow builds on both macOS and Linux in parallel:

- installs Rust and Python
- builds `target/release/sbrun`
- builds the Python wheel with `maturin build --release --strip`
- packages platform-specific tarballs (e.g. `sbrun-v0.0.3-macos-arm64.tar.gz`,
  `sbrun-v0.0.3-linux-x86_64.tar.gz`)
- uploads all assets and wheels to a single GitHub release

For the local release flow:

1. run `tools/bump.sh`
2. review and commit the version change
3. run `tools/release.sh`

## PyPI

Publish the Python package with:

```sh
maturin publish --release --strip
```

The CI workflow publishes both macOS and Linux wheels to PyPI automatically.

## Platform notes

**macOS**: sandbox is applied via Seatbelt (`libsandbox`). Requires macOS.

**Linux**: sandbox uses unprivileged user namespaces + mount namespaces
(inspired by [bubblewrap](https://github.com/containers/bubblewrap)). No root
or setuid required. Requires `kernel.unprivileged_userns_clone=1` (default on
most distros).
