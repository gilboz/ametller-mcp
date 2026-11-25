#!/usr/bin/env python3
"""Package only the plugin's explicitly listed, credential-free files."""
import argparse
import json
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / 'plugins/ametller'
FILES = ('.codex-plugin/plugin.json', '.mcp.json', 'scripts/launch.sh', 'skills/setup/SKILL.md')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    manifest = json.loads((PLUGIN / FILES[0]).read_text())
    destination = args.output or ROOT / 'target/packages' / f'ametller-plugin-{manifest["version"]}.zip'
    destination.parent.mkdir(parents=True, exist_ok=True)
    for name in FILES:
        source = PLUGIN / name
        if source.is_symlink() or not source.is_file() or not source.resolve().is_relative_to(PLUGIN.resolve()):
            raise SystemExit(f'Unexpected plugin input: {name}')
    with ZipFile(destination, 'w', compression=ZIP_DEFLATED) as archive:
        for name in FILES:
            member = ZipInfo('ametller/' + name, date_time=(2026, 11, 25, 0, 0, 0))
            member.compress_type = ZIP_DEFLATED
            member.external_attr = (0o100755 if name.endswith('.sh') else 0o100644) << 16
            archive.writestr(member, (PLUGIN / name).read_bytes())
    print(destination)


if __name__ == '__main__':
    main()
