# Intent

## Root fork

Access-control mutation abilities accepted nested `owner_user_id` and
`principal_id` scalar fields inside grant/request payloads, then overwrote them
from outer URA fields. That kept a legacy scalar identity model executable at
the mutation boundary.

## Objective

Make the governance ability wire boundary URA-only. Mutation payloads may carry
domain policy fields, but owner and non-token principal identity must be derived
from `owner_ura` and `principal_ura` before entering persistence models.

## Public behavior

- Public ability names and successful URA-shaped requests remain compatible.
- Scalar-only mutation inputs remain rejected.
- Nested scalar identity fields become unknown wire fields instead of tolerated
  compatibility payloads.
