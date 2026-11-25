# Independent review iterations

The user requested three iterations with fresh reviewers and fixers. Each agent
reads the current working tree; reviewers do not edit it. Account tests use
synthetic responses and temporary file-backed state, never the user's account.

## Iteration 1

Reviewer: `review_1`. Fresh fixer: `fix_1`.

The reviewer ran the 15 initial Rust tests, inspected the Cargo package inventory,
and ran the unpacked plugin smoke test. Three P2 findings:

1. Reservation detection did not include observed `c_ao_disponibilidad` /
   `c_ao_fechaReserva` metadata or existing-basket reservation mixing rules.
2. A redundant cart read could turn a verified successful mutation into a generic
   failure after its pending record had already been cleared.
3. Line-price reporting ignored order-level discounts and labeled the base unit
   price ambiguously.

The fresh fixer addressed all three. It added observed reservation checks for
selected and existing products, batched existing-product validation, reused the
verified basket for successful/no-op output, and applied order → item → raw
line-price precedence with `base_unit_price` labeling. Four added regressions
cover the reported cases; 19 Rust tests and Clippy pass.

## Iteration 2

Reviewer: `review_2`. Fresh fixer: `fix_2`.

Two P2 findings:

1. Keychain logout relied on `connected.json`, leaving an orphaned credential
   after a metadata-write failure, or failing cleanup when the Keychain item had
   already been deleted.
2. Quantity-zero removal inherited catalog and whole-basket reservation
   prerequisites that should apply only to additions/nonzero updates; an ordinary
   discontinued product could therefore become impossible to remove through MCP.

The fresh fixer made Keychain deletion independent of the metadata marker,
accepted item-not-found as already removed, and preserved other errors/backend
mismatches. Zero-quantity removal now uses target validation → pending record →
DELETE → authoritative read-back, with no catalog prerequisite. Fake Keychain
partial-state tests and removal/read-back-failure regressions pass. Formatting,
Clippy and all 23 Rust tests pass.

## Iteration 3

Reviewer: `review_3`. Fresh fixer: `fix_3`.

The final reviewer ran all 23 existing tests and checked authentication, storage,
cart reconciliation, MCP, installation/package files, plugin launcher and CI.
Two P2 findings were confirmed:

1. A partial account switch could leave metadata for account A with credentials
   for B, allowing live operations on B while cached status still showed A.
2. The manual uncertain-write override could clear account A's pending record
   while authenticated to account B.

The final fresh fixer bound loaded credentials to the saved account identity;
missing or mismatched metadata prevents account operations and requires local
login. Manual reconciliation now checks the pending record's owning account
before any cart request. Three synthetic/private-file regressions cover
mismatch, missing metadata, matching recovery and account-bound pending handling.
Formatting, Clippy and all 26 Rust tests pass.

All seven findings from the three review/fix cycles were addressed. The root
agent inspected the final fixes and performs the release/package checks recorded
in [verification](verification.md). Live registered-account and real Keychain
compatibility remain unverified; no agent accessed those during these cycles.
