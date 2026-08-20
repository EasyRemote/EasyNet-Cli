# Invariants

1. Federation wire DTOs are product daemon contracts, not extension bags.
2. Unknown fields must fail before route/admission logic can infer fallback
   semantics.
3. Directory origin rewriting remains the single cross-realm provenance
   authority; inbound fields cannot carry side-channel provenance.
4. Valid current DTO shapes are preserved byte-for-byte where tests pin them.
