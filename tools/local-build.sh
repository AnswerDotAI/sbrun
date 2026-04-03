#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
cargo build
cp target/debug/sbrun sbrun-bin
echo "Installed sbrun-bin"
if command -v maturin &>/dev/null; then
    maturin develop
    echo "Python package installed into active venv"
fi
