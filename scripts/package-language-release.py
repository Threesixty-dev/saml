#!/usr/bin/env python3
"""Bundle the language tools built for one release platform."""

import argparse
import tarfile
import zipfile
from pathlib import Path


PLATFORMS = {
    "macos-aarch64",
    "macos-x86_64",
    "linux-aarch64",
    "linux-x86_64",
    "windows-x86_64",
}
TOOLS = ("saml-lsp", "saml-check", "saml-map")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--platform", required=True, choices=sorted(PLATFORMS))
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    suffix = ".exe" if args.platform.startswith("windows-") else ""
    binaries = [args.input / f"{tool}{suffix}" for tool in TOOLS]
    for binary in binaries:
        if not binary.is_file():
            parser.error(f"missing binary: {binary}")

    args.output.mkdir(parents=True, exist_ok=True)
    stem = f"saml-language-v{args.version}-{args.platform}"
    if suffix:
        archive = args.output / f"{stem}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
            for binary in binaries:
                bundle.write(binary, binary.name)
    else:
        archive = args.output / f"{stem}.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            for binary in binaries:
                bundle.add(binary, arcname=binary.name)
    print(archive)


if __name__ == "__main__":
    main()
