# Intent: Node Runtime Core Backpressure Conformance

Declare Node for the shared `stream/backpressure_bound` conformance case by
using the existing Runtime Core stream/bidi state-machine evidence in
`sdk/node/test/runtime-core.test.mjs`.

This slice does not add daemon wire-provider support and does not change Node
from `seam` to provider-backed. It only aligns the Node seam report with the
generic SDK invariant that stream and bidi queues are bounded by named constants
and overflow projects a typed terminal backpressure state.
