"""Package a compiled binary and public setup files for a GitHub release."""

import argparse
from pathlib import Path
import re
import tarfile
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "x86_64-unknown-linux-gnu": "vaporator",
    "x86_64-pc-windows-msvc": "vaporator.exe",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))[
        "package"
    ]["version"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        parser.error("Release packaging requires a stable major.minor.patch version")

    name = f"vaporator-v{version}-{args.target}"
    binary = ROOT / "target" / args.target / "release" / TARGETS[args.target]
    # Only these public files may enter the archive. Never copy a working data directory.
    files = {
        TARGETS[args.target]: binary,
        "README.md": ROOT / "README.md",
        "docs/OPERATIONS.md": ROOT / "docs" / "OPERATIONS.md",
        "docs/PLAN.md": ROOT / "docs" / "PLAN.md",
        ".env.example": ROOT / ".env.example",
    }
    for path in files.values():
        if not path.is_file():
            parser.error(f"Missing release file: {path}")

    output = ROOT / "target" / "dist"
    output.mkdir(parents=True, exist_ok=True)
    if args.target.endswith("windows-msvc"):
        archive = output / f"{name}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as package:
            for relative, path in files.items():
                package.write(path, f"{name}/{relative}")
    else:
        archive = output / f"{name}.tar.gz"
        with tarfile.open(archive, "w:gz") as package:
            for relative, path in files.items():
                info = package.gettarinfo(path, arcname=f"{name}/{relative}")
                info.mode = 0o755 if path == binary else 0o644
                with path.open("rb") as source:
                    package.addfile(info, source)
    print(archive.name)


if __name__ == "__main__":
    main()
