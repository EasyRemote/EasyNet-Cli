# Boundary Proof

## Ownership

Feature discovery is Runtime Core metadata owned by the EasyNet-Cli daemon SDK
core and projected through C ABI/language facades. It must describe the SDK
capability model; it must not encode product cutover policy or product-specific
route readiness.

## Invariants

1. Feature-discovery `profiles` values use only `unsupported`, `seam`,
   `provider-backed`, or `cutover-ready`.
2. Provider implementation detail remains in `symbols`, conformance cases, and
   `sdk/conformance/sdk-parity-matrix.json`; it is not encoded as custom profile
   status vocabulary.
3. Product concepts such as EasyRemote, backend route ownership, HTTP auth, and
   browser fanout do not appear in the feature catalog.
4. The change is schema-first and fixture-backed so Go, Python, Node, Java, and
   Swift decode the same canonical facts.
5. No URI terminology or legacy input aliases are introduced.

## Rejected Designs

- Keeping `partial` labels and documenting them as equivalent to
  `provider-backed`: rejected because it preserves a second status taxonomy.
- Adding product cutover statuses such as `backend-ready`: rejected because
  product readiness belongs in boundary gates, not Runtime Core feature
  discovery.
- Encoding detailed strings such as `carrier_projection_partial`: rejected
  because detailed capability evidence already has a dedicated owner.
