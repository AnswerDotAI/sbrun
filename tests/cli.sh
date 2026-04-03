#!/bin/bash

set -euo pipefail

cd "$(dirname "$0")/.."

bin="${1:-target/debug/sbrun}"
[ -x "$bin" ] || { echo "missing test binary: $bin" >&2; exit 1; }

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/sbrun-test.XXXXXX")"
mkdir -p meta
cleanup() {
    rm -rf "$tmpdir"
    find .sbrun -mindepth 1 -maxdepth 1 -type d -name 'SBRUN_TEST_*' -exec rm -rf {} + 2>/dev/null || true
    rmdir .sbrun 2>/dev/null || true
    rm -f meta/.sbrun-test-*.stderr
}
trap cleanup EXIT

xdg_home="${tmpdir}/xdg-home"
xdg_dirs="${tmpdir}/xdg-dirs"
mkdir -p "${xdg_home}" "${xdg_dirs}"

shell_path="${SHELL:-/bin/bash}"
[ -x "$shell_path" ] || shell_path="/bin/bash"

run_env() {
    env XDG_CONFIG_HOME="$xdg_home" XDG_CONFIG_DIRS="$xdg_dirs" SHELL="$shell_path" "$bin" "$@"
}

assert_contains() {
    local haystack="$1" needle="$2"
    [[ "$haystack" == *"$needle"* ]] || { echo "expected output to contain: $needle" >&2; exit 1; }
}

expected_version="$(python3 - <<'PY'
import tomllib
from pathlib import Path
print(tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"])
PY
)"

help_out="$("$bin" --help)"
assert_contains "$help_out" "--write PATH"
assert_contains "$help_out" "--env-dir VAR"
assert_contains "$help_out" "--unset-env VAR"
assert_contains "$help_out" "--config PATH"
assert_contains "$help_out" "--no-config"

version_out="$("$bin" --version)"
assert_contains "$version_out" "sbrun ${expected_version}"

env_out="$(run_env -c 'printf "HOME=%s\nTMPDIR=%s\nHISTFILE=%s\nSBRUN_ACTIVE=%s\n" "$HOME" "$TMPDIR" "$HISTFILE" "$SBRUN_ACTIVE"')"
assert_contains "$env_out" "TMPDIR=/tmp"
assert_contains "$env_out" "SBRUN_ACTIVE=1"
assert_contains "$env_out" "HOME=${HOME}"

direct_out="$(run_env python3 -X utf8 -c 'import sys; print(sys.flags.utf8_mode)')"
[[ "$direct_out" == "1" ]] || { echo "expected utf8_mode=1, got: $direct_out" >&2; exit 1; }

forced_out="$(run_env -- python3 -c 'print("forced")')"
[[ "$forced_out" == "forced" ]] || { echo "expected forced, got: $forced_out" >&2; exit 1; }

tmp_file="/tmp/sbrun-test.$$.txt"
run_env --write /tmp -- python3 -c 'import sys; open(sys.argv[1], "w").write("ok")' "$tmp_file"
[[ "$(cat "$tmp_file")" == "ok" ]] || { echo "expected tmp write to succeed" >&2; exit 1; }
rm -f "$tmp_file"

deny_file="${HOME}/.sbrun-denied-test.$$"
deny_stderr="meta/.sbrun-test-deny.$$.stderr"
if run_env -- python3 -c 'import sys; open(sys.argv[1], "w").write("nope")' "$deny_file" >/dev/null 2>"${deny_stderr}"; then
    echo "expected home write to be denied" >&2
    exit 1
fi
grep -Eq 'Operation not permitted|PermissionError|Read-only file system' "${deny_stderr}"
rm -f "$deny_file"

env_name="SBRUN_TEST_ENVDIR_$$"
env_out="$(run_env --env-dir "$env_name" -- python3 -c 'import os, pathlib, sys; p = pathlib.Path(os.environ[sys.argv[1]]); (p / "ok.txt").write_text("ok"); print(p)' "$env_name")"
assert_contains "$env_out" "$(pwd)/.sbrun/${env_name}"
[[ "$(cat ".sbrun/${env_name}/ok.txt")" == "ok" ]] || { echo "expected env dir write to succeed" >&2; exit 1; }
rm -f ".sbrun/${env_name}/ok.txt"
rmdir ".sbrun/${env_name}"
rmdir .sbrun 2>/dev/null || true

unset_out="$(FOO=1 BAR=2 run_env --unset-env FOO --unset-env=BAR -- python3 -c 'import os; print(int("FOO" in os.environ), int("BAR" in os.environ))')"
[[ "$unset_out" == "0 0" ]] || { echo "expected env vars to be removed, got: $unset_out" >&2; exit 1; }

cat >"${tmpdir}/config.toml" <<'EOF'
version = 1
write = ["/tmp"]
EOF
config_out="/tmp/sbrun-config-test.$$.txt"
run_env --config "${tmpdir}/config.toml" -- python3 -c 'import sys; open(sys.argv[1], "w").write("config")' "$config_out"
[[ "$(cat "$config_out")" == "config" ]] || { echo "expected explicit config write to succeed" >&2; exit 1; }
rm -f "$config_out"

mkdir -p "${xdg_home}/sbrun"
cat >"${xdg_home}/sbrun/config.toml" <<'EOF'
version = 1
write = ["/tmp"]
EOF
implicit_out="/tmp/sbrun-implicit-test.$$.txt"
run_env -- python3 -c 'import sys; open(sys.argv[1], "w").write("implicit")' "$implicit_out"
[[ "$(cat "$implicit_out")" == "implicit" ]] || { echo "expected implicit config write to succeed" >&2; exit 1; }
rm -f "$implicit_out"

redirect_file="${HOME}/.sbrun-redirect-test.$$"
redirect_stderr="meta/.sbrun-test-redirect.$$.stderr"
if run_env -- python3 -c 'print("blocked")' >"$redirect_file" 2>"${redirect_stderr}"; then
    echo "expected redirected stdout to be denied" >&2
    exit 1
fi
grep -q "outside allowed writable paths" "${redirect_stderr}"
rm -f "$redirect_file"

SBRUN_ALLOW_STDIO_REDIRECTS=1 run_env -- python3 -c 'print("allowed")' >"$redirect_file"
[[ "$(cat "$redirect_file")" == "allowed" ]] || { echo "expected redirected stdout override to work" >&2; exit 1; }
rm -f "$redirect_file"

if command -v maturin >/dev/null 2>&1; then
    pyvenv="${tmpdir}/venv"
    python3 -m venv "$pyvenv"
    (
        source "${pyvenv}/bin/activate"
        maturin develop --quiet
        py_out="$(FOO=1 python -c 'import sbrun; sbrun.exec(["python3", "-c", "import os; print(int(\"FOO\" in os.environ))"], unset_env=["FOO"])')"
        [[ "$py_out" == "0" ]] || exit 1
    )
fi
