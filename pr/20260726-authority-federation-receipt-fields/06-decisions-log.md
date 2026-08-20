Decisions log
=============

2026-07-26
----------

- Treat Hub-named ability publication receipt facts as legacy protocol
  vocabulary, not as canonical runtime model.
- Do not introduce serde aliases for old field names.
- Keep unrelated canonical Hub transport/URA vocabulary out of this slice.
- Update SPEC v2 to require Authority receipt fields and to reject the retired
  Hub receipt fact names/types.
- Keep `since_abilities_revision` request naming unchanged because it already
  describes the caller-observed ability revision without naming Hub as owner.
