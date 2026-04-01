# Development

Developer workflow and release notes for `sbrun`.

## Local build

Build the compiled binary:

```sh
make
```

Install from source:

```sh
sudo make install
```

Install the Perl variant instead:

```sh
sudo make install-perl
```

The compiled `sbrun` binary applies the sandbox profile directly through
`libsandbox`.
The Perl variant in `sbrun.pl` mirrors the same CLI and config format, but
uses `/usr/bin/sandbox-exec`.

The Makefile defaults to `MACOSX_DEPLOYMENT_TARGET=13.0`. Override it when
needed:

```sh
make MACOSX_DEPLOYMENT_TARGET=14.0
```

`make install` and `make install-perl` also install the default global
allow-list config to `$(PREFIX)/etc/xdg/sbrun/config` if one does not already
exist.

## Testing

Run the local verification suite with:

```sh
pytest -q
```

The test suite covers:

- `make`, strict C compile, and `perl -c`
- runtime behavior for both `sbrun` and `sbrun.pl`
- interactive shell startup
- direct command mode and shell-flag mode
- writable directory and exact-file allow-lists
- envdir behavior via `-e/--envdir`
- redirect guarding for stdout/stderr

These tests are macOS-specific and need to run outside any parent sandbox that
would block nested seatbelt/sandbox operations.

## Versioning

The canonical version lives in `VERSION`.

- the C binary embeds that version at build time
- `sbrun.pl` reads `VERSION` when available in the repo and otherwise falls
  back to its built-in copy
- both variants expose it via `--version`

Bump the patch version with:

```sh
tools/bump.sh
```

That updates `VERSION` and the Perl fallback version string together.

## Release

Push a tag like `v0.1.0` to trigger the GitHub Actions release workflow in
`.github/workflows/release.yml`.

The workflow:

- runs on `macos-latest`
- installs `pytest`
- runs `make`
- runs `pytest -q`
- packages `sbrun`, `sbrun.pl`, `README.md`, and `sbrun.default.conf`
- uploads a versioned tarball like `sbrun-v0.1.0-macos-arm64.tar.gz`
- uploads `SHA256SUMS`

If a release for the tag already exists, the workflow updates the assets with
`gh release upload --clobber`.

For the local release flow:

1. run `tools/bump.sh`
2. review and commit the version change
3. run `tools/release.sh`

`tools/release.sh` reads `VERSION`, creates an annotated `vX.Y.Z` tag, pushes
the current `HEAD`, and then pushes the tag.

## Notes

- `README.md` is intended to stay user-facing; keep developer workflow details
  here.
- The repo currently ships both the compiled binary and the Perl fallback, so
  behavior changes should normally be implemented and tested in both.
