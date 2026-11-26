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

`login` prompts for your email and a hidden password in your terminal, signs in
directly over HTTPS, and checks the account before saving session tokens. It does
not open a browser. The password is not saved. An interactive terminal is required;
secrets cannot be supplied as arguments, piped input or MCP parameters.
`login --password` is a compatibility alias for the same flow.

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

CLI login cannot complete interactive website challenges. A failed route/network
request does not prove your password is wrong or that a challenge is present.
Failures distinguish sign-in from account verification, preserve the HTTP status
and omit response bodies.

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
Removing an ordinary existing line does not require that product to remain in
the current catalog. Known unsupported target-line types are still rejected.

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
The signed-in account must match the account that owns the pending write;
switching accounts does not authorize clearing another account's pending record.

## Presentation and checkout

Product tools return structured JSON with official photo URLs plus standard MCP
image blocks carrying actual thumbnails. The host controls how those images are
presented; a URL alone is not a rendering guarantee. CLI commands remain JSON.
Compare exact pack size and price per unit.

Use the official [Ametller shop](https://www.ametllerorigen.com/es/online) to review
delivery, substitutions, final totals and payment, signing into the same account
if needed. A successful cart change is not a purchase confirmation.
