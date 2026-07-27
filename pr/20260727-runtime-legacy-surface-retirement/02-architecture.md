# Architecture

## Boundary

Axon/canonical SDK owns protocol and runtime abstractions. EasyNet-Cli owns
daemon and product policy. This slice must remove product or legacy vocabulary
from canonical runtime code unless it is a deliberately classified distribution
facade or product provider.

## Discovery

Use codegraph for semantic candidates, then targeted `rg` to distinguish active
implementation from documentation, product compatibility endpoints, and
negative regression tests.
