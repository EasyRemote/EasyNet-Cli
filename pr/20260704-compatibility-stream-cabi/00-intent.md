# Compatibility stream C ABI intent

Implement the Python Compatibility profile streaming chat completion operation as a thin facade over the existing Runtime Core stream C ABI path.

The slice must not change `docs/spec/daemon-sdk-requirements-v1.md`. It only removes the Python `_missing` seam for compatibility stream chat where Rust already owns:

- the complete stream chat Invocation carrier;
- the Runtime Core stream open/callback/close lifecycle;
- the compatibility chat stream result projector.
