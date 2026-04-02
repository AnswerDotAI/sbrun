#!/bin/bash

set -euo pipefail

cd "$(dirname "$0")/.."

version="$(python3 - <<'PY'
import tomllib
from pathlib import Path

cargo = tomllib.loads(Path("Cargo.toml").read_text())
print(cargo["package"]["version"])
PY
)"

case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "release.sh: unsupported version format: $version" >&2; exit 1 ;;
esac

tag="v${version}"

if ! git diff --quiet || ! git diff --cached --quiet || [ -n "$(git ls-files --others --exclude-standard)" ]; then
    echo "release.sh: worktree must be clean before tagging ${tag}" >&2
    exit 1
fi

if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
    echo "release.sh: tag ${tag} already exists" >&2
    exit 1
fi

git tag -a "$tag" -m "Release ${tag}"
git push origin HEAD
git push origin "$tag"
