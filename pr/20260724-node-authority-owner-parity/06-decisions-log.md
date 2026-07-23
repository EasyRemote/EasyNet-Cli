# Decisions Log

- 2026-07-24: Selected Node authority owner parity as this iteration's seam because Go/Python already expose canonical owner URA facts while Node only carried scalar owner IDs.
- 2026-07-24: Kept `SessionAuthorityRequest.toJSON()` on the current provider wire by lowering canonical owner/principal URAs to scalar fields and omitting the URA fields from transport JSON.
- 2026-07-24: Added SPEC v2 checks for Node canonical owner helper/state/tests rather than relying only on public API inventory hash drift.
