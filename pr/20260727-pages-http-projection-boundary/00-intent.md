Goal: remove the misleading `pages.serve ability` compatibility framing from the local Pages HTTP listener path.

Non-goals:
- Do not implement remote Hub cutover in this iteration.
- Do not preserve a compatibility alias for the old `pages_serve_ability` module name.
- Do not claim canonical receipts for HTTP byte projection when the listener directly consumes the local fetch handler.

Acceptance criteria:
- The module name reflects an HTTP projection boundary, not an Ability implementation.
- The source no longer claims `canonical_invoke`, `Phase 2`, `01HUB.pages.serve`, or operational receipt emission for the direct path.
- Architecture gates fail if the old module name or false canonical-dispatch vocabulary reappears.
- Existing HTTP byte projection behavior remains unchanged.
