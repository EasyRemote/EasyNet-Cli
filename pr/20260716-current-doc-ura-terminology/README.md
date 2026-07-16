# Current Doc URA Terminology

## Goal

Converge current normative and product-facing EasyNet docs on URA
terminology for routable caller, agent, and resource identities.

## Root Fork

Runtime and SDK code now expose canonical URA vocabulary, but several
current docs still described EasyNet/Axon identities as URI:

- RFC-003 bidi admission membership checks.
- RFC-006-A Pages caller translation in English and Chinese copies.
- Pages/OpenAI API key resource identity.

That kept old address vocabulary alive in docs that implementers use for
admission, Hub caller translation, and API-key receipt trails.

## Boundary Proof

- Axon owns invocation/admission identity semantics.
- EasyNet-Cli owns Pages/Hub product behavior, but Hub-translated callers
  still use Axon/EasyNet routable URAs.
- HTTP request URI fields and historical RFC-001 migration plans are out
  of scope for this slice.

## Invariants

- Current docs do not call EasyNet/Axon routable identities URI.
- RFC-003 membership checks describe caller URA directory membership.
- RFC-006-A Pages docs describe principal/human-anon caller URA minting
  and validation in both language copies.
- Pages/OpenAI API docs describe API key resource identity as a
  capability URA key.

## Verification Plan

- Scan the current doc set for stale URI identity terminology.
- Run `tests/scripts/test_check_architecture_convergence.sh`.
- Run `tools/scripts/check-architecture-convergence.sh`.
- Run `git diff --check`.

## Verification Result

- `rg -n "caller URI|caller URIs|agent URI|agent URIs|Capability-URI|principal URI|principal URIs|URI prefix|Device URI|URI-only|\bURI\b|\bURIs\b" docs/rfc/AXON-RFC-003-invokebidi-protocol.md docs/rfc/AXON-RFC-006-A-easynet-pages.tex docs/rfc/AXON-RFC-006-A-easynet-pages.zh-CN.tex docs/PAGES_AND_LLM_API.md -S` returned no matches.
- `bash tests/scripts/test_check_architecture_convergence.sh` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `git diff --check` passed.
