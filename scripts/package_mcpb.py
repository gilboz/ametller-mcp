#!/usr/bin/env python3
"""Build a credential-free Claude bundle containing the native macOS arm64 binary."""
import argparse
import json
from pathlib import Path
import stat
import struct
import subprocess
import tomllib
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / 'plugins/claude'
FILES = {
    'manifest.json': PLUGIN / 'manifest.json',
    'login.command': PLUGIN / 'login.command',
    'README.md': ROOT / 'docs/claude-desktop.md',
    'LICENSE': ROOT / 'LICENSE',
    'NOTICE': ROOT / 'NOTICE',
}


def check_binary(binary, version):
    if binary.is_symlink() or not binary.is_file() or not binary.stat().st_mode & 0o111:
        raise ValueError('Binary must be an executable regular file, not a symlink.')
    with binary.open('rb') as source:
        header = source.read(8)
    # Thin 64-bit little-endian Mach-O with CPU_TYPE_ARM64. Do not mislabel an
    # ELF, Intel or universal executable as this deliberately narrow package.
    if header != struct.pack('<II', 0xFEEDFACF, 0x0100000C):
        raise ValueError('This bundle requires a native macOS Apple Silicon (arm64) binary.')
    reported = subprocess.check_output([str(binary.resolve()), '--version'], text=True, timeout=10)
    if reported.strip() != f'ametller-mcp {version}':
        raise ValueError('Binary and manifest versions differ; rebuild before packaging.')


def build(binary, output):
    manifest = json.loads(FILES['manifest.json'].read_text())
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    if manifest['version'] != version:
        raise ValueError('Manifest and Cargo versions differ.')
    check_binary(binary, version)
    files = {**FILES, 'server/ametller-mcp': binary}
    for name, source in files.items():
        if source.is_symlink() or not source.is_file():
            raise ValueError(f'Unexpected package input: {name}')
    output.parent.mkdir(parents=True, exist_ok=True)
    with ZipFile(output, 'w', compression=ZIP_DEFLATED) as archive:
        for name, source in files.items():
            member = ZipInfo(name, date_time=(2026, 11, 26, 0, 0, 0))
            member.create_system = 3
            member.compress_type = ZIP_DEFLATED
            mode = 0o755 if name in ('server/ametller-mcp', 'login.command') else 0o644
            member.external_attr = (stat.S_IFREG | mode) << 16
            archive.writestr(member, source.read_bytes())
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/ametller-mcp')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    output = args.output or ROOT / 'target/packages' / f'ametller-{version}-macos-arm64.mcpb'
    try:
        print(build(args.binary, output))
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(str(error)) from error


if __name__ == '__main__':
    main()
