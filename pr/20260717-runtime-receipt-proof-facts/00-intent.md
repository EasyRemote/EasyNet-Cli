# Intent

Advance RF-6 by separating opaque runtime receipt projection from required proof-bearing receipt validation in the Go and Python SDKs.

The SDK may preserve provider receipt JSON as an opaque projection, but any required runtime receipt constructor must reject missing descriptor, authority, proof, signature, hash, and parent-receipt binding facts.
