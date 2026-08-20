# Go session-authority subject helper convergence

## Goal

Move Go SDK session-authority subject admission into a dedicated authority
subject helper module, matching the Python SDK's shared
`_session_authority_subjects.py` boundary and keeping the canonical runtime
model language-neutral.

## Root abstraction problem

The Python SDK already isolates session-authority subject admission in a
shared helper consumed by runtime ability calls and invocation history. The Go
SDK used the same semantic predicate from both paths, but physically owned it
inside `runtime_ability.go`.

That file should own descriptor-bound runtime ability lowering, not authority
subject ownership rules. Keeping the predicate there makes invocation history
depend on a runtime-ability implementation detail and preserves a small
language-specific architectural divergence.

## Architectural decision

Create `sdk/go/session_authority_subjects.go` as the Go SDK authority subject
admission boundary.

The helper remains package-private and preserves the same behavior:

- exact subject equality is admitted;
- user-owned resources are admitted for the session owner;
- agent-owned resources are admitted only when the agent owner is the same
  session owner;
- path substring matches and invalid URAs are rejected.

## Boundary invariants

1. `authorized_runtime_session.go` and `runtime_ability.go` both consume the
   same Go helper.
2. The helper must not live in `runtime_ability.go`.
3. Python and Go SDKs both expose a dedicated subject-admission module/file.
4. Public SDK API and wire behavior remain unchanged.

## Verification

Completed:

- `gofmt -w sdk/go/runtime_ability.go sdk/go/session_authority_subjects.go`
- `(cd sdk/go && go test ./... -run 'TestAuthorizedRuntimeSessionHistory|TestRuntimeAbility|TestAuthorizedRuntimeSessionRejectsPathSubstringOwnerSubjectBeforeDispatch')`
- `PYTHONPATH=sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python python3 -m pytest -q sdk/python/tests/test_authorized_runtime_session.py sdk/python/tests/test_runtime_ability.py`
- `cargo fmt --check`
- `cargo check --features axon-pb`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `(cd sdk/go && go test ./...)`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
