# API Contract

Public Rust signatures remain unchanged.

Serialized plugin realtime permission readiness changes one state label:

- Retired: `unknown`
- New canonical label: `action_unavailable`

This is an intentional product-readiness tightening: the old label was not a forward-compatibility state; it represented a deterministic missing policy action path.
