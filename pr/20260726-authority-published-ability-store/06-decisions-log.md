Decisions log
=============

2026-07-26
----------

- Treat the daemon read-model store name as internal architecture, not a wire
  compatibility surface.
- Do not keep `HubPublishedAbilityStore` as a type alias; aliases would preserve
  the legacy architecture in the runtime model.
- Defer federation wire field renaming to a dedicated protocol migration.
- Rename daemon assembly, registry, and session-supervisor dependency fields to
  `authority_published_abilities` because they are internal runtime ownership,
  not wire compatibility.
- Keep `hub_published_abilities` and `hub_abilities_revision` only where they
  are serialized/deserialized federation receipt fields.
