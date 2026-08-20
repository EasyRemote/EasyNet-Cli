# Canonical Runtime Convergence V2 - Intent

## Objective

Define one cross-repository convergence target for EasyNet-Cli and EasyNet-Axon.
The target removes product concepts, parallel proof paths, and independently
evolving lifecycle implementations from the canonical SDK/runtime model.

## Expected Effect

Primary effect: architecture convergence.

Secondary effects:

- a single auditable invocation/proof path;
- one language-independent lifecycle contract;
- deterministic cancellation and resource reclamation; and
- lower cost of adding products because product policy no longer changes SDK
  abstractions.

## Non-Goals

- Do not claim a throughput or latency improvement without a benchmark.
- Do not add a compatibility fallback inside the canonical runtime.
- Do not move EasyNet product policy into Axon merely to make APIs convenient.
- Do not delete a public API before its callers have a versioned replacement.
