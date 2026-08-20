# Invariants

- Public action invocation remains descriptor-bound and provider-neutral.
- Runtime governance reads are not public action invocations.
- Catalogue reads must use the `ability_descriptor` provider path.
- Receipt/history/trace reads must use the `receipt_history` provider path.
- SDKs must not ask products to hand-write descriptor resolver provider JSON.
- SDK classification vocabulary must stay generic runtime vocabulary, not EasyNet or EasyRemote product vocabulary.
