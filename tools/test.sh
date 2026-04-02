#!/bin/bash

set -euo pipefail

cd "$(dirname "$0")/.."

cargo test
cargo build
bash tests/cli.sh target/debug/sbrun
