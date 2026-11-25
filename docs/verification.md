# Verification record — 25 November 2026

## Rust beta implementation

Initial implementation checks passed before review: formatting, Clippy with
warnings denied, and 15 Rust tests including synthetic registered login, rotating
refresh credentials, lost cart replies, absolute quantity/read-back behavior,
private files and real subprocess MCP stdio. Three fresh review/fix iterations
are in progress; final evidence is recorded here before publication.

The built Rust CLI also passed live anonymous product search and product detail
for product `42171`, returning the official image URL, description and purchase
unit metadata. The live authorization response omitted requested OAuth `state`;
the client permits that only for the direct response to its own request, with
pinned callback validation and PKCE. A supplied mismatched state remains rejected.

The unpacked plugin passed real stdio discovery and empty-session behavior through
its launcher. The installed Codex CLI also parsed the package natively and
discovered the MCP connection and setup skill without installing the plugin.
The setup skill validator passed; the Homebrew formula passed Ruby syntax checks.

## Initial research checks

- Anonymous storefront retrieval: HTTP 200, current bundle `713`.
- Public guest OAuth: authorize 303, token 200; tokens kept in RAM.
- Product search: HTTP 200 from direct SCAPI and the storefront proxy.
- Product detail: HTTP 200, with name, images, description, price, catalog availability and purchase quantity metadata.
- 125 JavaScript downloads and all 125 Prettier parses/formatted outputs.
- SHA-256 verification for every captured asset and formatted file.
- All local links in authored project documentation resolve.
- Python research scripts compile successfully.
- Capture/evidence/fixture scan found no literal JWT-shaped tokens.
- `cargo fmt --check`.
- `cargo clippy --locked --all-targets -- -D warnings`.
- `cargo test --locked --all-targets`: two passing contract tests (weighted-product purchase units and unknown inventory).

The contract tests use a fixed public fixture under `tests/fixtures/`; refreshing
live research evidence does not change that fixture. The initial data-contract
foundation has been extended into the Rust CLI and five-tool MCP described above.

## Observed limitations

Five public source-map probes returned HTTP 403. Readable output therefore retains bundle modules and minified names; no original source-map tree was reconstructed.

No live registered login, token refresh, cart mutation, delivery selection,
client-side photo rendering, checkout, payment, order submission or Android
integration was tested. Registered-session/cart tests use synthetic responses.
No account credentials or browser session were accessed. Existing projects'
live-test claims remain upstream claims, separate from this record.

See [catalog-probe.json](../sources/evidence/catalog-probe.json) for live statuses and [manifest.json](../sources/web/manifest.json) for individual asset requests.
