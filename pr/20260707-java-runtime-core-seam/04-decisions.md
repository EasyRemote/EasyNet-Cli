# Decisions

- Use plain `javac` and a dependency-free test harness for the initial seam so
  package tooling does not become part of the semantic claim.
- Keep JSON payloads as opaque strings at this seam level; canonical JSON and
  schema validation remain owned by daemon/Axon-backed providers.
- Model stream/bidi bounded histories directly in Java because facade-local
  retained-history state is a language object lifetime concern, not protocol
  truth.
- Remove the public `axonPB` feature flag from the Java seam. It exposed a
  provider/protobuf detail rather than a generic SDK capability state.
