# API contract

The runner consumes the normal Browser lifecycle environment plus six explicit
fixture commands: baseline prepare/reset, degraded apply/reset, and backpressure
apply/reset. It writes the canonical matrix to
`EASYNET_REMOTEAPP_MEDIA_ADAPTATION_EVIDENCE_JSON` or `--evidence-json`.

Production use composes it with:

```text
remoteapp-media-adaptation-e2e.sh --run --runner-cmd \
  tools/scripts/host-remoteapp-media-adaptation-e2e.sh
```
