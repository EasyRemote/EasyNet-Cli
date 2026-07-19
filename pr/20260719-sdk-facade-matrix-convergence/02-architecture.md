# Architecture

## Layering

1. Canonical SDK runtime model: generic invocation, lifecycle, receipt, session,
   and provider contracts.
2. EasyNet provider namespace: daemon-specific plugin sidecar helper facade.
3. EasyNet-Cli daemon: plugin lifecycle, template generation, route policy, and
   sidecar process execution.
4. Product templates: thin examples that import provider helpers and contain no
   duplicated protocol orchestration.

## Ownership

- The SDK owns the canonical runtime model.
- EasyNet-Cli owns plugin sidecar execution and template availability.
- Provider helper packages own language-specific convenience around the same
  sidecar frame contract.

## Capability States

- `Unsupported`: no public helper or template.
- `Seam`: matrix entry exists; helper/template are intentionally unavailable.
- `ProviderBacked`: helper delegates to the daemon/provider frame contract.
- `CutoverReady`: helper, template, conformance vectors, and negative gates pass.
