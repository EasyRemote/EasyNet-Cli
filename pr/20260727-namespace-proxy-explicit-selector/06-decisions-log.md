2026-07-27:
- Treat missing `ability_name` as a retired compatibility shape.
- Use `ability_name: null` for explicit directory/listing queries with no separate ability selector.
- Keep `peer_hub_urls` default-empty because it represents fanout scope, not selector inference.
