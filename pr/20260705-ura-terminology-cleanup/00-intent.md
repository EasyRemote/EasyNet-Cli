# Intent

## Goal

Remove non-spec retired address wording from current implementation and tests
where the value is an EasyNet/Axon identity, address, or descriptor. EasyNet
daemon SDK surfaces use URA terminology only.

## Non-Goals

- Do not edit `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not rewrite historical RFC/audit documents in this slice.
- Do not rename HTTP library request-target APIs.
- Do not change wire compatibility for already-retired unknown-field rejection.

## Acceptance Criteria

- Runtime-dispatch endpoint helper names use URA terminology.
- Implementation errors/comments that refer to caller, subject, device, or
  ability identities use URA.
- Tests no longer advertise or bless retired address aliases.
- URA factory outputs are never locally named as URI values in daemon/SDK
  production sources.
- Focused Rust/Go tests and terminology scan pass.

## 2026-07-16 Iteration

The OpenAI compatibility file resource dereference path already uses the
`blob_ura` owner as its source of truth. This iteration removes the remaining
local `uri` spelling for that URA value and adds an architecture-convergence
gate so the same root-fork cannot return through another file-resource caller.
