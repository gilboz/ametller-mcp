# Research sources

This directory separates acquired third-party material from our Rust source and documentation. Public browser bundles and reference checkouts are local research copies, ignored by Git. Do not publish an indiscriminate copy of this directory.

## Captures

- [GitHub manifest](github/manifest.json): repository URLs, exact commits and MIT license identification for three read-only reference checkouts. Each checkout retains its original LICENSE. No upstream programs or package scripts were executed.
- [Web manifest](web/manifest.json): source URLs, statuses, byte counts and SHA-256 hashes. Bundle `713` yielded 125 JavaScript assets totaling 6,575,321 bytes.
- [Formatted manifest](web/pretty-manifest.json): all 125 readable copies, line counts, output hashes and original hashes.
- [Formatter provenance](web/formatter.json): pinned Prettier 3.10.1 npm package and integrity. Only standalone formatter, Babel/ESTree parsers and license were extracted locally.
- [Catalog evidence](evidence/catalog-probe.json): live anonymous OAuth/search/detail statuses, with access/refresh tokens and shopper identifiers excluded. `search.json` and `product.json` contain public guest catalog responses.

## Capture coverage

The acquisition script reads the public storefront HTML, downloads same-origin Mobify script tags, parses the named Webpack chunk map, and follows statically visible dynamic-import IDs until no new chunks are discovered. Every requested JavaScript asset returned HTTP 200 and parsed successfully with Prettier. It never executes storefront JavaScript.

This includes browser route chunks that the homepage does not initially load. It does not claim access to the original source repository, TypeScript/JSX, server-only code, every possible runtime-generated asset URL, remote analytics code, or the Android app. The external Cloudflare beacon is listed in the manifest but not downloaded. Five `.js.map` requests returned HTTP 403; no further map-access attempt was made.

`raw/` preserves acquired bytes; `pretty/` is a derivative reading aid. The versioned config retains only API, locale, login and proxy fields; third-party analytics/maps/CAPTCHA site keys remain only in ignored raw captures. HTML was fetched anonymously, with no browser cookies or account session. Do not place future authenticated HARs, cookies, tokens, addresses, order history or payment information here; use private application storage outside the repository.

## Reproduction

Run the commands in the [root README](../README.md). Network captures intentionally refresh their manifests/evidence, so preserve a dated copy first when retaining historical comparisons. The Rust fixture under `tests/fixtures/` is an immutable copy of the initial public weighted-product response, independent of subsequent live probes.

To restore the reference checkouts, clone the repository URLs from `github/manifest.json` into their listed subdirectories and check out the recorded commits. Do not install their packages to read the source.

Retailer assets retain their original rights; the upstream MIT licenses apply to those independent implementations, not to Ametller's website. Any future source port must preserve the applicable notices. No upstream runtime source was copied into `src/` during this assessment.
