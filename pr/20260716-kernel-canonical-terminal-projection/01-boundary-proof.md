# Boundary Proof

`InvocationHandle::finalized()` verifies receipt-chain closure, signer
presence, exactly one terminal receipt, and state/receipt agreement in Axon.
Kernel must consume that immutable projection, never infer terminality from
events or synthesize a new signed receipt.

Pre-admission rejections have no Axon receipt and remain explicit unsigned
local failures. Runtime-admitted outcomes preserve the Axon terminal state and
cite its terminal receipt proof in the daemon presentation record.
