# Architecture

Root abstraction problem:

The tests used the production catalog convenience constructor as a fixture.
That constructor is allowed to consult daemon identity state; governance tests
should instead declare the authority boundary they need.

Refactoring:

- Add a local invocation-history catalog fixture with an explicit Device
  authority root.
- Route registration tests through that fixture.
- Keep the combined runtime test explicit because it validates the
  Device/Hub ownership split.
