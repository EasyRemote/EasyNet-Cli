# Intent

Converge the SDK receipt-history source model onto routable receipt authority.

Receipt history clients need the ledger's canonical URA to preserve causal and
receipt-chain identity. They do not need the daemon's local ledger file path,
which is process-local operational detail owned by `easynet-daemon`, not the
provider-neutral SDK public model.
