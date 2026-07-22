# SDK history session admission convergence

## Goal

Make Go and Python authorized runtime-session receipt history use the same
session-authority subject admission rule as canonical runtime invocation.

## Root abstraction problem

`AuthorizedRuntimeSession.history.list` copied a legacy receipt-specific
session-subject check that accepted only exact `authority.subject_ura ==
call.subject_ura`. Runtime invocation admission already has a canonical helper:
a session authority admits exact subjects and user/agent-owned resource subjects
for the same `session_owner_user_id`.

The duplicated history-only logic creates SDK/runtime divergence and encourages
product code to mint device-scoped or placeholder session authorities for
receipt reads instead of using the canonical user-session authority model.

## Invariants

1. Delegation proof history binding remains exact-subject, because a delegation
   proof grants one explicit subject.
2. Session authority history binding must reuse the same admission helper as
   runtime invocation binding.
3. User-owned resource subjects and agent-owned resource subjects for the same
   owner user are admitted by session authority.
4. Path-substring matches remain rejected; ownership must come from parsed URA
   owner fields.
5. Go and Python SDK behavior must remain structurally identical.

## Boundary proof

- Go already has `runtimeSessionAuthorityAdmitsSubject`.
- Python already has `session_authority_admits_subject` and `_session_authority_admits_subject`.
- Reusing those helpers removes the duplicate exact-only logic instead of adding
  another history-specific branch.

## Verification plan

1. Update Go/Python authorized runtime session tests to prove history admits
   owner-equivalent user resource subjects.
2. Preserve mismatch tests for device subject and path-substring spoofing.
3. Update the existing SPEC/architecture boundary gates to forbid exact-only
   history session helpers and require both SDKs to call the canonical helper.
4. Run targeted Go/Python tests, SDK/conformance gates, formatting, and
   codegraph sync.
