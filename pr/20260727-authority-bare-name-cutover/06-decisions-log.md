# Decisions log

- 2026-07-27: Treat Authority-owned `hub.*` bare ability names as product-era
  selector aliases. Canonical Authority routing must not strip Hub vocabulary
  into runtime facts.
- 2026-07-27: Narrowed the implementation after targeted tests showed
  daemon-internal Authority registry names (`authority.binding.*`,
  `meta.list_abilities`) are still legitimate LocalRuntime registration keys.
  The retired surface is specifically `hub.*` alias projection.
