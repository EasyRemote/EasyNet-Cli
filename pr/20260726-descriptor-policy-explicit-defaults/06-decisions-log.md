Decisions log
=============

2026-07-26
----------

- Treat descriptor visibility and scope as explicit policy facts rather than
  defaultable enum states.
- Keep constructor-authored policy defaults in `AbilityDescriptor::new` because
  those are explicit constructor semantics. Remove only trait-level defaults
  that unrelated code could inherit through generic construction.
