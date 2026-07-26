# Architecture

Mission ability ingress has one local parser boundary inside `automation/mission.rs`.

- `mission_args_object` validates the JSON envelope shape and allowed field set.
- `mission_required_string_arg` validates required non-empty string fields.
- `mission_optional_string_arg` validates optional string fields without fallback on type errors.

Handlers consume typed values from these helpers and then call the existing mission orchestration service. This keeps parsing/validation cohesive at the ability boundary and leaves execution/persistence responsibilities in the mission runtime.
