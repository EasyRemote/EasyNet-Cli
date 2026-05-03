# Intent

## Goal

Debug and tighten the EasyNet-Cli paths that broke after `easynet device join`:

1. The joined device may fail to appear ONLINE on operator-facing monitoring surfaces.
2. Cross-hub ability invoke may surface `target_offline` even when the target device is live.

## Non-goals

- Rework hosted-agent URI semantics.
- Redesign the federation directory wire format.
- Change the on-wire JSON field name `agent_uri`.

## Acceptance

1. Join-time wiring preserves the real device-to-hub dial target in `[daemon].hub_endpoint`.
2. Cross-hub target URIs canonicalize to `easynet:///r/<realm>/device/<node>` before presence lookup.
3. Monitoring surfaces do not synthesize legacy `/agent/<node>` device URIs for paired devices.
