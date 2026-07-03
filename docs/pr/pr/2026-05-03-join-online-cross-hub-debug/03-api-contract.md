# API Contract

## CLI `--node`

- Accept canonical `easynet:///r/<realm>/device/<node>`.
- Accept legacy `easynet:///r/<realm>/agent/<bare-node>` during migration and rewrite it to the canonical device URI.
- Reject real hosted-agent URIs (`/agent/<user>.<agent>`).

## `federation.list_user_devices`

- Response field name stays `agent_uri` for wire compatibility.
- Value carried in `agent_uri` is the canonical device URI in v4.1.4+.

## Monitoring surfaces

- Local paired device URI shown through fleet / network-health paths must also use the canonical device URI.
