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

Reviewer: `review_2`. Review in progress on the revised implementation.
