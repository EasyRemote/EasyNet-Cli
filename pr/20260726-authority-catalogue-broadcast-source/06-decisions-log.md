Decisions log
=============

2026-07-26
----------

- Treat catalogue `source` metadata as a public runtime DTO surface.
- Keep federation wire-field naming out of this slice to avoid mixing a wire
  compatibility migration with catalogue presentation cleanup.
- Rename `meta.rs` local parameters, test names, and assertion text from
  hub-published to Authority-published so the implementation no longer teaches
  that Hub is the catalogue authority.
- Keep `HubAbilitiesDiff`, `HubAbilityEntry`, and the current read-model file
  name unchanged in this slice because they are part of the federation
  wire/read-model boundary and require a dedicated migration.
