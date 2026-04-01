#!/bin/bash

set -euo pipefail

cd "$(dirname "$0")/.."

version_file="VERSION"
perl_file="sbrun.pl"
python_file="src/sbrun/__init__.py"
c_file="sbrun.c"

current_version="$(tr -d '\n' < "$version_file")"
case "$current_version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "bump.sh: unsupported version format: $current_version" >&2; exit 1 ;;
esac

IFS=. read -r major minor patch <<<"$current_version"
new_version="${major}.${minor}.$((patch + 1))"

printf '%s\n' "$new_version" > "$version_file"
perl -0pi -e 's/use constant BUILTIN_VERSION => "[^"]+";/use constant BUILTIN_VERSION => "'"$new_version"'";/' "$perl_file"
perl -0pi -e 's/__version__ = "[^"]+"/__version__ = "'"$new_version"'"/' "$python_file"
perl -0pi -e 's/#define SBRUN_VERSION "[^"]+"/#define SBRUN_VERSION "'"$new_version"'"/' "$c_file"

printf '%s\n' "$new_version"
