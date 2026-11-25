# Architecture

The runtime is a standalone Rust CLI and stdio MCP. It uses `rmcp`, Tokio,
`reqwest` with Rustls, and exact decimal arithmetic. It has no dependency on the
reference Python/JavaScript applications or another local MCP repository.

```text
CLI / MCP -> service -> catalog and cart operations -> pinned SCAPI proxy
                |                                       |
          private state store <--- SLAS login/refresh ---+
```

## Modules

| Module | Responsibility |
| --- | --- |
| `main` | Commands, hidden terminal login prompt and JSON output. |
| `mcp` | Five tool schemas, annotations, result handling and stdio transport. |
| `service` | Guest/account workflows and normalized product output. |
| `api` | Public configuration discovery, fixed HTTPS origins, parameter validation, request timeouts and bounded bodies. |
| `auth` | Guest and registered PKCE, account validation and refresh rotation. |
| `store` | Keychain/file credentials, private atomic state files and cross-process lock. |
| `cart` | Active basket discovery, exact quantity validation, mutations and uncertain-outcome reconciliation. |
| `error` | Bounded, non-secret errors; raw HTTP responses are not exposed. |

## Authentication

The CLI prompts privately for email/password and performs direct SLAS B2C login.
It consumes the authorization redirect without following it or opening a browser.
MCP has no password parameter or interactive login prompt. A real browser challenge
requires a future fallback; no challenge bypass is attempted.

Guest catalog tokens remain in RAM. Registered access/refresh tokens use Keychain
on macOS by default, or a private plaintext file backend when selected. Private
state lives outside source and package directories. A process lock covers token
refresh and the complete account/cart operation. Replacement refresh credentials
are saved before dependent follow-up requests, so a transient profile read does
not discard a successfully rotated refresh token.

Offline status reports saved metadata with `verification: not_checked`; it does
not claim current remote authentication. Live status and account operations
validate the registered customer through Ametller. No token values appear in MCP
output, errors or diagnostics. Logout removes local credentials, not the remote
account session, and retains unresolved cart-write evidence.

## Catalog and cart semantics

Search and detail use the live guest catalog. Prices are estimates, availability
is scoped to the catalog, and missing values remain unknown. Product output
includes official image URLs, descriptions stripped of markup, canonical links,
pack/unit information and min/step quantities. This is structured data; rich image
rendering depends on the host, and a custom MCP App is outside this release.

`get_cart` never creates a basket. `set_cart_item` sets the desired absolute
purchase-unit quantity; zero removes a known line. IDs are validated separately
from quantities. Decimal validation avoids float rounding errors. Unsupported
bonus, option and reservation behavior is rejected rather than guessed.

Before a mutation, the store persists intent and the target account/basket. A
lost response is not replayed automatically. The server reads back the result;
unresolved outcomes block later writes until a read observes the intended state
or the user explicitly acknowledges reconciliation through the local CLI.

There is no checkout, order submission, payment, delivery reservation, coupon
activation, address mutation or automatic substitution. Explicitly requested cart
changes use the host's normal tool controls. Suggestions are previewed before
adding. A model-supplied confirmation flag is not used as an authorization gate.

## Shared conventions and packaging

The neighboring Sanitas/Tuta projects informed dependency versions, local CLI
conventions and stdio regression checks. No shared application dependency or new
workspace was introduced. No GPL Tuta implementation is included in this crate.

`cargo package` produces a standard private `.crate` source archive. The Homebrew
formula builds the tagged source with locked dependencies. The Codex plugin
contains only a manifest, stdio launcher and setup skill; it locates the separately
installed binary. The release excludes storefront bundles, upstream checkouts,
account state and credentials.

See [usage](usage.md), [installation](installation.md), [verification](verification.md)
and [the original API map](frontend-api.md).
