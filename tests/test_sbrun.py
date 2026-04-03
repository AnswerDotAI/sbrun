import os, subprocess, platform, pytest
from pathlib import Path

SBRUN = Path(__file__).resolve().parent.parent / "target/debug/sbrun"
IS_LINUX = platform.system() == "Linux"
IS_MACOS = platform.system() == "Darwin"

def sbrun(*args, env_override=None, cwd=None, stdin=None, allow_fail=False):
    env = {**os.environ, "SBRUN_ALLOW_STDIO_REDIRECTS": "1", "XDG_CONFIG_DIRS": "/nonexistent", "XDG_CONFIG_HOME": "/nonexistent"}
    if env_override: env.update(env_override)
    r = subprocess.run([str(SBRUN), *args], capture_output=True, text=True, env=env, cwd=cwd, input=stdin)
    if not allow_fail: assert r.returncode == 0, f"sbrun failed:\nstdout: {r.stdout}\nstderr: {r.stderr}"
    return r

@pytest.fixture
def tmp(tmp_path): return tmp_path

# -- CLI basics --

def test_help():
    r = sbrun("--help")
    for flag in ("--kernel-install", "--write PATH", "--env-dir VAR", "--unset-env VAR", "--config PATH", "--no-config"): assert flag in r.stdout

def test_version():
    cargo = Path(__file__).resolve().parent.parent / "Cargo.toml"
    import tomllib
    version = tomllib.loads(cargo.read_text())["package"]["version"]
    assert sbrun("--version").stdout.strip() == f"sbrun {version}"

def test_unknown_option():
    r = sbrun("--bogus", allow_fail=True)
    assert r.returncode != 0

def test_kernel_install_requires_linux_root_or_sudo():
    if IS_LINUX and os.getuid() == 0 and os.geteuid() == 0: pytest.skip("would modify host sysctl config")
    r = sbrun("--kernel-install", allow_fail=True)
    assert r.returncode != 0
    expected = "only available on linux" if not IS_LINUX else "requires running as root"
    assert expected in r.stderr.lower()

# -- Environment --

def test_sbrun_active(): assert sbrun("--no-config", "--", "sh", "-c", "echo $SBRUN_ACTIVE").stdout.strip() == "1"

def test_tmpdir(): assert sbrun("--no-config", "--", "sh", "-c", "echo $TMPDIR").stdout.strip() == "/tmp"

def test_home_preserved():
    home = os.environ.get("HOME", "")
    assert sbrun("--no-config", "--", "sh", "-c", "echo $HOME").stdout.strip() == home

def test_unset_env():
    r = sbrun("--no-config", "--unset-env", "SECRET", "--", "sh", "-c", "echo ${SECRET:-gone}", env_override={"SECRET": "hunter2"})
    assert r.stdout.strip() == "gone"

def test_direct_command(): assert sbrun("--no-config", "--", "sh", "-c", "echo direct").stdout.strip() == "direct"

def test_shell_command():
    r = sbrun("--no-config", "-c", "echo shell")
    assert r.stdout.strip() == "shell"

# -- Sandbox enforcement --

def test_cwd_writable(tmp):
    r = sbrun("--no-config", "--", "sh", "-c", "touch testfile && echo ok", cwd=tmp)
    assert r.stdout.strip() == "ok"

def test_root_readonly():
    r = sbrun("--no-config", "--", "sh", "-c", "touch /sbrun-nope 2>&1; echo $?")
    assert r.stdout.strip() != "0"

def test_home_readonly():
    home = os.environ.get("HOME", "/tmp")
    r = sbrun("--no-config", "--", "sh", "-c", f"touch {home}/.sbrun-nope 2>&1; echo $?")
    assert r.stdout.strip() != "0"

def test_tmp_writable():
    r = sbrun("--no-config", "-w", "/tmp", "--", "sh", "-c", "echo ok > /tmp/sbrun-pytest && cat /tmp/sbrun-pytest")
    assert r.stdout.strip() == "ok"

