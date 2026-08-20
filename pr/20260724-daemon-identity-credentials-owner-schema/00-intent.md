# Intent

## Goal

Remove the daemon boot identity's duplicate credentials projection and make runtime caller identity load through the owning `persistence::config::Credentials` schema.

## Non-goals

- Do not change the public `credentials.json` format.
- Do not preserve retired `agent_ura` or `tenant_id` compatibility paths.
- Do not add a boot-local fallback identity source.

## Acceptance criteria

- Daemon boot identity no longer defines or deserializes a separate `StoredDeviceIdentity`.
- Boot identity loading uses `load_credentials_optional()` and the canonical `Credentials` validation path.
- Retired/unknown credentials fields are rejected by the owning schema before signer custody is attempted.
- SPEC v2 gate rejects reintroduction of the duplicate projection or retired sentinel types.
