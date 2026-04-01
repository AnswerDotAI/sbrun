#!/usr/bin/env python3

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "py-dist"


def version_from_file(path: Path) -> str:
    return path.read_text().strip()


def version_from_package(path: Path) -> str:
    namespace: dict[str, str] = {}
    exec(path.read_text(), namespace)
    return namespace["__version__"]


def run(*cmd: str) -> None:
    subprocess.run(cmd, cwd=ROOT, check=True)


def main() -> None:
    version = version_from_file(ROOT / "VERSION")
    package_version = version_from_package(ROOT / "src/sbrun/__init__.py")
    if version != package_version:
        raise SystemExit(f"VERSION ({version}) does not match src/sbrun/__init__.py ({package_version})")

    shutil.rmtree(DIST, ignore_errors=True)
    DIST.mkdir(parents=True, exist_ok=True)

    run(sys.executable, "-m", "pip", "wheel", ".", "--no-deps", "--no-build-isolation", "--wheel-dir", str(DIST))

    wheels = sorted(DIST.glob("sbrun-*.whl"))
    if not wheels:
        raise SystemExit("no wheel was built")

    wheel_paths = [str(path) for path in wheels]
    run(sys.executable, "-m", "twine", "check", *wheel_paths)
    run(sys.executable, "-m", "twine", "upload", *wheel_paths)


if __name__ == "__main__":
    main()
