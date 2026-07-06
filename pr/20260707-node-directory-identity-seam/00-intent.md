# Intent

Add a Node/TypeScript Directory + Identity seam that follows the same profile
boundary as the Go and Python SDKs without claiming provider-backed status.

The Node SDK is P1, but it must not evolve as a separate architecture. This
slice exposes profile clients over injected transports so extension hosts and
desktop tools can depend on canonical daemon concepts while providers remain
future work.
