# Development

Developer workflow and release notes for `sbrun`.

## Layout

- `src/main.rs`: CLI entrypoint
- `src/lib.rs`: shared runtime plus PyO3 module
- `src/sandbox.rs`: the small `unsafe` bridge to `libsandbox`
- `tests/cli.sh`: end-to-end shell integration tests
- `tools/test.sh`: standard local verification entrypoint

The repo intentionally ships one implementation: Rust. There is no C fallback,
Perl fallback, or compatibility layer for the old config or CLI.

## Local Build

Build the CLI:

```sh
cargo build
```

Build an optimized binary:

```sh
cargo build --release
```

Install the Python extension into the active virtualenv:

```sh
maturin develop --release
```

The compiled binary applies the sandbox profile directly through `libsandbox`.

## Testing

Run the full local verification suite with:

```sh
tools/test.sh
```

That runs:

- `cargo test`
- `cargo build`
- `tests/cli.sh`
- a `maturin develop` smoke test for the Python `exec` API when `maturin` is available

These integration tests are macOS-specific and need to run outside any parent
sandbox that would block nested seatbelt operations.

## Versioning

The canonical version lives in `Cargo.toml`.

Bump the patch version with:

```sh
tools/bump.sh
```

## Release

Push a tag like `v0.0.3` to trigger the GitHub Actions release workflow in
`.github/workflows/release.yml`.

The workflow:

- installs Rust and Python
- runs `tools/test.sh`
- builds `target/release/sbrun`
- builds the Python wheel with `maturin build --release --strip`
- packages `sbrun`, `README.md`, and `sbrun.default.toml`
- uploads a versioned tarball like `sbrun-v0.0.3-macos-arm64.tar.gz`
- uploads `SHA256SUMS`

For the local release flow:

1. run `tools/bump.sh`
2. review and commit the version change
3. run `tools/release.sh`

## PyPI

Publish the Python package with:

```sh
maturin publish --release --strip
```

This repo targets macOS arm64 wheels built from the same single Rust crate used
for the CLI.
