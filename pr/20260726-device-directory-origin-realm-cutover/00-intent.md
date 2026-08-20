# Device directory origin realm cutover

## Goal

Remove the retired `tenant_id` projection from `easynet device list` JSON rows.
The CLI already consumes canonical `federation.discover` directory entries with
`origin_realm`; the projected row must preserve that canonical field instead of
re-emitting a product-era tenant alias.

## Non-goals

- Do not change the federation directory protocol.
- Do not change table rendering semantics.
- Do not add a compatibility alias that emits both `tenant_id` and
  `origin_realm`.

## Acceptance criteria

1. `project_directory_entry` projects `origin_realm` as `origin_realm`.
2. No production `device list` projection emits `tenant_id`.
3. Tests assert the canonical field and absence of the retired alias.
4. SPEC v2 gate rejects future `tenant_id` projection regressions.
