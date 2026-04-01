from __future__ import annotations

import email.message
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from setuptools import setup
from setuptools.command.bdist_wheel import bdist_wheel as _bdist_wheel
from setuptools.command.bdist_wheel import safe_version, safer_name
from wheel.wheelfile import WheelFile


ROOT = Path(__file__).resolve().parent
PACKAGE_NAME = "sbrun"
SUMMARY = "Run commands under the macOS sandbox with writes confined to the working tree"


def package_version() -> str:
    namespace: dict[str, str] = {}
    exec((ROOT / "src/sbrun/__init__.py").read_text(encoding="utf-8"), namespace)
    return namespace["__version__"]


def deployment_target() -> str:
    return os.environ.get("MACOSX_DEPLOYMENT_TARGET", "13.0")


def wheel_platform_tag() -> str:
    parts = deployment_target().split(".", 1)
    major = parts[0]
    minor = parts[1] if len(parts) > 1 else "0"
    return f"macosx_{major}_{minor}_arm64"


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


class bdist_wheel(_bdist_wheel):
    def finalize_options(self) -> None:
        super().finalize_options()
        self.root_is_pure = False

    def get_tag(self) -> tuple[str, str, str]:
        return "py3", "none", wheel_platform_tag()

    def run(self) -> None:
        subprocess.run(["make"], cwd=ROOT, check=True)
        self.mkpath(self.dist_dir)

        name = safer_name(PACKAGE_NAME)
        version = safe_version(package_version())
        impl_tag, abi_tag, plat_tag = self.get_tag()
        dist_info = f"{name}-{version}.dist-info"
        data_scripts = f"{name}-{version}.data/scripts"
        wheel_name = f"{name}-{version}-{impl_tag}-{abi_tag}-{plat_tag}.whl"
        wheel_path = Path(self.dist_dir) / wheel_name

        metadata = email.message.Message()
        metadata["Metadata-Version"] = "2.1"
        metadata["Name"] = PACKAGE_NAME
        metadata["Version"] = package_version()
        metadata["Summary"] = SUMMARY
        metadata["Requires-Python"] = ">=3.9"
        metadata["Description-Content-Type"] = "text/markdown"
        metadata.set_payload((ROOT / "README.md").read_text(encoding="utf-8"))

        wheel_text = (
            "Wheel-Version: 1.0\n"
            "Generator: sbrun custom bdist_wheel\n"
            "Root-Is-Purelib: false\n"
            f"Tag: {impl_tag}-{abi_tag}-{plat_tag}\n"
        )

        with tempfile.TemporaryDirectory(prefix="sbrun-wheel.") as td:
            tree = Path(td)
            write_text(tree / "sbrun" / "__init__.py", (ROOT / "src/sbrun/__init__.py").read_text(encoding="utf-8"))
            script_path = tree / data_scripts / "sbrun"
            script_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / "sbrun", script_path)
            os.chmod(script_path, 0o755)
            write_text(tree / dist_info / "METADATA", metadata.as_string())
            write_text(tree / dist_info / "WHEEL", wheel_text)
            write_text(tree / dist_info / "top_level.txt", "sbrun\n")

            with WheelFile(str(wheel_path), "w") as wheel:
                wheel.write_files(str(tree))

        self.distribution.dist_files.append(("bdist_wheel", "", str(wheel_path)))


setup(
    name=PACKAGE_NAME,
    version=package_version(),
    description=SUMMARY,
    long_description=(ROOT / "README.md").read_text(encoding="utf-8"),
    long_description_content_type="text/markdown",
    packages=["sbrun"],
    package_dir={"": "src"},
    python_requires=">=3.9",
    cmdclass={"bdist_wheel": bdist_wheel},
)
