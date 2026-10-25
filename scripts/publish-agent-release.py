#!/usr/bin/env python3
"""Build a signed SAML agent update repository from local release binaries."""

import argparse
import base64
import hashlib
import json
import shutil
import struct
import subprocess
import time
from pathlib import Path


def validate_binary(platform: str, data: bytes) -> None:
    os_name, arch = platform.split("/")
    if os_name == "linux":
        machines = {"x86_64": 62, "aarch64": 183}
        valid = (
            len(data) >= 20
            and data[:6] == b"\x7fELF\x02\x01"
            and struct.unpack_from("<H", data, 18)[0] == machines[arch]
        )
    elif os_name == "macos":
        cpu_types = {"x86_64": 0x01000007, "aarch64": 0x0100000C}
        valid = (
            len(data) >= 8
            and data[:4] == b"\xcf\xfa\xed\xfe"
            and struct.unpack_from("<I", data, 4)[0] == cpu_types[arch]
        )
    else:
        machines = {"x86_64": 0x8664, "aarch64": 0xAA64}
        pe_offset = struct.unpack_from("<I", data, 0x3C)[0] if len(data) >= 0x40 else 0
        valid = (
            data[:2] == b"MZ"
            and pe_offset + 6 <= len(data)
            and data[pe_offset : pe_offset + 4] == b"PE\0\0"
            and struct.unpack_from("<H", data, pe_offset + 4)[0] == machines[arch]
        )
    if not valid:
        raise ValueError(f"binary does not match {platform}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--key", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--target", action="append", required=True, metavar="OS/ARCH=BINARY"
    )
    args = parser.parse_args()
    if not args.version.split(".") or not all(
        part.isdigit() for part in args.version.split(".")
    ) or len(args.version.split(".")) != 3:
        parser.error("--version must be a stable major.minor.patch version")
    root = Path(__file__).resolve().parent.parent / "updates" / "root.pub"
    der = subprocess.check_output(
        ["openssl", "pkey", "-in", str(args.key), "-pubout", "-outform", "DER"]
    )
    if len(der) != 44 or der[-32:] != root.read_bytes():
        parser.error("signing key does not match the pinned update root")
    targets = []
    seen_platforms = set()
    for spec in args.target:
        platform, separator, filename = spec.partition("=")
        if not separator or platform not in (
            "macos/aarch64", "macos/x86_64", "linux/aarch64", "linux/x86_64",
            "windows/aarch64", "windows/x86_64",
        ):
            parser.error(f"invalid target specification: {spec}")
        if platform in seen_platforms:
            parser.error(f"duplicate target platform: {platform}")
        seen_platforms.add(platform)
        binary = Path(filename)
        data = binary.read_bytes()
        if not data or len(data) > 64 * 1024 * 1024:
            parser.error(f"target is empty or too large: {binary}")
        try:
            validate_binary(platform, data)
        except ValueError as error:
            parser.error(str(error))
        path = f"saml-agent/{platform}/{args.version}"
        destination = args.output / "targets" / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(binary, destination)
        targets.append(
            {
                "platform": platform,
                "path": path,
                "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    now = int(time.time())
    manifest = {
        "schema_version": 1,
        "version": args.version,
        "issued_unix_secs": now,
        "expires_unix_secs": now + 7 * 24 * 3600,
        "targets": targets,
    }
    metadata = args.output / "metadata"
    metadata.mkdir(parents=True, exist_ok=True)
    manifest_path = metadata / "release.json"
    manifest_path.write_text(json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n")
    signature = subprocess.check_output(
        [
            "openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(args.key),
            "-in", str(manifest_path),
        ]
    )
    (metadata / "release.sig").write_text(base64.b64encode(signature).decode() + "\n")
    print(f"Signed SAML agent {args.version} for {', '.join(item['platform'] for item in targets)}")


if __name__ == "__main__":
    main()
