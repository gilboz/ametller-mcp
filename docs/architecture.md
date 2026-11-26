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
| `mcp` | Eight tool schemas, annotations, result handling and stdio transport. |
| `service` | Guest/account workflows and normalized product output. |
| `api` | Public configuration discovery, fixed HTTPS origins, parameter validation, request timeouts and bounded bodies. |
| `auth` | Guest and registered PKCE, account validation and refresh rotation. |
| `history` | Read-only online order sync, account-scoped SQLite and offline purchase queries. |
| `photos` | Public-only image downloads, bounded decoding/thumbnails, and MCP image blocks. |
| `store` | Keychain/file credentials, private atomic state files and cross-process lock. |
| `cart` | Active basket discovery, exact quantity validation, mutations and uncertain-outcome reconciliation. |
| `error` | Bounded, non-secret errors; raw HTTP responses are not exposed. |

## Authentication

`login` uses direct SLAS B2C login with terminal email and hidden password prompts,
UUIDv4 USID and PKCE over HTTPS. It consumes the pinned redirect without opening
its URL. No browser automation, debugging socket or extension is included.
`login --password` remains a compatibility alias. The registered account is
validated before a new login replaces saved credentials.

Unexpected sign-in responses report the HTTP status with a fixed message, never
a raw response body or an unsupported inference that a CAPTCHA is present.
Account-check HTTP errors identify the verification stage separately. MCP has no
password/token argument or interactive login tool; sign-in remains a local CLI
operation. Interactive website challenges are not supported by this flow.

SCAPI query parameters follow each service contract: `siteId` is always supplied;
`locale` is supplied for catalog and the supported basket operations, and omitted
for customer and custom endpoints. The customer API rejects `locale` with HTTP
400, including on the profile check after successful token acquisition.

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
Loaded credentials must match the saved account identity before account calls;
a partial account switch fails closed. Keychain cleanup handles orphaned
credentials or metadata without requiring both to exist.

## Catalog and cart semantics

Search and detail use the live guest catalog. Prices are estimates, availability
is scoped to the catalog, and missing values remain unknown. Product output
includes official image URLs, descriptions stripped of markup, canonical links,
pack/unit information and min/step quantities. MCP product tools also return
actual JPEG image content blocks with captions and `photo_previews` metadata
mapping each product to its content indexes. Search includes at most eight
previews (one per product); detail includes up to three. CLI JSON remains unchanged.
A host controls image presentation; a custom MCP App is outside this release.

The image client is separate from authenticated API requests. It accepts only
pinned HTTPS retailer static product paths, sends no account credentials, and
never follows redirects or retries. Each source is capped at 4 MiB and 8 megapixels
(with at most 4096 pixels per dimension and a configured 64 MiB decoder
allocation budget).
Previews have a maximum edge of 512 pixels and are JPEG-encoded at quality 78,
with a 192 KiB per-image limit and a 1.5 MiB total base64-data budget. Three jobs
run concurrently across all calls;
a photo failure or timeout leaves the catalog data and source URLs available.

`get_cart` never creates a basket. `set_cart_item` sets the desired absolute
purchase-unit quantity; zero removes a known line. IDs are validated separately
from quantities. Decimal validation avoids float rounding errors. Unsupported
bonus, option and reservation behavior is rejected rather than guessed.

Before a mutation, the store persists intent and the target account/basket. A
lost response is not replayed automatically. The server reads back the result;
unresolved outcomes block later writes until a read observes the intended state
or the user explicitly acknowledges reconciliation through the local CLI.
Manual acknowledgement is restricted to the account that owns the pending write.

There is no checkout, order submission, payment, delivery reservation, coupon
activation, address mutation or automatic substitution. Explicitly requested cart
changes use the host's normal tool controls. Suggestions are previewed before
adding. A model-supplied confirmation flag is not used as an authorization gate.

## Purchase-history cache

Online order summaries and normalized product lines use a separate SQLite file
per account, named by the SHA-256 of the customer ID under the state directory.
The database also binds its account identity in metadata. Owner-only permissions,
symlink/hardlink checks and SQLite NOFOLLOW protect the database and sidecars.
SQLite is bundled into the Rust executable; no external SQLite service is needed.
Raw order payloads, personal addresses, payment details and tokens are not stored.

Sync uses authenticated GET requests to customer orders and, only when a listing
omits product lines, the corresponding order-detail endpoint. Fifty-order pages
have a bounded per-call page/time budget and a 10,000-order traversal cap. Each
order, its replacement lines and the next offset commit atomically. Only after a
successful commit does the in-memory cursor advance. Failures retain earlier
committed work. Duplicate pages, inconsistent totals and wrong-account details
fail closed; completion is never inferred from a failed or truncated page.

Cached order/product reads access metadata and SQLite, not Keychain or network.
They include completeness, timestamps and refresh guidance. Cancelled, failed
and unknown statuses remain in the order log but do not contribute repeat-product
suggestions. Name matching finds product IDs first, then uses every eligible
historical row for those IDs to keep counts and latest details correct after
renaming/localization. Unknown monetary values remain null; a product subtotal
is not substituted for the complete order total. Decimal quantities/prices are
stored as text to avoid floating-point rounding.

Logout retains history; the local `clear-history` command removes only the
connected account's database. Cached history never authorizes a cart mutation.
See [history](history.md) for coverage and retention limits.

## Shared conventions and packaging

The neighboring Sanitas/Tuta projects informed dependency versions, local CLI
conventions and stdio regression checks. No shared application dependency or new
workspace was introduced. No GPL Tuta implementation is included in this crate.

`cargo package` produces a standard private `.crate` source archive. The Homebrew
formula builds the tagged source with locked dependencies. The Codex plugin
contains only a manifest, stdio launcher and setup skill; it locates the separately
installed binary. The separate Claude `.mcpb` includes a native Apple Silicon
executable, manifest, terminal login helper and documentation. Its packager checks
the native architecture and version and archives an explicit six-file inventory.
Both hosts reuse the default local session and process lock. The release excludes
storefront bundles, upstream checkouts, account state and credentials.

See [usage](usage.md), [installation](installation.md), [verification](verification.md)
and [the original API map](frontend-api.md).
