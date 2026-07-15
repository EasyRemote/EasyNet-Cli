Access-control URA mutation boundary
====================================

Root fork
---------

Access-control mutation paths are intended to be URA-owned, but the SDK still
accepts scalar `principal_id` input as an executable fallback and emits scalar
identity fields inside outgoing authority-proof mutation payloads.

Expected effect
---------------

This slice makes mutation/request payloads URA-only while preserving read-side
projection compatibility. Provider responses may still carry historical scalar
fields as observations; SDK callers cannot use those fields to authorize new
mutations.
