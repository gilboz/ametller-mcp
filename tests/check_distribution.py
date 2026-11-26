#!/usr/bin/env python3
"""Test an unpacked plugin with empty temporary state and the real Rust binary."""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
EXPECTED_FILES = {
    'ametller/.codex-plugin/plugin.json', 'ametller/.mcp.json',
    'ametller/scripts/launch.sh', 'ametller/skills/setup/SKILL.md',
}
TOOLS = {'authentication_status', 'search_products', 'get_product', 'get_cart', 'set_cart_item',
         'sync_purchase_history', 'get_purchase_history', 'get_purchased_products'}


class RpcProcess:
    def __init__(self, command, cwd, env=None):
        self.errors = tempfile.TemporaryFile(mode='w+')
        self.child = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=self.errors, text=True)
        self.replies = queue.Queue()
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        try:
            for line in self.child.stdout:
                self.replies.put(json.loads(line))
        except Exception as error:
            self.replies.put(error)

    def call(self, message):
        self.child.stdin.write(json.dumps(message) + '\n')
        self.child.stdin.flush()
        if 'id' not in message:
            return None
        deadline = time.monotonic() + 20
        while True:
            reply = self.replies.get(timeout=max(0, deadline - time.monotonic()))
            if isinstance(reply, Exception):
                raise AssertionError('Unexpected non-JSON stdout') from reply
            if reply.get('id') == message['id']:
                assert 'error' not in reply, reply.get('error')
                return reply['result']

    def close(self, require_success=False):
        if self.child.stdin and not self.child.stdin.closed:
            self.child.stdin.close()
        try:
            code = self.child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.child.terminate()
            try:
                code = self.child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.child.kill()
                code = self.child.wait(timeout=5)
        self.reader.join(timeout=2)
        try:
            while not self.replies.empty():
                assert not isinstance(self.replies.get_nowait(), Exception), 'Unexpected stdout on exit'
            if require_success:
                assert code == 0, f'Server exit code {code}'
        finally:
            self.errors.close()


def smoke(plugin, config, binary, state, version, via_path=False):
    env = os.environ.copy()
    for key in ('AMETLLER_MCP_BIN', 'AMETLLER_MCP_STATE_DIR', 'AMETLLER_MCP_CREDENTIAL_STORE'):
        env.pop(key, None)
    env['AMETLLER_MCP_STATE_DIR'] = str(state)
    env['AMETLLER_MCP_CREDENTIAL_STORE'] = 'file'
    if via_path:
        env['PATH'] = str(binary.parent) + os.pathsep + os.defpath
    else:
        env['AMETLLER_MCP_BIN'] = str(binary)
    cwd = (plugin / config['cwd']).resolve(strict=True)
    assert cwd.is_relative_to(plugin.resolve())
    process = RpcProcess([config['command'], *config['args']], cwd, env)
    success = False
    try:
        result = process.call({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
            'protocolVersion': '2025-11-25', 'capabilities': {},
            'clientInfo': {'name': 'ametller-package-test', 'version': '1'}}})
        assert result['serverInfo']['name'] == 'ametller-mcp'
        assert result['serverInfo']['version'] == version
        process.call({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        listed = process.call({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list', 'params': {}})
        assert {tool['name'] for tool in listed['tools']} == TOOLS
        for tool in listed['tools']:
            assert tool['inputSchema']['type'] == 'object'
            assert isinstance(tool['annotations']['readOnlyHint'], bool)
        status = process.call({'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call', 'params': {
            'name': 'authentication_status', 'arguments': {}}})
        assert not status.get('isError', False), status
        # An account read with no saved session must fail promptly, not prompt or log in.
        cart = process.call({'jsonrpc': '2.0', 'id': 4, 'method': 'tools/call', 'params': {
            'name': 'get_cart', 'arguments': {}}})
        assert cart.get('isError') is True, cart
        assert 'login' in json.dumps(cart).lower()
        success = True
    finally:
        process.close(require_success=success)


def codex_discovery(codex, unpacked):
    catalog = unpacked / '.agents/plugins/marketplace.json'
    catalog.parent.mkdir(parents=True)
    catalog.write_text(json.dumps({'name': 'ametller-package-test', 'plugins': [{
        'name': 'ametller', 'source': {'source': 'local', 'path': './ametller'},
        'policy': {'installation': 'AVAILABLE', 'authentication': 'ON_INSTALL'},
        'category': 'Productivity'}]}))
    process = RpcProcess([str(codex), 'app-server', '--stdio'], unpacked)
    try:
        process.call({'id': 1, 'method': 'initialize', 'params': {
            'clientInfo': {'name': 'ametller-package-test', 'version': '1'},
            'capabilities': {'experimentalApi': True}}})
        process.call({'method': 'initialized', 'params': {}})
        result = process.call({'id': 2, 'method': 'plugin/read', 'params': {
            'marketplacePath': str(catalog), 'pluginName': 'ametller'}})
        assert result['plugin']['mcpServers'] == ['ametller']
        assert any(s['name'] == 'ametller:setup' for s in result['plugin']['skills'])
    finally:
        process.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--codex', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='ametller-distribution-') as temporary:
        root = Path(temporary)
        archive = root / 'plugin.zip'
        subprocess.run([sys.executable, str(ROOT / 'scripts/package_plugin.py'), '--output', str(archive)],
                       check=True, capture_output=True)
        with ZipFile(archive) as package:
            assert set(package.namelist()) == EXPECTED_FILES
            package.extractall(root / 'unpacked')
        plugin = root / 'unpacked/ametller'
        manifest = json.loads((plugin / '.codex-plugin/plugin.json').read_text())
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
        assert manifest['version'] == version
        path = (plugin / manifest['mcpServers']).resolve(strict=True)
        assert path.is_relative_to(plugin.resolve())
        config = json.loads(path.read_text())['mcpServers']['ametller']
        assert config['command'] == '/bin/sh'
        assert (plugin / config['cwd'] / config['args'][0]).resolve().is_relative_to(plugin.resolve())
        smoke(plugin, config, binary, root / 'explicit', version)
        smoke(plugin, config, binary, root / 'path', version, via_path=True)
        env = {**os.environ, 'AMETLLER_MCP_BIN': str(root / 'absent')}
        failed = subprocess.run([config['command'], *config['args']], cwd=plugin,
                                env=env, text=True, capture_output=True, timeout=10)
        assert failed.returncode == 126 and not failed.stdout
        assert 'executable' in failed.stderr
        if args.codex:
            codex_discovery(args.codex.resolve(strict=True), root / 'unpacked')
    print('Unpacked plugin: real stdio discovery, empty-session behavior and launcher checks passed.')
    if args.codex:
        print('Codex discovered the MCP server and setup skill without installation.')


if __name__ == '__main__':
    main()
