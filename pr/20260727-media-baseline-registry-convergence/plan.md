# Media baseline registry convergence

## Goal

Make `tools/scripts/docker-media-bidi-e2e.sh` boot a clean Docker provider/caller topology without weakening daemon baseline conformance.

## Evidence

- Clean local runtime state was purged with `easynet device reset --force --yes`.
- Docker E2E reproduced a clean-state provider boot failure:
  `device baseline missing LocalRegistry abilities: ["mic.subscribe", "camera.subscribe", "camera.snapshot", "screen.subscribe", "screen.snapshot"]`.
- The failure happens before media/bidi invocation, during `ability-conformance`.

## Architectural rule

Device baseline conformance is the authority. Do not remove media abilities from the baseline and do not add a startup fallback. The registry build path must install the canonical implemented media handlers consistently with the descriptor/profile baseline.

## Implementation approach

1. Inspect the device registry assembly path for media baseline registration.
2. Consolidate registration through the media module owner.
3. Keep unimplemented media stubs out of the registry.
4. Verify with targeted baseline tests and the Docker media/bidi e2e.
