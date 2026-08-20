# Architecture

Introduce a cohesive `CallbackFrameProjection` value object at the FFI boundary.

Ownership:

- protobuf frame decoders own runtime/protocol interpretation;
- `CallbackFrameProjection` owns the pair of public frame JSON and explicit terminal state;
- reader loops own transport draining and callback delivery only.

This removes the reverse dependency from lifecycle control to public JSON serialization.

