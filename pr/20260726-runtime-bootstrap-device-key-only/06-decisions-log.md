# Decisions Log

## 2026-07-26

- Treat the Agent bootstrap alias as legacy owner ambiguity rather than source compatibility.
- Preserve the request field `owner_id` only as a partitioning guard.
- Keep federation candidate-key behavior out of scope because that path already requires a Device caller.
- Update SPEC v2 to require `bootstrap_identity_ura` and reject any restored `bootstrap_aliases`/Agent URA key registration.
