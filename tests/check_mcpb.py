#!/usr/bin/env python3
"""Exercise the exact command in an unpacked Claude bundle with isolated state."""
import argparse
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import tomllib
from zipfile import ZipFile

from check_distribution import RpcProcess, TOOLS

ROOT = Path(__file__).resolve().parents[1]
FILES = {'manifest.json', 'server/ametller-mcp', 'login.command', 'README.md', 'LICENSE', 'NOTICE'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    with tempfile.TemporaryDirectory(prefix='ametller-claude-test-') as temporary:
        root = Path(temporary)
        bundle = root / 'ametller.mcpb'
        subprocess.run([sys.executable, str(ROOT / 'scripts/package_mcpb.py'), '--binary',
                        str(args.binary.resolve()), '--output', str(bundle)], check=True, capture_output=True)
        unpacked = root / 'Extension with spaces'
        with ZipFile(bundle) as archive:
            assert set(archive.namelist()) == FILES
            archive.extractall(unpacked)
            for member in archive.infolist():
                mode = member.external_attr >> 16
                assert stat.S_ISREG(mode)
                (unpacked / member.filename).chmod(stat.S_IMODE(mode))
        manifest = json.loads((unpacked / 'manifest.json').read_text())
        assert manifest['manifest_version'] == '0.3'
        assert manifest['version'] == version
        assert manifest['server']['type'] == 'binary'
        assert manifest['compatibility']['platforms'] == ['darwin']
        assert {t['name'] for t in manifest['tools']} == TOOLS
        assert set(manifest['user_config']) == {'locale'}
        assert all(not field.get('sensitive', False) for field in manifest['user_config'].values())
        config = manifest['server']['mcp_config']
        command = config['command'].replace('${__dirname}', str(unpacked))
        assert Path(command) == unpacked / manifest['server']['entry_point']
        arguments = [arg.replace('${user_config.locale}', 'es') for arg in config['args']]
        # Keep the same executable and arguments Claude uses, with only explicit
        # temporary test storage added so this test cannot read the real Keychain.
        state = root / 'private-state'
        process = RpcProcess([command, '--state-dir', str(state), '--credential-store', 'file',
                              *arguments], root, {**os.environ, 'PATH': os.defpath})
        success = False
        try:
            result = process.call({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
                'protocolVersion': '2025-11-25', 'capabilities': {},
                'clientInfo': {'name': 'claude-bundle-test', 'version': '1'}}})
            assert result['serverInfo']['version'] == version
            process.call({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
            listed = process.call({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list', 'params': {}})
            assert {tool['name'] for tool in listed['tools']} == TOOLS
            status = process.call({'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call',
                                   'params': {'name': 'authentication_status', 'arguments': {}}})
            assert status['structuredContent']['authenticated'] is False
            assert not state.exists(), 'Discovery/offline status must not create state.'
            cart = process.call({'jsonrpc': '2.0', 'id': 4, 'method': 'tools/call',
                                 'params': {'name': 'get_cart', 'arguments': {}}})
            assert cart['isError'] is True and 'login' in json.dumps(cart).lower()
            success = True
        finally:
            process.close(require_success=success)
        # The helper resolves its sibling binary even from an unrelated working
        # directory. No terminal means it exits before any credentials/network.
        login = subprocess.run([str(unpacked / 'login.command')], cwd=root,
                               stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=10)
        assert login.returncode == 1 and not login.stdout
        assert 'interactive terminal' in login.stderr
    print('Claude bundle: native binary, inventory, executable modes, version, login helper and stdio checks passed.')


if __name__ == '__main__':
    main()
