# Verification record

## CLI login correction — 26 November 2026

Version `0.1.0-beta.3` restores terminal login as the only flow and removes the
Chrome/CDP helper and its runtime dependencies. Existing browser installations
are left untouched. The `--password` flag remains a compatibility alias.

The reported post-capture HTTP 400 was traced to `locale` being added to the
customer profile request. The captured storefront SDK accepts only `siteId` for
customer/profile and customer/baskets query parameters. An anonymous PKCE probe
reproduced HTTP 400 with `locale`; the same guest profile request without it
returned HTTP 404, as a guest is not a persisted registered customer. This proves
the parameter rejection, not successful registered account access. No user
credentials were requested or used, and no account/cart mutation was performed.

The client now adds locale only to catalog and the supported checkout basket
operations. Customer and custom endpoints omit it. Account-check failures report
the stage and HTTP status, with no response body. Regression coverage exercises
the strict query contract, terminal-only default login, and preservation of saved
credentials and pending cart state if the post-login account check fails.

Local formatting, strict Clippy, all 30 Rust tests, release build, Homebrew
formula syntax and setup-skill validation pass. The unpacked plugin passes real
stdio discovery and Codex native plugin discovery. A live anonymous product
search also succeeds. Independent review found no correctness/security blockers;
one stale authentication tool description was corrected before release.

Registered CLI sign-in, real Keychain persistence and cart compatibility still
require a successful private login by the account owner.

## Browser login update — 26 November 2026

Version `0.1.0-beta.2` makes browser login the default; direct terminal login is
available with `login --password`. The user reported direct CLI failure despite
successful website login. The old error represented every unexpected HTTP
response and did not establish either incorrect credentials or a CAPTCHA.
Direct login also now sends the documented UUIDv4 shopper identifier.

An independent fresh reviewer checked token URL/event boundaries, guest filtering,
response limits, account validation before saving, cleanup and CLI integration,
with no actionable P1/P2 findings. All 38 normal tests pass. The explicitly
selected installed-Chrome about:blank smoke also passed on this Mac: launch,
loopback CDP connection and browser/profile cleanup. Normal tests do not open a
browser. Actual user sign-in, real Keychain persistence and session refresh still
need a successful user login; no account session was captured during these tests.

Homebrew `0.1.0-beta.1` installation and `brew test` passed on 26 November, along
with installed-plugin discovery and guest product search. The records below
describe the initial release before that installation.

## Initial Rust beta implementation — 25 November 2026

The final implementation passes formatting, Clippy with warnings denied, and
26 Rust tests (22 unit, two captured-contract and two subprocess tests).
Coverage includes synthetic registered login, rotating refresh credentials,
lost cart replies, absolute quantity/read-back behavior, reservation exclusions,
discounted prices, private files, partial Keychain cleanup states, account
identity mismatches and real subprocess MCP stdio. Three fresh review/fix
iterations resolved seven material findings; see [the review record](reviews.md).

The built Rust CLI also passed live anonymous product search and product detail
for product `42171`, returning the official image URL, description and purchase
unit metadata. The live authorization response omitted requested OAuth `state`;
the client permits that only for the direct response to its own request, with
pinned callback validation and PKCE. A supplied mismatched state remains rejected.

The unpacked plugin passed real stdio discovery and empty-session behavior through
its launcher. The installed Codex CLI also parsed the package natively and
discovered the MCP connection and setup skill without installing the plugin.
The setup skill validator passed; the Homebrew formula passed Ruby syntax checks.
`cargo package` built the extracted standard `.crate` successfully, and an
isolated `cargo install --path` from that extracted package produced a working
executable. The unpacked plugin also passed stdio tests with that installed
binary. Package inventories exclude research checkouts, storefront bundles,
private state and credentials. Cargo, plugin and formula versions agree.

The Homebrew formula uses the installed Homebrew API's locked dependency-fetch
and offline-install helpers. Actual Homebrew installation was not performed;
Cargo installation was verified without changing the user's normal installation.
The plugin was parsed/tested but not installed into the user's Codex settings.

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
