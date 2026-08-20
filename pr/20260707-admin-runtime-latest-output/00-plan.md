# Admin Runtime Latest Output Plan

## Objective

Make the Go Admin + Gateway runtime projection and Python profile bridge
latest-only by removing fallback parsing for retired camelCase and generic
status/id output aliases.

## Current Defect

`sdk/go/admin_runtime.go` and `sdk/python/easynet_sdk/profile_bridge.py`
accepted alternate daemon output keys such as `agentUra`, `deviceUra`,
`sessionId`, `expiresUnixMs`, `status`, `id`, `granted_scopes`, and related
compatibility names while projecting Admin + Gateway DTOs. That kept legacy
output aliases alive inside provider-backed SDK profile paths.

## Steps

1. Decode only canonical snake_case Admin + Gateway output fields in Go and
   Python.
2. Preserve request fallbacks only where values come from typed SDK requests,
   not alternate daemon output names.
3. Add scaffold guards rejecting retired alias literals in Go and Python Admin
   projection paths.
4. Run Go admin tests, Python profile bridge tests, scaffold, and aggregate SDK
   gates.
