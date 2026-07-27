Architecture
============

The selected abstraction is the resolve-key request DTO, not the dispatcher.

The DTO owns request shape. The handler owns trust-anchor lookup and presented
key matching. The federated key resolver owns outbound request construction.
No layer should translate a retired alternate proof-material encoding into the
canonical field.