def test_extra_dir_writable(tmp):
    extra = tmp / "extra"
    extra.mkdir()
    r = sbrun("--no-config", "-w", str(extra), "--", "sh", "-c", f"touch {extra}/ok && echo ok", cwd=tmp)
    assert r.stdout.strip() == "ok"

def test_subdir_creation(tmp):
    r = sbrun("--no-config", "--", "sh", "-c", "mkdir -p a/b/c && echo hello > a/b/c/f && cat a/b/c/f", cwd=tmp)
    assert r.stdout.strip() == "hello"

def test_deny_write_shows_error():
    home = os.environ.get("HOME", "/tmp")
    r = sbrun("--no-config", "--", "sh", "-c", f"touch {home}/.sbrun-nope 2>&1", allow_fail=True)
    assert "Read-only file system" in r.stdout or "Operation not permitted" in r.stdout or r.returncode != 0

# -- Reads --

def test_read_system_files():
    r = sbrun("--no-config", "--", "sh", "-c", "head -1 /etc/hosts")
    assert r.stdout.strip() != ""

def test_binaries_on_path():
    r = sbrun("--no-config", "--", "sh", "-c", "which sh")
    assert "/sh" in r.stdout.strip()

# -- Config --

def test_explicit_config(tmp):
    cfg = tmp / "config.toml"
    cfg.write_text('version = 1\nwrite = ["/tmp"]\n')
    r = sbrun("--config", str(cfg), "--", "sh", "-c", "echo ok > /tmp/sbrun-cfg && cat /tmp/sbrun-cfg", cwd=tmp)
    assert r.stdout.strip() == "ok"

def test_xdg_config(tmp):
    xdg = tmp / "xdg" / "sbrun"
    xdg.mkdir(parents=True)
    (xdg / "config.toml").write_text('version = 1\nwrite = ["/tmp"]\n')
    xdg_env = {"XDG_CONFIG_HOME": str(tmp / "xdg"), "XDG_CONFIG_DIRS": "/nonexistent"}
    r = sbrun("--", "sh", "-c", "echo ok > /tmp/sbrun-xdg && cat /tmp/sbrun-xdg", cwd=tmp, env_override=xdg_env)
    assert r.stdout.strip() == "ok"

def test_missing_config_fails():
    r = sbrun("--config", "/nonexistent/config.toml", "--", "echo", "hi", allow_fail=True)
    assert r.returncode != 0
    assert "config" in r.stderr.lower()

def test_auto_creates_default_config(tmp):
    xdg = tmp / "xdg"
    cfg_path = xdg / "sbrun" / "config.toml"
    assert not cfg_path.exists()
    # First run creates the config
    sbrun("--", "sh", "-c", "echo ok", cwd=tmp, env_override={"XDG_CONFIG_HOME": str(xdg), "XDG_CONFIG_DIRS": "/nonexistent"})
    assert cfg_path.exists()
    content = cfg_path.read_text()
    assert "version = 1" in content
    assert "optional_write" in content
    # Second run should not print creation message
    r = sbrun("--", "sh", "-c", "echo ok", cwd=tmp, env_override={"XDG_CONFIG_HOME": str(xdg), "XDG_CONFIG_DIRS": "/nonexistent"})
    assert "created default config" not in r.stderr

# -- Redirect blocking --

def test_redirect_to_home_blocked(tmp):
    home = os.environ.get("HOME", "/tmp")
    target = Path(f"{home}/.sbrun-redirect-test")
    env = {**os.environ, "XDG_CONFIG_DIRS": "/nonexistent", "XDG_CONFIG_HOME": "/nonexistent"}
    with open(target, "w") as f:
        cmd = [str(SBRUN), "--no-config", "--", "sh", "-c", "echo blocked"]
        r = subprocess.run(cmd, stdout=f, stderr=subprocess.PIPE, text=True, env=env)
    assert r.returncode != 0
    assert "outside allowed writable paths" in r.stderr
    target.unlink(missing_ok=True)
