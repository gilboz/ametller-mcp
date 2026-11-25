# Frontend and API map

Captured from `https://ametllerorigen.com/es/online`, bundle `713`, on 25 November 2026. Paths below are observed contracts, not evidence that all operations have been exercised. Only guest authentication, search and product detail were tested live.

## Public configuration

The page's `script#mobify-data` contains `__CONFIG__.app.commerceAPI.parameters`:

| Parameter | Observed value |
| --- | --- |
| `shortCode` | `4jppt37a` |
| `organizationId` | `f_ecom_blzv_prd` |
| `siteId` | `ametller` |
| `clientId` | `fd3c9db8-2a0d-4f4b-9e74-294e068f9ae4` |
| Direct base | `https://4jppt37a.api.commercecloud.salesforce.com` |
| Storefront proxy | `https://ametllerorigen.com/mobify/proxy/api` |
| Locales / currency | `es`, `ca` / `EUR` |

The configuration also exposes Demandware/OCAPI proxy routes. Those are not needed for the verified catalog reads. Identifiers can change; discover and validate the current configuration instead of relying indefinitely on this snapshot. [Captured config](../sources/web/config.json).

## Main source entry points

The readable files preserve bundled module IDs and minified variable names; beautification does not recover original source names or comments.

| File under `sources/web/pretty/` | Investigation target |
| --- | --- |
| [main.js](../sources/web/pretty/main.js) | Webpack chunk table; app state; cart hook module `69121`; postal-code client module `96141`; custom saved-cart, site-preferences and Heroku clients. |
| [sdk.js](../sources/web/pretty/sdk.js) | SLAS auth helper `loginRegisteredUserB2C`, guest login/token refresh, basket/query mutation wrappers. |
| [vendor.js](../sources/web/pretty/vendor.js) | Generated Commerce SDK HTTP methods, parameter tables and resource paths; `addItemToBasket` and `updateItemInBasket`. |
| [pages-login.js](../sources/web/pretty/pages-login.js) | Login form calls `LoginRegisteredUserB2C`; guest basket merge after login. |
| [pages-cart.js](../sources/web/pretty/pages-cart.js) | Basket editing, quantity updates and removal. |
| [ametller-pages-cart.js](../sources/web/pretty/ametller-pages-cart.js) | Small locale-aware redirect to `/{language}/cart`; not the actual basket implementation. |
| [2018.js](../sources/web/pretty/2018.js) | Custom time-slot service and postal-code/slot UI. |
| [332.js](../sources/web/pretty/332.js) | Product-card add/remove behavior. |
| [pages-product.js](../sources/web/pretty/pages-product.js), [pages-product-detail.js](../sources/web/pretty/pages-product-detail.js) | Product detail and add-to-cart integration. |
| [ametller-pages-product-list.js](../sources/web/pretty/ametller-pages-product-list.js), [pages-product-list.js](../sources/web/pretty/pages-product-list.js) | Catalog/search routes and result rendering. |
| [pages-checkout.js](../sources/web/pretty/pages-checkout.js) | Shipping, address and payment workflows to investigate separately. |
| [pages-invoice.js](../sources/web/pretty/pages-invoice.js) | reCAPTCHA Enterprise load/execute and `/api/recaptcha-verify`. |

See [the complete file index](../sources/web/index.md), [raw hashes and acquisition statuses](../sources/web/manifest.json), and [formatted hashes](../sources/web/pretty-manifest.json).

## Standard API contracts

All SCAPI paths are relative to either API base above. `{org}` is the organization ID. Shopper requests carry `Authorization: Bearer <session>` and `siteId=ametller`; catalog reads explicitly choose `locale=es` or `ca`.

