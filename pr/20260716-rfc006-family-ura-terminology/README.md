# RFC-006 Family URA Terminology

## Goal

Converge the RFC-006 normative family on URA terminology so the
stateful EasyNet identity model has one address vocabulary across the
root RFC, archived RFC copy, and companion Markdown appendix.

## Root Fork

The root `docs/AXON-RFC-006-stateful-easynet.tex` had already moved to
URA terminology, while the `docs/rfc/` RFC-006 copies still described
routable caller and owner identities as URI. That left two normative
surfaces for the same state-object identity model.

## Boundary Proof

- Axon owns protocol identity/address semantics.
- EasyNet-Cli docs may describe daemon product behavior, but the RFC-006
  state model must not fork Axon's address vocabulary.
- HTTP request `uri` wording is out of scope; this slice only targets
  EasyNet/Axon routable caller, owner, and receipt identity terms.

## Invariants

- RFC-006 identity/address text uses URA, not URI.
- LaTeX cross references use `caller-ura` when referring to the principal
  caller addressing section.
- The architecture convergence gate covers the RFC-006 family, not only
  the root `docs/` copy.

## Verification Plan

- Scan the RFC-006 family for stale URI identity terminology.
- Run `tests/scripts/test_check_architecture_convergence.sh`.
- Run `tools/scripts/check-architecture-convergence.sh`.
- Run `git diff --check`.

## Verification Result

- `rg -n "\bURI\b|\bURIs\b|caller-uri|principal-URI|agent URI|owner_agent.*URI|\buri\b|\buris\b" docs/AXON-RFC-006-stateful-easynet.tex docs/rfc/AXON-RFC-006-stateful-easynet.tex docs/rfc/AXON-RFC-006-stateful-easynet.md -S` returned no matches.
- `bash tests/scripts/test_check_architecture_convergence.sh` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `git diff --check` passed.
