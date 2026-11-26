# Ametller MCP

A small local Rust MCP and CLI for Ametller Origen: search products, inspect names,
photos, descriptions and pack sizes, read your real cart, and set item quantities.
Sign in on the official website through an isolated window of your installed
Chrome, then use direct API calls for shopping. Direct CLI password login remains
available with `login --password`. There is no checkout/payment tool or background service.

Private repository: [gilboz/ametller-mcp](https://github.com/gilboz/ametller-mcp).
The current release is `0.1.0-beta.2`. See [verification](docs/verification.md) for
the checks actually performed; account login and real cart compatibility are
unverified until the account owner signs in and exercises them.

## Install

```sh
brew tap gilboz/ametller-mcp https://github.com/gilboz/ametller-mcp.git
brew install gilboz/ametller-mcp/ametller-mcp
codex plugin marketplace add gilboz/ametller-mcp --ref main
codex plugin add ametller@ametller-mcp
```

The repository is private, so GitHub access is required. Cargo and source archive
installation, plugin connection and removal are in [installation](docs/installation.md).
Nothing signs in or modifies your cart during installation.

## Use

Catalog browsing needs no account:

```sh
ametller-mcp search "yogur" --limit 5
ametller-mcp product 42171
```

For your real cart, run `ametller-mcp login` in your own terminal. Sign in on
Ametller's page in the Chrome window it opens; the helper captures the resulting
session locally and closes its temporary browser. No password or token is accepted
as a command argument or MCP tool parameter. On macOS, session credentials use
Keychain by default; an explicit file backend is available and uses private
permissions, not encryption. See [usage](docs/usage.md).

The MCP exposes five tools:

| Tool | Purpose |
| --- | --- |
| `authentication_status` | Report whether this local installation has saved account authentication. |
| `search_products` | Search the live guest catalog with official image URLs, pack sizes and price context. |
| `get_product` | Read a product's description, photos, availability and purchase-quantity rules. |
| `get_cart` | Read the registered account's cart; an absent cart stays absent. |
| `set_cart_item` | Set a selected product/line to an absolute quantity; zero removes it. |

Show proposed groceries before adding them unless the user has already asked for
those exact cart changes. Product suggestions do not imply authorization. A
returned cart is a reviewable estimate, not an order: delivery, final weighed
prices and payment are completed on the official website. Photo URLs are included;
a clickable photo-grid UI is not bundled and rendering depends on the MCP client.

## Development

Requires Rust 1.100+. Automated tests use temporary state and synthetic account
responses; they do not access Keychain or a live Ametller account.
The commands below apply to a Git checkout; the release `.crate` is the smaller
runtime source package described in the installation guide.

```sh
sh scripts/check.sh
cargo build --locked
python3 tests/check_distribution.py --binary target/debug/ametller-mcp
cargo package --locked
python3 scripts/package_plugin.py
```

The stdio server is `ametller-mcp serve`. Diagnostics go to stderr and stdout is
reserved for the MCP protocol. The MCP opens no HTTP listener; browser sign-in
temporarily uses a loopback-only Chrome debugging connection with a separate profile.

## Layout and research

```text
src/              Rust implementation
tests/            Mock protocol, cart, stdio and package checks
docs/             Usage, installation, architecture and research notes
plugins/ametller/  Codex plugin and setup skill
Formula/          Private Homebrew tap formula
sources/          Public research captures and pinned reference manifests
scripts/          Checks, plugin packaging and public research capture tools
```

The [assessment](docs/assessment.md) and [API map](docs/frontend-api.md) record the
original discovery. Downloaded storefront code and upstream checkouts stay local
and are excluded from Git/release packages. See [source provenance](sources/README.md)
and [architecture](docs/architecture.md). No other MCP repository is a runtime
dependency, and no neighboring project was modified.

Independent project; not affiliated with Ametller Origen. MIT licensed; see
[NOTICE](NOTICE) for reference-project and retailer-asset boundaries.