| Operation | Method and path | Evidence |
| --- | --- | --- |
| Guest authorize | `GET /shopper/auth/v1/organizations/{org}/oauth2/authorize` | Live 303 |
| Exchange guest code | `POST /shopper/auth/v1/organizations/{org}/oauth2/token` | Live 200 |
| Search | `GET /search/shopper-search/v1/organizations/{org}/product-search` | Live 200, direct and proxy |
| Product detail | `GET /product/shopper-products/v1/organizations/{org}/products/{productId}` | Live 200 |
| Existing customer baskets | `GET /customer/shopper-customers/v1/organizations/{org}/customers/{customerId}/baskets` | Source only |
| Create basket | `POST /checkout/shopper-baskets/v1/organizations/{org}/baskets` | Source only; explicit write |
| Read basket | `GET /checkout/shopper-baskets/v1/organizations/{org}/baskets/{basketId}` | Source only |
| Add items | `POST /checkout/shopper-baskets/v1/organizations/{org}/baskets/{basketId}/items` | Source only |
| Set line quantity | `PATCH /checkout/shopper-baskets/v1/organizations/{org}/baskets/{basketId}/items/{itemId}` | Source only |
| Remove line | `DELETE /checkout/shopper-baskets/v1/organizations/{org}/baskets/{basketId}/items/{itemId}` | Source only |

Guest authorization uses `hint=guest`, `response_type=code`, public `client_id`, `code_challenge`, `redirect_uri` and `channel_id`. The verifier is cryptographically random; challenge is unpadded base64url(SHA256(verifier)). Exchange uses form-encoded `grant_type=authorization_code_pkce`, code, verifier, client ID, redirect URI and channel ID. Validate callback origin/path before extracting the code. The probe used the registered public callback `https://www.ametllerorigen.com/callback` and did not navigate it.

Search uses `q`, `limit`, `offset`; `expand=images,prices` was tested. Detail used `allImages=true` and `expand=images,prices,availability,promotions`. Upstream implementations also show category refinement (`refine=cgid=...`) and batched product requests; those were not exercised in this assessment.

Observed upstream add body: `[{"productId":"42171","quantity":1}]`; set body: `{"quantity":2}`. These examples describe payload shapes only. Product ID and item ID are not interchangeable. Current frontend code additionally handles options, existing-line increments, reservation-specific products and cart restoration; those rules need evaluation before claiming full parity.

## Product output contract

Search names/IDs use `productName`/`productId`; detail uses `name`/`id`. Search photos are in `image.link`/`disBaseLink`; detail photos are in `imageGroups[].images[]`. Prefer a suitable image size and preserve the canonical `slugUrl` rather than guessing product links.

Descriptions can be absent or HTML: preserve meaning while producing safe readable text. Do not generate ingredients/allergen claims when the source omits them. Detail fields include `shortDescription`, optional `longDescription`, `price`, `pricePerUnit`, `unitMeasure`, `unitQuantity`, `minOrderQuantity`, `stepQuantity`, `inventory.orderable` and custom `c_ao_*` metadata.

The captured banana fixture has an estimated 0.9 kg bunch (`c_ao_pesoAproximado`) but purchase increment 1. Preserve both price/unit and pack/approximate-weight context. Use decimal arithmetic for money and quantities; the initial Rust DTO retains JSON numbers without converting to binary floats. Missing inventory means unknown. Catalog stock does not guarantee delivery stock, and guest pricing does not establish final Club discounts, shipping or weighed totals.

## Ametller-specific workflows

These paths were found in the current bundles and **not called**:

- `GET /custom/postal-codes/v1/organizations/{org}/postal-codes`: site ID plus `c_cp`, `c_comarca`, `c_provincia`, `c_poblacio`, pagination; also `/comarcas` and `/postal-codes/{id}`.
- `GET /custom/time-slots/v1/organizations/{org}/slots`: site ID and custom `c_*` query arguments assembled by the slot selector.
- `POST /custom/saved-cart/v1/organizations/{org}/saved-cart`: `{basketId, mode?}`; restoration at `/saved-cart/restore` with `{basketId}`.
- `GET /custom/saved-cart/v1/organizations/{org}/sushi-reserve`: `c_productIds`; source reads a `reserveProductIds` response to enforce special product/date behavior.

The existing Python project's [mobile Club investigation](../sources/github/andreu-alegre--ametller-cli/docs/mobile-club-investigation.md) describes a separate `app.ametllerorigen.cat` API/session for personal coupons. That is upstream evidence only and outside the initial catalog/cart requirement.

