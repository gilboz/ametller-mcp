# Ametller MCP

A small local Rust MCP and CLI for Ametller Origen: search products, inspect names,
photos, descriptions and pack sizes, read your real cart, and set item quantities.
Sign in privately through a terminal prompt, then use direct API calls for
shopping. No browser installation or automation is required. There is no
checkout/payment tool or background service.

Repository: [gilboz/ametller-mcp](https://github.com/gilboz/ametller-mcp).
The current development version is `0.1.0-beta.6`. Product tools return embedded image content
alongside structured product data and original photo URLs. See
[verification](docs/verification.md) for the checks performed and remaining limits.

## Install

### Claude desktop app

On an **Apple Silicon Mac** (M1 or later):

1. Download `ametller-0.1.0-beta.6-macos-arm64.mcpb` from the
   [release](https://github.com/gilboz/ametller-mcp/releases/tag/v0.1.0-beta.6).
2. Double-click it and choose **Install** in Claude. Alternatively, use **Settings
   → Desktop app → Extensions → Manage → Advanced settings → Install Extension…**
   (older versions: **Settings → Extensions**).
3. Leave the language as `es`, or enter `ca`, and enable Ametller.
4. Ask Claude: “Show me Greek yogurts with photos and prices.”

The bundle includes the Rust executable and runs without Homebrew, Node or Python.
Your existing default Ametller CLI/Keychain login on this Mac is reused; guest
browsing works without login. For a fresh installation, terminal sign-in, updates,
and platform limitations, see [Claude desktop setup](docs/claude-desktop.md).
This is a local desktop extension; it does not require cloud hosting.

### Homebrew and Codex

```sh
brew tap gilboz/ametller-mcp https://github.com/gilboz/ametller-mcp.git
brew install gilboz/ametller-mcp/ametller-mcp
codex plugin marketplace add gilboz/ametller-mcp --ref main
codex plugin add ametller@ametller-mcp
```

If repository access is restricted, sign into GitHub before downloading. Cargo and source archive
installation, plugin connection and removal are in [installation](docs/installation.md).
Nothing signs in or modifies your cart during installation.

## Use

Catalog browsing needs no account:

```sh
ametller-mcp search "yogur" --limit 5
ametller-mcp product 42171
```

For your real cart, run `ametller-mcp login` in your own terminal. Enter your
email and password at the prompts; the password is hidden and is not saved.
The client signs in directly over HTTPS and verifies the account before saving
session tokens. No password or token is accepted as a command argument or MCP
tool parameter. On macOS, session credentials use
Keychain by default; an explicit file backend is available and uses private
permissions, not encryption. See [usage](docs/usage.md).

The MCP exposes eight tools:

| Tool | Purpose |
| --- | --- |
| `authentication_status` | Report whether this local installation has saved account authentication. |
| `search_products` | Search the live guest catalog with embedded photos, pack sizes and price context. |
| `get_product` | Read a product's description, photos, availability and purchase-quantity rules. |
| `sync_purchase_history` | Refresh online order history into the private local SQLite cache; resume incomplete batches. |
| `get_purchase_history` | Read cached orders and their product lines without backend requests. |
| `get_purchased_products` | Find recent or frequent previously ordered products for repeat-shopping suggestions. |
| `get_cart` | Read the registered account's cart; an absent cart stays absent. |
| `set_cart_item` | Set a selected product/line to an absolute quantity; zero removes it. |

Show proposed groceries before adding them unless the user has already asked for
those exact cart changes. Product suggestions do not imply authorization. A
returned cart is a reviewable estimate, not an order: delivery, final weighed
prices and payment are completed on the official website. Standard MCP image
blocks carry the actual photo bytes; the host decides how to present them. A
custom shopping carousel is not bundled. CLI commands continue to return JSON.

## Purchase history and repeat shopping

After the private login, ask the agent in chat:

- “Refresh my Ametller order history.”
- “Show my recent orders.”
- “Which yogurts have I bought most often?”

The agent calls `sync_purchase_history`, `get_purchase_history`, and
`get_purchased_products`. You do not need to run history commands in a terminal.
The sync tool fetches online orders with read-only requests and upserts them into
an account-specific SQLite database. Ordinary history reads use only that
local database and include freshness/completeness information. The agent resumes
bounded batches when needed. Terminal commands remain an optional developer
interface; only private login is part of the normal user workflow.

This covers **online orders available through Ametller's API**. In-store receipt
emails are a separate source and are not imported here. Historical prices and
quantities do not establish today's price, stock or refill needs. Use current
product details before applying user-selected cart changes.

The cache stores normalized product lines and order summaries, not raw checkout
payloads, addresses, payment data or tokens. SQLite files use owner-only access;
they are not database-encrypted. Logout retains this purchase log. Run
`ametller-mcp clear-history` while connected to explicitly delete that account's
local history without changing remote orders. See [history details](docs/history.md).

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
# On an Apple Silicon Mac:
cargo build --release --locked
python3 scripts/package_mcpb.py --binary target/release/ametller-mcp
python3 tests/check_mcpb.py --binary target/release/ametller-mcp
```

The stdio server is `ametller-mcp serve`. Diagnostics go to stderr and stdout is
reserved for the MCP protocol. The MCP opens no HTTP listener or browser.

## Layout and research

```text
src/              Rust implementation
tests/            Mock protocol, cart, stdio and package checks
docs/             Usage, installation, architecture and research notes
plugins/ametller/  Codex plugin and setup skill
plugins/claude/    Claude desktop bundle manifest and terminal login helper
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
