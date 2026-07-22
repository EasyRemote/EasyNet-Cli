# Architecture

## Root abstraction defect

`LocalAbilityTarget` exposed `default_subject_ura()` publicly. That made descriptor-derived daemon-system subject policy look like a generic target property and encouraged callers in pages, principal lifecycle, discovery, MCP, and A2A adapters to assemble tuple subject facts procedurally.

## Clean target

`LocalAbilityTarget` owns the private daemon-system subject derivation. Public/product callers invoke named issuer methods that state the operation's authority source:

- daemon-system root invocation;
- daemon-system root invocation with verified metadata;
- public explicit-subject invocation.

The subject derivation policy is not visible as a reusable product adapter primitive.
