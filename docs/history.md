# Local purchase history

The MCP keeps an account-specific SQLite purchase log so browsing past orders
does not repeatedly query Ametller. This covers online orders returned by the
shop's Salesforce customer/order APIs. In-store POS receipt emails require a
separate import workflow and are not included.

## Refresh and browse

Ask the agent in chat to refresh history, show recent orders, or suggest
previously ordered products. It calls `sync_purchase_history`,
`get_purchase_history` and `get_purchased_products` for you. No terminal history
command is required; private login is the only normal terminal step.

History reads never access the network or Keychain. They select the account from
saved local connection metadata. A valid remote session is needed only for
refresh. If macOS prompts for access to the saved Keychain entry, approve that OS
prompt on your computer; do not send credentials to the agent.

The first refresh creates the database. Repeating a refresh after a completed
traversal starts at the newest page, updating old order statuses and product
lines as well as inserting new orders. An unfinished traversal resumes at its
committed offset. It does not duplicate products or orders.

## Coverage and freshness

Every result includes a `cache` object with last-attempt/last-complete timestamps,
age in seconds, stored order/product counts, reported backend total, completion
state and whether refresh is recommended. An absent cache has
`initial_sync_required: true`; it does not mean the account has no purchases.
A stale cache is never refreshed implicitly by a read.

Refresh requests 50 orders per page, with five pages per call by default and a
20-second order-fetch budget. The tool parameter `max_pages` accepts 1–100. When the stop reason
is `page_budget` or `time_budget`, the agent calls the same sync tool again to continue. Each
successfully stored order and its cursor commit together, so interrupted work
can resume. Failures preserve prior records and mark the last attempt failed.

The hard `order_limit` stop is **10,000 orders per traversal**; repeated sync calls
cannot move past it. Do not present such a cache as complete. Repeated pages,
changed totals, or mismatched account details stop safely. Use
`sync_purchase_history` with `restart: true` if the history changed during pagination.
Orders previously captured are retained even if the shop later stops listing
them, so stored counts can exceed the current backend total.

## Data and repeat-shopping suggestions

Stored fields include order ID/date/status, currency and known order total, plus
product IDs/names, line quantities, historical line totals and available official
photo URLs. Monetary/quantity values remain exact decimal strings; unknown
values stay null. A missing full order total is not replaced with a subtotal.
Addresses, customer email, payment instruments, raw responses and tokens are
excluded from the database.

Previously ordered products can be sorted by frequency or recency and searched
by any recorded name. Renaming or translating a product does not discard its
earlier purchases or hide the latest purchase. Frequency counts distinct orders.
The returned `last_line_quantity` describes one historical line; use the full
order to inspect duplicate/customized lines. Only `new`, `open`, and `completed`
orders contribute suggestions. Other statuses remain visible in the order log.

This is a starting point for suggestions, not a prediction that you are out of a
product. Historical price, quantity and photos do not establish current stock,
price or purchase increments. Use `get_product` for current data before a
user-authorized `set_cart_item`. Suggestions never add anything automatically.
Cached reads do not download product images; live catalog tools provide embedded
photos when requested.

## Storage and removal

The path is `<state-dir>/history-<customer-id-sha256>.sqlite3`. On macOS the default
state directory is `~/Library/Application Support/ametller-mcp`. The directory
and database are owner-only (0700/0600); SQLite itself is not encrypted. The
compiled server bundles SQLite, so no database server or separate SQLite install
is needed. Homebrew, Codex and Claude share this cache when they use the same
state directory and account.

Logout removes credentials but retains the log. To delete only the connected
account's local history, ask the agent to clear it before logging out. The agent
can run the local `clear-history` maintenance operation for you.

Remote orders and other accounts' caches are unaffected. No history data belongs
in Git, release archives, screenshots or diagnostics. Schema errors do not cause
an automatic database reset.

## Optional CLI for developers

The agent uses MCP tools in normal conversations. These equivalent commands are
available for development and automation; they are not setup steps for users:

```sh
ametller-mcp sync-history
ametller-mcp history --limit 20
ametller-mcp history --order-id ORDER_ID_FROM_HISTORY
ametller-mcp purchased-products yogur --sort frequent
```

## Source contracts

The captured frontend SDK declares customer-order queries with `siteId`, `offset`
and `limit` (no `locale`). `denya/ametller-origen-skill` and
`andreu-alegre/ametller-cli` use those customer-order routes;
`denya/ametller-origen-skill` also uses the registered shopper-order detail route.
The reference analytics code confirms that online orders and emailed in-store
tickets are different sources. See the pinned [reference manifest](../sources/github/manifest.json).
