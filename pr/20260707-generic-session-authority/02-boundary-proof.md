# Boundary Proof

The root abstraction defect was that session authority used product-shaped
backend/user/session fields while delegated authority already used generic
issuer/subject/audience facts. That made the SDK model depend on a current
product topology and forced P1 facades to choose between copying product names
or diverging from Go/Python.

The corrected ownership is:

- Rust daemon SDK core owns canonical authority payload materialization.
- Admission verifies issuer trust, caller binding, subject binding, audience,
  scope, expiry, and signature.
- Language facades project and validate typed DTO shape only.
- Product adapters may derive these generic facts from backend, browser,
  device, or session state before calling the SDK.

No language facade constructs canonical signing bytes or maps old session fields
into the new DTO. Removing the Go session Axon bridge prevents a lossy
compatibility layer from reintroducing the retired shape.
