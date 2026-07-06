# Architecture

The Directory/Identity profile already owns paginated daemon read-model clients and DTO seams. Enum projection belongs beside that profile because it is read-model presentation logic, not product HTTP logic and not daemon transport logic.

The SDK will provide a small generic normalizer plus named helpers for directory node state and trust level. Backend code can then consume the public SDK surface without knowing Axon enum helper names.

Layering:

- Axon owns protocol enum definitions.
- EasyNet-Cli SDK owns product-facing daemon read-model projection helpers.
- EasyNet backend owns browser/API presentation and consumes SDK projection output.
