# Assessment — 25 November 2026

This records the initial investigation. The subsequent implementation uses direct
CLI login, following the user's choice to keep the Rust runtime small. Browser
login below is an assessed alternative, not a dependency or implemented fallback.
See [current architecture](architecture.md) and [verification](verification.md).

## Recommendation

Use the website's Salesforce Commerce Cloud Shopper APIs (SCAPI) from Rust. This is practical for search, product detail and cart management. A live guest-session probe succeeded without credentials or CAPTCHA. Browser interaction should be needed principally for initial registered sign-in and any later challenge; the Android app is a fallback, not a prerequisite.

Existing projects already cover much of the requested behavior. There is no need to reverse engineer the entire platform from scratch. None of the three discovered MCP implementations is Rust, so the user's Rust requirement implies a small independent implementation or port informed by these references. Do not make one of the JavaScript/Python applications a runtime dependency merely to wrap it in Rust.

## Existing public implementations

GitHub repository searches used `ametller mcp`, `ametller` and `"ametller origen"`, plus web searches for GitHub API/MCP implementations. The first returned three repositories with MCP support. This is a recorded search result, not a claim that every public/private implementation was found.

| Project | Stack / license | Useful evidence | Limitations for this project |
| --- | --- | --- | --- |
| [igorsafonov-gif/ametller-origen](https://github.com/igorsafonov-gif/ametller-origen) | JavaScript / MIT | Small original implementation of SLAS auth, product images, catalog and real-cart operations | Its `getCart()` can create a basket and its availability helper always returns true; do not carry those semantics into the Rust design. |
| [denya/ametller-origen-skill](https://github.com/denya/ametller-origen-skill) | JavaScript / MIT | Expanded original with a read-only cart lookup, registered-session handling, cart tests and product presentation | Larger analytics/receipt/reorder scope than requested; authors' account/cart test claims were not independently reproduced here. |
| [andreu-alegre/ametller-cli](https://github.com/andreu-alegre/ametller-cli) | Python / MIT | Live config discovery, guest PKCE, decimal quantities, catalog, baskets, delivery/checkout investigation and MCP | More workflows than needed; its mobile Club API is separate from web shopping. |

Exact commits and local paths: [GitHub manifest](../sources/github/manifest.json). The checkouts were inspected as source; their programs, installation hooks and browser-login helpers were not run. Public search snippets were sometimes older than the checked-out code; the commit-pinned local files are the assessment basis.

## Independently verified

- `https://ametllerorigen.com/es/online`: HTTP 200; public `mobify-data` exposes PWA Kit configuration, ES/CA locales, EUR, and SCAPI identifiers.
- Captured **125 JavaScript files, 6,575,321 bytes**, from bundle `713`: HTML entries, Webpack named chunks and statically referenced dynamic imports. All 125 were parsed and beautified with Prettier 3.10.1. This is the reachable browser bundle graph, not the original development repository or server code.
- Five source-map probes returned HTTP 403; no original source-map reconstruction was possible from those URLs.
- Public-client guest PKCE: authorize HTTP 303, token exchange HTTP 200; observed access-token lifetime 1,800 seconds. Tokens were neither printed nor saved.
- Direct SCAPI product search and detail: HTTP 200. The same search through Ametller's `/mobify/proxy/api` also returned HTTP 200.
- Search returned official names, product IDs, prices and image URLs. Detail included a Spanish description, catalog orderability, quantity increments and approximate weight. [Probe report](../sources/evidence/catalog-probe.json), [product fixture](../sources/evidence/product.json).

Example observed product: `42171`, Manojo de Plátanos Canarios; €3.41 per bunch, approximately 0.9 kg, €3.79/kg, minimum/step quantity 1. These are timestamped guest catalog values, not a final delivery-price quote. Quantity 1 means one bunch, not one kilogram. The search engine reported 387 total matches for `platano`; only the first three were fetched, so the total does not establish exact relevance or full catalog coverage.

## CAPTCHA and authentication

The captured configuration contains a public reCAPTCHA site key. Executable reCAPTCHA integration was found in `pages-invoice.js`, including `/api/recaptcha-verify`. In the captured login component, the form calls `LoginRegisteredUserB2C`; no CAPTCHA invocation was found there. This is static evidence, not a guarantee that registered login cannot trigger protection.

The existing implementations use a normal browser login to obtain the user's registered SLAS session, then API requests and token refresh. Recommended initial design: user signs in locally; the MCP never takes a password as a tool parameter. If a challenge appears, let the user complete it. A CAPTCHA on one flow does not, by itself, justify moving all shopping to Android.

Salesforce documents the [public-client PKCE flow](https://developer.salesforce.com/docs/commerce/commerce-api/guide/slas-public-client.html) and [SLAS token behavior](https://developer.salesforce.com/docs/commerce/commerce-api/guide/slas.html). Registered authentication and refresh behavior for this user's account remain untested. Do not promise the session will last a particular number of days.

## Remaining uncertainty

| Area | Current evidence | What establishes completion |
| --- | --- | --- |
| Search, images, description | Live guest calls passed | Rust adapter and MCP result checked with ES/CA and multiple product types |
| Account/session | Website code and upstream implementations | Interactive local login, identity validation and refresh/restart test |
| Cart add/update/remove | Current SDK/browser code and upstream implementations | User-selected product in the real account, with before/after read-back |
| Delivery/stock | Custom postal-code and time-slot clients in frontend | Chosen delivery area/slot; confirm account basket matches website |
| Photo selection in chat | Image URLs available | Client-rendering check; an image URL alone does not guarantee a clickable photo grid |
| Final purchase | Not tested or implemented | Separate design for delivery, substitutions, total changes, payment and user authorization |
| Android | Not acquired | Only investigate if a demonstrated web limitation blocks the needed flow; optional Club coupons are a different feature |

No real cart, order, payment, delivery reservation, account setting or client configuration was changed during this assessment.

