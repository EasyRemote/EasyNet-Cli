# Architecture

## Root abstraction problem

The runtime already has a canonical distinction between the invocation subject and receipt/ledger filters, but SDK consumers still have to construct the read subject manually. That leaves product code free to default `subject_ura` to `callee_ura`, which produces correct-looking tuples until the admission gate rejects them with `AUTHORITY_SUBJECT_MISMATCH`.

## Target boundary

- SDK: owns product-neutral runtime subject constructors and authority predicates.
- Product/CLI/UI: selects realm and user id from its authenticated session, then calls the SDK constructor.
- Runtime admission: verifies the tuple; it does not repair malformed subjects.
- Receipt/history provider: receives ledger filters only after admission.

## Module shape

- Go: add constructor beside `runtimeSessionAuthorityAdmitsSubject`.
- Python: add constructor beside `_session_authority_subjects`.
- Node: add constructor beside authority subject predicates.
- CLI Rust: keep current private value object but gate it against SDK semantics until Rust SDK extraction can consume the shared model directly.
