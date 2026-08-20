Public contract
===============

- Public ability names and descriptor refs remain unchanged.
- Error families remain `CANONICAL_HISTORY_READ_REQUIRED` and
  `CANONICAL_CATALOGUE_READ_REQUIRED`.
- Direct product invocation of receipt-history abilities remains rejected with
  guidance to use the canonical invocation history read path.

Request contract
================

- The selected route supplies the ability identity.
- The envelope supplies the subject URA.
- The surface name (`Invoke`, `InvokeStream`, `InvokeBidi`) selects whether a
  governance read is allowed. Only unary Invoke may carry governance reads.

Tenant rules
============

- Subject URAs are parsed as canonical runtime identity URAs.
- Receipt-history subjects must be Resource URAs.
- Catalogue read local-device subject rules are explicit; no product caller may
  infer a subject from callee ownership.
