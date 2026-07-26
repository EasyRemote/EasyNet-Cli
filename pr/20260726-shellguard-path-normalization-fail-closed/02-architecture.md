Architecture
============

Domain boundary
---------------

`PathVerdict` is the state machine for the path constraint stage. Invalid path
normalization is a separate terminal state from "outside allowed roots".

Normalization
-------------

`normalise_target` and `fold_dot_dots` return `Result<PathBuf, String>`.
Callers must handle the error immediately; there is no fallback path.

Shell pipeline
--------------

`shell.run` maps both rejected and invalid path states to the same public
denial class, preserving user-facing safety while keeping internal diagnostics
precise.
