#!/bin/bash

set -euo pipefail

cd "$(dirname "$0")/.."

version_file="VERSION"
perl_file="sbrun.pl"

current_version="$(tr -d '\n' < "$version_file")"
case "$current_version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "bump.sh: unsupported version format: $current_version" >&2; exit 1 ;;
esac

IFS=. read -r major minor patch <<<"$current_version"
new_version="${major}.$((minor + 1)).0"

printf '%s\n' "$new_version" > "$version_file"
perl -0pi -e 's/use constant BUILTIN_VERSION => "[^"]+";/use constant BUILTIN_VERSION => "'"$new_version"'";/' "$perl_file"

printf '%s\n' "$new_version"
