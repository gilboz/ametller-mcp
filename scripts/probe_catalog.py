#!/usr/bin/env python3
"""Small live guest-only SCAPI probe. Tokens stay in RAM; no cart writes."""
import base64
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import secrets
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, quote, urlencode, urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener

ROOT = Path(__file__).resolve().parents[1]


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def main():
    config = json.loads((ROOT / 'sources/web/config.json').read_text())['app']['commerceAPI']['parameters']
    base = f"https://{config['shortCode']}.api.commercecloud.salesforce.com"
    auth = f"{base}/shopper/auth/v1/organizations/{config['organizationId']}/oauth2"
    redirect = 'https://www.ametllerorigen.com/callback'
    opener = build_opener(NoRedirect)
    evidence = {'tested_at': datetime.now(timezone.utc).isoformat(), 'account': 'anonymous guest',
                'writes': 'guest OAuth token issuance only; no basket, order or payment calls', 'steps': []}

    def request(label, url, form=None, token=None):
        headers = {'User-Agent': 'ametller-mcp-research/0.1', 'Accept': 'application/json'}
        if form:
            headers['Content-Type'] = 'application/x-www-form-urlencoded'
        if token:
            headers['Authorization'] = 'Bearer ' + token
        req = Request(url, data=urlencode(form).encode() if form else None, headers=headers)
        try:
            with opener.open(req, timeout=30) as res:
                evidence['steps'].append({'operation': label, 'status': res.status})
                return json.load(res)
        except HTTPError as error:
            evidence['steps'].append({'operation': label, 'status': error.code})
            if error.code in (302, 303):
                location = error.headers['Location']
                error.close()
                return location
            error.close()
            raise RuntimeError(f'{label}: HTTP {error.code}') from None

    try:
        verifier = secrets.token_urlsafe(48)
        challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('=')
        location = request('guest_authorize', auth + '/authorize?' + urlencode({
            'hint': 'guest', 'response_type': 'code', 'client_id': config['clientId'],
            'redirect_uri': redirect, 'code_challenge': challenge, 'channel_id': config['siteId']}))
        parsed, expected = urlparse(location), urlparse(redirect)
        if (parsed.scheme, parsed.netloc, parsed.path) != (expected.scheme, expected.netloc, expected.path):
            raise RuntimeError('Unexpected callback origin/path')
        code = parse_qs(parsed.query)['code'][0]
        session = request('guest_token', auth + '/token', form={
            'grant_type': 'authorization_code_pkce', 'code': code, 'code_verifier': verifier,
            'client_id': config['clientId'], 'redirect_uri': redirect, 'channel_id': config['siteId']})
        token = session['access_token']
        evidence['access_token_expires_in_seconds'] = session.get('expires_in')
        common = {'siteId': config['siteId'], 'locale': 'es'}
        path = f"/search/shopper-search/v1/organizations/{config['organizationId']}/product-search"
        search = request('search_direct', base + path + '?' + urlencode({**common, 'q': 'platano', 'limit': 3, 'expand': 'images,prices'}), token=token)
        (ROOT / 'sources/evidence/search.json').write_text(json.dumps(search, indent=2, ensure_ascii=False) + '\n')
        evidence['search_total'] = search.get('total')
        evidence['returned_products'] = [{'id': p['productId'], 'name': p['productName'], 'price': p.get('price'),
                                          'image_url': p.get('image', {}).get('disBaseLink') or p.get('image', {}).get('link')}
                                         for p in search.get('hits', [])]
        product_id = search['hits'][0]['productId']
        path_detail = f"/product/shopper-products/v1/organizations/{config['organizationId']}/products/{quote(product_id, safe='')}"
        detail = request('product_detail_direct', base + path_detail + '?' + urlencode({**common, 'allImages': 'true', 'expand': 'images,prices,availability,promotions'}), token=token)
        (ROOT / 'sources/evidence/product.json').write_text(json.dumps(detail, indent=2, ensure_ascii=False) + '\n')
        evidence['detail_keys'] = sorted(detail)
        proxy = request('search_storefront_proxy', 'https://ametllerorigen.com/mobify/proxy/api' + path + '?' + urlencode({**common, 'q': 'platano', 'limit': 1}), token=token)
        evidence['proxy_search_total'] = proxy.get('total')
        evidence['result'] = 'passed'
    except (RuntimeError, KeyError, ValueError, TypeError, IndexError, URLError, OSError) as error:
        evidence['result'] = 'failed'
        # Exception bodies and callback URLs are intentionally never persisted.
        evidence['error_type'] = type(error).__name__
    finally:
        (ROOT / 'sources/evidence/catalog-probe.json').write_text(json.dumps(evidence, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps(evidence, indent=2, ensure_ascii=False))
    if evidence.get('result') != 'passed':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
