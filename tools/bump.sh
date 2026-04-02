#!/bin/bash

set -euo pipefail

cd "$(dirname "$0")/.."

current_version="$(python3 - <<'PY'
import tomllib
from pathlib import Path

cargo = tomllib.loads(Path("Cargo.toml").read_text())
print(cargo["package"]["version"])
PY
)"

case "$current_version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "bump.sh: unsupported version format: $current_version" >&2; exit 1 ;;
esac

IFS=. read -r major minor patch <<<"$current_version"
new_version="${major}.${minor}.$((patch + 1))"

perl -0pi -e 's/^version = "[^"]+"/version = "'"$new_version"'"/m' Cargo.toml

printf '%s\n' "$new_version"
