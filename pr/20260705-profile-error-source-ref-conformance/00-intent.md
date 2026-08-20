# Profile Error Source Ref Conformance Intent

Turn the P0 Go/Python profile error source-reference implementation into a
shared conformance requirement.

This slice adds a language-neutral `error/profile_source_refs` case and makes
both P0 facade runners execute it against real profile validation and transport
error paths. The goal is to prove that profile-originated SDK errors expose
stable `profile` and `source_ref` details without changing protocol schemas,
daemon behavior, or public facade method signatures.
