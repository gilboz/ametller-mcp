# Usage

## Browse without sign-in

```sh
ametller-mcp search "leche" --limit 6
ametller-mcp --locale ca search "llet" --limit 6 --offset 0
ametller-mcp product 42171
```

Search and product details use an anonymous guest session kept in memory. Catalog
prices may differ from registered promotions or the final delivery basket.
Descriptions/images/pack metadata come from the shop. Missing availability or
allergen information is unknown; do not infer it. A quantity of one can mean one
pack or one bunch: preserve min/step quantities, pack size and approximate weight.

## Account connection

```sh
ametller-mcp login
ametller-mcp status
ametller-mcp status --live
ametller-mcp logout
```

Only `login` prompts for credentials, and it requires an interactive terminal.
Ordinary status reports local metadata; it does not prove the remote session is
currently accepted. Live status checks account access. Refresh tokens rotate
under a local process lock. Login passwords are not saved.

On macOS the default session backend is Keychain. For a Linux host, or an explicit
file-backend choice on macOS, use consistent flags for all commands:

```sh
ametller-mcp --state-dir /absolute/private/directory --credential-store file login
ametller-mcp --state-dir /absolute/private/directory --credential-store file serve
```

File storage is plaintext protected by owner-only permissions. State is separate
from the repository, Homebrew Cellar and plugin cache. The launcher forwards
matching path/backend settings listed in [installation](installation.md).

The initial release uses direct API login. It cannot handle an interactive
browser challenge. A failed route/network request does not prove your password
is wrong; avoid repeated sign-in attempts and preserve the reported error.

## Cart review and changes

```sh
ametller-mcp cart
ametller-mcp set-cart-item 42171 2
ametller-mcp set-cart-item 42171 0
```

Those last two commands change the real account cart. They set an absolute
quantity. Product/line IDs must come from the catalog/cart. An optional `--item-id`
must identify the intended line; ambiguous or unsupported options are rejected.
Use current quantity increments; decimals are represented exactly for validation.

Cart reads do not create a basket. A new basket may be created for an explicit
nonzero item change. The result contains the returned cart for review. There is
no order submission, payment, delivery reservation, coupon activation or personal
address editing in this release.

For a suggested shopping list, show exact products, pack sizes, quantities and
estimated costs and obtain the user's selection before applying changes. For an
explicit "set these bananas to two bunches" request, apply it and show the result.
No model-supplied `confirmed=true` flag is treated as proof of user approval.
Authorization is the user's request and the host's normal tool controls; this
minimal release has no independent cart approval UI.

## An uncertain write

A timeout or malformed reply can leave the server's outcome uncertain. The MCP
does not automatically repeat the mutation. It keeps a private pending record and
blocks further changes while the result is unresolved. Read the cart and compare
it with the official website. If the desired state is observed, reconciliation
can resolve the record. Otherwise the local command:

```sh
ametller-mcp cart --clear-uncertain
```

is an explicit acknowledgement after checking the real cart. It does not undo a
change or make the prior attempt safe to replay blindly. The MCP does not expose
this override as a tool argument.

## Presentation and checkout

The server returns structured JSON and text-compatible tool output with official
photo URLs. Hosts can display those as images/cards where supported; no rendering
guarantee is implied by returning a URL. Compare exact pack size and price per unit.

Use the official [Ametller shop](https://www.ametllerorigen.com/es/online) to review
delivery, substitutions, final totals and payment, signing into the same account
if needed. A successful cart change is not a purchase confirmation.
