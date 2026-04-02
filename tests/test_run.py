import os
import pty
import select
import shlex
import shutil
import signal
import subprocess
import termios
import time
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONF = ROOT / "sbrun.default.conf"
VERSION_FILE = ROOT / "VERSION"
IMPLEMENTATIONS = [
    pytest.param(ROOT / "sbrun", id="c"),
    pytest.param(ROOT / "sbrun.pl", id="perl"),
]


def fmt_cmd(cmd: list[str | Path]) -> str:
    return shlex.join(str(part) for part in cmd)


def run_cmd(cmd: list[str | Path], *, env: dict[str, str] | None = None, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        [str(part) for part in cmd],
        cwd=ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"{fmt_cmd(cmd)} failed with exit code {result.returncode}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def run_impl(
    impl: Path,
    *args: str,
    env: dict[str, str],
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    return run_cmd([impl, *args], env=env, check=check)


def history_file_name(shell_path: str) -> str:
    name = Path(shell_path).name
    if name == "bash":
        return ".bash_history"
    if name == "zsh":
        return ".zsh_history"
    return ".sh_history"


def interactive_shell_output(impl: Path, env: dict[str, str]) -> str:
    child_env = dict(env)
    child_env["PS1"] = ""
    child_env["PROMPT_COMMAND"] = ""

    pid, master = pty.fork()
    if pid == 0:
        os.execvpe(str(impl), [str(impl)], child_env)

    attrs = termios.tcgetattr(master)
    attrs[3] &= ~termios.ECHO
    termios.tcsetattr(master, termios.TCSANOW, attrs)
    os.write(
        master,
        b'printf "__SBBASH_TEST__SHELL=%s\\n__SBBASH_TEST__HOME=%s\\n__SBBASH_TEST__HISTFILE=%s\\n" "$SHELL" "$HOME" "$HISTFILE"\nexit\n',
    )

    chunks: list[bytes] = []
    deadline = time.time() + 15.0
    status = None
    while time.time() < deadline:
        ready, _, _ = select.select([master], [], [], 0.5)
        if ready:
            try:
                data = os.read(master, 4096)
            except OSError:
                break
            if not data:
                break
            chunks.append(data)

        waited, raw_status = os.waitpid(pid, os.WNOHANG)
        if waited == pid:
            status = raw_status
            if not ready:
                break

    if status is None:
        os.kill(pid, signal.SIGTERM)
        _, status = os.waitpid(pid, 0)

    try:
        while True:
            data = os.read(master, 4096)
            if not data:
                break
            chunks.append(data)
    except OSError:
        pass
    finally:
        os.close(master)

    if os.WIFEXITED(status):
        exit_code = os.WEXITSTATUS(status)
    elif os.WIFSIGNALED(status):
        exit_code = 128 + os.WTERMSIG(status)
    else:
        exit_code = 1

    output = b"".join(chunks).decode("utf-8", "replace")
    assert exit_code == 0, output
    return output


@pytest.fixture(scope="session")
def shell_path() -> str:
    for candidate in [os.environ.get("SHELL"), "/opt/homebrew/bin/bash", "/bin/bash"]:
        if candidate and Path(candidate).is_file() and os.access(candidate, os.X_OK):
            return candidate
    pytest.fail("could not find an executable shell for runtime tests")


@pytest.fixture(scope="session")
def expected_histfile(shell_path: str) -> str:
    return str(Path.home() / history_file_name(shell_path))


@pytest.fixture(scope="session")
def expected_version() -> str:
    return VERSION_FILE.read_text().strip()


@pytest.fixture(scope="session")
def xdg_config_dirs(tmp_path_factory: pytest.TempPathFactory) -> str:
    config_root = tmp_path_factory.mktemp("sbrun-xdg")
    config_dir = config_root / "sbrun"
    config_dir.mkdir()
    shutil.copy(DEFAULT_CONF, config_dir / "config")
    return str(config_root)


@pytest.fixture(scope="session")
def xdg_config_home(tmp_path_factory: pytest.TempPathFactory) -> str:
    return str(tmp_path_factory.mktemp("sbrun-xdg-home"))


@pytest.fixture(scope="session")
def runtime_env(shell_path: str, xdg_config_dirs: str, xdg_config_home: str) -> dict[str, str]:
    env = os.environ.copy()
    env["SHELL"] = shell_path
    env["XDG_CONFIG_DIRS"] = xdg_config_dirs
    env["XDG_CONFIG_HOME"] = xdg_config_home
    env["HISTSIZE"] = "0"
    env["HISTFILESIZE"] = "0"
    env["SAVEHIST"] = "0"
    return env


@pytest.fixture(scope="session")
def built_binary() -> Path:
    run_cmd(["make"])
    return ROOT / "sbrun"


def test_make_builds() -> None:
    run_cmd(["make", "clean"])
    run_cmd(["make"])


def test_strict_c_compile() -> None:
    cc = os.environ.get("CC", "cc")
    run_cmd([cc, "-O2", "-Wall", "-Wextra", "-Wpedantic", "-Werror", "-std=c11", "-o", "/tmp/sbrun-check", "sbrun.c", "-lsandbox"])


def test_clang_analyze_is_clean() -> None:
    run_cmd(["clang", "--analyze", "-Xanalyzer", "-analyzer-output=text", "sbrun.c"])


def test_perl_compiles() -> None:
    run_cmd(["perl", "-c", "sbrun.pl"])


def test_install_config_copies_default(tmp_path: Path) -> None:
    install_root = tmp_path / "install-root"
    run_cmd(["make", "install-config", f"DESTDIR={install_root}"])
    installed = install_root / "usr/local/etc/xdg/sbrun/config"
    assert installed.is_file()
    assert installed.read_text() == DEFAULT_CONF.read_text()


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_help_output(impl: Path, built_binary: Path, runtime_env: dict[str, str]) -> None:
    out = run_impl(impl, "--help", env=runtime_env).stdout
    assert "Usage:" in out
    assert "--version" in out
    assert "--writable PATH" in out
    assert "--envdir VAR" in out
    assert "--unsetenv VAR" in out


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_version_output(impl: Path, built_binary: Path, runtime_env: dict[str, str], expected_version: str) -> None:
    out = run_impl(impl, "--version", env=runtime_env).stdout.strip()
    assert out.endswith(f" {expected_version}")


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_shell_mode_sets_expected_env(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
    expected_histfile: str,
) -> None:
    out = run_impl(
        impl,
        "-c",
        'printf "HOME=%s\\nTMPDIR=%s\\nHISTFILE=%s\\nSBRUN_ACTIVE=%s\\n" "$HOME" "$TMPDIR" "$HISTFILE" "$SBRUN_ACTIVE"',
        env=runtime_env,
    ).stdout
    assert f"HOME={Path.home()}" in out
    assert "TMPDIR=/tmp" in out
    assert f"HISTFILE={expected_histfile}" in out
    assert "SBRUN_ACTIVE=1" in out


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_no_arg_mode_starts_interactive_shell(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
    shell_path: str,
    expected_histfile: str,
) -> None:
    out = interactive_shell_output(impl, runtime_env)
    assert f"__SBBASH_TEST__SHELL={shell_path}" in out
    assert f"__SBBASH_TEST__HOME={Path.home()}" in out
    assert f"__SBBASH_TEST__HISTFILE={expected_histfile}" in out


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_histfile_can_be_opened(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    out = run_impl(impl, "-c", 'exec 3>>"$HISTFILE"; exec 3>&-; printf ok', env=runtime_env).stdout
    assert out == "ok"


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_shell_path_is_preserved(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
    shell_path: str,
) -> None:
    out = run_impl(impl, "-c", 'printf %s "$SHELL"', env=runtime_env).stdout
    assert out == shell_path


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_direct_command_mode_preserves_flags(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    out = run_impl(impl, "python3", "-X", "utf8", "-c", "import sys; print(sys.flags.utf8_mode)", env=runtime_env).stdout
    assert out.strip() == "1"


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_force_command_mode_works(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    out = run_impl(impl, "--", "python3", "-c", 'print("forced")', env=runtime_env).stdout
    assert out.strip() == "forced"


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_tmp_write_is_allowed(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
    tmp_path: Path,
) -> None:
    out_path = Path(f"/tmp/sbrun-test-out.{time.time_ns()}")
    env = dict(runtime_env, TMP_OUT=str(out_path))
    try:
        run_impl(impl, "python3", "-c", 'import os; open(os.environ["TMP_OUT"], "w").write("ok")', env=env)
        assert out_path.read_text() == "ok"
    finally:
        out_path.unlink(missing_ok=True)


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_home_write_is_denied(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    deny_path = Path.home() / f".sbrun-denied-test.{time.time_ns()}"
    env = dict(runtime_env, DENY_PATH=str(deny_path))
    try:
        result = run_impl(
            impl,
            "python3",
            "-c",
            'from pathlib import Path; import os; Path(os.environ["DENY_PATH"]).write_text("x")',
            env=env,
            check=False,
        )
        assert result.returncode != 0
        assert not deny_path.exists()
    finally:
        deny_path.unlink(missing_ok=True)


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_stdio_redirect_guard_blocks_unlisted_home_file(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    redirect_path = Path.home() / f".sbrun-redirect-test.{time.time_ns()}"
    try:
        with redirect_path.open("w") as handle:
            result = subprocess.run(
                [str(impl), "-c", "printf blocked"],
                cwd=ROOT,
                env=runtime_env,
                text=True,
                stdout=handle,
                stderr=subprocess.PIPE,
                check=False,
            )
        assert result.returncode != 0
        assert "refusing to start" in result.stderr
    finally:
        redirect_path.unlink(missing_ok=True)


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_writable_dir_flag_allows_directory(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    allowed_dir = Path.home() / f".sbrun-cli-writable.{time.time_ns()}"
    allowed_file = allowed_dir / "allowed.txt"
    env = dict(runtime_env, CLI_FILE=str(allowed_file))
    allowed_dir.mkdir()
    try:
        denied = run_impl(
            impl,
            "python3",
            "-c",
            'import os; open(os.environ["CLI_FILE"], "w").write("x")',
            env=env,
            check=False,
        )
        assert denied.returncode != 0

        run_impl(
            impl,
            "-w",
            str(allowed_dir),
            "python3",
            "-c",
            'import os; open(os.environ["CLI_FILE"], "w").write("ok")',
            env=env,
        )
        assert allowed_file.read_text() == "ok"
    finally:
        allowed_file.unlink(missing_ok=True)
        allowed_dir.rmdir()


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_writable_file_flag_allows_exact_file_and_redirect(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    allowed_file = Path.home() / f".sbrun-cli-file.{time.time_ns()}"
    allowed_file.write_text("")
    env = dict(runtime_env, CLI_FILE=str(allowed_file))
    try:
        denied = run_impl(
            impl,
            "python3",
            "-c",
            'import os; open(os.environ["CLI_FILE"], "w").write("x")',
            env=env,
            check=False,
        )
        assert denied.returncode != 0

        run_impl(
            impl,
            "-w",
            str(allowed_file),
            "python3",
            "-c",
            'import os; open(os.environ["CLI_FILE"], "w").write("ok")',
            env=env,
        )
        assert allowed_file.read_text() == "ok"

        with allowed_file.open("w") as handle:
            result = subprocess.run(
                [str(impl), "-w", str(allowed_file), "-c", "printf redirected"],
                cwd=ROOT,
                env=runtime_env,
                text=True,
                stdout=handle,
                stderr=subprocess.PIPE,
                check=False,
            )
        assert result.returncode == 0, result.stderr
        assert allowed_file.read_text() == "redirected"
    finally:
        allowed_file.unlink(missing_ok=True)


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_envdir_flag_sets_project_local_directories(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    stamp = time.time_ns()
    var1 = f"SBRUN_TEST_ENVDIR_{stamp}"
    var2 = f"SBRUN_TEST_ENVDIR_B_{stamp}"
    envdir_root = ROOT / ".sbrun"
    dir1 = envdir_root / var1
    dir2 = envdir_root / var2
    file1 = dir1 / "one.txt"
    file2 = dir2 / "two.txt"
    env = dict(runtime_env, TEST_ENV_NAMES=f"{var1}:{var2}", **{var1: "/tmp/original-one", var2: "/tmp/original-two"})
    try:
        out = run_impl(
            impl,
            "-e",
            var1,
            "--envdir",
            var2,
            "python3",
            "-c",
            'import os; from pathlib import Path; '
            'names=os.environ["TEST_ENV_NAMES"].split(":"); '
            'pairs=[f"{name}={Path(os.environ[name])}" for name in names]; '
            '[(Path(os.environ[name]) / ("one.txt" if i == 0 else "two.txt")).write_text(name) for i, name in enumerate(names)]; '
            'print("\\n".join(pairs))',
            env=env,
        ).stdout
        assert f"{var1}={dir1}" in out
        assert f"{var2}={dir2}" in out
        assert file1.read_text() == var1
        assert file2.read_text() == var2
    finally:
        file1.unlink(missing_ok=True)
        file2.unlink(missing_ok=True)
        dir1.rmdir() if dir1.exists() else None
        dir2.rmdir() if dir2.exists() else None
        envdir_root.rmdir() if envdir_root.exists() and not any(envdir_root.iterdir()) else None


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_envdir_flag_rejects_invalid_names(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    result = run_impl(impl, "-e", "BAD-NAME", "python3", "-c", 'print("nope")', env=runtime_env, check=False)
    assert result.returncode != 0
    assert "invalid envdir variable name BAD-NAME" in result.stderr


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_unsetenv_flag_removes_requested_variables(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    stamp = time.time_ns()
    var1 = f"SBRUN_TEST_UNSETENV_{stamp}"
    var2 = f"SBRUN_TEST_UNSETENV_B_{stamp}"
    env = dict(runtime_env, TEST_ENV_NAMES=f"{var1}:{var2}", **{var1: "one", var2: "two"})
    out = run_impl(
        impl,
        "-v",
        var1,
        "--unsetenv",
        var1,
        f"--unsetenv={var2}",
        "python3",
        "-c",
        'import os; names=os.environ["TEST_ENV_NAMES"].split(":"); '
        'print("\\n".join(f"{name}={int(name in os.environ)}" for name in names))',
        env=env,
    ).stdout
    assert f"{var1}=0" in out
    assert f"{var2}=0" in out


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_unsetenv_flag_rejects_invalid_names(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    result = run_impl(impl, "-v", "BAD-NAME", "python3", "-c", 'print("nope")', env=runtime_env, check=False)
    assert result.returncode != 0
    assert "invalid unsetenv variable name BAD-NAME" in result.stderr


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
def test_unsetenv_flag_rejects_reserved_names(
    impl: Path,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    result = run_impl(impl, "--unsetenv", "PATH", "python3", "-c", 'print("nope")', env=runtime_env, check=False)
    assert result.returncode != 0
    assert "cannot unset reserved environment variable PATH" in result.stderr


@pytest.mark.parametrize("impl", IMPLEMENTATIONS)
@pytest.mark.parametrize("order", ["envdir-first", "unsetenv-first"])
def test_unsetenv_flag_conflicts_with_envdir(
    impl: Path,
    order: str,
    built_binary: Path,
    runtime_env: dict[str, str],
) -> None:
    stamp = time.time_ns()
    name = f"SBRUN_TEST_CONFLICT_{stamp}"
    args = ("-e", name, "--unsetenv", name) if order == "envdir-first" else ("-v", name, "--envdir", name)
    result = run_impl(impl, *args, "python3", "-c", 'print("nope")', env=runtime_env, check=False)
    assert result.returncode != 0
    assert f"cannot use --envdir and --unsetenv for the same variable {name}" in result.stderr
