#!/bin/bash

set -euo pipefail

cd "$(dirname "$0")/.."

cargo test
cargo build
pytest -q tests/test_sbrun.py
