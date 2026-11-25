#!/usr/bin/env python3
"""Fetch a pinned, project-local formatter, verifying npm's package integrity.

No npm install, install hooks, browser or global tooling changes.
"""
import base64
import hashlib
import io
import json
from pathlib import Path
import tarfile
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[1]
VERSION = '3.10.1'
with urlopen(f'https://registry.npmjs.org/prettier/{VERSION}', timeout=30) as response:
    metadata = json.load(response)
with urlopen(metadata['dist']['tarball'], timeout=30) as response:
    body = response.read()
actual = 'sha512-' + base64.b64encode(hashlib.sha512(body).digest()).decode()
if actual != metadata['dist']['integrity']:
    raise SystemExit('Prettier package integrity mismatch')
destination = ROOT / 'sources/tooling/prettier'
destination.mkdir(parents=True, exist_ok=True)
with tarfile.open(fileobj=io.BytesIO(body), mode='r:gz') as archive:
    for name in ['standalone.mjs', 'plugins/babel.mjs', 'plugins/estree.mjs', 'LICENSE']:
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(archive.extractfile('package/' + name).read())
(ROOT / 'sources/web/formatter.json').write_text(json.dumps({
    'package': 'prettier', 'version': VERSION, 'tarball': metadata['dist']['tarball'],
    'integrity': actual, 'scope': 'local research formatting only; no runtime dependency'}, indent=2) + '\n')
print(f'Prepared project-local Prettier {VERSION}')
