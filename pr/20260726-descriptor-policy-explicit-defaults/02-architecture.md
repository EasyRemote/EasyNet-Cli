Architecture
============

Root abstraction
----------------

Descriptor policy is part of the governed control-plane record. A type-level
default lets generic construction manufacture policy facts outside the
descriptor import boundary.

Boundary decision
-----------------

Remove `Default` from `Visibility` and `ScopeRule`. Policy-producing code must
name the selected state explicitly. This keeps constructor-authored defaults
local to the constructor and prevents unrelated read models from inheriting
policy via `Default::default()`.

Layering
--------

The policy state belongs to the core runtime descriptor model. SDK/product
facades should consume explicit policy facts and must not infer product
visibility or scope defaults.
