#!/usr/bin/env python3
"""Capture the public browser bundle graph; never execute storefront code."""
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
from datetime import datetime, timezone
from urllib.parse import urljoin, urlparse
from urllib.request import Request, urlopen
from urllib.error import HTTPError

ROOT = Path(__file__).resolve().parents[1]
RAW = ROOT / 'sources/web/raw'
ORIGIN = 'https://ametllerorigen.com'


class Scripts(HTMLParser):
    def __init__(self):
        super().__init__()
        self.urls, self.inline, self.active = [], {}, None

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == 'script':
            self.active = a.get('id')
            if a.get('src'):
                self.urls.append(urljoin(ORIGIN, a['src']))

    def handle_data(self, data):
        if self.active:
            self.inline[self.active] = self.inline.get(self.active, '') + data

    def handle_endtag(self, tag):
        if tag == 'script':
            self.active = None


def main():
    RAW.mkdir(parents=True, exist_ok=True)
    records = []

    def fetch(url, filename):
        try:
            with urlopen(Request(url, headers={'User-Agent': 'ametller-mcp-research/0.1'}), timeout=30) as response:
                body = response.read(20_000_001)
                if len(body) > 20_000_000:
                    raise ValueError('asset exceeds capture bound')
                status = response.status
                kind = response.headers.get('Content-Type', '')
            if filename.endswith(('.js', '.map')) and 'html' in kind:
                records.append({'url': url, 'status': status, 'error': 'HTML instead of asset'})
                return None
            (RAW / filename).write_bytes(body)
            records.append({'url': url, 'status': status, 'path': 'raw/' + filename,
                            'bytes': len(body), 'sha256': hashlib.sha256(body).hexdigest()})
            return body
        except HTTPError as error:
            records.append({'url': url, 'status': error.code})
            return None

    html = fetch(ORIGIN + '/es/online', 'storefront.html')
    if html is None:
        raise SystemExit('Storefront unavailable')
    parser = Scripts()
    parser.feed(html.decode())
    data = json.loads(parser.inline['mobify-data'])
    # Retain only the API, locale and login contract in the versioned snapshot.
    # The original public page/config remain in ignored raw captures; third-party
    # maps, analytics and CAPTCHA site keys are unnecessary in our repository.
    app = data['__CONFIG__']['app']
    config = {'app': {key: app[key] for key in ('commerceAPI', 'sites', 'login')},
              'ssrParameters': {'proxyConfigs': data['__CONFIG__']['ssrParameters']['proxyConfigs']}}
    (ROOT / 'sources/web/config.json').write_text(json.dumps(config, indent=2) + '\n')
    (RAW / 'mobify-data.json').write_text(json.dumps(data, indent=2) + '\n')
    urls = [u for u in parser.urls if urlparse(u).hostname == urlparse(ORIGIN).hostname
            and '/mobify/bundle/' in u]
    main_url = next(u for u in urls if u.endswith('/main.js'))
    base = main_url.rsplit('/', 1)[0] + '/'
    main_body = fetch(main_url, 'main.js').decode()
    # Webpack's statically visible chunk-name table. Do not eval JavaScript.
    mapping = re.search(r'\.u=\w+=>\(\(\{(.*?)\}\[\w+\]\|\|\w+\)\+"\.js"\)', main_body)
    if mapping is None:
        raise SystemExit('Webpack runtime shape changed; review chunk discovery')
    names = dict(re.findall(r'(\d+):"([^"/]+)"', mapping.group(1)))
    queue = set(urls + [base + name + '.js' for name in names.values()])
    seen, texts = {main_url}, {'main.js': main_body}
    for _ in range(250):
        # Explicit dynamic import IDs and loadable chunk arrays.
        for body in list(texts.values()):
            ids = set(re.findall(r'\.e\((\d+)\)', body))
            for group in re.findall(r'chunks:\[([\d, ]+)\]', body):
                ids.update(re.findall(r'\d+', group))
            queue.update(base + names.get(i, i) + '.js' for i in ids)
        pending = sorted(queue - seen)
        if not pending:
            break
        url = pending[0]
        seen.add(url)
        filename = url.rsplit('/', 1)[1]
        body = fetch(url, filename)
        if body is not None:
            texts[filename] = body.decode()
    else:
        raise SystemExit('Chunk bound reached; review capture')

    maps = set()
    for filename, body in texts.items():
        for relative in re.findall(r'[#@] sourceMappingURL=([^\s]+)', body):
            if re.fullmatch(r'[\w.-]+\.map', relative):
                maps.add(relative)
    # A few explicit public-map probes, even when no mapping comment is emitted.
    maps.update(['main.js.map', 'sdk.js.map', 'pages-login.js.map',
                 'ametller-pages-cart.js.map', 'pages-product-detail.js.map'])
    for filename in sorted(maps):
        fetch(base + filename, filename)
    manifest = {'captured_at': datetime.now(timezone.utc).isoformat(), 'entry_url': ORIGIN + '/es/online',
                'bundle_base': base, 'javascript_count': len(texts),
                'scope': 'All scripts reachable by HTML and static Webpack chunk tables/import IDs; no server code or dynamically constructed URLs.',
                'excluded_external_scripts': [u for u in parser.urls if u not in urls], 'assets': records}
    (ROOT / 'sources/web/manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({'javascript_count': len(texts), 'bytes': sum(len(t.encode()) for t in texts.values()),
                      'failed': [r for r in records if r.get('status') != 200 or r.get('error')]}))


if __name__ == '__main__':
    main()
